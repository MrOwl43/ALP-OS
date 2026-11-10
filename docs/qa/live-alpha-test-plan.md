<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# ALP-OS Live alpha: acceptance and test plan (QA)

Owner request 2026-10-18 (docs/plans/live-alpha.md), redesigned by the owner decisions of
2026-10-19 (docs/plans/live-alpha-decisions-2026-10-19.md, ADR 0011). Owner laptop: ASUS VivoBook
K3402ZA, Wi-Fi only, Secure Boot off. VM = automatic test in QEMU/OVMF
(`just test-vm`); HW = manual check on the real laptop (script in section 3).
Status: P = passes today, N = written, skips until the feature exists, M = manual only.

## 1. Requirement to test

| # | Owner requirement | VM test | HW check | Status |
|---|---|---|---|---|
| 1 | Hardware check, nothing changed; kiosk on tty1 (the text screen is only the fallback and is not expected) | T-LIVE-01 (report, internal disk read-only and bit-identical, `alp-kiosk.service` running on tty1, `alp-live-screen.service` not running) | H1 | P |
| 2 | Wired network (DHCP, firewall drops the rest) | T-LIVE-01 (DHCP, `policy drop`) | not possible on the K3402ZA (no port); USB-Ethernet if available | P |
| 3 | Wi-Fi: list networks, join, password never logged | N: T-API-01 checks `live/wifi/*` is refused with a bad Origin; joining is real-air only | H3 (Wi-Fi firmware, join, address) | M |
| 4 | Disks with an operating system are refused and never written; internal disks stay read-only; the archive goes on an EXTERNAL disk or USB stick | T-LIVE-02 (internal virtio disk with ESP + NTFS-type partition: not offered, create on it refused, byte-identical afterwards; `-device usb-storage` disk gets the archive) | H4 (external disk; Windows still boots after) | N |
| 5 | Archive and index encrypted; index on the stick opens by itself with the TPM, the admin password is the fallback | T-LIVE-02 case tpm (swtpm state kept: reopens after reboot with no password); case password (no TPM: state `locked`, wrong passwords 403 then 429 back-off, `POST unlock` opens it, file still there) | H5 (file `ALPArchive/archive.alp` is not readable text; TPM laptop restarts without a password) | N |
| 5b | Admin password chosen at first start | T-LIVE-02 (`lock.state` `setup`, `POST password/setup`, then `open`) | H3b | N |
| 5c | Recovery key reopens the external archive from a re-written stick (spec block 5) | T-LIVE-02 boot 3 (fresh stick overlay + same external disk; `has_archive` true; recovery restore; file still there). Skips cleanly while the image has no `lock` block / recovery endpoint | H5b (paper key) | N (skips) |
| 6 | Phone backup via Immich over the home network | T-IMMICH-01 (upload through the 443 proxy, reboot, sha256 equal) | H6 (real phone) | N |
| 7 | Delete on the phone/app never deletes on the server | T-IMMICH-01 (DELETE assets / trash empty refused, photo remains) | H6 step 5 | N |
| 8 | Sync chain create / join | N: T-API-01 covers state and Origin rule; create/join round trip to be added when `live/chain/*` is documented (two VMs on one socket network) | H7 (needs a second device) | M |
| 9 | Polished kiosk UI on the computer's own screen, Windows-style lock screen with non-sensitive status, one sign-in, idle auto-lock | T-UI-01 (screendump PNG under `out/qa/`, not blank; no `.manifest` needed). Lock screen and status are checked by eye on the PNG; the password path is T-LIVE-02 | H2, H8, H9 (look and feel is judged by eye) | N |
| 10 | Works from a USB stick with Secure Boot off | T-BOOT-01/03 (verity, tamper detected) boot the image as a disk; USB stick path is HW only | H1 (the whole run) | M |
| 11 | Console API only on loopback, bad Origin refused | T-API-01 | - | N |
| 12 | Size and memory budget | T-BUDGET-01 (system image <= 3814 MiB; idle memory <= 1907 MiB) | - | P |

