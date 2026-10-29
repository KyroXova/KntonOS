# TASK-KNT-004 — First-demo resource and performance acceptance plan

| Field | Value |
| --- | --- |
| Status | Ready |
| Requirement/source | `docs/project-scope.md`, owner decision 2026-10-29 |
| Milestone | A measurable first-demo test plan for boot, RAM, ISO size, and real-game performance. |
| Depends on | None |
| Owner/session | Unclaimed |
| Components | Benchmark and resource-budget specification under `docs/architecture/` |
| Review | Lead review; benchmarks must be reproducible |

## Goal and scope

Define how the first KntonOS demo will prove its low-resource claim and game-performance goal. Select a representative game, hardware profile, comparison Windows configuration, metrics, repeatable test procedure, and provisional targets. Do not manufacture benchmark results or assume a target without recording it as provisional.

## Acceptance

- [ ] Names the game/app, source/version, scene/workload, and licensing/availability assumptions.
- [ ] Defines target and reference hardware, firmware mode, graphics driver assumptions, and Windows comparison setup.
- [ ] Defines RAM-at-idle, boot-time, ISO-size, frame-time/FPS, and stability measurements.
- [ ] Specifies repeated-run method, variance handling, logs/artifacts, and pass/fail thresholds.
- [ ] Separates owner-confirmed goals from engineering-proposed provisional budgets.

## Integration agreement

- Producer: benchmark and budget specification.
- Consumers: platform, graphics, runtime, installer, and release-readiness tasks.
- Validation: test procedure must be executable by a second contributor and must not claim results before an implementation exists.

## Log and next handoff

Ready for assignment in parallel with TASK-KNT-003. It becomes the first-demo acceptance gate after lead review and owner confirmation of the selected game/hardware.
