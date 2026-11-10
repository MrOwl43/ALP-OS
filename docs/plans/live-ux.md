<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# ALP-OS Live: screen-by-screen flow (power-on to "the phone is backing up")

Status: plan, not built. Basis: manifesto, ADR 0006/0007/0008, spec block 5/7/8, owner log 2026-10-18.
Audience: a non-technical family member. Wording below is the single source for the text
console (80x24, English, keyboard) and the later kiosk web UI. Same steps, same words, same codes.

## Rules for every screen
- One decision per screen. Safe choice highlighted and chosen by Enter. Esc or Back always goes one step back.
- Ask nothing the system can find out. Never blame ("That did not work", never "You entered wrong").
- Footer on every screen: the keys that work. Header: "ALPArchive" and "Step N of 7".
- Errors: plain sentence, what to do, then the code, e.g. "Code: NET-101". Codes come from errors/catalog.toml
  (stream A range 100-199). Placeholders below are `<AREA>-1NN`; real numbers are assigned in the catalog.
- Nothing is ever deleted or formatted. Every screen that touches disks says so in its footer.
- Console: colours = green good, yellow note, red problem; box lines around the main text. Web: same text, same order.
- Every choice list also works with the number keys; no mouse needed.

## 0. Starting up (no input)
Text: "ALPArchive is starting. This can take up to a minute." Shown instead of a dark screen. (Today the screen is dark until the report; fix.)

## 1. Welcome and your computer  (Step 1 of 6)
Auto: runs the hardware check (alp_hw::inspect). Nothing to answer.
Text, all good:
  "Welcome to ALPArchive.
   This computer can keep your family's photos safe.
   [ OK ] Memory  [ OK ] Storage  [ OK ] Security chip  [ OK ] Network
   Nothing on this computer has been changed or will be deleted."
  Keys: Enter = Continue.
Text, with notes: yellow lines with one plain sentence each, e.g. "[NOTE] No security chip. Your archive still works and is still locked." Enter = Continue.
Text, cannot run (e.g. too little memory): "This computer is too small for ALPArchive. It needs at least 4 GB of memory. Nothing was changed. Code: HW-1NN". Keys: P = Power off, S = Save a note about this computer on the stick.
The hardware report still goes to ALP-REPORT on the stick; one line: "A note about this computer was saved on the stick."
Error: report cannot be saved -> yellow "The note could not be saved. You can keep going. Code: HW-1NN".

## 2. Network  (Step 2 of 6)
Auto: if a cable gives an address, skip this screen and show a one-line "Connected by cable" in the footer of step 3.
Only when no network: "ALPArchive needs your home network so your phone can find it."
  Choices (highlight the one that exists): "1  Connect with Wi-Fi" (only if a Wi-Fi card is found) / "2  Plug in a network cable, then press Enter".
Wi-Fi sub-screens (reuse wifi.rs wording):
  a) "Looking for Wi-Fi networks..." then a numbered list, strongest first, best signal highlighted. Keys: arrows/number, Enter = choose, Esc = back.
  b) "Password for <name>" (typed letters shown as dots; F2 = show them). Enter = Connect.
  c) "Connecting to <name>..." then "Connected. ALPArchive is on your home network."
Errors:
  - No Wi-Fi card works: "This computer has no working Wi-Fi. You can plug in a network cable instead. Code: NET-1NN".
  - No networks: "No Wi-Fi networks found. Move closer to your router, then press Enter to look again."
  - Password refused: "The network did not accept this password. Please try again." Enter = try again, Esc = other network. Code: NET-1NN.
  - Company login network: "This network needs a work or school login, which ALPArchive cannot use yet. Please pick another network." Code: NET-1NN.
  - Connected but no address after 30 s: "Connected, but the router did not give an address. Waiting again may help." Enter = wait, Esc = back. Code: NET-1NN.
Wi-Fi password is remembered on the stick (owner decision), so the next start joins by itself. "Forget this network" is offered under Network on the status screen.

