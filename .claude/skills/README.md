# Project skills

Skills Claude Code loads when working in this repo. Each `SKILL.md` ends with an
**ALP-OS context** section that narrows it to this project. Upstream text is untouched
above that section. Sources and pinned commits: [PROVENANCE.md](PROVENANCE.md).

## Workflow (how we work)

| Skill | Use it for |
|---|---|
| `writing-plans` | The plan every block needs before code. Saved in `docs/plans/`. |
| `test-driven-development` | Failing test first: cargo tests and `tests/vm/` scenarios. |
| `verification-before-completion` | Nothing is "done" without real `just check` / `just test-vm` output. |
| `systematic-debugging` | Root cause before fix. Boot, storage and update failures. |
| `requesting-code-review` | Uses the repo's `code-reviewer` agent. One round per PR. |
| `receiving-code-review` | Handling findings. Big decisions go to the owner. |
| `finishing-a-development-branch` | Branch, PR and merge rules. Never push to `main`. |
| `using-git-worktrees` | Only when two writers would touch the same files. |

Not installed on purpose: `subagent-driven-development`, `dispatching-parallel-agents`
(the owner limits us to one or two workstreams at a time to save usage).

## Security lens (what we audit)

| Skill | Block / area |
|---|---|
| `13-crypto-analysis` | Keys, LUKS, TPM, recovery key, update signing. Review only. |
| `20-supply-chain-security` | Reproducible builds, pinned deps and tools, SBOM, signing. |
| `15-blue-team-defense` | Hardening of the immutable image. Run inside the test VM. |
| `02-vulnerability-scanner` | Rust workspace, image packages, mobile lockfiles. |
| `09-web-security` | OpenAPI contracts, Immich exposure. |
| `08-network-security` | LAN exposure, pairing, first-boot traffic. |
| `17-mobile-security` | `Alparchive-mobile` (separate repo). |
| `10-cloud-security` | Container units and CI only. |
| `19-grc-compliance` | Later: privacy and licence documents, on request. |

Scope rules for all security skills: test only our own test VMs and builds; never run
scans or scripts against third-party hosts; never commit findings that contain secrets.
The offensive skills (exploit development, red team, malware, recon) are not part of this
repo's set.
