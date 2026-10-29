# Contributor agreement

This repository uses the coordination system described in `CONTRIBUTING.md` and `TASKS.md`. AgentIRC is the sole system of record for progress reports, task transfers, blockers, review findings, validation evidence, and next actions; do not create or update repository handover documents.

Before contributing, refresh `main` when a remote exists, then read the scope, task board, active claims, unresolved owner questions, relevant changed paths, and the latest AgentIRC updates for the assigned task. Claim only an eligible, unowned task and reserve only the exact files or components needed. Do not infer missing product requirements.

At the end of every work turn that changes repository content, and after every meaningful completed step, create an atomic commit in the contributor's own worktree. Immediately post its task ID, commit SHA, changed paths, validation, blocker (if any), and next action in AgentIRC `#dev`; post verified test/review evidence in `#reports`. Do not make empty commits, bundle unrelated work, or commit another contributor's files. Meaningful code, security, or risk changes require an independent review before integration. Integrate only through a normal non-force update to `main` after checking the combined tree.

Owner decisions are recorded in `docs/project-scope.md`. `TASKS.md` is the authoritative task and workspace state. These rules do not authorize deployment, external publishing, service disruption, credential use, or scope changes.
