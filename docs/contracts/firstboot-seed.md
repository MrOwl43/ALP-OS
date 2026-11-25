<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Contract: installer → first boot hand-off

Source of truth: `alp_hw::firstboot::FirstbootSeed` (re-exported as `alp_core::firstboot`)
(`crates/alp-hw/src/firstboot.rs`). File: `paths().firstboot_seed`
(`/var/lib/alp/firstboot-seed.toml` on the installed `state` partition),
mode 0600, owner alpd.

## Why

In the beta the user confirms the pool (which disks, what protection) on the
installer's Confirm screen, before the node identity and the ledger exist.
The seed carries that confirmation to first boot, where `alpd` creates the
pool.

## Example

```toml
format_version = 1
installer_version = "2026.10.17.1"
profile = "generic-x86_64"
protection_tier = "standard"
data_profile = "dup"          # single | dup | raid1 | raid1c3 | raid10
metadata_profile = "dup"
usable_bytes = 240000000000
protection = "bit-rot-only"   # none | bit-rot-only | disk-failure
confirmed_at = "2026-10-17T09:30:00Z"
plan_digest = "<64 hex digits>"

[[members]]
by_id = "nvme-Samsung_SSD_970_EVO_S4EWNX0N123456"
serial = "S4EWNX0N123456"
size_bytes = 500107862016
partition_uuid = "1b4e28ba-2fa1-11d2-883f-0016d3cca427"
```

## Rules

1. First boot refuses an unknown `format_version`.
2. First boot discovers the disks again. If any member is missing, its
   serial or size differs, its partition UUID is not an `alp-data`
   partition, or `alp_hw::safety::check_member` refuses it, first boot stops
   with `ALP-INST-005` and writes nothing.
3. First boot rebuilds the `PoolPlan` from the rediscovered members plus
   `data_profile`, `metadata_profile`, `usable_bytes` and `protection`
   (added 2026-11-25: without them the plan, and so the digest, could not
   be rebuilt), recomputes the plan digest and requires it to equal
   `plan_digest`; then it writes
   `LedgerEvent::FirstBoot` followed by
   `LedgerEvent::OperationConfirmed { action: "create-pool", plan_digest }`
   (with `confirmed_at` in the payload) and builds the `Confirmed<PoolPlan>`
   from that entry.
4. The seed is deleted only after first boot completes; until then a reboot
   resumes first boot from it.
5. The installer creates the `alp-data` partition on every member (dedicated
   data disks too, with `image/install-layout/data-disk/`), so partition
   UUIDs are known and the members are `PoolMember::ExistingPartition`.
6. The TOML is written and read only by `FirstbootSeed::to_toml` and
   `FirstbootSeed::from_toml` (alp-hw).
7. The installer and first boot compute `plan_digest` with the single
   implementation in `alp-hw` (`PlanDigest`), so both get the same value.
8. Optional `reopen = true` (added 2026-11-25, `format_version` stays 1;
   absent means false, and the installer writes it only when true): the
   members are the `alp-data` partitions of an existing archive to open
   with the recovery words (`docs/plans/0.2.8-reopen-archive.md`), never
   formatted. The profile fields and the digest are computed as for a new
   pool, so rules 2 and 3 check the same disks; the pool record then takes
   the real profile from `btrfs filesystem df` after opening.
