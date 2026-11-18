"""Required exact cases for the named Q corpora."""
import hashlib
import json
from pathlib import Path


def required_exact(here, cases):
    cases = Path(cases)
    manifest = Path(here) / (cases.stem + ".exact.json")
    if not manifest.is_file():
        return set()
    required = json.loads(manifest.read_text())
    digest = hashlib.sha256(cases.read_bytes()).hexdigest()
    if digest != required["sha256"]:
        raise SystemExit(f"FAIL Q {cases.name} sha256 {digest} != {required['sha256']}")
    return set(required["ids"])


def failures(count, exact, required):
    if count["not-implemented"] == sum(count.values()):
        yield ("Q", "every case is not-implemented (or the corpus is empty)")
    missing = sorted(required - exact)
    if missing:
        yield ("Q", f"{len(missing)} required cases were not exact: {', '.join(missing[:12])}")
