# Knton OS project scope

## Approved outcome

The owner has authorized a maintainable, expandable project foundation with independent contributor workspaces. KntonOS is a general-purpose operating system from Snatch, with an emphasis on gamers and everyday users. It will provide its own independently developed kernel, GUI, and applications. It should be exceptionally lightweight while offering a Windows 11-like desktop experience with lower resource costs and improved functionality. Broad support for Windows `.exe` applications, especially games, is a strategic compatibility goal.

## Current capability

The repository provides Git coordination documents, a task board, contributor workflow, AgentIRC-based progress reporting, and three clean local worktrees. It contains no product implementation, kernel prototype, GUI, application runtime, or Windows compatibility layer.

## Initial usable milestone

The first public demonstration should boot KntonOS and run a real game with measured, improved performance. Exact hardware, game, comparison system, and success threshold remain to be selected.

## Product constraints

- KntonOS must use an independently developed kernel; adapting or modifying another operating system's kernel is out of scope.
- The initial boot/platform target is modern x86-64 UEFI hardware. Broad hardware support is a direction, not an untestable promise for the first demonstration.
- Keep operating-system RAM use and ISO size minimal through deliberate architecture and measured budgets.
- Keep the project as a private monorepo.
- The default licensing posture is all rights reserved (ARR); project-owned components may be released under MIT only where the owner explicitly marks them as applicable.
- Choose implementation languages by component based on performance, safety, toolchain maturity, and auditability; do not optimize for line count. A very large codebase may result, but millions of lines are not a delivery metric.

## Explicit exclusions for this phase

- Kernel architecture, GUI framework, runtime/compatibility design, APIs, storage choices, and exact first-demo hardware/game benchmarks.
- Deployment, publishing, external accounts, and credential setup.
- Feature implementation or claims based on assumed requirements.

## Owner-action queue

### OQ-KNT-001 — Define the initial product slice

| Field | Value |
| --- | --- |
| Status | Partially resolved |
| Decision | Confirm the first-demo game, reference Windows setup, target hardware tier, and measurable performance/resource thresholds. |
| Why it matters | It determines architecture priorities, task boundaries, acceptance tests, and the first implementation claims. |
| Affected tasks | TASK-KNT-002 and all future product tasks. |
| What can proceed | Repository maintenance, requirements capture, feasibility research, and task refinement. |
| Raised | 2026-10-29 |

## Decisions

| Date | Issuer | Decision | Scope |
| --- | --- | --- | --- |
| 2026-10-29 | Project owner | Establish a maintainable, expandable project and free developer workspaces. | Repository coordination phase |
| 2026-10-29 | Project owner | KntonOS has its own kernel, GUI, and applications; aims for a low-resource Windows 11-like experience and Windows `.exe` compatibility. | Product direction; architectural approach remains undecided |
| 2026-10-29 | Project owner | General-purpose focus with preference for gamers and everyday users; use an independently developed native kernel; target modern UEFI hardware; first demo runs a real game with improved performance; private monorepo and ARR/MIT licensing posture. | Product constraints and first-demo direction |
