# Components

Every integrated component with its pinned version and license (spec Section 2.1).
Licenses are taken from each package's Debian copyright file or GitHub's license data.
"TBD" entries must be filled before the component ships in an image.

| Component | Version | Pinned how | License | Purpose |
|---|---|---|---|---|
| Debian | 13 "trixie" | snapshot.debian.org 20261016T023245Z | various (DFSG) | image base |
| Rust toolchain | 1.99.0 | `rust-toolchain.toml` | MIT / Apache-2.0 | build |
| mkosi | 26-3 (Ubuntu) | `tests/ci/build-tools.lock` | LGPL-2.1-or-later | image builder |
| cargo-deny | 0.20.2 | CI workflow | MIT / Apache-2.0 | CI dependency checks |
| cargo-audit | 0.22.2 | CI workflow | MIT / Apache-2.0 | CI advisory checks |
| ShellCheck | 0.11.0 | CI workflow; Ubuntu package on dev VM | GPL-3.0-or-later (Debian copyright) | shell script lint |
| systemd-repart, systemd-ukify | 259.5-0ubuntu3.4 | `tests/ci/build-tools.lock` | mainly LGPL-2.1-or-later, parts GPL-2.0-or-later (Debian copyright) | disk layout, UKI (build host) |
| erofs-utils | 1.9.1-1 | `tests/ci/build-tools.lock` | GPL-2.0-or-later / GPL-2.0-only; some files GPL-2.0 or Apache-2.0 (Debian copyright) | EROFS root (build host) |
| QEMU | 1:10.2.1+ds-1ubuntu3.2 | `tests/ci/build-tools.lock` | GPL-2.0 / LGPL-2 with additions (Debian copyright) | VM tests |
| OVMF (EDK II) | 2025.11-3ubuntu7.3 | `tests/ci/build-tools.lock` | BSD-2-Clause-Patent, BSD-3-Clause, OpenSSL and others (Debian copyright) | UEFI firmware for VM tests |
| actions/checkout | v7.0.1 | commit hash in workflows | MIT | CI |
| actions/upload-artifact | v7.0.2 | commit hash in workflows | MIT | CI |
| taiki-e/install-action | v2.87.28 | commit hash in workflows | Apache-2.0 (GitHub license data) | CI tool installs |
| zeroize (crate) | =1.9.1 | `Cargo.toml` workspace + `Cargo.lock` | Apache-2.0 OR MIT | zeroizing secrets (`alp-crypto`) |
| serde (crate) | =1.0.229 | `Cargo.toml` workspace + `Cargo.lock` | MIT OR Apache-2.0 | (de)serialization; test-only for now |
| toml (crate) | =1.1.8 | `Cargo.toml` workspace + `Cargo.lock` | MIT OR Apache-2.0 | test-only: error catalog check |
| python3-yaml | 6.0.3-1build1 | `tests/ci/build-tools.lock` | MIT (Debian copyright) | API contract check |
| iwd | 3.8-2 | snapshot.debian.org 20261016T023245Z | LGPL-2.1-or-later (Debian copyright) | Wi-Fi on Live (ADR 0008) |
| wireless-regdb | 2026.05.30-1~deb13u1 | snapshot | ISC (Debian copyright) | Wi-Fi regulatory channels |
| firmware-iwlwifi, -realtek, -mediatek, -atheros, -brcm80211 | 20250410-2 | snapshot (non-free-firmware) | redistributable non-free firmware (Debian copyright; not project code) | Wi-Fi and network chips (ADR 0008) |
| nftables, iproute2, systemd-timesyncd | 1.1.3-1, 6.15.0-1, 257.13-1~deb13u1 | snapshot | GPL-2.0 (nftables, iproute2), LGPL-2.1-or-later (systemd) | firewall, diagnostics, time |
| Immich server | v3.3.3 (`ghcr.io/immich-app/immich-server`, index sha256:bae7664f…) | digest in `image/immich/images.lock` | AGPL-3.0 (GitHub license data) | photo service for phone backup (ADR 0015); the image bundles ffmpeg (GPL) and other packages: source offer via upstream, SBOM VERIFY |
| Immich Postgres | 14 with VectorChord 0.4.3 and pgvecto.rs 0.2.0 (`ghcr.io/immich-app/postgres`, index sha256:bcf63357…) | digest in `image/immich/images.lock` | PostgreSQL License; VectorChord AGPL-3.0 or ELv2 (VERIFY); pgvecto.rs Apache-2.0 (VERIFY) | Immich's database |
| valkey | 9.1.2 (`docker.io/valkey/valkey:9`, index sha256:c123e371…) | digest in `image/immich/images.lock` | BSD-3-Clause (VERIFY) | Immich's job queue ("redis") |
| podman, conmon, netavark, aardvark-dns | 5.4.2+ds1-2+b2, 2.1.12-4, 1.14.0-2, 1.14.0-3 | snapshot | Apache-2.0 (Debian copyright, VERIFY in the manifest) | rootless containers, Quadlet (ADR 0015) |
| crun, passt, catatonit | 1.21-1, 0.0~git20250503.587980c-2+deb13u1, 0.2.1-2+b14 | snapshot | crun GPL-2.0-or-later (libcrun LGPL-2.1-or-later); passt GPL-2.0-or-later AND BSD-3-Clause; catatonit GPL-2.0-or-later (all VERIFY, Debian copyright) | OCI runtime, pod network (pasta), pod init |
| uidmap, dbus-user-session | 1:4.17.4-2, Debian trixie (VERIFY) | snapshot | uidmap (shadow) BSD-3-Clause and GPL-2.0-or-later; dbus AFL-2.1 OR GPL-2.0-or-later (VERIFY) | rootless ID ranges, the immich user's D-Bus |
| nginx | 1.26.3-3+deb13u7 | snapshot | BSD-2-Clause (Debian copyright) | HTTPS proxy for the phone app, certificate page (ADR 0015) |
| openssl | 3.5.7 (Debian trixie, VERIFY in the manifest) | snapshot | Apache-2.0 | device CA and server certificate (`alp-live-tls`) |
