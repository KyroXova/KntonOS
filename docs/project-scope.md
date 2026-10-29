# Knton OS project scope

## Approved outcome

The owner has authorized a maintainable, expandable project foundation with independent contributor workspaces. KntonOS is an operating system from Snatch, intended to provide its own kernel, GUI, and applications. It should be exceptionally lightweight while offering a Windows 11-like desktop experience with lower resource costs and improved functionality. It is intended to run Windows `.exe` applications.

## Current capability

The repository provides Git coordination documents, a task board, contributor workflow, handover baseline, and three clean local worktrees. It contains no product implementation, kernel prototype, GUI, application runtime, or Windows compatibility layer.

## Initial usable milestone

Pending owner decision: define the first user-visible KntonOS capability and its evidence of success.

## Explicit exclusions for this phase

- Kernel architecture, GUI framework, runtime/compatibility design, APIs, and storage choices.
- Deployment, publishing, external accounts, and credential setup.
- Feature implementation or claims based on assumed requirements.

## Owner-action queue

### OQ-KNT-001 — Define the initial product slice

| Field | Value |
| --- | --- |
| Status | In discovery |
| Decision | Confirm target users, first user-visible milestone, and exclusions. |
| Why it matters | It determines architecture, task boundaries, acceptance tests, and the first implementation claims. |
| Affected tasks | TASK-KNT-002 and all future product tasks. |
| What can proceed | Repository maintenance, requirements capture, feasibility research, and task refinement. |
| Raised | 2026-10-29 |

## Decisions

| Date | Issuer | Decision | Scope |
| --- | --- | --- | --- |
| 2026-10-29 | Project owner | Establish a maintainable, expandable project and free developer workspaces. | Repository coordination phase |
| 2026-10-29 | Project owner | KntonOS has its own kernel, GUI, and applications; aims for a low-resource Windows 11-like experience and Windows `.exe` compatibility. | Product direction; architectural approach remains undecided |
