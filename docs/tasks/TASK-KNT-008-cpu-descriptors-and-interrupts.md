# TASK-KNT-008 — Kernel CPU Infrastructure (GDT, TSS, IDT, Exceptions & Syscall MSRs)

| Field | Value |
| --- | --- |
| Status | Ready |
| Requirement/source | `docs/architecture/ADR-001-native-platform-and-compatibility.md` Slice 1 & Slice 2 |
| Milestone | GDT, TSS with dedicated `#DF` IST stack, IDT with 32 exception handlers, and `syscall`/`sysret` MSR initialization for Ring 3. |
| Depends on | TASK-KNT-006 |
| Owner/session | Jules Martin / dev |
| Components | `kernel/src/arch/x86_64/` |
| Review | Marcus verifies exception safety, IST stack isolation, and MSR initialization. |

## Goal and scope

Implement low-level x86-64 CPU descriptor tables and exception handlers:
1. **GDT & TSS**:
   - Kernel Code (Ring 0, 64-bit), Kernel Data.
   - User Code (Ring 3, 64-bit), User Data.
   - Task State Segment (TSS) with `RSP0` for privilege transitions and IST[0] for Double Fault (`#DF`) stack.
2. **IDT**:
   - Handlers for all 32 standard x86-64 exceptions (Divide Error, Page Fault, General Protection Fault, Double Fault).
   - Formatted serial diagnostic dumps (RIP, CS, RFLAGS, RSP, SS, error code, fault address CR2).
3. **Syscall Setup**:
   - Initialize `IA32_STAR`, `IA32_LSTAR` (entry point `syscall_entry`), and `IA32_FMASK` for fast Ring 3 system calls.

## Acceptance

- [ ] GDT loaded with correct segment selectors.
- [ ] TSS loaded via `ltr` with isolated double-fault stack.
- [ ] IDT loaded via `lidt` with exception gates.
- [ ] `syscall` MSRs configured for Ring 3 transition.
- [ ] Compiles cleanly with `cargo check` under `x86_64-unknown-none`.
- [ ] Atomic commits posted to AgentIRC `#dev` and evidence in `#reports`.
