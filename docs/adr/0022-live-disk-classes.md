<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# 0022: How ALP-OS Live tells the computer's own disks from external ones

- **Status:** accepted 2026-11-08. The Security and Network Expert approved the plan with R1-R8 (review notes of 2026-11-08, not in the repository).
- **Date:** 2026-11-08
- **Spec reference:**
  - owner decision 2026-10-19: Live never writes the computer's own disks;
  - Owner decision, 2026-11-08 (T58): the internal disk is shown greyed out, locked and offline, with a plain reason, and can never be chosen or formatted in Live;
  - docs/plans (storage-engineer) t58-disk-manager-t54-severity.md.

## Context

Live runs from a USB stick on a computer that may hold someone's whole life on its internal disk. The disk manager (T58) now lists every disk, including the internal one, so the rule that decides which disks Live may touch must be written in one place and enforced by alpd, not by the screen.

Before this ADR, the rule was written three times, slightly differently:
- `transport == Usb` in `archive::partitions`;
- `!removable && transport != Usb` in `inspect::is_internal`;
- the system-disk filter in `protect-disks`.

## Decision

`alp_live::disks::class_of` sorts each whole disk into one of three classes:

| Class | What it is | What Live may do |
|---|---|---|
| **Stick** | the disk the running system booted from (`is_system_disk`: the disk under the root filesystem, followed through dm-verity) | nothing (only Start over erases it) |
| **External** | a whole physical disk on the USB bus (its sysfs path goes through a USB device) that is not the stick | write the archive, format, check, open in Files and Import |
| **Internal** | everything else | list it, read SMART health; nothing else |

- **Partitions take the class of their disk.** A partition is never classed on its own.
- **Virtual devices are never disks** (loop, dm, md, zram, nbd, ram). Live itself runs on loop and dm devices. The hardware report already skips devices without a `device` link, and the classifier also refuses these names (review R1).
- **When unsure, Internal.** That covers an unknown transport, MMC/eMMC, an SD reader on the board, NVMe, SATA, virtio, Thunderbolt, eSATA, and a disk whose facts cannot be read. `removable` alone never makes a disk External.
- **If the stick is not found, every disk is Internal.** Otherwise the stick itself would look like an external USB disk.

### Where it is enforced

The class is computed again from the system at each call; an id or class sent by the screen is never trusted:
- `archive::partitions` sets `Part::external`, and every write path checks it (`allow_write`, `format::decision`, `media::open`);
- the routes that take a disk (`disks/*`, `archive/create`, `media/open`, import) answer `403 disk.locked` for the stick and internal disks;
- the format job checks the class and the `diskseq` once more, right before `wipefs`/`mkfs` (review R2).

The block layer is the second wall: `protect-disks` marks every non-stick disk read-only at boot and on plug-in. The only exceptions are the archive's one partition and a USB disk while a format job runs, pinned to that disk's `diskseq`.

## Consequences

- **A USB enclosure** holding an internal drive counts as External. That is what the user expects: they plugged it in.
- **An SD card in a laptop's built-in reader** counts as Internal and stays locked for now. If people need it, a later change can allow MMC devices that are `removable`, with Security's review.
- **Thunderbolt and eSATA disks are locked in Live.** Accepted: rare, and they fail safe.
- **The full install** will use its own rules for the computer's disks (the pool). This ADR covers Live only.
