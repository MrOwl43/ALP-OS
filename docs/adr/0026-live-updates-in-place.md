<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# 0026: Live updates in place: two update slots made at first start, signed by a key on GitHub

- **Status:** proposed 2026-11-15 (alpha 0.2.7). Step 1 (layout) built; the
  rest follows `docs/plans/live-update.md`. Needs the security review.
- **Date:** 2026-11-15
- **Spec reference:** block 2 (A/B updates, `systemd-sysupdate`, signed
  `SHA256SUMS`), `open-items.md` (signing key custody).

## Context

A Live stick is one written image: ESP, one root exactly as big as its
content, its verity partition, ALP-REPORT, alp-state. At the first start
alpd adds the encrypted index in the free space. A new version meant
writing the stick again, which empties the index.

## Decision

1. **The written image stays as it is.** Bigger images would make every
   stick slower to write. Instead, at the first start, the same
   `systemd-repart` run that adds the index first adds **two empty root
   slots** (6 GiB each, `Label=_empty`) with their verity slots (96 MiB)
   (`/usr/lib/alp/repart-update.d`). `systemd-sysupdate` writes new
   versions into them in turn. The root the stick was written with is never
   written again: it stays as the version to fall back to.
2. **Only on a stick of 28 GiB or more** (sold as "32 GB"). A smaller
   stick gets the layout of before (`repart-index.d`): all its room for the
   index, and new versions by writing the stick.
3. **Sticks set up before 0.2.7** have no slots (the index took the free
   space) and are written once more; the gallery comes back by ADR 0025.
4. **Signing (owner, 2026-11-15):** the release job signs `SHA256SUMS` with
   a key kept as a GitHub Actions secret; its public half is in the image.
   Good for the alpha and the beta: nothing to do per release. Whoever
   takes over the GitHub account could sign an update, so before the
   public release the key moves offline, with a rotation and revocation
   plan (spec `open-items.md`). The private key never enters the
   repository, a log, or a chat.
   Security review, 2026-11-16: a plain repository secret can be read by
   any workflow file on any branch, so by everyone who can push. The key
   is therefore a secret of the GitHub **environment `release`**, which
   the owner sets to "main branch only" with himself as required
   reviewer; the release job names that environment and runs only on
   main. A release with update files is made from main, after the merge.

4. **Start files on their own partition** (found while building, 2026-11-15):
   one UKI is about 250 MiB and the ESP is 512 MiB, so the UKIs of updates
   do not fit beside the written one. The update layout adds `alp-boot`
   (1 GiB, FAT, GPT type XBOOTLDR), which systemd-boot reads by its type.
   It is mounted at `/boot` only while an update is checked or written and
   while a start is marked good (`alpctl live update`; our own
   `systemd-bless-boot.service` in `/etc` takes the place of systemd's;
   the sysupdate timers are masked).
5. **Version order**: releases are numbered in the order they are made
   (0.2.6, 0.2.61, 0.2.7), which is not the order systemd compares
   (0.2.61 > 0.2.7). The image version (`IMAGE_VERSION`, update file names,
   partition labels) reads the third number as two digits: 0.2.7 is
   `0.2.70` (`tools/update-version.sh`); people see the short name.
   A two-digit third number that starts or ends with 0 is not a release
   name (0.2.10 would be the same version as 0.2.1); the tool and the
   release job refuse it.
6. **Update files** are cut out of the built stick image
   (`tools/update-files.sh`): the same root, hash partition and UKI a new
   stick has. The names carry the partition UUIDs (halves of the root
   hash), which sysupdate gives to the slot, so the UKI finds its root.
7. **No test door in the product**: the image has one update source and one
   keyring (the project's public key, `image/keys/`; empty until the key
   exists, so nothing is accepted). T-UPD-02 covers both inside the guest
   with bind mounts made as root; before it puts its own key in, it
   proves that correctly signed files from a key the image does not know
   are refused.

8. **An unhealthy new version restarts by itself** (contract
   `systemd-targets.md`, reboot on failure): `systemd-bless-boot.service`
   is in the start only when the started entry has a boot counter
   (systemd-bless-boot-generator), so its `JobTimeoutSec=10min` /
   `JobTimeoutAction=reboot-force` (the start hangs) and
   `FailureAction=reboot` (alpd does not answer: `alpctl live update
   healthy`) can never restart a version that was marked good or the
   written system. Limit: a version whose root cannot be mounted at all
   stops in the initrd; switching the computer off and on uses a try, and
   after three the version before starts (T-UPD-03).
   Limit (security review, 2026-11-16): "healthy" means that the system
   started and alpd answers. The index is still locked at that moment
   (it needs the person's password), so a new version whose photo
   service or phone door is broken is marked good all the same; the
   version before stays on the stick until the update after it.
   Marking good waits up to four minutes for another update action
   (someone pressed "Check"), so that a healthy start is not restarted
   for it. The computer is not switched off from the screen while a
   version is being written (`update.running`).
   After a version is installed the Updates row offers "Restart now"
   (`power/off` with `restart`); the screen comes back by itself.

## Consequences

- 13.2 GiB of a 32 GB stick go to the slots and `alp-boot`; the index keeps the rest.
- Three systems can be on a stick: the written one and two updates.
  Boot counting falls back to the previous one when a new one does not
  reach `alp-healthy.target`.
- New in the image: `systemd-container` (Debian's package for
  systemd-sysupdate and systemd-pull) and `dosfstools`.
- Tests: T-UPD-01 (layout on a 32 GiB and an 8 GiB stick), T-UPD-02 (files
  signed by an unknown key are refused; a full update: found, written,
  started, marked good, index opens). T-UPD-03 (a damaged new version: three
  failed tries, then the old version and its index). T-UPD-05 (a list of update
  files changed after signing is refused). T-UPD-04 (the power is cut while a version is written: the old version
  starts, the new one is offered again and installs).