## 3. Where to keep the archive  (Step 3 of 6)
Auto: lists partitions with free space, read-only until the user picks one. Hides the stick's own system parts, tiny partitions, and anything it cannot write to.
Text: "Where should ALPArchive keep your photos?
  ALPArchive adds one folder called ALPArchive with one locked file inside.
  Nothing is deleted and nothing is formatted."
  List (one line each): "Windows (C:)  ·  812 GB free  ·  room for about 190,000 photos" ; recommended one highlighted (most free space, not the stick); "Use the USB stick instead  ·  21 GB free  ·  room for about 5,000 photos".
  Keys: arrows/number, Enter = choose, Esc = back.
Then one confirm screen: "ALPArchive will create the folder ALPArchive on Windows (C:). You can delete the folder later in Windows to remove everything. How much room should it use?" Choices: "Up to 100 GB (recommended)" / "Up to half of the free space" / "Other size" (kiosk only: slider). Enter = Continue. (Size question: owner question 2.)
Errors:
  - Disk locked or full: "ALPArchive cannot write there (it is full or locked by Windows). Nothing was changed. Please pick another place. Code: STO-1NN".
  - Disk is hibernated/"fast start" Windows: "Windows did not close this disk fully. Start Windows once, choose Restart, then try again, or pick another place. Code: STO-1NN".
  - No suitable partition: only the stick is shown, with "No other place has enough room."
  - Not enough room anywhere: "There is not enough room to start (at least 64 GB). Code: STO-1NN".
  - BitLocker disks are out of scope: they are simply not listed.

## 4. Creating your locked archive  (Step 4 of 6)
Auto (update 2026-10-19): creates the encrypted file. Its key is kept on the stick. There is NO recovery-word step for the archive on Live and no word check. The words/QR (the seed) belong only to the sync chain (step 5b).
Text: "Creating your locked archive..." with a progress bar. One plain sentence: "The key to your archive is kept on this USB stick. Keep the stick safe, and keep it plugged in." Done: "Your archive is ready."
Error: "The archive could not be created. Nothing was changed. Code: STO-1NN".

## 5. Connect your phone  (Step 5 of 6)
### Beta (Immich app, temporary): one shared "Family" account for all phones; the screen shows address, account name and password once.
Auto: starts the gallery, creates the family account (name "Family", random password kept in the archive), builds the address and QR.
Text: "Now connect your phone.
  1. On your phone, install the app called Immich (App Store or Google Play).
  2. Join the same home network as this computer.
  3. Open the app and scan this picture. Or type this address: http://192.168.1.23:2283"
  Left: QR (contains address and one-time login). Right: address in large letters; then "Waiting for your phone..." with spinner.
  Keys: Enter when the phone shows its photos = Continue; H = Help (shows the three steps again); Esc = back.
  Auto-advance when the first phone logs in: "Your phone is connected."
  Errors: phone cannot reach the address -> help screen: "Is your phone on the same Wi-Fi as this computer? If it uses mobile data, turn that off." Code PAIR-1NN. Gallery not ready -> "The gallery is still starting. Please wait a moment..." after 120 s: "The gallery did not start. Press Enter to try again. Code: APP-1NN".
  QR is shown for 10 minutes; then "The picture expired. Press Enter for a new one." (PAIR-1NN).
### 5b. Sync chain (start or join; like Brave Sync)
Choice screen: "Is this your first ALPArchive computer?" 1 "Yes, start a new family chain" (default when no chain is found on the network, auto-detected: if one is found, "Join the chain" is highlighted instead) / 2 "No, join an existing chain".
  Start: "Open the ALPArchive app on your phone and scan this picture." Phone shows a six-digit code, user types it here only if the app asks (SPAKE2). Success: "Your phone is now the first member of your family chain."
  Join: "On a phone already in your chain, open ALPArchive, tap Add a device, and scan this picture." (QR from this screen) -> "Waiting for approval from your family chain..." Errors: expired code, wrong code, reused code: "That code is no longer valid. Press Enter for a new picture. Code: PAIR-1NN" (never says "wrong"). Phone login is not typed: the user never sees an account or password.
Seed (chain only): when a chain is started the screen shows the seed as QR plus numbered words. Say plainly: "These words are only for your sync chain. They let another computer or phone join it. They do not unlock your archive." Joining: scan the QR or type the words (one word per field, spell-check). Never called a recovery key.
Both: after pairing, "Turn on Automatic backup in the app, then press Enter."

