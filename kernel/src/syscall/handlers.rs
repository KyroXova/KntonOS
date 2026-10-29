//! Core NT Subsystem System Call Stubs and Implementation Handlers.

use super::validate::{
    validate_user_mut_ptr, validate_user_mut_slice, validate_user_slice, STATUS_INVALID_HANDLE,
    STATUS_INVALID_PARAMETER, STATUS_SUCCESS,
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
