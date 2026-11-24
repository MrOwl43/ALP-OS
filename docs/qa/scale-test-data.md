# Scale test data plan (draft)

Status: plan only, nothing implemented. The full run below is for AFTER the beta versions
(owner decision 2026-11-24). Until then, automated test data stays at or below 1 GB in total,
and the owner tests large files himself on real hardware.

## Now (before beta): small variant

- About 10,000 tiny photos (the generator below, 320x240) and a few short videos, under 1 GB
  in total, on small virtual disks (for example two 8 GB disks for RAID1).
- Same checks as the full run, at this size: every item lands once, the gallery lists them
  all, a disk pulled from the mirror loses nothing.

## Later (after beta): full scale

## Goal

Show that the archive stays usable and stable with a large library, without ever using
real personal data:

- 300,000 photo records and about 6,000 short videos (2%), spread over 15 years.
- 20 TB of disk space across 2 to 4 disks, in a RAID1 archive (Btrfs on LUKS2).

The test runs in CI in a QEMU/KVM test VM started by `tests/vm/run-vm.sh` (OVMF, swtpm),
like every other `t-*.sh` scenario. Proposed name: `tests/vm/t-scale-01.sh`.

## 1. Fake media

Everything is generated from one fixed seed, so two runs make byte-identical files.

### Photos

- Size: 640x480 JPEG, quality 70, about 15-25 KB each. 300,000 files are then about
  6 GB, not 60 GB. Gallery speed depends on the number of items, not their pixels.
- Content: a solid color picked from the seed and the record number written in the
  middle, so a broken thumbnail is easy to spot by eye.
- Metadata (written with `exiftool`):
  - `DateTimeOriginal`: random date and time between 2011-01-01 and 2025-12-31,
    with a few bursts (200 photos within one hour) as phones really make them.
  - GPS: 40% of photos get coordinates from a short list of fake places
    (random points over oceans are not realistic for map clustering); 60% get none.
  - Camera make/model: one of five fake values.
  - 1% have no date at all, 0.5% have a broken EXIF block (robustness).
- Duplicates: 1% are exact copies of another photo (tests duplicate handling).

One generator script (Python, standard library + Pillow, or ImageMagick) writes the files
and a `manifest.csv` (file name, date, GPS yes/no, checksum). A second step runs
`exiftool -overwrite_original` in batches using an argument file (`-@`), not one
process per photo. VERIFY: exact exiftool options for writing GPS reference tags.

### Videos

- 6,000 clips, 3 seconds, 320x240, H.264 in MP4, made with the `ffmpeg` `testsrc2`
  source with the record number drawn on it. About 100-200 KB each, about 1 GB total.
- 20 of them are long clips (2 minutes, 1280x720) to test playback and transcoding.
- Creation date set with `-metadata creation_time=...`. VERIFY: Immich reads this tag
  for the capture date.

### Where the media is made

Generating 300,000 files takes time. Make them once per seed, pack them into a
compressed tarball (CI cache keyed by the seed and the script's checksum), and copy
the tarball into the test VM on a second virtual disk. Do not generate inside the VM.

## 2. Large virtual disks

- 2 disks of 10 TB each for the RAID1 case (20 TB raw, 10 TB usable), and a second
  variant with 4 disks of 5 TB each. Both total 20 TB.
- Created with `qemu-img create -f qcow2 disk-N.qcow2 10T`. A new qcow2 file only
  takes a few hundred KB; it grows only with data actually written.
- Real host space used is roughly: data written (about 7 GB of media, times 2 for
  RAID1) plus Btrfs and LUKS metadata, plus thumbnails and database. Expect about
  25-35 GB per run. CI runners have limited disk (VERIFY: free space on the runner
  type we use); the test must check free space first and fail clearly if too low.
- Limits of this approach:
  - Speed of a sparse qcow2 file says nothing about real disk speed. Measure relative
    numbers (this run against the last run), never absolute disk speed.
  - "Disk near full" cannot be tested on a 10 TB disk without writing 10 TB. Use a
    separate small variant (2 disks of 20 GB) for the full-disk test.
  - Formatting large LUKS2 disks must not write the whole disk (no full wipe in this
    test), otherwise the file grows to its full size.

## 3. Loading into Immich

- Use the official Immich command line tool (`@immich/cli`, same version as the
  pinned server in `image/immich/images.lock`, today v3.3.3). VERIFY: the CLI version
  that matches server v3.3.3, and its login and upload options
  (`immich login`, `immich upload --recursive`, concurrency option).
- Upload through the same network path a phone uses, so the test covers the real
  upload route, not a shortcut.
- Upload in groups of 10,000 and record time per group, so a slowdown as the library
  grows shows up as a rising line, not only as a slow total.
- After upload, wait until Immich's background jobs (thumbnails, metadata) are idle
  before measuring. VERIFY: the API call that reports job queue status.

## 4. What to measure

| Measurement | How |
|---|---|
| Upload time per 10,000 items | timestamps around each group |
| Time until all thumbnails are made | job queue idle |
| Gallery first page load (timeline API) | API call time, median of 10 |
| Scroll to a date 10 years back | API call time |
| Open one photo and one long video | API call time, video plays |
| Database size | PostgreSQL size query |
| Memory use of Immich and alpd | peak and steady state |
| Btrfs scrub time for the whole archive | `btrfs scrub` duration |
| List a folder of 10,000 files | native screen / alpd API time |
| Behavior near a full disk (small variant) | clear warning, no data loss, no crash |
| Reboot with the full library | archive unlocks, gallery comes back |

## 5. Pass / fail limits

All TBD until a first baseline run on the CI runner. Proposed starting points:

- Gallery first page: under TBD seconds (median).
- No upload group more than TBD% slower than the first group.
- Memory: Immich plus alpd under TBD GB steady state.
- Zero failed uploads, zero missing items (count in Immich equals manifest).
- Every checksum in the manifest matches the stored original.
- Near-full disk: the user sees a warning before writes fail; nothing already stored
  is lost or changed.

## 6. Risks

- Run time: thumbnails for 300,000 items may take hours on a CI runner without a GPU.
  Option: a nightly job, and a small daily variant with 10,000 items.
- Host disk space on CI runners (see section 2).
- Fake media is simpler than real phone photos (HEIC, live photos, edited copies);
  add a small set of public-domain real-format samples later.
- Results on a virtual machine are not the same as a real 20 TB machine; one real
  hardware run before the beta is still needed.
- The test must never touch real disks: it only runs inside the test VM against qcow2
  files (iron rule).
