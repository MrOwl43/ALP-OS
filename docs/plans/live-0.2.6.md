<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# ALP-OS Live alpha 0.2.6 — work plan (2026-11-11)

Owner decision 2026-11-11: the next release is **alpha 0.2.6**, and the
first **beta is expected at 0.5**. This supersedes the 2026-10-28 roadmap
note "first beta = Live Beta 0.3"; the versions between now and 0.5 are
ordinary alphas. T27 loses its "v0.3" tag.

## What 0.2.6 is

0.2.5 is not released on the public repository yet: the merge chain of
PRs #6–#10 and the release run first (the Code Orchestrator owns the
merges; the runbook is in the private share). 0.2.6 then:

- finishes the 0.2.5 line: folds in T07 (Wi-Fi signal bars, branch
  `claude/t07-port-signal`, PR pending the owner's word);
- closes small owner-visible gaps: the bare nginx 404 on port 443 (T34),
  skipped setup steps lost on restart (the T09 remainder), the separate
  Live build profile (T10);
- starts the T48 phone/pairing security backlog before the beta
  (S2, S1, B3 — new rows T61–T63 below);
- adds VM tests that close 0.2.5 coverage gaps: reset/cancel (T08) and
  the version-update flow (T65).

Everything is sized for one Claude Code work unit each: claim it as
`docs/COLLABORATION.md` §2 says (own branch, draft PR, edit the Status
column in that branch). Contracts (`api/`, `docs/plans/live-alpha-api.md`)
change first, in their own commit.

## Work units

| ID | Task | Done means | Size | Depends on | Suggested branch |
| --- | --- | --- | --- | --- | --- |
| T34 | `https://<address>/` shows a small ALPArchive page instead of the bare nginx 404 ("the certificate works; open the app on your phone") | `location /` no longer `return 404`; the page is static, no scripts, links nothing; VM test asserts it; the certificate QR path is unchanged | S | owner OK | `claude/t34-443-page` |
| T61 | Pairing token is single-use (T48 S2) | a pending pairing request is answered once, then refused; unit test replays a request and fails it; `t-pair-01` updated | S | — | `claude/t61-single-use-token` |
| T62 | Endorsement v2 with a sequence number (T48 S1; exact format from the security review) | v2 verifies, v1 still accepted (compatibility window); `t-pair-01` checks the v2 signature; API doc updated first | M | — | `claude/t62-endorsement-v2` |
| T63 | Phone sessions idle 60 days are deleted (T48 B3) | a session last seen older than 60 days is removed at the periodic check and the removal is journalled; unit test with a shifted date | S | — | `claude/t63-idle-sessions` |
| T09 | The rest of T09: one skipped setup step is remembered across restarts (today only "finished or left setup" is kept) | skipping the Wi-Fi step, then restarting, returns to that step, not the first; state lives on the stick like `setup/finish`; API doc updated first; VM test | M | — | `claude/t09-setup-steps` |
| T08 | VM test T-LIVE-03: `live/reset` and archive-creation cancel | `tests/vm/t-live-03.sh` prints only ok/FAIL, passes in its VM group on CI | M | — | `claude/t08-live-03` |
| T10 | Separate mkosi Live profile: `just build-live` writes `out/live/`, the installer base stays clean | `just build-image` still builds the same bytes (repro-check); the Live-only settings move behind the profile | M | — | `claude/t10-live-profile` |
| T64 | Kiosk without a usable GPU (pixman/llvmpipe software rendering; the beta roadmap's P0 item) | the kiosk renders with llvmpipe; the existing work is re-applied as fresh commits on this repository (its original clone is never pushed from); VM check; second-developer review | M/L | — | `claude/t64-kiosk-nogpu` |
| T65 | VM test of the version update (beta item 4): rewrite the stick with a newer image; archive and key survive, the index is rebuilt | `tests/vm/t-upd-01.sh` passes on CI (new `update` VM group; watch the cost, it needs two images) | M | a released image | `claude/t65-update-test` |
| T27 | Floating black log window (advanced): journal stream API, F12, top-bar icon; later hidden outside advanced mode | F12 toggles it; it shows alpd/journal lines behind the console sign-in; ui smoke test | M/L | — | `claude/t27-log-window` |
| T26 | Advanced-mode gate (advanced): a default system password published with every release | the gate asks the password, slide-to-confirm; the password is printed in the release notes; docs updated | M | after T27 or with it | `claude/t26-advanced-gate` |
| T66 | Release chores 0.2.6 | `docs/ALPHA-0.2.6.md` notes in the style of `ALPHA-0.2.5.md`; QA "not hardware-tested" list; release workflow run with `release_version=0.2.6` | S | all merged | owner / lead |

Stretch: T27 and T26 may slip to the next alpha without hurting the rest.
T64 is P0 for the beta — start it early even if it lands late.

## Status corrections (made with this plan)

- **T05** is delivered by the Files (stage F, PR #8) and Terminal (PR #9)
  branches; merge pending.
- **T06** is done (T58: `GET live/disks` with role and `safe_to_remove`).
- **T07** code is done on `claude/t07-port-signal` (contract first); the
  PR waits for the owner's word.
- **T31** is done via T25/T39 (hardware specs in Settings > About and the
  About panel).
- **T60**: the server side is in the 0.2.5 snapshot (`tls::ensure`,
  `phone/connection`, the 5-minute check); PR #7 carries the test fixes
  (T-PAIR-SAN); what is left is the owner's hardware re-test.
- **T48**: S2, S1 and B3 become T61, T62 and T63; B1 waits for the app,
  B2 and the rate limits stay in T48 for a later alpha.

## Not in 0.2.6 (later alphas, toward beta 0.5)

- T04 own gallery (after T03 runs on CI), T13, T33 (owner OK pending),
  T35 level 3 (password reopens the backup disk), T36, T41 encrypt-later.
- T48 remainder: B1 (strict default when the app ships), B2 (each Immich
  session bound to its device certificate), the login rate limits.
- The Android app (needs its own plan and a Flutter build on CI).
- T11 (the full security review since 2026-10-18) — a Security Expert
  task, before the beta, not a Claude Code unit.
- T01 (the owner's field re-test) — needs the 0.2.5 image first.

## Release checklist (owner / Code Orchestrator)

1. Merge PRs #6–#10 into `live-alpha` (runbook in the private share);
   full CI green on `live-alpha` after each step.
2. PR for `claude/t07-port-signal` (T07), merged the same way.
3. Merge the 0.2.6 units above as their PRs pass CI.
4. T66: `docs/ALPHA-0.2.6.md`, QA list, then
   `gh workflow run ci.yml --ref live-alpha -f release_version=0.2.6`.
5. The owner writes the stick, field-tests (T01 pattern), and the QA
   list records what was not hardware-tested.
