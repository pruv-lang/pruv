"""Checks that generated shared-table files are up to date and that every pruvc/data/*.json is generated.

python tests/check_generated.py
(--only-py, --only-rs, --py PATH, --rs PATH, --work DIR are accepted and ignored: run_all passes them.)
"""
import glob
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "tools"))

import gen_shared  # noqa: E402


def main():
    bad = False
    for path in sorted(glob.glob(os.path.join(ROOT, "pruvc", "data", "*.json"))):
        name = os.path.splitext(os.path.basename(path))[0]
        if name not in gen_shared.TABLES:
            print("not in tools/gen_shared.py TABLES: pruvc/data/%s.json" % name)
            bad = True
    out = os.path.join(ROOT, gen_shared.OUT)
    try:
        with open(out, "rb") as f:
            current = f.read()
    except OSError:
        current = None
    if current != gen_shared.render().encode("utf-8"):
        print("stale: %s; run tools/gen_shared.py" % gen_shared.OUT)
        bad = True
    if bad:
        return 1
    print("check_generated: ok (%d tables)" % len(gen_shared.TABLES))
    return 0


if __name__ == "__main__":
    sys.exit(main())
