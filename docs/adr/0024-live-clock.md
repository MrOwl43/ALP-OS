<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# 0024: The Live clock: internet time, never writing the hardware clock, correction by hand

- **Status:** proposed 2026-11-15 (alpha 0.2.61). Needs the security review
  that every Live change gets before the beta.
- **Date:** 2026-11-15
- **Spec reference:** block 9 (network, time sync).

## Context

Field reports of alpha 0.2.5 and 0.2.6: the computer's clock ran hours
ahead, and phones refused certificates as "not valid yet". Two causes:

1. A computer that also runs Windows keeps its hardware clock in local
   time. Live reads it as UTC, so the clock is off by the zone's offset.
2. The clock was never corrected from the network. systemd-timesyncd was
   in the image, but nothing could look up the time servers' names: there
   was no resolver and no `/etc/resolv.conf`.

0.2.6 dated certificates one day back, which lets phones connect, but the
time on the screen and in the logs stayed wrong.

## Decision

1. **Name lookup: systemd-resolved** (Debian trixie package, same source
   and version as systemd). `/etc/resolv.conf` points at its loopback stub.
   LLMNR and mDNS are off; nothing listens on the network. It only serves
   this computer's own outgoing requests (today: the time servers).
2. **The hardware clock is never written.** `/etc/adjtime` in the image
   says `LOCAL`. systemd-timesyncd then leaves the kernel's
   hardware-clock update off (verified in systemd v257
   `src/timesync/timesyncd.c`: "All system time to RTC updates are
   disabled", and `timesyncd-manager.c` keeps `STA_UNSYNC` set). Without
   this, a clock set from the network is copied to the hardware clock in
   UTC every 11 minutes, and the computer's Windows would then show the
   wrong time. Live must not change the computer. No time zone is
   configured in the system, so the clock read at start is unchanged.
3. **"Sync now"** (`POST clock/sync`): restarts systemd-timesyncd, which
   asks a server at once. `state.clock.synced_at` shows the answer.
4. **Correction by hand** (`POST clock/base`), for a computer with no
   internet: in the time zone window the person sees the two possible
   readings of the hardware clock and picks the right one. For "local
   time", alpd moves the system clock back by the zone's offset (`date`,
   the offset from tzdata). The answer is kept on the stick and applied
   at the next start until the network sets the clock. It is refused once
   the network set the clock.

## Consequences

- The system clock stays UTC; the time zone is still only for display.
- `clock/base` can move the clock by at most 14 hours, only from a
  signed-in console, and never after a network sync. Certificates start
  one day back, so a move backwards does not make them "not valid yet".
- Outgoing traffic gains DNS requests (to the network's resolver) and NTP
  (to the servers systemd-timesyncd is built with, or the network's own).
- A stick moved to another computer keeps the "local time" answer; the
  time zone window corrects it, and a network sync overrides it.
- The installed system (not Live) owns its hardware and should keep the
  hardware clock in UTC and written; that is a separate decision.

## Rejected

- chrony instead of systemd-timesyncd: a new component for the same job.
- Guessing the offset automatically: nothing on the computer says which
  reading is right without a network.

## Addendum 2026-11-26 (0.2.8): certificates and a wrong clock

Network time is not authenticated, and a hardware clock can be years off.
The certificate maker (`alp-live-tls`, Live and installed) now:

1. Makes or renews nothing while the clock is earlier than the image's own
   date (`/usr/lib/os-release`, set by `SourceDateEpoch`) less a day; the
   certificates already there are kept (phones judge them by their own
   clock). With no certificate yet, pairing says to restart and try again.
2. Never replaces the CA because the clock says it expires (before: within
   a year). A time server pushing the clock ahead could otherwise force a
   new CA and make every phone pair again. A CA that really expires (10
   years) becomes a visible step later.
3. Replaces a CA that is "not valid yet" (made while the clock was ahead)
   only once systemd-timesyncd has set the clock.
4. Leaves the server certificate alone while the CA is not valid on the
   current clock. A server certificate renewed on a clock pushed ahead
   starts in the future; once the clock is right, `openssl verify` refuses
   it and it is made again, under the same CA.

Checked on the host by `tests/ci/check-live-tls.sh` (CI shell job).
