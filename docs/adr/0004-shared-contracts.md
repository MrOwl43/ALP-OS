# 0004 — Shared contracts before parallel work

- **Status:** accepted. Items marked *proposed* below were approved by the
  owner on 2026-10-17 (ADR 0005).
- **Date:** 2026-10-16
- **Spec reference:** Sections 2, 2.2, 4, 5, 9; blocks 2–8, 10; NEXT_TASK step 0.4
- **Review:** independent review of PR #4 (verdict "fix first"; all blockers
  and should-fix items addressed in this version)

## Context

Seven workstreams will run in parallel. NEXT_TASK requires the interfaces
they share to be defined and merged first, so streams do not invent their
own. Index: `docs/contracts/README.md`.

## Decisions

1. **Backend traits** as in spec Section 5, in `alp-hw`, with these
   deviations (*proposed*, approved in ADR 0005):
   - All traits are `Send + Sync` (alpd shares them across async tasks).
   - Destructive storage operations take a `Confirmed<T>`: `create_pool`,
     `apply_expansion` and `replace_disk` (one `DiskReplacement`). A
     confirmed value cannot be changed after the user saw it; `alpd` creates
     it only after recording the confirmation in the ledger.
   - `StorageBackend::on_event` delivers the block 4 events.
   - Pool members are partition-level (`PoolMember`); the safety rule
     `alp_hw::safety::check_member` (with tests) refuses anything holding the
     system (`ALP-STO-006`).
   - `Slot` is identified by GPT partition number; the root partition UUID
     identifies the image in it.
   - `VerifiedImage` is a version reported by systemd-sysupdate, which
     verifies signatures and hashes itself.
2. **Key material** (*proposed*, approved in ADR 0005): only `alp-crypto`
   creates or reads key bytes. `Secret`, `PoolKey` (64 bytes), `Identity`,
   `RecoveryKey`, `RecoverySheet`, `Passphrase` and `SealedBlob` never print
   their contents; `SealedBlob` is treated as secret because the keyfile
   backend stores the wrap key in it. `alp-storage` passes the pool key to
   cryptsetup with `PoolKey::write_key_file`. `alp-crypto` is split into
   `keys`, `seal`, `recovery` and `ledger` modules (streams B and C).
3. **Dependencies:** `zeroize` 1.9.1, `serde` 1.0.229 (test-only for now),
   `toml` 1.1.8 (test-only). All transitive crates MIT/Apache-2.0/Unicode-3.0.
4. **APIs** (OpenAPI 3.1) with per-operation listeners and security
   (`docs/contracts/api-access.md`): console and gallery on localhost, the
   LAN listener serving only the Immich proxy in the beta (*proposed*, approved in ADR 0005), a root-only unix socket. Console lock with attempt
   limits; recovery sheet needs the PIN again; revoking a device never
   deletes photos. Installer API on localhost; plans re-checked against the
   disks right before writing.
5. **Error catalog:** 26 codes; `errors/codes.lock` makes codes permanent;
   the test checks every `ALP-…` token in every tracked file.
6. **State paths:** `alp_core::state::paths()`: `state` at `/var/lib/alp`,
   service secrets stored sealed, first-boot seed, API socket, and the pool
   at `/var/lib/alp-pool` with `@archive`, `@snapshots`, `@system-backup`
   and `@apps` (*proposed*, approved in ADR 0005).
7. **Disk layout:** `docs/contracts/disk-layout.md` from
   `docs/research/disk-layout.md`; sizes *proposed* (ADR 0005).
   Slots found by version label / `_empty`; second slot created at install;
   repart formats only ESP and `state` (*proposed*, approved in ADR 0005).
8. **systemd targets:** `docs/contracts/systemd-targets.md`.
   `boot-complete.target` requires `alp-healthy.target` (drop-in shipped and
   checked in T-BOOT-01). No systemd-repart at boot (checked in T-BOOT-01).
   The mechanism gating rootless Immich on `alp-storage.target` is stream
   E's first VERIFY item.
9. **Hardware profile schema:** `alp_hw::profile::HardwareProfile`, matching
   `profiles/generic-x86_64.toml`.
10. **Beta gate:** `tests/e2e/t-e2e-01.sh` states each step's assertions,
    runs steps in strict subshells, and requires 3 passing runs in a row.
    Step bodies are written by the owning streams. Joins CI once it can pass.
11. **Confirmation and first boot (re-review):** `Confirmed<T>` is bound
    to a `PlanDigest` of the exact plan; the confirmation is written to the
    ledger first (`LedgerEvent::OperationConfirmed`). Pool creation confirmed
    in the installer travels in the first-boot seed
    (`docs/contracts/firstboot-seed.md`); first boot re-discovers the disks
    and refuses on any difference (`ALP-INST-005`).
12. **Sealed blobs** are saved and loaded only by `alp-crypto`
    (`SealedBlob::store`/`load`: mode 0600, temp file, fsync, rename); their
    bytes are wiped on drop.
13. **Change rule:** a contract change is one PR that updates every user;
    adding error codes or ledger events is not a contract change.

## Consequences

Streams can start against reviewed interfaces. Early contract mistakes need
a cross-stream PR. Proposed items may change when the owner answers;
sizes are permanent per machine after the first beta install.
