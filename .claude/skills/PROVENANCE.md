# Provenance

Vendored third-party skills. Reviewed before inclusion (scripts and hooks read; no
network exfiltration, no hidden commands). To update, re-copy from the pinned source,
diff against this tree, and re-apply the **ALP-OS context** section.

| Skills | Source | Commit | License |
|---|---|---|---|
| `13-crypto-analysis`, `20-supply-chain-security`, `15-blue-team-defense`, `02-vulnerability-scanner`, `17-mobile-security`, `09-web-security`, `08-network-security`, `10-cloud-security`, `19-grc-compliance` | https://github.com/Masriyan/Claude-Code-CyberSecurity-Skill | `a3be55a446b0f42fe5bb70f268ac10f6eeadc38a` | MIT |
| `writing-plans`, `test-driven-development`, `verification-before-completion`, `systematic-debugging`, `finishing-a-development-branch`, `requesting-code-review`, `receiving-code-review`, `using-git-worktrees` | https://github.com/obra/superpowers | `22f0b463e7a5e579904dd6211a520321035526f7` | MIT |

## Local changes to upstream text

- Superpowers: `superpowers:` cross-references replaced with local names; plans path
  `docs/superpowers/plans/` changed to `docs/plans/`; references to `subagent-driven-development`
  point to native execution; the JS/npm helper files (`find-polluter.sh`,
  `condition-based-waiting-example.ts`) were not copied and their references were replaced.
- Cyber skills: copied unchanged (scripts and examples included).
- Every skill: an `## ALP-OS context` section appended at the end.

License texts of the upstream projects: `LICENSE-superpowers.txt`, `LICENSE-cybersecurity-skills.txt`.
