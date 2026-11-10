<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# ALP-OS Live alpha 0.2.2 (2026-10-28)

File: `out/ALP-OS-Live-alpha-0.2.2.raw` (2.6 GB) on the build VM, built from
commit 88f3ae0 of `stream-a/live-alpha` (later commits change only tests and docs).
SHA-256: `f02b7ba7e599f761d85df3e8990486dd58c17f8e5f97493e70c87a0e7b5e32e7`

Fixes the empty grey page on the second start of alpha 0.2.1 on the
owner's laptop (docs/TASKS.md T28-T30).

## What changed

- The console answers at once. The hardware check, the battery read and
  the Wi-Fi network are read in the background; while the check runs the
  welcome screen shows "Checking this computer…" (`hardware.pending` in
  the API). `iwctl` gets a 10 s limit.
- Boot log on the stick: `boot-log.txt` on the ALP-REPORT drive, written
  40 s after the start and then every minute. The log of the start before
  is kept as `boot-log-previous.txt`. It lists what waits, what failed,
  ALPArchive's services, the console's answer time and their messages.
- The report drive has one user at a time (lock in `/run/alp/report.lock`);
  a mount left by a stopped run is undone by the next one.
- New VM test T-UI-02: the same stick started twice, with a security chip
  (TPM) and without.

## Tested

- `just check`: format, lints, unit tests and contract checks pass. Only the
  build-tool version check fails, because Ubuntu updated the build VM's
  systemd to 259.9 on 2026-10-28 (lock: 259.5); the lock update is a
  separate change.
- T-UI-02 in QEMU (2 GB RAM), both cases: the page, a UI file and the state
  answer in under 0.05 s on the second start, and the kiosk shows a screen.
- A Linux QA review of the new code; its four must-fix findings are fixed.

## Not tested (needs the real laptop)

- Whether this really fixes the grey page on the laptop's second start (the
  cause was never confirmed; the boot log now shows it if it comes back).
- `boot-log.txt` on a real USB stick: written after 40 s, kept after a
  restart, readable from Windows; behaviour on a slow stick or when power
  is cut mid-write.
- Battery and Wi-Fi reads on real hardware, and the real TPM (VM: swtpm).
- The welcome screen's "Checking this computer…" line was not seen in a
  screenshot (the check ends before the kiosk appears in the VM).
- The other VM scenarios (`just test-vm`) were not rerun for this release.

## Please check

1. Start the laptop twice (setup, switch off, start again): the second start
   must not hang on a grey page.
2. Send `boot-log.txt` and `boot-log-previous.txt` from the stick's
   ALP-REPORT drive, also when everything works.
