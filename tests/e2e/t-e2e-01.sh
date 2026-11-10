#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# T-E2E-01: the beta gate (NEXT_TASK session 3), fully automated in a VM.
#
# Written first. Each step below states exactly what it asserts; the owning
# stream replaces the step's body (which fails until then). The gate passes
# only when all steps pass in --runs consecutive runs (default 3). Not part
# of CI on pull requests until it can pass (main must stay green).
#
# Each step runs in a separate bash process with `set -euo pipefail`, so any
# failing command fails the step. (A subshell is not enough: when the caller
# uses `if`/`||`, bash ignores `set -e` inside the whole call, subshells
# included.) Step functions and helpers are exported with `export -f`; steps
# get the image path as their argument and must not rely on other globals.
# `--self-test` proves the runner reports a failure hidden inside a step.
#
# Usage: tests/e2e/t-e2e-01.sh [--runs N] <installer-usb-image>
#        tests/e2e/t-e2e-01.sh --self-test
set -euo pipefail

# Run exported step function $1 with argument $2 in a fresh bash process.
run_step() {
    "$BASH" -c 'set -euo pipefail; "$@"' run_step "$1" "$2"
}

if [[ ${1:-} == --self-test ]]; then
    # shellcheck disable=SC2329  # called through run_step
    hidden_failure() { false; true; }
    # shellcheck disable=SC2329  # called through run_step
    clean_step() { true; }
    export -f hidden_failure clean_step
    rc=0; run_step hidden_failure x || rc=$?
    (( rc != 0 )) || { echo "FAIL: self-test: a failing command inside a step was hidden" >&2; exit 1; }
    run_step clean_step x || { echo "FAIL: self-test: a clean step failed" >&2; exit 1; }
    for bad in 0 x -1 ""; do
        if "$0" --runs "$bad" "$0" >/dev/null 2>&1; then
            echo "FAIL: self-test: --runs '$bad' was accepted" >&2; exit 1
        fi
    done
    echo "ok: T-E2E-01 step runner reports failures inside steps and rejects bad --runs"
    exit 0
fi

runs=3
if [[ ${1:-} == --runs ]]; then runs=${2:-}; shift 2 || shift $#; fi
[[ $runs =~ ^[1-9][0-9]*$ ]] || { echo "FAIL: --runs must be a positive number" >&2; exit 2; }
image=${1:?usage: t-e2e-01.sh [--runs N] <installer-usb-image>}
[[ -f $image ]] || { echo "FAIL: installer image not found: $image" >&2; exit 2; }

not_implemented() { echo "    not implemented yet [stream $1]" >&2; return 1; }

# install [D]: boot the USB image with one empty 64 GB virtual disk; drive
#   the installer API (api/installer.openapi.yaml) only through its own
#   steps; the only scripted input is the confirmation word. Assert: the disk
#   matches docs/contracts/disk-layout.md (tests/contracts layout check on
#   the real disk), the first slot holds alp-os_<version>, the UKI is
#   alp-os_<version>+3.efi, and the USB image is byte-identical before/after.
step_install() { not_implemented D; }

# first-boot [C+D]: boot the installed disk without the USB image. Assert:
#   GET /api/v1/firstboot reaches complete=true with every step "done";
#   the console PIN can be set once without a session and then never again
#   without one; the recovery sheet is offered; the ledger has FirstBoot.
step_first_boot() { not_implemented "C+D"; }

# pool [B]: Assert: alp-data is LUKS2 (cryptsetup luksDump in the VM shows
#   two keyslots: pool key and recovery key), Btrfs data and metadata are
#   dup, subvolumes @archive @snapshots @system-backup @apps exist, and no
#   key material appears in the journal (grep for the key's hex and base64).
step_pool() { not_implemented B; }

# immich [E]: Assert: Immich answers through the LAN listener (alpd reverse
#   proxy) at the beta server_url; its containers run rootless; machine
#   learning is off; Immich's own port is not reachable from the LAN.
step_immich() { not_implemented E; }

# upload [E]: create a phone with POST /api/v1/devices; upload 200 test
#   photos (fixed seed, recorded SHA-256 list) through the Immich API with
#   the returned account. Assert: 200 assets, each hash matches.
step_upload() { not_implemented E; }

# reboot [A]: reboot the VM. Assert: alp-healthy.target reached, the slot is
#   blessed (no boot counter in the UKI name), pool unlocked unattended
#   (alp-storage.target reached, no ALP-KEY-002).
step_reboot() { not_implemented A; }

# photos-intact [E]: Assert: all 200 assets present, every hash matches.
step_photos_intact() { not_implemented E; }

# broken-update [A]: publish an update whose alp-healthy.target fails (a
#   required unit exits 1) to the local update source; apply it. Assert: the
#   VM booted the new version at least once (console shows its version),
#   boot counting reached zero, the previous version booted again,
#   /api/v1/updates reports rolled-back, ALP-BOOT-001 is raised.
step_broken_update() { not_implemented A; }

# photos-after-rollback [E]: Assert: all 200 assets present, hashes match.
step_photos_after_rollback() { not_implemented E; }

# power-cut [A+E]: start an upload of 50 more photos; kill QEMU (no clean
#   shutdown) after 20 have arrived; boot. Assert: btrfs check --readonly
#   on the pool and a scrub report no uncorrectable errors; ALP-PWR-001
#   raised; resuming the upload completes all 250; every hash matches.
step_power_cut() { not_implemented "A+E"; }

steps=(install first-boot pool immich upload reboot photos-intact
       broken-update photos-after-rollback power-cut)
export -f not_implemented
for id in "${steps[@]}"; do export -f "step_${id//-/_}"; done

one_run() {
    for id in "${steps[@]}"; do
        fn="step_${id//-/_}"
        rc=0
        run_step "$fn" "$image" || rc=$?
        if (( rc == 0 )); then
            echo "  PASS: $id"
        else
            echo "  FAIL: $id" >&2
            return 1   # later steps depend on earlier ones
        fi
    done
}

for ((run = 1; run <= runs; run++)); do
    echo "T-E2E-01 run $run of $runs"
    one_run || { echo "FAIL: T-E2E-01 (run $run)" >&2; exit 1; }
done
echo "PASS: T-E2E-01 ($runs runs in a row)"
