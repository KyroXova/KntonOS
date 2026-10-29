# Contributing to Knton OS

## Branch and integration model

`main` is the canonical branch. Until a remote is configured, this local repository is the coordination source of truth. Contributors use their assigned worktree and its matching `workspace/<name>` branch. A worktree branch is isolated work, not a claim and not integration.

## Claiming work

1. Read `TASKS.md`, the relevant card in `docs/tasks/`, and `HANDOVER.md`.
2. Select a task marked **Ready** and **Unclaimed**; do not start blocked work.
3. Update the task row and card with your human handle, distinct session identity, UTC time, branch/base, and exact component reservation. Commit this narrow claim to `main` only after synchronizing with canonical state. If no integration authority or remote exists, record the proposed claim in the card and ask the lead to publish it.
4. Implement only the card's scope and interface agreement. Record checkpoints in the card.
5. Arrange independent review for meaningful code or risk changes. The reviewer must inspect the fixed range and acceptance evidence.
6. An authorized integrator verifies the claim, review, acceptance evidence, and combined `main` tree, then releases reservations and marks the task Done.

## Contribution handover

Each contribution commit updates `HANDOVER.md` with its purpose, changed paths, checks actually run, reviewer/range if applicable, remaining work, and next owner. Do not claim checks, reviews, or integrations that did not occur.

## Product and scope changes

The product brief is not yet supplied. Add proposals as proposals; do not turn them into requirements. Record decisions and questions in `docs/project-scope.md` and update affected task cards only after owner direction.
