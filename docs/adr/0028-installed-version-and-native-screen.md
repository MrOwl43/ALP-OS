# 0028 — 0.2.8: the installed version, and a native screen instead of a browser

- **Status:** accepted (owner decisions, 2026-11-24)
- **Date:** 2026-11-24
- **Spec reference:** blocks 1, 2, 4, 7 and "Local console"; ADRs 0005, 0009, 0010
- **Overrides:** where spec v0.5 and this ADR differ, this ADR wins (spec header, v0.6 note).

## Context

Laptop tests of Live 0.2.7 showed that the Chromium kiosk can be broken or
escaped from the keyboard: F1 broke the screen, Ctrl+T opened a tab that
reached the Chrome Web Store, and the console switch did not work. The owner
judged that an operating system cannot rest on a browser kiosk. The owner
also set the scale target for the final system: tens of terabytes per home.

## Decisions

1. **Focus moves from Live to the installed system.** 0.2.8 is the first
   installed version. Live stays frozen at 0.2.7 and gets security fixes
   only. A later Live version copies files in an orderly way to a connected
   USB disk, so they can be viewed on another computer or backed up further.
2. **Boot from USB, then install.** 0.2.8 still starts from a USB stick.
   Its goal is to install itself to a local disk. The user may instead keep
   running from the USB stick, which then manages the data disks.
3. **Disks that hold an operating system are never offered.** During disk
   detection, a disk with an existing OS (any recognisable boot or system
   partition: ESP with a boot loader, Windows, Linux root, macOS) is shown
   greyed out with the reason, and cannot be chosen. Formatting such a disk
   (advanced mode only, after a warning screen and a slide to confirm) is a
   later version; 0.2.8 does not offer it. **VERIFY** the detection rules
   (partition type GUIDs, file system signatures through `blkid`) in the
   block 7 plan.
4. **Data protection by number of disks (spec block 4, unchanged).** One
   disk: Btrfs `dup` for data and metadata (protects against damaged files,
   not a dead disk; the screen says so). Two or more: `RAID1`. `RAID1c3`
   and `RAID10` allowed by explicit choice. RAID5/6 forbidden.
5. **Native screen, no browser.** The local screen becomes a native
   application: GTK 4 with libadwaita, written in Rust (gtk4-rs), running
   alone under the `cage` Wayland compositor. It talks to `alpd` over the
   same API the web console uses today, so no logic is rewritten. Chromium
   leaves the image. The Immich gallery, itself a web app, is shown inside
   the native app through an embedded WebKitGTK view locked to the gallery
   origin (no tabs, no address bar, no navigation elsewhere, no file
   dialogs except a later removable-media-only picker). Phones are not
   affected: they talk to Immich and the pairing API, not to this screen.
   This replaces the "kiosk browser engine" of spec block 1 and the
   localhost web front end of block 7 and "Local console". **VERIFY** the
   pinned Debian versions of GTK 4, libadwaita, gtk4-rs and WebKitGTK
   (6.0 API), their security update cadence, and that they run on the old
   test laptops' graphics (software rendering fallback).
6. **Built for tens of terabytes.** Every screen, list, upload path, scrub,
   snapshot and index job is tested with a large archive (hundreds of
   thousands of photos), not only a small test set. Lists page and stream;
   nothing loads a whole archive into memory.
7. **Unchanged:** remote access is in 0.2.9 (owner requirement); the phone
   connection and upload fixes (large files one at a time, resume after a
   stall) are in 0.2.8.

## Consequences

- Tests for the screen move from browser automation to an accessibility
  tree driver; **VERIFY** the tool (for example dogtail or AT-SPI directly)
  in the plan. The kiosk lockdown tests (T-SEC-KIOSK-01) are rewritten for
  the native app and must include F1, Ctrl+T, Ctrl+N, Alt+Tab and the
  console switch.
- The image loses Chromium (large) and gains GTK 4 and WebKitGTK.
- The web console code (`ui/live`) stays for the frozen Live line only.
