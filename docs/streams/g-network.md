<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Stream G — Network

## Status

Minimal block 9 pieces for ALP-OS Live step 1 (done by the stream A
session, owner asked for Live network first):
- `image/mkosi.conf.d/g-network.conf`: systemd-timesyncd, nftables, iproute2.
- `80-alp-wired.network`: DHCP on every physical wired port (built in or USB).
- `/etc/nftables.conf`: input policy drop; loopback, established, required
  ICMP/ICMPv6, DHCPv6 replies from the link. No services open yet.
- T-LIVE-01 checks DHCP address and the drop policy.

**Needs code-reviewer approval before merge (firewall is a sensitive area).**

## Next

mDNS (avahi), HTTPS port when the UI arrives, local CA and TLS; Wi-Fi
firmware is an owner decision (non-free firmware).
