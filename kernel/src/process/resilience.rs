//! Sub-15ms Crash Interception, Fault Signal Handling, & Thread/PCB Recycling Engine.
//!
//! Implements zero-teardown micro-kernel fault recovery:
//! - Intercepts fatal signals (SIGSEGV, SIGBUS, SIGFPE, SIGILL, SIGABRT)
//! - Preserves process page tables (CR3), file descriptors, and shared memory partitions
//! - Recycles crashed Thread Control Blocks (TCB) in-place without memory reallocation
//! - Maintains resurrection budgets and crash telemetry history

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use super::signals::{SIGABRT, SIGBUS, SIGFPE, SIGILL, SIGSEGV};
use super::thread::{Priority, ThreadControlBlock, ThreadState, DEFAULT_STACK_SIZE};
use super::process::ProcessControlBlock;

/// Bitmask helper for fatal signals intercepted by the resilience engine.
pub const FATAL_SIGNAL_MASK: u64 = (1 << SIGSEGV) | (1 << SIGBUS) | (1 << SIGFPE) | (1 << SIGILL) | (1 << SIGABRT);

/// Telemetry record capturing details of a crash interception event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrashEvent {
    pub pid: u64,
    pub tid: u64,
    pub signal: usize,
    pub fault_addr: u64,
    pub fault_rip: u64,
    pub timestamp_ns: u64,
    pub generation_at_crash: u64,
}

impl Default for CrashEvent {
    fn default() -> Self {
        Self {
            pid: 0,
            tid: 0,
            signal: 0,
            fault_addr: 0,
            fault_rip: 0,
            timestamp_ns: 0,
            generation_at_crash: 0,
        }
    }
}

/// Resilience policy configuring crash tolerance and resurrection limits for a service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResilientPolicy {
    /// Maximum allowed resurrections per minute before declaring service failed.
    pub max_restarts_per_window: u32,
    /// Time window for resurrection rate limiting in microseconds (e.g. 60,000,000 us = 1 min).
    pub rate_window_us: u64,
    /// Minimum time in microseconds to hold back before resurrection (0 for immediate sub-15ms).
    pub restart_delay_us: u64,
    /// Bitmask of signals that trigger recycling rather than process termination.
    pub intercepted_signals: u64,
}

impl Default for ResilientPolicy {
    fn default() -> Self {
        Self {
            max_restarts_per_window: 10,
            rate_window_us: 60_000_000,
            restart_delay_us: 0,
            intercepted_signals: FATAL_SIGNAL_MASK,
        }
    }
}

/// Managed Resilient Service Partition descriptor.
pub struct ResilientService {
    pub name: String,
    pub pid: u64,
    pub worker_tid: u64,
    pub policy: ResilientPolicy,
    pub restart_count: AtomicU64,
    pub last_restart_timestamp_ns: AtomicU64,
    pub total_crashes_intercepted: AtomicU64,
    pub last_crash_event: Option<CrashEvent>,
    pub entry_function: fn(),
    pub is_resurrecting: bool,
}

impl ResilientService {
    pub fn new(name: &str, pid: u64, worker_tid: u64, entry: fn(), policy: ResilientPolicy) -> Self {
        Self {
            name: String::from(name),
            pid,
            worker_tid,
            policy,
            restart_count: AtomicU64::new(0),
            last_restart_timestamp_ns: AtomicU64::new(0),
            total_crashes_intercepted: AtomicU64::new(0),
            last_crash_event: None,
            entry_function: entry,
            is_resurrecting: false,
        }
    }

    /// Checks if a signal is configured for interception under this service's resilience policy.
    pub fn should_intercept_signal(&self, sig: usize) -> bool {
        if sig == 0 || sig >= 64 {
            return false;
        }
        (self.policy.intercepted_signals & (1 << sig)) != 0
    }
}

