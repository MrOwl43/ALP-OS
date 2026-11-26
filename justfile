# SPDX-License-Identifier: AGPL-3.0-or-later
# ALP-OS developer commands. Run `just` to list them.

out := justfile_directory() / "out"
image := out / "alp-os.raw"
# Immich images in the image build (ADR 0015): `ALP_IMMICH_IMPORT=0 just
# build-image` builds without them (no download; Immich cannot start).
export ALP_IMMICH_IMPORT := env_var_or_default("ALP_IMMICH_IMPORT", "1")
# Which image (image/mkosi.profiles): "live" (frozen at 0.2.7, Chromium kiosk)
# or "installed" (0.2.8, ADR 0028: native screen and the installer).
profile := env_var_or_default("ALP_PROFILE", "live")

default:
    @just --list

# Format check, lints and unit tests (same as CI)
check:
    cargo fmt --all --check
    tests/lib/locks.sh cargo cargo clippy --workspace --all-targets --locked -- -D warnings
    tests/lib/locks.sh cargo cargo test --workspace --locked
    shellcheck tests/vm/*.sh tests/ci/*.sh tests/contracts/*.sh tests/e2e/*.sh tests/lib/*.sh
    python3 -I tests/contracts/check-openapi.py api/*.openapi.yaml
    python3 -I tools/split-spec.py --check
    python3 -I tests/ui/check-i18n.py
    tests/contracts/check-install-layout.sh
    tests/e2e/t-e2e-01.sh --self-test
    tests/ci/check-snapshot-epoch.sh image/mkosi.conf
    ALP_LOCK_SKIP='^(qemu-system-x86|qemu-utils|ovmf|swtpm|socat|ffmpeg|libheif-examples|libheif-plugin-x265)$' tests/ci/check-build-tools.sh tests/ci/build-tools.lock

# Build the OS disk image into out/ and write out/SHA256SUMS
build-image: (_build out)
    @echo "Artifacts and hashes in {{out}}"

# Block 1 acceptance: two clean builds of this commit give identical hashes
repro-check:
    rm -rf {{out}}/repro-a {{out}}/repro-b
    just _build {{out}}/repro-a
    just _build {{out}}/repro-b
    diff -u {{out}}/repro-a/SHA256SUMS {{out}}/repro-b/SHA256SUMS
    @echo "PASS: two clean builds are identical"

_build dir: _stage
    mkdir -p {{dir}}
    # Each output directory has its own package cache, so repro builds start clean.
    # At most one image build at a time on this machine (tests/lib/locks.sh).
    tests/lib/locks.sh image-build mkosi -C image --profile={{profile}} --image-version="${ALP_IMAGE_VERSION:-$(tools/update-version.sh)}" --output-directory={{dir}} --package-cache-dir={{dir}}/pkgcache -f build
    # Hash only build artifacts (test results such as budget.json are not artifacts).
    cd {{dir}} && rm -f budget.json && find . -maxdepth 1 -type f ! -name SHA256SUMS -printf '%P\n' | LC_ALL=C sort | xargs sha256sum > SHA256SUMS

# Programs and data that go into the image (image/mkosi.conf.d/a-live.conf):
# alpctl and alpd as static musl binaries (ADR 0007), so they run on Debian
# trixie; the Live UI (ui/live, without the browser-only mock) for alpd.
_stage:
    CARGO_PROFILE_RELEASE_STRIP=symbols RUSTFLAGS="--remap-path-prefix={{justfile_directory()}}=/alp-os --remap-path-prefix=$HOME/.cargo=/cargo" \
        tests/lib/locks.sh cargo cargo build --release --locked --target x86_64-unknown-linux-musl -p alpctl -p alp-core -p alp-installer
    # The installed version's own programs (mkosi.profiles/installed): the
    # installer (static, like alpctl) and the native screen, built against
    # the image's own GTK in a pinned container (tools/build-screen.sh).
    rm -rf target/alp-installed-tree && mkdir -p target/alp-installed-tree
    if [ "{{profile}}" = installed ]; then \
        install -D -m 0755 target/x86_64-unknown-linux-musl/release/alp-installer target/alp-installed-tree/usr/bin/alp-installer && \
        mkdir -p target/alp-installed-tree/usr/lib/alp && cp -r --no-preserve=mode,ownership image/install-layout target/alp-installed-tree/usr/lib/alp/install-layout && \
        tools/build-screen.sh target/alp-screen && \
        install -D -m 0755 target/alp-screen/alp-screen target/alp-installed-tree/usr/lib/alp/alp-screen; fi
    rm -rf target/alp-image-tree
    install -D -m 0755 target/x86_64-unknown-linux-musl/release/alpctl target/alp-image-tree/usr/bin/alpctl
    install -D -m 0755 target/x86_64-unknown-linux-musl/release/alpd target/alp-image-tree/usr/bin/alpd
    # Build number for About (owner, 2026-10-30): the commit and its date (not
    # the build time, so two builds of one commit stay identical).
    mkdir -p target/alp-image-tree/usr/lib/alp
    printf '%s %s\n' "$(git rev-parse --short=12 HEAD)$(git diff --quiet HEAD -- . ':!out' 2>/dev/null || echo -dirty)" "$(git log -1 --format=%cs)" >target/alp-image-tree/usr/lib/alp/build
    mkdir -p target/alp-image-tree/usr/share/alp/ui
    cp -r --no-preserve=mode,ownership ui/live target/alp-image-tree/usr/share/alp/ui/live
    rm -f target/alp-image-tree/usr/share/alp/ui/live/mock.js
    install -D -m 0644 profiles/generic-x86_64.toml target/alp-image-tree/usr/share/alp/profiles/generic-x86_64.toml
    # The update key's public half (image/keys/README.md, ADR 0026) as the
    # keyring systemd-sysupdate checks signatures with; without the key an
    # empty keyring, which accepts nothing (and replaces the distribution
    # keys that systemd-container ships there).
    mkdir -p target/alp-image-tree/usr/lib/systemd
    if [ -f image/keys/update-signing.asc ]; then gpg --batch --no-options --dearmor <image/keys/update-signing.asc >target/alp-image-tree/usr/lib/systemd/import-pubring.gpg; else : >target/alp-image-tree/usr/lib/systemd/import-pubring.gpg; fi
    # ttyd for the Terminal (image/terminal/ttyd.lock): upstream's static
    # release, checksum-pinned (Debian trixie has no ttyd package).
    image/terminal/fetch.sh {{out}}/ttyd target/alp-image-tree
    # Immich's pinned container images (image/immich/images.lock, ADR 0015):
    # downloaded once into out/immich-images (every blob digest-checked; a
    # cache) and hard-linked into image/immich-images, where
    # image/immich/import.postinst reads them inside the mkosi build.
    rm -rf image/immich-images
    if [ "$ALP_IMMICH_IMPORT" != 0 ]; then image/immich/fetch.sh {{out}}/immich-images && { cp -al {{out}}/immich-images image/immich-images 2>/dev/null || { rm -rf image/immich-images && cp -r {{out}}/immich-images image/immich-images; }; }; fi

# VM scenarios (docs/qa/live-alpha-test-plan.md): T-CORE (boot, verity, budget),
# T-BOOT-03 tampered root, T-LIVE-01 hardware report and read-only disks,
# T-LIVE-02 encrypted archive, T-API-01 console API, T-IMMICH-01 phone backup,
# T-UI-01 kiosk screen, T-UI-02 second start, T-PAIR-01 phone pairing, T-TERM-01 Terminal. ALP_QA_STRICT=1 turns skips into failures.
test-vm: test-vm-base test-vm-ui test-vm-immich test-vm-storage test-vm-update

# The same scenarios in three groups that each fit CI's hour (they run as
# parallel jobs there; owner, 2026-10-30).
test-vm-base:
    tests/vm/t-core.sh {{image}}
    tests/vm/t-boot-03.sh {{image}}
    tests/vm/t-live-01.sh {{image}}
    tests/vm/t-live-02.sh {{image}}
    tests/vm/t-wifi-01.sh {{image}}

test-vm-ui:
    tests/vm/t-api-01.sh {{image}}
    tests/vm/t-ui-01.sh {{image}}
    tests/vm/t-ui-02.sh {{image}}
    tests/vm/t-pair-01.sh {{image}}
    tests/vm/t-term-01.sh {{image}}

# T-IMMICH-01 (with T-PAIR-02), T-IMMICH-02 (the gallery on a new stick),
# T-FORMAT-01 (the format test set) and T-LIVE-03 need the photo service.
test-vm-immich:
    tests/vm/t-immich-01.sh {{image}}
    tests/vm/t-immich-02.sh {{image}}
    tests/vm/t-format-01.sh {{image}}
    tests/vm/t-live-03.sh {{image}}

# Updates in place (ADR 0026): T-UPD-01 (the stick's layout) and T-UPD-02 (a
# full update: a second image of this commit with a higher version is built,
# offered as update files and started). Its own CI job: two image builds.
test-vm-update:
    tests/vm/t-upd-01.sh {{image}}
    ALP_IMAGE_VERSION=9.9.90 just _build {{out}}/next
    tests/vm/t-upd-02.sh {{image}} {{out}}/next/alp-os.raw 9.9.90

# The installed version (ALP_PROFILE=installed, 0.2.8): T-INST-01 installs
# onto a virtual disk next to a fake Windows disk that must stay untouched.
test-vm-installed:
    tests/vm/t-core.sh {{image}}
    tests/vm/t-screen-01.sh {{image}}
    tests/vm/t-inst-01.sh {{image}}
    tests/vm/t-inst-02.sh {{image}}
    tests/vm/t-immich-03.sh {{image}}

# T-LIVE-04 (reset, cancel, real wipe), T-LIVE-05 (T49 quick and secure
# format) and T-LIVE-06 (read-only Files): external disks, in their own CI job.
test-vm-storage:
    tests/vm/t-live-04.sh {{image}}
    tests/vm/t-live-05.sh {{image}}
    tests/vm/t-live-06.sh {{image}}

# Remove image build output
clean-image:
    mkosi -C image --output-directory={{out}} clean

# T-E2E-01 beta gate (fails until the beta exists; not run on pull requests)
test-e2e image=(out / "alp-os.raw"):
    tests/e2e/t-e2e-01.sh {{image}}

# Live UI (docs/plans/live-ui-0.2.4.md): syntax, texts, and T-UI-SMOKE over the
# mock API in headless Chromium (needs the playwright package; CI job "ui").
check-ui:
    for f in ui/live/*.js; do node --check "$f"; done
    python3 -I tests/ui/check-i18n.py
    node tests/ui/smoke.mjs
