<!-- Generated from docs/spec.md (Version 0.5) by tools/split-spec.py. Do not edit; edit docs/spec.md and rerun the tool. -->

### Block 3 — State partition and configuration

**Goal.** One small, durable place for everything that is neither OS nor user data.

**Decisions.** `state` holds: node identity, wrapped keys, `alpd` SQLite database, ledger, configuration, pool metadata (`device UUIDs`, profile). `/etc` is an overlay over the read-only root with its upper layer on `state`. Canonical path `/var/lib/alp`. State is mounted early with `ProtectSystem=strict` semantics for all services except `alpd`.

**Constraints.** Write amplification matters on old SSDs and USB drives: batch writes, use SQLite WAL, avoid chatty logging to `state` (journald is volatile-capped or on its own small area). Daily encrypted backup of `state` into the data pool (see block 5 for the key) so loss of the system disk is recoverable.

**Interfaces.** `alp_core::state::paths()` is the only source of paths. No hard-coded paths elsewhere.

**Acceptance.** Replacing a slot does not change the state content. Wiping `state` and restoring from the pool backup using the recovery key yields the same node identity and a mountable pool (VM test).

**Out of scope.** Syncing state between nodes.
