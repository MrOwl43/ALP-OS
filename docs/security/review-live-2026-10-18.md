<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Security review: ALP-OS Live code (2026-10-18)

Reviewer: security and encryption engineer (independent of the author).
Branch `stream-a/live-hwtest`, commits up to `06756da`. Scope:
`crates/alpctl/src/{archive,wifi,live}.rs`, `crates/alp-hw/src/inspect.rs`,
`image/units/alp-live-*.service`,
`image/extra/usr/lib/udev/rules.d/90-alp-live-protect.rules`,
`image/extra/etc/nftables.conf` (before this review's rewrite),
`image/repart.d/*.conf`. Owner security stance for Live: ADR 0009 #5.

Ranking: **blocker** = can lose or corrupt user data, or leak a secret, in a
normal field test; fix before testers get the image. **should-fix** = real
risk in an unusual but plausible case, or a fix that is cheap now.
**note** = accepted risk, or hardening for later.

Kernel facts used below were checked against linux-6.12.y (Debian kernel
6.12.111): `block/blk-core.c` `bio_check_ro()` only *warns* when a bio
writes to a read-only device; it does not fail it. The read-only flag is
enforced on `write(2)` to the node (`block/fops.c`: `-EPERM`), on rw
mounts and on ioctls, not on writes from a filesystem or driver that was
already writing. `include/linux/blkdev.h`: a partition is read-only if its
own flag **or** the disk's flag is set, so writing one partition needs the
whole disk writable. Partitions re-created by a partition-table re-read
start with the flag clear.

## Blockers

### B1 Hibernated Windows is checked only on the chosen partition

`crates/alpctl/src/archive.rs:82`, `:206-212`, `:220`, `:448-451`.
`hiberfil.sys` exists only on the Windows system volume (C:). If the user
picks D: (another partition, or a second internal disk) while Windows is
hibernated or used Fast Startup, the probe finds no `hiberfil.sys` on D:,
Live writes to D:, and when Windows resumes it writes back its cached view
of D: and corrupts it: the user's own files and the archive. Fast Startup
is on by default on Windows 10/11, so this is the normal case, not an edge.

Fix:
1. Before every rw mount (in `create` and in every `open`), probe **every**
   NTFS partition on every non-stick disk read-only and refuse all NTFS
   targets if any of them has a `hiberfil.sys` starting with `hibr`.
2. If any partition on the computer is BitLocker (cannot be checked), refuse
   NTFS targets with "Windows on this computer is locked (BitLocker); Live
   cannot check that Windows is fully shut down" (BitLocker is out of scope,
   ADR 0009 #4).
3. Also refuse when the target NTFS volume is marked dirty (VERIFY how
   ntfs3 in 6.12 reports it; `ntfsinfo`/`ntfsfix -n` from ntfs-3g are an
   option if ntfs3 does not).
4. QA test: fixture disk with two NTFS partitions, `hiberfil.sys` with
   `HIBR` on p1, target p2: `candidates` and `create` refuse p2, `open`
   refuses an archive on p2.

### B2 The key file can silently land in RAM

`crates/alpctl/src/archive.rs:253-273`, `:427`, `:484-504`;
`image/units/alp-live-state.service`.
`key_file()` and the record write to `/var/lib/alp/live/` whether or not
the stick's `alp-state` partition is mounted there. If `state-mount` failed
(partition not found because the system disk was not detected, ext4 error,
stick read-only), `/var/lib/alp` is a directory on the `/var` tmpfs. Choosing
"a" on the screen then creates a real encrypted archive whose only key is in
RAM: after the next reboot the archive can never be opened. With the Immich
app's "free up space" this loses photos for good.

Fix: one function `state_ready()` that checks `/var/lib/alp` is a mount
point **and** its `st_dev` equals the device number of the `alp-state`
partition on the system disk; `create`, `open`, `key_file`, `write_psk` and
the screen's "a" entry refuse with a plain message and an error code when it
is false. QA test: boot with the state partition's GPT name changed;
"a" must refuse and nothing may be written to any internal disk.

### B3 Losing or re-writing the stick loses the archive (owner decision)

`crates/alpctl/src/archive.rs:253-273`; ADR 0009 #5.
The only copy of the key is the file on the stick. Writing a newer alpha
image to the same stick (the natural way testers update) re-creates
`alp-state` and deletes the key; so does a dead stick. The archive is then
unreadable, by design. Combined with the Immich app's "free up space"
(deletes phone copies after backup) this is permanent photo loss.

This is a product decision, not a code bug. Options, in product terms:
- (a) Keep as is, and say it loudly: the screen and tester guide say "This
  is a test version. Keep your photos on the phone too. Writing a new
  version to this stick erases the archive's key." Cheapest; testers may
  still lose photos.
- (b) Same as (a), plus an update method that keeps `alp-state` (write only
  the system partitions; systemd-sysupdate later, spec block 3).
- (c) Add a second LUKS key slot with a printed recovery key. Contradicts
  ADR 0009 #5 for Live.

Recommendation: (a) now, (b) before a wider field test. Owner to decide.

## Should-fix

### S1 Opening mounts the Windows partition read-write before checking it is the right one

`archive.rs:441-458`. `open()` finds the partition by PARTUUID (first match),
makes it writable and mounts it rw, and only then compares the LUKS UUID.
A cloned disk (same PARTUUID and filesystem UUID, common after a disk
upgrade) gets mounted rw by mistake, or, if the clone was made after the
archive, the clone's copy of the archive is opened and written (two
diverging archives). Fix: require exactly one partition with the recorded
PARTUUID (refuse duplicates); mount read-only, `losetup --read-only`, check
`luksUUID`, detach, then `allow_write` and mount rw.

