//! NT Syscall Table and Calling Convention Dispatch Router.
//!
//! Maps standard 64-bit Windows NT subsystem system call numbers to kernel handlers.
//! Syscall convention:
//! - RAX: System call ID / Service number
//! - R10: First parameter (RCX is used by CPU syscall instruction for return RIP)
//! - RDX: Second parameter
//! - R8:  Third parameter
//! - R9:  Fourth parameter

use super::handlers;
use super::validate::{STATUS_NOT_IMPLEMENTED, STATUS_SUCCESS};
use core::sync::atomic::{AtomicU64, Ordering};

pub const NT_TEST_SYS: u64 = 0x0000;
pub const NT_WRITE_FILE: u64 = 0x0008;
pub const NT_READ_FILE: u64 = 0x000A;
pub const NT_ALLOCATE_VIRTUAL_MEMORY: u64 = 0x0018;
pub const NT_TERMINATE_PROCESS: u64 = 0x002C;
pub const NT_YIELD_EXECUTION: u64 = 0x0046;

static DISPATCH_COUNT: AtomicU64 = AtomicU64::new(0);

/// Returns the human-readable NT system call name for diagnostic logging.
pub const fn syscall_name(id: u64) -> &'static str {
    match id {
        NT_TEST_SYS => "NtTestSys",
        NT_WRITE_FILE => "NtWriteFile",
        NT_READ_FILE => "NtReadFile",
        NT_ALLOCATE_VIRTUAL_MEMORY => "NtAllocateVirtualMemory",
        NT_TERMINATE_PROCESS => "NtTerminateProcess",
        NT_YIELD_EXECUTION => "NtYieldExecution",
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
        _ => {
            crate::serial_write_str("[KNT-SYSCALL] Unimplemented NT service ID: 0x");
            // Log hex id
            STATUS_NOT_IMPLEMENTED
        }
    }
}
