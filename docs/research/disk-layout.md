<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Research: disk layout for an installed ALP-OS system

*Researcher report, 2026-10-16, for the disk-layout contract (NEXT_TASK step
0.4). Target: Debian 13 "trixie", systemd 257.13. Facts carry source links;
"Assessment" sections are judgement. Saved by the lead; decisions taken from
it are in ADR 0004 and docs/DECISIONS-NEEDED.md.*

## Summary

- **Types:** standard Discoverable Partitions types for ESP, root and
  root-verity (no root-verity-sig in v1); `linux-generic` for `state` and the
  data area. Not `var`/`srv`/`home`: those are auto-mounted, and LUKS on them
  is auto-unlocked at boot.
- **Updates:** systemd-sysupdate 257 never creates partitions. The second
  slot must exist labelled `_empty`, and the first slot must carry a label
  matching the update pattern (e.g. `alp-os_<version>`), or the second update
  fails. Create both slots at install, not with repart on first boot.
- **ESP:** two current UKIs (132.8 MiB each) fit in 512 MiB with ~245 MiB
  spare, but the UKI grew 56 → 133 MiB during block 1 and an ESP can never be
  grown after install. Recommends 1 GiB (deviation from spec: owner decision).
- **Sizes:** ESP 1 GiB; two slots of 4 GiB root + 64 MiB verity; `state`
  2 GiB; rest unallocated (dedicated system disk) or one data partition
  (single disk).
- **Spec conflicts:** nine listed in section 5; none blocks the layout.

## 1. Partition types

### Facts

| Role | DPS name / repart identifier | Type UUID |
|---|---|---|
| ESP | `SD_GPT_ESP` / `esp` | `c12a7328-f81f-11d2-ba4b-00a0c93ec93b` |
| Root (x86-64) | `SD_GPT_ROOT_X86_64` / `root`, `root-x86-64` | `4f68bce3-e8cd-4db1-96e7-fbcaf984b709` |
| Root verity (x86-64) | `SD_GPT_ROOT_X86_64_VERITY` / `root-verity` | `2c7357ed-ebd2-46d9-aec1-23d437ec2bf5` |
| Root verity signature (x86-64) | `SD_GPT_ROOT_X86_64_VERITY_SIG` / `root-verity-sig` | `41092b05-9fc8-4523-994f-2def0408b176` |
| `/var` | `SD_GPT_VAR` / `var` | `4d21b016-b534-45c2-a9fb-5c16e091fd2d` |
| `/srv` | `SD_GPT_SRV` / `srv` | `3b8f8425-20e0-4f3b-907f-1a25a76f98e8` |
| Generic Linux data | `SD_GPT_LINUX_GENERIC` / `linux-generic` | `0fc63daf-8483-4772-8e79-3d69d8477de4` |

