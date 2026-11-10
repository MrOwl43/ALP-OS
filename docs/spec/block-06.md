<!-- Generated from docs/spec.md (Version 0.5) by tools/split-spec.py. Do not edit; edit docs/spec.md and rerun the tool. -->

### Block 6 — Audit ledger

**Goal.** A tamper-evident local record of important events that later becomes the base of the Accountabilibuddy protocol.

**Decisions.** Append-only table in SQLite on `state`. Each entry has `seq`, `timestamp`, `event_type`, `payload` (canonical serialization, defined and versioned), `prev_hash`, `hash = SHA3-256(prev_hash || canonical_entry)` and an Ed25519 `signature` by the identity key. The structure must be extendable to a Merkle tree with checkpoints later. Mirror the ledger to the pool daily.

**Events in v1.** First boot, device pairing and revocation, service enable/disable, update applied or rolled back, pool create/expand/replace, key rotation, recovery export, advanced mode toggled, admin login.

**Constraints.** Ledger events never contain secrets or file names or file contents. Verification runs at every boot and on demand, and a broken chain raises a clear alert but does not halt the system.

**Interfaces.** `alp-crypto::ledger::{append, verify, export_range}`.

**Acceptance.** Modifying any past row in the database makes `verify` fail and report the first bad sequence number. Concurrent appends keep a valid chain.

**Out of scope.** Cross-node verification, public anchoring, peer sync.
