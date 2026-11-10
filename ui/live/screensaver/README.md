<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Photo-frame scenes

Default screensaver pictures: calm green hills and blue skies, drawn as small SVG files
(`scene-1.svg` ... `scene-5.svg`, a few KB each, no network). They are original illustrations.

## Hook for real photos later

Put licence-checked CC0 photos in `ui/live/screensaver/photos/` and list them in
`ui/live/screensaver/photos/index.json`, for example `["hills.jpg", "lake.jpg"]`.
When that file exists, the screensaver uses those pictures instead of the SVG scenes.
Record each photo's source and licence in `photos/LICENSES.md` BEFORE adding it. Never
download pictures without checking the licence. Later (not now): the user's own photos from the archive.
