//! Userland Execution and Syscall Dispatch Engine for KntonOS.
//!
//! Provides fast system call routing matching Windows NT calling conventions,
//! userland buffer sanitation, and core NT kernel service stubs.

pub mod handlers;
pub mod table;
pub mod validate;

pub use handlers::{NT_CONSOLE_HANDLE, NT_CURRENT_PROCESS};
pub use table::{
    dispatch, syscall_name, total_dispatches, NT_ALERT_THREAD_BY_THREAD_ID,
    NT_ALLOCATE_VIRTUAL_MEMORY, NT_CREATE_SECTION, NT_MAP_VIEW_OF_SECTION, NT_READ_FILE,
    NT_TERMINATE_PROCESS, NT_TEST_SYS, NT_WAIT_FOR_ALERT_BY_THREAD_ID, NT_WRITE_FILE,
    NT_YIELD_EXECUTION,
};
pub use validate::*;

/// Initializes the system call subsystem.
pub fn init() {
    crate::serial_write_str("[KNT-SYSCALL-INIT] Initializing userland NT syscall engine...\r\n");
    crate::serial_write_str("[KNT-SYSCALL] Fast dispatch table registered for NT subsystem\r\n");
}

/// Executes comprehensive self-test suite for syscall dispatch and validation.
pub fn test_syscall_subsystem() -> bool {
    crate::serial_write_str("[KNT-SYSCALL-TEST] Executing syscall engine self-test suite...\r\n");

    // --- TEST 1: Userland Pointer & Slice Validation ---
    {
        // 1.1 Null pointer must return STATUS_ACCESS_VIOLATION
        if validate_user_slice(core::ptr::null(), 64) != Err(STATUS_ACCESS_VIOLATION) {
            crate::serial_write_str("[KNT-PANIC] Null pointer did not return ACCESS_VIOLATION!\r\n");
            return false;
        }

        // 1.2 Kernel higher-half pointer must return STATUS_ACCESS_VIOLATION
        let kernel_ptr = 0xFFFF_8000_0000_0000 as *const u8;
        if validate_user_slice(kernel_ptr, 64) != Err(STATUS_ACCESS_VIOLATION) {
            crate::serial_write_str("[KNT-PANIC] Kernel pointer did not return ACCESS_VIOLATION!\r\n");
            return false;
        }

        // 1.3 Pointer wrapping/overflow must return STATUS_ACCESS_VIOLATION
        let overflow_ptr = (USER_SPACE_END - 32) as *const u8;
        if validate_user_slice(overflow_ptr, 128) != Err(STATUS_ACCESS_VIOLATION) {
            crate::serial_write_str("[KNT-PANIC] Overflowing buffer did not return ACCESS_VIOLATION!\r\n");
            return false;
        }

        // 1.4 Valid userland range must pass
        let valid_user_addr = (USER_SPACE_START + 0x1000) as *const u8;
        if validate_user_slice(valid_user_addr, 64).is_err() {
            crate::serial_write_str("[KNT-PANIC] Valid userland slice failed validation!\r\n");
            return false;
        }

        crate::serial_write_str("[KNT-SYSCALL-VALIDATE-PASS] Userland memory bounds and access violation guards verified\r\n");
    }

    // --- TEST 2: Dispatch Routing and Service Table ---
    {
        // Test NT_TEST_SYS
        let res = dispatch(NT_TEST_SYS, 0, 0, 0, 0);
        if res != STATUS_SUCCESS {
            crate::serial_write_str("[KNT-PANIC] NtTestSys dispatch failed!\r\n");
            return false;
        }

        // Test Unimplemented Syscall
        let res_unimpl = dispatch(0x0FFF, 0, 0, 0, 0);
        if res_unimpl != STATUS_NOT_IMPLEMENTED {
            crate::serial_write_str("[KNT-PANIC] Unknown syscall did not return STATUS_NOT_IMPLEMENTED!\r\n");
            return false;
        }

        crate::serial_write_str("[KNT-SYSCALL-TABLE-PASS] NT syscall table dispatch routing verified\r\n");
    }

    // --- TEST 3: Core Handlers (NtWriteFile, NtAllocateVirtualMemory, NtYieldExecution) ---
    {
        // 3.1 NtYieldExecution
        if dispatch(NT_YIELD_EXECUTION, 0, 0, 0, 0) != STATUS_SUCCESS {
            crate::serial_write_str("[KNT-PANIC] NtYieldExecution failed!\r\n");
            return false;
        }

        // 3.2 NtWriteFile with access violation protection
        let bad_ptr = 0xFFFF_FFFF_8000_0000u64;
        let av_res = dispatch(NT_WRITE_FILE, NT_CONSOLE_HANDLE, 0, bad_ptr, 32);
        if av_res != STATUS_ACCESS_VIOLATION {
            crate::serial_write_str("[KNT-PANIC] NtWriteFile did not guard against invalid kernel pointer!\r\n");
            return false;
        }

        // 3.3 NtWriteFile with valid userland buffer
        let test_buf_addr = (USER_SPACE_START + 0x2000) as *mut u8;
        let test_msg = b"[KNT-SYSCALL-DISPATCH] Verified NtWriteFile userland output\r\n";
        unsafe {
            core::ptr::copy_nonoverlapping(test_msg.as_ptr(), test_buf_addr, test_msg.len());
        }
        let write_res = dispatch(
            NT_WRITE_FILE,
            NT_CONSOLE_HANDLE,
            0,
            test_buf_addr as u64,
            test_msg.len() as u64,
        );
        if write_res != STATUS_SUCCESS {
            crate::serial_write_str("[KNT-PANIC] NtWriteFile failed with valid buffer!\r\n");
            return false;
        }

        // 3.4 NtAllocateVirtualMemory
        let base_addr_slot = (USER_SPACE_START + 0x3000) as *mut usize;
        let size_slot = (USER_SPACE_START + 0x3008) as *mut usize;
        unsafe {
            *base_addr_slot = 0;
            *size_slot = 4096;
        }
        let alloc_res = dispatch(
            NT_ALLOCATE_VIRTUAL_MEMORY,
            NT_CURRENT_PROCESS,
            base_addr_slot as u64,
            size_slot as u64,
            0,
        );
        if alloc_res != STATUS_SUCCESS {
            crate::serial_write_str("[KNT-PANIC] NtAllocateVirtualMemory failed!\r\n");
            return false;
        }
        unsafe {
            if *base_addr_slot == 0 || *size_slot != 4096 {
                crate::serial_write_str("[KNT-PANIC] NtAllocateVirtualMemory did not return allocated address/size!\r\n");
                return false;
            }
        }

        // 3.5 NtTerminateProcess
        let term_res = dispatch(NT_TERMINATE_PROCESS, NT_CURRENT_PROCESS, 0, 0, 0);
        if term_res != STATUS_SUCCESS {
            crate::serial_write_str("[KNT-PANIC] NtTerminateProcess failed!\r\n");
            return false;
        }

        crate::serial_write_str("[KNT-SYSCALL-HANDLERS-PASS] Core NT handlers (write, alloc, yield, term) verified\r\n");
    }

    crate::serial_write_str("[KNT-SYSCALL-TESTS-PASS] All userland syscall engine checks passed\r\n");
    true
}
