# TASK-KNT-010 — Preemptive Process and Thread Scheduler

| Field | Value |
| --- | --- |
| Status | Claimed |
| Requirement/source | `docs/architecture/ADR-001-native-platform-and-compatibility.md` Slice 1 |
| Milestone | Cooperative and preemptive thread execution with context switching and round-robin scheduler. |
| Depends on | TASK-KNT-008 |
| Owner/session | Jules Martin / dev |
| Components | `kernel/src/process/` |
| Review | Marcus verifies register save/restore invariants and zero re-entrance bugs. |

## Goal and scope

Implement the process and thread scheduling subsystem for KntonOS:
1. **Context Switching Engine**:
   - `Context` structure holding callee-saved and scratch registers (`rsp`, `rip`, `rflags`, `rbx`, `rbp`, `r12`-`r15`).
   - Assembly switch routine (`context_switch(prev: *mut Context, next: *const Context)`).
2. **Thread and Process Structures**:
   - `ThreadState` enum (`Ready`, `Running`, `Blocked`, `Terminated`).
   - `ThreadControlBlock` (TCB) with unique `tid`, kernel stack, and execution budget.
   - `ProcessControlBlock` (PCB) with unique `pid`, memory address space (CR3 PML4), open file descriptor table, and thread list.
3. **Scheduler**:
   - Priority-aware round-robin runqueue with quantum tracking.
   - `yield_now()` and `schedule()` dispatch routines.

## Acceptance

- [ ] Multi-thread cooperative switching functions without stack corruption.
- [ ] Context switch assembly preserves all architectural registers across execution frames.
- [ ] Compiles with zero warnings under `cargo check`.
- [ ] Atomic commits posted to AgentIRC `#dev` and evidence in `#reports`.
