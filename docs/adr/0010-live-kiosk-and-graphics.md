<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# 0010 — Live kiosk: cage + Chromium, KMS graphics, alpd console listener

- **Status:** accepted (lead; owner request 2026-10-18 for a graphical UI;
  security spec `docs/security/kiosk-lockdown.md`)
- **Date:** 2026-10-19
- **Spec reference:** block 1 (kiosk stack), block 8 (local console),
  ADR 0007 decision 1 (text screen) is superseded for Live

## Decisions

1. **Console listener in `alpd`** (`crates/alp-core`): std-only HTTP/1.1 on
   `127.0.0.1:8411`, one thread per connection, `Connection: close`.
   Serves `/usr/share/alp/ui` (staged from `ui/`, without `ui/live/mock.js`)
   and `/api/v1/live/...` (`docs/plans/live-alpha-api.md`). Host header
   must be `127.0.0.1:8411`/`localhost:8411`; POST needs the exact Origin
   and a JSON Content-Type (`docs/contracts/api-access.md`). The Live logic
   moved from `alpctl` into the library crate `alp-live`, shared by `alpctl`
   and `alpd`. New crates (exact pins, licences checked): `serde_json`
   1.0.154 (MIT/Apache-2.0), `qrcodegen` 1.8.0 (MIT), `bip39` 3.0.0 with
   `bitcoin_hashes`/`hex-conservative` (CC0-1.0, added to `deny.toml`).
2. **Compositor: `cage` 0.2.0-2** (Debian trixie, pinned snapshot
   20261016T023245Z; 76 KiB). One full-screen Wayland client, no desktop.
   Runs as the unprivileged `alp-kiosk` user (sysusers.d) through
   `alp-kiosk.service` with `PAMName=alp-kiosk` (pam_unix + pam_systemd:
   a logind session on seat0 gives cage the screen and input devices),
   `TTYPath=/dev/tty1`, `Restart=always`, `-s` so a technician can reach the
   tty2 admin login. Verified in the VM: cage and the browser run as
   `alp-kiosk`, `/dev/dri/card0` (bochs) is used.
3. **Browser: Chromium** (trixie-security 155.0.8059.79-1~deb13u1 in the
   snapshot, 322 MB installed) with managed policies in
   `/etc/chromium/policies/managed/alp-kiosk.json` (security's list:
   `URLBlocklist ["*"]`, `URLAllowlist` the console and gallery, no dev
   tools, downloads, printing, sign-in, file dialogs) and flags `--kiosk
   --ozone-platform=wayland`, profile on tmpfs. Firefox ESR 153.5 (301 MB)
   was tried first and rejected: its `WebsiteFilter` uses WebExtension
   match patterns, which cannot contain a port, so the exception for
   `http://127.0.0.1:8411/*` never matches and the kiosk showed "Blocked
   Page" (seen in the VM, 2026-10-19). Both browsers get Debian security
   updates; `cog`/WPE is out (open CVEs, no URL restriction, block-1 notes).
4. **Graphics: KMS drivers instead of `nomodeset`.** Debian's 6.12 kernel
   has no `simpledrm` (`CONFIG_SYSFB_SIMPLEFB` is not set, VERIFIED in
   `debian/config` of linux 6.12.111-1), so without a GPU driver there is
   no DRM device and no Wayland. The image ships the kernel's own drivers
   (i915, xe, amdgpu, radeon, nouveau, virtio-gpu, bochs) plus the
   non-free-firmware packages `firmware-intel-graphics` (29 MB),
   `firmware-amd-graphics` (92 MB), `firmware-nvidia-graphics` (65 MB), all
   20250410-2, and Mesa (`libgl1-mesa-dri`, `libegl-mesa0`) for software
   rendering (`WLR_RENDERER_ALLOW_SOFTWARE=1`) on chips without 3D.
   NVIDIA GSP firmware is not shipped (very new cards may fall back to text).
5. **Fallback: the text screen.** `alp-kiosk.service` waits up to 30 s for
   `/dev/dri/card*`; when the kiosk fails (4 starts in 5 minutes),
   `OnFailure=alp-live-screen.service` puts the existing text screen on
   tty1. tty2 is only the admin login (`docs/security/admin-account.md`),
   other VTs are empty (`NAutoVTs=0`). `alp-live-splash.service` shows
   "ALPArchive is starting" on tty1 during boot.
6. **Kernel command line (from PR #7):** `root=/dev/mapper/root
   systemd.gpt_auto=0 rd.systemd.gpt_auto=0 systemd.swap=0 noresume` and
   the TPM setup services masked, so nothing auto-activates the computer's
   disks, swap, hibernation image or TPM.

## Consequences

- Image size grows by the browser, Mesa and firmware (budget 4 GB, ADR 0009).
- The kiosk is not part of `alp-healthy.target`; a broken display never
  affects backups (spec block 8 console rule 1).
- QA: `T-LIVE-01` must no longer expect `alp-live-screen.service` active
  (it runs only when the kiosk fails); `T-UI-01` takes the screenshot.
