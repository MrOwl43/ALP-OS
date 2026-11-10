<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# ui/ — the one shared ALP-OS web UI

A single web UI codebase used by every screen (spec Section 2.2):

- the graphical installer (served on localhost by `alp-installer`),
- the local console on the server's own screen (kiosk session, served by `alpd` on localhost),
- a laptop browser on the home network (served by `alpd` over HTTPS),
- the phone app (how it reuses this code: ADR when the mobile phase starts).

Layouts differ for large screens and phones; the API, wording and error codes are
identical. All user-facing text lives in translation files and layouts must work
right-to-left.

## ui/live/ (Live setup assistant, kiosk)

Static HTML, CSS and plain JavaScript. No build step, no framework, no CDN, no web
fonts: the kiosk is offline. Open `ui/live/index.html` through any static server
(it fetches `i18n/en.json`); `alpd` serves it on 127.0.0.1:8411.

- `app.js` screens and routing; `api.js` the only place that knows the API shapes;
  `mock.js` fake API for `?mock=1` (extra: `&danger=trash|delete|chain|forget|reset1|reset2|reset3&slide=0.5` (warning screens), `&view=spec|phone|...`, `&screen=home-empty|where-blocked|disks|admin|network-up|welcome|welcome-fail|password|network|where|where-empty|progress|phone|recovery|done|unlock|unlock-wait|lock|lock-warn|home|home-warn|home-full|home-missing|settings|settings-chain|chain-seed|chain-join|confirm|saver`,
  `&fail=wifi|archive|phone|chain|unlock|wait`, `&theme=dark`, `&dir=rtl`, `&fast=1`).
- All user-facing text is in `i18n/en.json` (`{name}` placeholders). Add `xx.json` for another language.
- Layout uses logical CSS properties; set `dir` on `<html>` for right-to-left.
- Fonts (system, must be in the image): `fonts-noto-core` (Noto Sans) first, then `fonts-dejavu-core` (DejaVu Sans, DejaVu Sans Mono).
- QR codes are server-provided SVG, shown through `<img src="data:...">` so scripts in them never run. No QR library is vendored.
- Keys: Enter = primary button, Esc = Back, 1-9 pick the n-th choice, arrow keys move in lists. Touch and mouse work the same.


Screens (setup, in order): welcome + hardware check, network, admin password, backup disk
(plug in, choose, size, create; reopen an old archive with its recovery key), phone (certificate
first, then address + family password), recovery key, all set. Everyday: lock screen (non-sensitive
status), sign in, home, settings (phones, recovery key, sync chain, photo frame, network, switch
off), password-again page for deleting and emptying the trash (`?view=confirm&reason=delete|trash`),
idle lock after `console.idle_lock_seconds`, optional photo-frame screensaver (off by default,
stored in the browser as `alp.ui.prefs`). Demo: `&idle=5` locks after 5 seconds in the mock.

Top bar on every screen (skip setup, network, power), skippable steps with reminders on home, Settings > Disks, and screensaver scenes in `live/screensaver/`: see `docs/plans/live-ux.md` (update 2026-10-20).
