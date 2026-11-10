<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Live admin account (spec for the lead)

Owner stance (ADR 0009 #5): a simple password for the Linux admin user,
local console only, no network login. What this protects: casual use of the
computer's keyboard. What it does not: anyone with the stick (Secure Boot
off, key file on the stick).

## Account

- User `admin`, in the image (build time: sysusers.d or mkosi postinst,
  VERIFY which one mkosi runs), home on tmpfs (`tmpfiles`:
  `d /var/home/admin 0700 admin admin -`), shell `/bin/bash`, groups
  `systemd-journal`, `adm`. Shadow entry in the image is locked (`!`).
  Root stays locked.
- The password hash is **not** in `/etc/shadow` (read-only root). It lives
  on the stick: `/var/lib/alp/live/admin.pwd`, mode 0600 root:root, one line
  `admin:$y$…` (yescrypt).
- PAM checks it with `pam_pwdfile` (Debian `libpam-pwdfile` 2.0-1+b1, uses
  libxcrypt `crypt_r`; VERIFIED in the pinned snapshot). Never use its
  `debug` option: it logs the hash.
- Hash made with `mkpasswd --method=yescrypt --stdin` (Debian `whois`
  5.6.3, VERIFIED: yescrypt supported, password read from stdin). The
  password goes only to its stdin: never argv, environment, a file or a log.
  Write the file atomically (temp 0600, fsync, rename, fsync dir). Refuse if
  `/var/lib/alp` is not the mounted `alp-state` partition (review B2).

## Setting the password

- First start: the screen asks "Choose a password for the maintenance login
  (for support only)", typed twice, hidden, minimum 8 characters, no other
  rules. Until set, no admin login exists (file absent = every login fails).
- Can be changed from the Live screen later without the old one (whoever is
  at the screen already has the stick). `passwd` is disabled
  (`password required pam_deny.so`).
- Owner question: chosen (recommended) or shown as a generated password.

## Where login works

- Only `getty@tty2`. logind drop-in: `NAutoVTs=0`, `ReserveVT=0` (no other
  gettys); tty1 is the screen/kiosk.
- `/etc/security/access.conf`: `+:admin:tty2` then `-:ALL:ALL`, enforced by
  `pam_access` in `login`. Serial console (`console=ttyS0` makes a serial
  getty) is therefore refused too.
- No SSH: `openssh-server` is never in the image; the firewall drops 22.

`/etc/pam.d/login` (lead):

```
auth     requisite  pam_nologin.so
auth     required   pam_faildelay.so delay=3000000
auth     requisite  pam_faillock.so preauth
auth     [success=1 default=bad] pam_pwdfile.so pwdfile=/var/lib/alp/live/admin.pwd
auth     [default=die] pam_faillock.so authfail
auth     sufficient pam_faillock.so authsucc
auth     required   pam_deny.so
account  required   pam_access.so
account  required   pam_unix.so
password required   pam_deny.so
@include common-session
```

`/etc/security/faillock.conf`: `deny = 5`, `unlock_time = 300` (state in
`/run/faillock`, reset at reboot). VERIFY in the VM that `pam_unix` account
accepts the locked shadow entry; if not, use `pam_permit.so` after
`pam_access.so`.

Root actions: `sudo` 1.9.16p2 (VERIFIED in snapshot) with
`/etc/sudoers.d/alp-admin`: `admin ALL=(ALL:ALL) ALL` and
`Defaults:admin use_pty, timestamp_timeout=5`; `/etc/pam.d/sudo` uses the
same `pam_pwdfile` + `pam_faillock` + `pam_access` lines.

## Tests (QA, T-SEC-ADMIN-01)

Admin login works on tty2 with the test password; fails on tty3–6 (no
getty), on the serial console and for root; 5 wrong passwords lock it for
5 min; `admin.pwd` is 0600 root and on the stick; the journal and
`/proc/*/cmdline` captured during setup never contain the test password;
no `sshd` in the image; `passwd` as admin fails.
