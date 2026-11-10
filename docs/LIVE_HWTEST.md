<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Testing ALP-OS Live on a real computer (interim version, no phone backup yet)

This version runs entirely from the USB stick. It checks the computer, walks
you through setup, and creates an **encrypted archive on an external disk**.
Phone backup (the gallery) is **not in this version**: that step only says
"coming in the next version".

The computer's own disks are never changed: they are switched to read-only
as soon as ALP-OS starts, and they are not offered as a place for the archive.

## What you need

- A USB stick of 8 GB or more. **Everything on it will be erased.**
- An external USB disk for the archive (at least 1 GB of free space).
  To be safe, use a disk with nothing important on it for this test.
- A wired network (USB-Ethernet adapter) or Wi-Fi.
- The files `alp-os.raw` and `SHA256SUMS` from `out/`.

## 1. Write the image to the stick

First check the file is not damaged: its SHA256 must match the line for
`alp-os.raw` in `SHA256SUMS` (on Windows: `certutil -hashfile alp-os.raw SHA256`).

**Windows:** use [Rufus](https://rufus.ie) or [balenaEtcher](https://etcher.balena.io).
In Rufus pick the stick, click SELECT and choose `alp-os.raw` (set the file
filter to "All files" if it is not listed), then START. Choose "DD image"
mode if Rufus asks.

**Ubuntu:** find the stick with `lsblk` (for example `/dev/sdb`; check the
size so you do not pick the wrong disk), then:

    sudo dd if=alp-os.raw of=/dev/sdX bs=4M conv=fsync status=progress

## 2. Computer settings (BIOS / UEFI setup)

- UEFI on (legacy / CSM boot off).
- **Secure Boot off** (beta images are not signed yet).
- Security chip (TPM) **on** if the computer has one (often called "TPM",
  "PTT" on Intel or "fTPM" on AMD). With it, the archive opens by itself at
  every start. Without it, you type the password at every start.
- Start from the USB stick: usually a key such as F12, F11, F9, Esc or F8
  right after power-on opens the boot menu.

## 3. What you should see (first start)

1. The maker's logo, then the ALPArchive starting screen for up to a minute.
2. **Welcome** with the hardware check: one line per part (OK, note or
   problem).
3. **Network**: plug in the cable, or pick your Wi-Fi and type its password.
4. **Admin password**: choose one and write it down. It opens the archive
   and the lock screen.
5. **Where should your backup live?** Plug in the external disk now if you
   have not. Only the external disk is offered; choose it and the size.
   A progress screen follows.
6. **Connect your phone**: says it is coming in the next version. Press Continue.
7. **Recovery key**: write it down on paper. It is the only way back in if
   the password is lost or the stick breaks.
8. The finish screen, then the home screen.

After about 5 minutes without use, the screen locks. Sign in with the admin
password.

## 4. What to test after setup

- **Restart with the TPM on:** the archive opens by itself; you see the lock
  screen. Sign in with the password.
- **Restart without a TPM (or TPM off in BIOS):** ALPArchive asks for the
  password before anything else. A wrong password is refused, and after
  several wrong tries it asks you to wait a little.
- **Restart without the external disk:** the home screen says the backup
  disk is missing. Plug it back in.
- **Lock screen:** press Lock (or wait 5 minutes), then sign in again.

To turn off, press the power button briefly. The stick also keeps a
hardware report: on a Windows or Ubuntu computer a drive named
**ALP-REPORT** appears with one file per computer tested.

## 5. Known gaps in this version

- No phone backup and no gallery. "Open my gallery" on the home screen does
  not work yet, and some texts already talk about photos.
- Using a fresh stick with an existing external disk (the recovery key) was
  only tested in a virtual machine.

## 6. What to report

- Where it stopped or looked wrong, with a photo of the screen.
- The computer model, whether it has a TPM, and network type (cable or Wi-Fi).
- The ALP-REPORT files.
- If the screen stays dark for more than two minutes, or the computer does
  not start from the stick: the model and a photo of its boot menu.
