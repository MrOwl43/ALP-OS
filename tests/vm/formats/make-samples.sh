#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Make the format test set of T-FORMAT-01: one small picture or one-second
# clip per format, all made here from a test pattern (no downloaded files,
# nothing with a licence of its own). Camera RAW cannot be made this way: it
# needs real files from real cameras, so those are downloaded with a pinned
# SHA-256 each (raw-samples.lock, docs/qa/formats.md).
#
# Usage: tests/vm/formats/make-samples.sh <out-dir>
# Needs ffmpeg; heif-enc (libheif-examples) for HEIC; curl for RAW.
# ALP_RAW=0 leaves RAW out. ALP_RAW_CACHE=<dir> keeps the downloads between
# runs (checked again each time). ALP_RAW_URL=<base> fetches from another
# copy of the same files.
set -euo pipefail
out=${1:?usage: make-samples.sh <out-dir>}
mkdir -p "$out"
ff() { ffmpeg -hide_banner -loglevel error -nostdin -y "$@"; }
# A different pattern per file (the seed), so no two files are the same photo.
n=0
pic() { n=$((n + 1)); ff -f lavfi -i "testsrc2=size=320x240:rate=1,hue=h=$((n * 23))" -frames:v 1 "$@"; }
clip() { n=$((n + 1)); ff -f lavfi -i "testsrc2=size=320x240:rate=25,hue=h=$((n * 23))" -f lavfi -i "sine=frequency=$((300 + n * 20)):sample_rate=48000" -t 1 "$@"; }

# Pictures.
pic "$out/photo-jpeg.jpg"
pic "$out/photo-png.png"
pic -c:v libwebp "$out/photo-webp.webp"
pic "$out/photo-tiff.tiff"
pic "$out/photo-bmp.bmp"
pic "$out/photo-gif.gif"
pic -c:v jpeg2000 "$out/photo-jpeg2000.jp2"
pic -c:v libjxl "$out/photo-jpegxl.jxl"
pic -c:v libaom-av1 -still-picture 1 -crf 30 "$out/photo-avif.avif"
if command -v heif-enc >/dev/null; then
    pic "$out/.heic-source.png"
    heif-enc -q 60 -o "$out/photo-heic.heic" "$out/.heic-source.png" >/dev/null
    rm -f "$out/.heic-source.png"
else
    echo "make-samples: no heif-enc; HEIC is left out" >&2
fi

# Clips: container-codec.
aac=(-c:a aac -b:a 64k)
clip -c:v libx264 -pix_fmt yuv420p "${aac[@]}" "$out/video-mp4-h264.mp4"
clip -c:v libx265 -pix_fmt yuv420p -tag:v hvc1 -x265-params log-level=error "${aac[@]}" "$out/video-mp4-h265.mp4"
clip -c:v libx265 -pix_fmt yuv420p10le -tag:v hvc1 -x265-params log-level=error "${aac[@]}" "$out/video-mov-h265-10bit.mov"
clip -c:v libaom-av1 -cpu-used 8 -crf 40 -pix_fmt yuv420p "${aac[@]}" "$out/video-mp4-av1.mp4"
clip -c:v libvpx-vp9 -deadline realtime -pix_fmt yuv420p -c:a libopus -b:a 48k "$out/video-webm-vp9.webm"
clip -c:v prores_ks -profile:v 2 -pix_fmt yuv422p10le -c:a pcm_s16le "$out/video-mov-prores.mov"
clip -c:v mjpeg -q:v 5 -pix_fmt yuvj422p -c:a pcm_s16le "$out/video-mov-mjpeg.mov"
clip -c:v libx264 -pix_fmt yuv420p "${aac[@]}" "$out/video-mkv-h264.mkv"
clip -c:v mpeg4 -q:v 5 -c:a mp3 "$out/video-avi-mpeg4.avi"
clip -c:v mpeg2video -q:v 5 -c:a mp2 "$out/video-mpg-mpeg2.mpg"
clip -c:v libx264 -pix_fmt yuv420p -c:a ac3 -f mpegts "$out/video-mts-h264.mts"
clip -c:v libx264 -pix_fmt yuv420p "${aac[@]}" -f 3gp "$out/video-3gp-h264.3gp"
clip -c:v wmv2 -c:a wmav2 "$out/video-wmv-wmv2.wmv"
clip -c:v flv1 -c:a mp3 -ar 44100 "$out/video-flv-flv1.flv"
clip -c:v mpeg2video -q:v 5 -c:a pcm_s16le -f mxf "$out/video-mxf-mpeg2.mxf"

# Camera RAW: real files, downloaded, each checked against its pinned SHA-256.
# A file that does not arrive is left out with a note; a file that arrives
# changed stops the test (it is not the file the result is about).
lock=$(dirname "$(realpath "$0")")/raw-samples.lock
raw_url=${ALP_RAW_URL:-https://github.com/MrOwl43/ALP-OS/releases/download/test-raw-samples-1}
cache=${ALP_RAW_CACHE:-$out/.raw-cache}
if [[ ${ALP_RAW:-1} != 0 ]] && grep -qv '^#' "$lock"; then
    mkdir -p "$cache"
    while read -r sum name _camera; do
        [[ -z $sum || $sum == \#* ]] && continue
        [[ $name =~ ^raw-[a-z0-9-]+\.[a-z0-9]+$ ]] || { echo "make-samples: bad name in raw-samples.lock: $name" >&2; exit 1; }
        if ! echo "$sum  $cache/$name" | sha256sum -c --status 2>/dev/null; then
            rm -f "$cache/$name"
            curl -fsSL --retry 3 --retry-delay 5 -m 300 -o "$cache/$name.part" "$raw_url/$name" \
                && mv "$cache/$name.part" "$cache/$name" \
                || { rm -f "$cache/$name.part"; echo "note: $name could not be downloaded; left out" >&2; continue; }
            echo "$sum  $cache/$name" | sha256sum -c --status \
                || { echo "make-samples: $name does not match its pinned SHA-256" >&2; exit 1; }
        fi
        cp "$cache/$name" "$out/$name"
    done <"$lock"
    [[ -n ${ALP_RAW_CACHE:-} ]] || rm -rf "$cache"
fi
find "$out" -type f | wc -l
