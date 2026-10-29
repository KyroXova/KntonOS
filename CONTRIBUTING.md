# Contributing to Knton OS

## Branch and integration model

`main` is the canonical branch. Until a remote is configured, this local repository is the coordination source of truth. Contributors use their assigned worktree and its matching `workspace/<name>` branch. A worktree branch is isolated work, not a claim and not integration.

## AgentIRC is the coordination record

All progress reports, task transfers, blockers, review findings, validation evidence, and next actions belong in the Knton OS AgentIRC rooms—not repository Markdown handover files. Use `#dev` for implementation progress and coordination, `#reports` for verified validation/review evidence, `#bugs` for defects, and `#tasks` for task assignment and claim activity.

## Claiming work

1. Read `TASKS.md`, the relevant card in `docs/tasks/`, and the latest AgentIRC updates for the task.
2. Select a task marked **Ready** and **Unclaimed**; do not start blocked work.
3. Update the task row and card with your human handle, distinct session identity, UTC time, branch/base, and exact component reservation. Commit this narrow claim to `main` only after synchronizing with canonical state. If no integration authority or remote exists, record the proposed claim in the card and ask the lead to publish it.
4. Implement only the card's scope and interface agreement. Record checkpoints in the card.
5. Arrange independent review for meaningful code or risk changes. The reviewer must inspect the fixed range and acceptance evidence.
6. An authorized integrator verifies the claim, review, acceptance evidence, and combined `main` tree, then releases reservations and marks the task Done.

## Commit and progress cadence

Each work turn that changes repository content must end with an atomic commit in the assigned worktree. Each meaningful completed step must also be committed, even when the wider task remains unfinished. Immediately after each such commit, post an AgentIRC `#dev` update with the task ID, SHA, changed paths, validation run, remaining work, and next action. Post independent-review verdicts and verified test evidence to `#reports`.

Do not create empty commits, split a single inseparable change merely for count, commit unvalidated claims as facts, or mix another contributor's work into a checkpoint. Research-only turns with no repository change require an AgentIRC update but no empty commit.

## Product and scope changes

The product brief is not yet supplied. Add proposals as proposals; do not turn them into requirements. Record decisions and questions in `docs/project-scope.md` and update affected task cards only after owner direction.
