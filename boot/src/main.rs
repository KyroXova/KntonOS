#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[no_mangle]
pub extern "win64" fn efi_main(
    _image_handle: *const core::ffi::c_void,
    _system_table: *const core::ffi::c_void,
) -> usize {
    kntonk::serial_init();
    kntonk::serial_write_str("\r\n[KNT-UEFI-START] KntonOS UEFI Bootloader (kboot.efi) initialized\r\n");
    kntonk::serial_write_str("[KNT-UEFI] Transferring execution to KntonOS micro-kernel entry\r\n");

    // Transfer control to independent kernel entry
    kntonk::kmain();
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    kntonk::serial_write_str("\r\n[KNT-PANIC] Bootloader panic occurred\r\n");
    loop {
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}
