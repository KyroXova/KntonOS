//! Thread Control Block (TCB), ThreadState, Priority, and Thread Lifecycle management.

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

static NEXT_TID: AtomicU64 = AtomicU64::new(1);

pub const DEFAULT_STACK_SIZE: usize = 16384; // 16 KiB
pub const DEFAULT_TIME_QUANTUM: usize = 10;   // 10 scheduler ticks per quantum

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    Ready,
    Running,
    Blocked,
    Terminated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Idle = 0,
    Low = 1,
    Normal = 2,
    High = 3,
    Realtime = 4,
}

pub struct ThreadControlBlock {
    pub tid: u64,
    pub pid: u64,
    pub name: String,
    pub state: ThreadState,
    pub priority: Priority,
    pub rsp: u64,
    pub stack: Vec<u8>,
    pub quantum_remaining: usize,
    pub total_ticks: u64,
    pub signal_mask: u64,
    pub pending_signals: u64,
    pub saved_sigcontext: Option<super::signals::SigContext>,
}

impl ThreadControlBlock {
    /// Creates a newly initialized kernel thread ready to execute `entry`.
    pub fn new_kernel_thread(pid: u64, name: &str, priority: Priority, entry: fn()) -> Self {
        let tid = NEXT_TID.fetch_add(1, Ordering::Relaxed);
        let mut stack = Vec::with_capacity(DEFAULT_STACK_SIZE);
        stack.resize(DEFAULT_STACK_SIZE, 0);

        // Align stack top to 16 bytes
        let stack_top = (stack.as_ptr() as u64 + DEFAULT_STACK_SIZE as u64) & !0x0F;

        // Initialize stack frame for `switch_context` (win64 calling convention):
        // 10 64-bit entries (80 bytes total):
        let rsp = stack_top - 80;

        unsafe {
            let p = rsp as *mut u64;
            *p.add(9) = thread_bootstrap as *const () as usize as u64;
            *p.add(8) = 0;
            *p.add(7) = 0;
            *p.add(6) = 0;
            *p.add(5) = 0;
            *p.add(4) = entry as *const () as usize as u64;
            *p.add(3) = 0;
            *p.add(2) = 0;
            *p.add(1) = 0;
            *p.add(0) = 0x202;
        }

        Self {
            tid,
            pid,
            name: String::from(name),
            state: ThreadState::Ready,
            priority,
            rsp,
            stack,
            quantum_remaining: DEFAULT_TIME_QUANTUM,
            total_ticks: 0,
            signal_mask: 0,
            pending_signals: 0,
            saved_sigcontext: None,
        }
    }

    /// Creates an existing/idle thread representation for the boot context.
    pub fn new_idle_thread(pid: u64, name: &str) -> Self {
        let tid = NEXT_TID.fetch_add(1, Ordering::Relaxed);
        Self {
            tid,
            pid,
            name: String::from(name),
            state: ThreadState::Running,
            priority: Priority::Idle,
            rsp: 0,
            stack: Vec::new(),
            quantum_remaining: DEFAULT_TIME_QUANTUM,
            total_ticks: 0,
            signal_mask: 0,
            pending_signals: 0,
            saved_sigcontext: None,
        }
    }
}

/// Trampoline that executes a newly switched thread's entry function.
/// R12 contains the function pointer stored during stack initialization.
#[unsafe(naked)]
unsafe extern "C" fn thread_bootstrap() {
    core::arch::naked_asm!(
        "sub rsp, 32", // 32-byte Win64 shadow space
        "call r12",
        "add rsp, 32",
        "call {exit}",
        "ud2",
        exit = sym thread_exit_handler,
    );
}

#[no_mangle]
extern "C" fn thread_exit_handler() -> ! {
    crate::serial_write_str("[KNT-PROC] Thread execution completed. Terminating.\r\n");
    crate::process::scheduler::exit_current_thread();
}
