# ALP-OS — Progress

Short state for the next session. Details: ADRs, `docs/streams/*.md`, PRs.

## 0.2.8 (decided 2026-11-24, ADR 0028)

- Focus moves to the installed version: boot from USB, install to a local disk or keep running from the
  stick; disks with an OS greyed out (formatting them comes later); dup on one disk, RAID1 on two or more;
  native GTK 4 screen (gtk4-rs, cage, WebKitGTK view locked to the gallery), Chromium removed; built and
  tested for tens of TB. Live frozen at 0.2.7 (security fixes only). Remote access stays in 0.2.9.
- Done: pairing race fix (approved request reads pending 3 s while nginx reloads); mobile app retries after
  pairing and shows the real reason (mobile branch claude/project-thread-6e05pr; APK not built yet).
- Open field bugs (0.2.7 laptop tests): uploads stall; shutdown did not power off; old laptops (2013/2016)
  do not boot; Wi-Fi on TP-Link Smart Connect; old archive refused the ALP password; kiosk escapes (moot
  once Chromium leaves). Home screen "All good" text; gallery "Upload more photos" blocked by kiosk policy.
- Done since: installer disk list, plan and executor (`alp-installer`, mocks only; T-INST-01 written, not
  yet in CI); pool plan digest (`alp-hw`, SHA3-256, golden test); alpd `install/disks|plan|start` +
  `state.install`; native screen (`crates/alp-screen`): sign-in, home, settings, install pages (disk list
  with greyed reasons, confirm with warning and slide, progress). App: files over 80 MB one at a time.
- Owner's 0.2.7 test notes (2026-11-24), to carry into the 0.2.8 screens: (1) Wi-Fi wrong password takes a
  full minute; answer as soon as the router answers. (2) Backup USB disk (installed system only; Live keeps other formats): no NTFS option; default and only
  choice is format to exFAT; an empty ext4 disk may become an archive without formatting. (3) Recovery
  code check said "No new recovery code is waiting" right after generating; on return the flow restarted
  (bug in recovery/create -> check state). (4) Phone steps: say a restart of the phone is needed after
  installing the certificate. (5) Phone scan: the message blinks every 1-1.5 s; keep one steady
  "Looking for your phone..." with a spinner. (6) Home dashboard too crammed in the middle; spread the
  panels, uneven grid.
- 0.2.7 ALP-REPORT (2026-11-24): uploads stopped because the laptop suspended 18:06-21:15 (lid); fixed:
  logind/sleep.conf.d never sleep (T-LIVE-01 checks). Shutdown: alpd steps ok (immich 34 s); the report
  was cut during the stop, so what hung later is still unknown. Postgres/valkey health checks time out
  (slow stick I/O). Fixed too: Wi-Fi wrong password fast fail; recovery check window 30 min + clear
  "expired" answer (cause not in the log).
- Done since: notes (1), (3), (4), (5) fixed; (6) native home is an uneven grid of tiles (screenshot
  checked); (2) rule written (`installed_backup_disk`) and the backup-disk plan changed to exFAT, the
  backup job itself comes with the backup role. Image profiles `live` / `installed` build in CI;
  T-INST-01 passes in CI (2026-11-25): Windows disk greyed and unchanged, the target gets the contract
  layout, slot 1 filled by repart CopyBlocks=auto, systemd-boot + the running UKI on the new ESP.
  T-INST-01 part 2 passes (2026-11-25): the installed disk starts alone (stick out), root from slot 1,
  seed on its alp-state (alp-live-state now settles udev itself; alp-live-protect is masked there).
- First start of the installed system (plan `docs/plans/0.2.8-first-start.md`, 2026-11-25): layout gains
  `alp-index` (contract); the seed carries the whole pool plan (contract, reader in `alp_hw::firstboot`);
  `alp_storage::seed_check` + `alp_storage::pool` (LUKS2 per member, Btrfs dup/raid1, resumable, degraded
  = read-only); `alp_live::pool` (key in the index, @archive at the archive place, @immich at Immich's
  data folder); API `POST live/archive/create-pool`, state `pool`. T-INST-02 passes in CI (2026-11-25,
  run 45641929679): two data disks become RAID1, file written, reopens after restart with the TPM,
  read-only and readable with one data disk missing.