Release gate for the alpha: all VM tests pass with `ALP_QA_STRICT=1` (a SKIP
then counts as a failure) and the owner completes H1 to H8.

## 2. Automatic tests (what they check, cost)

`just test-vm` (guest RAM 1 GiB unless noted; each run prints a `TIMING:` line):
T-BOOT-01 verity root, T-BOOT-03 two tampered images refuse to boot,
T-BUDGET-01 (4 GiB guest, 30 s idle), T-LIVE-01, T-LIVE-02 (now 5 boots: tpm 2, password 3, with swtpm in the first case and an extra USB disk), then the three
new ones: T-API-01 (1 boot), T-IMMICH-01 (2 boots, 4 GiB guest), T-UI-01
(1 boot, 2 GiB). Harness rules (`tests/vm/run-vm.sh`): waits for the host to
settle after an image build (load and free memory), at most 2 VMs and 1 image
build at a time (`tests/lib/locks.sh`), tests use their own images
(`just _build out/qa`, then `tests/vm/t-*.sh out/qa/alp-os.raw`), never
`out/alp-os.raw` while somebody may rebuild it.

Measured on the dev VM (6 CPUs, ~5 GB free, other agents running), image in `out/qa/`:

| Scenario | Before | After | Peak host memory (QEMU) |
|---|---|---|---|
| T-BOOT-01 + T-BUDGET-01 | 42 s + 232 s (181 s of it waiting for a quiet host) | 44 s, one shared boot (`tests/vm/t-core.sh`) | 1.17 GiB (4 GiB guest) |
| T-BOOT-03 | 426 s (2 x 120 s timeout plus waiting) | 114 s (stops 30 s after the expected failure) | 1.0 GiB |
| T-LIVE-01 | 24 s | 26 s | 1.0 GiB (guest 1 GiB, was 2 GiB) |
| T-LIVE-02 | 104 s | 34 s (2 boots) | 1.0 GiB |
| T-API-01 / T-UI-01 / T-IMMICH-01 | new | 29 s / skip / skip today | 1.0 / ~1.05 / ~1.2 GiB expected |

Savings: guest RAM default 1 GiB; no fixed settle wait (only waits while the CPU
is busy or memory is short, max 120 s); one boot for BOOT-01 + BUDGET-01;
negative tests do not wait for the full timeout; QEMU peak memory printed on every run.

T-IMMICH-01 never runs on the dev VM (owner instruction 2026-10-19: Immich crashes it). It skips
unless `GITHUB_ACTIONS=true` (GitHub Actions) or `ALP_ALLOW_IMMICH=1`; `just test-vm` therefore skips it
locally. It uses the external-disk flow (password setup, archive on a `usb-storage` disk, unlock in round 2)
and only needs KVM (or `ALP_ALLOW_TCG=1`), QEMU, curl, jq, socat.

## 3. Manual test script for the owner (about 45 minutes)

You need: the stick written with the new image, the laptop, a phone with the
Immich app, your home Wi-Fi name and password, paper and pen. Do not use a
computer whose data you cannot afford to lose (nothing is deleted or formatted,
but this is an alpha). Tell me what you saw at each step: "OK", or what was
different (a photo of the screen helps).

H1. Start. Plug the stick in, switch on, press the boot-menu key (ASUS: Esc or
F2, then pick the stick). Secure Boot stays off. Expect: a screen within one
minute, never a dark screen that stays dark. It says what is checked and that
nothing on the computer will be changed.

H2. Look. Expect: a clean welcome screen, large readable text, one question at a time, a
button that is clearly the safe choice, no technical words, no error codes unless something is wrong.

The screens come in this order (owner decision 5): Welcome and hardware check, Network,
Admin password, External disk, Phone, Recovery page, All set. There is no chain step; the
sync chain is joined behind the scenes when the phone is paired (seed and join are in Settings).

