<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Working on ALP-OS in parallel (people and Claude agents)

Two or more developers, each possibly driving their own Claude Code agent, work on
the same GitHub repository. Git is the only shared
channel. This file is the rulebook; `CLAUDE.md` points every agent here.

## 1. Start of every session (people and agents)

1. `git fetch origin` and read, in this order: `CLAUDE.md`, this file,
   `docs/TASKS.md` (who works on what), `docs/PROGRESS.md` (top),
   `docs/plans/live-alpha-decisions-2026-10-19.md` and the owner decision log
   (`docs/DECISIONS-NEEDED.md` for open questions).
2. Pick ONE task from `docs/TASKS.md` that is `open`, or one the owner gave you.

## 2. Claiming a task (prevents two people doing the same work)

1. Create a branch from the integration branch `stream-a/live-alpha`:
   `git switch -c <who>/<task-id>-<short-name> origin/stream-a/live-alpha`
   (`<who>`: `dev1`, `dev2`, …; e.g. `dev2/T07-gallery`).
2. In that branch, change the task's line in `docs/TASKS.md` to
   `claimed by <who>, branch <name>`, commit ("Claim T07"), push, and open a
   **draft pull request into `stream-a/live-alpha`**. The draft PR is the claim:
   if someone else already has an open PR for the task, pick another task.
3. Areas have owners for the duration of a task (`docs/plans/live-alpha.md`,
   "Roles and file ownership"). If your task must change files another open PR
   is changing, say so in your PR description and coordinate before editing.

## 3. Doing the work

- Small commits, clear English messages; SPDX header on new files.
- Before each push: `just check` must pass.
- **Testing (2026-10-24):** CI no longer runs on pushes to working branches; run it on demand (Actions → CI → Run workflow) or through a PR. Agents verify by code review; the owner and the second developer test by hand.
- **Never run QEMU/VM tests on a developer VM that runs inside VirtualBox**
  (nested virtualization crashes it). VM tests run on GitHub Actions: every push
  to a branch with an open PR runs `CI` (`.github/workflows/ci.yml`), including
  the image build and `just test-vm`. Read results with
  `gh pr checks <n>` / `gh run view <id> --log-failed`.
- Immich is never run locally on the dev VM; only on CI.
- Contracts (`api/`, `docs/contracts/`, `docs/plans/live-alpha-api.md`) are
  shared: change them first, in their own commit, and note the change at the top
  of the document so the other side can adapt.
- Secrets, keys, tokens: never in the repo or in logs.

## 4. Finishing

1. Mark the PR ready for review when CI is green. PR description: Before /
   After / How, and the test evidence (CI run link).
2. Sensitive areas (encryption, keys, storage, updates, firewall, pairing) need
   an independent review (a person or the `code-reviewer` agent) before merge.
3. The integration branch `stream-a/live-alpha` is merged by the lead
   (currently the owner's Claude session) after CI is green; `main` is merged
   only by the owner's decision.
4. Update `docs/TASKS.md` (task `done`, PR link) and `docs/PROGRESS.md` in the
   same PR.

## 5. Keeping branches fresh

- Rebase or merge `origin/stream-a/live-alpha` into your branch at least daily
  and before marking ready. Resolve conflicts in your branch, never on the
  integration branch.
- Never force-push a branch someone else works on; never rewrite
  `stream-a/live-alpha` or `main`.

## 6. Talking to each other

- Questions about a task: comment on its PR.
- Product questions for the owner: add them to `docs/DECISIONS-NEEDED.md` in
  your PR (options, consequences, a recommendation) and continue with other work.

## 7. Setting up a second developer (owner does this once)

1. GitHub → repository Settings → Collaborators → add the developer's account
   (role: Write).
2. The developer clones the repo, installs the tools from `tests/ci/build-tools.lock`
   (or works only on code/UI and relies on CI for images and VM tests), and starts
   Claude Code in the repository folder. `CLAUDE.md` loads automatically and
   points here.
