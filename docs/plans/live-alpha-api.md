<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# ALP-OS Live alpha — console API (`/api/v1/live/...`)

Status: contract for UX and QA, 2026-10-19 (lead developer). Changes are
listed here, newest first, before the code changes.

- 2026-11-08 (0.2.5 field report: phone refused the certificate; additive):
  1. `POST pair/open` refuses with `pair.certificate_not_served` (plain
     message) while nginx does not yet send a certificate naming every
     address in the QR; alpd checks what nginx really sends and restarts it
     when it differs from the file. No QR is made then.
  2. `POST phone/pair`: the same check; when it fails, `ready` is false,
     `immich.reason` says why, and `qr_text`/`qr_svg` are `null` (they are
     `null` too whenever `ready` is false). The certificate page fields are
     unchanged.
  3. New `GET phone/connection` (signed in) → `{addresses, ports: [{name,
     wifi, carrier, address}], index_open, only_paired_phones, nginx_active,
     certificate: {sha256, names}, ca_sha256, served: [{address, answers,
     sha256, names, same_as_file, names_address, ok}], refused_requests:
     [journal lines of nginx.service, plain text only]}`: what a phone meets on each address, for a
     connection check screen. Opens one TLS connection per address.
- 2026-10-30, third change (ADR 0016 level 3; additive): `POST
  archive/create` takes `alp_password` (not with `serious`): the archive is
  also bound to it (`password.conf`), so `POST archive/recover {partuuid,
  alp_password}` reopens it on another stick with the password alone. Every
  correct password typed later re-binds it (a new stick's password too).
  The setup recovery-code step comes only with strong protection.
  `archive.password_reopen`: whether that binding exists (the UI reminds
  to type the password once when it does not). An archive made with strong
  protection is marked (`ALPArchive/strong`) and never gets one; wrong
  passwords on `archive/recover` count with the password back-off.
- 2026-10-30, second change (T40; additive): `POST recovery/check
  {recovery_key, passphrase?}` → `{opens: true|false}`: whether the typed
  words open this archive (a copy of `recovery.conf` in /run is re-wrapped
  and thrown away; nothing changes). Wrong answers count with the password
  back-off (`password.wait`). Errors as `archive/recover`.
- 2026-10-30 (0.2.4, phone pairing, ADR 0017; additive):
  1. Console (signed in, index open): `POST pair/open` → `{uri, qr_svg,
     expires_in}` (5-minute window, new single-use token); `POST pair/close`;
     `POST pair/approve {id}` → `{device}`; `POST pair/reject {id}`;
     `POST pair/revoke {serial}`; `POST pair/strict {on}` (only paired
     phones may use the Immich API).
  2. Phone (through nginx `POST /alp/pair/v1/request|status`, no cookie):
     `pair/v1/request {token, name, model, os, csr}` → `{id, code}` or
     `pair.token` (wrong, used or expired token), `pair.csr`;
     `pair/v1/status {id}` → `{state: pending, code}` | `{state: approved,
     certificate, ca_cert, identity}` | `{state: rejected}` | `unknown`.
  3. State `pairing: {window_seconds, pending: [{id, name, model, os,
     code}], devices: [{serial, name, model, os, added, revoked}], strict}`;
     locked view: `pending` and `devices` empty.
  4. `GET https://<address>/.well-known/alparchive/identity` (nginx):
     `{v, alg: "ed25519", id, ca, endorsement, ca_cert}`.
- 2026-10-28, second change (T35, protection levels, ADR 0016; additive):
  1. `POST password/setup` takes `"protection"`: `"none"` (no password;
     `password` may be `""` or absent), `"console"` (a password for the
     console only), or absent / anything else (the encrypted index, as
     before). `none` and `console` keep everything unencrypted: a plain
     index on the stick and the plain folder `ALPArchive/files` on the
     backup disk.
  2. `lock.protection`: `null` before setup, then `"none"`, `"console"`,
     `"password"` or `"strong"` (serious encryption chosen).
  3. With `none` the console needs no sign-in, and `phone/pair` returns
     the random phone password in `password` with `uses_main_password:
     false`.
  4. `POST archive/create` with a password (strong protection) on an
     unprotected stick answers `archive.serious_plain`.
  5. `POST reset` keeps an unprotected archive's photos on the
     backup disk (only its id file is removed).
- 2026-10-28 (T33, which devices are connected; additive):
  1. `phone.devices`: the family's signed-in devices, most recently seen
     first: `[ { "id": "<Immich session id>", "name": "Pixel 8", "os":
     "Android", "kind": "phone" | "tablet" | "browser", "app_version":
     "3.3.3" | null, "last_seen": "<ISO 8601>" } ]`. The console's own
     gallery (a Linux browser session) is not listed. `last_seen` is
     updated by Immich about once an hour. `phone.paired` and
     `phone.phones` now count these devices (before, every Immich session,
     so the console's gallery login counted as a phone).
  2. `phone.connected_now`: home network addresses with an open connection
     to port 443 right now, e.g. `["192.168.1.50"]` (Immich keeps no
     address per device; an open phone app keeps a connection).
  3. `POST phone/devices/remove` `{ "id": "<session id>" }`: signs that
     device out (it must sign in again with the ALP password). `200 {}`;
     `phone.device_unknown` for an unknown id. Needs the archive open.
  4. Locked view: `connected_now` is `[]`, `devices[].id` and
     `devices[].app_version` are `null`, `devices[].name` is masked.
  5. `system.release` (T31): the image's name and version, e.g. "ALP-OS
     Live alpha 0.2.3", or `null`.
- 2026-10-26 (owner's alpha 0.2 hardware test, tasks T14, T15, T16, T18,
  T21, T23, T25; systems developer). All additive except items 1, 3 and 4.
  1. **Shutdown with steps (T15).** `POST power/off` now answers
     `202 { "job": Job }` (was `{}`). alpd shuts down in order by itself:
     job id `power.off` (or `power.restart` with `{ "restart": true }`),
     steps `immich` 10 ("Stopping the photo service…"), `archive` 40
     ("Closing the archive safely…"), `index` 70 ("Locking the index…"),
     `saving` 85 ("Saving everything to the disks…"), `off` 100 ("Turning
     off now…" / "Restarting now…"). Every step has a time limit (at most
     about 75 s in all, then it goes on regardless); a step that did not
     finish shows `message` "… took too long; going on." and the shutdown
     continues. If the computer is still on 90 s after the power-off
     request to the system, it is forced off. New state block
     `power: { "state": "on" | "stopping", "job": Job | null }`. A second
     call while stopping answers `202` with the same job. While
     `power.state` is `stopping` the UI shows "Shutting down…" with
     `job.message` and must expect requests to fail (alpd stops last):
     keep showing the last message. The job's `can_cancel` is `false`.
     Free route as before (works on the lock screen). Each
     shutdown adds a few lines to `shutdown-log.txt` on the stick's
     ALP-REPORT drive (steps and their seconds; no names or secrets).
  2. **Addresses everywhere (T14).** New `network.addresses`: every home
     network IPv4 address of this computer, wired first, e.g.
     `[ { "address": "192.168.1.23", "url": "https://192.168.1.23",
     "port": "enp3s0", "kind": "wired" } ]`; `[]` when offline.
     `network.address` stays (the first one). Shown also on the lock
     screen (the locked view keeps it).
  3. **`phone/pair` always answers 200 with the address (T14, T23).**
     Body `{}` or `{ "password": "…" }` (the main ALP password). It no
     longer fails with `409 archive.not_open`, `409 network.offline`,
     `503 immich.starting` or `500 immich.failed`: those become a reason
     in the answer, so the address can always be shown and typed.
     ```json
     { "ready": false, "needs_password": false,
       "server_url": "https://192.168.1.23",
       "addresses": ["https://192.168.1.23", "https://10.0.0.5"],
       "email": "family@alparchive.local", "password": null,
       "uses_main_password": true,
       "immich": { "state": "starting",
                   "reason": "The photo service is still starting. Unpacking the photo service for the first time (about 2.5 GB): part 2 of 3…" },
       "qr_text": "https://192.168.1.23", "qr_svg": "<svg…>",
       "certificate_sha256": "AB:CD:…", "certificate_url": "http://192.168.1.23/",
       "certificate_qr_svg": "<svg…>" }
     ```
     `ready`: a phone can sign in now (Immich `ready` and the family
     account uses the main password). `immich.state`: as `immich.state`
     in `GET state`; `immich.reason`: one plain sentence why it is not
     ready (or `null`). `needs_password: true` (with `ready: false`) when
     Immich is ready but the family account does not use the main
     password yet and alpd has not seen it since starting: ask for the
     main password and call `phone/pair` again with it. `server_url`,
     `qr_text`, `qr_svg`, `certificate_url`, `certificate_qr_svg` are
     `null` only when there is no address (`addresses` `[]`, reason says
     to connect a cable or Wi-Fi). `certificate_sha256` is `null` until
     the certificate exists. Errors left: `403 password.wrong`,
     `429 password.wait` (only with `password`), `400 request.json`,
     `409 locked`, `401 console.locked`.
  4. **No family password any more (T23).** Phones sign in to the Immich
     app with `email` (`family@alparchive.local`) and **the main ALP
     password**. `password` in the pair answer is always `null`;
     `uses_main_password` is `true`. alpd sets the family account's
     password to the main password whenever it is typed (`password/setup`,
     `unlock`, `console/signin`, `password/check`, `phone/pair` with
     `password`) and Immich is ready (it waits for Immich up to 45
     minutes, in memory only); a future "change password" does the same.
     Phones signed in with the old family password stay signed in.
     **`POST phone/reset` answers `410 phone.reset_gone`** ("Phones now
     use your ALPArchive password."); `400 phone.password` is gone.
  5. **Battery (T18).** New `system: { "battery": null |
     { "percent": 87, "charging": true } }` (`null` on computers without
     a battery). Read in the background every 30 s since alpha 0.2.2
     (T28; a laptop's battery can answer slowly). `charging`: plugged in
     (charging, full, or held below full by the computer).
  6. **This computer (T25).** New `hardware.specs` (read once at start):
     ```json
     { "cpu": { "model": "Intel(R) Core(TM) i5-12500H", "threads": 16 },
       "memory_bytes": 16777216000,
       "disks": [ { "name": "nvme-Samsung_SSD_980_…", "model": "Samsung SSD 980", "size_bytes": 512110190592,
                    "kind": "nvme", "internal": true, "system": false } ],
       "graphics": [ { "vendor": "Intel", "pci_id": "8086:46a3" } ],
       "network": [ { "name": "wlan0", "kind": "wifi", "driver": "iwlwifi", "usb": false, "working": true } ],
       "tpm": "2.0", "uefi": true, "secure_boot": false,
       "battery": { "model": "L20M4PC1", "health_percent": 91 } }
     ```
     `disks[].name`: the disk's `/dev/disk/by-id` name (long; show
     `model` first). `disks[].kind`: `nvme` | `sata` | `usb` | `virtio` | `other`;
     `system`: the stick ALP-OS runs from. `tpm`: `"2.0"` | `"1.2"` |
     `null`. `secure_boot`: `true` | `false` | `null` (unknown).
     `battery`: `null` without a battery; `health_percent` (full charge
     now against when new) may be `null`. Locked view: `hardware.specs`
     stays (no names in it).
  7. **Gallery in the kiosk (T16).** New `immich.console_url`:
     `"http://127.0.0.1:2283/"` while `immich.state` is `ready`, else
     `null`: the Immich web gallery on this computer only (loopback; the
     kiosk's browser now allows exactly this address besides the
     console). Do not link the kiosk to `https://<address>` (port 443
     serves only the phone app's API). Immich v3.3.3 sends no
     `X-Frame-Options` and no CSP by default (its helmet middleware is
     off unless `IMMICH_HELMET_FILE` is set; we do not set it), and the
     console's CSP now has `frame-src http://127.0.0.1:2283`, so the UI
     shows the gallery in an `<iframe>` below ALPArchive's own top bar
     (home button = the way back), with
     `sandbox="allow-scripts allow-same-origin allow-forms allow-downloads"`
     (no `allow-top-navigation`: Immich cannot replace the console).
     Sign in there with `family@alparchive.local` and the main password.
  8. **Why it takes long (T21).** New `immich.detail`: one plain sentence
     while `immich.state` is `starting` (else `null`), e.g. "Unpacking
     the photo service for the first time (about 2.5 GB): part 2 of 3,
     1.1 GB of 2.5 GB…", "Starting the photo service…", and after 3
     minutes "Still starting: the first start sets up the photo database.
     This can take a few minutes more." New `immich.seconds_starting`
     (seconds since alpd saw it starting, or `null`). Show `detail`
     under the waiting text.

- 2026-10-25 (task T03, Immich in the image, ADR 0015; Immich developer):
  1. **The admin may choose the family password.** `POST live/phone/reset`
     body may be `{ "password": "…" }` (6–64 characters, no space at the
     start or end, no control characters): the family account gets exactly
     this password. Without it (body `{}`) a new short one is made as
     before (a common word and 4 digits, e.g. `river-4821`). New error
     `400 phone.password`. Phones already logged in stay logged in.
     `phone/pair` is unchanged (the same password on every call).
  2. **`certificate_sha256` is the device's own certificate authority**
     (the file the phone installs once from `certificate_url`, i.e.
     `http://<address>/alparchive.crt`, name-constrained to
     `alparchive.local` and private IPv4 ranges). It stays the same when
     the address changes: the HTTPS server certificate under it is renewed
     by itself. The certificate page (port 80) and HTTPS (443) answer only
     while Immich runs (`immich.state` not `off`).
  3. `immich.state` is `off` while the index or the archive is closed;
     `starting` until Immich answers (the first start of a new archive
     sets up the database and can take a few minutes).

- 2026-10-24, third change (owner requirements for ALPHA 0.2, storage v2,
  ADR 0014; storage developer):
  1. **No big archive file any more.** A new archive is an encrypted
     folder (`ALPArchive/data`, gocryptfs) that grows as photos arrive.
     `archive/create` body is `{ "partuuid": "…" }`; `size_bytes` may
     still be sent and is ignored. Candidate fields `default_size_bytes`,
     `min_size_bytes` and `max_size_bytes` stay for old clients (meaning:
     the room the archive can grow into); the UI should drop the size
     chooser and show `free_bytes`. `archive.size_bytes`/`free_bytes` are
     the backup disk's size and free space.
  2. **New job steps** of `archive.create`: `preparing` 5 (incl. repair),
     `checking` 8–40 (health check, then a 256 MiB write/read test with
     `bytes_done`/`bytes_total`/`bytes_per_second`; message e.g.
     "Checking the disk: 120 MB of 256 MB at 18 MB/s…"), `encrypting` 60,
     `formatting` 80, `done` 100. `reserving` is no longer used. Cancel
     works in every step before `done` (it deletes only the test file and
     the unfinished `ALPArchive/data` folder it made itself).
  3. **New failure codes** (`error.code` of a failed job): `ALP-STO-115`
     the disk writes slower than 3 MB/s, `ALP-STO-116` the read/write test
     failed (possible faulty drive), `ALP-STO-117` the disk's own health
     check (SMART) reports a problem, `ALP-STO-118` the automatic repair
     did not work, `ALP-STO-119` preparing (erasing) the disk failed,
     `ALP-STO-120` this disk cannot be prepared. `ALP-STO-110`…`114` keep
     their steps; `ALP-STO-111` (writing) is not used by new archives.
  4. **Automatic repair.** A disk that was not removed safely (exFAT/NTFS
     dirty flag) is no longer refused: the candidate is `usable: true` with
     new fields `repair: true` and `note` ("This disk was not removed
     safely last time. ALPArchive will repair it first."), and the job
     repairs it (message "This disk was not removed safely last time.
     Repairing it first…"). The same happens when the archive is opened at
     start-up. Only a failed repair is an error (`ALP-STO-118`).
     `ALP-STO-108` is no longer returned. Every candidate has `repair`
     (`false` unless so) and `note` (`null` unless so).
  5. **Prepare this disk.** Every candidate has new fields `disk_id`
     (e.g. `sdb-17`: the disk as plugged in this time) and `can_prepare`
     (`true` when the whole disk may be erased and formatted as exFAT: an
     external disk with nothing usable on it, no `ALPArchive` folder, no
     operating system, not write-protected, and every partition FAT32,
     unknown, or unreadable). New `POST live/disks/prepare` (below). While
     it runs, the shared archive job slot holds it (`archive.job.id` =
     `disk.prepare`; `archive.state` stays `none`).
  6. **Recovery code storage (T02 seam).** `recovery/create` binds the 15
     words to new archives (`409 recovery.not_ready` is no longer returned
     for them). Archives made before this change keep their 64-letter key;
     binding words to them fails with the message "This archive was made
     with an earlier test version: its 64-letter recovery key still works.
     Recovery words need a new archive." (T02 maps it to
     `409 recovery.old_archive`). `archive/recover` with 15 words opens new
     archives; with the 64-letter key, old ones.
  7. `candidates[].has_archive` is also true for a new-style `ALPArchive`
     folder; `archive/recover` opens both kinds.

- 2026-10-24, second change (owner decision 2026-10-24 "passwords and
  codes", task T02, ADR 0013; lead). Word counts tell the codes apart.
  1. **One main password.** The admin password opens everything (unchanged
     endpoints). With a TPM chip the index opens by itself, unless the
     archive was made with serious encryption (item 4).
  2. **Recovery code: 15 words** (BIP-39 English, 160 random bits +
     checksum) instead of the 64-letter recovery key.
     `POST recovery/create` body `{}` or `{ "passphrase": "…" }`
     (required, 8–128 characters, when `archive.serious` is true) answers
     `{ "words": [15 words], "recovery_key": "w1 w2 … w15",
     "qr_text": "alparchive-recovery:1:w1-w2-…-w15", "qr_svg": "<svg…>" }`.
     The code is never stored; the passphrase is never shown back.
     **Storage v2 binds it to the archive** (`alp_live::recovery::bind`);
     until that lands, `recovery/create` answers
     `409 recovery.not_ready` and shows no code (a code that would not
     open anything is never shown). New error: `400 recovery.passphrase`
     (missing or not 8–128 characters).
     `POST archive/recover` body `{ "partuuid", "recovery_key",
     "passphrase"? }`: `recovery_key` is the 15 words (any case, spaces or
     dashes) or the old 64-letter key (alpha archives made before this
     change keep working). Errors: `400 recovery.chain_code` (8 words:
     "This is a chain code …"), `400 recovery.word_count`,
     `400 recovery.word_unknown` (+ `index`, 1-based),
     `400 recovery.checksum`, `400 recovery.format` (neither words nor an
     old key), `400 recovery.passphrase`. With 15 valid words the
     `archive.recover` job starts and, until storage v2, fails with
     `recovery.not_ready`.
  3. **Chain code: 8 words** (88 bits: 80 random + 8-bit checksum, ADR
     0013) instead of 24; never an encryption key. `chain/create` and
     `chain/code` return 8 `words`, `qr_text` `alparchive-chain:2:w1-…-w8`.
     A chain made before keeps its 24 words (shown and accepted as
     before). `chain/join` errors: `400 chain.recovery_code` (15 words:
     "This is a recovery code …"), `400 chain.word_count`,
     `400 chain.word_unknown` (+ `index`), `400 chain.checksum`, and
     **`429 chain.wait`** (+ `retry_after_seconds`): after 5 wrong codes
     each try waits 30 s, doubling up to 1 hour.
  4. **Serious encryption** (optional, chosen at archive creation):
     `archive/create` body may add `"serious": true` and then needs
     `"password"` (the admin password; same back-off as sign-in; errors
     `403 password.wrong`, `429 password.wait`). Before the job starts
     the TPM key slot of the index is removed and auto-unlock stays off
     (`lock.tpm` = new value `disabled`: the password is typed at every
     start); the recovery code then also needs the passphrase. New field
     `archive.serious` (`true` | `false`).

- 2026-10-21 (owner field bug: "Creating your locked archive" stopped at
  55 % "Making room…" on an 8 GB exFAT USB stick; lead):
  1. **Job progress with bytes and speed.** The Job object has new fields
     (all may be `null`): `bytes_done`, `bytes_total` (space reserved so
     far, really on the disk, not only in memory), `bytes_per_second`,
     `seconds_left`, `seconds_running`, `seconds_since_progress` and
     `can_cancel`. They are computed when `GET state` is answered, so they
     change every second. While reserving, `message` reads e.g. "Reserving
     space: 3.2 GB of 7.4 GB at 4.1 MB/s, about 18 minutes left." and is
     updated about every second. After 60 s without progress `message`
     becomes "Still working: the disk has not answered for 2 minutes. Keep
     it plugged in, or press Cancel." The UI should show `message` during
     `reserving` (done minimally in ui/live/app.js by the lead).
  2. **Cancel.** New `POST archive/cancel` (below). New job states
     `cancelling` (still counts as running: `archive.state` stays
     `creating`) and `cancelled` (`archive.state` back to `none`, no error).
  3. **Every failed step has a code.** A failed `archive.create` job has
     `error.key = archive.failed` and `error.code`: `ALP-STO-110` preparing
     the disk, `ALP-STO-111` writing failed, `ALP-STO-112` the disk stopped
     answering (a step ran past its time limit), `ALP-STO-113` locking
     (encryption), `ALP-STO-114` setting up the archive; a refusal found
     while preparing keeps its own code (`ALP-STO-100`…`109`). Every step
     has a time limit; nothing waits forever.
  4. **Smaller default size.** `default_size_bytes` = the smaller of
     `max_size_bytes` and 16 GB on exFAT and NTFS, 64 GB on ext4. Any size
     up to `max_size_bytes` can still be chosen. An unfinished
     `archive.alp.partial` from an earlier try counts as free space (it is
     reused or cut down).
  5. **`archive/candidates` returns `409 archive.busy`** while a job runs
     (looking mounts the disks).
  6. **Field logs.** Job updates go to the journal with the prefix
     `ALP-JOB` (every step change, at least every 15 s, and the end with
     code). New `alpctl live diag` saves `diag-<machine>-<time>.txt` on the
     stick's ALP-REPORT drive: last job status, disks, loop devices, memory,
     alpd's job/archive lines and the kernel's disk lines. alpd saves it by
     itself when a creation job fails.
  7. The UI client gives up on a request after 20 s (ui/live/api.js), so a
     hanging request can never stop the state polling.
  8. (2026-10-24, UX API gaps 1-3) New `POST password/check` and
     `POST reset` (below); `archive/cancel` answers `409
     archive.not_creating` when nothing is being created (as the mock).
     The image build runs `ui/stage-docs.sh`, so `docs/spec.md` is served
     as `ui/live/docs/spec.md` for the kiosk's specification page.
- 2026-10-20, second change (owner: Live is a portable backup station for
  most plug-and-play media, exFAT explicitly; lead):
  1. **Only the backup disk itself is checked.** The computer's own Windows
     being asleep, using Fast Startup or BitLocker no longer refuses an
     external disk: `ALP-STO-106` is not returned any more (the catalog
     entry stays). New codes for the disk's own state: `ALP-STO-108` the
     disk was not removed safely (exFAT VolumeDirty flag, NTFS dirty
     flag), `ALP-STO-109` Windows on this disk is asleep or used Fast
     Startup (its own `hiberfil.sys`). The same check runs before
     `archive/create`, `archive/recover` and opening at start-up.
  2. **exFAT archives are written in full.** Linux 6.12's exFAT cannot
     reserve space at once (no fallocate), so the archive file is filled
     with zeros (never sparse). This takes time: about the archive size
     divided by the disk's write speed (e.g. 500 GB at 150 MB/s ≈ 1 hour).
     The `archive.create` job stays in step `reserving` meanwhile; its
     `percent` moves from 20 to 58 and `message` shows the percent of the
     reservation ("Reserving 500 GB for the archive… 42%"). The UI should
     say that this can take a while and the disk must stay plugged in. An
     interrupted reservation continues where it stopped on the next try.
- 2026-10-20 (owner field test: no USB disk was accepted; lead):
  1. **Small margin instead of 20 GB.** The archive leaves only a safety
     margin free on the external disk: 1 GiB or 5 % of the free space,
     whichever is smaller. Smallest archive 1 GiB (about 1.1 GB free
     needed). `default_size_bytes` = `max_size_bytes` = most of the free
     space (free minus margin, whole MiB); the user can choose less.
  2. **Disks without partitions.** A filesystem written straight onto the
     device (e.g. a stick formatted as exFAT without a partition table) is
     a candidate too: `name` = `disk` (e.g. `sdb`), `partuuid` =
     `disk-<filesystem UUID>` (use it like a PARTUUID in `archive/create`).
     A disk with nothing on it is listed as `disk-<name>`, not usable.
  3. **Every external disk is listed with its reason.** New field
     `code` on each candidate: the catalog code (`errors/catalog.toml`)
     when `usable` is false, else `null`. `reason` is one plain sentence
     with the action, ready to show (no "ALPArchive cannot use this place:"
     prefix any more). Codes: `ALP-STO-100` not enough space (the numbers
     are in the sentence), `ALP-STO-101` FAT32, `ALP-STO-102` operating
     system (or its system partition), `ALP-STO-103` empty, unknown or
     unsupported format, `ALP-STO-104` write-protected, `ALP-STO-105`
     BitLocker, `ALP-STO-106` (no longer returned, see the second change),
     `ALP-STO-107` the disk could not be read. MBR extended-partition
     containers are not listed.
  4. Error bodies now carry `code` where one exists: `422 archive.refused`
     from `archive/create` has the same `code` and sentence as the
     candidate.
  5. Preparing (erasing) a disk: later, not in the alpha (owner).
- 2026-10-19, third change (owner decisions items 2, 6, 7, 8; lead):
  1. **Console sign-in (lock screen).** New `console` block in state
     (`state`: `open` | `locked`, `idle_lock_seconds`). New endpoints
     `POST live/console/signin`, `POST live/console/lock`,
     `POST live/console/activity`. A successful sign-in, `unlock` or
     `password/setup` answers with a session cookie (`alp_console`,
     HttpOnly, SameSite=Strict); the browser sends it by itself. While the
     console is locked every call except `state`, `console/signin`,
     `unlock`, `password/setup` (first start only) and `power/off` answers
     `401 console.locked`, and `GET state` hides names (see "Locked view").
     During first start (`lock.state = setup`) the console is open.
  2. **TPM auto-unlock.** `lock.tpm`: `none` (no chip) | `on` (the chip
     opens the index at start) | `off` (chip present, not used yet or it
     refused; the password opens the index and turns it on again).
  3. **Recovery key** (setup "Recovery page"): `POST live/recovery/create`
     makes a new recovery key for the archive and shows it once (key +
     QR). `archive.recovery`: `none` | `set`. `POST live/archive/recover`
     opens an archive from another (re-written) stick with its recovery key.
  4. **Delete with password:** `POST live/gallery/delete` and
     `POST live/gallery/trash/empty` ask for the password again.
  5. `phone/pair` also returns `certificate_url` (plain-HTTP page on the
     home network that installs the server's certificate on the phone) and
     `certificate_qr_svg`.
  6. Text fixed: candidates are external disks and USB sticks only.
- 2026-10-19, second change (owner storage decisions, ADR 0011):
  1. New `lock` block in state and two endpoints: `POST live/password/setup`
     (first start: choose the password) and `POST live/unlock` (every
     start). Until `lock.state` is `open`, archive, phone and chain calls
     answer `409 locked`.
  2. `archive/candidates` lists only external disks and USB sticks (never
     the computer's own disks, never a disk with an operating system on
     it). An empty list means "plug in the disk or USB stick you want to
     use"; the UI polls it every few seconds.
  3. Space: `archive.space` and a new `index` block (`size_bytes`,
     `free_bytes`, `space`: `ok` | `low` | `full`); `immich.state` can be
     `paused` (space full, uploads stopped).
  4. `hardware.checks[].code`: catalog code (e.g. `ALP-HW-103`) or `null`.
- 2026-10-19 (UX requests, additive only):
  1. `hardware.checks[].id`: stable id (`cpu`, `memory`, `uefi`,
     `secure_boot`, `tpm`, `storage`, `graphics`, `wired_network`; other
     checks: the item in snake_case).
  2. `phone.phones` (phones logged in) and `phone.last_upload` (always
     `null` in the alpha: Immich has no cheap admin call for it).
  3. `POST live/phone/pair` is now idempotent: the same login every call.
     New `POST live/phone/reset` makes a new password.
  4. New `POST live/archive/forget` for `archive.state = missing`.
  5. (Superseded by the second change: external disks and USB sticks only.)

Served by `alpd` on `http://127.0.0.1:8411` only (console listener,
`docs/contracts/api-access.md`). Same listener serves the UI: `GET /` answers
`302 Location: /live/`, and `GET /<path>` serves files from
`/usr/share/alp/ui/<path>` (source `ui/<path>`, so `ui/live/index.html` is
`/live/`). No directory listings; `index.html` for paths ending in `/`.

## Rules for every request

- JSON in and out, UTF-8. Every response has `Cache-Control: no-store`.
- `Host` must be `127.0.0.1:8411` or `localhost:8411`, else `421`.
- **POST** needs `Origin: http://127.0.0.1:8411` and
  `Content-Type: application/json` (a JSON body, `{}` when there is nothing
  to send), else `403 request.forbidden`. Body at most 64 KiB.
- UI pages get `Content-Security-Policy: default-src 'self'; img-src 'self'
  data:; style-src 'self' 'unsafe-inline'; object-src 'none'; base-uri 'none';
  frame-ancestors 'none'; frame-src http://127.0.0.1:2283` (the last part
  since 2026-10-26: the Immich gallery in an iframe). So: no inline `<script>`, nothing from the internet.
- Console lock: see "Console sign-in" below (third change; ADR 0009 #5 is
  superseded here by owner decision 8).
- Long jobs (Wi-Fi connect, archive create): the POST answers `202` at once
  with the job; progress is read by polling `GET /api/v1/live/state` (once a
  second is fine). One job per area at a time (`409 <area>.busy`).

### Error body (every 4xx/5xx)

```json
{ "error": { "key": "wifi.password_refused",
             "message": "The network did not accept this password. Please try again.",
             "code": null } }
```

`key` is stable (UX may map it to its own wording). `message` is a plain
sentence ready to show. `code`: the catalog code (`errors/catalog.toml`)
where one exists (so far `archive.refused`: `ALP-STO-100`…`107`), else `null`.

### Job object

```json
{ "id": "archive.create", "state": "running", "step": "reserving",
  "message": "Reserving space: 3.2 GB of 7.4 GB at 4.1 MB/s, about 18 minutes left.",
  "percent": 36, "bytes_done": 3200000000, "bytes_total": 7400000000,
  "bytes_per_second": 4100000, "seconds_left": 1025, "seconds_running": 800,
  "seconds_since_progress": 0, "can_cancel": true, "error": null }
```

`state`: `running` | `cancelling` | `done` | `failed` | `cancelled`.
`percent`: 0–100 or `null` when unknown. The byte and time fields are
`null` when they do not apply (see the 2026-10-21 change). `error`: an error object (as above, without the outer `error` key)
when `failed`. A finished job stays in the state until the next job of that
area starts.

## Endpoints

### `GET /api/v1/live/state` → 200

```json
{
  "api": 1,
  "hardware": {
    "machine": "ASUSTeK COMPUTER INC. VivoBook K3402ZA",
    "verdict": "ok",
    "checks": [ { "id": "memory", "item": "Memory", "status": "ok", "value": "15.6 GB", "advice": "" } ],
    "report_file": "hw-ASUSTeK-...-20261019-0941.txt",
    "specs": { "cpu": { "model": "…", "threads": 8 }, "memory_bytes": 16777216000, "…": "see 2026-10-26 item 6" }
  },
  "network": {
    "online": true,
    "address": "192.168.1.23",
    "ports": [ { "name": "wlan0", "kind": "wifi", "state": "connected",
                 "address": "192.168.1.23", "ssid": "Home" } ],
    "wifi_present": true,
    "addresses": [ { "address": "192.168.1.23", "url": "https://192.168.1.23",
                     "port": "wlan0", "kind": "wifi" } ],
    "job": null
  },
  "lock": { "state": "open", "retry_after_seconds": 0, "tpm": "on" },
  "console": { "state": "open", "idle_lock_seconds": 300 },
  "index": { "size_bytes": 28000000000, "free_bytes": 27000000000, "space": "ok" },
  "archive": {
    "state": "open",
    "space": "ok",
    "place": { "partuuid": "…", "name": "sdb1", "label": "My Passport" },
    "size_bytes": 107374182400,
    "free_bytes": 106300000000,
    "job": null,
    "error": null,
    "recovery": "set",
    "serious": false
  },
  "immich": { "state": "ready", "error": null, "detail": null, "seconds_starting": null,
              "console_url": "http://127.0.0.1:2283/" },
  "power": { "state": "on", "job": null },
  "system": { "battery": { "percent": 87, "charging": true } },
  "phone": { "server_url": "https://192.168.1.23", "account": "family@alparchive.local",
             "paired": true, "phones": 1, "photos": 1284, "last_upload": null },
  "chain": { "state": "member", "chain_id": "3f9c2a1be04d7781", "role": "created",
             "since": "2026-10-19T09:41:00Z" }
}
```

- `hardware.verdict` / `checks[].status`: `ok` | `note` | `fail`.
  `report_file`: name on the stick's `ALP-REPORT` drive, or `null` (not saved).
  Since alpha 0.2.2 (T28) the check runs in the background after alpd
  starts: until it is done `hardware` is `{"machine": "", "verdict":
  "note", "checks": [], "report_file": null, "specs": null, "pending":
  true}` (usually for well under a second); `pending` is absent after.
- `network.online`: some port has a home-network address; `address` is the
  one shown to the user (phones use it). `ports[].kind`: `wired` | `wifi`;
  `state`: `connected` | `waiting` (cable in / Wi-Fi joined, no address yet) |
  `off`. `ssid` only for Wi-Fi. `job`: the Wi-Fi connect job or `null`.
- `lock` is about the encrypted index on the stick: `state`: `setup`
  (first start: no password chosen yet) | `locked` (the index is closed:
  no TPM chip, or the chip did not open it; the unlock screen asks for the
  password) | `open`. `retry_after_seconds` > 0 after several wrong
  passwords (shared by unlock, sign-in and delete). `tpm`: `none` | `on` |
  `off` | `disabled` (see change list; `disabled`: serious encryption,
  the password is typed at every start).
- `console` is about the screen: `state`: `open` | `locked` (lock screen:
  sign in with the admin password). `idle_lock_seconds`: the screen locks
  by itself after this many seconds without `console/activity` or another
  call that needs the console open (`GET state` does not count).
- `archive.recovery`: `none` (no recovery code made yet) | `set`; `none`
  unless `open`. `archive.serious`: `true` when the archive was made with
  serious encryption (recovery needs the code and the passphrase).
- `index`: the encrypted index on the stick (database, thumbnails, keys);
  zeros until open. `space` (also `archive.space`): `ok`, `low` (below
  10 %), `full` (reserve reached: uploads are paused).
- `archive.state`: `locked` (password not typed yet) | `none` (not set up with this stick) | `creating` (see
  `job`) | `open` | `missing` (set up, but the disk or file is not found; see
  `error`) | `failed` (see `error`). `place`, sizes: `null`/0 unless `open`.
- `immich.state`: `off` (archive not open) | `starting` | `ready` |
  `paused` (no room left; see `error`) | `failed`.
- `phone`: `server_url` and `account` are `null` until the first pairing;
  `paired` = a phone is logged in to the family account; `phones` = how
  many; `photos` = photos and videos kept (0 if unknown); `last_upload`:
  RFC 3339 time or `null` (always `null` in the alpha).
- `hardware.checks[].id` is stable; `item`, `value`, `advice` are English
  text for display.
- `chain.state`: `none` | `member`; `role`: `created` | `joined`;
  `chain_id`: 16 hex characters, safe to show (it does not reveal the words).

### `POST /api/v1/live/password/setup` → 200 `{}`

First start only. Body `{ "password": "…" }` (8–128 characters, no
control characters; the UI asks twice and compares). Makes room on the
stick, locks the index with this password, opens it, and sets the same
password for the maintenance login (tty2). Takes up to about 30 seconds.
Answers with the console session cookie (the console stays open).
Errors: `400 password.too_short`, `400 password.invalid`,
`409 index.exists` (already chosen), `409 state.not_ready` (the stick's
storage is missing), `500 index.failed`.

### `POST /api/v1/live/unlock` → 200 `{}`

Body `{ "password": "…" }`. Opens the index, then the archive (if set up),
then Immich starts. Also signs in the console (cookie), so the password is
typed once. With a TPM chip it turns auto-unlock on again (`lock.tpm` =
`on`) for the next start. When the index is already open (the chip opened
it) the password is checked and the console signed in, nothing else. Errors: `403 password.wrong`, `429 password.wait`
(too many tries; `retry_after_seconds` in the error object),
`404 index.missing` (the stick's index was not found), `500 index.failed`.
After 5 wrong passwords each try waits 30 s, doubling up to 1 hour.

### `GET /api/v1/live/wifi/networks` → 200

Scans; takes up to about 6 seconds.

```json
{ "networks": [ { "ssid": "Home", "security": "psk", "signal": 4, "connected": false } ] }
```

Strongest first. `security`: `open` | `psk` | `8021x` | `wep`. `signal` 0–4.
Errors: `409 wifi.no_device`.

### `POST /api/v1/live/wifi/connect` → 202 `{ "job": Job }`

Body `{ "ssid": "Home", "password": "…" }` (`password` omitted or `""` for
open networks). The password is stored on the stick for the next start
(owner decision) and never logged. Job steps: `joining`, `address`; `done`
when the port has an address (up to about 60 s).
Errors: `400 wifi.password_length` (8–63 characters), `400 wifi.enterprise`
(work or school login), `400 wifi.wep`, `404 wifi.not_found`,
`409 wifi.no_device`, `409 wifi.busy`. Failed job keys:
`wifi.password_refused`, `wifi.no_address`, `wifi.failed`.

### `GET /api/v1/live/archive/candidates` → 200

Looks at external disks and USB sticks plugged in (read-only; takes a
few seconds). The computer's own disks and the ALP-OS stick are never
listed. Every partition of every external disk is listed (a disk without
partitions as one entry), usable or not; one that is not usable has
`usable: false`, a `reason` and a `code`.

```json
{ "candidates": [ {
    "partuuid": "…", "name": "sdb1", "label": "My Passport", "fs": "ntfs",
    "disk": "sdb", "disk_model": "WD My Passport", "size_bytes": 999000000000,
    "free_bytes": 812000000000, "usable": true, "reason": null, "code": null,
    "recommended": true, "default_size_bytes": 810925686784,
    "min_size_bytes": 1073741824, "max_size_bytes": 810925686784,
    "has_archive": false },
  { "partuuid": "…", "name": "sdc1", "label": "STICK", "fs": "vfat",
    "disk": "sdc", "disk_model": "SanDisk Ultra", "size_bytes": 32000000000,
    "free_bytes": 0, "usable": false, "code": "ALP-STO-101",
    "reason": "FAT32 can't hold the archive. Use a disk formatted as exFAT, NTFS or ext4, or another disk.",
    "recommended": false, "default_size_bytes": 0, "min_size_bytes": 1073741824,
    "max_size_bytes": 0, "has_archive": false } ] }
```

Usable ones first, the recommended one (most free space) first of all.
`reason`: plain sentence with the action when `usable` is false; `code`:
its catalog code (list in the 2026-10-20 change). Sizes: the archive
keeps a margin of 1 GiB or 5 % of the free space (the smaller) free;
`default_size_bytes` = the smaller of `max_size_bytes` and 16 GB (exFAT, NTFS) or 64 GB (ext4). `free_bytes`, `default_size_bytes`
and `max_size_bytes` are 0 when `usable` is false.
`has_archive`: an `ALPArchive` folder is there (`usable` is false): it can
be opened with its recovery key (`archive/recover`).

### `POST /api/v1/live/archive/create` → 202 `{ "job": Job }`

Body `{ "partuuid": "…", "size_bytes": 107374182400 }`. Job steps and
percent: `preparing` 5, `reserving` 20, `encrypting` 60, `formatting` 80,
`done` 100. Then Immich starts by itself (`immich.state`).
Errors: `409 locked`, `404 archive.not_found`, `400 archive.size` (outside min–max),
`409 archive.exists` (this stick already has an archive),
`409 archive.busy`, `422 archive.refused` (message says why and `code`
is the catalog code, as in `archive/candidates`). Failed job keys:
`archive.failed`, with `code` `ALP-STO-110`…`114` by step (2026-10-21).

### `GET /api/v1/live/disks` → 200 `{ "disks": [Disk] }` (feature `disks`, T58)

Every disk, including the ALPArchive stick and the computer's own disks, classed by ADR 0022. Behind the console sign-in gate.

```json
{"id": "sdb-12", "name": "sdb", "model": "SanDisk Extreme", "size_bytes": 1000204886016,
 "kind": "stick" | "external" | "internal", "role": "system" | "backup" | "other",
 "locked": true, "lock": "stick" | "internal_live" | "backup" | "write_protected" | "open" | null,
 "safe_to_remove": false, "health": "healthy" | "wear" | "failing" | null, "temperature_c": 38,
 "parts": [{"partuuid": "...", "name": "sdb1", "label": "...", "fs": "exfat", "size_bytes": 0}]}
```

- `id` is the disk id the other disk routes take (`<name>-<diskseq>`).
- `model` and `label` are made safe for the screen, but are still text from the device: show them as text only.
- No serial numbers or WWNs are shown.
- `health` and `temperature_c` come from SMART (read at most once a minute per disk; a disk asleep is not woken). Either is null when unknown.
- **In Live, every internal disk is `locked`, with `lock: "internal_live"`.** Its partitions are listed only so the person can recognise it. Nothing on it is mounted or read.
- **Every disk route refuses the stick and internal disks with `403 disk.locked` (ALP-STO-124),** whatever id the screen sends: `disks/prepare`, `disks/format/estimate`, `disks/format` and `archive/create`. Files and Import refuse them too: they list only external disks.

### `GET /api/v1/live/events` → 200 `{ "events": [Event] }` (feature `events`, T58)

What alpd did to disks, oldest first: the last 200 events since alpd started. Behind the console sign-in gate.

`Event = {"at": 1793900000, "kind": "disk.format", "disk": "SanDisk Extreme", "size_bytes": 0, "mode": "quick" | "secure" | null, "fs": "exfat" | "ext4" | null, "result": "started" | "done" | "failed" | "stopped", "code": "ALP-STO-122" | null}`

- **A fixed form.** No passwords, tokens, file names or labels.
- **The list lives in alpd's memory.** A local process cannot add to it: a journal line tagged like ours never reaches it (review R5).
- **The journal also gets each event**, as an `ALP-EVENT …` line from alpd.service.

### `POST /api/v1/live/disks/eject` → 200 `{}` (feature `disk_eject`, T58)

Body `{"disk_id": "sdb-12"}`.
- **What it does:** syncs and switches off an external disk that is not open (its SCSI device is deleted), so it can be pulled out. It then drops out of `live/disks`.
- **No password:** nothing on the disk is changed.
- **Errors:**
  - `403 disk.locked` (the stick or an internal disk);
  - `404 disk.not_found`;
  - `409 disk.open` (close it in Files first);
  - `409 archive.busy`;
  - `500 disk.eject_failed`.
- **Event:** `disk.eject done`.

### `POST /api/v1/live/disks/check` → 202 `{ "job": Job }` (feature `disk_check`, T58)

Body `{"disk_id": "sdb-12"}`.
- **What it does:** runs the job `disk.check`, a read-only check of every partition of an external disk that is not open: `fsck.exfat -n`, `e2fsck -n -f` or `ntfsfix -n`. Other kinds are reported as "cannot be checked here".
- **The sandbox (review R8).** Each check runs in a `systemd-run` sandbox with:
  - no network;
  - only that partition, read-only (`DevicePolicy=closed`, `DeviceAllow=<dev> r`);
  - no capabilities;
  - memory, task and time limits.
- **Output.** The fsck output is never logged or shown. The done job's `message` gives the result in words per partition, e.g. "Photos (EXFAT): no problems found." A problem is reported as "problems found. Nothing was changed; …".
- **Stopping:** it can be stopped with `archive/cancel`.
- **Errors:**
  - `403 disk.locked`;
  - `409 archive.busy`;
  - a failed job has code `ALP-STO-125`.
- **Events:** `disk.check started`, then `done`, `failed` or `stopped`.

### `POST /api/v1/live/disks/prepare` → 202 `{ "job": Job }`

(2026-10-24, third change.) "Prepare this disk": erases a whole external
disk and formats it as exFAT (label `ALPArchive`, one partition; MBR, GPT
above 2 TiB). Body `{ "disk_id": "sdb-17", "partuuids": ["…", …],
"password": "…" }`: `disk_id` and the `partuuid` of every candidate of that
disk exactly as the UI listed them (the warning screen shows them all:
label, format, size). Only when that disk's candidates have
`can_prepare: true`. Needs a signed-in console and the admin password
(same back-off). The UI shows a warning that lists what will be erased and
uses slide-to-confirm before calling it. Job id `disk.prepare`, steps
`checking` 5, `erasing` 30, `formatting` 60, `done` 100 (about a minute;
cannot be cancelled). Afterwards the disk is listed again as a usable
exFAT candidate (`archive/candidates`).
Errors: `403 password.wrong`, `429 password.wait`, `409 archive.busy`
(a job runs), `409 archive.exists` (this stick already has an archive),
`404 disk.not_found` (unplugged, or another disk now has this name),
`409 disk.changed` (the partitions differ from the list: look again),
`422 disk.refused` (`code` `ALP-STO-120`, message says why). Failed job
key: `disk.failed`, code `ALP-STO-119`.

### `POST /api/v1/live/disks/format/estimate` → 200

(2026-11-03, T49, Advanced > Disks.) What the "Quick format / Secure
format" choice shows, for one external disk as plugged in now. Body
`{ "disk_id": "sdb-17", "partuuids": ["…", …] }` (as for `disks/prepare`).
Needs a signed-in console; no password (nothing is written: the speed comes
from a 64 MiB read of the disk, halved, so the minutes are "about").
Answer:
`{ "size_bytes": 32000000000,
   "quick":  { "text": "…", "seconds": 60 },
   "secure": { "text": "…", "seconds": 1680, "about": true, "bytes_per_second": 19000000 } }`.
Show both `text`s as they are (they must not promise more: quick format
leaves old files recoverable; secure format makes them very hard to
recover, not impossible on USB sticks and SSDs). Show "about N min" from
`secure.seconds`; while the job runs, `seconds_left` uses the measured speed.
`warnings` (list, may be empty) names what is on the disk; the last
screen before the slide must say each one: `bitlocker` (BitLocker data,
lost for good), `windows` (Windows or another system's partitions, also a
Windows dynamic disk: the disk may start a computer), `alparchive_folder`
(an ALPArchive folder, e.g. another stick's archive). Formatting is still
allowed (external disk, review G3).
Errors: `404 disk.not_found`, `409 disk.changed`, `409 archive.busy`,
`422 disk.refused` (`code` `ALP-STO-123`), `403 disk.locked` (`ALP-STO-124`).

**Added 2026-11-08 (T58):**
- **`confirm`:** a one-time confirmation (32 hex characters, review R4). `disks/format` must send it back.
  - It is bound to what this estimate saw: the disk id with its diskseq, the size, the PARTUUIDs, and the file system of a disk without partitions.
  - It is good for 5 minutes and works once; any attempt uses it up, right or wrong.
  - A newer estimate replaces it. Locking the console or a new sign-in drops it.
  - It protects against races (the disk changed between the screens), not against access: the password does that.
- **`bytes_used`:** bytes in use on the partitions that could be looked at read-only, or null.
- **`files`:** null for now.
- **New warning `system`:** a Linux root, /usr or swap partition, or another installed operating system. The disk may start a computer.

### `POST /api/v1/live/disks/format` → 202 `{ "job": Job }`

(2026-11-03, T49; owner: "offer quick format / secure format, explain the
difference, secure format takes about x min".) Formats a whole external
disk that is **not the archive's disk**: a new partition table and one
exFAT partition (MBR, GPT above 2 TiB). Body
`{ "disk_id": "sdb-17", "partuuids": ["…", …], "mode": "quick" | "secure",
"label": "Photos", "password": "…" }`. `label` is optional (default `USB`;
1–11 of A–Z a–z 0–9 space `-` `_`). Needs a signed-in console and the admin
password (same back-off). The UI shows the warning listing what will be
erased, the mode's text and time, and slide-to-confirm with the disk name
on the last screen. The server checks again, here and in the job: external
(USB) only; never the stick, the computer's own disks or the archive's disk
(by `disk_id` and PARTUUID); not write-protected; nothing of it mounted
(Import or Files must eject it first); partitions exactly as listed.
Job id `disk.format`. Quick: `checking` 2, `formatting` 88–92, `done` 100
(seconds). Secure: `checking` 2, `erasing` 5–85 (message "Overwriting with
zeros… N% (about M min left)", `bytes_done`, `bytes_total`,
`bytes_per_second`, `seconds_left`), `formatting`, `done`. Only the
`erasing` step can be cancelled (`archive/cancel`; `can_cancel` is true
then): the job ends `cancelled` with "Stopped. The disk is only partly
erased and has no file system now; format it again to use it." A done job's
message names the method, e.g. "Done: the disk is formatted (secure:
discarded, then overwritten with zeros)." If only the first and last
16 MiB could be overwritten, the job **fails** (never "Done") with a message
that says the disk was only partly erased.
Errors: `400 request.json` (mode, label), `403 password.wrong`,
`429 password.wait`, `409 archive.busy`, `404 disk.not_found`,
`409 disk.changed`, `422 disk.refused` (`code` `ALP-STO-123`). Failed job
key: `disk.format_failed`, code `ALP-STO-122`.

**Added 2026-11-08 (T58):**
- **Body:** also `"confirm"` (from the estimate, required) and `"fs": "exfat" | "ext4"` (optional, default exfat).
- **ext4:** a Linux partition type, `mkfs.ext4 -m 0`, and a label of up to 16 characters (exFAT stays at 11).
- **New errors:**
  - `409 format.confirm_stale` (no confirmation, a used or old one, or the disk changed): go back to the estimate.
  - `403 disk.locked` (`ALP-STO-124`): the stick or an internal disk.
- **The job checks again, right before each write, that the disk is the same and still external** (review R2).
- **Events:** every format writes events to `GET live/events`: `started`, then `done`, `failed` (with its code) or `stopped`. While it runs the disk is
writable through its own allowance (`/run/alp/allowed-format`), so an open
archive is not touched; afterwards every partition and the disk are
read-only again.

### Files, read only (stage F, feature `media_files`, 2026-11-04)

External disks for Settings > Advanced > Files (crates/alp-core/src/live_media.rs on
crates/alp-live/src/media.rs). All need a signed-in console. A disk is named only by
its opaque `id` from `state.media`; a path is a JSON list of names the server listed
(each one name: never empty, `.`, `..`, or containing `/`). Every call checks the
disk's role again (only external disks, never the stick, the computer's disks or
the archive's disk; allowed file systems only).

- `state.media` (signed in only; `[]` otherwise): `[{id, label, fs_type, size_bytes,
  mounted, refused}]`; `refused` is the reason as text, or `null`. Refreshed at most
  every 5 s, and at once after open and eject.
- `POST media/open {media}` → 200 `{}`: mounted read-only (`ro,nosuid,nodev,noexec,
  nosymfollow`, the partition read-only in the kernel).
- `POST media/list {media, path, offset}` → 200 `{entries: [{name, dir, size_bytes}],
  truncated, skipped}`; at most 10,000 entries per page; links and special files are
  never listed. Only on an open disk (`409 media.not_open`).
- `GET media/file?media=<id>&path=<JSON list, URL-encoded>`: only on an open disk;
  `GET` only (no `HEAD`); only `.jpg .jpeg .png .gif .webp .bmp .mp4 .m4v .mov .webm`
  and, always as `text/plain; charset=utf-8`, `.txt .log .md .csv .json .xml .ini
  .cfg .conf .nfo` (by name; never SVG or HTML: `422 media.type`). Headers: the type, `Content-Disposition: inline`,
  `Content-Security-Policy: sandbox; default-src 'none'`, `Accept-Ranges: bytes`,
  and (on every console answer) `X-Content-Type-Options: nosniff`, `Cache-Control:
  no-store`. One `Range` (206, at most 4 MiB per answer; 416 when outside the file);
  without `Range` a file is sent whole only up to 32 MiB (`413 media.too_large`).
  Text is a preview only: a text answer carries at most 256 KiB (the UI's `TEXT_MAX`),
  with or without `Range`; a larger text file without `Range` gets `413`.
  `Sec-Fetch-Dest`, when sent, must be `image`, `video` or `empty` (403). Paths are
  never logged.
- `POST media/eject {media}` → 200 `{}`: unmounted, and the id is forgotten (an old
  link gives 404); the next `state.media` gives the disk a new id.

Errors: `400 media.path`, `404 media.not_found` (unplugged, or ejected), `409
media.not_open`, `409 media.busy`, `413 media.too_large`, `422 media.refused`
(message says why), `422 media.type`, `500 media.failed`, `500 media.unreadable`.

### `POST /api/v1/live/archive/cancel` → 202 `{ "job": Job }`

Body `{}`. Stops a running `archive.create` job (or the `erasing` step of a
`disk.format` job, see above): it answers at once with
`state: cancelling`; the job stops at its next check (within about a
second while reserving; a running program is stopped), removes only its
own `ALPArchive/archive.alp.partial` (the folder and its README stay),
makes the disk read-only again and ends `cancelled`. Calling it again while
cancelling is fine. Once the archive file has its final name the job is not
stopped any more and ends `done`.
Errors: `409 archive.not_creating` (nothing is being created),
`409 archive.busy` (an `archive.recover` job runs; it cannot be stopped).

### `POST /api/v1/live/password/check` → 200 `{}`

Body `{ "password": "…" }`. Checks the admin password without signing in
(no cookie, the console session is unchanged); for asking the password
again before a warning screen. Needs a signed-in console. Same back-off as
sign-in (shared counter). Errors: `403 password.wrong`,
`429 password.wait`, `409 index.missing`, `400 request.json`.

### `POST /api/v1/live/reset` → 202 `{ "job": Job }`

(Changed 2026-11-03: a job, because an unprotected stick is now erased for
real. Before, the answer was 200 `{ "archive": "…" }`.)

Body `{ "password": "…" }`. "Delete everything and start over". Needs a
signed-in console and the admin password (same back-off). The console is
signed out at once. Job id `reset` in `archive.job`, steps: `deleting` 0,
`erasing` 10 to 99 (with `done_bytes`, `total_bytes` and a message such as
"Erasing the stick… 40% (about 12 min left)"), then `done`; the job's
`message` then says what happened on the backup disk, and `lock.state` is
`setup`, `archive.state` `none`: first start.

What it does: Immich stops; on the backup disk only ALPArchive's own files
in the `ALPArchive` folder are deleted (the archive, its key files
`password.conf`, `recovery.conf`, `recovery.conf.new`, `README.txt`; the
folder too if it is empty then; any other file stays), and only after the
archive's id showed it is the one made with this stick; the stick forgets
the archive. The index on the stick:

- **Encrypted (levels 3, 4):** `cryptsetup erase` (every key slot: password,
  TPM, recovery), then its header with `wipefs` (logged if that fails; the
  key slots are gone already).
- **Unprotected (levels 1, 2):** the gallery database and thumbnails are
  plain on it, so the partition is erased for real (owner, 2026-11-03):
  `blkdiscard` where supported, then every byte is overwritten with zeros;
  if that fails part-way, at least the first and last 16 MiB are, and the
  log says which. A power cut during it lands at setup, which finishes the
  erase first. On USB flash, wear-levelling can keep old copies in spare
  blocks: "very hard to recover", not "impossible".

Wi-Fi networks remembered on the stick stay. When the backup disk is not
plugged in, cannot be written, or holds another archive, nothing is deleted
on it (index only); the message says so in plain words. If deleting on the
disk fails, the index is not touched. Errors before the job:
`403 password.wrong`, `429 password.wait`, `409 archive.busy` (a job runs).
A failed job: `reset.failed`, code `ALP-STO-121`.

### `POST /api/v1/live/archive/recover` → 202 `{ "job": Job }`

Body `{ "partuuid": "…", "recovery_key": "…" }`. Only when this stick has
no archive yet (`archive.state = none`) and the candidate has
`has_archive: true`. Opens that archive with its recovery key (spaces,
dashes and case do not matter), gives it a new key inside this stick's
index and remembers it; Immich then starts with the photos already there.
The recovery key keeps working. Job id `archive.recover`, steps:
`checking` 10, `opening` 50, `done` 100.
Errors: `409 archive.exists`, `404 archive.not_found`,
`400 recovery.format` (not 64 letters), `409 archive.busy`,
`422 archive.refused`. Failed job keys: `recovery.wrong` ("This recovery
key does not open this archive."), `archive.failed`.

### `POST /api/v1/live/recovery/create` → 200

(Changed 2026-10-24: 15 words, see the change list; the text below is the
old 64-letter key, which old archives keep.)

Body `{}`. Makes a new recovery key for the open archive (an extra key
slot on the archive, made by `systemd-cryptenroll --recovery-key`) and
returns it once. It is never stored on the stick or anywhere else: the
user prints it or takes a photo. A previous recovery key stops working.

```json
{ "recovery_key": "fkbl-…", "qr_text": "fkblrkiu-…", "qr_svg": "<svg …>…</svg>" }
```

`recovery_key`: 64 letters in 8 groups of 8 separated by `-` (systemd's
"modhex" alphabet `cbdefghijklnrtuv`, so no letter can be misread).
Errors: `409 archive.not_open`, `500 recovery.failed`.

### Console sign-in

- `POST /api/v1/live/console/signin` → 200 `{}` + cookie. Body
  `{ "password": "…" }` (the admin password). Errors: `403 password.wrong`,
  `429 password.wait`, `409 index.missing` (no password chosen yet).
- `POST /api/v1/live/console/lock` → 200 `{}`. Body `{}`. Locks now (the
  UI calls it for "Lock" and when its own idle timer ends).
- `POST /api/v1/live/console/activity` → 200 `{ "idle_lock_seconds": 300 }`.
  Body `{}`. The user is active; call it on input, at most every 30 s.
- Locked view of `GET state`: `archive.place.name` and `.label` and
  `network.ports[].ssid` keep their first character, the rest becomes `•`
  (at most 8); `phone.server_url`, `phone.account` and `chain.chain_id` are
  `null`; `hardware.report_file` is `null`. Free space, disk health
  (`archive.state`, `immich.state`), network state and photo counts stay.

### `POST /api/v1/live/gallery/delete` → 200 `{ "deleted": 3 }`

Body `{ "ids": ["<immich asset id>", …], "password": "…" }` (1–1000 ids).
Moves these photos and videos of the family account to Immich's trash
(kept 30 days). Errors: `403 password.wrong`, `429 password.wait`,
`400 request.json`, `409 archive.not_open`, `503 immich.starting`.

### `POST /api/v1/live/gallery/trash/empty` → 200 `{}`

Body `{ "password": "…" }`. Deletes the trash for good. Same errors.
The gallery itself (view, scroll, search in the kiosk) is not in this
change.

### `POST /api/v1/live/archive/forget` → 200 `{}`

Body `{}`. Only when `archive.state` is `missing`: this stick forgets that
archive so a new place can be chosen. The old `ALPArchive` file is never
touched (its record stays on the stick under another name).
Errors: `409 archive.forget` (archive open, or none set up),
`409 archive.busy`.

### `POST /api/v1/live/phone/pair` → 200

(Changed 2026-10-26, items 3 and 4: always 200 with the addresses and a
reason; phones use the main password; the answer below is the old one.)
Body `{}` or `{ "password": "…" }`. Makes sure the family account exists
and returns its login.

### `POST /api/v1/live/phone/reset` → 410

Removed 2026-10-26 (item 4): answers `410 phone.reset_gone`.

```json
{ "server_url": "https://192.168.1.23", "email": "family@alparchive.local",
  "password": "river-4821", "qr_svg": "<svg …>…</svg>",
  "qr_text": "https://192.168.1.23", "certificate_sha256": "AB:CD:…",
  "certificate_url": "http://192.168.1.23/",
  "certificate_qr_svg": "<svg …>…</svg>" }
```

`certificate_url` is a one-page site on the home network (plain HTTP,
port 80) with the server's certificate and short steps for Android and
iPhone; the phone installs it once, then the Immich app connects over
HTTPS. Pairing also starts the family chain behind the scenes when there
is none (`chain.state` becomes `member`).

The official Immich app (v3.3.3) cannot scan a login QR (VERIFIED: its
deep links only open photos/albums), so the QR holds only the address; the
user types email and password. Errors: `409 archive.not_open`,
`409 network.offline`, `503 immich.starting` (try again in a few seconds),
`500 immich.failed`.

### `POST /api/v1/live/chain/create` → 200

(Changed 2026-10-24: 8 words, see the change list.)
Body `{}`. Starts a new family chain. The 24 words (BIP-39, 256 bits)
belong only to the chain (like Brave Sync), not to the archive's key.

```json
{ "chain_id": "3f9c2a1be04d7781", "words": ["word1", "…", "word24"],
  "qr_text": "alparchive-chain:1:word1-word2-…-word24", "qr_svg": "<svg …>…</svg>" }
```

Errors: `409 chain.exists`.

### `GET /api/v1/live/chain/code` → 200

Same body as create: shows the words and QR again ("view the chain code",
as in Brave Sync). Errors: `404 chain.none`.

### `POST /api/v1/live/chain/join` → 200 `{ "chain_id": "…" }`

Body `{ "words": ["…", …] }` (24 words, any case, extra spaces ignored) or
`{ "qr_text": "alparchive-chain:1:…" }`.
Errors: `400 chain.word_count`, `400 chain.word_unknown` (the error object
also has `"index": 7`, 1-based), `400 chain.checksum` (words are real words
but not a valid set: one is probably wrong), `409 chain.exists`.

### `POST /api/v1/live/chain/leave` → 200 `{}`

Body `{}`. Forgets the chain on this stick (the words still work elsewhere).
Errors: `404 chain.none`.

### `POST /api/v1/live/power/off` → 202 `{ "job": Job }`

Body `{}` or `{ "restart": true }`. Stops Immich, closes the archive and
the index safely, saves, then powers off (or restarts). Steps, time
limits and the `power` state block: 2026-10-26 change, item 1.

### Terminal (stage F, feature `terminal`, 2026-11-04)

Plan and security rules: `docs/plans/live-terminal.md` (revision 2, review
T1-T6). Offered only when the image carries `alp-terminal.service` and ttyd.
Behind the console sign-in like every route (`401 console.locked`).

- `POST terminal/start {password}` → 200 `{ "url": "http://127.0.0.1:8413/<32 hex>/" }`.
  The password is checked every time, in the sign-in counter and back-off
  (`403 password.wrong`, `429 password.wait` with `retry_after_seconds`).
  `409 terminal.running` when a session runs; `409 terminal.no_password`
  when no ALP password was chosen (protection level 1, or setup);
  `500 terminal.failed` when the session did not come up. The address is
  given only here: never in the state or any log.
- `POST terminal/stop {}` → 200 `{}`, also when no session runs.
- State: `terminal: {"state": "off" | "running"}`, never the address.
- A session ends when the console locks (`console/lock`, the idle lock or
  a new sign-in), on `terminal/stop`, when its browser leaves (ttyd
  `--once`), after 10 minutes without input at the prompt, after one hour,
  and when alpd restarts. Then nothing listens on 8413: the old address
  answers nothing.

## Other errors

`404 request.not_found`, `405 request.method`, `400 request.json` (body is
not valid JSON or misses a field), `413 request.too_large`,
`401 console.locked` (message: "Sign in first."), `500 internal` (message: "Something went wrong inside ALPArchive. Restarting
the computer usually helps.").

## 0.2.5 changes (2026-11-03)

- **Recovery code in two steps (T42, security review 2026-11-01).**
  `POST recovery/create {password, replace?, passphrase?}` checks the ALP
  password (a wrong one counts in the back-off), answers 409
  `recovery.exists` when a code exists and `replace` is not true, binds the
  new code as `ALPArchive/recovery.conf.pending` and returns `words`,
  `recovery_key`, `qr_text`, `qr_svg`, `check` (three positions 1 to 15)
  and `expires_seconds` (900). `POST recovery/commit {words: {"<pos>":
  "<word>"}}` checks those words and renames the pending file to
  `recovery.conf` (422 `recovery.commit_wrong`; after 5 tries 410
  `recovery.commit_gone`; 410 `recovery.expired`; 409
  `recovery.no_pending`). `POST recovery/cancel` forgets it. Cancel,
  timeout, console lock and an alpd restart keep the old code; a stale
  pending file is deleted when the archive opens. Unencrypted archives
  answer 409 `recovery.not_encrypted` (create and check).
- **Setup finished (T09).** `POST setup/finish` keeps
  `/var/lib/alp/live/setup-done` on the stick's Live state; `state.setup =
  {finished}`. The UI opens the everyday screens after a restart when it is
  set. Reset everything removes it.
- **Features.** `state.features` lists what this alpd can do, so the Live UI
  0.2.4 console shows only those flows. Today: `recovery_commit` (the two-step
  code above; the UI reads `check` as its `confirm` positions). Without
  `setup_progress` the UI calls `setup/finish` when setup ends.

## 0.2.5 feedback changes (2026-11-08, Storage Engineer; T54, T58)

**Alert levels (feature `status_levels`, ADR 0023).**
- **Every error object has `level`:** `note` | `action` | `now` | `critical`. It comes from its catalog code's `severity`, otherwise from its key (answers to what was typed are `note`), otherwise `action`.
- **`state.alerts`** is `[{key, level, message, code}]`: the standing problems.
  - space on the stick and on the backup disk (`index.space_low|full`, `archive.space_low|full`);
  - the archive's `error`;
  - Immich paused (`space.full`);
  - the archive disk's own SMART check: `archive.disk_wear` (action), `archive.disk_failing` (critical, ALP-STO-117).
- **`live/disks`** gives each disk `level` from its health (spec block 10): healthy `null`, wear `action`, failing `now`, or `critical` for the disk that holds the archive.
  - The list is empty when everything is fine.
- **`state.status`** is the most serious alert, or `{"level": "ok", "key": null, "message": null, "code": null}`.
- **While the status is `critical`:**
  - `gallery/delete` and `gallery/trash/empty` answer `409 archive.critical`;
  - new phone uploads get 503 from nginx (`POST /api/assets`, `PUT /api/assets/<id>/original`), and the app tries again later.
  - **Still allowed:** formatting another USB disk, recovery, export, Files and the disk check (review R6).
  - Critical comes only from alpd's own checks, and clears only when a later check passes (R7).

**Disks (features `disks`, `events`, `disk_check`, `disk_eject`, ADR 0022).** See `GET live/disks`, `GET live/events`, `disks/eject`, `disks/check` and the 2026-11-08 additions to `disks/format/estimate` and `disks/format` above.

**Screen settings (T56; features `prefs` and `display_modes`).**
- **`state.prefs`** is `{natural_scrolling: bool, ui_scale: 1|1.25|1.5|1.75|2|null, home_background: "blue"}`. `null` means automatic. The lock screen sees it too (it needs the scale).
- **`POST live/prefs {…}`** takes any subset of those keys and answers `{prefs}`. An unknown key or a value not in the list gets `422 prefs.invalid`, and then nothing changes. Kept on the stick in `/var/lib/alp/live/prefs.json` (written atomically, checked again when read); Start over removes it.
- **`state.display`** is `{width_mm, height_mm, auto_scale}`. The size comes from the screen's EDID (base block only, after a header and checksum check; `null` when unknown). `auto_scale` is the listed step nearest to about 96 effective DPI, or 1. No EDID text (monitor name, serial) is ever sent.
- **`GET live/display/modes`** → `{modes: [{id, width, height, refresh, native, current}]}` for the first screen. `refresh` is in Hz. `id` is the position in this list. `503 display.unavailable` when the screen cannot be asked (no kiosk running).
- **`POST live/display/mode {id}`** → `{revert_in: 15}`. The screen switches. Unless `POST live/display/mode/keep {}` comes within 15 s, it switches back by itself (a black screen needs no answer). One change at a time: `409 display.busy`. An unknown `id`: `404 display.not_found`. The screen refusing: `500 display.failed`.
- **`POST live/display/mode/revert {}`** → `{}`: go back at once (the Revert button). Nothing waiting: `409 display.no_pending`.
- **`POST live/display/mode/keep {}`** → `{}`. The mode is kept, and set again each time the kiosk starts, but only on the same screen (a hash of its EDID identity). Nothing waiting: `409 display.no_pending`.
- `display_modes` is listed in `features` only when the image has `wlr-randr`.
- **Lock after (feature `idle_lock_pref`).** `state.prefs.idle_lock_seconds` is 60, 300 (default), 900 or 1800, set through `POST live/prefs`. There is no "never" (Security, 2026-11-08); anything else gets `422 prefs.invalid`. alpd's own idle lock and `console.idle_lock_seconds` follow it.
