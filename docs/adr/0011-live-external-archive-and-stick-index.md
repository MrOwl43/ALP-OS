<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# 0011 — Live storage: archive on an external disk, encrypted index on the stick

- **Status:** accepted (owner decisions 2026-10-19, relayed by the
  coordinator; supersedes ADR 0009 #2 for targets and #5 for the key file)
- **Date:** 2026-10-19
- **Spec reference:** blocks 4, 5, 8; ADR 0009; security review
  `docs/security/review-live-2026-10-18.md` (B1, B2)

## Decisions

1. **Disks with an operating system are never written.** A disk is refused
   as a whole when any partition has an OS type (EFI System, Microsoft
   reserved/recovery, Windows LDM metadata/data, Storage Spaces, Linux
   root or /usr Discoverable Partition types for x86-64, x86, arm64) or a
   filesystem with `\Windows\System32` (any case), `/etc/os-release` or
   `/usr/lib/os-release` (read-only probe). Internal disks are not offered
   at all and stay read-only always (`allow_write` refuses them).
2. **The archive (originals) goes on an external disk or USB stick** the
   user plugs in (USB transport, not the boot stick): the encrypted
   container file `ALPArchive/archive.alp` (LUKS2 + Btrfs, as ADR 0009),
   up to the external filesystem's free space minus the reserve. Never
   delete or format. B1 stays: no Windows volume (NTFS, exFAT) is written
   while any NTFS partition on the computer is hibernated / Fast Startup,
   or while a BitLocker partition exists (cannot be checked).
3. **The index lives on the boot stick:** a partition `alp-index` (GPT type
   `A3E1C6B2-5B7E-4F0A-9C1D-6B2F0E8A4D11`, ALP-OS's own) that
   `systemd-repart` adds in the stick's free space at first start
   (`/usr/lib/alp/repart-index.d`; alpd passes only the boot stick's disk).
   It holds Immich's Postgres, thumbnails, encoded videos, profile images
   and database dumps, alpd's Immich secrets, the chain words, and **the
   archive's key file**. Immich's originals (`library/`, `upload/`) stay on
   the external archive.
4. **The index is encrypted with the user's password** (owner: must be
   encrypted): LUKS2 with one passphrase key slot, cryptsetup defaults
   (argon2id), Btrfs with `compress=zstd:3` inside. The kiosk asks for the
   password at every start; alpd passes it to `cryptsetup` on stdin only
   (never argv, environment or a log). For the alpha the same password is
   the admin (maintenance) login's (`docs/security/admin-account.md`), set
   once at first start. The stick alone therefore no longer opens the
   archive; without the password, index and archive are unreadable.
5. **Non-secret state stays on `alp-state`:** archive record (where the
   archive is), index record (`index.toml`: the index partition's
   PARTUUID), admin password hash, remembered Wi-Fi networks, hardware
   reports.
6. **Never run full** (owner requirement): live/state reports size, free
   and a level (`ok`, `low` below the profile's `capacity_low_percent`,
   `full` below a reserve of 2 %, at least 512 MiB) for the index and the
   archive. At `full` alpd pauses Immich (uploads stop with a clear
   message on the screen) until space is back.
7. **The index location is one config value** (`index.toml`); the mount
   point `/var/lib/alp-index` is fixed, so moving the index is a copy plus
   a config change.

## Later (not in the alpha)

- Move the index by itself to another disk (or the archive disk) when the
  stick fills; stronger compression (`zstd:15`, recompress old thumbnails);
  several archive disks (Btrfs multi-device) for scale.
- Recovery when the password is forgotten (a recovery key, spec block 5);
  today a forgotten password means index and archive are lost.
- Separate the admin login password from the index passphrase.

## Consequences

- First start asks for a password before anything else is stored.
- Losing the stick loses the index (thumbnails and database can be rebuilt
  from the originals by Immich; the archive's key cannot): the alpha
  warning text stays ("Keep your photos on your phone too").
- Immich's start order: index unlocked → archive open → Immich.