- 0.2.8 item: old laptops (2013/2016) do not start the stick. Cause unknown, not reproduced; likely
  candidates: firmware with legacy BIOS only or 32-bit UEFI (we ship 64-bit UEFI only), or the
  graphics driver hanging at start. Owner: one is a MacBook A1278 (13", 2009-2012, Apple 64-bit EFI,
  boot menu by holding Option; NVIDIA 9400M/320M or Intel HD 3000/4000); an ASUS laptop starts by EFI
  and stays on a blinking cursor (30+ min). No kernel text seen, so the kernel may never get control:
  suspects are the 241 MB start file (UKI) on old firmware, or the hand-over to the graphics driver.
  Step 1 done: `earlycon=efifb` shows the first kernel messages. Step 2 (2026-11-25): the initrd
  had every kernel module and all their firmware (mkosi's default without `KernelInitrdModules=`);
  now only disks, USB, SD/eMMC, keyboards, TPM (T-BUDGET-01: start file within 128 MiB); a "safe
  graphics" (nomodeset) entry in the same start file (UKI profile); the boot menu shows 3 s; the
  installer copies loader.conf to the installed ESP (before: no `editor no` there). Next: the owner
  tries the next build on both laptops and sends a photo of the screen.
- Home screen names a missing pool disk (2026-11-25): the pool record keeps each disk's name from
  the installer (by-id + size), `state.archive.missing_disks`, alert `archive.disk_missing`
  (ALP-STO-001); T-INST-02 part 4 checks it.
- Recovery code on the pool (2026-11-25, server side): `alp_storage::pool` add/remove/test of LUKS
  recovery slots 1 and 2 (key and secret only on stdin), `alp_live::pool` two-step with a state file
  in the index (`pending N` / `commit N`, finished after a restart); `recovery/create|commit|check`
  work on the installed system; T-INST-02 part 3 checks it. Not yet: opening the pool with the words
  after the system disk is lost (needs a "recover" flow on a fresh install).
- Native setup screens (2026-11-25, `crates/alp-screen/src/setup.rs`, step from `model::setup::step`):
  password -> archive progress (starts `archive/create-pool`, retry on failure) -> recovery code (password,
  15 words, three typed back) -> phone (pairing pages) -> Done (`setup/finish`); checked against a mock
  alpd under Xvfb. The screen now starts on the sign-in page (it used to show the install page first).
- Next: native setup screens in a VM test (T-SCREEN-01) (password, pool progress, recovery, phone) on alp-screen;
  check gallery and phone approval on the native screen in a VM; code review of installer + pool
  (encryption/storage: opus reviewer) before the PR; `/claude-security` scan before release.

## Alpha 0.2.7 (started 2026-11-15, same branch, on top of 0.2.61)

- Owner's choice: one version with (1) the gallery back on a new stick, (2) a new version without erasing
  the stick, (3) remote access through our own relay; plus a format test set (RAW of many makers, video
  codecs) for the second beta.
- (1) written: `crates/alp-live/src/immich_restore.rs`, ADR 0025, T-IMMICH-02. Uses Immich's own restore
  flow and `immich-admin reset-admin-password`. Only CI can prove it (needs the photo service in a VM).
- (1) green in CI. Format test set part 1 done (`docs/qa/formats.md`); camera RAW needs a session on the
  "ALP-OS" cloud environment (raw.pixls.us allowed there).
- (2) ADR 0026, `docs/plans/live-update.md`: layout (slots + `alp-boot`), sysupdate transfer files,
  `crates/alp-live/src/update.rs` (`alpctl live update bless|check|apply`), `tools/update-files.sh`,
  T-UPD-01/02 in the CI group "update" (builds a second image as version 9.9.90; green 2026-11-16),
  Public history (2026-11-16): 14 commits of 2026-11-10 (in main, live-alpha and both release tags) carry
  co-author and session lines; whether to rewrite the history is the owner's open decision.
  `tests/ci/check-commit-messages.sh` (CI job shell) refuses such lines in every commit since 2026-11-11.
  Security review 2026-11-16 (clock, gallery restore, updates): fixed in code: signing key in the
  environment `release` (main only), unknown-key test, restore retries and keeps its backup aside,
  bless waits for the lock, no switch-off while writing, version names. Open: owner's decision on
  restore from an archive that is not encrypted (ADR 0025); links on a plain archive (immich.rs
  ensure_dir/bind follow them); clock set far ahead by the network (certificate dated in the future).
  alpd feature `update` (`state.update`, `update/check`, `update/apply`), Settings > About > Updates, release
  job step "Update files (signed)" (rolling release tag `updates`; skipped until the Actions secret
  `UPDATE_SIGNING_KEY` and `image/keys/update-signing.asc` exist: the owner needs steps for that). Still to
  do: restart button after an update, T-UPD-04 (power cut; T-UPD-03 rollback and T-UPD-05 bad signature
  pass in t-upd-02.sh, with the restart of an unhealthy counted start), bump
  `image/extra/usr/lib/alp/release` to 0.2.7 at release (the release job checks it).
- (3) trial step 1 done: `docs/plans/remote-access.md`, `spikes/iroh-tunnel` (iroh 1.4.1 + own iroh-relay on
  one machine: direct, relay-only, refusal of an unapproved phone). Product design there (own unprivileged
  `alp-tunnel`, never inside alpd). Step 2 needs the owner's phone and a relay server on the internet
  (owner decision), and the app repository.
- Next: the owner's update key and relay decision; T-UPD-04; security check of ADR 0024/0025/0026.

## Alpha 0.2.61 (2026-11-15, branch claude/project-thread-o9ostf off live-alpha)

- On the branch, no PR yet (needs the owner's OK): the phone's own certificate dated back; name lookup
  (systemd-resolved) so the clock is set from the network; the hardware clock is never written
  (`/etc/adjtime` LOCAL); `clock/sync`, `clock/base`, `prefs.lock_hide` (ADR 0024,
  `docs/plans/live-alpha-api.md`); the time zone window; lock screen options; the gallery stays during a
  video backup; Time & Language; gallery link cleanup; field-log fixes. Notes: `docs/ALPHA-0.2.61.md`.
- Open: security review of ADR 0024 and of the 0.2.6 gallery / T62 / T63 before the beta; hardware test by
  the owner; T67 (Wi-Fi rules file at early start).
- Next after 0.2.61: remote access through our own relay, then backup between homes.

## Live UI 0.2.5 owner feedback (2026-11-08, branch claude/project-thread-ptxu5r off stream-a/live-alpha)

- T53-T59 and the one-QR phone step, UI side: `docs/plans/live-ui-0.2.5-feedback.md`. New server calls are
  behind the features `prefs`, `display_modes`, `status_levels` and `disks` (Storage Engineer). T-UI-SMOKE passes.
- Next: security review of the patch, design review of the screenshots, then a PR with the owner's OK.

## Live UI 0.2.5 stage F, Files and Disks (2026-11-04, branch claude/project-thread-ptxu5r off stream-a/integrate-0.2.5)

- Files: a read-only browser of external disks (feature `media_files`). Disks: every disk is listed, and the
  refused ones show their reason; format uses the estimate's texts and shows the time left. Plan:
  `docs/plans/live-ui-0.2.5-stage-f.md`. T-UI-SMOKE passes.
- Terminal: only a plan so far (`docs/plans/live-terminal.md`); it waits for the Security Expert's OK.
- Server work for the Lead: `media/open|list|file|eject` and `media_files` (the shapes are in the plan).

## Live UI 0.2.4, console screens (2026-11-03, branch claude/project-thread-ptxu5r)

- UX spec stages A-E, UI side (plan: `docs/plans/live-ui-0.2.4.md`): system
  bar and full-screen layout, error manager behind (i), setup progress kept by
  alpd, recovery code intro / Replace (password, warning, slide, 3 words),
  strict-off and password-change warnings, settings in nine categories with
  the new wording, Immich only in the phone step and Credits, Photos full
  screen, Import picker, reset job polling, quick/secure format (T52).
  New server capabilities are gated by `features` in state, so the UI runs on
  today's alpd. `just check-ui` / CI job `ui` (T-UI-SMOKE over the mock) pass.
  Not pushed: needs the owner's OK and the Security Expert's review. Open: the
  Lead's server side (features list in the plan).

## Alpha 0.2.3 (2026-10-29)

- T31 About, T32 OS-like Back, T33 connected devices, T34 page on 443,
  T35 protection levels (ADR 0016: unprotected file pipe, levels 1, 2, 4
  done; level 3 password-bound reopening still open). Two reviews, fixes
  in. T-UI-02 passes (tpm, password, plain). `out/ALP-OS-Live-alpha-0.2.3.raw`,
  notes `docs/ALPHA-0.2.3.md`. Local commits only. Next: level 3 reopening,
  Strong-protection shortcut (owner's blocker), T36, build-tools lock.

## Alpha 0.2.2 (2026-10-28)

- T28 alpd answers before the hardware check, battery and Wi-Fi reads (all
  in the background); T29 boot log on ALP-REPORT every minute; T30 VM test
  T-UI-02 (passes, TPM and password). QA review: 4 must-fix found and fixed.
  `out/ALP-OS-Live-alpha-0.2.2.raw`, notes `docs/ALPHA-0.2.2.md`. Local
  commits only (not pushed). Open: build-tools lock vs host systemd 259.9;
  owner's laptop test and boot logs.

## Alpha 0.2 (2026-10-25)

- Storage v2 (ADR 0014, gocryptfs folder, disk checks, repair, prepare),
  recovery words bound to new archives, Immich in the image (ADR 0015;
  images unpacked by the immich user on first start because mkosi 26 builds
  without root). QA review fixed 9 bugs. `just check` passes; one image
  build: `out/ALP-OS-Live-alpha-0.2.raw`. Nothing run on hardware or VM:
  `docs/ALPHA-0.2.md` lists what is untested. Next: owner's hardware test
  (T01), T-IMMICH-01 on CI, T04 gallery.

## T02 passwords and codes (2026-10-24)

- 15-word recovery code (`alp_live::recovery`), 8-word chain code with
  SHA-256 checksum and rate-limited joins, serious encryption (TPM slot of
  the index removed, flag inside the index). ADR 0013. Unit tests only (no
  VM run). Open: storage v2 must implement `recovery::bind/open`; until
  then `recovery/create` answers `recovery.not_ready` and T-LIVE-02's
  recovery part SKIPs.

## State (2026-10-18)

- Branch `stream-a/live-hwtest` (local, not pushed; owner approves push/PR):
  ALP-OS Live hardware-test image. `alp_hw::inspect`, `alpctl live report`
  / `protect-disks`, report partition `ALP-REPORT`, ADR 0007, owner guide
  `docs/LIVE_HWTEST.md`. T-LIVE-01, T-BOOT-01/03, T-BUDGET-01 pass
  (system image 382 MiB, idle memory 239 MiB). Waiting for real-laptop
  reports. `just repro-check` not rerun yet.
- First real laptop (ASUS VivoBook K3402ZA) passed; no wired port, Wi-Fi
  firmware missing. Report now names network chips without a working
  driver. Owner: Live may write files to the computer's disks, never delete
  or format (decisions.md 2026-10-18).
- Live step 1, network: DHCP on wired ports, timesyncd, nftables default
  drop, screen shows cable state and address (`alpctl live screen`).
  `docs/streams/g-network.md`; firewall needs code-reviewer before merge.
  Next: encrypted archive as one container file in an `ALPArchive` folder
  on an existing partition (refuse hibernated/Fast Startup Windows and
  BitLocker), fallback on the stick; then Immich and phone backup.
- Wi-Fi (ADR 0008): iwd + non-free firmware, network chooser on the Live
  screen (Enter). Image file 1.13 GiB, system image 728 MiB. Joining a real
  network is tested by the owner on hardware.
- Owner Live decisions (ADR 0009): 4 GB image limit, writes but never
  deletes/formats, key file on the stick, seed only for the sync chain.
  Root EROFS zstd-compressed (system image 531 MiB).
- Encrypted archive (`alpctl live archive`, `archive.rs`): partition
  chooser on the screen (type a), `ALPArchive/archive.alp` = fixed-size
  file, LUKS2 + Btrfs dup, key on the stick's `alp-state` partition, opens
  at start-up; one partition writable, the rest of that disk read-only.
  T-LIVE-02 passes (create, reopen after restart, other partition intact).
  Needs code-reviewer approval before merge (encryption, storage). NTFS
  (ntfs3) and the Fast Startup check are verified only by unit test so far.
  Next: Immich (docs/plans/live-immich.md), then the graphical UI.

## State (2026-10-17)

- `main`: block 1 core (reproducible, verified EROFS image; VM tests
  T-BOOT-01/03, T-BUDGET-01; CI builds and boots it), shared contracts
  (ADR 0004), owner decisions (ADR 0005), two images (ADR 0006), spec v0.5
  (split copy in `docs/spec/`), stream rules and machine locks.
- No open owner decisions except a low-priority spec wording fix
  (`docs/DECISIONS-NEEDED.md` item 9).

## Plan (owner, 2026-10-17: usage is the bottleneck)

1. **Now: stream A only**, first deliverable = ALP-OS Live hardware-test
   USB image (notes: `docs/streams/a-image.md`). Tell the owner as soon as
   it is ready.
2. Then streams B and C, one or two at a time; then D, E, F, G.
3. Fresh session after each finished task.

## Open problems

- Dev VM is short of memory (~1 GB free with desktop apps open): a VM test
  right after an image build can time out; rerun once the load settles.

- Builds are identical on one machine, not yet between the dev VM and CI
  (pinned tools tree; ADR 0003).
