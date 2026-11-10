<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# ALP-OS Live alpha 0.2.3 (2026-10-29)

File: `out/ALP-OS-Live-alpha-0.2.3.raw` (2.6 GB) on the build VM, built from
commit d4bd442 of `stream-a/live-alpha` (later commits change only tests and docs).
SHA-256: `2def7d841e172ed0281d853654d3c28126bea4e7760fd226967e33b332f91afb`

Owner's requests of 2026-10-28 (docs/TASKS.md T31-T35, ADR 0016).

## What changed

- **Encrypt or not (T35).** The first protection screen asks "Do you want
  to encrypt your backup disk?". Yes (recommended): the ALP password, the
  encrypted archive as before, "Strong protection" (renamed from
  "Encryption") on the disk step. No: an UNPROTECTED screen and a slide to
  confirm, then a password for the screen only, or none. Unprotected
  archives are normal files in `ALPArchive/files` on the backup disk, and
  "Delete everything" leaves those photos there. Without a password, phones
  get a short phone password shown on the phone screen.
- **Connected devices (T33).** Settings lists each signed-in device (model,
  system, last seen) with "Sign out", and the addresses using ALPArchive
  now. The console's own gallery no longer counts as a phone.
- **About (T31).** Settings > About: system version, protection, and the
  hardware specs.
- **Back like an operating system (T32).** Back and Esc return to where you
  came from. After setup, missing steps open from the home screen,
  Settings or the top bar, without the step row.
- **Phone browser (T34).** `https://<address>/` shows "The connection is
  safe" instead of a 404. The certificate page explains iOS's Certificate
  Trust Settings and no longer mentions an Android switch that does not
  exist.

## Tested

- `just check`: all pass except the build-tool version check (the VM's
  systemd 259.9 against the lock's 259.5; separate change).
- T-UI-02 in QEMU, three cases (security chip, password, unprotected): the
  console answers in under 0.2 s on the second start, the kiosk shows a
  screen, and the unprotected archive is the plain folder with nothing
  encrypted mounted. The security-chip case failed once while a second test
  VM ran on the same host (blank kiosk for 5 minutes) and passed on its own.
- Two reviews (Linux QA, storage security); their must-fix findings are
  fixed, among them: an edited stick config cannot turn an encrypted stick
  into an unprotected one.
- The screens were tried in the browser with test data.

## Not tested

- Nothing on the laptop or phones yet. T-IMMICH-01 (phone backup, the new
  `/` page) runs only on CI.
- The unprotected setup on a real USB stick and disk (exFAT, NTFS), the
  device list with real Android and iOS apps, "Sign out" of a phone.
- Not built yet: the ALP password reopening an encrypted disk from another
  stick (ADR 0016 level 3; encrypted sticks still offer the 15-word code),
  a safe shortcut for Strong protection, T36 (alpaca animations).

## Please check

1. New stick, choose "No": warning, slide, no password; phone screen shows
   a phone password; the disk on Windows shows `ALPArchive/files`.
2. New stick, choose "Yes": as before; Settings > About says "Encrypted".
3. Settings: devices with names; sign one out.
4. Back and Esc from Settings, About and a home reminder.
5. On the phone: `https://<address>/` after installing the certificate.
