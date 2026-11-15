<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# The update key (public half only)

`update-signing.asc` is the public half of the key that signs `SHA256SUMS`
of every release's update files (ADR 0026). `just build-image` puts it into
the image as `/usr/lib/systemd/import-pubring.gpg`; `systemd-sysupdate`
refuses update files whose signature it cannot check with it.

While this file is missing, the image has an empty keyring and accepts no
update at all.

The private half never enters this repository, a log or a chat. It lives in
the repository's Actions secret `UPDATE_SIGNING_KEY` (owner's decision,
2026-11-15) and moves offline before the public release.
