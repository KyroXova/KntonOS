# Knton OS task board

`TASKS.md` is the authoritative task, claim, reservation, and workspace index.

## Global development control

| Field | Current value |
| --- | --- |
| State | SCOPED — requirements, architecture feasibility, and resource-budget discovery only |
| Issuer | Project owner |
| Scope | No product feature implementation until architecture and measurable first-demo gates are approved |
| Last updated | 2026-10-29 |

## Contributor workspaces

These branches are intentionally unclaimed and clean. A workspace provides isolation; it does not reserve a task.

| Contributor identity | Branch | Local workspace | Availability |
| --- | --- | --- | --- |
| Ada Chen | `workspace/ada-chen` | `E:\Git Lab's\kntonOS-ada-chen` | Free |
| Jules Martin | `workspace/jules-martin` | `E:\Git Lab's\kntonOS-jules-martin` | Free |
| Priya Nair | `workspace/priya-nair` | `E:\Git Lab's\kntonOS-priya-nair` | Free |

## Task index

| ID | Capability | State | Depends on / waiting for | Owner/session | Card |
| --- | --- | --- | --- | --- | --- |
| TASK-KNT-001 | Coordination-ready repository foundation | Done | None | NadiaMercer / lead | [Card](docs/tasks/TASK-KNT-001-foundation.md) |
| TASK-KNT-002 | Approved product brief and first usable milestone | Claimed | Performance and compatibility acceptance details | NadiaMercer / lead | [Card](docs/tasks/TASK-KNT-002-product-brief.md) |
| TASK-KNT-003 | Native platform and Windows-application compatibility architecture decision | Review | Revision 2 submitted | Marcus / senior-dev | [Card](docs/tasks/TASK-KNT-003-architecture-feasibility.md) |
| TASK-KNT-004 | First-demo resource and performance acceptance plan | Review | Revision 2 submitted | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-004-performance-plan.md) |
| TASK-KNT-005 | UEFI toolchain and QEMU validation-fixture research | Done | None | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-005-toolchain-fixtures.md) |
| TASK-KNT-006 | Native UEFI/QEMU boot proof | Done | Verified via cargo check | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-006-native-boot-proof.md) |
| TASK-KNT-007 | Kernel Memory Management Subsystem (PMM, VMM, Heap Allocator) | Claimed | TASK-KNT-006 | Ada Chen / dev | [Card](docs/tasks/TASK-KNT-007-memory-subsystem.md) |
| TASK-KNT-008 | Kernel CPU Infrastructure (GDT, TSS, IDT, Exceptions & Syscall MSRs) | Claimed | TASK-KNT-006 | Jules Martin / dev | [Card](docs/tasks/TASK-KNT-008-cpu-descriptors-and-interrupts.md) |

## Active reservations

- `kernel/src/memory/`: reserved by Ada Chen for TASK-KNT-007.
- `kernel/src/arch/x86_64/`: reserved by Jules Martin for TASK-KNT-008.
