<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# 0015 — Live: Immich inside the signed image

- **Status:** accepted (owner goal for Live alpha 0.2, task T03)
- **Date:** 2026-10-25
- **Spec reference:** blocks 8, 9; ADR 0005 #1/#5/#7, ADR 0009, ADR 0011,
  ADR 0014; plan `docs/plans/live-immich.md`

## Decision

1. **Immich v3.3.3 with machine learning off**, three upstream images
   pinned by digest in `image/immich/images.lock` (server, Immich's
   Postgres 14 + VectorChord, valkey 9). The ML image is not shipped.
2. **Images inside the signed root, read-only, loaded on first start**
   (plan B of the Immich plan, 2026-10-25): `just _stage` downloads them
   with `image/immich/fetch.sh` (every blob digest-checked, cached in
   `out/immich-images`) and hard-links them into `image/immich-images`;
   `image/immich/import.postinst` checks the digests again and copies the
   compressed blobs (`dir:` format) to `/usr/lib/alp/immich-images`.
   On start, the user unit `immich-images.service` runs
   `/usr/lib/alp/alp-immich-load` as `immich`: its rootless podman unpacks
   any image whose pinned ID (config digest) is missing into its own store
   in the index (about 2.5 GB, minutes on a stick; again only after an
   update changes an image). Plan A (a pre-unpacked store in /usr) was
   dropped: mkosi 26 builds without root and maps one ID, so layers with
   other owners (e.g. `/etc/gshadow` root:42) cannot be unpacked at build
   time (build failed: "insufficient UIDs or GIDs"). Nothing is ever pulled
   from the network (`Pull=never`); Immich updates and rolls back with the
   system. Old images are not removed in the alpha. `ALP_IMMICH_IMPORT=0`
   builds without images (Immich cannot start then).
3. **Size:** the EROFS root is zstd-compressed (`image/repart.d/10-root.conf`
   `Compression=zstd`, done earlier, ADR 0009). Estimate from the plan:
   images ~0.82 GB compressed (already gzip) plus the system; T-BUDGET-01 enforces the Live limit of
   ADR 0009 (4 GB).
4. **Rootless Quadlet pod under `immich` (uid/gid 50000, subuid
   50001–54096)**: `/etc/containers/systemd/users/50000/`. Container root
   is host uid 50000 (no `User=`), the uid the archive's exFAT/NTFS disk is
   mounted with (ADR 0014). Only `127.0.0.1:2283` is published.
5. **Start and stop with the index and the archive:** `alp-immich.service`
   (not at boot; alpd starts it; `PartOf=` both, mount-point conditions)
   runs `alpctl live immich prepare`, then `loginctl enable-linger immich`
   and starts `user@50000.service`; stopping disables lingering again.
   **No lingering at boot**, because logind would start the user manager
   before the index (the user's home) is open: the linger file lives in
   `/var/lib/systemd/linger` on the tmpfs `/var` and only while the unit
   runs. Lingering is needed at all because since systemd 256 the user
   manager's own PAM session (class `manager`) does not keep the user
   alive, so logind would stop a manager started without a session
   (QA review 2026-10-25, pam_systemd(8)). Prepare: index
   folder `immich` (owned 50000) bound at `/var/lib/immich` (home, podman
   store, Postgres, valkey, thumbnails); originals `upload`, `library`,
   `backups` in `/var/lib/alp-pool/files/immich` bound at
   `/run/alp/immich/originals`; a random database password kept in the
   index and written to `/run/alp/immich/db.env` (0600, uid 50000).
   Database dumps go to the archive (`backups`), next to the originals, so
   losing the stick does not lose them.
6. **nginx in front, TLS with a device CA in the index:**
   `/usr/lib/alp/alp-live-tls` makes a small CA (10 years,
   name-constrained to `alparchive.local` and private IPv4 ranges) and a
   server certificate (825 days) naming the address the phone uses,
   renewed by itself when the address changes; keys stay in
   `/var/lib/alp-index/alp/tls` (encrypted index; moved from the state
   partition of `docs/security/nginx-live.md`, owner: "stored in the
   index"). nginx therefore runs only with Immich. Port 80 serves only a
   static certificate page and `alparchive.crt` (the console shows its QR);
   nftables opens 80 and 443 for private source ranges only.
7. **Family account with a short password** (owner): generated (word +
   4 digits) or chosen by the admin (6–64 characters), shown on the console,
   kept in the index. The admin account is used through an API key only.

## Evidence

- Versions and digests: `docs/plans/live-immich.md` section 2 (registry
  APIs, 2026-10-18); Debian package versions there (snapshot
  20261016T023245Z).
- `Compression=zstd` in repart and `CONFIG_EROFS_FS_ZIP_ZSTD=y`: plan
  section 2 and ADR 0009.
- Unit tests: `crates/alp-live/src/immich.rs` (password rules, mountinfo
  parsing, db.env, fingerprints, storage v2 paths).

## Not verified (VERIFY on CI / real hardware)

- The postinst import inside the mkosi sandbox (podman pull with overlay
  as root, `mkosi-chroot`, 6 GiB free), the measured image size, and
  `Environment=ALP_IMMICH_IMPORT` pass-through.
- Rootless podman using the read-only additional store in place (shifted
  owners, read-only locks), Quadlet reading `users/50000`, `Notify=healthy`,
  pasta's loopback publish and the source address Immich sees (trusted
  proxy, login rate limit), cgroup limits under `user@50000.service`
  started without a session; file capabilities lost by the owner shift.
- Postgres and Immich on the index (btrfs, `chattr +C`) and on gocryptfs
  over exFAT/NTFS (`uid=50000`).
- The certificate script (OpenSSL 3.5 commands, name constraints on
  Android and iOS), installing the CA on Android and iPhone, and whether
  the Immich app trusts a user CA or needs "allow self-signed".
- Memory on a 4 GB machine; T-IMMICH-01..05 on CI.
- `user@50000.service` staying up under runtime lingering (systemd 257
  logind) and stopping cleanly through `disable-linger` + `systemctl stop`.
- Opaque directories in the imported layers: the store is written by a
  root podman (opaque markers as `trusted.overlay.opaque` xattrs, or
  whatever the mkosi sandbox's user namespace allows), while rootless
  native overlay only honours `user.overlay.*` (`userxattr`); a layer that
  replaces a directory could show the lower layer's files. Also whether
  the image's root filesystem (erofs) keeps those xattrs and the 0:0
  whiteout devices. Check with `getfattr -d -m - ` on the store and a
  file listing inside the running containers.
- The FUSE scope ordering (`Before=alp-immich.service
  alp-live-archive.service`, `vault::mount_args`) at shutdown: Immich
  stops, the folder is unmounted, then the daemon's scope.

## Consequences

- The system image is much bigger; every Immich update is a system update.
- The first start of a new archive takes minutes (database set-up).
- Phones trust the device CA once; a new address needs no new install.
