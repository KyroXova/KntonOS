//! Core NT Subsystem System Call Stubs and Implementation Handlers.

use super::validate::{
    validate_user_mut_ptr, validate_user_mut_slice, validate_user_ptr, validate_user_slice,
    STATUS_INVALID_HANDLE, STATUS_INVALID_PARAMETER, STATUS_SUCCESS,
};
use crate::memory::{PAGE_SIZE, PMM};

/// Handle representing stdout / serial console output in NT subsystem.
pub const NT_CONSOLE_HANDLE: u64 = 0x0000_0001;
pub const NT_CURRENT_PROCESS: u64 = 0xFFFF_FFFF_FFFF_FFFF;

/// NtYieldExecution: Yields current CPU timeslice.
pub fn sys_yield_execution() -> u32 {
    core::hint::spin_loop();
    STATUS_SUCCESS
}

/// NtWriteFile: Writes data from a userland buffer to an open handle (serial/console).
/// - arg1: Handle to file/device
/// - arg2: Reserved/Event handle
/// - arg3: Userland buffer pointer
/// - arg4: Byte length of buffer
pub fn sys_write_file(handle: u64, _event: u64, buffer_ptr: u64, length: u64) -> u32 {
    // Validate buffer bounds before touching memory
    let slice = match validate_user_slice(buffer_ptr as *const u8, length as usize) {
        Ok(s) => s,
        Err(status) => return status,
    };

    // Console output handler
    if handle == NT_CONSOLE_HANDLE || handle == 0 || handle == 1 {
        for &b in slice {
            crate::serial_write_byte(b);
        }
        STATUS_SUCCESS
    } else {
        STATUS_INVALID_HANDLE
    }
}

/// NtReadFile: Reads data into a userland buffer from an open handle.
/// - arg1: Handle to file/device
/// - arg2: Reserved/Event handle
/// - arg3: Userland destination buffer pointer
/// - arg4: Maximum byte length to read
pub fn sys_read_file(handle: u64, _event: u64, buffer_ptr: u64, length: u64) -> u32 {
    let _slice = match validate_user_mut_slice(buffer_ptr as *mut u8, length as usize) {
        Ok(s) => s,
        Err(status) => return status,
    };

    if handle == NT_CONSOLE_HANDLE || handle == 0 {
        // No input pending on COM1
        STATUS_SUCCESS
    } else {
        STATUS_INVALID_HANDLE
    }
}

/// NtAllocateVirtualMemory: Allocates or commits virtual memory pages for a process.
/// - arg1: Process handle
/// - arg2: Pointer to BaseAddress (in/out)
/// - arg3: Pointer to RegionSize (in/out)
/// - arg4: Allocation type and protection flags
pub fn sys_allocate_virtual_memory(
    process_handle: u64,
    base_addr_ptr: u64,
    region_size_ptr: u64,
    _protect: u64,
) -> u32 {
    if process_handle != NT_CURRENT_PROCESS && process_handle != 0 {
        return STATUS_INVALID_HANDLE;
    }

    let base_ptr = match validate_user_mut_ptr(base_addr_ptr as *mut usize) {
        Ok(p) => p,
        Err(status) => return status,
    };

    let size_ptr = match validate_user_mut_ptr(region_size_ptr as *mut usize) {
        Ok(p) => p,
        Err(status) => return status,
    };

    let req_size = unsafe { *size_ptr };
    if req_size == 0 {
        return STATUS_INVALID_PARAMETER;
    }

    // Align to 4 KiB page size
    let page_count = (req_size + PAGE_SIZE - 1) / PAGE_SIZE;
    let actual_size = page_count * PAGE_SIZE;

    // Allocate physical frames from PMM
    let mut pmm = PMM.lock();
    let _first_frame = match pmm.allocate_frame() {
        Some(f) => f,
        None => return 0xC000_0017, // STATUS_NO_MEMORY
    };

    // If base address was 0, assign virtual address in userland pool
    unsafe {
        if *base_ptr == 0 {
            *base_ptr = 0x0000_0000_1000_0000; // 256 MiB user heap base
        }
        *size_ptr = actual_size;
    }

    STATUS_SUCCESS
}

/// NtTerminateProcess: Terminates the target process with an exit status code.
/// - arg1: Process handle
/// - arg2: Exit status code
pub fn sys_terminate_process(process_handle: u64, exit_code: u64) -> u32 {
    crate::serial_write_str("[KNT-SYSCALL] NtTerminateProcess called (handle: ");
    if process_handle == NT_CURRENT_PROCESS || process_handle == 0 {
        crate::serial_write_str("CURRENT, exit: ");
        if exit_code == 0 {
            crate::serial_write_str("0: SUCCESS)\r\n");
        } else {
            crate::serial_write_str("NON_ZERO)\r\n");
        }
        STATUS_SUCCESS
    } else {
        crate::serial_write_str("INVALID)\r\n");
        STATUS_INVALID_HANDLE
    }
}

