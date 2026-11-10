<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Live UI 0.2.4: console screens and navigation (plan)

Owner: Live UI Developer. Source: the approved UX spec "ALP-OS Live 0.2.4:
UX specification" (owner, 2026-11-03), section 9 stages A-E, UI side only;
the security review (`ux-0.2.4-security-review.md`) for the recovery code,
gallery and import rules; T52 and the reset job from the Storage Engineer.

## Files

- `ui/live/index.html`, `style.css`: new frame. A 36px system bar (screen
  name left; network, disk, battery, (i), settings, power right), everyday
  screens edge to edge, step screens as a 640px column on an even
  background. No browser feel: no text cursor or selection outside fields,
  no context menu, no drag, no Ctrl+wheel or pinch zoom, no overscroll,
  `:focus-visible` rings only, thin scrollbars.
- `ui/live/app.js`: every screen below. `ui/live/api.js`: new calls and the
  `features` gate. `ui/live/mock.js`: the new calls for `?mock=1`.
- `ui/live/i18n/en.json`: new wording (spec section 5), Immich removed except
  the store name on the phone step and About > Credits.
- Removed: "Read the specification" (screen, Markdown renderer,
  `ui/stage-docs.sh`, the justfile line and `ui/live/docs/spec.md`).
- `tests/ui/`: `check-i18n.py` (every key used exists; no "Immich" outside
  the allowed keys) and `smoke.mjs` (Playwright against the mock: every
  screen draws, no page errors, no browser dialog, disk labels shown as
  text only). Pre-installed Chromium; no new dependency in the repo.

## Server features the UI waits for (Stream A Lead)

Every new server capability is announced in state as
`features: ["setup_progress", "recovery_commit", ...]`. Without the name
the UI keeps today's behaviour, so the UI can merge before the server.

| Feature | UI side here | Server contract proposed (Lead decides) |
|---|---|---|
| `setup_progress` (T09) | Setup resumes where it stopped after a reboot; once finished it never returns; skipped steps become home reminders | state `setup: {complete, skipped: ["network", ...]}`; `POST setup/progress {complete?, skipped?}` |
| `recovery_commit` (T43) | Recovery intro screen, never makes a code on load; Replace = password, warning, slide; type 3 words to commit; words wiped on leave | `POST recovery/create {password, replace?, passphrase?}` → words + `confirm: [i, j, k]` (1-based); `POST recovery/commit {words: {"i": "w"}}`; `POST recovery/discard`; `409 recovery.exists` |
| `password_change` | Settings > Password & Recovery: current password, warning (phones sign in again), slide | `POST password/change {password, new_password}` |
| `events` | (i) panel shows the server's events as well as the console's own | state `events: [{id, time, level, key, message, code, step, detail, resolved, action}]`; `POST report/save` |
| `gallery_session` (T46) | Full-screen Photos under the system bar on its own origin; no second sign-in line | `immich.gallery_url` (`http://127.0.0.1:8412/`) set only once the session cookie is ready |
| `media_import` (T47) | Import button and our own picker; progress in the system bar | `GET media`; `POST media/browse {media, folder?, offset?}` (opaque ids); `POST import {media, items}` → 202 job `import`; `POST media/eject {media}` |
| `natural_scrolling` (T50) | Switch in Display & Input | state `prefs.natural_scrolling`; `POST prefs {natural_scrolling}` |

Already on `storage/wipe-format-media`: `POST reset` → 202 job `reset`
(polled until `lock.state == setup`), `disks/format/estimate`,
`disks/format` (job `disk.format`, cancel while `erasing`). The UI accepts
the old 200 answer of `reset` too.

## Rules kept

- Disk names, labels and file names from external disks go in through
  `textContent` only (the `h()` helper makes text nodes; no `innerHTML` with
  server data).
- Every action that cannot be undone, or weakens protection: password,
  warning screen (what is lost, what stays), slide.
- Gallery auto sign-in and Import stay hidden until the server announces
  them, which happens only after the security OK.
- Security review 2026-11-03: Replace appears only with `recovery_commit`
  (U1); the gallery is framed only for `http://127.0.0.1:<port>` on another
  origin than the console's (U2), without `allow-downloads` (U3). The
  Advanced password gate is a convenience only (U4): the server must still
  ask the password for write mode, format and import, and refuse the
  internal disk, the stick and the archive itself.

## Tests

`just check-ui` (new): `node --check` on every UI file, `check-i18n.py`,
and the Playwright smoke run over the mock. The VM tests T-UI-01/02 keep
checking the real kiosk on CI.
