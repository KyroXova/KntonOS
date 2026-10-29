//! Priority-aware Round-Robin Scheduler with quantum tracking and cooperative dispatch.

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use super::context::switch_context;
use super::thread::{Priority, ThreadControlBlock, ThreadState, DEFAULT_TIME_QUANTUM};
use crate::memory::spinlock::Spinlock;
use crate::serial_write_str;

pub struct Scheduler {
    pub current: Option<Box<ThreadControlBlock>>,
    pub runqueues: [VecDeque<Box<ThreadControlBlock>>; 5],
    pub blocked_threads: alloc::vec::Vec<Box<ThreadControlBlock>>,
    pub switch_count: u64,
}

impl Scheduler {
    pub const fn new() -> Self {
        Self {
            current: None,
            runqueues: [
                VecDeque::new(),
                VecDeque::new(),
                VecDeque::new(),
                VecDeque::new(),
                VecDeque::new(),
            ],
            blocked_threads: alloc::vec::Vec::new(),
            switch_count: 0,
        }
    }

    /// Pushes a ready thread into its corresponding priority runqueue.
    pub fn enqueue(&mut self, thread: Box<ThreadControlBlock>) {
        let p_idx = thread.priority as usize;
        self.runqueues[p_idx].push_back(thread);
    }
}

pub static SCHEDULER: Spinlock<Option<Scheduler>> = Spinlock::new(None);

/// Initializes the global scheduler and establishes the initial boot/idle thread context.
pub fn init() {
    let mut guard = SCHEDULER.lock();
    let mut sched = Scheduler::new();

    // Establish current execution context as idle thread (PID 0, TID 1)
    let idle_thread = Box::new(ThreadControlBlock::new_idle_thread(0, "knton_idle"));
    sched.current = Some(idle_thread);

    *guard = Some(sched);
    serial_write_str("[KNT-SCHED] Scheduler initialized with priority-aware runqueues\r\n");
}

/// Spawns a new kernel thread and enqueues it for execution.
pub fn spawn(name: &str, priority: Priority, entry: fn()) -> u64 {
    let thread = Box::new(ThreadControlBlock::new_kernel_thread(1, name, priority, entry));
    let tid = thread.tid;

    let mut guard = SCHEDULER.lock();
    if let Some(ref mut sched) = *guard {
        sched.enqueue(thread);
    }

    tid
}

/// Cooperatively yields the processor to the next highest-priority ready thread.
pub fn yield_now() {
    let (prev_rsp_ptr, next_rsp) = {
        let mut guard = SCHEDULER.lock();
        let sched = match guard.as_mut() {
            Some(s) => s,
            None => return,
        };

        // Pick next thread: check priorities from Realtime (4) down to Idle (0)
        let mut next_opt = None;
        for queue in sched.runqueues.iter_mut().rev() {
            if let Some(t) = queue.pop_front() {
                next_opt = Some(t);
                break;
            }
        }

        let mut next = match next_opt {
            Some(t) => t,
            None => return, // No other thread available, continue running current
        };

        let mut prev = match sched.current.take() {
            Some(p) => p,
            None => return,
        };

        let is_terminated = prev.state == ThreadState::Terminated;
        let is_blocked = prev.state == ThreadState::Blocked;

        if !is_terminated && !is_blocked {
            prev.state = ThreadState::Ready;
            prev.quantum_remaining = DEFAULT_TIME_QUANTUM;
        }

        next.state = ThreadState::Running;
        next.quantum_remaining = DEFAULT_TIME_QUANTUM;

        let prev_rsp_ptr = &raw mut prev.rsp;
        let next_rsp = next.rsp;

        // Re-enqueue prev thread into its priority runqueue ONLY if runnable
        if is_blocked {
            sched.blocked_threads.push(prev);
        } else if !is_terminated {
            sched.runqueues[prev.priority as usize].push_back(prev);
        }
        sched.current = Some(next);
        sched.switch_count += 1;

        (prev_rsp_ptr, next_rsp)
    };

    // Perform low-level register swap and stack switch outside the spinlock
    unsafe {
        switch_context(prev_rsp_ptr, next_rsp);
    }
}

/// Marks the current thread as Blocked and context switches to the next available thread.
pub fn block_current_thread() {
    let (prev_rsp_ptr, next_rsp) = {
        let mut guard = SCHEDULER.lock();
        let sched = match guard.as_mut() {
            Some(s) => s,
            None => return,
        };

        let mut next_opt = None;
        for queue in sched.runqueues.iter_mut().rev() {
            if let Some(t) = queue.pop_front() {
                next_opt = Some(t);
                break;
            }
        }

        let mut next = match next_opt {
            Some(t) => t,
            None => return,
        };

        let mut prev = match sched.current.take() {
            Some(p) => p,
            None => return,
        };

        prev.state = ThreadState::Blocked;
        next.state = ThreadState::Running;
        next.quantum_remaining = DEFAULT_TIME_QUANTUM;

        let prev_rsp_ptr = &raw mut prev.rsp;
        let next_rsp = next.rsp;

        sched.blocked_threads.push(prev);
        sched.current = Some(next);
        sched.switch_count += 1;

        (prev_rsp_ptr, next_rsp)
    };

    unsafe {
        switch_context(prev_rsp_ptr, next_rsp);
    }
}

/// Unblocks a previously blocked thread by TID, re-enqueueing it into its priority runqueue.
pub fn wake_thread(tid: u64) -> bool {
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return false,
    };

    if let Some(idx) = sched.blocked_threads.iter().position(|t| t.tid == tid) {
        let mut thread = sched.blocked_threads.remove(idx);
        thread.state = ThreadState::Ready;
        thread.quantum_remaining = DEFAULT_TIME_QUANTUM;
        sched.enqueue(thread);
        true
    } else {
        false
    }
}

/// Returns the TID of the currently active thread.
pub fn current_thread_id() -> u64 {
    let guard = SCHEDULER.lock();
    if let Some(ref sched) = *guard {
        if let Some(ref current) = sched.current {
            return current.tid;
        }
    }
    0
}

/// Terminates the current thread and immediately switches to the next available thread.
pub fn exit_current_thread() -> ! {
    {
        let mut guard = SCHEDULER.lock();
        if let Some(ref mut sched) = *guard {
            if let Some(ref mut current) = sched.current {
                current.state = ThreadState::Terminated;
            }
        }
    }
    yield_now();
    loop {
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}

/// Periodic timer tick hook for quantum tracking and preemption.
pub fn timer_tick() {
    let should_yield = {
        let mut guard = SCHEDULER.lock();
        if let Some(ref mut sched) = *guard {
            if let Some(ref mut current) = sched.current {
                current.total_ticks += 1;
                if current.quantum_remaining > 0 {
                    current.quantum_remaining -= 1;
                }
                current.quantum_remaining == 0
            } else {
                false
            }
        } else {
            false
        }
    };

    if should_yield {
        yield_now();
    }
}
