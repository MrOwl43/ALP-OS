<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Plan — Block 1: image builder

Spec v0.4, block 1. Goal: reproducible, signed images and an installer image
from Debian Stable. Work in small steps; each step ends with a passing test.

## Steps

1. **Pin inputs.** Debian packages from snapshot.debian.org (mkosi
   `Snapshot=`), build tools from a pinned tools tree (`ToolsTree=yes`, same
   snapshot), fixed `SOURCE_DATE_EPOCH`, fixed `Seed=` for partition UUIDs.
   Test: `just repro-check` builds twice from clean and compares
   `SHA256SUMS` (spec acceptance: identical artifact hashes).
2. **Read-only verified root.** Root as EROFS with a dm-verity hash
   partition (systemd-repart definitions in `image/repart.d/`, `Verity=hash`
   for now; signed verity comes with signing keys). Roothash in the UKI.
   Test: T-BOOT-01 still passes; new T-BOOT-03 boots a copy with one changed
   root block and expects it not to reach alp-healthy.target.
   (A/B slots are block 2.)
3. **Budgets.** Record image size and idle memory (from the VM) in the
   manifest; `just test-vm` fails over budget (spec Section 3: base system
   under 2 GB idle).
4. **SBOM.** Generate an SPDX or CycloneDX SBOM from the mkosi package
   manifest (VERIFY tool choice in an ADR).
5. **Kiosk stack (spec v0.4).** ADR choosing the Wayland kiosk compositor
   and browser engine (Debian availability, security cadence, memory, touch),
   fonts with wide script coverage (including right-to-left), display firmware. Add to image; measure
   memory against budget.
6. **Installer image.** Bootable USB/ISO that starts the kiosk session
   (installer screens themselves are block 7). mkosi 26 has no ISO output:
   see open question below.
7. **CI.** Build the image and run T-BOOT-01 in GitHub Actions (KVM is
   available, ADR 0001); aarch64 smoke build (no full image).
8. **Signing.** UKI and verity signing with keys injected in CI; never in
   the repo. Needs a key-custody decision (spec Section 10) before real keys.

## Files

`image/mkosi.conf`, `image/repart.d/*.conf`, `image/mkosi.tools.conf` (if
needed), `justfile`, `tests/vm/*.sh`, `docs/adr/0003-*`, `docs/components.md`.

## Dependencies

No Rust crates. Build tools: mkosi 26 (host), tools tree from Debian
trixie snapshot. Possibly `xorriso` (ISO), an SBOM generator (ADR).

## Status (2026-10-16)

- Done: step 2 (EROFS + verity, T-BOOT-01 checks the verified root, T-BOOT-03
  data and hash tampering), CI image build + VM scenarios (part of step 7).
- Partly done: step 1 (snapshot, epoch and seed pinned; build tools pinned by
  exact Ubuntu versions; repro-check passes on one machine and runs nightly;
  pinned tools tree still open); step 3 (sizes and idle memory recorded in
  `out/budget.json` and checked against the 2 GB memory budget; size budget
  decided: 2 GB, ADR 0005).
- Decisions and limits recorded in ADR 0003.
- Owner decision: a USB disk image (`.raw`) is fine for the beta; no ISO.
- Known issue: builds are identical on the same machine (spec acceptance),
  but CI (ubuntu-26.04 runner) and the dev VM produce different `.efi`,
  initrd and `.raw` hashes, likely from build-tool version differences.
  Fix with a pinned tools tree (stream A).
- Kiosk research so far (step 5, now stream D/A):
  - WPE WebKit (`cog`) is ruled out: Debian trixie's wpewebkit 2.48.3 has many
    open CVEs marked "no DSA, ignored" (security-tracker.debian.org).
  - Candidates with Debian security support: `cage` 0.2.0 with Firefox ESR or
    Chromium (both updated through trixie-security).
  - Adding either browser makes the image 1.4–1.5 GB (size budget 2 GB:
    ADR 0005).
  - Memory measurement not finished: the experiment unit needs `chvt` and
    a working probe; to redo in the kiosk stream.
