# TASK-KNT-003 — Native platform and Windows-application compatibility architecture decision

| Field | Value |
| --- | --- |
| Status | Ready |
| Requirement/source | `docs/project-scope.md`, owner decisions 2026-10-29 |
| Milestone | A reviewable architecture decision record for the first native KntonOS vertical slice. |
| Depends on | None |
| Owner/session | Unclaimed |
| Components | Architecture decision record under `docs/architecture/`; scope/integration links as needed |
| Review | Independent lead review; high-risk platform and compatibility decision |

## Goal and scope

Compare feasible approaches for an independently developed x86-64 UEFI kernel and a broad Windows `.exe`/game compatibility boundary. Recommend a smallest credible native vertical slice. This is research and planning only: no copied kernel code, no third-party kernel modification, no assertion that every Windows application will run.

## Acceptance

- [ ] Defines the first boot-to-user-space path and kernel/user-space responsibility boundary.
- [ ] Evaluates implementation-language choices by component, including performance, safety, ecosystem maturity, and debugging implications.
- [ ] Separates what KntonOS can implement natively from any compatibility/runtime behavior and identifies legal/technical risks.
- [ ] Gives a staged compatibility plan with a demonstrable first `.exe` or game target, not an unbounded "all apps" claim.
- [ ] States the validation fixtures, emulator/hardware needs, and open owner decisions.

## Integration agreement

- Producer: architecture decision record.
- Consumers: kernel, GUI, graphics, runtime, and performance tasks created after approval.
- Contract: recommendation must identify explicit component boundaries, interfaces to be specified next, and no use of a modified third-party kernel.
- Validation: sources and assumptions must be cited; an independent reviewer validates that the recommendation meets owner constraints.

## Log and next handoff

Ready for assignment to the implementation lead. On completion, request independent lead review and update TASK-KNT-002/TASK-KNT-004 dependencies.
