<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# The Terminal: security notes (2026-11-04)

Plan and rules: `docs/plans/live-terminal.md` (revision 2, review T1-T6).
Server review: `terminal-server-review-2026-11-04.md` (Security and Network
Expert; approved, with R1 and R2 before merge).

## Where the protection is

- **The user.** The shell runs as `alp-term` (uid 58413), with no password, no login, no sudo, and no group but its own.
- **The firewall.** An `nftables` output rule refuses every new connection that alp-term opens to this computer: loopback, `::1` and the computer's own addresses (`firewall-live.md`). The unit does not start without the rule.
- **The sandbox.** `alp-terminal.service`: an empty, read-only `/run`, with `/var/lib`, `/var/log` and the mount points hidden, no capabilities, no new namespaces, and other users' processes hidden.
- **Netlink is allowed in the sandbox.** ttyd's libwebsockets finds the address of `--interface lo` with `getifaddrs()`, which reads rtnetlink. Without `AF_NETLINK` that call fails and libwebsockets binds to every address (0.0.0.0) instead of failing (4.3.3, `lws_interface_to_sa`; seen in T-TERM-01). The firewall still dropped 8413 from the network, but the bind must be loopback on its own. Netlink lets the shell read links and addresses (`ip addr`); changing them needs `CAP_NET_ADMIN`, which the unit does not have. T-TERM-01 checks that ttyd listens on 127.0.0.1:8413 only.
- **Only ttyd's port can be bound.** `SocketBindAllow=tcp:8413` and `SocketBindDeny=any`, so the shell cannot listen on a port it picks. systemd never filters port 0, so a random free port is still possible; the firewall drops it from the network. T-TERM-01 checks that binds to tcp and udp 8499 fail, which also proves that this systemd has the BPF support the setting needs. It also checks that ttyd's only listener is 127.0.0.1:8413.
- **The command log is not part of it.** It is a record kept in good faith, and it is easy to get around (T4).

## ttyd comes from upstream, not Debian

Debian trixie has no `ttyd` package, so the image takes upstream's static 1.7.7 release, pinned by SHA-256 in `image/terminal/ttyd.lock`. The version must be bumped deliberately, one reviewed change at a time.

**What the binary bundles.** It is static musl, 1.7.7-40e79c7. It carries its own libwebsockets, libuv and mbedTLS 2.28.5, and that mbedTLS release line is end of life. None of these get Debian security updates.

This is accepted, for three reasons:

- ttyd's TLS is never used (no `--ssl`);
- it serves loopback only, and the firewall also drops 8413 from the network;
- it runs sandboxed, for one password-checked session.

**At each ALP-OS release,** check upstream ttyd for a newer release or security fixes, and bump `ttyd.lock` if needed (ADR 0021).

## Known and accepted (review of 2026-11-04)

1. **ttyd's WebSocket path check is a prefix match.** ttyd 1.7.7 (`protocol.c`) compares the request path only up to the request's own length, so a WebSocket to `/` may pass it.
   - The random base path then stops browsers only, through `--check-origin`. It does not stop a local program that fakes the Origin header.
   - `--once` and `--max-clients 1` end the window as soon as the kiosk connects.
   - A program that wins the race already runs as another local user, and it gets only the sandboxed alp-term shell.
   - T-TERM-01 reports what the image does (`note:` line).
2. **The base path is in ttyd's arguments.** Other local users can read it in `/proc/<pid>/cmdline`. The impact is the same as in item 1. There is no system-wide `hidepid`.
3. **The console idle lock ends a session.** Keys typed in the terminal frame never reach the console page, so the 5-minute idle lock ends the session even while someone types.
   - This is on purpose: the frame never keeps the console awake (focus is not input).
   - If it proves too short, the fix is a longer idle limit, capped by the server, only while a session runs. It is never a keep-alive.
4. **Abstract unix sockets** are not hidden by the filesystem sandbox. T-TERM-01 fails on any abstract listener outside an empty allowlist (review R1). An X server (Xwayland) would be one: the image has no Xwayland, and cage must not start one.
5. **DNS.** If the resolver is on loopback (for example 127.0.0.53), names do not resolve in the session, because the firewall rule refuses that connection too. T-TERM-01 prints the resolver. Use addresses, or `ping` a numeric address, for diagnostics.
