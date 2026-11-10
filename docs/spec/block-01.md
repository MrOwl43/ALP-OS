<!-- Generated from docs/spec.md (Version 0.5) by tools/split-spec.py. Do not edit; edit docs/spec.md and rerun the tool. -->

### Block 1 — Image builder

**Goal.** Produce reproducible, signed images and the Live and Installer images from Debian Stable.

**Decisions.** Use `mkosi` (not live-build, not custom scripts). Outputs: UKI, root filesystem image (EROFS preferred, SquashFS acceptable) with a dm-verity hash partition image, the ALP-OS Live raw USB image now and the ALP-OS Installer hybrid ISO later, system image of at most 2 GB, update bundle (for sysupdate), SBOM. **VERIFY** the Debian Stable release, its kernel, and that `systemd` is recent enough for boot assessment, `systemd-sysupdate`, `systemd-repart` and UKI support; if not, use the newest supported source (for example backports) and record it in an ADR.

**Constraints.** Pin package versions through a snapshot mirror so rebuilds are identical. Build with `SOURCE_DATE_EPOCH`. Architecture is a build parameter (CI also builds aarch64 as a smoke test, no full image). Signing keys never live in the repo; CI signs with injected secrets. Include only packages needed: systemd, btrfs-progs, cryptsetup, nftables, avahi-daemon, podman, samba (in container only), smartmontools, nut-client (optional), ca-certificates, firmware needed for common NICs, storage controllers and display adapters, and the kiosk stack: a Wayland kiosk compositor (for example `cage`) plus a lightweight kiosk browser engine, fonts with wide script coverage (including right-to-left scripts). **VERIFY** the kiosk compositor and browser choice in an ADR (Debian availability, security update cadence, memory footprint, touch support); no full desktop environment.

**Interfaces.** `just build-image` produces `out/` containing all artifacts and a manifest with hashes.

**Acceptance.** Two clean builds of the same commit give identical artifact hashes. The USB image boots in QEMU with OVMF. Image size and idle memory are recorded and checked against budget.

**Out of scope.** ARM images, ZFS modules, a package manager on the target.