/// NtCreateSection: Creates a shared memory section.
/// - arg1: Pointer to receive SectionHandle (*mut u64)
/// - arg2: Desired access flags
/// - arg3: Section size in bytes
/// - arg4: Object attributes / name pointer
pub fn sys_create_section(
    section_handle_ptr: u64,
    _desired_access: u64,
    size: u64,
    _name_ptr: u64,
) -> u32 {
    let out_ptr = if section_handle_ptr != 0 {
        section_handle_ptr as *mut u64
    } else {
        return STATUS_INVALID_PARAMETER;
    };

    if size == 0 {
        return STATUS_INVALID_PARAMETER;
    }

    let mut sm = crate::ipc::SECTION_MANAGER.lock();
    match sm.create_section("Section", size as usize) {
        Ok(handle) => {
            unsafe {
                *out_ptr = handle;
            }
            STATUS_SUCCESS
        }
        Err(status) => status,
    }
}

/// NtMapViewOfSection: Maps a view of a section into the virtual address space.
/// - arg1: Section handle
/// - arg2: Process handle (NT_CURRENT_PROCESS)
/// - arg3: Pointer to BaseAddress (*mut usize)
/// - arg4: View size in bytes
pub fn sys_map_view_of_section(
    section_handle: u64,
    process_handle: u64,
    base_addr_ptr: u64,
    _view_size: u64,
) -> u32 {
    if process_handle != NT_CURRENT_PROCESS && process_handle != 0 {
        return STATUS_INVALID_HANDLE;
    }

    let base_ptr = if base_addr_ptr != 0 {
        base_addr_ptr as *mut usize
    } else {
        return STATUS_INVALID_PARAMETER;
    };

    let preferred = unsafe { *base_ptr };
    let sm = crate::ipc::SECTION_MANAGER.lock();
    match sm.map_view(section_handle, preferred) {
        Ok(mapped_vaddr) => {
            unsafe {
                *base_ptr = mapped_vaddr;
            }
            STATUS_SUCCESS
        }
        Err(status) => status,
    }
}

/// NtWaitForAlertByThreadId: Fast futex wait operation.
/// - arg1: Address of futex variable (*const u32)
/// - arg2: Expected value (u32)
/// - arg3: Timeout in ticks (0 = infinite)
pub fn sys_wait_for_alert_by_thread_id(
    address: u64,
    expected_val: u64,
    timeout_ticks: u64,
    _reserved: u64,
) -> u32 {
    if address == 0 || (address & 0x03) != 0 {
        return STATUS_INVALID_PARAMETER;
    }

    let timeout = if timeout_ticks == 0 {
        None
    } else {
        Some(timeout_ticks)
    };

    match crate::ipc::futex_wait(address as *const u32, expected_val as u32, timeout) {
        Ok(()) => STATUS_SUCCESS,
        Err(crate::ipc::FutexError::ValueMismatch) => 0xC000_0001, // STATUS_UNSUCCESSFUL
        Err(crate::ipc::FutexError::InvalidAddress) => STATUS_INVALID_PARAMETER,
        Err(crate::ipc::FutexError::Timeout) => 0x0000_0102, // STATUS_TIMEOUT
        Err(crate::ipc::FutexError::QueueUnavailable) => 0xC000_0017,
    }
}

/// NtAlertThreadByThreadId: Fast futex wake operation.
/// - arg1: Thread ID or futex address
/// - arg2: Wake count (if > 0, wakes futex address in arg1; if 0, wakes TID in arg1)
pub fn sys_alert_thread_by_thread_id(
    target: u64,
    count: u64,
) -> u32 {
    if count > 0 && target != 0 {
        crate::ipc::futex_wake(target as *const u32, count as u32);
        STATUS_SUCCESS
    } else if target != 0 {
        if crate::ipc::futex_wake_tid(target) {
            STATUS_SUCCESS
        } else {
            STATUS_INVALID_HANDLE
        }
    } else {
        STATUS_INVALID_PARAMETER
    }
}

/// NtSigAction: Sets or retrieves action associated with a signal.
/// - arg1: Signal number
/// - arg2: Pointer to new SigAction (*const SigAction)
/// - arg3: Pointer to receive old SigAction (*mut SigAction)
pub fn sys_sigaction(signum: u64, act_ptr: u64, oldact_ptr: u64) -> u32 {
    let sig = signum as usize;
    if sig == 0 || sig >= crate::process::NSIG {
        return STATUS_INVALID_PARAMETER;
    }
    if sig == crate::process::SIGKILL || sig == crate::process::SIGSTOP {
        return STATUS_INVALID_PARAMETER;
    }

    let pid = {
        let guard = crate::process::SCHEDULER.lock();
        if let Some(ref sched) = *guard {
            if let Some(ref curr) = sched.current {
                curr.pid
            } else {
                1
            }
        } else {
            1
        }
    };

    let old_action = {
        let sm = crate::process::signals::SIGNAL_MANAGER.lock();
        match sm.get_sigaction(pid, sig) {
            Ok(act) => act,
            Err(_) => return STATUS_INVALID_PARAMETER,
        }
    };

    if oldact_ptr != 0 {
        let out_ptr = match validate_user_mut_ptr(oldact_ptr as *mut crate::process::SigAction) {
            Ok(p) => p,
            Err(status) => return status,
        };
        unsafe {
            *out_ptr = old_action;
        }
    }

    if act_ptr != 0 {
        let in_ptr = match validate_user_ptr(act_ptr as *const crate::process::SigAction) {
            Ok(p) => p,
            Err(status) => return status,
        };
        let new_action = unsafe { *in_ptr };
        let mut sm = crate::process::signals::SIGNAL_MANAGER.lock();
        if sm.set_sigaction(pid, sig, new_action).is_err() {
            return STATUS_INVALID_PARAMETER;
        }
    }

    STATUS_SUCCESS
}

