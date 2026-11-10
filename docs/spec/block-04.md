<!-- Generated from docs/spec.md (Version 0.5) by tools/split-spec.py. Do not edit; edit docs/spec.md and rerun the tool. -->

### Block 4 — Storage: encrypted Btrfs pool

**Goal.** A pool that protects user data against bit rot, survives disk failure when redundancy exists, and can grow easily.

**Decisions.** Btrfs on top of LUKS2, one LUKS container per data disk, all unlocked with the same pool key. Everything goes through `StorageBackend`/`BtrfsBackend` in `alp-storage`.

- Automatic planning: one disk: metadata `dup`, data `dup` (self-healing for bit rot, effective capacity is half; show the effective capacity and say it is partial protection and not protection against disk failure; provide an advanced setting to switch data to `single`). Two or more disks: `RAID1` (data and metadata). Allowed profiles: `dup`, `single` (only by explicit advanced setting), `RAID1`, `RAID1c3`, `RAID10`. **RAID5/6 are forbidden.**
- Subvolumes: `@archive` (user data), `@snapshots` (read-only snapshots), `@system-backup` (encrypted state backups). Mount with `compress=zstd:3` only if CPU budget allows (decide by benchmark on the reference machine, record as ADR) and `noatime`.
- Maintenance: monthly scrub (throttled, resumable across reboots), automated read-only snapshots (hourly 24, daily 14, weekly 8, tunable constants in the profile) as protection against deletion and ransomware, SMART monitoring via `smartmontools` with plain-language alerts.
- Expansion: `propose_expansion` computes the resulting usable capacity and shows it first. `apply_expansion` runs `device add` followed by a throttled background `balance`. Replacing with a larger disk uses `replace` then `resize`. No expansion without explicit confirmation. Disks of different sizes are allowed and the UI explains usable capacity honestly.
- USB disks are supported as data disks. Disconnect during operation is a normal event: the pool goes read-only or degraded, an alert is raised, nothing crashes, and reconnecting recovers automatically when safe.

**Constraints.** Parse command output only from machine-readable forms (`--json`, `-b`, `/sys`) wherever available. **VERIFY** `btrfs-progs` features in the shipped version. Never run `btrfs check --repair` automatically.

**Interfaces.** The `StorageBackend` methods in Section 5, plus events: `PoolDegraded`, `DiskMissing`, `ScrubFinished{errors_found, errors_corrected}`, `CapacityLow`.

**Acceptance (VM, virtual disks).** Create pool on 1 disk and on 2 disks. Pull one disk of a RAID1 pool: data stays readable, pool reports degraded. Reinsert or replace the disk and rebuild completes with no data loss. Corrupt a block on one mirror copy, run scrub, error is detected and repaired. Add a disk of a different size and confirm reported capacity matches the plan.

**Out of scope.** ZFS, RAID5/6, dedup, send/receive between nodes, quota per user.
