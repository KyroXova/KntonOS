# Integration plan

## Current integration boundary

The only current component is repository coordination. `main` is the canonical integration target. Contributor worktrees are isolated branches based on the initial foundation commit.

## Product boundary

The eventual product needs at least a native x86-64 UEFI boot/platform layer, independently developed kernel, desktop GUI, core applications, graphics/input/audio stack, and a Windows `.exe` compatibility/runtime boundary. A Linux-derived or modified third-party kernel is excluded. No application components or contracts are approved. TASK-KNT-003 will recommend a legally and technically supportable compatibility boundary; TASK-KNT-004 will define the measurable first-demo gate. Only then will the lead add a minimal producer/consumer contract and an end-to-end implementation sequence.

## Integration gate

Product implementation remains blocked until TASK-KNT-002 has a measurable demonstration definition and TASK-KNT-003 establishes an approved architecture direction. Architecture research and performance-planning work may proceed within the scoped control in `TASKS.md`.
