# 0005 — Owner decisions of 2026-10-17

- **Status:** accepted
- **Date:** 2026-10-17
- **Source:** owner's answers to docs/DECISIONS-NEEDED.md (items 1–8)

| # | Question | Decision |
|---|---|---|
| 1 | System image size limit | **2 GB** for the system image (root + verity + UKI, i.e. what one update ships). Checked by T-BUDGET-01. |
| 2 | Spec wording "installer ISO" | Keep the deviation recorded in ADR 0003 (USB disk image); the owner will update the spec. |
| 3 | ESP size | **1 GB** (spec's 512 MB to be updated). |
| 4 | Smallest single disk | **64 GB** (`min_single_disk_bytes` in the hardware profile). |
| 5 | Where Immich's container images live | **Inside the system image**: one tested, signed update path; slots 8 GiB root + 128 MiB verity. |
| 6 | Spec wording items 1–11 | **All approved** as listed (slot labels and `_empty`; OpenPGP `SHA256SUMS.gpg`; repart formats only ESP and `state`; second slot at install; UKIs only (BLS Type 2); `state` mounted early by label; image version required; storage trait in `alp-hw`; key bytes only inside `alp-crypto`, which also owns the TLS key; confirmed plans for create/expand/replace; `@apps` subvolume). |
| 7 | Laptop browser management in the beta | **No**: beta management only on the server's own screen; the LAN carries only photo backup (Immich proxy). |
| 8 | Who helps beta testers | Beta testers contact the owner at **alparchive@proton.com** (to appear in docs/BETA_TESTING.md). |

## Consequences

- ADR 0004's *proposed* items are accepted; the disk-layout sizes are final
  for the beta (permanent per machine once installed).
- The spec (v0.5) should be updated by the owner for items 2, 3 and 6.
