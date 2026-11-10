<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# 0021: The Terminal's ttyd comes from upstream's release, pinned by checksum

- **Status:** accepted 2026-11-04. The owner chose "ttyd's official release file"; the Security and Network Expert approved it (both hashes checked independently).
- **Date:** 2026-11-04
- **Spec reference:**
  - spec priority 4 (reuse mature components);
  - CLAUDE.md: pin every dependency, VERIFY versions against real packages;
  - docs/plans/live-terminal.md (revision 2, review T1-T6);
  - docs/security/terminal.md.

## Context

The Terminal plan (approved by Security) names "ttyd (MIT; the Debian package)". The first CI image build with it failed: `E: Unable to locate package ttyd` in our pinned snapshot (trixie, 20261016T023245Z). Debian trixie does not ship ttyd, so the plan's assumption was wrong (VERIFY item).

## Options

1. **Upstream's static release binary.** ttyd 1.7.7, `ttyd.x86_64` from the GitHub release. It is built with musl and needs no libraries. It is the same program and version whose source and flags the review checked.
2. **Build ttyd from source in CI.** That means ttyd plus libwebsockets, libuv and json-c, on Debian or musl. It gives the same program with a longer, more fragile build to keep up.
3. **Postpone the Terminal.**

## Decision (proposed)

Option 1:

- **The pin.** `image/terminal/ttyd.lock` records the SHA-256 and URL of `ttyd.x86_64`, and of the release's `LICENSE` (MIT).
  - The binary's pin `8a217c96…4f55` equals the entry in the release's own `SHA256SUMS`, checked on 2026-11-04.
  - The LICENSE pin `8b7e100f…625b` covers the tag's `LICENSE` file.
- **The download.** `image/terminal/fetch.sh` runs in `just build-image` (`_stage`). It downloads over HTTPS (`curl -f`, with retries) into a per-output cache named by checksum. It checks the SHA-256 every time, and a missing or wrong file fails the build. The binary is installed at `/usr/bin/ttyd` (0755) and the license at `/usr/share/doc/ttyd/LICENSE`.
- **Changing the version** is a reviewed change to the lock file, one at a time, like the Debian snapshot and Immich's image pins (ADR 0015).
- **How it runs is unchanged.** It runs only in `alp-terminal.service`:
  - as `alp-term`, with the full sandbox (T3);
  - behind the firewall rule (T1);
  - started only by alpd after the password check (T5).

  T-TERM-01 checks that it runs as alp-term, on 127.0.0.1:8413 only, and prints its version.

## Consequences

- **Security updates are ours to track.** The binary bundles libwebsockets, libuv and mbedTLS 2.28.5 (end of life; TLS unused); see docs/security/terminal.md. Check upstream at each ALP-OS release. Debian's security team does not cover this binary, so a ttyd or libwebsockets fix needs a lock bump. The exposure is small: ttyd listens on loopback only, the firewall also drops 8413 from the network, and it runs only during a password-checked session as a sandboxed user.
- **The trust is upstream's release process** (GitHub release assets), fixed to one exact file by the pin. A changed asset at the same URL fails the build instead of entering the image.
- **Licensing.** MIT allows redistribution with the notice, which ships in the image.