H3. Network. Choose your Wi-Fi from the list, type the password. Expect: it
connects within 30 seconds and shows the computer's address. Wrong password:
a friendly sentence, you can try again. Restart later: Wi-Fi asks again or remembers (note which).

H3b. Admin password. Type a password of your own twice. Expect: a short wait (up to 30 seconds), no technical words.
Too short or different: a friendly sentence. Write the password down, you need it at every start without a TPM.

H4. External disk. Plug in an external disk or USB stick (not the ALP-OS stick). Expect: it is listed with its free space within
a few seconds; your Windows disk and any disk with an operating system are NOT offered. Pick it. A screen says
nothing on the computer's own disks is changed and shows the folder name `ALPArchive`. After finishing,
plug the external disk into Windows: a folder `ALPArchive` with one big file exists, and the computer's Windows still starts fine.

H5. Encryption and restart. Open the `ALPArchive` folder from Windows: the file cannot be opened or read.
Restart the laptop. With a TPM (the K3402ZA has one; note if the BIOS shows it as "fTPM/PTT"): the lock screen is skipped or opens by itself and the photos are still counted.
Without a TPM: the lock screen asks for the admin password; a wrong one is refused, and after five wrong tries it makes you wait.

H5b. Recovery page. Print or photograph the recovery key and QR. Expect: it says plainly this reopens the archive if the stick is lost.
(Alpha test, optional but valuable: write the image to a second stick, plug in the same external disk, choose "recover" and type the key: your photos come back.)

H6. Phone. Install Immich on the phone, same Wi-Fi. Scan the QR (it holds only the server address).
(1) The phone may warn about the certificate: accept it as the screen says (open owner question 7).
(2) Log in with the family email and the short shared password shown (like "river-4821"). (3) Turn on automatic backup for 10 photos.
(4) The screen shows the photos arriving. (5) Delete one photo in the Immich
app: expect it is refused or only removed on the phone, and the photo stays on
the server screen count. (6) Switch the laptop off by pulling the power (it is
designed to survive), start again: after the sign-in the phone resumes by itself and all photos are still counted.

H7. Sync chain (Settings, not part of setup). Open Settings, show the chain code. Expect: a code or QR, only the chain is
shown (no account, no password). If you have a second device: join it with the code.

H8. Calm. Leave it for 10 minutes: the screen does not crash or flicker; the fan
does not scream; the status stays green.

H9. Lock screen (owner decision 8). After the first start the everyday screen is a Windows-style lock screen. Expect:
only non-sensitive status (device icons with masked names, network, free space, disk health, last backup); no photo names,
no full disk names. One sign-in with the admin password opens the interface, including the gallery (view, scroll, search).
Deleting a file and emptying the trash ask for the password again. Leave it idle: it locks by itself. The photo-frame screensaver is off by default.
Also check: nothing in the interface runs out of space unannounced (free-space warning appears before "full"; uploads pause, not fail).

## 4. Known gaps

- The VM's "computer disk" is a GPT disk with a vfat ESP and an NTFS-type partition (mkfs.ntfs, empty), so the OS-disk refusal is tested by ESP/partition type; the `\\Windows\\System32` file probe, BitLocker and Fast Startup refusals are unit-tested only. H4 covers a real Windows disk once.
- Real TPM sealing is tested with swtpm only; H5 covers a real TPM. Changing the TPM or Secure Boot state (PCR change) is not tested in the VM yet.
- Recovery uses `POST live/recovery/create` and `POST live/archive/recover` (API doc, third change). The console lock screen itself (`console.*`, session cookie, re-ask on delete) is not automated; T-LIVE-02 probes run without a cookie and use only the always-allowed calls. Shown by eye (H9).
- Idle auto-lock, re-ask on delete and the screensaver are UI behaviours: judged by eye (H9), not automated.
- Wi-Fi joining, the phone app certificate warning and the delete behaviour of the real app need real hardware.
- T-IMMICH-01 and T-API-01 field names follow docs/plans/live-alpha-api.md; adjust the scripts when it changes.
