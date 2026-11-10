# 0002 — Project license: AGPL-3.0-or-later

- **Status:** accepted
- **Date:** 2026-10-16
- **Spec reference:** Section 4 (`LICENSE`), Section 2.1 (license review), spec v0.4

## Context

The project needed a license before code is shared. The owner decided on
AGPL-3.0-or-later (NEXT_TASK.md, session 3), and spec v0.4 records it.

## Decision

All project code is licensed AGPL-3.0-or-later.

- `LICENSE` holds the unmodified license text from
  https://www.gnu.org/licenses/agpl-3.0.txt
  (sha256 `0d96a4ff68ad6d4b6f1f30f713b18d5184912ba8dd389f86aa7710db079abcb0`).
- Every source and configuration file starts with
  `SPDX-License-Identifier: AGPL-3.0-or-later` (as a comment in the file's syntax).
- `Cargo.toml` sets `license = "AGPL-3.0-or-later"` for the workspace;
  every crate inherits it.

## Consequences

- AGPL section 13: anyone who interacts with ALP-OS over a network (the web UI
  served by `alpd`) must be offered the corresponding source. The UI should
  show a "source code" link; to be done with the UI shell.
- Matches Immich's AGPL-3.0 license, so patches to Immich and ALP-OS code are
  under the same terms.
- Dependencies must be AGPL-compatible. The `cargo deny` allow-list (MIT,
  Apache-2.0, Unicode-3.0) is compatible; any new license on the list needs a
  review entry in `docs/components.md`.
- Third-party components keep their own licenses (listed in
  `docs/components.md`); the SBOM (block 1) records them per image.
- Not legal advice: a legal review of the final component list is still an
  open item (spec Section 10).
