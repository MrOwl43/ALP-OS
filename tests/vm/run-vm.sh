#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Shared VM runner for the scenarios in tests/vm/t-*.sh.
# Boots a disk image headless in QEMU with OVMF (UEFI) and watches the serial
# console for the marker printed when systemd reaches alp-healthy.target.
#
# Usage: tests/vm/run-vm.sh <disk-image> [timeout-seconds]
# Env:   OVMF_CODE, OVMF_VARS  override firmware paths
#        ALP_ALLOW_TCG=1       run without KVM (software emulation, very slow)
#        ALP_RAM=<size>        guest memory (default 1G)
#        ALP_EXPECT_FAIL=<ERE> negative test: pass only if <ERE> appears on the
#                              console and alp-healthy.target is still not
#                              reached when QEMU exits or the timeout expires
#        ALP_PROBE=<commands>  newline-separated absolute commands run in the
#                              guest after alp-healthy.target by a test-only
#                              unit injected as systemd credentials (the image
#                              is unchanged); their output is printed between
#                              ALP-PROBE-BEGIN and ALP-PROBE-END on stdout.
#                              '%' and '$' are escaped, so systemd runs each
#                              command literally (no specifiers or variables).
#        ALP_PROBE_DELAY=<s>   seconds to wait before the probe runs (default 0)
#        ALP_QEMU_ARGS=<args>  extra QEMU arguments, whitespace-separated
#        ALP_OVERLAY=<file>    keep the image overlay in <file> across runs
#        ALP_HOST_HOOK=<cmd>   run on the host (bash -c) once alp-healthy.target is
#                              reached (and the probe is done), with the VM still
#                              running; the VM stops when it ends and its exit
#                              status is the result. Gets ALP_MONITOR_SOCK (QEMU
#                              HMP socket, for screendump) and ALP_SERIAL_LOG.
#        ALP_TPM=1             attach a virtual TPM 2.0 (swtpm). Its state lives in
#                              ALP_TPM_STATE=<dir> if set (kept across boots of the
#                              same stick, the caller deletes it), else it is thrown away
#        ALP_SETTLE_MAX=<s>    wait up to <s> (default 120) for the host to calm
#                              down before booting: at least 30% CPU idle and enough
#                              free memory for the guest (after an image build)
#        ALP_SETTLE_SKIP=1     do not wait
#        ALP_FAIL_GRACE=<s>    with ALP_EXPECT_FAIL: seconds to keep watching after
#                              the expected message before passing (default 30)
# Prints one "TIMING:" line at the end: settle wait, boot time, peak host
# memory (resident set of QEMU, MiB).
set -euo pipefail

# At most one test VM at a time on this machine (tests/lib/locks.sh).
if [[ -z ${ALP_VM_SLOT_HELD:-} ]]; then
    ALP_VM_SLOT_HELD=1 exec "$(dirname "$(realpath "$0")")/../lib/locks.sh" vm "$0" "$@"
fi

image=${1:?usage: run-vm.sh <disk-image> [timeout-seconds]}
timeout_s=${2:-180}
marker='ALP-OS: alp-healthy.target reached'
ovmf_code=${OVMF_CODE:-/usr/share/OVMF/OVMF_CODE_4M.fd}
ovmf_vars=${OVMF_VARS:-/usr/share/OVMF/OVMF_VARS_4M.fd}
expect_fail=${ALP_EXPECT_FAIL:-}
probe=${ALP_PROBE:-}

[[ -f $image ]] || { echo "FAIL: image not found: $image (run 'just build-image')" >&2; exit 2; }
for f in "$ovmf_code" "$ovmf_vars"; do
    [[ -f $f ]] || { echo "FAIL: OVMF firmware not found: $f" >&2; exit 2; }
done

if [[ -w /dev/kvm ]]; then
    accel=(-accel kvm -cpu host)
elif [[ ${ALP_ALLOW_TCG:-0} == 1 ]]; then
    echo "WARNING: /dev/kvm unavailable, using slow software emulation" >&2
    accel=(-accel tcg -cpu max)
    timeout_s=$((timeout_s * 10))
else
    echo "FAIL: /dev/kvm is missing or not writable." >&2
    echo "      Enable nested virtualization for this VM (see docs/PROGRESS.md)," >&2
    echo "      or set ALP_ALLOW_TCG=1 to try slow software emulation." >&2
    exit 2
fi

