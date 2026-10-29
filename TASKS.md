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
| Jules Martin | `workspace/jules-martin` | `E:\Git Lab's\kntonOS-jules-martin` | Active (TASK-KNT-037) |
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
| TASK-KNT-037 | VirtIO GPU 2D/3D Hardware Acceleration & Scanout Display Driver | Claimed | TASK-KNT-013 | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-037-virtio-gpu.md) |
| TASK-KNT-038 | VFS Unified Inode Cache, Lock Manager, & ACL Access Controls | Claimed | TASK-KNT-009 | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-038-vfs-acl.md) |
| TASK-KNT-039 | Hardware RNG (RDRAND/RDSEED) & ChaCha20 Cryptographic Entropy Pool | Claimed | TASK-KNT-033 | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-039-csprng.md) |

## Active reservations

- `kernel/src/drivers/gpu/`: reserved by Jules Martin for TASK-KNT-037.
- `kernel/src/fs/acl.rs`, `kernel/src/fs/cache.rs`: reserved by Priya Nair for TASK-KNT-038.
- `kernel/src/crypto/rng.rs`, `kernel/src/crypto/chacha20.rs`: reserved by Ada Chen for TASK-KNT-039.
