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

## Consequences

- 12.2 GiB of a 32 GB stick go to the slots; the index keeps the rest.
- Three systems can be on a stick: the written one and two updates.
  Boot counting falls back to the previous one when a new one does not
  reach `alp-healthy.target`.
- Test: T-UPD-01 (layout on a 32 GiB and an 8 GiB stick). T-UPD-02..05 come
  with the next steps (update, rollback, power cut, bad signature).
