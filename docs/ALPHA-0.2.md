<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# ALP-OS Live alpha 0.2 (2026-10-25)

File: `out/ALP-OS-Live-alpha-0.2.raw` (2.6 GB) on the build VM, built from
commit 4695ba3 of `stream-a/live-alpha`.
SHA-256: `c29a041a98c0d7f86b9ced40b70be819599b87e7d5c41b575c0b3f826729a42c`

Write it to a USB stick (16 GB or more) as in `docs/LIVE_HWTEST.md`
(Rufus or balenaEtcher on Windows, `dd` on Linux).

## New since alpha 0.1

- One main password; 15-word recovery code; 8-word code to join devices;
  optional "Encryption" (password at every start, extra passphrase) (ADR 0013).
- New archive storage (ADR 0014): an encrypted folder that grows as photos
  arrive. No big empty file, so creating an archive takes about a minute.
- Before a disk is used: health check (where the disk supports it) and a
  256 MB write/read test; disks slower than 3 MB/s on average are refused with a reason.
- A disk that was not removed safely is repaired automatically.
- "Prepare this disk": an unusable external disk can be erased and formatted
  (password, warning listing what is erased, slide to confirm).
- Immich inside the image (ADR 0015): phone backup page with server address,
  certificate QR code and a short family password you can choose.
- Earlier fixes: archive creation progress, cancel and error codes, diagnosis
  file in ALP-REPORT, full reset, slide-to-confirm, step skipping, new
  background, specification button.

## What to expect

- The first time Immich starts (and after an update) it unpacks its images
  onto the stick: some minutes, about 2.5 GB of the stick's encrypted area.
- Archives made with alpha 0.1 still open with their 64-letter key; the new
  15-word code works only for archives made with 0.2.

## Not tested in practice (please report what you see)

Nothing below has run on a real computer, a VM or a phone. Only code review,
unit tests and one image build were done.

1. The image booting at all (no VM test was run for this build).
2. Creating, opening and closing the new encrypted folder (gocryptfs) on
   exFAT, NTFS and ext4 disks, its speed on USB sticks, and file ownership on
   exFAT/NTFS while Immich writes.
3. The disk test (256 MB, 3 MB/s average) on real sticks and USB disks.
4. Automatic repair of a disk not removed safely (exFAT and NTFS).
5. Health check (SMART) through USB adapters.
6. "Prepare this disk" (erase and format).
7. Recovery with the 15 words on a new stick; old alpha 0.1 archives opening.
8. Immich: unpacking its images on first start, starting and stopping with
   the archive, staying up, memory on 4 GB machines.
9. Certificate and QR code page, Android and iPhone accepting the
   certificate, the Immich app connecting and backing up.
10. Choosing the family password.
11. The console screens themselves (the new screens were never shown in a
    browser).
12. Shutdown order (Immich, then the archive, then the disk).

Details: "Not verified" sections of ADR 0014 and ADR 0015.
