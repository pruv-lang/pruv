#!/usr/bin/env python3
"""L3-2 Q mutants: single replacements in supervise.pruv, run against the A3 fixtures on rs/rust.

Each mutant is classified as rejected (verification or compilation fails), detected (a fixture differs or the run
fails) or SURVIVED. Writes results/mutants.json.
"""
import json
import shutil
import subprocess
import sys

sys.dont_write_bytecode = True
import common  # noqa: E402
import test_a3  # noqa: E402

HERE = common.HERE
WORK = HERE / 'work' / 'mut'

MUTANTS = [
    ('stopped swapped', 'if tag == 2 { stopped = 1; }', 'if tag == 2 { stopped = 2; }'),
    ('stop byte as disconnect', '                tag = 2;', '                tag = 3;'),
    ('no draining after end', 'if ek == 3 { draining = true; }', 'if ek == 3 { draining = false; }'),
    ('no drain on stop', 'let mut more: bool = true;', 'let mut more: bool = false;'),
    ('signal status dropped', 'if x >= -2147483648 && x <= 2147483647 {', 'if x >= 0 && x <= 2147483647 {'),
    ('spawn failure as exit', 'opened, 2, code, 0, 0, 0);', 'opened, 1, code, 0, 0, 0);'),
    ('reaped not set', '        reaped = 1;\n      }\n    }\n    // A normal',
     '        reaped = 0;\n      }\n    }\n    // A normal'),
    ('stream 2 dropped', 'f = take(move f, inout cw, inout buf, inout ts, 2, d2);', ''),
    ('stream index swapped', 'ts, 1, d1);', 'ts, 2, d1);'),
    ('file_open inverted', 'if opened { h[8] = 1; }', 'if !opened { h[8] = 1; }'),
    ('stdin kept open', 'let p: Process = proc_close_stdin(move p0);\n  if !proc_ok(p) {',
     'let p: Process = move p0;\n  if !proc_ok(p) {'),
    ('slot check dropped', 'if s < 0 || s >= ev_cap(l) { return 0; }', ''),
]


def main():
    cases = []
    test_a3.run_lines = lambda name, root, harness, c, **kw: cases.extend(c) or 0
    test_a3.main()
    payload = ''.join(line + '\n' for _, line, _ in cases)
    g2, run = common.configure(WORK)
    src = (HERE / 'supervise.pruv').read_text()
    rows = []
    for name, old, new in MUTANTS:
        assert src.count(old) == 1, name
        d = WORK / name.replace(' ', '_')
        shutil.rmtree(d, ignore_errors=True)
        d.mkdir(parents=True)
        for m in ('args', 'capfile', 'packet'):  # tail comes from the shared package (--manifest)
            shutil.copy(HERE / f'{m}.pruv', d / f'{m}.pruv')
        (d / 'supervise.pruv').write_text(src.replace(old, new))
        common.copy_manifest(HERE / 'supervise.pruv', d)
        try:
            exe = common.build(g2, run, 'rs', 'rust', d / 'supervise.pruv', d / 'build', test_a3.HARNESS, 'mut',
                               common.manifest_flags(d / 'supervise.pruv'))
        except Exception as exc:
            rows.append(dict(mutant=name, result='rejected', detail=str(exc)[:300]))
            print(f'{name}: rejected', flush=True)
            continue
        try:
            r = subprocess.run([str(exe)], input=payload, capture_output=True, text=True, timeout=180)
            got = r.stdout.splitlines()
            bad = [lab for (lab, _, w), g in zip(cases, got) if w != g]
            if r.returncode != 0 or len(got) != len(cases):
                bad.append(f'rc={r.returncode} lines={len(got)}')
        except subprocess.TimeoutExpired:
            bad = ['timeout']
        result = 'detected' if bad else 'SURVIVED'
        rows.append(dict(mutant=name, result=result, detail=', '.join(bad)[:300]))
        print(f'{name}: {result} {bad[:5]}', flush=True)
    (HERE / 'results').mkdir(exist_ok=True)
    (HERE / 'results' / 'mutants.json').write_text(json.dumps(rows, indent=1, ensure_ascii=False) + '\n')
    return 0


if __name__ == '__main__':
    sys.exit(main())
