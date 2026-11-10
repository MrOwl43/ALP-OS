# ALP-OS — Progress

Short state for the next session. Details: ADRs, `docs/streams/*.md`, PRs.

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
