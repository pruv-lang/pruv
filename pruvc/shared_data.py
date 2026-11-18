"""Shared tables: pruvc/data/<name>.json is the single source for values that pruvc and pruvc-rs must agree on.

The Rust side is generated from the same files by tools/gen_shared.py.
"""
import json
import os

_CACHE = {}


def load(name):
    """Return {item name: value} of pruvc/data/<name>.json, in file order (cached per name)."""
    if name not in _CACHE:
        path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "data", name + ".json")
        with open(path, encoding="utf-8") as f:
            doc = json.load(f)
        _CACHE[name] = {item["name"]: item["value"] for item in doc["items"]}
    return _CACHE[name]
