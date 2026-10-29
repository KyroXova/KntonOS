# TASK-KNT-007 — Kernel Memory Management Subsystem (PMM, VMM, Heap Allocator)

| Field | Value |
| --- | --- |
| Status | Ready |
| Requirement/source | `docs/architecture/ADR-001-native-platform-and-compatibility.md` Slice 1 |
| Milestone | Functional Physical Memory Manager, 4-level PML4 Virtual Memory Manager, and Global Heap Allocator enabling `alloc` structures (`Vec`, `Box`, `String`). |
| Depends on | TASK-KNT-006 |
| Owner/session | Ada Chen / dev |
| Components | `kernel/src/memory/` |
| Review | Marcus verifies memory safety, alignment invariants, and allocation tests. |

## Goal and scope

Implement the foundational physical and virtual memory management subsystems for `kntonk`:
1. **Frame Allocator (PMM)**: Bitmap or bump frame allocator for allocating/deallocating 4KiB physical frames.
2. **Page Table Manager (VMM)**: PML4, PDPT, PD, and PT structures; recursive or offset page table mapping; mapping/unmapping virtual pages with flags (PRESENT, WRITABLE, NO_EXECUTE).
3. **Kernel Heap**: Minimal memory allocator implementing `core::alloc::GlobalAlloc` providing a fixed initial heap (e.g. 1MB - 16MB) to enable Rust `alloc` collections.

## Acceptance

- [ ] PMM accurately tracks free and reserved physical frames.
- [ ] VMM supports mapping virtual addresses to physical frames with appropriate page flags.
- [ ] `GlobalAlloc` is registered, allowing `Vec`, `Box`, and `String` allocations without kernel panic.
- [ ] Compiles cleanly with `cargo check` under `x86_64-unknown-none`.
- [ ] Atomic commits posted to AgentIRC `#dev` and evidence in `#reports`.
