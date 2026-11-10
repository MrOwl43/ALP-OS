# Plan: Immich on ALP-OS Live (phone photo backup, beta level)

Status: proposal, 2026-10-18. Spec block 8 and 9, ADR 0005 items 1/5/7, ADR 0007/0008.
Marks: **VERIFIED** (checked today, source given) or **VERIFY** (open, to check before or during the work).
Related plan: docs/plans/live-archive.md (the encrypted container file this plan only mounts onto).

## 1. Scope (owner decisions, fixed)

- Immich ships INSIDE the signed read-only system image (ADR 0005 #5); system image limit 2 GB (#1).
- Immich is used as is, never rewritten. Rootless Podman, Quadlet units, pinned image digests.
- Machine learning OFF (the ML container is not shipped at all in this phase). Deleting on the phone never deletes on the server.
- Beta pairing: the official Immich app + server address + an account that `alpd`/`alpctl` creates (temporary; the QR chain replaces it later).
- LAN only. No management from a laptop browser (ADR 0005 #7): the web UI of Immich is not exposed in the beta except what the phone app needs.
- Library mount point needed from the archive plan: `/var/lib/alp/archive` (an already-unlocked, mounted, writable filesystem). Immich never starts if it is not a mount point.

## 2. What Immich needs (current stable: v3.3.3)

- **VERIFIED** latest stable release is v3.3.3, published 2026-10-14 (https://github.com/immich-app/immich/releases/tag/v3.3.3).
- **VERIFIED** upstream compose uses 4 services: server, machine-learning (skipped), valkey 9 as "redis", Postgres 14 with VectorChord 0.4.3 + pgvecto.rs 0.2.0
  (https://github.com/immich-app/immich/releases/download/v3.3.3/docker-compose.yml and docker-compose.rootless.yml: same images, `user: 1000:1000`, `no-new-privileges`, `cap_drop NET_RAW`).
- **VERIFIED** the DB image must be upstream's own (extension is bundled); Postgres runs with `--data-checksums`, `shm_size 128mb`.
- **VERIFIED** Immich requirements page: minimum 6 GB RAM, 8 GB recommended; "for 4 GB systems it can run with machine learning disabled"; Postgres needs at least 2 GB of RAM if limits are used (docs/docs/install/requirements.md @ v3.3.3).
  Note: spec hardware.md says 4 GB minimum; this is consistent only with ML off. Recommend 8 GB in user text (spec already says so).

### Images, sizes, digests (linux/amd64; read from the registries' manifests today, uncompressed = sum of unpacked layer tars)

| Image | Index digest (pin) | amd64 manifest | Compressed | Uncompressed |
|---|---|---|---|---|
| ghcr.io/immich-app/immich-server:v3.3.3 | sha256:bae7664ff3aa4c2c1fcbdcdd86b40b105643c5f861f3e6a770fc760089f25862 | sha256:8c94984724f79503eb431dfd34ea58b57111c186df8030a66f39ffacf51d7c81 | 583.1 MiB | 1651.2 MiB |
| ghcr.io/immich-app/postgres:14-vectorchord0.4.3-pgvectors0.2.0 | sha256:bcf63357191b76a916ae5eb93464d65c07511da41e3bf7a8416db519b40b1c23 | sha256:b1d33572a9a0634aa6d344077467ad32847812a37fa3859f702952cba22a2f55 | 195.8 MiB | 728.5 MiB |
| docker.io/valkey/valkey:9 (9.1.2) | sha256:c123e3715db63d06d4ad6964884037aa0d5d4d703939b9929954112889708e1d | sha256:67878c70fc179bb928b1a9b510860aff1c475ff245e91d98d4b2133326bf8e3e | 45.5 MiB | 119.9 MiB |
| **Total** | | | **824.4 MiB** | **2499.6 MiB** |

All **VERIFIED** (GHCR/Docker Hub registry API, 2026-10-18; Postgres and valkey digests also match the upstream compose file). The server image is not digest-pinned upstream (tag only); we pin the index digest above.
Not shipped: immich-machine-learning (ML off; also keeps InsightFace models out, spec block 8 rule 3).

### Podman stack (Debian trixie, snapshot 20261016T023245Z)

- **VERIFIED** (https://snapshot.debian.org/archive/debian/20261016T023245Z/dists/trixie/main/binary-amd64/Packages.xz): podman 5.4.2+ds1-2+b2 (85.6 MiB installed), crun 1.21-1, conmon 2.1.12-4, netavark 1.14.0-2, aardvark-dns 1.14.0-3, passt 0.0~git20250503.587980c-2+deb13u1, uidmap 1:4.17.4-2, catatonit 0.2.1-2+b14, nginx 1.26.3-3+deb13u7 (1.5 MiB).
- Whole stack is about 130 MiB installed (estimate, incl. gpgme/gnupg dependencies pulled by podman). **VERIFY** from the mkosi manifest.
- **VERIFY** Quadlet (podman-system-generator) is inside the Debian `podman` package and reads `/etc/containers/systemd/users/` for rootless units.
- Kernel (Debian 6.12.111-1): **VERIFIED** `CONFIG_OVERLAY_FS=m`, `CONFIG_USER_NS=y`, `CONFIG_CGROUP_PIDS=y`, `CONFIG_EROFS_FS_ZIP_ZSTD=y/LZMA=y`
  (https://sources.debian.org/data/main/l/linux/6.12.111-1/debian/config/config and `kernelarch-x86/config`). **VERIFY** the overlay module is in the Live initrd/modules and loads in rootless use.

## 3. Size verdict (system image limit 1907 MiB in out/budget.json)

- **Done (2026-10-25, ADR 0015):** plan A. The root is zstd EROFS
  (`image/repart.d/10-root.conf`, `Compression=zstd`) and the images are a
  read-only store at `/usr/lib/alp/containers` (`image/immich/import.postinst`).
  The Live limit is now 4 GB (ADR 0009); the size is still to be measured on CI.
  Storage moved since this plan: database and thumbnails in the index on the
  stick (ADR 0011), originals in `/var/lib/alp-pool/files/immich` (ADR 0014).

- **VERIFIED** today's root is EROFS with NO compression (`dump.erofs -S`: "compressed files 0", compress rate 100%): current system 728.1 MiB.
- As is (uncompressed EROFS, images unpacked): 728 + 2500 + ~130 = **about 3.35 GiB. Does NOT fit 2 GB.**
- Fits only if we change how the root is stored. Two ways, both fit:
  - **A (recommended): compress the EROFS root (zstd, level high) and ship the images as a read-only podman image store.**
    Estimate: images ~0.8 to 0.95 GB, existing system shrinks to about 0.3 GB, total ~1.3 to 1.5 GB (margin 400+ MiB). Estimate only; measure in step 1.
    Needs `mkfs.erofs -zzstd,level=...` via repart (**VERIFY** the exact `SYSTEMD_REPART_MKFS_OPTIONS_EROFS` setting in the pinned systemd-repart 259 and erofs-utils 1.9.1) and the read speed on a USB stick (**VERIFY**).
  - **B (fallback): ship the images as OCI archives (layers stay gzip, ~825 MiB, no EROFS change needed) and `podman load` them on first boot / after each update.**
    Total about 728 + 825 + 130 = ~1.68 GB (margin ~230 MiB, tight). Costs 2.5 GB extra on the archive and several minutes after every update; rollback then needs a reload.
- Not a fix: ML container (would add ~1+ GB), anything that rewrites Immich images (forbidden).

## 4. How the images get into the read-only image

1. Build step (CI, before mkosi): for each image run `skopeo copy --preserve-digests docker://<name>@<index digest>`, check the digests against `image/immich/images.lock` (names, digests, versions, one-line justification), cache by digest in `out/pkgcache`.
2. Plan A: import into a podman store at `/usr/lib/alp/containers/` (`overlay` driver, built as root in the build sandbox; **VERIFY** this works inside mkosi, otherwise do it in a rootful CI step and pass the tree via ExtraTrees).
   Runtime `storage.conf` in `/etc/containers/`: `additionalimagestores = ["/usr/lib/alp/containers"]`; writable graphroot and runroot of the `immich` user go to `/var/lib/alp/archive/.alp/immich/containers` (never on tmpfs `/var`, which is RAM).
3. Quadlet units reference a local tag (`localhost/alp/immich-server:v3.3.3` etc.) with `Pull=never`, created at build time; a test checks the tag resolves to the pinned digest (**VERIFY**: index digests may not resolve for an imported image; the lock file and a build-time check carry the pin).
4. Image updates ship new images inside the new system slot, so Immich updates and rolls back with the system (spec block 8). Never auto-bump Postgres major (14); a major bump needs its own migration plan.
5. Components table (docs/components.md): add Immich v3.3.3 (AGPL-3.0), Postgres 14 (PostgreSQL license), VectorChord 0.4.3 and pgvecto.rs 0.2.0 (**VERIFY** licenses: VectorChord is believed AGPL-3.0 or ELv2; pgvecto.rs Apache-2.0), valkey 9.1.2 (BSD-3, **VERIFY**), podman and friends (Apache-2.0 / GPL, from Debian copyright files), and the SBOM of the container contents (includes ffmpeg, GPL: source-offer note).

## 5. Services (all run by one unprivileged `immich` user, rootless, lingering)

- Quadlet files in `/etc/containers/systemd/users/` (in the image): `immich-valkey.container`, `immich-postgres.container`, `immich-server.container`, one pod network `immich.network`. Order: valkey and postgres first (health checks), then server.
- All three: `ConditionPathIsMountPoint=/var/lib/alp/archive`, `Pull=never`, `NoNewPrivileges`, `DropCapability=NET_RAW` (as upstream rootless), read-only root plus tmpfs `/tmp`, only these mounts. Server publishes `127.0.0.1:2283` only. Postgres, valkey: no published ports.
- Server environment: `IMMICH_CONFIG_FILE` pointing to an image-shipped JSON with `machineLearning.enabled=false` (**VERIFIED** key exists, docs/docs/install/config-file.md @ v3.3.3), job concurrency 1 for thumbnails and video, trash on (default 30 days), `DB_URL`, `REDIS_HOSTNAME`, `IMMICH_TRUSTED_PROXIES=127.0.0.1`. The file locks the settings in the admin UI.
- Paths on the archive mount: `.alp/immich/library` (UPLOAD_LOCATION, photos, thumbnails, DB dumps), `.alp/immich/postgres`, `.alp/immich/redis`, `.alp/immich/containers`. Owner uid of `immich`. **VERIFY** Postgres works on the archive filesystem (btrfs: mark the postgres dir NOCOW; checksums come from `--data-checksums`).
- Database password and the Immich admin password: random at first start, kept only in `/var/lib/alp` (state, 0600) and never logged.
- cgroup limits (see section 8): Postgres 1 GiB, server 1.5 GiB, valkey 128 MiB, `CPUWeight`/`IOWeight` low so the local screen and updates stay responsive.

## 6. Database storage and backup

- **VERIFIED** Immich writes automatic DB dumps to `UPLOAD_LOCATION/backups`, daily 02:00, keeps 14 (docs/docs/administration/backup-and-restore.md @ v3.3.3). They are on the same archive as the library, so they are protected by the same encryption and snapshots (block 4) later.
- Add: a dump job right before a system update reboot (**VERIFY** the job API: `POST /api/queues/...` or CLI `immich-admin`), so a slot rollback always has a fresh dump.
- Beta recovery test: delete the Postgres dir, start from the last dump (Immich maintenance restore, **VERIFY** it works headless) and see all assets again.
- Metadata-only dump is not a photo backup; photos live only in the library. Off-disk copy of the archive is a later feature.

## 7. Network, firewall, TLS

- Port change: nftables input gets `tcp dport 443 ip saddr { 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16 } accept` (+ IPv6 `fe80::/10, fc00::/7` for the link/ULA) in `image/extra/etc/nftables.conf`; everything else stays dropped (spec block 9). 2283, 5432, 6379 never open; mDNS (udp 5353, link-local) later when avahi is added.
- A host service (nginx, not in a container) is the only listener on 443: `nginx` (Debian 1.26.3, **VERIFIED** in snapshot) with `ssl_protocols TLSv1.3`, proxy to `127.0.0.1:2283`, large body and long timeouts for videos, runs as its own user with `CAP_NET_BIND_SERVICE` (rootless containers cannot bind 443). This is a beta stand-in for the `alpd` gateway (spec block 8); replaced when `alpd` exists.
- Delete guard at the proxy (second line, because the settings alone cannot guarantee it): return 403 for `DELETE /api/assets`, `DELETE /api/asset-files/*`, `POST /api/trash/*`, `DELETE /api/albums/*`, `DELETE /api/admin/*`. **VERIFIED** these routes exist in the v3.3.3 OpenAPI file (open-api/immich-openapi-specs.json). The phone app can still upload and view. **VERIFY** which Immich app delete actions only touch the phone ("delete from device") and that the app shows a sane message when the server answers 403.
- TLS for the beta: self-signed certificate made on first start (key generated by the image tooling, stored 0600 on the archive's `.alp`, never in the repo; the real key owner is `alp-crypto` later, ADR 0005 #6). Fingerprint is shown on the server screen. **VERIFY** the Android and iOS Immich apps can be told to accept a self-signed certificate (believed: advanced setting "allow self-signed SSL certificate").
- Address shown on the screen: `https://<ip>` always, `https://alparchive.local` once `avahi-daemon` is in the image (**VERIFY**: mDNS on Android apps is unreliable, so the IP is the primary).
- Account: on first start `alpctl` calls `POST /api/auth/admin-sign-up` on loopback (**VERIFIED** route) and creates one user for the phone (`POST /api/admin/users`, **VERIFIED**); the Live screen has a "Phone login" panel that resets and shows a fresh password on demand. The password is never logged or stored in clear.

## 8. Memory on a 4 GB machine (all VERIFY by measurement; budget.json idle today is 256 MiB)

- Expected steady state (idle, after first import): base 256 + postgres 300 to 500 + server 400 to 700 + valkey 20 + nginx 10 + podman helpers 50 = about 1.1 to 1.6 GB. Under the 2 GB idle budget (spec Section 3) only if the numbers hold: measure.
- Peaks: thumbnails and video transcoding while uploading (ffmpeg) add 0.5 to 1 GB. The cgroup caps above protect the OS; swap-free machine, so caps must kill/restart a service, not the system (`OOMPolicy=continue`, restart on failure, upload resumes: the app retries).
- On top of RAM the EROFS and archive page cache is reclaimable. `/var` is tmpfs on Live: container graphroot must not sit there (section 5).
- Extend T-BUDGET-01 to record memory with Immich idle and during the upload test; fail if idle > 1907 MiB.

## 9. Tests (VM scenario, QEMU/OVMF, 4 GB guest, uses the archive test fixture from live-archive.md)

- **T-BUDGET-01** (exists): system image <= 1907 MiB with Immich included; memory idle.
- **T-IMMICH-01**: boot; wait `GET /api/server/ping` over 443 (**VERIFIED** route); sign up admin and a user through the script; upload 50 deterministic photos plus 2 short videos (fixtures, sha256 list) with `POST /api/assets` multipart; download originals (`GET /api/assets/{id}/original`) and compare sha256; also compare the files in the library on disk; reboot (clean), repeat reads; then power-cut variant during an upload and check the upload can be repeated without duplicates (`POST /api/assets/bulk-upload-check`, **VERIFIED** route).
- **T-IMMICH-02** (safety): `DELETE /api/assets` through 443 gets 403 and the asset remains; `POST /api/trash/empty` blocked; ML: no ML container, `system-config` shows `machineLearning.enabled=false`.
- **T-IMMICH-03** (network): from the host, only 443 answers on the guest; 2283/5432/6379 closed; a connection from a public-range source address (second tap/netns) gets no answer; no outgoing pulls (run with guest egress blocked: all three containers still start, `Pull=never`).
- **T-IMMICH-04** (supply chain): every Quadlet image maps to a digest in `image/immich/images.lock`; build fails if a digest or version differs; archive not mounted -> Immich services do not start and nothing is written to tmpfs.
- **T-IMMICH-05** (containment): container tries to write outside its mounts and to the host: fails (spec block 8 acceptance).
- Recovery drill (manual, once): delete the DB, restore from dump.

## 10. Steps (one at a time, each ends with a passing test run)

1. Spike (no commit of images): zstd EROFS size and boot time with the three images in an additional store; decide A or B by the measured total (target <= 1.6 GB). Report the numbers.
2. images.lock + fetch/verify script + image/mkosi.conf.d/h-immich.conf (podman, nginx, units, storage.conf).
3. Quadlet units + provisioning in `alpctl` + "Phone login" panel.
4. nginx + nftables + self-signed TLS.
5. Tests T-IMMICH-01..05, budget numbers, ADR 0009 (how Immich is shipped, delete guard, beta TLS), components.md, PROGRESS.md.

## Owner questions (plain English)

1. **Warning on the phone for the beta.** The phone app will probably show that the server's security certificate is not from a known company, because the server makes its own. Options: (a) accept this for the beta and show a short "tap Allow" instruction on the server screen; (b) use plain unencrypted HTTP inside the home for the beta (simpler, but breaks the "always encrypted, HTTPS only" promise). Recommendation: (a). If the phone app refuses it, I will come back before doing (b).
2. **Memory.** Immich officially wants 6 GB; with the photo-recognition features off it works on 4 GB but may be slow while it first processes a big upload. Recommendation: keep 4 GB as the minimum for the beta, tell testers 8 GB is better. No answer needed unless you want a higher minimum.