### S2 Failures leave the disk writable and the partition mounted

`archive.rs:355-430` (`create`), `:452-466` (`open`). Any error after
`allow_write` returns without undoing it: the disk stays writable, the NTFS
partition stays mounted rw (ntfs3 marks it dirty until a clean unmount), the
LUKS mapping may stay open, and the next attempt fails on "already mounted"
until a reboot. The screen then says "Nothing else was changed" (`:624`),
which is not true (`README.txt`, `.partial` and the open mounts remain).
Fix: a guard that calls the `close()` steps on every error path after
`allow_write`; adjust the message to say what is left (the folder).

### S3 Kernel names identify the writable partition

`archive.rs:238`, `live.rs:47-51`. `/run/alp/allowed-partition` holds a
kernel name such as `sdb1`. If the archive disk is unplugged and another
disk is plugged in and gets the same name, `protect-disks` leaves that new
disk writable at disk level and its `sdb1` writable. Fix: store
`<disk>:<diskseq>:<partuuid>` (`/sys/block/<disk>/diskseq`, kernel ≥ 5.15)
and match all three; also validate the stored name is `[a-z0-9]+` before
using it in a path (`live.rs:51`).

### S4 A partition-table re-read makes the other partitions writable

`90-alp-live-protect.rules:4`, `archive.rs:229-250`. On the archive disk the
disk is writable and the other partitions are protected only by their own
flag. Partitions re-created by a re-read (BLKRRPART, udev's synthesized
change after a writer closes the whole-disk node) start writable. Fix: also
run `protect-disks` for `ENV{DEVTYPE}=="partition"` and for `ACTION=="change"`:
`ACTION=="add|change", SUBSYSTEM=="block", ENV{DEVTYPE}=="disk|partition", KERNEL!="loop*|ram*|zram*|dm-*|sr*|nbd*|md*", RUN+="/usr/bin/alpctl live protect-disks"`.

### S5 Two ALP-OS sticks share partition UUIDs

`image/mkosi.conf` (`Seed=`), `archive.rs:494`, `live.rs:392`. The fixed
build seed gives every stick the same PARTUUIDs. `state_mount` and the report
writer find the right partition on the system disk, but then mount it
through `/dev/disk/by-partuuid/…`, which points to whichever stick udev saw
last. With two sticks plugged in, Live can read the other stick's key and
record, or write its key to the wrong stick. Fix: mount `/dev/<kernel name>`
of the partition found on the system disk (add the kernel name to
`alp_hw::PartitionInfo`), and pass `-t ext4` / `-t vfat` explicitly.

### S6 Partition types that must never be targets