ram_mib=1024
# 2G: the initrd (~240 MB with the kiosk drivers) does not load in 1G
# ("EFI stub: Failed to load initrd: Out of resources").
ram=${ALP_RAM:-2G}
case $ram in *G) ram_mib=$(( ${ram%G} * 1024 )) ;; *M) ram_mib=${ram%M} ;; esac
# Wait until the machine is calm: right after an image build the page cache and
# build processes hold memory, and a VM started then can time out.
settle_start=$SECONDS
if [[ ${ALP_SETTLE_SKIP:-0} != 1 ]]; then
    settle_max=${ALP_SETTLE_MAX:-120}
    # A guest only touches what it uses: about 1.2 GiB resident, even with 4 GiB.
    need_kib=$(( ((ram_mib < 1536 ? ram_mib : 1536) + 700) * 1024 ))
    while :; do
        # Busy now = less than 30 percent of the CPU idle over a 3 s sample
        # (the 1-minute load average lags and stays high long after a build).
        read -r _ u1 n1 s1 i1 _ </proc/stat; sleep 3; read -r _ u2 n2 s2 i2 _ </proc/stat
        t1=$((u1 + n1 + s1 + i1)); t2=$((u2 + n2 + s2 + i2))
        idle_pct=$(( 100 * (i2 - i1) / (t2 - t1 + 1) ))
        avail=$(awk '/^MemAvailable:/ {print $2}' /proc/meminfo)
        if (( idle_pct >= 30 && avail >= need_kib )); then break; fi
        if (( SECONDS - settle_start >= settle_max )); then
            echo "WARNING: host still busy after ${settle_max}s (CPU idle ${idle_pct}%, ${avail} kB free); booting anyway" >&2
            break
        fi
    done
fi
settle_s=$((SECONDS - settle_start))

work=$(mktemp -d)
qemu_pid=
swtpm_pid=
# shellcheck disable=SC2329  # invoked via trap
cleanup() {
    # QEMU first: swtpm does not end on SIGTERM while QEMU holds its socket,
    # so waiting for it first hung the run after the probe (T-UI-02).
    if [[ -n $qemu_pid ]]; then kill "$qemu_pid" 2>/dev/null || true; wait "$qemu_pid" 2>/dev/null || true; fi
    if [[ -n ${swtpm_pid:-} ]]; then kill "$swtpm_pid" 2>/dev/null || true; wait "$swtpm_pid" 2>/dev/null || true; fi
    rm -rf "$work"
}
trap cleanup EXIT

cp "$ovmf_vars" "$work/vars.fd"
# Boot a throwaway overlay so the test never modifies the image under test.
# ALP_OVERLAY=<file> keeps the overlay between runs (a second boot of the
# same stick); the caller deletes it.
overlay=${ALP_OVERLAY:-$work/disk.qcow2}
[[ -f $overlay ]] || qemu-img create -q -f qcow2 -F raw -b "$(realpath "$image")" "$overlay" "${ALP_OVERLAY_SIZE:-8G}"  # sparse; room for the index partition
serial_log="$work/serial.log"
monitor=(-monitor none)
if [[ -n ${ALP_HOST_HOOK:-} ]]; then monitor=(-monitor "unix:$work/monitor.sock,server,nowait"); fi
read -r -a extra_args <<<"${ALP_QEMU_ARGS:-}"
if [[ ${ALP_TPM:-0} == 1 ]]; then
    tpm_dir=${ALP_TPM_STATE:-$work/tpm}
    mkdir -p "$tpm_dir"
    swtpm socket --tpm2 --tpmstate "dir=$tpm_dir" --ctrl "type=unixio,path=$work/swtpm.sock" >"$work/swtpm.log" 2>&1 &
    swtpm_pid=$!
    for _ in $(seq 50); do [[ -S $work/swtpm.sock ]] && break; sleep 0.1; done
    [[ -S $work/swtpm.sock ]] || { echo "FAIL: swtpm did not start" >&2; cat "$work/swtpm.log" >&2; exit 2; }
    extra_args+=(-chardev "socket,id=chrtpm,path=$work/swtpm.sock")
    extra_args+=(-tpmdev "emulator,id=tpm0,chardev=chrtpm" -device "tpm-tis,tpmdev=tpm0")
fi

if [[ -n $probe ]]; then
    unit="[Unit]
