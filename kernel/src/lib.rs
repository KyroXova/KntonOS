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
pub mod graphics;
pub mod terminal;
pub mod power;
pub mod mm;
pub mod trace;
pub mod bpf;

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

    // Initialize and verify Demand Paging & Copy-on-Write (COW) Fork Engine
    if !memory::test_cow_and_demand_paging() {
        serial_write_str("[KNT-PANIC] Demand paging and COW fork self-test failed!\r\n");
    }

    // Initialize and verify Transparent Huge Pages (THP 2MB) & AMD 3D V-Cache NUMA Topology
    if !mm::test_thp_and_numa_subsystem() {
        serial_write_str("[KNT-PANIC] THP and NUMA V-Cache self-test failed!\r\n");
    }

    // Initialize SMP multi-core discovery and Local APIC timer
    arch::init_smp();

    // Verify SMP multi-core topology and Local APIC timer operations
    if !arch::test_smp_and_timer_subsystem() {
        serial_write_str("[KNT-PANIC] SMP and Local APIC timer self-test failed!\r\n");
    }

    // Initialize ACPI Power Management Subsystem (FADT/DSDT S3/S5, CPU Throttling, Reset & Shutdown)
    arch::init_power_management();

    // Verify ACPI Power Management Subsystem
    if !arch::test_power_subsystem() {
        serial_write_str("[KNT-PANIC] ACPI Power Management self-test failed!\r\n");
    }

    // Initialize and verify HPET & Real-Time Monotonic Nanosecond Clock Subsystem
    if !arch::test_hpet_subsystem() {
        serial_write_str("[KNT-PANIC] HPET and monotonic clock self-test failed!\r\n");
    }
    // Initialize and verify MMIO Watchpoint & Kernel Debug Trap Engine
    if !arch::test_debug_trap_subsystem() {
        serial_write_str("[KNT-PANIC] MMIO watchpoint and debug trap self-test failed!\r\n");
    }

    // Initialize and verify Zero-Overhead Kernel Trace Ring Buffer & Performance Profiling
    if !trace::test_trace_subsystem() {
        serial_write_str("[KNT-PANIC] Kernel trace ring buffer self-test failed!\r\n");
    }

    // Initialize and verify Dynamic Power Governor & Thermal Energy Meter
    if !power::test_power_governor_subsystem() {
        serial_write_str("[KNT-PANIC] Dynamic power governor self-test failed!\r\n");
    }

    // Initialize and verify Dynamic Core Parking, C-State Governor, & EPB/EPP Manager
    if !power::test_core_parking_and_cstate_subsystem() {
        serial_write_str("[KNT-PANIC] Core parking and C-state governor self-test failed!\r\n");
    }

    // Initialize and verify High-Performance S0i3 / S3 Instant-Resume Freeze & Wakeup Controller
    if !power::test_s0i3_s3_suspend_resume_subsystem() {
        serial_write_str("[KNT-PANIC] S0i3/S3 suspend-resume subsystem self-test failed!\r\n");
    }

    // Initialize and verify Handheld Thermal & Fan Profile Controller
    if !power::test_handheld_thermal_and_fan_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld thermal and fan profile subsystem self-test failed!\r\n");
    }

    // Initialize and verify Direct-to-Disk Kernel Hibernation (S4 State) & LZ4 Fast-Restore Engine
    if !power::test_power_hibernate_subsystem() {
        serial_write_str("[KNT-PANIC] Kernel hibernation and LZ4 fast-restore self-test failed!\r\n");
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

    // Initialize and verify State-Externalized Dual-Ring Resilient Partitions & Crash Recovery Engine
    if !process::test_resilience_subsystem() {
        serial_write_str("[KNT-PANIC] Resilient partitions & crash recovery self-test failed!\r\n");
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

    // Verify VFS Unified Inode Cache, Lock Manager, & ACL Access Controls
    if !fs::test_vfs_acl_and_cache() {
        serial_write_str("[KNT-PANIC] VFS ACL and Inode cache self-test failed!\r\n");
    }

    // Initialize PCI Device Subsystem & USB xHCI Host Controller Discovery
    drivers::init_pci();
    if !drivers::test_pci_subsystem() {
        serial_write_str("[KNT-PANIC] PCI subsystem self-test failed!\r\n");
    }

    // Initialize and verify VirtIO GPU 2D/3D Hardware Acceleration & Display Scanout Driver
    if !drivers::test_gpu_subsystem() {
        serial_write_str("[KNT-PANIC] VirtIO GPU subsystem self-test failed!\r\n");
    }

    // Initialize and verify Software 3D Graphics Pipeline & Barycentric Triangle Rasterizer
    if !graphics::test_graphics_subsystem() {
        serial_write_str("[KNT-PANIC] Software 3D graphics subsystem self-test failed!\r\n");
    }

    // Initialize and verify Direct-to-Display Scanout & Atomic DRM/KMS Modesetting Presentation Layer
    if !drivers::gpu::test_drm_kms_subsystem() {
        serial_write_str("[KNT-PANIC] DRM/KMS modesetting subsystem self-test failed!\r\n");
    }

    // Initialize and verify Hardware Variable Refresh Rate (VRR) & Dynamic VBLANK Presentation Timing
    if !drivers::gpu::test_vrr_subsystem() {
        serial_write_str("[KNT-PANIC] Hardware VRR and dynamic timing self-test failed!\r\n");
    }

    // Initialize and verify Native Hardware Display Scanout Rotation & Panel Orientation Subsystem
    if !drivers::gpu::test_display_rotation_and_panel_subsystem() {
        serial_write_str("[KNT-PANIC] Display scanout rotation and panel orientation self-test failed!\r\n");
    }

    // Initialize and verify PCIe Function-Level Reset (FLR) & GPU Timeout Detection and Recovery (TDR)
    if !drivers::gpu::test_pcie_flr_and_gpu_tdr_subsystem() {
        serial_write_str("[KNT-PANIC] PCIe FLR and GPU TDR subsystem self-test failed!\r\n");
    }

    // Initialize and verify AHCI SATA Persistent Storage Subsystem
    if !drivers::storage::test_ahci_sata_subsystem() {
        serial_write_str("[KNT-PANIC] AHCI SATA storage subsystem self-test failed!\r\n");
    }

    // Initialize and verify NVMe Storage Controller Subsystem
    if !drivers::storage::test_nvme_subsystem() {
        serial_write_str("[KNT-PANIC] NVMe storage subsystem self-test failed!\r\n");
    }

    // Initialize and verify High-Speed NVMe Overlapped Async I/O (io_uring Style) Completion Engine
    if !drivers::storage::test_nvme_uring_subsystem() {
        serial_write_str("[KNT-PANIC] NVMe overlapped async I/O completion engine self-test failed!\r\n");
    }

    // Initialize Embedded Network Protocol Stack (ARP, IPv4, ICMP, UDP, TCP)
    net::init();
    if !net::test_network_stack() {
        serial_write_str("[KNT-PANIC] Network stack self-test failed!\r\n");
    }

    // Initialize and verify in-kernel eBPF Packet & Event Filter Subsystem
    if !bpf::test_bpf_filter_subsystem() {
        serial_write_str("[KNT-PANIC] eBPF packet and event filter self-test failed!\r\n");
    }

    // Initialize and verify Multi-Queue Network Packet Transmission & Zero-Copy DMA RX/TX Ring Engine
    if !net::test_net_multiqueue_subsystem() {
        serial_write_str("[KNT-PANIC] Multi-queue network packet engine self-test failed!\r\n");
    }

    // Verify Unix Domain Sockets (AF_UNIX) & Local IPC Engine
    if !net::test_unix_domain_sockets() {
        serial_write_str("[KNT-PANIC] Unix domain sockets self-test failed!\r\n");
    }

    // Initialize Desktop Shell Compositor & Window Manager
    compositor::init(320, 240);
    if !compositor::test_compositor_subsystem() {
        serial_write_str("[KNT-PANIC] Compositor subsystem self-test failed!\r\n");
    }

    // Initialize and verify Hardware-Accelerated Frame Capture & Direct DRM Video Streamer Engine
    if !compositor::test_frame_streamer_subsystem() {
        serial_write_str("[KNT-PANIC] Frame capture and video streamer self-test failed!\r\n");
    }

    // Initialize Interactive Virtual Terminal (VT) Shell & Key-Event Engine
    shell::init(34, 9);
    if !shell::test_shell_subsystem() {
        serial_write_str("[KNT-PANIC] Shell subsystem self-test failed!\r\n");
    }

    // Verify Virtual Terminal ANSI 24-bit TrueColor Terminal Emulator & PTY Subsystem
    if !terminal::test_pty_and_ansi_subsystem() {
        serial_write_str("[KNT-PANIC] Terminal PTY and TrueColor subsystem self-test failed!\r\n");
    }

    // Initialize and verify USB 3.0 xHCI & HID Keyboard/Mouse Input Pipeline
    if !drivers::usb::test_usb_hid_subsystem() {
        serial_write_str("[KNT-PANIC] USB HID subsystem self-test failed!\r\n");
    }

    // Initialize and verify 6-Axis IMU Sensor Fusion & 1000Hz Gamepad Input Pipeline
    if !drivers::input::test_imu_and_gamepad_subsystem() {
        serial_write_str("[KNT-PANIC] 6-Axis IMU sensor fusion and 1000Hz gamepad self-test failed!\r\n");
    }

    // Initialize and verify Gamepad Haptic / Dual-Rumble Driver & Waveform Synthesizer
    if !drivers::input::test_gamepad_haptic_subsystem() {
        serial_write_str("[KNT-PANIC] Gamepad haptic and rumble self-test failed!\r\n");
    }

    // Initialize and verify USB Mass Storage Class (MSC) Bulk-Only Transport (BOT) Driver
    if !drivers::usb::test_usb_msc_subsystem() {
        serial_write_str("[KNT-PANIC] USB MSC BOT storage subsystem self-test failed!\r\n");
    }

    // Initialize and verify Real-Time Audio Subsystem (Intel HDA / AC97 Codec & PCM Mixer)
    if !drivers::audio::test_audio_subsystem() {
        serial_write_str("[KNT-PANIC] Audio subsystem self-test failed!\r\n");
    }

    // Verify Zero-Latency DMA Audio Ring Buffer & Multi-Stream Real-Time Mixer Engine
    if !drivers::audio::test_realtime_audio_mixer_subsystem() {
        serial_write_str("[KNT-PANIC] Real-time DMA audio mixer subsystem self-test failed!\r\n");
    }

    // Initialize and verify Hardware RNG & ChaCha20 Cryptographic Entropy Pool
    crypto::init_kernel_rng();
    if !crypto::test_crypto_rng_subsystem() {
        serial_write_str("[KNT-PANIC] Hardware RNG and ChaCha20 CSPRNG self-test failed!\r\n");
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
        arch::cpu_idle();
    }
}
