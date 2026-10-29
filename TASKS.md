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
| Ada Chen | `workspace/ada-chen` | `E:\Git Lab's\kntonOS-ada-chen` | Active (TASK-KNT-017) |
| Jules Martin | `workspace/jules-martin` | `E:\Git Lab's\kntonOS-jules-martin` | Active (TASK-KNT-014) |
| Priya Nair | `workspace/priya-nair` | `E:\Git Lab's\kntonOS-priya-nair` | Active (TASK-KNT-016) |

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
| TASK-KNT-014 | Win32 PE64 Userland Process Loader & User Execution | Claimed | TASK-KNT-010, TASK-KNT-011 | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-014-pe-process-loader.md) |
| TASK-KNT-015 | VirtIO Network Device Driver (virtio-net) & Packet Ring Buffer | Done | Verified via QEMU harness | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-015-virtio-net.md) |
| TASK-KNT-016 | Interactive Virtual Terminal (VT) Shell & Key-Event Engine | Claimed | TASK-KNT-012 | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-016-terminal-shell.md) |
| TASK-KNT-017 | Embedded Network Protocol Stack (ARP, IPv4, ICMP, UDP) | Claimed | TASK-KNT-015 | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-017-network-stack.md) |

## Active reservations

- `kernel/src/pe/` & `kernel/src/process/`: reserved by Jules Martin for TASK-KNT-014.
- `kernel/src/shell/`: reserved by Priya Nair for TASK-KNT-016.
- `kernel/src/net/`: reserved by Ada Chen for TASK-KNT-017.
