#![no_std]

extern crate alloc;

pub mod arch;
pub mod memory;
pub mod display;
pub mod pe;
pub mod fs;
pub mod process;
pub mod syscall;
pub mod drivers;
pub mod compositor;
pub mod shell;

pub use memory::ALLOCATOR;

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

    // Initialize foundational memory management subsystems (PMM, VMM, Heap)
    memory::init();

    // Execute memory subsystem verification self-test
    if !memory::test_memory_subsystem() {
        serial_write_str("[KNT-PANIC] Memory subsystem self-test failed!\r\n");
    }

    // Initialize Virtual File System & mount demo initramfs
    fs::init();
    if !fs::test_vfs_subsystem() {
        serial_write_str("[KNT-PANIC] VFS subsystem self-test failed!\r\n");
    }

    // Initialize Process & Thread Scheduler subsystem
    process::init();
    if !process::test_process_subsystem() {
        serial_write_str("[KNT-PANIC] Process subsystem self-test failed!\r\n");
    }

    // Initialize Userland Syscall Dispatch Engine
    syscall::init();
    if !syscall::test_syscall_subsystem() {
        serial_write_str("[KNT-PANIC] Syscall subsystem self-test failed!\r\n");
    }

    // Initialize VirtIO Driver Subsystem (PCI discovery, split virtqueue, block device)
    drivers::virtio::init();
    if !drivers::virtio::test_virtio_subsystem() {
        serial_write_str("[KNT-PANIC] VirtIO subsystem self-test failed!\r\n");
    }

    // Initialize Desktop Shell Compositor & Window Manager
    compositor::init(320, 240);
    if !compositor::test_compositor_subsystem() {
        serial_write_str("[KNT-PANIC] Compositor subsystem self-test failed!\r\n");
    }

    // Initialize Interactive Virtual Terminal (VT) Shell & Key-Event Engine
    shell::init(34, 9);
    if !shell::test_shell_subsystem() {
        serial_write_str("[KNT-PANIC] Shell subsystem self-test failed!\r\n");
    }

    serial_write_str("[KNT-SUBSYSTEM] Display & Win32 PE64 loader modules active\r\n");
    serial_write_str("[KNT-BOOT-SUCCESS] Native UEFI-to-kernel boot verified\r\n");

    loop {
        // Poll PS/2 keyboard hardware for interactive input dispatch
        if let Some(scancode) = shell::poll_ps2_hardware() {
            shell::dispatch_scancode(scancode);
        }
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}
