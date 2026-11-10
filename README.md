<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# ALP-OS

ALP-OS is the operating system of ALPArchive: an open source, immutable appliance
Linux that turns a PC into a private, always-encrypted home archive. The first
product is automatic phone backup over the home network, with
[Immich](https://immich.app) as the gallery.

**Status: alpha (ALP-OS Live 0.2.5).** Live runs from a USB stick: it checks the
computer, creates an encrypted archive on an external disk, and pairs phones for
backup. It never writes to the computer's own disks. Not for real photos yet.

## Principles

1. Stability and reliability above everything.
2. Simple for a non-technical user.
3. Security and privacy by construction: the project never holds a user's key.
4. Mature open source components, no home-made cryptography.

## Where to look

- `docs/spec.md`: the implementation specification (split copy in `docs/spec/`).
- `docs/adr/`: architecture decisions.
- `docs/ALPHA-0.2.5.md`: release notes of the current alpha.
- `docs/COLLABORATION.md`: how to work on the code.
- `justfile`: `just check` (lint and unit tests), `just build-image` (the Live image).

## License

AGPL-3.0-or-later, see `LICENSE`. Contact: alparchive@proton.com.
