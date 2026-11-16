<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# ALP-OS Live alpha 0.2.61 (draft, not released yet)

File: `ALP-OS-Live-alpha-0.2.61.img.xz` on the GitHub pre-release
`alpha-0.2.61`, built by CI from `live-alpha`. Test with unimportant files
only.

First field test of 0.2.6 (a phone backed up photos through the regular
photo app after the certificate was installed) and the boot log of a
Wi-Fi-only laptop.

## What changed

- **ALPArchive app.** The app refused to connect with "certificate not
  valid until ...": the phone's own certificate was not dated back like the
  others. It is now.
- **The clock is set from the internet.** It never was: the computer could
  not look up the time servers. Settings > Time & Language has a "Sync now"
  button.
- **The computer's own clock is left alone.** ALP-OS Live never writes it,
  so Windows on the same computer keeps its time.
- **Time zone window.** Search for a city or an offset, pick one, see the
  time it gives, then confirm. Without internet it asks which of two times
  is right and puts the clock right from the answer (a computer that also
  runs Windows keeps its clock in local time).
- **Lock screen.** It says that names are shortened and which details are
  hidden. Settings > Display & Input chooses what it shows: phones, the
  network name, free room, backup activity. The lock screen and the photo
  frame show the time in the chosen zone.
- **Gallery during a video backup.** The gallery no longer leaves the
  computer's screen with "still starting" while a phone backs up videos.

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

- The clock set from the internet on a real network, and "Sync now".
- That Windows keeps its time after ALP-OS Live ran with internet.
- The clock put right by hand without internet.
- The gallery during a long video backup.

- Video playback on the console after the change.
- The time zone picker on the first setup screen.

## Known open items

- The gallery hides links by their address only; the account menu (sign out)
  is still shown and does nothing. The locked folder is hidden until the
  console can open it.
- The Wi-Fi rules file is not found at early start (T67).
