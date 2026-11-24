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
- Done: `alp_installer::exec` (executor through `run::Runner`, preflight
  with re-check and by-id comparison, device guard, codes
  `ALP-INST-403`..`408`), `seed.rs` (seed TOML), scripted `alp-installer`
  CLI, `tests/vm/t-inst-01.sh` (CI only).
- Blocked: a real install refuses with `ALP-INST-407` until block 4's
  `PlanDigest for PoolPlan` exists (stream B). The image must ship the
  installer, `install-layout` and install-time sysupdate definitions
  (stream A).
- Next: install-time sysupdate spike with stream A; T-INST-02..05.

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
