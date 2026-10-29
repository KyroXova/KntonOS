# TASK-KNT-011 — Userland Execution & Syscall Dispatch Engine

| Field | Value |
| --- | --- |
| Status | Claimed |
| Requirement/source | `docs/architecture/ADR-001-native-platform-and-compatibility.md` Slice 1 & Slice 2 |
| Milestone | Fast `syscall`/`sysretq` dispatch table mapping Win32 NT-subsystem syscalls to kernel handlers. |
| Depends on | TASK-KNT-008, TASK-KNT-009 |
| Owner/session | Ada Chen / dev |
| Components | `kernel/src/syscall/` |
| Review | Marcus verifies userland pointer validation and kernel stack swap safety. |

## Goal and scope

Implement the userland syscall entry and dispatch infrastructure:
1. **Syscall Table & Handlers**:
   - Dispatch table matching NT syscall conventions (`rax` = syscall number, `r10`, `rdx`, `r8`, `r9` = args).
   - Core NT stubs: `NtYieldExecution`, `NtWriteFile` (serial/console debug out), `NtReadFile`, `NtAllocateVirtualMemory`, `NtTerminateProcess`.
2. **Userland Pointer Sanitation**:
   - Safe validation of user space memory buffers (`validate_user_ptr`, `validate_user_slice`).
   - Zero kernel panic on malformed userland addresses.
3. **Execution Verification**:
   - Unit tests executing mock syscall invocations via software dispatch.

## Acceptance

- [ ] Syscall table routes NT identifiers to corresponding kernel handlers.
- [ ] Invalid pointers return `STATUS_ACCESS_VIOLATION` (0xC0000005) without kernel panic.
- [ ] Compiles with zero warnings under `cargo check`.
- [ ] Atomic commits posted to AgentIRC `#dev` and evidence in `#reports`.
