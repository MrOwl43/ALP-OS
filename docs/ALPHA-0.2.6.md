<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# ALP-OS Live alpha 0.2.6 (draft, not released yet)

File: `ALP-OS-Live-alpha-0.2.6.img.xz` on the GitHub pre-release
`alpha-0.2.6`, built by CI from `live-alpha`. The SHA-256 is in the
`.sha256` file next to it. Test with unimportant files only.

Field report of the 0.2.5 test image (a laptop and a phone) and the first
items of the phone security backlog.

## What changed

- **Phones can connect when the computer's clock is wrong.** Many computers
  keep their clock in local time, so the stick's clock could run hours ahead
  until the network corrected it. A certificate made in that time was "not
  valid yet" for the phone, and pairing failed. Certificates now start one
  day back.
- **Date and time on screen.** The top bar shows the time. Settings >
  Display & Input has "Date and time" with a time zone. A clock the network
  has not confirmed yet is marked.
- **Check screen.** The network line comes last. Without a network it offers
  "Connect to Wi-Fi" right there.
- **Photos on the computer's screen.** The gallery now opens on the console
  itself. The browser holds no password or key: nginx asks alpd for every
  request, and alpd answers only while the console is signed in. Signing in,
  signing out and changing the password through the gallery are blocked.
- **Skipped setup steps are remembered.** A step put off in setup stays put
  off after a restart, and its reminder stays on the home screen.
- **Phone sign-ins not used for 60 days are removed**, once a day, and the
  removal is written to the journal.
- **Stronger server identity.** The identity a phone checks now also signs
  the number of the stick's certificate authority, so an old one cannot be
  brought back. Phones with the first app keep working.
- **Phone app.** A clock problem is said plainly, and the app checks the
  numbered identity.

## Tested

- CI on GitHub: Rust, shell, supply chain, the UI smoke test, and the VM
  tests of 0.2.5 plus T-PAIR-CLOCK and T-GAL-01 (both in T-IMMICH-01), the
  version 2 identity in T-PAIR-01 and the skipped steps in T-LIVE-02.
- The phone app's checks: JVM unit tests.

## Not tested on hardware

- Everything new here: the clock fix on a real computer with a local-time
  clock, the gallery on the real screen (speed, live updates), Wi-Fi from
  the check screen, and the time zone setting.

## Known open items

- The gallery design differs from the earlier "session cookie" condition of
  the security review and still needs its final review
  (`docs/security/nginx-live.md`).
- Live updates inside the gallery (new photos appearing by themselves) are
  not verified.
- There is no way yet to set the clock by hand without a network.
- Not in this release: the separate Live build profile, the kiosk without a
  usable GPU, and the version-update VM test.
