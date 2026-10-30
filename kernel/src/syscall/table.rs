//! NT Syscall Table and Calling Convention Dispatch Router.
//! Maps standard 64-bit Windows NT subsystem system call numbers to kernel handlers.

use super::handlers;
use super::validate::{STATUS_NOT_IMPLEMENTED, STATUS_SUCCESS};
use core::sync::atomic::{AtomicU64, Ordering};

pub const NT_TEST_SYS: u64 = 0x0000;
pub const NT_WRITE_FILE: u64 = 0x0008;
pub const NT_READ_FILE: u64 = 0x000A;
pub const NT_ALLOCATE_VIRTUAL_MEMORY: u64 = 0x0018;
pub const NT_MAP_VIEW_OF_SECTION: u64 = 0x0028;
pub const NT_TERMINATE_PROCESS: u64 = 0x002C;
pub const NT_YIELD_EXECUTION: u64 = 0x0046;
pub const NT_CREATE_SECTION: u64 = 0x004A;
pub const NT_SIGACTION: u64 = 0x0050;
pub const NT_KILL: u64 = 0x0051;
pub const NT_SIGRETURN: u64 = 0x0052;
pub const NT_SIGPROCMASK: u64 = 0x0053;
pub const NT_WAIT_FOR_ALERT_BY_THREAD_ID: u64 = 0x0182;
pub const NT_ALERT_THREAD_BY_THREAD_ID: u64 = 0x0183;

pub const SYS_SIGACTION: u64 = NT_SIGACTION;
pub const SYS_KILL: u64 = NT_KILL;
pub const SYS_SIGRETURN: u64 = NT_SIGRETURN;
pub const SYS_SIGPROCMASK: u64 = NT_SIGPROCMASK;

static DISPATCH_COUNT: AtomicU64 = AtomicU64::new(0);

/// Returns the human-readable NT system call name for diagnostic logging.
pub const fn syscall_name(id: u64) -> &'static str {
    match id {
        NT_TEST_SYS => "NtTestSys",
        NT_WRITE_FILE => "NtWriteFile",
        NT_READ_FILE => "NtReadFile",
        NT_ALLOCATE_VIRTUAL_MEMORY => "NtAllocateVirtualMemory",
        NT_MAP_VIEW_OF_SECTION => "NtMapViewOfSection",
        NT_TERMINATE_PROCESS => "NtTerminateProcess",
        NT_YIELD_EXECUTION => "NtYieldExecution",
        NT_CREATE_SECTION => "NtCreateSection",
        NT_SIGACTION => "NtSigAction",
        NT_KILL => "NtKill",
        NT_SIGRETURN => "NtSigReturn",
        NT_SIGPROCMASK => "NtSigProcMask",
        NT_WAIT_FOR_ALERT_BY_THREAD_ID => "NtWaitForAlertByThreadId",
        NT_ALERT_THREAD_BY_THREAD_ID => "NtAlertThreadByThreadId",
        _ => "UnknownSyscall",
    }
}

/// Returns the total number of syscall invocations handled.
pub fn total_dispatches() -> u64 {
    DISPATCH_COUNT.load(Ordering::Relaxed)
}

/// Routes an incoming NT syscall to its registered kernel implementation.
pub fn dispatch(id: u64, arg1: u64, arg2: u64, arg3: u64, arg4: u64) -> u32 {
    DISPATCH_COUNT.fetch_add(1, Ordering::Relaxed);

    match id {
        NT_TEST_SYS => {
            crate::serial_write_str("[KNT-SYSCALL] NtTestSys dispatch verified\r\n");
            STATUS_SUCCESS
        }
        NT_WRITE_FILE => handlers::sys_write_file(arg1, arg2, arg3, arg4),
        NT_READ_FILE => handlers::sys_read_file(arg1, arg2, arg3, arg4),
        NT_ALLOCATE_VIRTUAL_MEMORY => {
            handlers::sys_allocate_virtual_memory(arg1, arg2, arg3, arg4)
        }
        NT_TERMINATE_PROCESS => handlers::sys_terminate_process(arg1, arg2),
        NT_YIELD_EXECUTION => handlers::sys_yield_execution(),
        NT_CREATE_SECTION => handlers::sys_create_section(arg1, arg2, arg3, arg4),
        NT_MAP_VIEW_OF_SECTION => handlers::sys_map_view_of_section(arg1, arg2, arg3, arg4),
        NT_SIGACTION => handlers::sys_sigaction(arg1, arg2, arg3),
        NT_KILL => handlers::sys_kill(arg1, arg2),
        NT_SIGRETURN => handlers::sys_sigreturn(),
        NT_SIGPROCMASK => handlers::sys_sigprocmask(arg1, arg2, arg3),
        NT_WAIT_FOR_ALERT_BY_THREAD_ID => {
            handlers::sys_wait_for_alert_by_thread_id(arg1, arg2, arg3, arg4)
        }
        NT_ALERT_THREAD_BY_THREAD_ID => handlers::sys_alert_thread_by_thread_id(arg1, arg2),
        _ => {
            crate::serial_write_str("[KNT-SYSCALL] Unimplemented NT service ID: 0x");
            STATUS_NOT_IMPLEMENTED
        }
    }
}
