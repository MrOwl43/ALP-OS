<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# 0014 — Live storage v2: a growing encrypted folder, disk checks, repair, prepare

- **Status:** accepted (owner requirements for ALPHA 0.2, 2026-10-24;
  implemented 2026-10-25, task T12: code and unit tests, not yet run on
  hardware or CI)
- **Date:** 2026-10-24
- **Spec reference:** blocks 4, 5; ADR 0009, ADR 0011 (supersedes 0011 #2
  for new archives); owner decisions 2026-10-24 (passwords and codes, task
  T02, ADR 0013, whose recovery-code storage this ADR implements)

## Context

ALPHA 0.1 kept the archive as one big file (`ALPArchive/archive.alp`,
LUKS2 + Btrfs) that had to be written in full before use: on exFAT
(no fallocate in Linux 6.12) that is a zero fill of many gigabytes. On USB
sticks this took up to an hour, heated the stick and was the source of the
55 % field bug. The owner's binding requirements (2026-10-24):

1. No big empty preallocated archive file. Space and health are checked up
   front instead.
2. A backup disk is usable when it has enough free space and passes a
   read/write test (about 256 MiB written with fsync, read back, verified,
   deleted) at 3 MB/s or more (owner, 2026-10-25; was 5 MB/s); slower or failing disks are refused with a
   clear reason. A SMART health check where the disk supports it (USB SSD
   or HDD through SAT); sticks are skipped.
3. A dirty exFAT/NTFS volume ("not removed safely") is repaired
   automatically after one line of explanation; only a failed repair is
   shown as an error. Never "take it to Windows".
4. "Prepare this disk": an external disk that cannot be used (FAT32,
   unknown format, unreadable) can be erased and formatted as exFAT, behind
   the password, a warning listing what is erased, and slide-to-confirm.

Unchanged: never delete user files; one main password (index on the stick,
TPM auto-unlock, ADR 0011/0012); an alpha user's old LUKS archive must
still open.

## Options considered

1. **Keep LUKS, grow the file** (sparse file + Btrfs, `fallocate` later).
   No: exFAT cannot hold sparse files (the kernel zero-fills on extend), so
   growth means the same slow zero writes, only spread out; a loop device
   on a FUSE-free exFAT file also hides free space from the user.
2. **gocryptfs** (file-level encryption, one encrypted file per file, a
   folder that grows as photos arrive). Debian package, mature (since
   2015, audited 2017), AES-256-GCM for content, EME for names, scrypt for
   the password; works on any filesystem that stores files (exFAT, NTFS,
   ext4). Cost: file sizes, count and folder shape are visible (not
   contents or names); a FUSE daemon; no Btrfs snapshots.
3. **fscrypt** (kernel). No: ext4/F2FS/UBIFS only, not exFAT/NTFS.
4. **CryFS / securefs / rclone crypt.** CryFS hides sizes but its block
   files are slower and less proven on exFAT; securefs and rclone crypt are
   less used for this; none is clearly more mature than gocryptfs.
5. **VeraCrypt container.** Same preallocation problem as option 1, and
   not in Debian main.

## Decision

New archives are a **gocryptfs folder** `ALPArchive/data/` on the backup
disk, mounted decrypted at `/var/lib/alp-pool/files` (parent directory
`/var/lib/alp-pool` root-only, mode 0700). Nothing is preallocated.

Layout on the backup disk (all other files stay untouched):

```
ALPArchive/README.txt                what this folder is (plain text)
ALPArchive/data/gocryptfs.conf       master key wrapped by the archive password
ALPArchive/data/gocryptfs.diriv      (gocryptfs; also the archive's id)
ALPArchive/data/…                    encrypted files and folders
ALPArchive/recovery.conf             same master key wrapped by the recovery secret
```

Keys and passwords:

- **Archive password:** 32 random bytes from the kernel as 64 hex
  characters, in the encrypted index on the stick
  (`/var/lib/alp-index/keys/archive.pass`, mode 0600), next to where the
  LUKS key file was. So the main password (and the TPM, ADR 0012) opens the
  archive through the index, as before; serious encryption (T02) works the
  same way because it only changes how the index opens. gocryptfs reads it
  with `-passfile` (mount) or on stdin (`-init`, `-passwd`); never argv,
  environment or a log.
