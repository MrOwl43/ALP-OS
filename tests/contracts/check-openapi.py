#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Contract check: API files parse, are OpenAPI 3.1, every $ref resolves, and\nevery alpd operation declares its listeners (none on the LAN in the beta).
Usage: tests/contracts/check-openapi.py api/*.openapi.yaml"""
import sys

import yaml


def resolve(doc, ref):
    node = doc
    for part in ref.removeprefix("#/").split("/"):
        node = node[part.replace("~1", "/").replace("~0", "~")]
    return node


def refs(node):
    if isinstance(node, dict):
        for key, value in node.items():
            if key == "$ref":
                yield value
            else:
                yield from refs(value)
    elif isinstance(node, list):
        for value in node:
            yield from refs(value)


status = 0
for path in sys.argv[1:]:
    with open(path) as f:
        doc = yaml.safe_load(f)
    try:
        assert doc["openapi"].startswith("3.1"), "not OpenAPI 3.1"
        found = list(refs(doc))
        for ref in found:
            resolve(doc, ref)
        listeners = doc.get("x-alp-listeners")
        if listeners is not None:
            # Access contract (docs/contracts/api-access.md): every operation
            # names known listeners; nothing is served on the LAN in the beta.
            for route, item in doc["paths"].items():
                for method, op in item.items():
                    if method not in ("get", "put", "post", "patch", "delete"):
                        continue
                    on = op.get("x-alp-listeners")
                    assert on, f"{method.upper()} {route}: no x-alp-listeners"
                    unknown = set(on) - set(listeners)
                    assert not unknown, f"{method.upper()} {route}: unknown listeners {unknown}"
                    assert "lan" not in on, f"{method.upper()} {route}: management API on the LAN"
                    if method in ("post", "put", "patch"):
                        body = op.get("requestBody", {}).get("content", {})
                        assert "application/json" in body, (
                            f"{method.upper()} {route}: needs a JSON body (CSRF rule)")
        print(f"ok: {path}: {len(doc['paths'])} paths, {len(found)} refs resolve")
    except (AssertionError, KeyError, TypeError) as e:
        print(f"FAIL: {path}: {e!r}", file=sys.stderr)
        status = 1
sys.exit(status)