Source for the table: [Discoverable Partitions Specification (DPS)](https://uapi-group.org/specifications/specs/discoverable_partitions_specification/).

- The built image (`out/alp-os.raw`, `sfdisk -d`) already has `esp`,
  `root-x86-64` and `root-x86-64-verity`, the latter two with attribute bit 60
  (read-only). DPS: bit 63 no-auto, bit 60 read-only, bit 59 grow-file-system;
  read-only is recommended on verity partitions ([DPS](https://uapi-group.org/specifications/specs/discoverable_partitions_specification/)); repart sets it by
  default for verity types ([repart.d(5) 257](https://manpages.debian.org/trixie/systemd-repart/repart.d.5.en.html)).
- Root partition UUID = first 128 bits of the root hash; verity UUID = last
  128 bits ([DPS](https://uapi-group.org/specifications/specs/discoverable_partitions_specification/); [repart.d(5) `Verity=`](https://manpages.debian.org/trixie/systemd-repart/repart.d.5.en.html)). With `roothash=` on the command line,
  systemd finds data and hash partitions by those UUIDs
  ([systemd-veritysetup-generator(8)](https://manpages.debian.org/trixie/systemd-cryptsetup/systemd-veritysetup-generator.8.en.html)). Each UKI therefore points at its own slot.
- systemd-gpt-auto-generator auto-mounts the first `home`/`srv`/`var`/`tmp`
  partition on the boot disk and unlocks it if it is LUKS; the ESP is
  automounted on `/efi` or `/boot` ([systemd-gpt-auto-generator(8) 257](https://manpages.debian.org/trixie/systemd/systemd-gpt-auto-generator.8.en.html)).
- A `var` partition is only mounted if its UUID equals
  `HMAC-SHA256(machine-id, type-uuid)` truncated ([DPS](https://uapi-group.org/specifications/specs/discoverable_partitions_specification/)).
- `linux-generic`: "No automatic mounting takes place for other Linux data
  partitions… The installer needs to mount them explicitly" ([DPS](https://uapi-group.org/specifications/specs/discoverable_partitions_specification/)).

### Assessment

- ESP, root, root-verity: standard types (already used in `image/repart.d/`).
- No root-verity-sig in v1: the roothash travels inside the UKI, and once the
  UKI is signed its signature covers the hash.
- `state`: `linux-generic`, label `alp-state`, mounted explicitly. `var` is
  unsuitable: (1) its mount needs `/etc/machine-id`, while the `/etc` overlay
  upper layer lives on `state`; (2) ALP-OS wants `/var/lib/alp` and the `/etc`
  upper layer, not all of `/var`; (3) gpt-auto mounts `/var` after the initrd,
  but `state` is needed early. The UKI command line is identical on every
  machine, so `state` must be found by something install-independent (label,
  or type + label). Mechanism: block 3, with a test.
- Data: `linux-generic`, label `alp-data`, one per data disk, LUKS2 inside,
  opened only by `alp-storage`. `srv`/`home` would make systemd try to unlock
  LUKS at boot and wait for a key, breaking Section 6 ("A pool that cannot be
  opened never blocks boot"). A GPT with one partition (rather than
  whole-disk LUKS) keeps single-disk and data-disk handling the same.

## 2. systemd-sysupdate 257: A/B slots and UKIs

### Facts

Man pages: [sysupdate.d(5)](https://manpages.debian.org/trixie/systemd-container/sysupdate.d.5.en.html), [systemd-sysupdate(8)](https://manpages.debian.org/trixie/systemd-container/systemd-sysupdate.8.en.html). In trixie,
sysupdate ships in `systemd-container` ([filelist](https://packages.debian.org/trixie/amd64/systemd-container/filelist)); `image/mkosi.conf` does not
install it yet.

- "Partitions must exist already, and a special partition label `_empty` is
  used to indicate empty partitions" ([sysupdate.d(5) `Path=`](https://manpages.debian.org/trixie/systemd-container/sysupdate.d.5.en.html)). Only partitions
  of the right type labelled `_empty` are candidates; the smallest large
  enough is chosen ([sysupdate-partition.c v257](https://github.com/systemd/systemd/blob/v257/src/sysupdate/sysupdate-partition.c)). One instance must always
  remain ([sysupdate-transfer.c v257](https://github.com/systemd/systemd/blob/v257/src/sysupdate/sysupdate-transfer.c)).
- The GPT label carries the version, e.g. `foobarOS_@v` / `foobarOS_@v_verity`
  with `MatchPartitionType=root` / `root-verity`, `PartitionFlags=0`,
  `ReadOnly=1`; `@u` in the source name fixes the partition UUID so
  `roothash=` finds the slot ([sysupdate.d(5) Example 1](https://manpages.debian.org/trixie/systemd-container/sysupdate.d.5.en.html)). GPT labels are at most
  36 characters ([gpt.h v257](https://github.com/systemd/systemd/blob/v257/src/shared/gpt.h)).
- Transfers run in alphabetical order; data is written first, then labels and
  names are switched in order with disk syncs (not atomic) ([sysupdate.d(5)](https://manpages.debian.org/trixie/systemd-container/sysupdate.d.5.en.html)).
  A power cut during writing leaves the target `_empty`.
- `ProtectVersion=%A` needs `IMAGE_VERSION=` in os-release; the image sets no
  `ImageVersion=` yet.
- `Verify=yes` checks `SHA256SUMS.gpg` with `gpg` against
  `import-pubring.gpg` ([sysupdate.d(5)](https://manpages.debian.org/trixie/systemd-container/sysupdate.d.5.en.html); [pull-common.c v257](https://github.com/systemd/systemd/blob/v257/src/import/pull-common.c)).
- `Path=auto` is the disk holding the booted root; an explicit whole-disk
  `Path=` and `--definitions=` / `--transfer-source=` (v257) also exist.
- UKI transfer example: `Type=regular-file`, `Path=/EFI/Linux`,
  `PathRelativeTo=boot`, `MatchPattern=foobarOS_@v+@l-@d.efi foobarOS_@v+@l.efi foobarOS_@v.efi`,
  `TriesLeft=3`, `TriesDone=0`, `InstancesMax=2` ([sysupdate.d(5)](https://manpages.debian.org/trixie/systemd-container/sysupdate.d.5.en.html)).
- Boot counting: `+LEFT[-DONE]` in the file name; entries with 0 tries left
  sort last; systemd-bless-boot drops the counter after `boot-complete.target`
  ([Automatic Boot Assessment](https://systemd.io/AUTOMATIC_BOOT_ASSESSMENT/); [BLS](https://uapi-group.org/specifications/specs/boot_loader_specification/)). systemd-bless-boot is in trixie's
  `systemd-boot` package.
- systemd-repart never shrinks, moves or deletes partitions; `Label=` applies
  only to new partitions, otherwise the label is the type name (not `_empty`)
  ([repart.d(5) 257](https://manpages.debian.org/trixie/systemd-repart/repart.d.5.en.html)). Debian enables `systemd-repart.service` at boot.

### Assessment

- Labels: A `alp-os_<v>` / `alp-os_<v>_verity`; B `_empty`. Versions of digits
  and dots only (e.g. `2026.10.16.1`).
- The A slot and its UKI must use sysupdate's patterns after install, or the
  second update fails with "no available partition" and stray UKIs remain.
- Create B at install (one tested code path; no repart at every boot on the
  installed system).
- Preferred install method: repart creates all slots `_empty`, then the
  installer runs `systemd-sysupdate --definitions=<install set>` against the
  target disk with local sources, so the first install uses the same code as
  every update. Local `regular-file` sources are not verified by sysupdate,
  so the installer checks payload hashes itself.
- Transfer order: verity, root, UKI last.

## 3. Does a 512 MB ESP hold two UKIs?

- Spec block 2: ESP 512 MB. Measured: UKI 139,213,824 B (132.8 MiB; initrd
  ≈121 MiB compressed); systemd-boot 124,416 B; ESP built with 4096-byte
  sectors. An earlier build the same day had a 56 MiB UKI.
- BLS: use the ESP as `$BOOT` if "large enough (let's say at least 1G)"
  ([BLS](https://uapi-group.org/specifications/specs/boot_loader_specification/)).
- Two UKIs + boot loader + FAT overhead ≈ 267 MiB of 512 MiB; the largest UKI
  for which two still fit is ≈255 MiB.

Assessment: fits today, but the ESP is permanent; recommend 1 GiB for
installed systems (owner decision) and shrinking the initrd (block 1).

## 4. Recommended order and sizes

| # | Role | Type | Label at install | Size |
|---|---|---|---|---|
| 1 | ESP | `esp` | `esp` | 1 GiB (spec: 512 MB, owner decision) |
| 2 | root slot 1 | `root` | `alp-os_<v>` | 4 GiB |
| 3 | verity slot 1 | `root-verity` | `alp-os_<v>_verity` | 64 MiB |
| 4 | root slot 2 | `root` | `_empty` | 4 GiB |
| 5 | verity slot 2 | `root-verity` | `_empty` | 64 MiB |
| 6 | state | `linux-generic` | `alp-state` | 2 GiB |
| 7 | data (single disk only) | `linux-generic` | `alp-data` | rest |

- Root 4 GiB: base root 246 MiB today; kiosk builds 878–977 MiB; slots can
  never grow. If Immich/PostgreSQL container images ship inside root: 8 GiB
  root + 128 MiB verity.
- Verity 64 MiB covers up to ≈7.9 GiB of root (measured ratio 1.9 MiB per
  246 MiB).
- State 2 GiB: SQLite, ledger, wrapped keys, `/etc` upper layer, all small.
- Dedicated system disk ≥32 GB: ≈11.1 GiB used, rest unallocated (state is
  last, so it can grow later). Single disk: data takes the rest; suggest a
  minimum disk of 64 GB (owner decision).

## 5. Spec conflicts or gaps (reported, not resolved)

1. "root-a / root-b" are not systemd names: sysupdate stores versions in
   labels and finds free slots by `_empty`; `BootBackend::current_slot()` must
   identify the slot by the booted root's partition UUID/number.
2. ESP 512 MB vs the BLS's ≥1 G guidance and UKI growth: owner decision.
3. With UKIs only BLS Type #2 entries are used (Type #1 unused).
4. "signed `SHA256SUMS`": v257 verifies only an OpenPGP `SHA256SUMS.gpg` via
   `gpg`; the target image needs `systemd-container`, a gpg binary and the
   project public key.
5. Section 2 rule 6 (only `alp-storage` runs `mkfs`/`cryptsetup`) vs block 2
   (layout via systemd-repart, which formats internally): needs an explicit
   exception (installer's repart formats only ESP and `state`; LUKS2/Btrfs on
   data only by `alp-storage`).
6. Block 3: `state` must be mounted in the initrd by an install-independent
   key (label/type); the DPS `var` type cannot be used.
7. The spec does not say when the B slot is created; recommend at install.
8. `ProtectVersion=%A` needs `IMAGE_VERSION`; block 1 must set an image version.
9. Section 3 gives no minimum disk size for the single-disk case.

## Recommendation, main risk, uncertainties

Adopt the layout in section 4: DPS types for ESP/root/verity, `linux-generic`
for state and data, slots created at install with B `_empty`, A slot and UKI
named by sysupdate's patterns (ideally written by sysupdate itself as the
last install step), no systemd-repart at boot on installed systems.

Main risk: slot and ESP sizes are permanent per installation.

Uncertain, needs a test or decision: sysupdate filling the A slot on an empty
target disk; root discovery with two root partitions on 257; producing
release files with sysupdate-compatible names from mkosi; ESP behaviour on the
reference laptop; where container images live (4 vs 8 GiB slots); minimum
single-disk size; how `state` is mounted in the initrd.
