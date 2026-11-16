<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# 0025: The gallery comes back when a backup disk is opened with a new stick

- **Status:** proposed 2026-11-15 (alpha 0.2.7). Needs the security review
  that every Live change gets before the beta.
- **Date:** 2026-11-15
- **Spec reference:** block 8 (photos), block 5 (recovery).

## Context

The photos are on the backup disk, encrypted. The gallery's database
(which photo is which, albums, people, the phone's "already saved" list)
and its small pictures are in the index on the stick (ADR 0015). Each
test version is written over the stick, which empties the index. The disk
then opened as before with the ALP password or the 15 words, but the
gallery was empty and the phone sent every photo again. The same happens
to a person whose stick breaks or is lost.

Immich (v3.3.3) writes backups of its database beside the photos
(`backups/immich-db-backup-<time>-v<version>-pg<version>.sql.gz`, every
night at 02:00, the last 14 kept) and has its own restore flow for a
server that nobody has set up yet (`server/src/controllers/
database-backup.controller.ts`, `maintenance/`; read in the v3.3.3
source).

## Decision

1. **Restore with Immich's own flow, never our own SQL.** At the photo
   service's start with a new index (no database folder yet), a backup on
   the archive marks a restore as pending (a file in the index). Once the
   photo service answers, alpd calls, on loopback:
   `POST /api/admin/database-backups/start-restore` (allowed only while
   Immich has no owner), then with the cookie it answers
   `POST /api/admin/maintenance {"action": "restore_database",
   "restoreBackupFilename": <newest>}`, and polls
   `GET /api/admin/maintenance/status` until Immich leaves its
   maintenance mode (worked) or reports an error (alpd then sends
   `{"action": "end"}`; the gallery starts empty, as before).
2. **Nothing may set up the empty gallery in between.** While a restore
   is pending the photo service counts as "starting" (the screen says
   "Bringing your gallery back from the backup disk"), `admin_key`
   refuses, and the family-account worker waits.
3. **alpd's key after the restore: Immich's own
   `immich-admin reset-admin-password`.** The restored database has an
   owner whose password nobody ever knew and whose API key was on the old
   stick. alpd runs Immich's documented tool in the server's container (as
   the immich user, `podman exec -i`), gives it a new random password on
   its input (never an argument, never logged), signs in with it once,
   makes its API key as on a first start, and forgets the password.
   - *Rejected: a copy of alpd's API key on the backup disk.* It would
     put a working credential beside the photos, in plain text on a disk
     set up without encryption, and disks written by 0.2.61 and older
     would have none. With the tool, every disk that has a backup works.
4. **Backups when a Live computer needs them.** A Live computer is
   usually off at 02:00. alpd asks Immich for a backup with its own job
   (`POST /api/jobs {"name": "backup-database"}`) after the number of
   photos and videos changed and then stood still for a minute, at most
   every 10 minutes, and once more before turning off (waiting up to 30 s
   for the file).
5. **The small pictures are made again** (`PUT /api/jobs/
   thumbnailGeneration`, `videoConversion`, `force: true`: the restored database still lists
   the old files, so "only the missing ones" finds nothing); they were in
   the old index. On a large gallery this runs for hours in the
   background; the photos open meanwhile.

## Consequences

- A new stick shows the old gallery some minutes after the disk is
  opened; the phone finds its photos already saved and sends nothing
  twice. Changes made after the last backup (at most about 10 minutes of
  uploads when the computer lost power) are not in the gallery; those
  photos are still on the disk, and the phone sends them again.
- Not brought back: what else lived only in the index. Phones must be
  connected again (the device's certificate authority is new), and the
  console's settings are the defaults. "A new version without erasing the
  stick" (next) removes the need for most restores.
- A disk whose archive never got a backup file (alpha 0.2.6 and older ran
  no backup while the computer was off at night) has nothing to restore:
  the gallery starts empty as before. The photos are on the disk.
- The restore trusts the backup file on the archive. On an encrypted
  archive only someone with the key can write it. **Decided (security review and owner, 2026-11-16):**
  on an archive that is not encrypted, someone who had the disk in their
  hands can put a prepared backup file there. It is read as the
  database's administrator when a new stick opens the disk, which is
  more than replacing photos: it can add accounts or links the attacker
  knows, and run commands inside the database's container. So on such an
  archive the person is asked first (feature `restore_ask`:
  `state.immich.restore_ask`, `POST gallery/restore {"restore": bool}`;
  the marker carries a second line `ask` until then). "No" starts an
  empty gallery; the backup stays in `backups/alp-kept/`. An encrypted
  archive restores by itself as before. Covered by unit tests and the UI
  smoke test, not yet by a VM test (T-IMMICH-02 uses an encrypted
  archive).
- The restore never touches the photos. A restore that fails is tried
  again, three times; then the behaviour is the one before this ADR.
  The backup the restore starts from is first copied to
  `backups/alp-kept/` (security review, 2026-11-16): Immich keeps only
  its 14 newest backups, and an empty gallery after a failed restore
  would push the real one out within hours. The gallery is taken over
  without a restore only when Immich itself says it has an owner
  (`GET /api/server/config`, `isInitialized`), not on any refusal.
  Folders and files that are links are not used for this.
  The photo service's folders on the archive are refused when one of
  them is a link (root would change the owner of, or mount, what it
  points to).
- Test: T-IMMICH-02 (two sticks, one disk).
