<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Picture and video formats (alpha 0.2.7)

**The rule that does not depend on the format:** every file a phone or a
card sends is kept on the backup disk exactly as it is, byte for byte.
"Support" below only means: the gallery can show a small picture of it and
play it.

## The format test set: T-FORMAT-01

`tests/vm/t-format-01.sh` uploads one small file per format (made by
`tests/vm/formats/make-samples.sh` from a test pattern) and prints for each
`FORMAT <file> taken= kept= preview= play= must=`. Every file must come back
unchanged. The formats marked `must` must also get a preview (and, for
video, be served for playing); the others are reported, so the table of a
CI run is the current truth.

| Kind | In the set |
| --- | --- |
| Pictures | JPEG, PNG, WebP, TIFF, BMP, GIF, JPEG 2000, JPEG XL, AVIF, HEIC |
| Video (container, codec) | MP4 H.264; MP4 H.265; MOV H.265 10-bit; MP4 AV1; WebM VP9; MOV ProRes; MOV Motion JPEG; MKV H.264; AVI MPEG-4; MPG MPEG-2; MTS H.264; 3GP H.264; WMV; FLV; MXF MPEG-2 |

## Result (CI run 43751701211, 2026-11-15, Immich v3.3.3)

All 25 files were taken and came back byte for byte. 24 of 25 got a preview
and every video was served for playing. The one without a preview:
**JPEG 2000** (`.jp2`): kept, no picture in the gallery.

## Not in the set yet: camera RAW

RAW files cannot be made from a test pattern; they need real files from
real cameras. Immich reads RAW with LibRaw (Canon CR2/CR3, Nikon NEF, Sony
ARW, Fujifilm RAF, Panasonic RW2, Olympus ORF, Pentax PEF, Leica, Hasselblad,
Phase One, Sigma, Samsung, DNG; list in Immich v3.3.3
`server/src/utils/mime-types.ts`). Next step: a second set of real,
freely licensed sample files (one per maker), fetched by the test with a
pinned checksum each.

## Known limits

- Cinema RAW video (Blackmagic RAW, RED, ProRes RAW) and some of the newest
  compressed RAW variants have no open source reader. The files are kept;
  the gallery shows no picture of them.
- A camera newer than the LibRaw in Immich: kept, no preview until Immich
  updates.
- H.265 and AAC are free software here, but patent holders can ask fees of
  companies that sell products with them. To be checked by a lawyer before
  a paid service or hardware.
