<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Live UI 0.2.5: owner feedback of 2026-11-08 (UI side)

Source: the UX addendum (`live-0.2.5-ux-addendum.md`, project files) on top of the 0.2.4 UX spec. Every new
server call is behind a `features` name, so the UI runs on today's alpd and falls back to its own reading.

| Item | What the UI does | Server part (Storage Engineer) |
| --- | --- | --- |
| T57 | Top bar: network icon, then the first address (the one phones use) in the mono font; "No network" in grey. | none |
| T56 | Automatic scale from `display.width_mm` (or the device pixels), 5-step slider (100-200 %), resolution row with a 15 s Keep / Revert dialog. CSS `zoom`; viewport units go through `--vh` / `--vw`. | feature `prefs`: `prefs.ui_scale`, `POST live/prefs` as a **partial** update (only the fields sent change). Feature `display_modes`: `GET live/display/modes`, `POST live/display/mode {id}` (reverts by itself after 15 s), `POST live/display/mode/keep`. `display {width_mm, height_mm, auto_scale}` in state (the UI uses `auto_scale` when `ui_scale` is null). Revert is left to alpd's own timer (`revert_in`); a `display/mode/revert` call would make the Revert button instant. |
| T59 | A full-screen frame (gallery, terminal) is fixed between the bars (`--chrome`, measured) and the page does not scroll. Checked at all 5 steps in T-UI-SMOKE. | none |
| T53 | Home only: blue gradient, red gradient from "action" on, 600 ms crossfade (none with reduced motion), frosted tiles, headline changes too. Background picker slot (one thumbnail until the photos ship). | `prefs.home_background` later |
| T54 | One legend: ok (check), note (i), action (i + "Action"), now (triangle + "Now"), critical (octagon + "Stop", yellow frame, 1.2 s pulse). Critical bar under the top bar; Import, delete, format refused with a sheet. Other tray marks are neutral grey. | feature `status_levels`: `state.status {level, key, message, code}`, `state.alerts[]`, `409 archive.critical`. |
| T55 | Welcome list uses the legend and an icon per line. Network from the live state: cable = "Best choice", Wi-Fi = works plus a grey cable hint, none = a note. Raw values under "Technical details". During setup the legend never goes past "note" for warnings. | none |
| Phone | One QR: the ALPArchive app's pairing code (`pair/open`), opened on the step and closed when leaving it. The free photo app's way (its address QR, the certificate QR, the typed address) sits under "Other ways to connect". | none (`pair.certificate_not_served` is shown like any error) |
| T58 | Two panes. Internal disk: 40 % opacity, lock, "Offline", not selectable, explanation and Technical details. Stick and archive: viewable, Format locked ("use Start over in Advanced"). Partition bar by role, rows for file system, free space, health, temperature. Format: name and exFAT; ext4 and secure erase under Technical details; password; slide with "What is removed" / "What stays"; progress; "{name} is ready." with Eject. Eject, Check for errors. | feature `disks`: `GET live/disks`, estimate `confirm`, `files`, `bytes_used`, (later) `photos_not_in_archive`; format `fs`, `confirm`, `409 format.confirm_stale`; features `disk_eject` / `disk_check`: `POST disks/eject`, `POST disks/check` (the UI shows the returned job's message, then `arch.job` with id `disk.check`). Each disk's `level` shows as a legend mark beside its health. New estimate warning `system`. |

Design review 1 (2026-11-08): own drop-down list instead of `<select>`; Lock after is a choice (1, 5, 15, 30 min; no Never, security review) behind the feature `idle_lock_pref` (`POST live/prefs {idle_lock_seconds}`; alpd accepts only 60, 300, 900, 1800, 422 otherwise, and checks the stored value again when read); format stays allowed at critical (security U1).

All slide screens now show the two lists side by side with an icon on each item. Disk names and labels are
text nodes only.

Tests: `just check-ui` (T-UI-SMOKE adds the disk manager, the old-alpd disk list, the four legend levels with
the critical pause, and the gallery fit at five sizes).
