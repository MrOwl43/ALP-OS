#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Builds alp-screen (the native screen, ADR 0028) against the image's own
# GTK 4 / libadwaita: inside a pinned Debian trixie container whose apt
# sources point at the image's snapshot (image/mkosi.conf Snapshot=), so the
# binary links exactly the libraries the image ships. alpd and alpctl stay
# static musl builds (ADR 0007); a GTK program cannot be static.
#
# Usage: tools/build-screen.sh [out-dir]   (default target/alp-screen)
# Needs docker or podman. Takes a cargo slot (tests/lib/locks.sh).
set -euo pipefail
cd "$(dirname "$0")/.."

# debian:trixie-20261123, digest read from Docker Hub on 2026-11-24.
IMAGE="docker.io/library/debian@sha256:3cda3ca472ce45507a5109c6a3907093e24d9ae67d55954c9a07a45665dafba2"
SNAPSHOT=$(sed -n 's/^Snapshot=//p' image/mkosi.conf)
TOOLCHAIN=$(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml)
OUT=${1:-target/alp-screen}
[[ -n $SNAPSHOT && -n $TOOLCHAIN ]] || { echo "build-screen: snapshot or toolchain not found" >&2; exit 1; }

engine=$(command -v podman || command -v docker || true)
[[ -n $engine ]] || { echo "build-screen: needs podman or docker" >&2; exit 1; }
mkdir -p "$OUT"

# shellcheck disable=SC2016  # the script runs inside the container
tests/lib/locks.sh cargo "$engine" run --rm \
    -v "$PWD:/src:ro" -v "$PWD/$OUT:/out" \
    -e SNAPSHOT="$SNAPSHOT" -e TOOLCHAIN="$TOOLCHAIN" \
    "$IMAGE" bash -euo pipefail -c '
        rm -f /etc/apt/sources.list.d/debian.sources
        echo "deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/$SNAPSHOT/ trixie main" >/etc/apt/sources.list
        # The container is newer than the snapshot: pin everything to the
        # snapshot and step its base packages back, so the -dev packages
        # match and the binary links what the image ships.
        printf "Package: *\nPin: origin snapshot.debian.org\nPin-Priority: 1001\n" >/etc/apt/preferences.d/snapshot
        apt-get update -q
        apt-get -y -q --allow-downgrades dist-upgrade
        apt-get install -y -q --no-install-recommends ca-certificates curl gcc libc6-dev pkg-config libgtk-4-dev libadwaita-1-dev libwebkitgtk-6.0-dev
        # The toolchain of rust-toolchain.toml, from rustup (same as CI).
        curl -fsSL https://sh.rustup.rs | sh -s -- -y -q --profile minimal --default-toolchain "$TOOLCHAIN"
        . "$HOME/.cargo/env"
        # The sources without build output (target/ and out/ can be many GB).
        mkdir /build && tar -C /src --exclude=./target --exclude=./out --exclude=./.git -cf - . | tar -C /build -xf - && cd /build
        CARGO_PROFILE_RELEASE_STRIP=symbols RUSTFLAGS="--remap-path-prefix=/build=/alp-os" \
            cargo build --release --locked -p alp-screen --features gui
        install -m 0755 target/release/alp-screen /out/alp-screen
    '
echo "build-screen: $OUT/alp-screen"
