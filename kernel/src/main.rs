#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    kntonk::kmain();
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    kntonk::serial_write_str("\r\n[KNT-PANIC] Kernel panic occurred\r\n");
    loop {
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}
