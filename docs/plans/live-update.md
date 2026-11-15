<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Plan: a new version without erasing the stick (alpha 0.2.7, spec block 2 for Live)

Status: plan, 2026-11-15. Nothing built yet. Two owner decisions are open
(marked **DECISION**).

## Why

Today a new version means writing the whole stick again. That empties the
index: the gallery's database (0.2.7 puts it back, ADR 0025), the device's
certificate authority (every phone must be connected again), the console's
settings and the ALP password. With an update in place none of that is
lost.

## What the spec says (block 2, not built for Live yet)

Two root slots with dm-verity, systemd-boot with boot counting,
`alp-healthy.target` blessed by `systemd-bless-boot`, updates by
`systemd-sysupdate` with a signed `SHA256SUMS`. No custom updater.

## The stick today

`ESP 512M | root (EROFS, as small as it gets) | root-verity | ALP-REPORT 64M
| alp-state 256M | alp-index (made at first start in the free space)`.
One root slot, sized to fit exactly: nothing can be written beside it.

## Steps

1. **Layout** (`image/repart.d`): `root-a`, `root-a-verity`, `root-b`,
   `root-b-verity` with fixed sizes (root 6 GiB, verity 128 MiB; today's
   root is about 3 GiB), slot b empty (`Label=_empty`). The stick needs
   about 14 GiB before the index; the smallest stick becomes 32 GB
   (**VERIFY** today's stated minimum). Partition labels carry the version
   (`alp-root-0.2.7`), as sysupdate matches on them.
2. **Boot**: UKIs named by version on the ESP with boot counting (`+3`),
   `systemd-bless-boot` after `alp-healthy.target`. Already in the image for
   the installed layout (T-BOOT tests); check it holds on the Live layout.
3. **Transfer files** (`image/sysupdate.d`): root, verity, UKI from one
   HTTPS folder; `Verify=yes` (signed `SHA256SUMS`, the project's public key
   in the image); `InstancesMax=2`.
4. **Release** (`ci.yml` release job): besides the stick image, publish
   `alp-os_<version>.root.raw.xz`, `.verity.raw.xz`, `.efi`, `SHA256SUMS`
   and `SHA256SUMS.gpg`. **DECISION 1: where the signing key lives.**
5. **alpd + UI**: `GET live/update` (current version, newer version if
   any, last result), `POST live/update/check`, `POST live/update/apply`
   (job with progress; then asks to restart). Settings, "About and
   updates": "Version 0.2.7. Version 0.2.8 is ready: Update". Never by
   itself in the alpha (**DECISION 2** later: automatic at night). Refused
   while a phone uploads or a disk job runs.
6. **Tests**: T-UPD-01 a good update is blessed after the restart and the
   index is as before (ALP password, gallery, paired phone); T-UPD-02 a
   broken image rolls back by itself; T-UPD-03 power cut while the other
   slot is written leaves the running slot bootable; T-UPD-04 a `SHA256SUMS`
   with a wrong signature is refused.

## What does not change

- The first 0.2.7 stick must still be written the old way once (the
  layout changes). From then on: Update.
- `/etc/adjtime`, the index and `alp-state` are never touched by an
  update. Database changes of a newer Immich run at its first start, as
  upstream does; rollback after such a change needs the database backup
  (ADR 0025) and is a later step.

## Risks

- A stick that is slow or nearly worn out: writing 3 GiB takes minutes;
  the job shows progress and can be stopped before the restart.
- The signing key is the most valuable secret of the project: whoever
  holds it can put software on every ALPArchive. Custody, rotation and
  revocation are an open item of the spec (`open-items.md`).
