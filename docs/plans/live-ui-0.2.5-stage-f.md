<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Live UI 0.2.5, stage F: Files and Disks (plan)

Owner: Live UI Developer. The plan is based on two sources: the approved UX spec (section 9, stage F) and
the Code Orchestrator's order of 2026-11-04. The branch starts from `stream-a/integrate-0.2.5`.
Terminal has its own plan (`live-terminal.md`) and waits for the Security Expert's OK.

## Files (Settings > Advanced > Files, feature `media_files`)

Files is a read-only browser of external disks, built on `media.rs`:

- Disks on the left come from state `media`: `[{id, label, fs_type, size_bytes, mounted, refused}]`.
  - A refused disk shows the server's reason as text and cannot be opened.
- The UI sends only the opaque media id plus a path. The path is a list of names that the server
  itself listed (`["DCIM", "100CANON"]`). The server checks the path again with `components()` and `O_NOFOLLOW`.
- Long folders arrive in pages: "Show more" asks again with `offset` = the number of entries already shown,
  while `truncated` is true. Names that are not UTF-8 are counted (`skipped`) and reported, never shown.
- Previews:
  - photos (jpg, png, gif, webp, bmp) open in `<img>`;
  - videos (mp4, m4v, mov, webm) open in `<video controls>`, with no autoplay;
  - both load by URL on the console's own origin, which the CSP `'self'` already allows;
  - text files: the first 256 KB is read with fetch and put into a text node (`<pre>`);
  - every other kind of file says "cannot be opened here". Nothing is ever run, saved or opened with another program.
- There is no write action of any kind: no rename, delete, copy, move or drag. Eject is offered.

Proposed server shapes (the Storage Engineer builds them; branch storage/media-files):

| Call | Server side |
|---|---|
| `POST media/open {media}` | `media::mount_ro` |
| `POST media/list {media, path: [names], offset}` → `{entries: [{name, dir, size_bytes}], truncated, skipped}` | `MountedMedia::walk` |
| `GET media/file?media=<id>&path=<JSON list>` | `MountedMedia::open`. The answer must: take only the types the UI previews (`image/jpeg\|png\|gif\|webp\|bmp`, `video/mp4\|quicktime\|webm`, `text/plain; charset=utf-8`) and refuse the rest; send `X-Content-Type-Options: nosniff`, `Content-Disposition: inline`, `Content-Security-Policy: sandbox; default-src 'none'` and `Cache-Control: no-store`; support `Range`, because video needs it; never log the path. |
| `POST media/eject {media}` | `media::eject` |

## Disks (Settings > Advanced > Disks)

- Every disk of the computer is listed. The list is built from `hardware.specs.disks` (kernel names) plus
  `archive/candidates` (partitions, `disk_id`), so it needs no `live/disks`. The UI still uses `live/disks` if alpd ever offers it.
- Refused disks show a lock and a reason:
  - the ALPArchive stick (`system`);
  - the computer's own disks (`internal`);
  - the archive's disk;
  - a disk with no partitions found.

  The server refuses all of these again (`format::decision`).
- Format (T52):
  - The two choices show the estimate's own `quick.text` and `secure.text` as given, plus the time ("about N minutes" when `about` is true).
  - Then the password.
  - Then the last screen before the slide lists every partition and every `warnings` entry.
  - The progress screen shows the job message and "About N min left" from `seconds_left`.
  - Stop appears only while `can_cancel` is true in step `erasing`.

## Tests

`tests/ui/smoke.mjs` gains:

- the `files` preset;
- a Files flow:
  - names from the disk are text only, including an `<img onerror>` folder name and a `<b>` disk label;
  - paging and the skipped count;
  - photo and text previews, where text stays text;
  - an unknown kind of file;
  - a refused disk;
  - eject;
  - Files is hidden without `media_files`;
- a Disks check: three refused disks with their reasons, Format only on the others;
- in the format flow, the estimate's text, the time and the time left.
