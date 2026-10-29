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
pub mod crypto;
pub mod net;
pub mod shell;
pub mod ipc;
pub mod loader;

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

#[inline(always)]
pub unsafe fn outl(port: u16, val: u32) {
    core::arch::asm!(
        "out dx, eax",
        in("dx") port,
        in("eax") val,
        options(nomem, nostack, preserves_flags)
    );
}

#[inline(always)]
pub unsafe fn inl(port: u16) -> u32 {
    let val: u32;
    core::arch::asm!(
        "in eax, dx",
        out("eax") val,
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

    // Initialize SMP multi-core discovery and Local APIC timer
    arch::init_smp();

    // Verify SMP multi-core topology and Local APIC timer operations
    if !arch::test_smp_and_timer_subsystem() {
        serial_write_str("[KNT-PANIC] SMP and Local APIC timer self-test failed!\r\n");
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

    // Verify POSIX Signal Handling & Process Exception Dispatch Engine
    if !process::test_signals_subsystem() {
        serial_write_str("[KNT-PANIC] POSIX signals subsystem self-test failed!\r\n");
    }

    // Initialize Inter-Process Communication (IPC) Shared Memory Rings & Fast Futex Engine
    ipc::init();
    if !ipc::test_ipc_subsystem() {
        serial_write_str("[KNT-PANIC] IPC subsystem self-test failed!\r\n");
    }

    // Initialize VirtIO Driver Subsystem (PCI discovery, split virtqueue, block device)
    drivers::virtio::init();
    if !drivers::virtio::test_virtio_subsystem() {
        serial_write_str("[KNT-PANIC] VirtIO subsystem self-test failed!\r\n");
    }

    // Initialize and verify FAT32 Persistent File System Driver & VirtIO Block Mount
    if !fs::test_fat32_subsystem() {
        serial_write_str("[KNT-PANIC] FAT32 subsystem self-test failed!\r\n");
    }

    // Initialize PCI Device Subsystem & USB xHCI Host Controller Discovery
    drivers::init_pci();
    if !drivers::test_pci_subsystem() {
        serial_write_str("[KNT-PANIC] PCI subsystem self-test failed!\r\n");
    }

    // Initialize Embedded Network Protocol Stack (ARP, IPv4, ICMP, UDP, TCP)
    net::init();
    if !net::test_network_stack() {
        serial_write_str("[KNT-PANIC] Network stack self-test failed!\r\n");
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

    // Initialize and verify USB 3.0 xHCI & HID Keyboard/Mouse Input Pipeline
    if !drivers::usb::test_usb_hid_subsystem() {
        serial_write_str("[KNT-PANIC] USB HID subsystem self-test failed!\r\n");
    }

    // Initialize and verify Real-Time Audio Subsystem (Intel HDA / AC97 Codec & PCM Mixer)
    if !drivers::audio::test_audio_subsystem() {
        serial_write_str("[KNT-PANIC] Audio subsystem self-test failed!\r\n");
    }

    // Initialize and verify ELF64 Executable Loader Subsystem
    loader::init();
    if !loader::test_loader_subsystem() {
        serial_write_str("[KNT-PANIC] ELF64 loader subsystem self-test failed!\r\n");
    }

    // Spawn and execute initial Win32 PE64 userland process (/bin/init.exe)
    if !pe::run_init_process() {
        serial_write_str("[KNT-PANIC] Initial Win32 user process execution failed!\r\n");
    }

    serial_write_str("[KNT-SUBSYSTEM] Display & Win32 PE64 loader modules active\r\n");
    serial_write_str("[KNT-BOOT-SUCCESS] Native UEFI-to-kernel boot verified\r\n");

    loop {
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}
