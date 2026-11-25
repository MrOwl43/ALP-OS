#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Checks inside a built start file (UKI), without booting it.

  check-uki.py profile <uki> <profile-id> <cmdline-word>
      The UKI has a profile with this ID whose own .cmdline section holds
      the word (the kernel itself also contains words like "nomodeset", so
      a plain search of the file proves nothing).
  check-uki.py modules <uki> <module> [<module> ...]
      The initrd carries each kernel module (basename, '-' and '_' alike).
      The initrd is mkosi's concatenation of cpio archives, some zstd
      compressed (microcode raw, base initrd and modules initrd zstd),
      with zero padding between them.

Exit 0 when every check passes, 1 otherwise; prints one line per check.
Field bug 0.2.7 (old laptops, blinking cursor): image/mkosi.conf
KernelInitrdModules= and image/mkosi.uki-profiles/.
"""
import struct
import subprocess
import sys


def sections(data):
    """PE sections in file order: (name, bytes)."""
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    if data[pe:pe + 4] != b"PE\0\0":
        sys.exit("FAIL: not a PE file")
    count = struct.unpack_from("<H", data, pe + 6)[0]
    opt_size = struct.unpack_from("<H", data, pe + 20)[0]
    table = pe + 24 + opt_size
    out = []
    for i in range(count):
        off = table + 40 * i
        name = data[off:off + 8].rstrip(b"\0").decode("ascii", "replace")
        vsize, _, rsize, rptr = struct.unpack_from("<IIII", data, off + 8)
        out.append((name, data[rptr:rptr + min(vsize, rsize) if vsize else rsize]))
    return out


def profiles(secs):
    """[(profile text, cmdline text)] for each .profile and the .cmdline after it."""
    found, current = [], None
    for name, body in secs:
        if name == ".profile":
            if current is not None:
                found.append(current)
            current = [body.decode("utf-8", "replace"), ""]
        elif name == ".cmdline" and current is not None:
            current[1] = body.rstrip(b"\0").decode("utf-8", "replace")
    if current is not None:
        found.append(current)
    return found


def cpio_names(blob):
    """(names, end): file names of the newc cpio archives at the start of
    blob, and where they end (zero padding between archives skipped)."""
    names, pos = [], 0
    while True:
        nxt = pos
        while nxt < len(blob) and blob[nxt] == 0:
            nxt += 1
        if blob[nxt:nxt + 6] not in (b"070701", b"070702") or nxt + 110 > len(blob):
            return names, pos
        pos = nxt
        h = blob[pos:pos + 110]
        size = int(h[54:62], 16)
        nsize = int(h[94:102], 16)
        name = blob[pos + 110:pos + 110 + nsize - 1].decode("utf-8", "replace")
        pos = (pos + 110 + nsize + 3) & ~3
        pos = (pos + size + 3) & ~3
        if name != "TRAILER!!!":
            names.append(name)


def unzstd(blob):
    """Decompress blob, or None. Retries without trailing zero padding."""
    for cut in (blob, blob.rstrip(b"\0")):
        out = subprocess.run(["zstd", "-dcq"], input=cut, capture_output=True)
        if out.returncode == 0:
            return out.stdout
    return None


def initrd_names(initrd):
    """Names in mkosi's initrd: uncompressed cpio archives first (microcode),
    then zstd compressed ones, possibly with padding between them."""
    names, pos = cpio_names(initrd)
    zstd = b"\x28\xb5\x2f\xfd"
    rest = initrd[pos:]
    # Split at every zstd magic; a piece that does not unpack alone had a
    # magic inside its compressed data, so join it with the next piece.
    starts, at = [], rest.find(zstd)
    while at >= 0:
        starts.append(at)
        at = rest.find(zstd, at + 1)
    if rest.strip(b"\0") and (not starts or rest[:starts[0]].strip(b"\0")):
        sys.exit("FAIL: initrd: unknown data after the uncompressed part")
    i = 0
    while i < len(starts):
        for j in range(i + 1, len(starts) + 1):
            end = starts[j] if j < len(starts) else len(rest)
            out = unzstd(rest[starts[i]:end])
            if out is not None:
                names += cpio_names(out)[0]
                i = j
                break
        else:
            sys.exit("FAIL: initrd: zstd could not unpack it")
    return names


def main():
    if len(sys.argv) < 4 or sys.argv[1] not in ("profile", "modules"):
        sys.exit(__doc__)
    data = open(sys.argv[2], "rb").read()
    secs = sections(data)
    ok = True
    if sys.argv[1] == "profile":
        want_id, word = sys.argv[3], sys.argv[4]
        hit = [c for p, c in profiles(secs)
               if any(l.strip() in (f"ID={want_id}", f'ID="{want_id}"') for l in p.splitlines())]
        if hit and word in hit[0].split():
            print(f"  ok: profile {want_id} starts with {word}")
        else:
            print(f"  FAIL: profile {want_id} with {word} on its command line not found", file=sys.stderr)
            ok = False
    else:
        initrd = b"".join(body for name, body in secs if name == ".initrd")
        have = set()
        for n in initrd_names(initrd):
            base = n.rsplit("/", 1)[-1]
            for ext in (".zst", ".xz", ".gz"):
                base = base.removesuffix(ext)
            if base.endswith(".ko"):
                have.add(base[:-3].replace("-", "_"))
        print(f"  initrd: {len(have)} kernel modules")
        for m in sys.argv[3:]:
            if m.replace("-", "_") in have:
                print(f"  ok: initrd has {m}")
            else:
                print(f"  FAIL: initrd lacks {m}", file=sys.stderr)
                ok = False
    sys.exit(0 if ok else 1)


main()
