# TASK-KNT-006 — Native UEFI/QEMU boot proof

| Field | Value |
| --- | --- |
| Status | Assigned |
| Requirement/source | Owner native-kernel and UEFI constraints; `docs/integration-plan.md` |
| Milestone | A minimal independently developed x86-64 UEFI image that reaches the KntonOS kernel entry, emits a serial success signal, and halts under QEMU/OVMF. |
| Depends on | TASK-KNT-005 revision; provisional boot-slice contract |
| Owner/session | Jules Martin / dev |
| Components | `boot/`, `kernel/`, `scripts/`, `Cargo.toml`, `rust-toolchain.toml` |
| Review | Marcus verifies architectural conformance; NadiaMercer verifies independent acceptance evidence |

## Goal and scope

Create only the first native boot boundary: a UEFI executable loads an independently developed kernel image, the kernel starts on x86-64, writes deterministic serial diagnostics, reports success, and halts. Keep the implementation compact and reproducible on the agreed QEMU/OVMF fixture.

## Exclusions

- No borrowed or modified third-party kernel.
- No GUI, graphics driver, Windows compatibility layer, `.exe` execution, networking, or game-performance claim.
- No unverified tool invocation or generated artifacts committed without a reproducible build path.

## Acceptance

- [ ] A documented command builds the UEFI image from a clean checkout.
- [ ] QEMU/OVMF reaches a deterministic serial success sentinel and exits or halts predictably.
- [ ] The automated runner is valid code, has a bounded timeout, and fails on panic or missing success.
- [ ] The implementation and third-party dependencies are identified with licenses and provenance.
- [ ] A second contributor can reproduce the result from the committed instructions.

## AgentIRC reporting

Jules Martin posts a `#dev` update after every meaningful committed step and posts verification evidence to `#reports`. Marcus and NadiaMercer record review outcomes in AgentIRC.
