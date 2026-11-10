<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# 0018 — Encrypt later: an unprotected Live becomes encrypted

- **Status:** implemented 2026-10-30 (code review fixes in; runs only in CI so far, T-LIVE-03)
- **Date:** 2026-10-30
- **Spec reference:** ADR 0016 (protection levels), ADR 0014 (storage v2), ADR 0011 (index)

## Context

ADR 0016 lets a Live stick run unprotected (level 1 no password, level 2
console password only): plain Btrfs index on the stick, plain folder
`ALPArchive/files` on the backup disk. The owner wants a way back:
"encrypt everything afterwards", behind three demanding confirmation
screens: confirm, type the password, and a final slide.

## Decision (proposed)

A job `archive.encrypt`, started from Settings > Protection, runs with the
photo service stopped and turns the stick into level 3 (encrypted with the
ALP password, ADR 0016):

1. **Checks first, nothing changed:** the archive is open and plain; the
   backup disk has room for a second copy of `ALPArchive/files` plus 1 GB
   (the encrypted copy is made next to the plain one; gocryptfs adds a few
   percent); the stick's index partition has 32 MiB to give for a LUKS2
   header. Otherwise refuse with the reason and how much room is missing.
2. **Index, in place:** `btrfs filesystem resize -32M`, unmount, then
   `cryptsetup reencrypt --encrypt --type luks2 --reduce-device-size 32M`
   with the ALP password (cryptsetup's own online re-encryption, which
   survives a power cut and resumes: no own cryptography, no copy of the
   index elsewhere), open, mount, TPM slot as at setup. `protection`
   leaves the plain config; the console hash and phone password files are
   removed (the ALP password takes over).
3. **Archive, copy then switch:** a new gocryptfs folder `data.partial`
   (archive password in the index, as at creation), mounted at a temporary
   place; `ALPArchive/files` is copied in (`cp -a`, then every file's
   SHA-256 compared through the decrypted view); `data.partial` becomes
   `data`; the record says gocryptfs; `password.conf` is bound (level 3).
4. **Clean-up, last:** only after the verified switch, the plain folder is
   deleted (an unencrypted copy left behind would defeat the point; the
   final slide says so). Then Immich starts again on the encrypted view
   (same mount point, so its paths do not change).

The steps record their progress inside the index (`encrypt-step`), so a
power cut resumes at the step it reached; before step 3 ends, the plain
archive is still the archive.

**Confirmation (owner):** 1. what will happen, how long, how much room;
2. type the ALP password twice (level 1) or the console password, which
becomes the ALP password (level 2); 3. slide "Encrypt, and delete the open
copy when it is done".

## Storage review (2026-10-30): the steps to implement

In-place `cryptsetup reencrypt` stays (no room on the stick for a second
index partition; parking the index elsewhere leaves plaintext or loses it on
a power cut).

1. Checks: Btrfs has >= 1 GiB free, the disk has room for the photos + 1 GiB,
   the password passes the rules. `pg_dump` of Immich's database into
   `ALPArchive/encrypt-backup/` (removed at the end).
2. Stop Immich; alpd refuses unlock, archive and gallery calls while
   `encrypting`. The step is kept in the plain `index.toml` until the index
   is encrypted (the LUKS header's `online-reencrypt` flag is the truth).
3. `btrfs filesystem resize -64M` while mounted; check the filesystem now
   ends 32 MiB before the partition; unmount; nothing may hold it.
4. Offline: `cryptsetup reencrypt --encrypt --type luks2
   --reduce-device-size 32M --pbkdf argon2id --pbkdf-memory 262144
   --key-file=- --batch-mode /dev/X` (password on stdin). After a power
   cut the stick asks for the password; the unlock path runs `cryptsetup
   reencrypt --resume-only` first when `luksDump` shows the requirement.
   No TPM enrolment while it runs.
5. Open, mount, `btrfs filesystem resize max`; the encrypted level; console
   hash and phone password removed; maintenance login and TPM as at setup.
6. New `data.partial`, mounted at a temporary place; copy file by file
   (temporary name, fsync, rename); a manifest of verified paths (SHA-256
   read back through the decrypted view) in the index, so a resume skips
   what is done; count and size compared before the switch; rename to
   `data`, record, `password.conf` bound and checked.
7. Immich starts on the encrypted view and answers; only then
   `ALPArchive/files`, `plain-id.txt` and the dump are removed.

The UI must say: phone backups pause meanwhile; it can take long (shown);
after a power cut it continues at the next start once the password is
typed; data written before may still be recoverable from the stick's flash
and the disk (deleting does not erase): for full protection use a new stick
and a freshly formatted disk.

## Code review (2026-10-30): what the implementation adds

- The TPM slot is made only at the very end, so a run cut off after the
  lock began always asks for the password at the next start and resumes
  from the unlock path; a run cut off before cryptsetup began is undone at
  start (Btrfs gets its room back, the step is forgotten).
- Resumes after the copy's rename or the record switch open the existing
  encrypted archive instead of making a new key.
- Before anything changes: room on the disk for a second copy + 1 GiB, and
  no links or special files in the plain folder (they would be lost).
- Each file: a short unique temporary name, fsync of the file and its
  folder, then SHA-256 compared; before deleting the open copy every file
  is compared again through the remounted encrypted archive.
- A failure puts the plain archive back and starts Immich again; the run can
  be started again. While it runs, unlock, the TPM and power-off are refused.
- Not done: the `pg_dump` of Immich's database (the index is encrypted in
  place, so the database is never copied).

## Open points (VERIFY on CI)

- `cryptsetup reencrypt --encrypt` on a mounted-then-unmounted Btrfs after a
  shrink: VERIFY with the cryptsetup in the image (Debian trixie) on CI.
- The time on a slow USB stick (the index holds Immich's database and
  thumbnails) and on a slow disk (a full copy of the photos).
- What the UI shows if the computer is switched off mid-way.

## Consequences

- The stick ends exactly like a level 3 stick made at setup.
- Needs twice the photos' size on the backup disk for a while.
