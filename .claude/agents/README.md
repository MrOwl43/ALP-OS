# Agents

Specialists the lead (the main session) hands one task at a time. They map to the workstreams in
`docs/streams/README.md`; the stream rules there apply to all of them.

| Agent | Scope | Model |
|---|---|---|
| `stream-a-image` | Image build, verified root, A/B updates, rollback (blocks 1-2) | opus |
| `stream-b-storage-keys` | State partition, Btrfs on LUKS, keys, recovery key (blocks 3-5) | opus |
| `stream-c-core` | `alpd`, API, ledger, health, error catalog (blocks 6, 10) | sonnet |
| `stream-d-installer` | Installer, hardware check, disk plan (block 7) | opus |
| `stream-e-photos` | Immich Quadlet units, pinned digests (block 8) | sonnet |
| `stream-f-ui` | Shared web UI (sections 2.2, 8) | sonnet |
| `stream-g-network` | Firewall, networkd, mDNS, local CA and TLS (block 9) | sonnet |
| `test-harness` | VM tests, locks, end-to-end gates | sonnet |
| `docs-adr` | ADRs, PROGRESS, stream notes | sonnet |
| `code-reviewer` | Independent review, one round per PR | opus |
| `researcher` | Upstream research, licenses, versions | sonnet |

## How they are used

- **The lead is the orchestrator.** Subagents cannot start other subagents, so the main session
  picks the next task from `docs/PROGRESS.md`, hands it to one agent, reviews the plan, and
  collects the report.
- **One stream at a time, two at most** (owner's usage rule in `CLAUDE.md`). Defining many agents
  costs nothing until they run.
- **Models follow the owner's rule:** opus for encryption, keys, updates, rollback and storage
  (and the installer, because it partitions disks); sonnet elsewhere. Change a model in the agent's
  frontmatter if the owner decides otherwise.
- **Review gate:** changes to encryption, keys, updates, rollback, storage, pairing or the firewall
  go through `code-reviewer` once before the lead merges.
- **Owner decisions:** agents never decide product questions. They describe options in plain
  English and stop.
- **Skills** live in `.claude/skills/` (see its README). Agents load them on demand.
