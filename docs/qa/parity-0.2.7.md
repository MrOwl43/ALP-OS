# Native screen vs the 0.2.7 web screen: parity list

Rule (owner, 2026-11-26): before every test build, every page, setup step
and setting that 0.2.7 had must exist on the native screen (`alp-screen`)
or be marked here as consciously dropped or moved, with the reason. The
first-start VM test (T-SCREEN-01 part "first start") must pass too.

Status: **ok** ported · **gap** missing (must be ported) · **later** moved
to a later version with the owner's agreement · **dropped** on purpose.

## First setup (0.2.7 order, with the 0.2.8 install step)

| 0.2.7 step | What it does | Native | Status |
|---|---|---|---|
| Welcome | hardware check list, date, time and time zone (zone picker, "which time is right" question), network line with "Connect to Wi-Fi" | `start.rs` welcome page | ok (2026-11-26) |
| Too small | under 4 GB memory: power off | — | ok |
| Network | cable status, Wi-Fi list, password, WPA3→WPA2 note, Check again, skip with a note | `start.rs` network page | ok (2026-11-26) |
| (0.2.8) Place | install on a disk, or keep running from the stick | install pages | ok |
| Protect / Password | protection level and password | setup password page (password only) | later: owner decision (0.2.8 archives are always encrypted) |
| Where / Progress | archive disks and making the archive | install page data disks + setup pool page | ok |
| Recovery | 15 words, type three back, skip with a reminder | setup recovery pages | ok |
| Phone | connect a phone (pairing), skip | setup phone page + pairing pages | ok |
| Done | "All set" | home | ok |
| Reminders | skipped steps shown on home with "Do it now" | — | ok (Set up takes you to the right Settings page) |

## Everyday pages

| 0.2.7 page | Native | Status |
|---|---|---|
| Lock / unlock (sign in) | lock page | ok |
| Home (tiles, gallery button) | home page | ok |
| Gallery (locked embedded view) | gallery page | ok |
| Status tray (network, disk, battery, clock) | top bar | ok, check items |
| Errors list behind (i) with codes | — | ok |
| Phone approve pop-up | pairing pages | ok |
| Shut down / restart with steps | top bar power + alpd | ok (no step list: gap, small) |
| Import from a USB disk | — | later: alpd has no import routes (media/browse, import) yet |
| Files (USB disks) | — | ok: read only; photos and text open; videos are not played yet |
| Disks and format (danger + slide) | — | ok |
| Terminal (advanced) | — | ok (locked viewer on the local terminal port) |
| Reset everything (danger + slide) | — | ok (password, two warnings, slide, progress) |
| Chain (8-word code, join, leave) | `chain.rs` | ok (2026-11-26) |
| Screen saver | `saver.rs` (starts after the lock time, locks, any touch ends it; the choice is kept in a small file) | ok (2026-11-26) |

## Settings (0.2.7 had ten categories)

| Category | 0.2.7 rows | Native | Status |
|---|---|---|---|
| Phones | list, add phone, disconnect, only paired phones | `phones.rs`, `dialogs.rs` | ok (2026-11-26) |
| Password | change password, recovery code check / make / replace, serious encryption | `account.rs` (change password only when alpd announces `password_change`, which it does not yet; "serious" extra passphrase not ported, see notes) | ok (2026-11-26) |
| Disk | archive place, space bar, encrypt later | `disk.rs` ("Set up" opens the install pages while running from a stick, else says what to plug in) | ok (2026-11-26) |
| Chain | show / leave / set up | `chain.rs` (alpd still has all chain routes) | ok (2026-11-26) |
| Network | status, change Wi-Fi | Network page (from settings) | ok (2026-11-26) |
| Display | size, resolution, natural scrolling, background, lock after, lock screen shows, screen saver | `display.rs` (size applies as text size; natural scrolling and idle choice show only when alpd announces them) | ok (2026-11-26) |
| Time | date and time, sync now, time zone | sync + zone | ok (2026-11-26) |
| Power | battery, shut down, restart | power | ok |
| About | version, update | about | ok |
| Advanced | disks, files, terminal, reset, install | install | ok (10-minute password gate; install row kept) |