`archive.rs:39-43`. Add: Windows LDM metadata `5808C8AA-7E8F-42E0-85D2-E1E90434CFB3`
and LDM data `AF9B60A0-1431-4F62-BC68-3311714A69AD` (dynamic disks, spanned
volumes), Storage Spaces `E75CAF8F-F680-4CEE-AFA3-B001E56EFC2D`, and the MBR
types `0x27` (hidden recovery), `0x42` (LDM), `0x05`/`0x0f`/`0x85` (extended).
Also refuse GPT partitions with attribute bit 0 (platform required) or 62
(hidden), from `ID_PART_ENTRY_FLAGS` (VERIFY the udev key on trixie).

### S7 Read-only probe of ext4 can fail for the wrong reason

`archive.rs:178-197`. An ext4 whose journal needs recovery cannot be mounted
`ro` on a read-only device (the kernel refuses rather than replay), so the
candidate is shown as unusable with a raw `mount failed` text. Fix: probe
ext4 with `-o ro,noload` and report "this Linux partition was not shut down
cleanly" when `needs_recovery` is set (`dumpe2fs -h` or the superblock
feature flags). Never replay a foreign journal.

### S8 Power cut between creating the archive file and writing the record

`archive.rs:420-427`. After the rename and before the record is synced,
`archive.alp` exists but the stick has no record. Next start says "no
archive set up", and `create` refuses because an archive is there: the user
is stuck. Fix: when `has_archive` is true and the stick has the key file,
offer "open the existing archive": check with
`cryptsetup open --test-passphrase --key-file … <loop>` and write the record.

### S9 LUKS header has no backup

`archive.rs:377-395`. The archive sits inside a file on ntfs3/exFAT; a
damaged first 16 MiB (header and key slot) loses everything. Fix: right
after `luksFormat`, `cryptsetup luksHeaderBackup <loop> --header-backup-file
/var/lib/alp/live/archive.luks-header` (mode 0600, fsync). It holds only the
key slot wrapped by the key file, which is already next to it, so it adds
no exposure. Also pin the format explicitly (`--cipher aes-xts-plain64
--key-size 512 --hash sha256`) so a cryptsetup update cannot change it
silently (these are the cryptsetup 2.7.5 defaults; VERIFY in the man page of
the pinned package).

### S10 Closing hides failures and detaches every loop device

`archive.rs:469-481`. Errors are ignored; if `umount` fails (Immich still
writing), the disk is made read-only anyway, which in 6.12 does not stop the
mounted filesystem from writing (see the kernel note above). `losetup -D`
detaches every loop device, not only ours. Fix: stop on the first failure
and report it; detach only the device found by `losetup -j <file>`. Units:
the Immich services must be ordered `After=`/`BindsTo=alp-live-archive.service`
so they stop first, and `alp-live-archive.service` needs a
`TimeoutStopSec=` long enough for a Btrfs unmount on a slow USB disk
(e.g. 5 min) so systemd does not kill it halfway.

### S11 ntfs3 behaviour after a power cut (VERIFY)

`archive.rs:174-176`. ntfs3 does not journal its own metadata changes. After
a power cut the volume stays dirty; Windows runs chkdsk on its next start,
and ntfs3 may refuse a rw mount of a dirty volume without `force`. VERIFY
on 6.12 with a QA power-cut test on an NTFS fixture: (1) does `open` still
work, (2) what message the user sees. Expected user text: "Windows did not
check this disk after a power cut. Start Windows once, then shut it down
fully." Never pass `force`.

### S12 Terminal control characters from devices and Wi-Fi names

`wifi.rs:164`, `:176`, `:200`; `archive.rs:576-590`; `live.rs:295`, `:321`.
SSIDs (any nearby access point), USB disk model strings and DMI strings are
printed to tty1 as they are. `strip_ansi` (`wifi.rs:26-41`) only removes
ESC sequences from iwctl's output and swallows text after a lone ESC. A
crafted SSID or USB stick can redraw the screen (fake "type yes" prompts).
Fix: one `display_safe(s, max)` that replaces control characters
(`char::is_control`, which covers C0 and C1), bidi overrides
(U+202A–U+202E, U+2066–U+2069) and zero-width characters with `?`, and caps
the length; use it for every device-supplied string.

