# 0027 — Wi-Fi: a second try with WPA3 off (or on) when a password network does not join

- **Status:** accepted
- **Date:** 2026-11-22
- **Spec reference:** block 9; ADR 0008

## Context

An alpha tester's laptop did not join a WPA3 network on Live 0.2.51 with
the right password. iwd lists WPA3-Personal (SAE) and WPA2 networks alike as
`psk`, and stores only the passphrase, which is enough for both. iwd already
falls back to WPA2 on a mixed WPA2/WPA3 network when the driver does not
claim SAE support (`wiphy_select_akm`, iwd 3.10 source). It does not when a
driver claims SAE but does not finish its sign-in; iwd's own answer for those
drivers is the `[DriverQuirks] SaeDisable=<driver>` setting in main.conf
(iwd.config(5), iwd 3.5 or later; the image has iwd 3.8). The laptop's chip
and the router's mode are not known.

## Decision

1. When a password network does not join within 20 s, alpd tries once more
   the other way: it writes `[DriverQuirks] SaeDisable=<driver>` (driver name
   from `/sys/class/net/<dev>/device/driver`, letters, digits, `_-.` only) to
   `/var/lib/iwd/.alp-main.conf` on the stick, or removes that file when it
   was already there (a WPA3-only network needs WPA3), restarts iwd, stores
   the password again and connects again.
2. The way that joins is kept; if neither joins, the earlier setting is put
   back. The person then reads that the password was not accepted, and that a
   WPA3-only router may not work with this computer's Wi-Fi (switch the
   router to WPA2/WPA3 mixed, or use a cable).
3. The iwd unit binds the stick's file read-only over `/etc/iwd/main.conf`
   (`BindReadOnlyPaths=-…`; the image ships a comment-only main.conf as the
   mount point), so the setting lasts across restarts and needs no writable
   `/etc`.
4. `live/state` carries `network.wpa3_off`; the console says when this
   computer uses WPA2 because WPA3 sign-in did not finish.
5. The diagnosis file gets a Wi-Fi part: device, driver, WPA3 on/off,
   `iwctl station show` and the last 200 lines of iwd's journal (iwd logs no
   passwords; network names appear).

## Consequences

- A wrong password now takes about a minute to be refused (two tries and two
  iwd restarts) instead of 20 s.
- With WPA3 off, this computer cannot join WPA3-only networks; the second try
  turns it back on for such a network, and the screen says so.
- The real failure (a driver that claims SAE and fails) cannot be simulated.
  T-WIFI-01 checks a real join on simulated radios and that the stick's file
  reaches iwd; unit tests cover the rest. Confirmation needs the tester's
  laptop.
