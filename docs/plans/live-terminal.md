<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Live Terminal (stage F): plan for the security review

Status: **plan only, no code yet**. Revision 2 adds T1 to T6 from the security review
(`live-ui-stage-f-review-2026-11-04.md`). The Security and Network Expert approves it
before anything is built.

## What it is for

A text console for support and for advanced users: to read logs, see disks and network,
and run diagnostic commands, without a second keyboard login on tty2. It is not a way to
change the system: the image stays immutable and the archive stays out of reach.

## Rules

1. **A separate, unprivileged user.**
   - The shell runs as a new system user, `alp-term`, with no password and no sudo.
   - It is not in the `disk`, `adm`, `systemd-journal` or `alp` groups.
   - It cannot read `/run/alp`, the index, the archive mount, the stick's state partition or Immich's data.
2. **No local services (T1, the main protection).**
   - The console API on 8411 trusts only browser headers, which any local program can fake. So the shell must never reach a local service.
   - **Network rule.** An nftables rule in the image refuses every new loopback connection made by alp-term: `oifname "lo" meta skuid "alp-term" ct state new reject`. Answers on the ttyd connection that the kiosk opened still work, because they are not new connections.
   - **Unit rule.** The unit also has `IPAddressDeny=127.0.0.0/8 ::1/128` for the shell. VERIFY that this works with ttyd's own listener. If it does not, put the shell in its own network namespace with only outside access.
   - **Blocked services:** alpd 8411, the installer 8410, the gallery 8412, Immich 2283, Postgres 5432 and Redis 6379. ttyd's own port is open only to the kiosk's connection.
   - **Unix sockets.** AF_UNIX is used only for `/dev/log` (logger). `/run/alp`, `/run/podman` and `/run/user` are inaccessible to the shell.
3. **Only in Advanced mode, only with the password (T5).**
   - The Terminal button sits in Settings > Advanced.
   - `terminal/start` checks the ALP password on the server every time. It uses the same failed-try counter and lockout as sign-in, and it refuses while the console is locked. The Advanced gate in the page alone is never enough (review U4).
   - Start and stop are recorded with the tag `alp-terminal`.
