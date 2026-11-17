#!/usr/bin/env python3
"""Dev loop: run the oracle and work/dev/harness on a fixture group (or ids) and compare COMPLETE observations.
Usage: python3 dev_compare.py S            (whole group)      python3 dev_compare.py S S01 S05    (group + ids)
Prints one line per case: ok, or the first differing JSON path with both values."""
import sys
sys.dont_write_bytecode = True
import json
import subprocess
from pathlib import Path

import common
import oracle

HERE = Path(__file__).resolve().parent
HARNESS = HERE / 'work' / 'dev' / 'harness'


def first_diff(a, b, path='$'):
    if type(a) != type(b):
        return path, a, b
    if isinstance(a, dict):
        for k in a:
            if k not in b:
                return f'{path}.{k}', a[k], '<missing>'
        for k in b:
            if k not in a:
                return f'{path}.{k}', '<missing>', b[k]
        for k in a:
            d = first_diff(a[k], b[k], f'{path}.{k}')
            if d:
                return d
        return None
    if isinstance(a, list):
        for i in range(min(len(a), len(b))):
            d = first_diff(a[i], b[i], f'{path}[{i}]')
            if d:
                return d
        if len(a) != len(b):
            return f'{path}.length', len(a), len(b)
        return None
    return None if a == b else (path, a, b)


def run_harness(cases, timeout=300):
    stdin = ''.join(json.dumps(common.strip_case(c), ensure_ascii=True) + '\n' for c in cases)
    p = subprocess.run([str(HARNESS)], input=stdin, capture_output=True, text=True, encoding='utf-8', timeout=timeout)
    out = [json.loads(l) for l in p.stdout.split('\n') if l.strip()]
    if len(out) != len(cases):
        raise RuntimeError(f'harness returned {len(out)} results for {len(cases)} cases (rc {p.returncode}): {p.stderr[-1500:]}')
    return out


def main(argv):
    group = argv[1] if len(argv) > 1 else 'S'
    ids = set(argv[2:])
    cases = [c for c in common.load_group(group) if not ids or c['id'] in ids]
    want = oracle.run_oracle([common.strip_case(c) for c in cases])
    got = run_harness(cases)
    bad = 0
    for c, w, g in zip(cases, want, got):
        d = first_diff(w, g)
        if d is None:
            print(f'{c["id"]}: ok')
        else:
            bad += 1
            path, a, b = d
            print(f'{c["id"]}: DIFF at {path}\n   oracle: {json.dumps(a, ensure_ascii=False)[:300]}\n   impl:   {json.dumps(b, ensure_ascii=False)[:300]}')
    print(f'{len(cases) - bad}/{len(cases)} equal')
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