- **Recovery code (T02):** `recovery::Code::unlock_secret` (BIP-39 seed of
  the 15 words, with the optional passphrase, as 128 hex characters) is the
  password of a **second gocryptfs config** `ALPArchive/recovery.conf`:
  a copy of `gocryptfs.conf` re-wrapped with `gocryptfs -passwd -config`
  (old password then new password on stdin). alpd never sees the master
  key. A new code replaces `recovery.conf` atomically.
- **Recover on a new stick:** copy `recovery.conf`, re-wrap it with the
  recovery secret → a new archive password (stored in the new stick's
  index); exit code 12 means a wrong code. The old `gocryptfs.conf` is
  kept as `ALPArchive/gocryptfs.conf.replaced-<time>` (never deleted).
- **Copies in the index:** `keys/archive-gocryptfs.conf` (the config) is
  copied into the index after every change, like the LUKS header backup
  (review S9). Restoring it is a manual step for now.

Mounting: `systemd-run --scope --unit=alp-archive-files --quiet --collect
-- gocryptfs -q -allow_other -noexec -passfile <index>/keys/archive.pass
-- <disk>/ALPArchive/data /var/lib/alp-pool/files`. The scope keeps the
FUSE daemon alive when alpd or the oneshot unit that started it restarts
(a daemon in alpd's cgroup would be killed with it). Running as root with
`-allow_other`, gocryptfs creates files as the calling user
(`PreserveOwner`) and the kernel checks permissions (`default_permissions`).
Immich (T03, uid 50000) gets `archive::immich_originals_dir()`
(`/var/lib/alp-pool/files/immich`, created and chowned by T03's prepare
step) bind-mounted at `/run/alp/immich/originals`.

exFAT and NTFS have no per-file owners, so the backing disk is mounted
read-write with `uid=50000,gid=50000,fmask=0077,dmask=0077` (exfat and
ntfs3 options): the decrypted view shows uid 50000, only it and root can
write. ext4 keeps real owners. `ARCHIVE_UID` (50000) mirrors Immich's user.

Disk checks before an archive is made (job `archive.create`, new step
`checking`, 8–40 %):

1. **Repair** (step `preparing`): a dirty flag (exFAT VolumeDirty, NTFS
   `$Volume` dirty) is repaired with `fsck.exfat -p` (exfatprogs) or
   `ntfsfix -d` (ntfs-3g) on the unmounted device, after the job message
   "This disk was not removed safely last time. Repairing it first…". The
   flag is read again afterwards; still dirty or a failed tool →
   `ALP-STO-118`. The same repair runs before the archive is opened at
   start-up and before recover (unplugging without "eject" is normal).
   Hibernated Windows on the disk (`ALP-STO-109`) is still refused: no
   repair can make that safe.
2. **SMART** (`smartctl -H -n standby`, then `-d sat` when the first try
   could not talk to the disk; 30 s limit each): exit bit 3 (failing) or
   bit 4 (pre-fail attribute at threshold) → refused, `ALP-STO-117`. Any
   other answer (not supported, a stick, a bridge without SAT) → skipped
   silently.
3. **Read/write test:** `ALPArchive/.alp-disk-test` (256 MiB of a
   pseudo-random pattern), written in 4 MiB blocks with `fsync` every
   32 MiB; the disk is then unmounted and mounted again (so the read comes
   from the disk, not the page cache), read back, compared, deleted.
   Write speed (bytes / time until the last fsync) under 3 MB/s, or more
   than 100 s for the writing, → refused, `ALP-STO-115`; an I/O error or a
   difference → refused as a possibly faulty drive, `ALP-STO-116`.
   Cancel stops it and deletes the test file.

Space: a disk is usable with at least 1.1 GB free (as before); there is no
size to choose. `archive.size_bytes`/`free_bytes` are the backing disk's
(the archive grows into it); the existing low/full levels and Immich's
pause at "full" (reserve 2 %, at least 512 MiB) keep the disk from running
out.

**Prepare this disk** (`POST live/disks/prepare`, job `disk.prepare`):
only an external (USB) disk that is not the ALP-OS stick, has no
operating system, is not write-protected, has no `ALPArchive` folder, no
usable partition, and whose every partition is refused for FAT32, unknown
format or an unreadable/unrepairable volume. Needs the admin password and
the disk's `disk_id` (kernel name plus `diskseq`, new at every plug-in) and
the exact list of partitions shown to the user, so a different disk
plugged in meanwhile is never erased. Then: the disk is made writable
(`/run/alp/allowed-partition` = the disk), `sfdisk --lock --wipe always
--wipe-partitions always --label dos` (GPT above 2 TiB) with one partition
of type 7 (GPT: Microsoft basic data), `mkfs.exfat -L ALPArchive`, read-only
again. Codes: `ALP-STO-119` (failed), `ALP-STO-120` (not allowed).

**Old LUKS archives** (`archive.toml` without `format`, or a disk with
`ALPArchive/archive.alp`) still open, read and write exactly as before
(loop device, cryptsetup, Btrfs, their 64-letter recovery key). No
migration in the alpha (copying a whole archive on a slow stick is the
same heat problem); a 15-word code cannot be bound to them (`recovery/create`
says so). The ALPHA 0.1 creation path (zero fill) is removed.

## Implementation notes (2026-10-25)

- Code: `crates/alp-live/src/archive.rs` (create, checks, repair, open,
  recover, reset, prepare), `vault.rs` (gocryptfs command lines, mount in
  the scope), `job.rs` (`run_code`, `run_input`), `recovery.rs` (`bind`,
  `open` delegate to `archive::bind_recovery` / `recover_with_secret`);
  API in `crates/alp-core/src/live_api.rs`; codes `ALP-STO-115`…`120`.
- The folder is made as `ALPArchive/data.partial` and renamed to `data`
  when gocryptfs is done (the point after which cancel no longer stops the
  job); cancel or failure removes only `data.partial` and
  `.alp-disk-test`. The record (`archive.toml`) has `format = "gocryptfs"`
  and `archive_id` (the folder's `gocryptfs.diriv` as hex), checked on a
  read-only mount before every open and before reset deletes anything.
- Old LUKS archives: their Btrfs stays mounted at `/var/lib/alp-pool`, and
  its `@archive` subvolume is bind-mounted at `/var/lib/alp-pool/files`, so
  `archive::is_open()` (= `/var/lib/alp-pool/files` is mounted),
  `immich_originals_dir()` and the space numbers are the same for both
  kinds.
- A failed repair is remembered per plug-in (`/run/alp/repair-failed/`):
  that volume then shows `ALP-STO-118` in the list and counts as
  unrepairable for "Prepare this disk".
- The write test counts as fast enough when it finished within a second
  (tiny or very fast writes cannot be timed reliably); otherwise
  bytes / seconds ≥ 3 MB/s and at most 100 s.
- `/usr/lib/modules-load.d/alp-fuse.conf` loads `fuse` at boot. alpd runs
  as root, so `-allow_other` needs no `user_allow_other` in
  `/etc/fuse.conf` (that option is only for non-root mounts, fuse(8)).

## Evidence

- Debian trixie snapshot 20261016T023245Z, `main/binary-amd64/Packages`
  (VERIFIED 2026-10-24, .deb SHA-256 checked, copyright files read):
  `gocryptfs 2.5.1-2+b1` (Expat/MIT; depends `fuse3`, `libssl3t64`),
  `fuse3 3.17.2-3` (GPL-2+/LGPL-2.1), `exfatprogs 1.2.9-1+deb13u1`
  (GPL-2+), `ntfs-3g 1:2022.10.3-5+deb13u2` (GPL-2+, LGPL-2+; ships
  `ntfsfix`), `smartmontools 7.4-3` (GPL-2+), `fdisk 2.41.5-0+deb13u1`
  (`sfdisk`, GPL-2+).
- gocryptfs 2.5.1 source (`gocryptfs_2.5.1.orig.tar.xz` from the snapshot,
  read, not run): password from stdin when it is not a terminal, one line
  each (`internal/readpassword`); `-passwd` reads the old password then the
  new one and writes the config via `<config>.tmp` + fsync + rename
  (`internal/configfile`); `-init` needs an empty CIPHERDIR (exit 6);
  exit 12 = wrong password; as root it mounts directly (`DirectMount`), and
  with `-allow_other` sets `PreserveOwner` and `default_permissions`
  (`mount.go`); the daemon moves its stdout/stderr to `logger`. Man page
  flags used: `-init -passwd -config -passfile -allow_other -noexec -q`.
- fsck.exfat(8): `-p` repairs without questions when safe. ntfsfix(8):
  `-d` clears the dirty flag if the volume can be fixed. smartctl(8):
  exit status bit mask (bit 3 failing, bit 4 pre-fail); `-n standby`;
  `-d sat`. sfdisk(8): `--lock`, `--wipe`, `--wipe-partitions`, `--label`.
  mkfs.exfat(8): `-L`.
- **Not verified (no hardware or VM runs allowed now; QA and CI later):**
  only unit tests ran (`just check`: parsing, decisions, command lines,
  the disk test on a temporary folder). Not run at all yet:
  - gocryptfs `-init`, `-passwd -config` (recovery.conf, recover) and the
    mount through `systemd-run --scope` on the real image; the scope
    keeping the daemon through an alpd or alp-live-archive restart; the
    unmount waiting for the daemon before the disk is unmounted;
  - gocryptfs speed on exFAT USB sticks and with Immich's many small files
    (thumbnails stay on the index; originals are fewer and larger);
  - exFAT/ntfs3 `uid=/gid=/fmask=/dmask=` under gocryptfs with Immich
    writing; whether gocryptfs' owner change (`PreserveOwner`) of a file
    created by root is refused by exFAT (EPERM) and how gocryptfs reacts.
    QA review 2026-10-25: exfat and ntfs3 (Linux 7.0 modules, parameter
    names read with `modinfo`/`strings`) have no `quiet` option, so this
    cannot be papered over with a mount option (an unknown option fails
    the mount). gocryptfs 2.x is expected (from memory of its source, not
    re-read) to create files with the caller's ids by switching the
    thread's euid (`syscallcompat.*User`), not by `chown`: then nothing is
    refused, files show uid 50000 anyway, and only callers with uid 50000
    or root can write (fmask/dmask 0077). A `chown` to another uid (an app
    doing it on purpose) still fails with EPERM. Check with a non-50000
    container uid writing into `/data/upload`;
  - the 256 MiB test file and `README.txt` are now replaced, never
    followed when a symbolic link of that name is there (ext4/NTFS disks
    from elsewhere), and `ALPArchive` itself must be a real folder; reset
    never deletes through a linked `ALPArchive` (QA review 2026-10-25);
  - the repair (`fsck.exfat -p` / `ntfsfix -d`, up to 10 minutes) is not
    stopped by cancel (killing a repair halfway is worse); cancel takes
    effect right after it;
  - "Prepare this disk" checks the disk's `diskseq` again right before
    `blockdev --setrw`, `sfdisk` and `mkfs.exfat`; the small window
    between that check and the program opening `/dev/<name>` remains
    (`/dev/disk/by-diskseq/<n>` would close it, if udev made it in time);
  - renaming `gocryptfs.conf.recovered` over `data/gocryptfs.conf` on
    exFAT/NTFS (rename onto an existing file);
  - exFAT is case-insensitive while gocryptfs names are base64 (a clash
    needs two 128-bit-random encrypted names that differ only in case:
    ignored);
  - `fsck.exfat -p` clearing VolumeDirty and its exit codes; `ntfsfix -d`
    on ntfs3-written volumes; the dirty flag read again afterwards;
  - `smartctl -H -n standby` (and `-d sat`) through common USB bridges and
    on sticks (expected: skipped);
  - the 256 MiB test on a USB 2 stick (about 90 s at 3 MB/s) and the
    unmount/mount between writing and reading;
  - `sfdisk --lock --wipe always --wipe-partitions always` with the one-line
    script (`type=7` / the GPT basic-data type), the kernel re-reading the
    table, udev not making the new partition read-only before
    `blockdev --setrw`, `mkfs.exfat -L ALPArchive`, and the disk showing up
    again as a usable exFAT candidate;
  - an old (ALPHA 0.1) LUKS archive opening with the new bind mount, and
    Immich (T03) using `/var/lib/alp-pool/files/immich` on it;
  - the UI screens (no JavaScript runtime on the development VM; the mock
    covers the flows).

## Consequences

- Creating an archive takes about a minute (checks) instead of up to an
  hour; the stick is written only as photos arrive.
- No Btrfs snapshots or checksums on new archives (spec block 4 wants
  snapshots): Immich's 30-day trash is the only undo in the alpha.
  Revisit for the installed system (ADR 0009 stays for it).
- File sizes, counts and dates on the backup disk are visible to someone
  holding the disk; names and contents are not.
- The image gains gocryptfs, fuse3, exfatprogs, ntfs-3g, smartmontools,
  fdisk (about 15 MB installed).
- UX: no size chooser; a "checking the disk" step; a one-line repair note;
  a "Prepare this disk" danger flow (password, list of what is erased,
  slide to confirm).
