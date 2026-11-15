#!/usr/bin/env python3
"""Clang optimization remarks for Pruv's LLVM output (docs/PERF_SPEC.md, "Optimization remarks").

The generated .ll has no debug info, so printed -Rpass remarks are `<unknown>:0:0` without a function name; this reads
the optimization record YAML (-fsave-optimization-record) instead, which carries `Function:`.

Usage: python3 bench/perf/remarks.py [--compiler py|rs] [--pass REGEX] [--kind missed,passed,analysis]
                                     [--function SUBSTR] <program-or-.pruv-path> ...
"""
import argparse, platform, re, subprocess, sys
from pathlib import Path

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
sys.path.insert(0, str(ROOT / 'bench' / 'g2'))
import run as g2  # noqa: E402

ARM = platform.machine().lower() in ('arm64', 'aarch64')  # same CPU flag logic as run.py llvm_objects
DEFAULT_PASSES = 'loop-vectorize|slp-vectorizer|licm|inline|loop-unroll'
DOC = re.compile(r'^--- !(\w+)')
ITEM = re.compile(r'^\s*- (\w+):\s?(.*)$')
TOP = re.compile(r'^(\w+):\s?(.*)$')


def unquote(v):
    v = v.strip()
    if len(v) >= 2 and v[0] == "'" and v[-1] == "'":
        return v[1:-1].replace("''", "'")
    if len(v) >= 2 and v[0] == '"' and v[-1] == '"':
        return re.sub(r'\\(.)', lambda m: {'n': '\n', 't': '\t'}.get(m.group(1), m.group(1)), v[1:-1])
    return v


def closed(v):
    """True when a scalar's quotes are complete (a quoted scalar may wrap onto continuation lines)."""
    v = v.strip()
    if v[:1] == "'":
        return len(v) >= 2 and v[-1] == "'" and (len(v) - len(v.rstrip("'"))) % 2 == 1 and len(v) > 1
    if v[:1] == '"':
        return len(v) >= 2 and v[-1] == '"' and not v.endswith('\\"')
    return True


def parse(text):
    """Yield (kind, pass, function, message) per document of an optimization record."""
    lines = text.splitlines()
    i, cur = 0, None
    out = []

    def scalar(v):
        nonlocal i
        while not closed(v) and i + 1 < len(lines):
            i += 1
            v += ' ' + lines[i].strip()
        return unquote(v)

    def flush():
        if cur and cur['kind']:
            out.append((cur['kind'], cur.get('Pass', ''), cur.get('Function', ''), ''.join(cur['args'])))

    in_args = False
    while i < len(lines):
        ln = lines[i]
        m = DOC.match(ln)
        if m:
            flush()
            k = m.group(1).lower()
            kind = 'passed' if k == 'passed' else 'missed' if k == 'missed' else 'analysis' if k.startswith('analysis') else None
            cur, in_args = {'kind': kind, 'args': []}, False
        elif cur is not None:
            t = TOP.match(ln)
            if t:
                in_args = t.group(1) == 'Args'
                if t.group(1) in ('Pass', 'Name', 'Function'):
                    cur[t.group(1)] = scalar(t.group(2))
            elif in_args:
                it = ITEM.match(ln)
                if it and it.group(1) != 'DebugLoc':
                    cur['args'].append(scalar(it.group(2)))
        i += 1
    flush()
    return out


def resolve(arg):
    p = Path(arg)
    if p.suffix == '.pruv':
        return p.resolve()
    return HERE / arg / f'{arg}.pruv'


def run_one(arg, a):
    src = resolve(arg)
    if not src.exists():
        raise SystemExit(f'remarks: no such program: {src}')
    work = HERE / 'work' / 'remarks' / src.stem
    work.mkdir(parents=True, exist_ok=True)
    c = g2.compile_pruv(a.compiler, str(src), str(work), 'llvm')
    if c['rc'] != 0 or not c['ll']:
        raise SystemExit(f'remarks: {src.name}: compile/verify failed: {c["errors"] or c["stderr"]}')
    rec = work / f'{src.stem}.opt.yaml'
    cmd = [g2.CLANG, '-O3', '-mcpu=native' if ARM else '-march=native', '-Wno-override-module', '-c', c['ll'],
           '-o', work / 'gen.o', '-fsave-optimization-record', f'-foptimization-record-file={rec}',
           f'-foptimization-record-passes={a.passes}']
    r = subprocess.run([str(x) for x in cmd], capture_output=True, text=True)
    if r.returncode != 0:
        raise SystemExit(f'remarks: clang failed ({r.returncode}): {(r.stdout + r.stderr)[-2000:]}')
    kinds = set(a.kind.split(','))
    seen, rows = set(), []
    for kind, ps, fn, msg in parse(rec.read_text()):
        if kind not in kinds or (a.function and a.function not in fn):
            continue
        line = f'{kind.upper()} {ps} {fn}: {msg}'
        if line not in seen:
            seen.add(line)
            rows.append((kind, ps, line))
    print(f'== {src.stem} ({a.compiler}/llvm)')
    for _, _, line in rows:
        print(line)
    counts = {}
    for kind, ps, _ in rows:
        counts[(kind, ps)] = counts.get((kind, ps), 0) + 1
    for (kind, ps), n in sorted(counts.items()):
        print(f'summary {kind} {ps}: {n}')
    if not rows:
        print('summary: no remarks')


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--compiler', default='rs', choices=['py', 'rs'])
    ap.add_argument('--pass', dest='passes', default=DEFAULT_PASSES, metavar='REGEX')
    ap.add_argument('--kind', default='missed,analysis')
    ap.add_argument('--function', default='')
    ap.add_argument('programs', nargs='+')
    a = ap.parse_args()
    bad = set(a.kind.split(',')) - {'missed', 'passed', 'analysis'}
    if bad:
        ap.error(f'unknown kind: {",".join(sorted(bad))}')
    for p in a.programs:
        run_one(p, a)
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
