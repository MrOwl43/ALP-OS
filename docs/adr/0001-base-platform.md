# 0001 — Base platform: Debian 13 "trixie"

- **Status:** accepted
- **Date:** 2026-10-16
- **Spec reference:** Section 2.1, block 1 (VERIFY), Section 10 (toolchain versions)

## Context

The spec requires Debian Stable as the image base and asks to VERIFY that its
systemd supports boot counting, systemd-sysupdate, systemd-repart and UKIs,
and to record kernel, systemd, mkosi, btrfs-progs and Podman versions.

## Evidence (checked 2026-10-16)

Sources: `https://deb.debian.org/debian/dists/stable/Release`,
`https://qa.debian.org/madison.php` (suites trixie, trixie-backports),
`https://packages.debian.org` file lists and contents search for trixie/amd64.

| Item | Version in Debian 13 (trixie) | Notes |
|---|---|---|
| Debian Stable | 13.7 "trixie" (Release file dated 2026-09-12) | |
| Kernel (`linux-signed-amd64`) | 6.12.107 in trixie; first image build installed `linux-image-amd64` 6.12.111-1 (newer point release, likely from the security archive) | backports offer up to 7.2.6 |
| systemd | 257.13-1~deb13u1 | |
| mkosi (Debian package) | 25.3-7 | not used for building, see below |
| btrfs-progs | 6.14-1 | backports: 7.1-1~bpo13+1 |
| Podman | 5.4.2+ds1-2 | |
| erofs-utils | 1.8.6-1 | EROFS root is possible |
| cryptsetup | 2.7.5-2 | |
| Samba | 4.22.11+dfsg-0+deb13u1 | |

systemd features required by the spec, and the trixie package that ships them:

| Feature | Present | Package / file |
|---|---|---|
| Boot counting / boot assessment | yes | `systemd-boot`: `systemd-bless-boot`, `systemd-bless-boot-generator`; `systemd`: `systemd-boot-check-no-failures` |
| systemd-boot | yes | `systemd-boot-efi`: `systemd-bootx64.efi` |
| UKI | yes | `systemd-ukify`: `ukify` (stub from `systemd-boot-efi`) |
| systemd-repart | yes | `systemd-repart` (includes verity definitions examples) |
| systemd-sysupdate | yes | `systemd-container`: `/usr/lib/systemd/systemd-sysupdate` (+ `.service`/`.timer`) |

Build host (development VM): Ubuntu 26.04.1 LTS with mkosi 26 (Ubuntu package
26-3). Latest upstream mkosi release: v27.1 (2026-09-28).

## Decision

1. Base the image on Debian 13 "trixie" with its stock kernel 6.12 and
   systemd 257. All features the spec needs are present; no backports needed now.
2. Build with mkosi 26 (`MinimumVersion=26` in `image/mkosi.conf`).

## Consequences / open points

- Whether the 6.12 kernel is enough for the reference hardware (NICs, storage)
  is checked in the hardware matrix; backports are the fallback (would need an ADR).
- mkosi itself is not yet pinned to an exact version for CI builds, and package
  versions are not yet pinned via snapshot.debian.org (mkosi `Snapshot=`).
  Both are required by block 1 (reproducible builds) and are its next tasks.
- Debian trixie man pages do not mark systemd-sysupdate as experimental, but its
  exact behaviour in v257 must be tested in block 2.

## CI runner and KVM

Source: GitHub changelog 2024-04-02 "Hardware accelerated Android virtualization
now available": standard GitHub-hosted Linux runners (including 2-vCPU) expose
KVM; the runner user must be granted access with a udev rule. VM tests can
therefore run in CI. Since PR #3 the CI `image` job builds the image and runs
`just test-vm` on `ubuntu-26.04` (ADR 0003).
