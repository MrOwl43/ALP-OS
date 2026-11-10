#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Live UI text check (docs/plans/live-ui-0.2.4.md).

1. Every literal key the UI asks for (t("…"), "…" keys in KEY_ICON, step/danger
   prefixes are listed explicitly) exists in ui/live/i18n/en.json.
2. "Immich" appears in no text except the allowed keys: the photo service is
   named only where a person needs the name (the app's store name on the phone
   step) and in About > Credits (UX spec 4).
Exit 1 with a list on any problem.
"""
import json
import re
import sys
from pathlib import Path

root = Path(__file__).resolve().parents[2]
ui = root / "ui" / "live"
texts = json.loads((ui / "i18n" / "en.json").read_text(encoding="utf-8"))
src = (ui / "app.js").read_text(encoding="utf-8")

used = set(re.findall(r'\bt\("([a-z0-9_.]+)"', src))
used |= set(re.findall(r'(?:Key|key|reason|hint|slowKey)\s*\(?\s*"([a-z][a-z0-9_]*\.[a-z0-9_.]+)"', src))
# Keys built from a prefix at run time: every value must exist.
dynamic = {
    "cat.": ["phones", "password", "disk", "chain", "network", "display", "power", "about", "advanced"],
    "bar.": ["gallery", "settings", "computer", "disks", "admin", "import", "format"],
    "step.": [str(i) for i in range(1, 8)],
    "about.protection.": ["none", "console", "password", "strong"],
    "fmt.warn.": ["bitlocker", "windows", "alparchive_folder", "system"],
    "disks.role.": ["backup", "system", "other"],
    "disks.kind.": ["stick", "external", "internal"],
    "disks.health.": ["healthy", "wear", "failing"],
    "lv.": ["ok", "note", "action", "now", "critical"],
    "lv.word.": ["action", "now", "critical"],
    "crit.paused.what.": ["import", "delete"],
}
for pre, vals in dynamic.items():
    used |= {pre + v for v in vals}
ignore = {k for k in used if k.endswith(".")}
missing = sorted(k for k in used - ignore if k not in texts)

allowed_immich = {"phone.app.name", "credit.immich", "about.credits.d"}
immich = sorted(k for k, v in texts.items() if "immich" in str(v).lower() and k not in allowed_immich)

bad = False
if missing:
    bad = True
    print("FAIL: keys used by ui/live/app.js but missing in en.json:")
    for k in missing:
        print("  " + k)
if immich:
    bad = True
    print("FAIL: texts that name Immich outside the allowed places (UX spec 4):")
    for k in immich:
        print("  " + k)
if bad:
    sys.exit(1)
print(f"ok: {len(used - ignore)} text keys present, Immich named only in {sorted(allowed_immich)}")