## 6. First backup and "everything is working"  (Step 6 of 6)
Auto: waits for the first upload.
Text: "Your phone is backing up. 1,284 of 8,310 photos so far. You can leave the phone plugged in; it keeps going." Updates live. Enter = Continue to status.
Error/slow: no upload after 5 minutes -> "Nothing has arrived yet. Check that Automatic backup is on in the app. Code: APP-1NN".

## 7. Status screen  (Step 7 of 6, stays on screen; the home screen from now on)
Text:
  "ALPArchive is working.                         (big green dot)
   Photos kept: 1,284          Room left: 790 GB (about 180,000 photos)
   Phones: 1 connected         Last backup: just now
   Locked and private: yes     Kept in: Windows (C:) / ALPArchive
   Address for other phones: http://alparchive.local:2283"
  Yellow/red lines replace the dot text when needed, each with one action and code: archive file not found, low space (below 10 percent), network lost, phone not seen for 7 days.
  Keys: A = Add another phone (back to step 5), K = Show recovery help (explains where the paper is; never shows the words), N = Network, S = Switch off safely, Q = Hide screen (kiosk: screensaver, status stays on).
Do not turn off warning: a pull of the plug is safe; "Switch off safely" is only advice.

## 8. Next time you start it
- Boot stick again: screen 0, then auto steps: hardware check (silent unless a problem), network (cable, or Wi-Fi if remembered), find the ALPArchive folder on the disks it knows (by the archive file's ID, no question), unlock using the key kept on the stick.
- If all found: straight to the status screen within about a minute. No questions. "Welcome back. Your archive is ready."
- If the folder is gone or moved: "We cannot find your archive. It may be on another disk that is not plugged in. Nothing was deleted." Choices: Look again / Pick the place myself (step 3 list) / Start a new archive (warns that the old file stays and is not touched). Code STO-1NN.
- If the stick is a different stick: "This stick does not have the key for your archive. Nothing was deleted. Plug in the original stick." Code: KEY-1NN. (No archive words are ever typed.)
- Without Wi-Fi memory: shows the Wi-Fi screen; phones keep waiting and resume by themselves.

## Owner decisions (2026-10-19)
Closed: Wi-Fi password remembered on the stick; archive key on the stick, no recovery words on Live; one shared Family account in the beta; no word check; the seed is only for the sync chain; BitLocker computers out of scope. Still open: archive size (default: fixed 100 GB, "make it bigger" later).

## Update 2026-10-19 (owner storage decisions, ADR 0011)
Steps are now 7: 1 Your computer, 2 Password, 3 Network, 4 Archive, 5 Phone, 6 Sync chain, 7 All set.
- Step 2, first start: "Choose your ALPArchive password" (typed twice, strength hint). Plain warning: a forgotten password cannot be recovered in this alpha; keep photos on the phone too. The same password is the maintenance (admin) password.
- Every later start: with a TPM2 chip the index opens by itself (no screen, straight to status). The unlock screen appears only when there is no chip or automatic unlock fails: "This computer could not open your archive by itself. Type your ALPArchive password." After wrong tries a wait message with a countdown. The first-start password stays as that fallback and as the admin password.
- Step 4: only external disks or USB sticks. Empty list: "Plug in the disk or USB stick you want to use for the backup", polled every few seconds. Disks with an operating system are shown greyed out with the reason. Key warning (shown at size choice and during creation): "This alpha keeps the archive key on this USB stick. Writing a new ALP-OS version to the stick, or losing it, makes the archive unreadable. Keep your photos on your phone too."
- Status: low space (yellow) and full space / uploads paused (red) from archive.space, index.space and immich.state=paused. Missing archive: Look again, or Choose another place (archive/forget; old folder untouched).
- Phone: the QR holds only the server address; the family types email and password once. "Make a new password" calls phone/reset.

## Update 2026-10-19 (owner decisions, UX build in ui/live)
Setup is 7 steps: 1 Welcome and your computer, 2 Network, 3 Password (twice), 4 Backup disk (external only,
polled; refusal reasons shown; an old archive can be reopened with its recovery key), 5 Phone (first a QR to
install the certificate once, with one sentence each for Android and iPhone; then a QR with the server address
and the short family password shown large; Immich footnote), 6 Recovery key (key + QR, print or photograph,
shown once, Continue only after "I have saved it"), 7 All set ("Your photos are home and safe").
No chain step: chain start/join and the 24 words live in Settings.
Everyday: lock screen with non-sensitive status (device icons with masked names, network, free space, disk
health, photos kept or last backup) -> sign in with the admin password -> home (status, space warnings,
gallery link, Settings, Lock). Showing the family password or chain words, and making a new recovery key,
ask the password again; the password-again page (`/live/?view=confirm&reason=delete|trash&ids=...`) calls
`gallery/delete` and `gallery/trash/empty`. Idle lock follows `console.idle_lock_seconds`; the UI pings
`console/activity` at most every 30 s. Photo-frame screensaver toggle (browser setting, off by default).
Unlock-by-password screen when `lock.state = locked`, with a line about the security chip when `lock.tpm = off`.

### API gaps found by UX (not in live-alpha-api.md)
1. `phone.devices[] {kind, name}` for the lock screen's device icons (today only the count `phones`). The
   locked view should send names already masked; the UI masks again if it sees no bullet.
2. `archive.health` (`ok|warn|bad`, from disk SMART or Btrfs errors); the UI derives it from state and space until then.
3. `phone.last_upload` is always null, so the lock screen shows "Photos kept" instead of "Last backup".
4. Screensaver needs a way to read recent photo thumbnails (`gallery/recent`); the UI shows placeholder slides.
5. Photo-frame and any other kiosk setting are stored in the browser; a `settings` endpoint would survive a profile reset.
6. No "setup finished" flag: after a restart mid-setup the UI cannot tell whether the recovery key was shown;
   it skips the page once `archive.recovery = set`. Suggest `setup.state`.
7. The gallery's delete guard must send people to `/live/?view=confirm&reason=delete|trash&ids=<comma list>`
   (or call the endpoints itself); the UI page does the password ask and the call.
8. The family password can be shown again only via `phone/pair` (works, idempotent); the idle-lock time cannot be changed from Settings.
9. Locked view hides `phone.server_url`; the home gallery link needs the open view (it is read after sign-in).

## Update 2026-10-20 (owner feedback after the laptop test)
- **Top bar on every screen** (setup, unlock, lock, everyday): left brand and step dots; right "Skip setup" (setup only),
  backup-disk icon (everyday, amber/red when space is low/full), network icon (Wi-Fi bars, cable icon, or crossed
  Wi-Fi with "Not connected") that opens the network chooser, and a power button with a short confirm. On the lock and
  unlock screens the network icon only shows status ("Sign in to change the network"); power works without sign-in.
- **Every step can be skipped** ("Set up later"): network, password, backup disk, phone, recovery. The password step
  says the archive cannot be created without it; the disk step shows "Choose your password first" if it was skipped.
  Phone and recovery only appear once an archive exists. Open steps are derived from the real state
  (`reminders()`), so the home screen lists them with a "Set up" button that resumes the wizard at that step, also
  after a reload. The "All set" screen turns into "ready for the rest" with the same list.
- **Back keeps state** (in memory for the session): chosen Wi-Fi, typed Wi-Fi password and the scanned list, the two
  password fields, the chosen disk and size, the recovery key shown. Finished steps show a "done" panel (connected
  to X, password set, backup disk ready) instead of asking again.
- **Leave the wizard any time**: "Skip setup" in the top bar goes to home even after a failed step. Home, Settings,
  Disks, Network (top bar), Advanced are reachable. Idle lock is off while no password exists.
- **Backup disk step** lists every external disk. Unusable ones keep their `reason` (one sentence incl. what to do),
  the `code` (ALP-STO-100…107) and, for an old archive, a "Reopen" button. No disk plugged in: "No external disk or USB
  stick is plugged in." Per the owner there is NO "Prepare this disk" button (ALPArchive never erases an external disk).
- **Settings > Disks**: read-only list (name, model, size, type, use = ALPArchive system / Backup / Not used,
  "Safe to unplug" / "In use" badge). Settings > Advanced is a placeholder for the terminal.
- **Screensaver default**: five small SVG scenes (green hills, blue sky, clouds) in `ui/live/screensaver/`, slow cross
  fade; drop-in hook for licence-checked photos in `screensaver/README.md`.
- QA: `&shot=1` in the mock delays the load event so headless screenshots wait (serve `/__slow`); new presets
  `home-empty|where-blocked|disks|admin|network-up`. Screens: the CI artifact `ui-screens`.

### API gaps (2026-10-20)
1. `GET live/disks` does not exist: the Disks page falls back to archive place + candidates (no system stick, no
   internal disks, no real safe-to-remove). Wanted: `{id, name, model, size_bytes, kind: stick|external|internal,
   role: system|backup|other, safe_to_remove}`.
2. `network.ports[].signal` (0-4) for the connected Wi-Fi bars; the icon shows full bars until then.
3. No endpoint for a terminal/admin session; Advanced is a placeholder.
4. candidates cannot tell "nothing plugged in" from "plugged in but unreadable"; whole-disk candidates (lead's 2026-10-20 change) cover this.
5. Skipped steps live in the browser's memory only; a `setup` flag from the server (gap 6 above) would survive a restart.

## Update 2026-10-24 (owner feedback after alpha0.1)
- **Slide to confirm** (`slideConfirm` in `ui/live/app.js`, reusable): a knob dragged fully to the end. Mouse and touch
  (pointer events), keyboard (arrow keys move it by a quarter, End confirms, Home resets, Esc cancels the screen) and
  screen readers (`role=slider`, value text). Letting go early springs back. Works right-to-left.
- **Extreme actions** now go: password -> warning screen (what is deleted, what stays, "cannot be undone") -> slide.
  Used for: empty trash, delete photos (the gallery guard page `?view=confirm&reason=trash|delete`), leave the sync
  chain, forget a disk. Wording is in `i18n/en.json` under `danger.*`.
- **Settings > Advanced > "Delete everything and start over"**: password, then three screens (button: what is deleted;
  button: what stays + "cannot be undone, not even with the recovery key"; slide). Then the UI forgets its session state
  and restarts at Welcome. Calls `POST live/reset` (proposal below).
- **Step dots are buttons**: jump to any step. Tooltip and label say done / you are here / not done yet (from real
  state). A future step that needs an earlier one (phone, recovery without a backup disk) shows "The backup disk comes first".
- **Background**: light green to white to light blue gradient (dark theme: deep green to blue); text contrast stays AA.
- **Settings > About > "Read the specification"**: in-kiosk page rendering `docs/spec.md` with a tiny safe Markdown
  renderer (text nodes only, links shown as text). The file is staged as `ui/live/docs/spec.md` by `ui/stage-docs.sh`
  (run it at image build time; a copy is committed now). Missing file shows a plain notice.
- **No traps**: "Skip setup" and the dots work on every setup screen except the "computer too small" screen (only power off
  makes sense there). Archive creation shows **Cancel**, calling `POST live/archive/cancel` (lead is adding it).
- QA: mock presets `&danger=trash|delete|chain|forget|reset1|reset2|reset3` (`&slide=0.5` shows a half-dragged knob),
  `&view=spec|phone`. Screens: the CI artifact `ui-screens`.

### API gaps (2026-10-24)
1. **`POST live/reset`** (new). Body `{ "password": "…" }`. Checks the admin password, then deletes the `ALPArchive`
   folder on the backup disk (only that folder; other files untouched), wipes and recreates the index on the stick
   (gallery data, family account, admin password, recovery key, chain membership, TPM key slot) and sets
   `lock.state = "setup"`. Works also when `archive.state` is `missing` (index only). 200 `{}`; errors `403 password.wrong`,
   `429 password.wait`, `409 archive.busy`. The UI then reloads state and starts at Welcome.
2. `POST live/archive/cancel` (lead adding). Mock returns `409 archive.not_creating` if nothing is being made.
3. A password check that does not open the console: the UI uses `console/signin` to check the password before the warning
   screen, then sends the password again with the action. Wanted: `POST live/password/check` `{password}` -> 200/403.
4. `gallery/delete` and `trash/empty` receive the password after the warning, so the password is kept in browser memory for
   the length of the flow only.
5. `GET live/disks` is still missing (see 2026-10-20).

## Update 2026-10-26 (owner findings from the alpha 0.2 hardware test, T14-T25; UI developer)
- **Top bar (T18)** on every screen: home button (after setup, signed in), the IP address (owner: visible for now; "+1"
  when there are more, all in the tooltip), battery percent when `system.battery` is not null, backup disk and network
  icons coloured green (ok) / amber (warning) / red (problem), settings icon (after setup), power. A hidden place for the
  log icon of T27. On the lock screen the icons only show status.
- **Step row (T19)** centred under the top bar, an icon and a short name per step. New order: 1 Your computer, 2 Network,
  3 Password, 4 Backup disk, 5 Recovery, 6 Phone, 7 All set (**the phone is the last step**, T22).
- **Wi-Fi (T17, T20):** one tap on a network opens the password step (open networks: a "Connect to X" step); a double
  click cannot press the next step's buttons. Back, "Refresh Wi-Fi", "Set up later" and Continue sit in a toolbar
  above the list. Action buttons carry standard inline SVG icons (`ICON` and `KEY_ICON` in `app.js`; `data-ico`
  overrides).
- **Waiting screens (T21):** `waitLine()` shows what happens now and, when slow, why: `immich.detail`, job
  `message`, or a plain time-based text (`wait.*` keys).
- **Connect your phone (T22, T23):** 1) which app (the official Immich app, Google Play / App Store), 2) the certificate
  once (when the address is https), 3) scan the QR code in the app or type the address (every `addresses` entry, big),
  sign in with `email` and "your ALP password". `phone/pair` is called again every 5 s until `ready`; `needs_password`
  asks the ALP password once. No family password anywhere (Settings rows removed).
