<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Contract: installed-system disk layout

Source of truth: `image/install-layout/` (systemd-repart definitions).
Research: [docs/research/disk-layout.md](../research/disk-layout.md).
Decisions: ADR 0004 and ADR 0005 (owner: 1 GB ESP, Immich inside the system
image so 8 GiB slots, 64 GB single-disk minimum). Sizes are permanent per
machine once installed.

| # | Role | GPT type | Label at install | Size |
|---|---|---|---|---|
| 1 | ESP (systemd-boot + up to two UKIs) | `esp` | `esp` | 1 GiB |
| 2 | root, slot 1 | `root` (x86-64) | `_empty`, then `alp-os_<version>` | 8 GiB |
| 3 | verity, slot 1 | `root-verity` | `_empty`, then `alp-os_<version>_verity` | 128 MiB |
| 4 | root, slot 2 | `root` | `_empty` | 8 GiB |
| 5 | verity, slot 2 | `root-verity` | `_empty` | 128 MiB |
| 6 | state (`/var/lib/alp`, `/etc` overlay upper) | `linux-generic` | `alp-state` | 2 GiB |
| 7 | data, single-disk machines only (dedicated data disks: `image/install-layout/data-disk/`) | `linux-generic` | `alp-data` | rest of disk |

Rules

- Dedicated data disks: GPT with one `linux-generic` partition labelled
  `alp-data` (`image/install-layout/data-disk/`). The installer creates it
  on every data disk the user confirmed; after install, `alp-storage` uses
  the same definition for added or replacement disks. LUKS2 and Btrfs
  inside are created only by `alp-storage`.
- Pool members are `alp_hw::PoolMember`: an existing `alp-data` partition
  (created by the installer on any confirmed disk, or by `alp-storage` on a
  disk added later), or a whole new data disk during expansion/replacement. A member is
  identified by its GPT partition UUID (by-id names are for display only).
  `alp_hw::safety::check_member` refuses the system disk and any disk or
  partition holding the ESP, a root/verity slot or `alp-state`
  (`ALP-STO-006`); `alp-storage` calls it right before writing.
- Pool subvolumes: `@archive`, `@snapshots`, `@system-backup` (spec block 4)
  and `@apps` (application databases; approved in ADR 0005), mounted under `/var/lib/alp-pool` (`paths().pool`).
- Slots are not called "A"/"B": systemd-sysupdate writes the version into
  the GPT label of the slot it fills and uses `_empty` slots as free.
  The stable identity of a slot is its GPT partition number; the root
  partition UUID comes from the image's root hash and changes with every
  update (`alp_hw::Slot`).
- The installer creates all partitions (both slots `_empty`), then fills the
  first slot and installs the UKI as `alp-os_<version>.efi` using
  systemd-sysupdate with install-time transfer definitions (stream A + D;
  needs a test spike).
- The installed system runs no systemd-repart at boot (nothing changes the
  partition table after install).
- Versions are digits and dots only (e.g. `2026.10.16.1`); every image sets
  `IMAGE_VERSION` (block 1, needed by `ProtectVersion=%A`).
- `state` and data use `linux-generic` so systemd never auto-mounts or
  auto-unlocks them; `state` is found by its label (block 3 designs the
  early mount).
- Minimum disk: the system partitions need about 19.2 GiB (systemd-repart's
  own calculation). Installer minimums (hardware profile): 32 GB for a
  dedicated system disk, 64 GB for a single-disk machine (ADR 0005).
