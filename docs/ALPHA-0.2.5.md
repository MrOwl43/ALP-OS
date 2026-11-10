<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# ALP-OS Live alpha 0.2.5 (2026-11-04)

File: `ALP-OS-Live-alpha-0.2.5.img.xz` on the private GitHub pre-release
`alpha-0.2.5`, built by CI from `stream-a/live-alpha`. The SHA-256 is in the
`.sha256` file next to it. Test with unimportant files only.

Owner's 0.2.4 feedback (docs/TASKS.md T42-T52) and the security reviews.

## What changed

- **Security fix.** On a computer with a TPM security chip, any password was
  accepted (sign-in, recovery code, delete, reset), because cryptsetup opened
  the disk with the chip instead of checking the typed password. Every
  password check now ignores the chip.
- **Recovery code in two steps (T42).** Opening the page makes no code. The
  ALP password comes first; replacing a code needs a warning and a slide; the
  new code counts only after three of its words are typed back. Leaving the
  page wipes the words, and the old code keeps working. No Print button.
- **New console (Live UI 0.2.4).** A thin system bar, errors collected in the
  (i) panel, settings in categories, password + warning + slide for risky
  steps, Advanced asks the password once for 10 minutes.
- **Disks (T49).** Quick or secure format of an external disk, with a time
  estimate and Stop. "Delete everything" really erases an unprotected stick.
  Only disks are offered when the stick itself was recognised.
- **Setup is remembered (T09).** After setup, a restart opens the everyday
  screens (and the lock screen), not setup again.
- **Boot logs (T44).** The last 5 starts are kept on the stick.
- **Phones (D2).** The pairing QR carries the family sign-in; phones sign in
  with the ALP password. The regular Immich app still works (certificate
  installed on the phone).

## Tested

- CI on GitHub: Rust, shell, supply chain, the UI smoke test (every screen
  over test data), and the VM tests T-BOOT-01/03, T-BUDGET-01, T-LIVE-01 to
  05, T-API-01, T-UI-01/02, T-PAIR-01, T-IMMICH-01 (with T-PAIR-02).
- Reviews: Security and Network Expert (approved), Linux QA (its blocker and
  two medium findings fixed before this release).

## Not tested on hardware

- Everything: nothing of 0.2.5 ran on the laptop or a real phone yet.
- In particular: the TPM fix on real fTPM/PTT chips, secure format and the
  real wipe on real USB sticks and USB-SATA bridges (speed, `blkdiscard`),
  card readers with several slots, write-protect switches, NTFS/HFS+ disks,
  the early boot log on slow USB 2 sticks, and the kiosk during long jobs.

## Known open items

- Setup after a cut-off reset still finishes the wipe inside the request;
  a second try now waits (setup.busy) instead of wiping twice.
- A stick with a bad region cannot be set up again after a failed wipe.
- The state can be slow while a recovery code is being made on slow disks.
