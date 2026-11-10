<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Plan: ALP-OS Live step 2 — the encrypted archive

Status: draft for review. Spec: overview 1.2, blocks 3, 4, 5. Binding owner decision 2026-10-18:
Live may **write files** to disks already in the computer, **never deletes or formats** them.
Versions: VERIFIED = read from the pinned trixie snapshot 20261016T023245Z (package cache of the
image build: man pages, `/boot/config-6.12.111+deb13-amd64`); VERIFY = still to check.

## 0. Conflicts to record first (stop-and-report items)

1. Spec 1.2 and ADRs 0006/0007 say Live "never writes to internal disks, data pool on the stick".
   The 2026-10-18 decision changes this. New **ADR 0009** records it; the owner updates the spec.
2. The disk-layout contract allows only `alp-data` partitions as pool members. A container file is a
   new `PoolMember::ContainerFile { disk, partition_uuid, path }` plus a `check_member` rule: a
   contract change (ADR 0004 rule 13), one PR updating `alp-hw`, `alp-storage`, `alpctl`.
3. Spec block 5 "wrap key sealed to TPM2" is realised with `systemd-creds` (section 4), not own code.

## 1. Where the archive lives

- User picks one partition from a list of **safe** candidates (section 5). On it Live creates folder
  `ALPArchive\` with `archive.alp` (the container) and `README.txt` (plain words: "Encrypted
  ALPArchive. Deleting this file deletes your archive."). Nothing else on the partition is touched.
- Fallback when no partition is safe: the stick. At first use, systemd-repart adds a partition
  `alp-data` in the free space **of the stick only** (Live's own medium; the system disk found as in
  ADR 0007). LUKS2 goes straight onto that partition (same as the installed layout; no file).
- Live state on the stick: new build-time partition `alp-state` (ext4, 256 MiB, `linux-generic`,
  repart.d/40-live-state.conf), mounted at `/var/lib/alp` via `alp_core::state::paths()`.

## 2. Making exactly one partition writable

Kernel rule (VERIFY on 6.12 by test T-LA-09): a partition is read-only if its own flag **or** its
disk's flag is set, so the chosen partition P can only be written if its disk D is set writable.
New `alpctl live allow-write <PARTUUID>` (run by the archive service, never by hand):
1. Re-inspect disks; refuse if P is on the system disk, missing, or fails section 5 again.
2. `blockdev --setro` every **other** partition of D; read `/sys/class/block/<p>/ro` = 1 for each.
3. `blockdev --setrw D`, `blockdev --setrw P`; check sysfs: D 0, P 0, all others still 1.
4. Any failure: `blockdev --setro D` and refuse (`LIVE-1xx`, stream A range).
5. Write `/run/alp/allowed-partition` (PARTUUID). `alpctl live protect-disks` and the udev rule read
   it; the rule is extended to partition `add|change` events so a partition rescan on D re-protects
   every partition except P (VERIFY that flags reset on rescan; test covers it).
Residual risk: while open, the whole-disk node `/dev/D` is writable (partition table area). Nothing
in ALP-OS opens it; T-LA-09 proves the bytes outside P are unchanged. Every other disk stays read-only.
Host mount: `/run/alp/host/<PARTUUID>`, `nosuid,nodev,noexec`, first read-only (checks), then rw.

## 3. Container format, size, filesystem inside

- **Fixed size, fully allocated, not sparse.** Reasons: (a) exFAT has no sparse files; (b) a sparse
  file can hit "host disk full" mid-write, which turns into I/O errors under Btrfs; (c) once the
  space is allocated, normal archive writes change no metadata of the host filesystem, which makes
  non-journaling hosts (exFAT) and NTFS much safer on power loss. Allocation: `fallocate`; where the
  host driver lacks it, write zeros with a progress bar (VERIFY fallocate on ntfs3 and exfat 6.12).
- Size: user picks with a slider; default and reserve in Owner question 1. Minimum 32 GiB.
- **Growable later, offline, by explicit action:** extend file, `cryptsetup resize`,
  `btrfs filesystem resize max`. Not in step 2 (needs its own test).
- Stack: file → loop device (`losetup --sector-size 4096`, VERIFY flag in util-linux 2.41.5) →
  LUKS2 → Btrfs. Kernel: `BLK_DEV_LOOP=m`, `DM_CRYPT=m`, `BTRFS_FS=m`, `CRYPTO_ADIANTUM=m`,
  `CRYPTO_CHACHA20=m` (VERIFIED, kernel config 6.12.111).
- LUKS2 (cryptsetup 2.7.5, VERIFIED libcryptsetup12 2:2.7.5-2; cryptsetup CLI and flags VERIFY):
  `luksFormat --type luks2 --sector-size 4096 --cipher aes-xts-plain64 --key-size 512` with AES-NI,
  else `--cipher xchacha12,aes-adiantum-plain64 --key-size 256` (VERIFY with `cryptsetup benchmark`
  in VM). Slot 0: pool key (64 random bytes, `PoolKey::write_key_file`, stdin/`--key-file=-`, never
  argv), cheap KDF `--pbkdf pbkdf2` (VERIFY minimum iterations). Slot 1: recovery, argon2id default.
  Header backup (`luksHeaderBackup`) kept on the stick's `alp-state` (it holds no usable key alone).
- Btrfs (btrfs-progs 6.14-1, VERIFIED in cache): metadata `dup`, data `dup` (spec block 4, single
  disk), subvolumes `@archive @snapshots @system-backup @apps`, `noatime`, mounted at
  `/var/lib/alp-pool`. Compression: decided later by benchmark ADR (spec). Never `check --repair`.

## 4. Keys (Standard tier; the project never holds a key)

- Generated by `alp-crypto` (random from `getrandom`): pool key 512-bit, recovery key 256-bit.
- **TPM2 present:** pool key sealed with `systemd-creds encrypt --with-key=tpm2 --tpm2-pcrs=7
  --name=alp-pool` (systemd 257.13, VERIFIED in man page: values `host|tpm2|host+tpm2|null|auto`,
  PCR 7 is the default; `--tpm2-public-key` exists for signed policies). Blob stored on the stick
  with `SealedBlob::store` (`Tpm2Seal` backend). PCR 7 = Secure Boot state and keys: survives
  Live updates; changes when Secure Boot or its keys change. PCR 11 with a signed policy comes
  when image signing lands (block 1 step 8). Plain consequence: the blob opens only on this
  machine's TPM; someone holding both the laptop **and** the stick can open it.
- **No TPM:** pool key stored as keyfile on the stick (`PassphraseOrKeyfileSeal`, mode 0600).
  Risk said in one sentence on screen: "Keep the stick apart from the computer: together they open
  the archive." A copied stick opens the archive; a lost stick needs the recovery key.
- **Unseal fails** (other machine, TPM cleared, Secure Boot toggled, firmware reset): ask for the
  recovery words, open slot 1, re-seal for the current TPM, ledger event. Never a silent fallback.
- **Recovery key:** 24 BIP39 English words (256-bit entropy + checksum; crate `bip39`, VERIFY
  version/licence) and a QR of the same words (crate `qrcode`, VERIFY). The LUKS slot-1 secret and
  the state-backup key are HKDF-SHA256 outputs with distinct context strings (crate `hkdf`, VERIFY;
  RFC 5869 test vectors). Shown once on the text screen (6×4 word grid; QR from Unicode half blocks,
  VERIFY glyphs in TER16x32), never written to the stick, a log or the ledger.
  Confirmation: Owner question 3. Phone escrow follows with pairing (step 3).

## 5. Refusing unsafe partitions (`alp_hw::live_target::assess`, pure function + probes)

Probe read-only first (`blkid -p`, read-only mount). Refuse with a plain reason when:
- system disk, ESP, MSR, Windows Recovery, swap, LVM/md/LUKS members, `alp-state`;
- **BitLocker** (`blkid` TYPE `BitLocker`, VERIFY in util-linux 2.41.5): "Windows has locked this
  drive (BitLocker)." Likely common: many new Windows 11 laptops encrypt by default (Owner Q 2);
- **NTFS with Windows hibernated or Fast Startup:** `hiberfil.sys` header non-empty (same check
  ntfs-3g does, VERIFY format) or volume dirty flag. Message: "Shut down Windows fully: hold Shift
  while clicking Shut down." Checked **every boot**, not only at creation;
- FAT32/vfat (4 GiB file limit), read-only media, ext4 with error state, Btrfs/XFS/other (not in v1);
- free space below size + reserve.
Drivers: **ext4** (journaled, preferred), **NTFS via kernel `ntfs3`** (`NTFS3_FS=m`, VERIFIED;
no FUSE, avoids loop-over-FUSE writeback deadlocks; mount without `force` so dirty volumes stay
refused, VERIFY behaviour), **exFAT** via kernel `exfat` (`EXFAT_FS=m`, VERIFIED) allowed only because
the file is preallocated; warned as weaker. Spike before coding: ntfs3 vs ntfs-3g on power-cut
tests (T-LA-06); if ntfs3 corrupts, switch to ntfs-3g and record why in ADR 0009.

## 6. Crash safety (power loss)

- Btrfs is copy-on-write: after a cut it opens at the last commit. Flushes must reach the host
  disk: loop turns flush into fsync of the file (VERIFY on 6.12 for ext4, ntfs3, exfat; T-LA-06).
- Creation is resumable: work on `ALPArchive\archive.alp.partial`, steps recorded in
  `/var/lib/alp/live/archive.toml` (atomic write, temp+fsync+rename); final rename only when LUKS,
  slots, Btrfs and subvolumes exist and the state record is synced. After a cut, Live reuses its own
  `.partial` file (overwrites it, deletes nothing).
- NTFS left dirty by a cut: refused until Windows checks it (Owner question 4).

## 7. Re-finding and unlocking on the next boot

`alp-live-archive.service` (after `alp-live-protect`, state mount, network not needed):
1. Read `archive.toml`: host PARTUUID, filesystem UUID, path, LUKS UUID, seal kind.
2. Find P by PARTUUID; check filesystem UUID; run section 5 checks again; `allow-write`; mount.
3. `cryptsetup isLuks` and LUKS UUID must match; `losetup`; unseal; `cryptsetup open
   --key-file=-`; mount Btrfs subvolumes. Screen shows "Archive open" with size and free space.
4. Not found (disk missing, other computer): plain message, nothing written. On a fresh stick with
   no record: scan safe partitions read-only for `ALPArchive\archive.alp` with a LUKS2 header and
   offer to open it with the recovery words, then re-seal to this stick and TPM.
Shutdown: unmount Btrfs, `cryptsetup close`, detach loop, unmount host, `setro D`.

## 8. Code and files

`alp-crypto`: `create_recovery`, `export_recovery`, `verify_recovery`, `Tpm2Seal` and
`PassphraseOrKeyfileSeal` (call `systemd-creds`). `alp-hw`: `live_target` assessment + `check_member`
rule. `alp-storage`: `ContainerBackend` (losetup/cryptsetup/btrfs, machine-readable output only).
`alpctl live`: `allow-write`, `archive create|open|close`, screen pages. `image/`: `a-live.conf`
packages (cryptsetup, btrfs-progs, exfatprogs, tpm2 libs), repart.d/40-live-state.conf, units, udev.
New dependencies (bip39, qrcode, hkdf, getrandom) pinned with one-line reasons and licence check.

## 9. Tests (QEMU/OVMF, swtpm, virtual disks only; harness may format **virtual** disks)

- T-LA-01 ext4 target, swtpm: create, write files, reboot, opens unattended, files intact.
- T-LA-02 no TPM: keyfile path, same as 01.
- T-LA-03 NTFS (made with mkntfs in the harness): clean ok; hibernation header refused; dirty refused.
- T-LA-04 BitLocker signature fixture refused; FAT32, read-only, too-full partitions refused.
- T-LA-05 exFAT target with preallocated file works.
- T-LA-06 power cut (kill QEMU during writes) ×20 per host type: opens, scrub clean, fsynced files there.
- T-LA-07 new swtpm state: recovery words open, re-seal, next boot unattended.
- T-LA-08 stick booted in a second VM: plain message; stick-fallback archive opens with words.
- T-LA-09 protection: other partitions of D and other disks refuse writes (`dd` fails), checksums of
  every byte outside the container file unchanged; partition rescan keeps them read-only.
- T-LA-10 secret scan of all disk images, journal and screen log: no pool key, recovery bytes or words.
- T-LA-11 stick fallback: repart grows only the stick; internal disks bit-for-bit unchanged.
- Unit: target assessment table, BIP39 official vectors, HKDF RFC 5869 vectors, state-file round trip.

## Owner questions

1. **How much of the user's disk may the archive take?** a) user chooses, default 50 % of free
   space, always leaving 20 GB free for Windows; b) fixed default 100 GB; c) all free space minus
   20 GB. Recommend **a**: Windows keeps working and the user stays in control.
2. **Laptops where Windows locked the disk (BitLocker, often on by default):** a) archive on the
   stick or an external USB disk, with a short guide to turn BitLocker off for those who want the
   internal disk; b) stick only. Recommend **a**. (Also: may an external USB disk be used? Recommend yes.)
3. **Recovery words before the phone exists:** a) user must re-type 3 of the 24 words before the
   archive is used; b) show once, no check (spec Standard tier). Recommend **a** until phone escrow
   exists: without it a lost stick on a non-TPM laptop would lose the archive.
4. **After a power cut on a Windows disk:** a) ask the user to start Windows once so it checks the
   disk, then restart ALP-OS; b) open the archive anyway. Recommend **a** (stability first).
5. **One stick, one archive?** On another computer the stick shows "your archive is on another
   computer" and does not create a second one. Recommend yes, keeps the product simple.
