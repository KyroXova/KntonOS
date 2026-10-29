# Integration plan

## Current integration boundary

The only current component is repository coordination. `main` is the canonical integration target. Contributor worktrees are isolated branches based on the initial foundation commit.

## Product boundary

The eventual product needs at least a kernel/platform layer, desktop GUI, core applications, and a Windows `.exe` compatibility/runtime boundary. The owner has not yet chosen whether this is a new kernel, a Linux-derived system, a compatibility layer, a virtualized Windows environment, or another approach. No application components or contracts are approved. Once the first milestone and compatibility approach are chosen, the lead will add a minimal producer/consumer contract, compatibility expectations, validations, and an end-to-end demonstration before broad parallel feature work begins.

## Integration gate

Product work remains blocked until TASK-KNT-002 records an approved initial milestone. Routine documentation maintenance may continue within the scoped control in `TASKS.md`.
