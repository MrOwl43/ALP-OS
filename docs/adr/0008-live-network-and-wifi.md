# 0008 — Live network: wired, Wi-Fi and non-free firmware

- **Status:** accepted (owner decision 2026-10-18: "Wi-Fi is part of the
  compatibility requirements"; the owner's laptop has Wi-Fi only)
- **Date:** 2026-10-18
- **Spec reference:** block 9; ADR 0007

## Decisions

1. **Wired:** systemd-networkd, DHCP on every physical wired port.
2. **Wi-Fi:** iwd 3.8 joins the network; networkd does DHCP
   (`EnableNetworkConfiguration` stays at its default, off). The user picks
   the network and types the password on the Live text screen
   (`alpctl live screen`, press Enter). The passphrase goes only into
   iwd's state file under `/var/lib/iwd` (mode 0600, tmpfs on Live: gone
   after a restart), escaped per iwd.network(5); it is never on a command
   line or in a log. Enterprise (802.1X) networks are refused with a plain
   message.
3. **Firmware:** Debian's `non-free-firmware` section is enabled for
   `firmware-iwlwifi`, `-realtek`, `-mediatek`, `-atheros`, `-brcm80211`
   (all 20250410-2 in the pinned snapshot) plus `wireless-regdb`. This is
   the same choice Debian's own installer images make since Debian 12.
   The kernel loads compressed firmware (`CONFIG_FW_LOADER_COMPRESS_XZ/ZSTD=y`).
   Non-free firmware is not project code and stays outside the AGPL; it is
   listed in the SBOM and `docs/components.md` with its license.
4. **Firewall** (nftables, input policy drop) is unchanged: Wi-Fi adds no
   open ports.

## Consequences

- Image size grows by the firmware (budget 2 GB, ADR 0005; T-BUDGET-01).
- VM tests cannot join a real Wi-Fi network; T-LIVE-01 checks iwd runs and
  the firmware is present, and the owner tests joining on real hardware.
