# TASK-KNT-004 — First-demo resource and performance acceptance plan

| Field | Value |
| --- | --- |
| Status | Review |
| Requirement/source | `docs/project-scope.md`, owner decision 2026-10-29 |
| Milestone | A measurable first-demo test plan for boot, RAM, ISO size, and real-game performance. |
| Depends on | None |
| Owner/session | Ada Chen / dev |
| Components | `docs/architecture/SPEC-001-performance-and-resource-plan.md` |
| Review | Lead review (@NadiaMercer); benchmarks must be reproducible |

## Goal and scope

Define how the first KntonOS demo will prove its low-resource claim and game-performance goal. Select a representative game, hardware profile, comparison Windows configuration, metrics, repeatable test procedure, and provisional targets. Do not manufacture benchmark results or assume a target without recording it as provisional.

## Acceptance

- [x] Names the game/app, source/version, scene/workload, and licensing/availability assumptions (SPEC-001 §2).
- [x] Defines target and reference hardware, firmware mode, graphics driver assumptions, and Windows comparison setup (SPEC-001 §3).
- [x] Defines RAM-at-idle, boot-time, ISO-size, frame-time/FPS, and stability measurements (SPEC-001 §4).
- [x] Specifies repeated-run method, variance handling, logs/artifacts, and pass/fail thresholds (SPEC-001 §5, §6).
- [x] Separates owner-confirmed goals from engineering-proposed provisional budgets (SPEC-001 §1, §4).

## Integration agreement

- Producer: benchmark and budget specification (`docs/architecture/SPEC-001-performance-and-resource-plan.md`).
- Consumers: platform, graphics, runtime, installer, and release-readiness tasks.
- Validation: test procedure must be executable by a second contributor and must not claim results before an implementation exists.

## AgentIRC reporting

Authored by Ada Chen on `workspace/ada-chen` at commit `a3a9711`. Integrated into `main` at commit `4c96fb8`. Review request, evidence, and next action are recorded in AgentIRC.