- **Codes (T24):** ALP password blue + mountain (label "ALP password"), recovery code red + owl, chain code gold +
  elephant; recovery screens have a red frame, chain screens a gold one.
- **Settings > This computer (T25)** from `hardware.specs` (older alpd: the start-up checks).
- **Gallery (T16):** "View my gallery" opens `immich.console_url` in a sandboxed iframe below the top bar with
  "Back to ALPArchive"; a CSP refusal, no load within 20 s, or the photo service not running shows a plain message
  with the way back. Sandbox `allow-scripts allow-same-origin allow-forms` (Immich's own origin; no top
  navigation, no pop-ups, no downloads). alpd refuses API calls a browser marks `Sec-Fetch-Site: same-site` or
  `cross-site`, so the framed gallery (same site, other port) cannot use the console's cookie.
- **Idle lock in the gallery (lead decision 2026-10-26):** taps inside the frame never reach the console, so while
  the gallery is shown the idle lock waits 30 minutes instead of `console.idle_lock_seconds` (the longer of the two),
  and the UI sends `console/activity` once a minute meanwhile so alpd's own session does not lock first. Leaving the
  gallery is a tap ("Back to ALPArchive" or Home), so the normal idle time applies again from there.
- **Shutdown (T15):** "Shutting down..." with the `power.off` steps and `job.message`; when alpd has been gone 20 s
  (or after 2 minutes): "You can now switch off the computer and remove the stick."
- QA (mock): `&battery=none|low|charging`, `&addrs=2`, `&immich=slow`, preset `home-starting`, `&fail=needpw|off`,
  `&pstage=2` (phone connect step), `&off=1` (shutdown screen), `&view=computer|gallery` with a signed-in preset.

### Not covered / to check on the hardware
1. Clicks inside the gallery frame do not count as activity: settled by the 30-minute idle lock in the gallery
   (above); check on the hardware that a long gallery visit locks after 30 minutes, not 5.
2. A Chromium error page inside the frame (e.g. Immich not answering) fires `load`, so it is not detected; the
   "Back to ALPArchive" button stays visible above it.
