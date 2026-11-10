<!-- Generated from docs/spec.md (Version 0.5) by tools/split-spec.py. Do not edit; edit docs/spec.md and rerun the tool. -->

### Block 9 — Network and firewall

**Goal.** Minimal attack surface on the LAN, nothing exposed to the internet.

**Decisions.** `systemd-networkd` (DHCP by default), `avahi-daemon` for mDNS, `systemd-timesyncd` for time, `nftables` with default drop on input. Allow loopback, established traffic, required ICMP/ICMPv6, mDNS from the local link, HTTPS (443) and SMB (445, redirected) from private address ranges only. No SSH by default. **Advanced mode** is an explicit opt-in in the UI that enables SSH with key-only authentication, shows a persistent warning, and is recorded in the ledger. No UPnP, no NAT-PMP, no automatic port forwarding.

**Acceptance.** A port scan from outside the LAN range finds nothing. From the LAN only 443, 445 and mDNS answer. With advanced mode off, port 22 is closed.

**Out of scope.** Built-in VPN, relay client, IPv6 transition tooling.
