#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Split docs/spec.md into docs/spec/*.md so each stream reads only what it
needs (overview + rules + its blocks). docs/spec.md stays the single file the
owner edits; the split files are generated and must not be edited.

Usage: tools/split-spec.py          write docs/spec/
       tools/split-spec.py --check  fail if docs/spec/ is out of date
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SPEC = ROOT / "docs/spec.md"
OUT = ROOT / "docs/spec"

# Output file per top-level section number; section 7 is split per block.
SECTIONS = {
    "0": "overview.md", "1": "overview.md", "9": "overview.md",
    "2": "rules.md", "3": "hardware.md", "4": "layout.md", "5": "traits.md",
    "6": "system.md", "8": "tests.md", "10": "open-items.md", "11": "future.md",
}


def split(text):
    lines = text.splitlines(keepends=True)
    header, files, current = [], {}, None
    for line in lines:
        m = re.match(r"^## (\d+)\. ", line)
        b = re.match(r"^### Block (\d+) ", line)
        if m:
            current = SECTIONS.get(m.group(1), "blocks-intro.md") if m.group(1) != "7" else "blocks-intro.md"
        elif b:
            current = f"block-{int(b.group(1)):02d}.md"
        if current is None:
            header.append(line)
        else:
            files.setdefault(current, []).append(line)
    version = next((l for l in header if l.startswith("Version ")), "").split(" · ")[0].strip()
    note = (f"<!-- Generated from docs/spec.md ({version}) by tools/split-spec.py. "
            "Do not edit; edit docs/spec.md and rerun the tool. -->\n\n")
    out = {}
    for name, body in files.items():
        prefix = note + ("".join(header) if name == "overview.md" else "")
        out[name] = prefix + "".join(body).rstrip("\n") + "\n"
    index = [note, "# ALP-OS spec, split by topic\n\n",
             "Read `overview.md` and `rules.md` always; then only the files for your work.\n\n"]
    for name in sorted(out):
        first = next(l for l in out[name].splitlines() if l.startswith("#"))
        index.append(f"- [{name}]({name}): {first.lstrip('# ').strip()}\n")
    out["README.md"] = "".join(index)
    return out


def main():
    want = split(SPEC.read_text())
    check = "--check" in sys.argv[1:]
    have = {p.name: p.read_text() for p in OUT.glob("*.md")} if OUT.exists() else {}
    if check:
        if have != want:
            stale = sorted(set(have) ^ set(want) | {n for n in want if have.get(n) != want[n]})
            print("FAIL: docs/spec/ is out of date; run tools/split-spec.py:", ", ".join(stale),
                  file=sys.stderr)
            return 1
        print(f"ok: docs/spec/ matches docs/spec.md ({len(want)} files)")
        return 0
    OUT.mkdir(exist_ok=True)
    for name in set(have) - set(want):
        (OUT / name).unlink()
    for name, text in want.items():
        (OUT / name).write_text(text)
    print(f"wrote {len(want)} files to docs/spec/")
    return 0


sys.exit(main())
