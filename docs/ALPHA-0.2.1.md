<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# ALP-OS Live alpha 0.2.1 (2026-10-26)

File: `out/ALP-OS-Live-alpha-0.2.1.raw` (2.6 GB) on the build VM, built from
commit 13f7b6d of `stream-a/live-alpha`.
SHA-256: `a654a0fb92158caf4b475cd17aa272c212d7b9557c4ad41ab59568b20fa97be8`

Fixes from the owner's alpha 0.2 hardware test (docs/TASKS.md T14-T25).

## What changed

- Phone: Immich now starts after unlock and after a restart (it did not
  before, so the phone page waited forever). The phone page always shows the
  address to type; it is the last setup step: which app, certificate, then
  QR code or typed address, account email and your ALP password.
- Family password removed: phones sign in with the ALP password.
- Gallery opens inside the console (no more "blocked" page), with "Back to
  ALPArchive". The screen locks after 30 minutes of no use while it is open.
- Shutdown: ordered (photo service, archive, stick) with steps on screen,
  then power off; forced after a time limit if something hangs.
- Top bar on every screen: IP address, battery (laptops), coloured disk and
  network icons, home and settings.
- Wi-Fi: one tap opens the password step (double click fixed), controls at
  the top, "Refresh Wi-Fi", icons on buttons.
- Setup steps centred with names and icons; status line on every waiting
  screen that says why when something is slow.
- Password colours: ALP password blue (mountain), recovery code red (owl),
  chain code gold (elephant).
- Settings > This computer: the PC's specs.

## Faster testing in VirtualBox (Windows), instead of a USB stick

1. Copy `ALP-OS-Live-alpha-0.2.1.raw` to Windows. In a Command Prompt in
   the VirtualBox folder (`C:\Program Files\Oracle\VirtualBox`):

       VBoxManage convertfromraw C:\path\ALP-OS-Live-alpha-0.2.1.raw C:\path\alp.vdi --format VDI
       VBoxManage modifymedium disk C:\path\alp.vdi --resize 32768

   (the second line gives it 32 GB, room for the stick's encrypted area).
2. New VM: type Linux, Debian 64-bit, 4096 MB memory or more (8192 better),
   2+ CPUs. "Use an existing virtual hard disk": alp.vdi.
3. Settings: System > Enable EFI (on), Secure Boot off. Display > Graphics
   VMSVGA, 128 MB. Network > Adapter 1 "Bridged Adapter" (so your phone on
   the same Wi-Fi can reach it).
4. Backup disk: plug a real USB stick or disk into the PC and give it to the
   VM (Devices > USB, needs the VirtualBox Extension Pack). A second virtual
   disk counts as an internal disk and is refused on purpose.
5. Start. Every new image: convert again (step 1) and swap the disk.

Works in the VM: screens, setup, password, archive on the USB stick, Immich,
phone connection, gallery, shutdown. Needs the real laptop: Wi-Fi, battery,
security chip (TPM), real internal disks, speed of real sticks.
Not tried yet: the system booting from a virtual SATA disk instead of a
USB stick; if it refuses or the stick area is not made, tell me.

## Please check

1. Wi-Fi: tap and double tap a network (touch and mouse).
2. Top bar: battery, IP address, colours; also on the lock screen.
3. Step row fits on the screen.
4. Status lines while Immich unpacks (first start, some minutes).
5. Phone: QR code in the Immich app, typed address, sign in with the ALP
   password.
6. Gallery opens inside the console; Back returns home.
7. Switch off: steps shown, the computer really turns off.
8. Settings > This computer.
9. Colours in the dark theme.

Everything from alpha 0.2's "not tested" list (docs/ALPHA-0.2.md) still
applies. Nothing here has run on a real computer, a VM or a phone yet.