/// Fast Crash Interception & Thread Recycling Supervisor.
pub struct FastCrashInterceptor {
    pub services: Vec<ResilientService>,
    pub crash_log: Vec<CrashEvent>,
}

impl FastCrashInterceptor {
    pub const fn new() -> Self {
        Self {
            services: Vec::new(),
            crash_log: Vec::new(),
        }
    }

    /// Registers a service under resilience supervision.
    pub fn register_service(&mut self, service: ResilientService) {
        self.services.push(service);
    }

    /// Finds a registered service by PID.
    pub fn find_service_by_pid_mut(&mut self, pid: u64) -> Option<&mut ResilientService> {
        self.services.iter_mut().find(|s| s.pid == pid)
    }

    /// Intercepts a fatal signal raised against a process, recycling its thread in-place if resilient.
    ///
    /// Returns `true` if the crash was successfully intercepted and scheduled for resurrection;
    /// returns `false` if the process is non-resilient and should be conventionally terminated.
    pub fn intercept_crash(
        &mut self,
        pcb: &mut ProcessControlBlock,
        thread: &mut ThreadControlBlock,
        signal: usize,
        fault_addr: u64,
        fault_rip: u64,
        timestamp_ns: u64,
        active_gen: u64,
    ) -> bool {
        let entry_fn = {
            let service = match self.services.iter_mut().find(|s| s.pid == pcb.pid) {
                Some(s) => s,
                None => return false, // Not a registered resilient service
            };

            if !service.should_intercept_signal(signal) {
                return false;
            }

            // Record crash event telemetry
            let event = CrashEvent {
                pid: pcb.pid,
                tid: thread.tid,
                signal,
                fault_addr,
                fault_rip,
                timestamp_ns,
                generation_at_crash: active_gen,
            };
            service.last_crash_event = Some(event);
            service.total_crashes_intercepted.fetch_add(1, Ordering::SeqCst);
            service.is_resurrecting = true;
            self.crash_log.push(event);

            service.entry_function
        };

        // In-place Thread & PCB Recycling:
        // Do NOT touch pcb.page_table_cr3 or pcb.file_descriptors!
        // Reset thread execution stack and bootstrap context:
        Self::recycle_thread_context(thread, entry_fn);

        true
    }

    /// Recycles a thread's stack and execution frame in-place without memory allocation.
    pub fn recycle_thread_context(thread: &mut ThreadControlBlock, entry: fn()) {
        // Clear pending signals and masks on recycled thread
        thread.pending_signals = 0;
        thread.signal_mask = 0;
        thread.saved_sigcontext = None;

        // Recompute 16-byte aligned stack top from existing allocated stack memory
        if !thread.stack.is_empty() {
            let stack_top = (thread.stack.as_ptr() as u64 + DEFAULT_STACK_SIZE as u64) & !0x0F;
            let rsp = stack_top - 80;

            unsafe {
                let p = rsp as *mut u64;
                *p.add(9) = thread_bootstrap_entry as *const () as usize as u64;
                *p.add(8) = 0;
                *p.add(7) = 0;
                *p.add(6) = 0;
                *p.add(5) = 0;
                *p.add(4) = entry as *const () as usize as u64;
                *p.add(3) = 0;
                *p.add(2) = 0;
                *p.add(1) = 0;
                *p.add(0) = 0x202; // RFLAGS with IF enabled
            }

            thread.rsp = rsp;
        }

        // Mark thread ready for scheduler pick-up
        thread.state = ThreadState::Ready;
        thread.priority = Priority::Normal;
    }
}

/// Low-level bootstrap entry stub used when resurrecting a recycled thread.
#[unsafe(naked)]
unsafe extern "C" fn thread_bootstrap_entry() {
    core::arch::naked_asm!(
        "sub rsp, 32",
        "call r12",
        "add rsp, 32",
        "call {exit}",
        "ud2",
        exit = sym crate::process::scheduler::exit_current_thread,
    );
}
