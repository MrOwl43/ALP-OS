<!-- Generated from docs/spec.md (Version 0.5) by tools/split-spec.py. Do not edit; edit docs/spec.md and rerun the tool. -->

### Block 10 — Health, monitoring and power

**Goal.** The user always knows whether everything is fine, in plain words.

**Decisions.** `alpd` aggregates: SMART status, scrub results, pool health, free space, slot state, last update result, ledger verification, time sync, power events. The UI maps them into the status legend (UX addendum 0.2.5, T54; ADR 0023): All fine (blue check), Note (white i, blue frame), Action needed (yellow i), Respond now (red i), Critical (red i with a yellow frame, pulsing: stop using the archive until fixed). Each level has its own shape and label, a one-sentence explanation and a recommended action. Notifications go to paired devices (through the app when connected; no external push service in v1). `journald` is persistent with a hard size cap. Hardware watchdog is enabled if present. `GenericPcPower` detects a UPS through NUT when available and triggers a clean shutdown on low battery. Unclean power loss is handled by Btrfs and by automatic recovery on next boot, and the UI recommends a UPS once, calmly.

**Error codes.** Every condition that reaches the user maps to a code from `errors/catalog.toml` (Section 2.2). Health states in the UI show the plain message, the recommended action and the code. CI checks that every code used in the source exists in the catalog, that no code is defined twice, and that every catalog entry has a title, a user action and technician detail.

**Acceptance (VM).** Injected disk error yields exactly one clear alert. Simulated power loss during upload leaves the pool consistent and the upload resumable. Alerts clear automatically when the condition resolves.

**Out of scope.** Email or cloud alerting, fleet monitoring.
