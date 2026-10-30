//! Userland memory slice and pointer validation for syscall safety.
//! Guarantees that any pointer passed across the user/kernel boundary resides

pub const USER_SPACE_START: usize = 0x0000_0000_0001_0000; // 64 KiB guard page
pub const USER_SPACE_END: usize = 0x0000_7FFF_FFFF_FFFF;   // 128 TiB canonical userland boundary

pub const STATUS_SUCCESS: u32 = 0x0000_0000;
pub const STATUS_NOT_IMPLEMENTED: u32 = 0xC000_0002;
pub const STATUS_ACCESS_VIOLATION: u32 = 0xC000_0005;
pub const STATUS_INVALID_HANDLE: u32 = 0xC000_0008;
pub const STATUS_INVALID_PARAMETER: u32 = 0xC000_000D;
pub const STATUS_BUFFER_TOO_SMALL: u32 = 0xC000_0023;

/// Validates whether a given virtual address falls within userland boundaries.
#[inline(always)]
pub fn is_user_address(addr: usize) -> bool {
    addr >= USER_SPACE_START && addr <= USER_SPACE_END
}

/// Validates that a user-provided pointer is non-null, aligned, and within userland space.
pub fn validate_user_ptr<T>(ptr: *const T) -> Result<*const T, u32> {
    if ptr.is_null() {
        return Err(STATUS_ACCESS_VIOLATION);
    }

    let addr = ptr as usize;
    let align = core::mem::align_of::<T>();
    let size = core::mem::size_of::<T>();

    if (addr & (align - 1)) != 0 {
        return Err(STATUS_ACCESS_VIOLATION);
    }

    let end_addr = match addr.checked_add(size) {
        Some(end) => end,
        None => return Err(STATUS_ACCESS_VIOLATION),
    };

    if addr < USER_SPACE_START || end_addr > USER_SPACE_END {
        return Err(STATUS_ACCESS_VIOLATION);
    }

    Ok(ptr)
}

/// Validates that a user-provided mutable pointer is non-null, aligned, and in userland.
pub fn validate_user_mut_ptr<T>(ptr: *mut T) -> Result<*mut T, u32> {
    validate_user_ptr(ptr as *const T).map(|p| p as *mut T)
}

/// Validates a contiguous userland buffer slice.
/// Returns a safe byte slice reference or STATUS_ACCESS_VIOLATION.
pub fn validate_user_slice<'a>(ptr: *const u8, len: usize) -> Result<&'a [u8], u32> {
    if len == 0 {
        return Ok(&[]);
    }

    if ptr.is_null() {
        return Err(STATUS_ACCESS_VIOLATION);
    }

    let addr = ptr as usize;
    let end_addr = match addr.checked_add(len) {
        Some(end) => end,
        None => return Err(STATUS_ACCESS_VIOLATION),
    };

    if addr < USER_SPACE_START || end_addr > USER_SPACE_END {
        return Err(STATUS_ACCESS_VIOLATION);
    }

    unsafe { Ok(core::slice::from_raw_parts(ptr, len)) }
}

/// Validates a contiguous mutable userland buffer slice.
/// Returns a safe mutable byte slice reference or STATUS_ACCESS_VIOLATION.
pub fn validate_user_mut_slice<'a>(ptr: *mut u8, len: usize) -> Result<&'a mut [u8], u32> {
    if len == 0 {
        return Ok(&mut []);
    }

    if ptr.is_null() {
        return Err(STATUS_ACCESS_VIOLATION);
    }

    let addr = ptr as usize;
    let end_addr = match addr.checked_add(len) {
        Some(end) => end,
        None => return Err(STATUS_ACCESS_VIOLATION),
    };

    if addr < USER_SPACE_START || end_addr > USER_SPACE_END {
        return Err(STATUS_ACCESS_VIOLATION);
    }

    unsafe { Ok(core::slice::from_raw_parts_mut(ptr, len)) }
}
