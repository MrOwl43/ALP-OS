<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# ALP-OS Live alpha 0.2.61 (draft, not released yet)

File: `ALP-OS-Live-alpha-0.2.61.img.xz` on the GitHub pre-release
`alpha-0.2.61`, built by CI from `live-alpha`. Test with unimportant files
only.

First field test of 0.2.6 (a phone backed up photos through the regular
photo app after the certificate was installed) and the boot log of a
Wi-Fi-only laptop.

## What changed

- **Time & Language.** Settings has its own category for the date, the time
  zone and the language (English only for now). The first setup screen
  shows the time and lets you pick the time zone.
- **Videos on the computer's screen.** The gallery no longer copies a video
  into memory while the player is paused, which could stop the part that
  serves the gallery.
- **Tidier gallery.** Links to parts that have no use on the computer's
  screen are hidden: sharing, account settings, administration, purchase,
  utilities, workflows and the locked folder (it cannot work here yet).
- **Faster start without a network.** The start no longer waits two minutes
  for a network on a computer where no Wi-Fi was picked yet.
- **Hardware check.** The page shows the release name instead of "0.0.0".
- A helper of the photo service no longer fails once at every start in the
  screen's own session.

## Tested

- CI on GitHub as for 0.2.6; T-LIVE-01 also checks the release name on the
  hardware check.

## Not tested on hardware

- Video playback on the console after the change.
- The time zone picker on the first setup screen.

## Known open items

- The ALPArchive app could not connect in the 0.2.6 field test (cause not
  known yet).
- The gallery hides links by their address only; the account menu (sign out)
  is still shown and does nothing. The locked folder is hidden until the
  console can open it.
- The Wi-Fi rules file is not found at early start (T67).
