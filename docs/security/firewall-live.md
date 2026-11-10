<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Firewall for ALP-OS Live alpha

File: `image/extra/etc/nftables.conf` (owner: security). Checked with
`unshare -rn nft -c -f image/extra/etc/nftables.conf` (nft 1.1.6 on the dev
VM; the image has nftables 1.1.3 and uses no newer syntax).

## Rules (input policy drop)

1. `lo` accepted.
2. Packets from the network to `127.0.0.0/8` or `::1` dropped (spoofs).
3. TCP 2283, 5432, 6379, 8410, 8411, 8412, 8413 dropped from the network by an
   explicit rule, not only by their 127.0.0.1 bind address. The console API
   (8411) is never reachable from the LAN even if a service binds wrongly.
4. Established/related accepted, invalid dropped.
5. ICMP errors, echo, IPv6 neighbour discovery, router adverts, MLD queries
   from the link (needed for IPv6 multicast/mDNS), DHCPv6 from the link.
6. TCP 443 (nginx) only from RFC 1918 (`10/8`, `172.16/12`, `192.168/16`),
   `fc00::/7` and `fe80::/10`.
7. mDNS UDP 5353 (224.0.0.251 / ff02::fb, from private / link-local
   sources): two lines, commented out. The lead uncomments them in the same
   change that adds avahi-daemon.

Forward: drop. Output: accept (alpha).

## Notes

- Phones reach the server by its IPv4 address (shown on the screen). A home
  network with only public IPv6 addresses (no ULA) will not reach 443 over
  IPv6; this is intended.
- `flush ruleset` removes every other table. Rootless Podman (pasta) does not
  use nftables, so nothing is lost; if rootful netavark is ever used, change
  to deleting only `table inet alp`.
- 169.254.0.0/16 (cable straight to a laptop, no DHCP) is not allowed; add it
  only if field tests need it.

## Tests (QA, T-IMMICH-03 / T-SEC-FW-01)

- From the host: 443 answers; 8411, 2283, 5432, 6379, 22 get no answer.
- From a public source address (second tap / netns with e.g. 203.0.113.5):
  443 gets no answer.
- In the guest: `nft list chain inet alp input` shows `policy drop` and the
  local-only rule; its counter rises when the host probes 8411.

## Output: the Terminal's user (2026-11-04)

One output rule: `oifname "lo" meta skuid 58413 ct state new reject`. The
Terminal's shell runs as `alp-term` (uid 58413) and may open no new
connection to this computer itself: loopback and the computer's own network
addresses (both leave through `lo`), IPv4 and IPv6 (`inet` table). The
console API trusts browser headers that any local program can fake, so this
rule is what keeps the shell away from it, Immich, Postgres and Redis
(security review T1, `docs/plans/live-terminal.md`). Answers on the
connection the kiosk opened to ttyd are not new, so the session works.
`alp-terminal.service` refuses to start when the rule is not loaded
(`/usr/lib/alp/alp-term-check`). T-TERM-01 proves it.
