<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Kiosk lockdown requirements (for the lead)

The server's screen shows only the console UI (`http://127.0.0.1:8411`) and,
if added, the gallery (`http://localhost:8412`). Nothing else: no address
bar, no other sites, no files, no developer tools, no shell.

## Compositor

- `cage` 0.2.0 (VERIFIED in snapshot; `-s` = "Allow VT switching",
  off by default). Run with `-s` only so the admin can reach tty2; with
  `NAutoVTs=0` every other VT is empty (admin-account.md).
- Run as an unprivileged user `alp-kiosk`: no password, no shell login, not
  in `disk`, `sudo`, `adm`; no access to `/var/lib/alp`. Unit on tty1 with
  `PAMName=`, `TTYPath=/dev/tty1`, `Restart=always` (closing the browser
  restarts the kiosk). The old root text screen must not run on tty1 at the
  same time.
- The UI talks only to alpd; it never runs as root and never shells out.
- Kernel: `kernel.sysrq = 176` (sync, remount read-only, reboot only).
  `debug-shell.service` masked; root stays locked (emergency mode then asks
  for a root password that does not exist).

## Browser: Chromium with managed policies (recommended)

Chromium 150.0.7871.181-1~deb13u1 (snapshot). Firefox ESR 140 with
`policies.json` (`WebsiteFilter`, `DisableDeveloperTools`, downloads off)
is the alternative. `cog` (WPE) is small but cannot restrict which sites a
link opens, so not recommended.

Flags: `--kiosk --ozone-platform=wayland --no-first-run --noerrdialogs
--disable-session-crashed-bubble --overscroll-history-navigation=0
--user-data-dir=/run/alp-kiosk/profile http://127.0.0.1:8411/`
(profile on tmpfs: nothing persists).

`/etc/chromium/policies/managed/alp-kiosk.json` (VERIFY the names against
the Chrome Enterprise policy list for version 150 and that Debian reads this
path):

```json
{
  "URLBlocklist": ["*"],
  "URLAllowlist": ["http://127.0.0.1:8411", "http://localhost:8412"],
  "DeveloperToolsAvailability": 2,
  "DownloadRestrictions": 3,
  "AllowFileSelectionDialogs": false,
  "PrintingEnabled": false,
  "IncognitoModeAvailability": 1,
  "BrowserGuestModeEnabled": false,
  "BrowserAddPersonEnabled": false,
  "BrowserSignin": 0,
  "SyncDisabled": true,
  "PasswordManagerEnabled": false,
  "AutofillAddressEnabled": false,
  "AutofillCreditCardEnabled": false,
  "TranslateEnabled": false,
  "SearchSuggestEnabled": false,
  "DefaultSearchProviderEnabled": false,
  "MetricsReportingEnabled": false,
  "SafeBrowsingProtectionLevel": 0,
  "NetworkPredictionOptions": 2,
  "AlternateErrorPagesEnabled": false,
  "BackgroundModeEnabled": false,
  "TaskManagerEnabled": false,
  "EditBookmarksEnabled": false,
  "BookmarkBarEnabled": false,
  "ExtensionInstallBlocklist": ["*"],
  "DefaultNotificationsSetting": 2,
  "DefaultGeolocationSetting": 2,
  "AudioCaptureAllowed": false,
  "VideoCaptureAllowed": false,
  "ScreenCaptureAllowed": false,
  "HomepageLocation": "http://127.0.0.1:8411/",
  "RestoreOnStartup": 4,
  "RestoreOnStartupURLs": ["http://127.0.0.1:8411/"]
}
```

If joining a chain needs the webcam to scan a QR code, allow only the
console: `"VideoCaptureAllowedUrls": ["http://127.0.0.1:8411"]`.

## Tests (QA, T-SEC-KIOSK-01)

From the kiosk: `file:///etc/passwd`, `chrome://settings`, `view-source:`,
`https://example.org` and a link to `immich.app` are blocked; Ctrl+Shift+I,
F12, Ctrl+O, Ctrl+S, Ctrl+P and Ctrl+H do nothing; Ctrl+Alt+F3 shows an
empty VT; Ctrl+Alt+F2 shows the admin login; closing the browser brings the
kiosk back; `ps` shows the browser running as `alp-kiosk`.
