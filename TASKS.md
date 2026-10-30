# Knton OS task board

`TASKS.md` is the authoritative task, claim, reservation, and workspace index.

## Global development control

| Field | Current value |
| --- | --- |
| State | ACTIVE IMPLEMENTATION — Parallel kernel subsystems, high-frequency atomic commit cadence |
| Issuer | Project owner |
| Scope | Modular micro-kernel subsystems with strict atomic commit process, zero-panic invariants, and continuous QEMU verification |
| Last updated | 2026-10-29 |

## Contributor workspaces

These branches are managed by their respective developers with granular commit logs and clean branch hygiene.

| Contributor identity | Branch | Local workspace | Status |
| --- | --- | --- | --- |
| Ada Chen | `workspace/ada-chen` | `E:\Git Lab's\kntonOS-ada-chen` | Active (TASK-KNT-039) |
| Jules Martin | `workspace/jules-martin` | `E:\Git Lab's\kntonOS-jules-martin` | Active (TASK-KNT-040) |
| Priya Nair | `workspace/priya-nair` | `E:\Git Lab's\kntonOS-priya-nair` | Active (TASK-KNT-038) |

## Task index

| ID | Capability | State | Depends on / waiting for | Owner/session | Card |
| --- | --- | --- | --- | --- | --- |
| TASK-KNT-001 | Coordination-ready repository foundation | Done | None | NadiaMercer / lead | [Card](docs/tasks/TASK-KNT-001-foundation.md) |
| TASK-KNT-002 | Approved product brief and first usable milestone | Claimed | Performance and compatibility acceptance details | NadiaMercer / lead | [Card](docs/tasks/TASK-KNT-002-product-brief.md) |
| TASK-KNT-003 | Native platform and Windows-application compatibility architecture decision | Done | Integrated on main | Marcus / senior-dev | [Card](docs/tasks/TASK-KNT-003-architecture-feasibility.md) |
| TASK-KNT-004 | First-demo resource and performance acceptance plan | Done | Integrated on main | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-004-performance-plan.md) |
| TASK-KNT-005 | UEFI toolchain and QEMU validation-fixture research | Done | None | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-005-toolchain-fixtures.md) |
| TASK-KNT-006 | Native UEFI/QEMU boot proof | Done | Verified via QEMU harness | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-006-native-boot-proof.md) |
| TASK-KNT-007 | Kernel Memory Management Subsystem (PMM, VMM, Heap Allocator) | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-007-memory-subsystem.md) |
| TASK-KNT-008 | Kernel CPU Infrastructure (GDT, TSS, IDT, Exceptions & Syscall MSRs) | Done | Verified via QEMU harness | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-008-cpu-descriptors-and-interrupts.md) |
| TASK-KNT-009 | Virtual File System (VFS) and Memory-Mapped Initramfs | Done | Verified via QEMU harness | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-009-vfs-and-ramdisk.md) |
| TASK-KNT-010 | Preemptive Process and Thread Scheduler (TCB, PCB, Context Switch) | Done | Verified via QEMU harness | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-010-process-scheduler.md) |
| TASK-KNT-011 | Userland Execution & Syscall Dispatch Engine | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-011-syscall-dispatch.md) |
| TASK-KNT-012 | Desktop Shell Compositor & Window Manager | Done | Verified via QEMU harness | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-012-compositor-window-manager.md) |
| TASK-KNT-013 | VirtIO Block and Storage Device Drivers | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-013-virtio-block.md) |
| TASK-KNT-014 | Win32 PE64 Userland Process Loader & User Execution | Done | Verified via QEMU harness | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-014-pe-process-loader.md) |
| TASK-KNT-015 | VirtIO Network Device Driver (virtio-net) & Packet Ring Buffer | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-015-virtio-net.md) |
| TASK-KNT-016 | Interactive Virtual Terminal (VT) Shell & Key-Event Engine | Done | Verified via QEMU harness | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-016-terminal-shell.md) |
| TASK-KNT-017 | Embedded Network Protocol Stack (ARP, IPv4, ICMP, UDP) | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-017-network-stack.md) |
| TASK-KNT-018 | SMP Multi-Core Discovery and Local APIC Timer Subsystem | Done | Verified via QEMU harness | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-018-smp-apic.md) |
| TASK-KNT-019 | Transmission Control Protocol (TCP) State Machine and Stream Sockets | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-019-tcp-stack.md) |
| TASK-KNT-020 | PCI Device Subsystem and USB xHCI Host Controller Discovery | Done | Verified via QEMU harness | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-020-pci-xhci.md) |
| TASK-KNT-021 | Domain Name System (DNS) Resolver & HTTP/1.1 Micro-Client | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-021-dns-http.md) |
| TASK-KNT-022 | IPC Zero-Copy Shared Memory Rings & Fast Futex Synchronization | Done | Verified via QEMU harness | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-022-ipc-futex.md) |
| TASK-KNT-023 | USB 3.0 xHCI Enumeration & HID Keyboard/Mouse Input Pipeline | Done | Verified via QEMU harness | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-023-usb-hid.md) |
| TASK-KNT-024 | DHCP IPv4 Lease Client & Network Auto-Configuration | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-024-dhcp-client.md) |
| TASK-KNT-025 | Desktop GUI Controls & Acrylic Backdrop Surface Renderer | Done | Verified via QEMU harness | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-025-gui-controls.md) |
| TASK-KNT-026 | ELF64 Executable Loader & Userland Execution Subsystem | Done | Verified via QEMU harness | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-026-elf64-loader.md) |
| TASK-KNT-027 | BSD / WinSock Sockets Abstraction Layer & Event Readiness Polling | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-027-socket-layer.md) |
| TASK-KNT-028 | VT Shell Pipelines, I/O Redirection, and Process Job Control | Done | Verified via QEMU harness | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-028-shell-pipelines.md) |
| TASK-KNT-029 | POSIX Signal Handling & Process Exception Dispatch Engine | Done | Verified via QEMU harness | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-029-signals.md) |
| TASK-KNT-030 | Embedded HTTP/1.1 Micro-Server & Kernel REST Telemetry API | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-030-http-server.md) |
| TASK-KNT-031 | FAT32 Persistent File System Driver & VirtIO Block Mount Integration | Done | Verified via QEMU harness | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-031-fat32.md) |
| TASK-KNT-032 | Dynamic Linker & Shared Library Loader with GOT/PLT Relocations | Done | Verified via QEMU harness | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-032-dynlink.md) |
| TASK-KNT-033 | TLS 1.3 Cryptographic Handshake & AES-128-GCM AEAD Record Layer | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-033-tls.md) |
| TASK-KNT-034 | Desktop Taskbar, Clock Widget, Application Launcher, & System Tray | Done | Verified via QEMU harness | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-034-taskbar.md) |
| TASK-KNT-035 | ACPI Power Management Subsystem (FADT/DSDT S3/S5, CPU Throttling, Reset & Shutdown) | Done | Verified via QEMU harness | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-035-power.md) |
| TASK-KNT-036 | Real-Time Audio Subsystem (Intel HDA / AC97 Codec Driver & PCM Audio Streamer) | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-036-audio.md) |
| TASK-KNT-037 | VirtIO GPU 2D/3D Hardware Acceleration & Scanout Display Driver | Done | Verified via QEMU harness | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-037-virtio-gpu.md) |
| TASK-KNT-038 | VFS Unified Inode Cache, Lock Manager, & ACL Access Controls | Done | Verified via QEMU harness | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-038-vfs-acl.md) |
| TASK-KNT-039 | Hardware RNG (RDRAND/RDSEED) & ChaCha20 Cryptographic Entropy Pool | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-039-csprng.md) |
| TASK-KNT-040 | Software 3D Graphics Pipeline & Barycentric Triangle Rasterizer | Done | Verified via QEMU harness | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-040-rasterizer.md) |
| TASK-KNT-041 | AHCI SATA Persistent Storage Controller Driver & Port Command Queue Engine | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-041-ahci-sata.md) |
| TASK-KNT-042 | Virtual Memory Demand Paging, Copy-on-Write (COW) Fork Engine, & Page Fault Handler | Done | Verified via QEMU harness | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-042-cow-paging.md) |
| TASK-KNT-043 | Unix Domain Sockets (AF_UNIX) & Local IPC Streaming / Datagram Engine | Done | Verified via QEMU harness | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-043-unix-sockets.md) |
| TASK-KNT-044 | USB Mass Storage Class (MSC) Bulk-Only Transport (BOT) Protocol Driver | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-044-usb-msc.md) |
| TASK-KNT-045 | kpkg Native Package Manifest Parser, Merkle Tree Content-Addressed Storage (CAS), & Zstandard Archive Decompressor | Claimed | TASK-KNT-038 | ArdenKestrel / dev | [Card](docs/tasks/TASK-KNT-045-kpkg-cas.md) |
| TASK-KNT-046 | NVMe (Non-Volatile Memory Express) Storage Controller Driver & Submission/Completion Queue Engine | Done | Verified via QEMU harness | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-046-nvme.md) |
| TASK-KNT-047 | High-Precision Event Timer (HPET) & Real-Time Monotonic Nanosecond Clock Subsystem | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-047-hpet.md) |
| TASK-KNT-048 | Virtual Terminal ANSI 24-bit TrueColor Emulator & PTY Master/Slave Multiplexer | Done | Verified via QEMU harness | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-048-pty-truecolor.md) |
| TASK-KNT-049 | Dynamic Power Governor & Thermal Energy Meter (AMD SMU / Intel RAPL MSRs & Closed-Loop PID Frequency Governor) | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-049-power-governor.md) |
| TASK-KNT-050 | Direct-to-Display Hardware Scanout Engine & Atomic DRM/KMS Modesetting Presentation Layer | Done | Verified via QEMU harness | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-050-drm-modeset.md) |
| TASK-KNT-051 | 6-Axis IMU Sensor Fusion & 1000Hz High-Performance Gamepad Input Pipeline | Done | Verified via QEMU harness | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-051-imu-gamepad.md) |
| TASK-KNT-052 | Transparent Huge Pages (THP 2MB) & AMD 3D V-Cache NUMA Topology Awareness | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-052-thp-numa.md) |
| TASK-KNT-053 | Hardware Variable Refresh Rate (VRR / FreeSync) & Dynamic VBLANK Presentation Timing | Done | Verified via QEMU harness | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-053-vrr-timing.md) |
| TASK-KNT-054 | Native Hardware Display Scanout Rotation & Panel Orientation Subsystem (Portrait Panel Zero-Overhead Scanout) | Done | Verified via QEMU harness | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-054-display-rotation.md) |
| TASK-KNT-055 | High-Throughput Zero-Copy Shared-Memory IPC Ring (knton-ipc-ring) & Fast UMWAIT/TPAUSE Synchronization | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-055-ipc-ring.md) |
| TASK-KNT-056 | PCIe Function-Level Reset (FLR) & GPU Timeout Detection and Recovery (TDR) Subsystem | Done | Verified via QEMU harness | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-056-pci-tdr.md) |
| TASK-KNT-057 | State-Externalized Dual-Ring Resilient Service Partitions & Sub-15ms Crash Recovery Engine | Done | Verified via QEMU harness | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-057-service-resilience.md) |
| TASK-KNT-058 | Wayland Client Protocol Compositor Backend (wl_surface, wl_buffer, wl_shm) & Sub-Surface Synchronization | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-058-wayland-compositor.md) |
| TASK-KNT-059 | Dynamic Core Parking, C-State Governor, & Energy-Performance Bias (EPB / EPP) Manager | Done | Verified via QEMU harness | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-059-core-parking.md) |
| TASK-KNT-060 | Zero-Latency DMA Audio Ring Buffer & Multi-Stream Real-Time Mixer Engine | Done | Verified via QEMU harness | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-060-dma-audio-mixer.md) |
| TASK-KNT-061 | Memory-Mapped I/O (MMIO) Watchpoint & Kernel Hypervisor-Assisted Debug Trap Engine | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-061-mmio-debug-traps.md) |
| TASK-KNT-062 | In-Game Quick-Access Hardware Plane Overlay & Performance Telemetry HUD | Done | Verified via QEMU harness | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-062-overlay-hud.md) |
| TASK-KNT-063 | High-Performance S0i3 / S3 Instant-Resume Freeze & Wakeup Controller | Done | Verified via QEMU harness | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-063-suspend-resume.md) |
| TASK-KNT-064 | Zero-Overhead Kernel Trace Ring Buffer & Performance Profiling Event Engine | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-064-perf-events.md) |
| TASK-KNT-065 | eBPF-Style Bytecode In-Kernel Packet & Event Filter Engine | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-065-bpf-filter.md) |
| TASK-KNT-066 | Kernel-Direct Hardware Gamepad Haptic / Dual-Rumble Driver & Waveform Synthesizer | Done | Verified via QEMU harness | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-066-gamepad-haptics.md) |
| TASK-KNT-067 | High-Speed NVMe Overlapped Async I/O (io_uring Style) Completion Engine | Done | Verified via QEMU harness | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-067-nvme-uring.md) |
| TASK-KNT-068 | Unified Handheld PC Thermal & Fan Profile Controller (Multi-Point Curves & Acoustic Pacing) | Done | Verified via QEMU harness | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-068-thermal-fan.md) |
| TASK-KNT-069 | Hardware-Accelerated Frame Capture & Direct DRM Dumb-Buffer Video Streamer Engine | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-069-frame-capture.md) |
| TASK-KNT-070 | Multi-Queue Network Packet Transmission & Zero-Copy DMA RX/TX Ring Engine | Done | Verified via QEMU harness | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-070-net-multiqueue.md) |
| TASK-KNT-071 | Direct3D / Vulkan-Style Command Buffer Recording & Kernel Hardware Queue Submission Engine | Done | Verified via QEMU harness | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-071-gpu-cmdbuf.md) |
| TASK-KNT-072 | Direct-to-Disk Kernel Hibernation (S4 State) Memory Dump & LZ4 Fast-Restoration Engine | Done | Verified via QEMU harness | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-072-hibernate-s4.md) |
| TASK-KNT-073 | Kernel Dynamic Instrumentation Probes (kprobes / ftrace Style) & Instruction Detour Engine | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-073-kprobes-ftrace.md) |
| TASK-KNT-074 | Unified Handheld PC Ambient Light Sensor (ALS) & HDR Auto-Brightness / Contrast Governor | Done | Verified via QEMU harness | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-074-als-hdr.md) |
| TASK-KNT-075 | Real-Time Kernel Audio Spatializer & Binaural HRTF 3D Positional Audio DSP Pipeline | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-075-audio-spatializer.md) |
| TASK-KNT-076 | Kernel-Direct PCIe Hot-Plug Event Daemon & External eGPU Hot-Connect Controller | Done | Verified via QEMU harness | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-076-pcie-egpu.md) |
| TASK-KNT-077 | Unified Handheld PC Battery Gauge, Coulomb Counter, & Smart Charging State Machine | Done | Verified via QEMU harness | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-077-battery-gauge.md) |
| TASK-KNT-078 | Zero-Latency Kernel-Bypassed Input Polling Ring & Micro-Jitter Smoothing Filter | Done | Verified via QEMU harness | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-078-input-ring.md) |
| TASK-KNT-079 | Dynamic VMM Kernel Memory Compactor & Transparent Page Defragmentation Engine | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-079-memory-compactor.md) |
| TASK-KNT-080 | Unified Handheld PC Gyro Aiming & Motion Assist Dynamic Angle Governor | Done | Verified via QEMU harness | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-080-gyro-aiming.md) |
| TASK-KNT-081 | Zero-Copy Direct Storage Direct3D 12 Compressed GpuTexture Streaming Pipeline | Done | Verified via QEMU harness | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-081-direct-storage.md) |
| TASK-KNT-082 | Real-Time Game Audio Equalizer (Parametric 10-Band EQ & Dynamic Bass Booster) | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-082-audio-eq.md) |
| TASK-KNT-083 | Unified Handheld PC Backlight PWM Modulator & Flicker-Free DC Dimming Governor | Done | Verified via QEMU harness | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-083-backlight-pwm.md) |
| TASK-KNT-084 | Zero-Copy GPU Asynchronous Compute Dispatch & Shader Execution Fence Engine | Done | Verified via QEMU harness | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-084-gpu-compute.md) |
| TASK-KNT-085 | Micro-Kernel Lock-Free Ring Buffer Object Pool & Slab Cache Allocator | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-085-slab-allocator.md) |
| TASK-KNT-086 | Unified Handheld PC Multi-Zone Haptic Actuator & Resonant Frequency Tracking Governor | Done | Verified via QEMU harness | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-086-lra-haptics.md) |
| TASK-KNT-087 | Kernel-Direct NVMe ZNS (Zoned Namespaces) & Copy Offload Engine | Done | Verified via QEMU harness | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-087-nvme-zns.md) |
| TASK-KNT-088 | High-Frequency Kernel High-Resolution Timers (hrtimer) & Red-Black Tree Wheel Engine | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-088-hrtimer.md) |
| TASK-KNT-089 | Direct3D 12 Dynamic Constant Buffer Pool & Ring Allocator Engine | Done | Verified via QEMU harness | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-089-cbv-ring.md) |
| TASK-KNT-090 | Unified Handheld PC USB-C DisplayPort Alternate Mode & Hot-Plug Modeset Daemon | Done | Verified via QEMU harness | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-090-dp-altmode.md) |
| TASK-KNT-091 | Zero-Copy Shared-Memory Audio Client Protocol & Real-Time Waveform Mixer Ring | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-091-audio-client.md) |
| TASK-KNT-092 | Unified Handheld PC Capacitive Touch Controller & Multi-Finger Gesture Engine | Done | Verified via QEMU harness | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-092-touch-gesture.md) |
| TASK-KNT-093 | Direct3D 12 Raytracing Acceleration Structure (BVH) Memory Manager & DispatchRays Engine | Done | Verified via QEMU harness | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-093-dxr-bvh.md) |
| TASK-KNT-094 | Unified Kernel Dynamic Frequency Scaling (DVFS) & Micro-Voltage Stepping Governor | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-094-dvfs-governor.md) |
| TASK-KNT-095 | Unified Handheld PC Capacitive Touchpad & Virtual Trackball Emulation Engine | Claimed | TASK-KNT-051 | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-095-touchpad-trackball.md) |
| TASK-KNT-096 | Direct3D 12 Variable Rate Shading (VRS Tier 2) & Eye-Tracked Foveated Rasterizer | Claimed | TASK-KNT-071 | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-096-vrs-foveated.md) |
| TASK-KNT-097 | Kernel-Direct Hardware Video Decode & Zero-Copy Presentation Streamer | Claimed | TASK-KNT-069 | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-097-video-decode.md) |

## Active reservations

- `kernel/src/pkg/`: reserved by ArdenKestrel for TASK-KNT-045.
- `kernel/src/drivers/input/touchpad.rs`, `kernel/src/drivers/input/trackball.rs`: reserved by Jules Martin for TASK-KNT-095.
- `kernel/src/gfx/vrs.rs`, `kernel/src/gfx/foveated.rs`: reserved by Priya Nair for TASK-KNT-096.
- `kernel/src/video/decode.rs`, `kernel/src/video/nv12.rs`: reserved by Ada Chen for TASK-KNT-097.
