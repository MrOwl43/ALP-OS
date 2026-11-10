# 0007 — ALP-OS Live hardware-test image

- **Status:** accepted (owner decisions 2026-10-17/18; technical choices by stream A)
- **Date:** 2026-10-18
- **Spec reference:** block 1 (outputs), ADR 0006, stream rules 10–12

## Decisions

1. **Text screen, no graphics stack** (owner: "simple text screen, designed
   smart and nice, without using resources it does not need"). The report is
   drawn on the Linux text console (tty1) with ANSI colours and box lines.
   The kiosk stack (compositor + browser) comes with the full Live beta.
2. **`nomodeset fbcon=font:TER16x32`.** The firmware framebuffer (efifb,
   `CONFIG_FB_EFI=y`) shows the screen; no graphics driver loads, so no
   graphics firmware is shipped and a missing-firmware GPU cannot blank the
   screen. TER16x32 is built into Debian's 6.12 kernel
   (`CONFIG_FONT_TER16x32=y`, checked in `/boot/config-6.12.111+deb13-amd64`
   from the pinned snapshot). The serial console stays the primary console
   (`console=ttyS0`) so the VM tests are unchanged; on a laptop the screen is
   dark until the report appears.
3. **Report on the stick, readable on Windows.** A 64 MB FAT partition,
   GPT label `ALP-REPORT`, type "Microsoft basic data"
   (`EBD0A0A2-B9E5-4433-87C0-68B6B72699C7`, checked with
   `sfdisk --list-types`). `alpctl` mounts it only to write the report,
   found by partition UUID **on the system disk only**, then unmounts.
   File: `hw-<maker-model>-<YYYYMMDD-HHMM>.txt` (UTC), CRLF line ends.
   No disk serial numbers or network addresses are collected.
4. **The computer's own disks are read-only.** `alpctl live protect-disks`
   runs `blockdev --setro` on every disk except the one ALP-OS runs from
   (found by following the root filesystem's device through dm-verity to its
   partition and disk). If that disk cannot be found, every disk is
   protected. It runs early (`alp-live-protect.service`, sysinit) and again
   for every disk added later (udev rule `90-alp-live-protect.rules`).
   Nothing in the image mounts internal disks in the first place; this is
   the second line of defence. T-LIVE-01 checks a write fails and the disk
   is bit-for-bit unchanged.
5. **Build environment for programs in the image (stream rule 11):** static
   `x86_64-unknown-linux-musl` builds with the pinned Rust 1.99.0 toolchain
   (target added in `rust-toolchain.toml`). No dependency on the image's
   glibc; one toolchain for host and image code. Paths are remapped and
   symbols stripped for reproducible output. Alternative rejected: building
   inside Debian trixie (its rustc is older than the workspace's
   `rust-version`).
6. **Hardware detection** is `alp_hw::inspect` (stream rule 12): sysfs,
   procfs and the udev database only; checks via `inspect::assess` with the
   minimums read from `profiles/generic-x86_64.toml`.

## Consequences

- The Installer image (later) must leave out `a-live.conf`'s disk guard and
  report partition (mkosi profiles or a separate drop-in set).
- User-facing failures on the Live screen are plain sentences for now;
  numbered error codes (stream A range 100–199) follow with the full Live.