### S13 SSIDs on iwctl's command line

`wifi.rs:201`. No shell is involved (no injection), but an SSID that starts
with `-` may be parsed by iwctl as an option (VERIFY whether iwctl's
`getopt_long` permutes arguments). Fix: connect through iwd's D-Bus API
(`net.connman.iwd.Network.Connect` on the network's object path from
`Station.GetOrderedNetworks`), which also removes the fragile table parsing
(`wifi.rs:46-80`: an SSID equal to "psk", or with trailing spaces, is
misread).

### S14 Wi-Fi passphrase file

`wifi.rs:206-234`. Good: never on argv or in logs, mode 0600. To fix:
(1) write atomically (temp file 0600 in the same directory, fsync, rename,
fsync the directory) so a power cut cannot leave an empty file; (2) VERIFY
that ell's settings parser keeps trailing spaces; if not, escape a trailing
space as `\s` too (QA: passphrase `" ab cd  "` connects); (3) check 8–63
printable ASCII characters (WPA passphrase rule) or 64 hex digits, instead
of byte length.

### S15 Serialise disk-protection changes

`live.rs:42-76`, `archive.rs:229-250`. `protect-disks` (udev, one run per
new disk) and `allow_write` can interleave. The interleavings are fail-safe
(the worst case makes the archive disk read-only again and `create` fails),
but they produce confusing errors. Fix: both take an exclusive `flock` on
`/run/alp/disks.lock`.

## Notes

- N1 While the archive is open the whole-disk node (e.g. `/dev/nvme0n1`)
  is writable (`archive.rs:243`); the kernel cannot do otherwise (see the
  kernel note). Rule for all code: never open a whole-disk node for
  writing. QA: grep test that `alpctl` has no `--setrw` outside
  `allow_write` and never opens `/dev/<disk>` for write.
- N2 Read-only flags do not stop a writer that started before them
  (`bio_check_ro` only warns). The udev rule fires during coldplug, before
  anything mounts internal disks; keep it that way (no automounter, no
  mdadm/lvm2 auto-assembly in the image; today's manifest has neither:
  keep a QA check on the manifest).
- N3 Everything runs as root (`alp-live-screen.service`). Acceptable for the
  text screen; the kiosk must not (docs/security/kiosk-lockdown.md).
- N4 Key handling is sound: 64 bytes from `/dev/urandom`, `create_new`,
  mode 0600, fsync; passed to cryptsetup as a path, never on argv or in a
  log. PBKDF2 with 1000 iterations is fine for a full-entropy 512-bit key
  (cryptsetup's minimum for pbkdf2 is 1000; VERIFY in 2.7.5).
- N5 Anyone holding the stick can read the archive key and remembered
  Wi-Fi passwords (owner decision, ADR 0009 #5). Say so in the tester guide.
- N6 `size_mib << 20` (`archive.rs:538`) wraps on absurd input; the bounds
  check catches it, but use `checked_mul`.
- N7 fallocate on ntfs3 and exFAT (VERIFY in 6.12): confirm the file is
  really allocated and that writes far into it (Btrfs mirror superblocks at
  64 MiB and 256 GiB) do not trigger long zero-filling stalls ("valid data
  length"). QA: 200 GB archive on an NTFS fixture, time `mkfs.btrfs`.
- N8 `console=ttyS0` (`image/mkosi.conf`) starts a serial login on machines
  with a serial port. Root is locked, so harmless today; the admin account
  rules (docs/security/admin-account.md) restrict logins to tty2.
- N9 The previous `nftables.conf` was correct (policy drop, needed ICMP,
  DHCPv6 only from the link). Rewritten in this review for 443 and an
  explicit local-only rule (docs/security/firewall-live.md).
- N10 `repart.d`: no secret is made at build time; `alp-state` is created
  empty and the key is made on the stick at run time. Good. Keep it so (QA:
  the built image contains no `archive.key`, no `*.key`, no `server.key`).
- N11 No command injection found: every external command is run with an
  argument list, never through a shell. Partition labels and SSIDs are only
  printed (S12) or passed as single arguments (S13).