4. **A command log, kept in good faith (T4).**
   - Each command line that bash runs goes to the journal with the tag `alp-terminal`.
   - The log is easy to get around (`exec sh`, a program's own shell), and command arguments can hold secrets (`curl -u name:password`). **It is a record, not a security control.** The security rests on rules 1, 2 and 6, never on the log.
   - Screen text and program input are never logged.
   - The terminal shows a line at the top: "Commands you run here are kept in the system log."
5. **Lock.**
   - The session ends when the console locks, when the screen saver starts, when Advanced mode expires, and when the person leaves the Terminal screen.
   - The 5-minute console idle lock ends it too. Typing in the terminal frame does not keep the console awake (no keep-alive from the frame; security verdict 2026-11-04), and the screen says so.
   - Ending stops the whole unit, so its cgroup and every process in it go.
   - There is no `cron` or `at` in the image (the build checks this), and alp-term has no lingering user manager. Nothing survives the session.
6. **Logs only through a fixed helper (T6).**
   - alpd provides one read-only journal helper with fixed arguments and a fixed allowlist of units and tags.
   - It never passes text typed in the shell to `journalctl` (for example `-D`, `--file` or `--output-fields`).
   - It filters out pairing tokens and keys the same way the main log does.

## How (reuse, no new protocol)

- **Terminal server:** `ttyd` (MIT; the Debian package; **VERIFY** the version in the pinned trixie snapshot), run as `alp-term` by a systemd template unit, `alp-terminal@.service`. alpd starts it only after a password-checked request.
  - It listens on `127.0.0.1:8413` only, an origin separate from the console's, like the gallery (review U2).
  - **One-time address, not a password (T2).** Each session gets a random 128-bit base path (`--base-path /<random>`), plus `--once`, `--max-clients 1` and `--check-origin`, so other local origins cannot open its WebSocket. The path is returned once to the console and never logged.
  - **VERIFY** for the pinned version: whether `--writable` is needed for input (recent ttyd is read-only by default), and how `--check-origin` behaves when the page sits in a frame on another origin.
  - The shell is `bash --norc --noprofile` with a fixed, read-only `PROMPT_COMMAND` that sends the last command line to `logger -t alp-terminal`.
- **Sandboxing in the unit (T3):**
  - `User=alp-term`, `NoNewPrivileges=yes`, `ProtectSystem=strict`, `ProtectHome=yes`, `PrivateTmp=yes`, `PrivateDevices=yes`, `PrivateIPC=yes`;
  - `RestrictNamespaces=yes`, `RestrictSUIDSGID=yes`, `LockPersonality=yes`;
  - `ProtectKernelTunables=yes`, `ProtectKernelModules=yes`, `ProtectKernelLogs=yes`, `ProtectControlGroups=yes`;
  - `ProtectProc=invisible`, `ProcSubset=pid`, `KeyringMode=private`, `UMask=0077`;
  - `SystemCallFilter=@system-service`, `SystemCallArchitectures=native`;
  - `CapabilityBoundingSet=` (empty), `AmbientCapabilities=` (empty);
  - `RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6`, so `ping` and `ip` work toward the outside only (rule 2);
  - `InaccessiblePaths=/run/alp /var/lib/alp /var/lib/alp-index /srv /var/lib/containers /run/podman /run/user /run/alp/media`, plus the Import and Files mount points;
  - `IPAddressDeny=` for loopback (rule 2);
  - `MemoryMax=256M`, `TasksMax=64`, `RuntimeMaxSec=3600`.
- **Console side:**
  - a full-screen frame under the system bar, sandboxed with `allow-scripts allow-same-origin` only, with no downloads or popups;
  - the address is checked like the gallery's: `http:`, `127.0.0.1`, an origin other than the console's;
  - copy and paste work only inside the frame.
- **API (the Storage Engineer decides the shapes):**
  - `POST live/terminal/start {password}` returns `{url}`, or `403 password.wrong`, `429` (lockout) or `409 terminal.running`;
  - `POST live/terminal/stop`;
  - state field `terminal: {state: "off" | "running"}`;
  - feature `terminal`.
- **Kiosk:** add `http://127.0.0.1:8413` to the Chromium `URLAllowlist` and the console CSP `frame-src` only when the feature ships.

## Build notes from the approval (round 2, 2026-11-04)

1. **Where the rule goes.** The nftables rule sits in an `inet` table on the `output` hook. Connections to the computer's own LAN address also go out through `lo`, so T-TERM-01 also tries the LAN address on ports 80 and 443.
2. **IPAddressDeny and ttyd.** `IPAddressDeny=` also filters incoming traffic, so it may block the kiosk's connection to ttyd. If it does, use the network-namespace fallback, or rely on the nftables rule alone. The nftables rule is never loosened.
3. **The base path is the only secret.** It is returned only by `terminal/start`, never put in the state or in any log, and dropped when the session ends.

## What the console UI needs from the server (for the Storage Engineer)

The UI side is built behind the feature `terminal` (`ui/live/app.js` `openTerminal`). It needs exactly:

1. **The feature name.** `"terminal"` in the state's `features` list, only when every system rule above is in the image.
2. **Start.** `POST /api/v1/live/terminal/start` with body `{"password": "..."}`:
   - `200 {"url": "http://127.0.0.1:8413/<128-bit random>/"}` when the session starts. The UI frames the address only if it is `http:`, host `127.0.0.1`, has a port, and is on an origin other than the console's; otherwise it calls stop and shows an error.
   - `403 password.wrong`, `429` with `retry_after_seconds` (the sign-in lockout), `409 terminal.running`, `409 terminal.no_password` (the UI offers "Set a password"), and `401 console.locked` while locked (the UI goes to the lock screen). Otherwise the UI shows the server's message.
3. **Stop.** `POST /api/v1/live/terminal/stop`, body `{}`, answering `200 {}` even when no session runs. The UI calls it when the person leaves the screen.
4. **State.** `terminal: {"state": "off" | "running"}`, never containing the address. When it turns `off` while the screen is open (lock, idle or Advanced timeout), the UI drops the frame and says the session ended.
5. **Kiosk and console.**
   - `http://127.0.0.1:8413` is added to the Chromium `URLAllowlist`, and to the console CSP `frame-src` next to 8412.
   - ttyd's `--check-origin` must accept the frame. The frame's own origin is 8413, and its parent is the console on 8411: VERIFY.
6. **Clean up.** The base path is dropped when the session ends, so an old address returns 404.

## Not in this plan

- Root access, a package manager or writable system paths. Changing the system stays an update, never something done by hand.
- A terminal over the network (SSH). It stays local only.

## Tests

- A VM test, T-TERM-01, checks:
  - a session starts only with the right password and counts toward the sign-in lockout;
  - it runs as `alp-term`;
  - `curl http://127.0.0.1:8411/api/v1/live/state` with faked `Host`, `Origin` and `Sec-Fetch-Site` headers fails from the terminal, and so do 8410, 8412, 2283, 5432 and 6379 (T1);
  - `/run/alp`, `/run/podman`, `/var/lib/alp-index` and the archive cannot be read;
  - other users' processes are hidden;
  - a command line is logged;
  - the session ends on lock and on the idle timeout, and no process of alp-term is left afterwards;
  - a second client and a wrong base path are refused.
- The UI smoke test checks that Terminal is hidden without the feature, asks for the password, and refuses a bad frame address.