Description=ALP test-only probe
After=alp-healthy.target
ConditionPathExists=!/etc/initrd-release
[Service]
Type=oneshot
StandardOutput=tty
TTYPath=/dev/console
ExecStartPre=/bin/sleep ${ALP_PROBE_DELAY:-0}
ExecStart=/bin/echo ALP-PROBE-BEGIN"
    while IFS= read -r cmd; do
        [[ -z $cmd ]] && continue
        cmd=${cmd//%/%%}; cmd=${cmd//\$/\$\$}
        unit+=$'\n'"ExecStart=$cmd"
    done <<<"$probe"
    unit+=$'\n'"ExecStart=/bin/echo ALP-PROBE-END"
    dropin=$'[Unit]\nWants=alp-test-probe.service'
    cred=io.systemd.credential.binary
    extra_args+=(
        -smbios "type=11,value=$cred:systemd.extra-unit.alp-test-probe.service=$(printf '%s\n' "$unit" | base64 -w0)"
        -smbios "type=11,value=$cred:systemd.unit-dropin.alp-healthy.target=$(printf '%s\n' "$dropin" | base64 -w0)"
    )
fi

qemu-system-x86_64 \
    -machine q35 "${accel[@]}" -smp 2 -m "$ram" \
    -drive if=pflash,format=raw,readonly=on,file="$ovmf_code" \
    -drive if=pflash,format=raw,file="$work/vars.fd" \
    -drive if=virtio,format=qcow2,file="$overlay" \
    -display none -serial file:"$serial_log" "${monitor[@]}" \
    -nic none -no-reboot "${extra_args[@]}" &
qemu_pid=$!

peak_kib=0
boot_start=$SECONDS
sample_peak() {
    local v
    v=$(awk '/^VmHWM:/ {print $2}' "/proc/$qemu_pid/status" 2>/dev/null) || return 0
    [[ -n $v ]] && peak_kib=$v
    return 0
}
timing() { echo "TIMING: settle ${settle_s}s, boot ${1}s, guest ${ram_mib} MiB, peak host RSS $((peak_kib / 1024)) MiB"; }
run_hook() {
    local rc=0
    ALP_MONITOR_SOCK="$work/monitor.sock" ALP_SERIAL_LOG="$serial_log" bash -c "$ALP_HOST_HOOK" || rc=$?
    sample_peak
    timing $((SECONDS - boot_start))
    exit "$rc"
}
console_tail() { echo "Last console lines:" >&2; tail -n 40 "$serial_log" >&2 || true; }
seen() { grep -qE "$1" "$serial_log" 2>/dev/null; }

deadline=$((SECONDS + timeout_s))
fail_seen=
while (( SECONDS < deadline )); do
    sample_peak
    if grep -qF "$marker" "$serial_log" 2>/dev/null; then
        if [[ -n ${ALP_HOST_HOOK:-} && -z $expect_fail ]] && { [[ -z $probe ]] || seen 'ALP-PROBE-END'; }; then
            [[ -z $probe ]] || sed -n '/ALP-PROBE-BEGIN/,/ALP-PROBE-END/p' "$serial_log" | tr -d '\r'
            run_hook
        fi
        if [[ -n $expect_fail ]]; then
            echo "FAIL: reached alp-healthy.target but a failure was expected" >&2
            exit 1
        fi
        if [[ -z $probe ]]; then
            echo "PASS: reached alp-healthy.target in $((SECONDS - boot_start))s"
            timing $((SECONDS - boot_start))
            exit 0
        fi
        if seen 'ALP-PROBE-END'; then
            echo "Reached alp-healthy.target; probe output:"
            sed -n '/ALP-PROBE-BEGIN/,/ALP-PROBE-END/p' "$serial_log" | tr -d '\r'
            timing $((SECONDS - boot_start))
            exit 0
        fi
    fi
    if [[ -n $expect_fail && -z $fail_seen ]] && seen "$expect_fail"; then
        fail_seen=1
        fail_at=$SECONDS
        grep -m1 -E "$expect_fail" "$serial_log" | tr -d '\r' | sed 's/^/  console: /'
    fi
    # Negative test: once the expected failure is on the console, a boot that is
    # going to reach alp-healthy.target does so within a few seconds. Wait
    # ALP_FAIL_GRACE (default 30) seconds, then pass; do not wait for the full timeout.
    if [[ -n $fail_seen ]] && (( SECONDS - fail_at >= ${ALP_FAIL_GRACE:-30} )); then
        echo "PASS: expected failure seen; alp-healthy.target not reached ${ALP_FAIL_GRACE:-30}s later"
        exit 0
    fi
    if ! kill -0 "$qemu_pid" 2>/dev/null; then
        if [[ -n $fail_seen ]]; then
            echo "PASS: expected failure seen; the VM stopped without reaching alp-healthy.target"
            exit 0
        fi
        echo "FAIL: QEMU exited before alp-healthy.target." >&2
        console_tail
        exit 1
    fi
    sleep 2
done
if [[ -n $fail_seen ]]; then
    echo "PASS: expected failure seen; alp-healthy.target not reached within ${timeout_s}s"
    exit 0
fi
echo "FAIL: timed out after ${timeout_s}s." >&2
console_tail
exit 1
