<!-- Generated from docs/spec.md (Version 0.5) by tools/split-spec.py. Do not edit; edit docs/spec.md and rerun the tool. -->

## 6. System overview

**Boot.** UEFI loads systemd-boot, which picks a Unified Kernel Image (UKI) using boot counting. The UKI carries kernel, initrd and command line, including the dm-verity root hash. The initrd assembles the verified read-only root. systemd starts services and reaches `alp-healthy.target`, after which the slot is marked good. If the counter reaches zero first, the previous slot boots automatically.

**Storage after boot.** `alp-storage` unlocks and mounts the data pool after boot, not from the initrd. A pool that cannot be opened never blocks boot and never fails the health target. The system stays reachable and reports `storage unavailable`.

**Install.** A bootable USB image starts a minimal graphical kiosk session that shows the installer screens from `alp-installer`'s local web front end. The user sees a welcome screen, a plain hardware check, the disks with one recommendation, one clear confirmation, progress, and "done". After the first reboot the screen shows the first-boot steps and a QR to join the first phone. **Local console.** Whenever a screen is attached, the same kiosk stack shows the shared UI from `alpd` on localhost.

**Update.** `systemd-sysupdate` fetches signed images from a static HTTPS server, writes the inactive slot, and the box reboots into trial mode in a quiet window.