/// NtKill: Sends a signal to a process or thread.
/// - arg1: Target PID (0 or NT_CURRENT_PROCESS = current process)
/// - arg2: Signal number
pub fn sys_kill(pid: u64, sig: u64) -> u32 {
    let sig = sig as usize;
    if sig >= crate::process::NSIG {
        return STATUS_INVALID_PARAMETER;
    }

    let target_pid = if pid == NT_CURRENT_PROCESS || pid == 0 {
        let guard = crate::process::SCHEDULER.lock();
        if let Some(ref sched) = *guard {
            if let Some(ref curr) = sched.current {
                curr.pid
            } else {
                1
            }
        } else {
            1
        }
    } else {
        pid
    };

    if sig == 0 {
        return STATUS_SUCCESS;
    }

    let mut sm = crate::process::signals::SIGNAL_MANAGER.lock();
    match sm.send_signal(target_pid, sig) {
        Ok(()) => {
            let mut sched_guard = crate::process::SCHEDULER.lock();
            if let Some(ref mut sched) = *sched_guard {
                let mut wake_list = alloc::vec::Vec::new();
                for t in sched.blocked_threads.iter() {
                    if (t.pid == target_pid || target_pid == 0) && (t.signal_mask & (1 << sig)) == 0 {
                        wake_list.push(t.tid);
                    }
                }
                for tid in wake_list {
                    if let Some(pos) = sched.blocked_threads.iter().position(|t| t.tid == tid) {
                        let mut thread = sched.blocked_threads.remove(pos);
                        thread.state = crate::process::ThreadState::Ready;
                        sched.enqueue(thread);
                    }
                }
            }
            STATUS_SUCCESS
        }
        Err(_) => STATUS_INVALID_HANDLE,
    }
}

/// NtSigReturn: Restores the execution context after signal handler execution.
pub fn sys_sigreturn() -> u32 {
    let mut guard = crate::process::SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return STATUS_INVALID_HANDLE,
    };

    if let Some(ref mut curr) = sched.current {
        if let Some(saved) = curr.saved_sigcontext.take() {
            curr.signal_mask = saved.oldmask;
            curr.rsp = saved.rsp;
            STATUS_SUCCESS
        } else {
            STATUS_INVALID_PARAMETER
        }
    } else {
        STATUS_INVALID_HANDLE
    }
}

/// NtSigProcMask: Examines and changes blocked signals mask for current thread.
/// - arg1: How flag (SIG_BLOCK=0, SIG_UNBLOCK=1, SIG_SETMASK=2)
/// - arg2: Pointer to new mask (*const u64)
/// - arg3: Pointer to receive old mask (*mut u64)
pub fn sys_sigprocmask(how: u64, set_ptr: u64, oldset_ptr: u64) -> u32 {
    let mut guard = crate::process::SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return STATUS_INVALID_HANDLE,
    };

    let curr = match sched.current.as_mut() {
        Some(c) => c,
        None => return STATUS_INVALID_HANDLE,
    };

    if oldset_ptr != 0 {
        let out_ptr = match validate_user_mut_ptr(oldset_ptr as *mut u64) {
            Ok(p) => p,
            Err(status) => return status,
        };
        unsafe {
            *out_ptr = curr.signal_mask;
        }
    }

    if set_ptr != 0 {
        let in_ptr = match validate_user_ptr(set_ptr as *const u64) {
            Ok(p) => p,
            Err(status) => return status,
        };
        let new_set = unsafe { *in_ptr };
        let unblockable = (1u64 << crate::process::SIGKILL) | (1u64 << crate::process::SIGSTOP);
        let safe_set = new_set & !unblockable;

        match how as u32 {
            crate::process::SIG_BLOCK => {
                curr.signal_mask |= safe_set;
            }
            crate::process::SIG_UNBLOCK => {
                curr.signal_mask &= !safe_set;
            }
            crate::process::SIG_SETMASK => {
                curr.signal_mask = safe_set;
            }
            _ => return STATUS_INVALID_PARAMETER,
        }
    }

    STATUS_SUCCESS
}
