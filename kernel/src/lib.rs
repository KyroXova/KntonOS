#![no_std]

pub mod arch;

// 16550A UART COM1 port
pub const COM1_PORT: u16 = 0x3F8;

#[inline(always)]
pub unsafe fn outb(port: u16, val: u8) {
    core::arch::asm!(
        "out dx, al",
        in("dx") port,
        in("al") val,
        options(nomem, nostack, preserves_flags)
    );
}

#[inline(always)]
pub unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    core::arch::asm!(
        "in al, dx",
        out("al") val,
        in("dx") port,
        options(nomem, nostack, preserves_flags)
    );
    val
}

pub fn serial_init() {
    unsafe {
        outb(COM1_PORT + 1, 0x00); // Disable interrupts
        outb(COM1_PORT + 3, 0x80); // Enable DLAB (set baud rate divisor)
        outb(COM1_PORT + 0, 0x01); // Set divisor to 1 (115200 baud) lo byte
        outb(COM1_PORT + 1, 0x00); //                               hi byte
        outb(COM1_PORT + 3, 0x03); // 8 bits, no parity, one stop bit
        outb(COM1_PORT + 2, 0xC7); // Enable FIFO, clear them, with 14-byte threshold
        outb(COM1_PORT + 4, 0x0B); // IRQs enabled, RTS/DSR set
    }
}

pub fn serial_write_byte(b: u8) {
    unsafe {
        // Wait for Line Status Register (0x3FD) empty transmitter holding register (bit 5)
        while (inb(COM1_PORT + 5) & 0x20) == 0 {
            core::arch::asm!("pause", options(nomem, nostack, preserves_flags));
        }
        outb(COM1_PORT, b);
    }
}

pub fn serial_write_str(s: &str) {
    for b in s.bytes() {
        if b == b'\n' {
            serial_write_byte(b'\r');
        }
        serial_write_byte(b);
    }
}

#[no_mangle]
pub extern "C" fn kmain() -> ! {
    serial_init();
    serial_write_str("\r\n[KNT-KERNEL-INIT] KntonOS micro-kernel bootstrap reached\r\n");

    // Initialize low-level CPU infrastructure (GDT, TSS, IDT, Syscall MSRs)
    arch::init();

    serial_write_str("[KNT-BOOT-SUCCESS] Native UEFI-to-kernel boot verified\r\n");

    loop {
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}
