# TASK-KNT-005 — UEFI toolchain and QEMU validation-fixture research

| Field | Value |
| --- | --- |
| Status | Revision requested |
| Requirement/source | Owner UEFI-native-kernel constraint; TASK-KNT-003 |
| Milestone | A reproducible proposed toolchain and emulator-validation fixture plan for the first native boot slice. |
| Depends on | None |
| Owner/session | Jules Martin / dev |
| Components | `docs/architecture/RESEARCH-001-toolchain-and-emulation-fixtures.md` |
| Review | Marcus architecture review |

## Goal and scope

Research a reproducible x86-64 UEFI development, build, and QEMU-based validation path suitable for an independently developed kernel. Identify exact assumptions, debugging options, generated artifacts, and the smallest boot validation that future implementation can use. This task must not create product kernel code before TASK-KNT-003's decision.

## Acceptance

- [x] Identifies a candidate UEFI build and boot fixture with version-pinning and reproducibility requirements.
- [x] Defines QEMU invocation requirements, firmware dependencies, serial/debug capture, and artifact locations.
- [x] Defines the first proposed automated validation signals without claiming implementation results.
- [x] Records host-platform constraints and open decisions for the architecture lead.

## AgentIRC review outcome

Lead review found that the proposed `boot_runner.py` snippet is not valid Python and must not be presented as runnable. Correct it as part of the native boot proof. Evidence and next action are recorded in AgentIRC; do not create repository status-transfer files.
