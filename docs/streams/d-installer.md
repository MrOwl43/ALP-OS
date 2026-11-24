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
- Next: installer library around it (run lsblk, boot disk from
  `inspect`, install plan, repart, image copy, first-boot seed); VM tests
  with 1 and 2 disks and a fake Windows disk.

## Open problems

- Owner questions in the plan: disks with files but no system; data disk
  minimum; LUKS disks refused.
- VERIFY items listed in the plan (util-linux 2.41 on trixie, `TRAN` for
  SD cards, blkid without JSON).

## Contract change requests

- Hardware profile (`crates/alp-hw/src/profile.rs`): add a data disk
  minimum (`min_data_disk_bytes`) once the owner sets the value. Until then
  the caller passes it in `blockdev::Limits`.
