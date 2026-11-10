<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Contract: systemd target and unit names

Fixed by ADR 0004. Streams use exactly these names. Changing one is a
contract change (one PR updating every user).

| Unit | Owner stream | Meaning and rules |
|---|---|---|
| `alp-healthy.target` | A (block 2) | The system is healthy. Requires (block 2): `alpd` answering on its socket, networking up, time sync attempted. **Must never require** the data pool, Immich, first boot or the local console. `boot-complete.target` has a drop-in (`image/units/boot-complete.target.d/50-alp-healthy.conf`) with `Requires=` and `After=alp-healthy.target`, so a failed health check stops `systemd-bless-boot` (checked in T-BOOT-01). **Reboot on failure (stream A, VERIFY + T-BOOT-02):** while a boot is still being assessed (the UKI carries a boot counter), the machine must reboot by itself if `alp-healthy.target` fails or is not reached within 5 minutes, so boot counting can fall back to the previous version; this uses systemd's own settings (e.g. `JobTimeoutSec=`/`JobTimeoutAction=reboot`, `FailureAction=`, `OnFailure=`), never a custom watchdog, and must never reboot a blessed system. |
| `alp-storage.target` | B (block 4) | The data pool is unlocked and mounted (paths from `alp_core::state::paths().pool`). Reached only on success; failure raises `ALP-STO-005` and never blocks boot or `alp-healthy.target`. Immich must not start unless this target is active. Immich runs rootless; how the rootless units are gated on this system target (e.g. system Quadlet units with `User=`, or a system gate service) is stream E's first VERIFY item, decided in an ADR before any Immich code. |
| `alp-firstboot.target` | C/D (block 7) | First-boot setup is running (key generation, pool creation, console PIN, recovery sheet, phone join). Pulled in only while first boot is not complete. Outside `alp-healthy.target`'s dependency tree: a failed first boot never causes a rollback. |
| `alp-console.target` | D (block 7, console) | The local console kiosk session is wanted (a display is connected). Not part of `alp-healthy.target`; supervised separately (spec block 8 "Local console" rule 1). |
| `alpd.service` | C | The daemon. `Type=notify`. API socket path from `alp_core::state::paths()` (`/run/alp/alpd.sock`). |
| `alp-healthy-report.service` | A | Prints `ALP-OS: alp-healthy.target reached` on the console; the VM tests depend on this exact line. |

Rules

- Names use the `alp-` prefix; no stream adds a target outside this table
  without a contract PR.
- Nothing in `alp-healthy.target`'s dependency tree may depend on
  `alp-storage.target`, `alp-console.target` or any Immich unit. A test in
  stream A checks this with `systemctl list-dependencies` in the VM.
- Test-only units (VM probes) use the prefix `alp-test-` and are injected as
  credentials, never shipped in the image.
- `systemd-repart` is not installed in the target image (if it ever is, its
  service is masked): nothing changes the partition table after install
  (checked in T-BOOT-01).
