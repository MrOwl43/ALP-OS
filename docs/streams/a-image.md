<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Stream A — Image and updates

## Plan: ALP-OS Live hardware-test image (first deliverable, ADR 0006/0007)

- `crates/alp-hw/src/inspect.rs`: hardware detection and checks (std only).
- `crates/alpctl/src/live.rs`: `alpctl live report` (screen + file on the
  stick) and `alpctl live protect-disks`. New runtime dependency: `toml`
  (already pinned in the workspace) to read the hardware profile.
- Image: `image/mkosi.conf.d/a-live.conf`, `image/repart.d/30-live-report.conf`,
  units `alp-live-protect.service`, `alp-live-report.service`, udev rule
  `90-alp-live-protect.rules`; `just _stage` builds alpctl (musl) into
  `target/alp-image-tree`.
- Tests: unit tests (fake sysfs tree), VM scenario T-LIVE-01.
- VERIFY: kernel font and efifb in Debian's kernel config (done, ADR 0007);
  GPT basic-data type (done); `SecureBoot` efivar name (checked in the VM).

## Status

See `docs/PROGRESS.md`.

## Next

1. Owner tests the image on real laptops (`docs/LIVE_HWTEST.md`).
2. Fix what the reports show (drivers, detection gaps).
3. Then the full Live beta (kiosk, data pool on the stick, T-LIVE-01 grows).

## Open problems

- Cross-machine reproducibility (CI vs dev VM hashes differ): pinned tools tree.
