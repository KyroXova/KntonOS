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
| Ada Chen | `workspace/ada-chen` | `E:\Git Lab's\kntonOS-ada-chen` | Active (TASK-KNT-011) |
| Jules Martin | `workspace/jules-martin` | `E:\Git Lab's\kntonOS-jules-martin` | Active (TASK-KNT-010) |
| Priya Nair | `workspace/priya-nair` | `E:\Git Lab's\kntonOS-priya-nair` | Active (TASK-KNT-009) |

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
| TASK-KNT-012 | Desktop Shell Compositor & Window Manager | Claimed | TASK-KNT-006 | Priya Nair / dev | [Card](docs/tasks/TASK-KNT-012-compositor-window-manager.md) |

## Active reservations

- `kernel/src/process/`: reserved by Jules Martin for TASK-KNT-010.
- `kernel/src/compositor/`: reserved by Priya Nair for TASK-KNT-012.
