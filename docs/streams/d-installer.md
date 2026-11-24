<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Stream D — Installer (block 7)

## Plan

0.2.8 (ADR 0028): [docs/plans/0.2.8-installer.md](../plans/0.2.8-installer.md)
(disk discovery, operating-system detection, two start paths, pool plan,
tens of TB, VERIFY status).

## Status

- Done: `alp_hw::inspect::blockdev` parses `lsblk --json` (and
  `blkid -p -o export`), lists every disk with its roles or one reason,
  greys out disks with an operating system, never offers the start stick.
  Unit tests over committed fixtures (`crates/alp-hw/tests/fixtures/lsblk/`).
  New code `ALP-INST-400` (disk list unreadable).
- Done: `alp_installer::plan` (install plan for both paths, erase list,
  strict re-check, codes `ALP-INST-401`/`402`) and
  `alp_installer::capacity` (usable bytes). Unit tests only.
- Next: the executor (run lsblk, boot disk from `inspect`, compare by-id
  names in the re-check, repart, image copy, first-boot seed with
  `PlanDigest`) and the VM tests T-INST-01..05 from the plan.

## Open problems

- Owner questions answered 2026-11-24 (files: offered with warning and
  slide; data minimum 32 GB; LUKS greyed out).
- Stream A: the 0.2.8 stick needs an `alp-state` partition for the
  keep-running path.
- VERIFY items listed in the plan (util-linux 2.41 on trixie, `TRAN` for
  SD cards, blkid without JSON).

## Contract change requests

- **Hardware profile: data disk minimum** (owner set 32 GB on 2026-11-24).
  Change: add `pub min_data_disk_bytes: u64` to `HardwareProfile`
  (`crates/alp-hw/src/profile.rs`) and `min_data_disk_bytes =
  32_000_000_000` to `profiles/generic-x86_64.toml`. Users to update in
  the same `contract:` PR: none construct `HardwareProfile` today (no
  loader yet); `alp_live::live::minimums` reads keys by name and is not
  affected; `alp_installer::plan::MIN_DATA_DISK_BYTES` is replaced by the
  profile value. Until then the constant is used.
