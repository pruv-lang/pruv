"""Explicit G10 proc source mutants; never count crashes/timeouts as rejection.

Each mutant names the stage expected to catch it: `static` (the checker or the verifier rejects the
source) or `oracle` (the source is accepted, because it only mishandles what the child prints, and the
fixture grader reports wrong results). The two are reported separately (G10_PROC_SPEC Q gate 4).
"""
import sys
sys.dont_write_bytecode = True
import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
from pathlib import Path
import tempfile
import run as G

# pid, family, expected stage, old text, replacement. The old text must occur exactly once.
EXPLICIT = [
 ('q01', 'missing_consume', 'static', 'let r: Result<i64> = proc_wait(move p);\n  return -10 - e;', 'return -10 - e;'),
 ('q02', 'missing_consume', 'static', 'if !done { return kill_with(move p, -3); }', 'if !done { return -3; }'),
 ('q01', 'overwrite', 'static', 'let mut p: Process = proc_spawn(program, args, cwd);',
  'let mut p: Process = proc_spawn(program, args, cwd);\n  p = proc_spawn(program, proc_args_new(), cwd);'),
 ('q01', 'branch', 'static', 'if !proc_ok(p) { return fail_wait(move p); }',
  'if !proc_ok(p) { let z: i64 = fail_wait(move p); }'),
 ('q02', 'temporary', 'static', 'p = proc_close_stdin(move p);',
  'let t: i64 = proc_size(proc_close_stdin(move p), 1);'),
 ('q01', 'unchecked_io', 'static', 'if !proc_ok(p) { return fail_wait(move p); }', ''),
 ('q02', 'unchecked_io', 'static', 'used = used + 1;\n    if !proc_ok(p) { return kill_with(move p, -2); }', 'used = used + 1;'),
 ('q01', 'write_after_close', 'static', 'p = proc_read_all(move p);',
  'p = proc_write_str(move p, "late");\n  p = proc_read_all(move p);'),
 ('q01', 'range', 'static', 'proc_byte(p, stream, i)', 'proc_byte(p, stream, i + 1)'),
 ('q02', 'range', 'static', 'proc_byte(p, 1, i) == 10', 'proc_byte(p, 1, n) == 10'),
 ('q02', 'range', 'static', 'proc_read(move p, 1, 4096)', 'proc_read(move p, 3, 4096)'),
 ('q02', 'eof_misread', 'oracle', 'if n == 0 {', 'if n < 4096 {'),
 ('q02', 'eof_misread', 'oracle', 'if n == 0 {', 'if proc_eof(p, 2) {'),
 ('q01', 'read_all_duplication', 'oracle', 'let out: i64 = digest(p, 1);',
  'p = proc_read_all(move p);\n  if !proc_ok(p) { return fail_kill(move p); }\n  let out: i64 = digest(p, 1);'),
 ('q02', 'line_boundary', 'static', 'if cur > 0 {', 'if cur >= 0 {'),
 ('q02', 'line_boundary', 'oracle', 'if cur > longest { longest = cur; }\n          cur = 0;',
  'if cur > longest { longest = cur; }'),
]
CONTROLS = [
 ('q01', 'kill_after_failed_spawn', 'accepted', 'let r: Result<i64> = proc_wait(move p);\n  return -10 - e;',
  'let r: Result<i64> = proc_kill(move p);\n  return -10 - e;'),
 ('q02', 'smaller_chunks', 'accepted', 'proc_read(move p, 1, 4096)', 'proc_read(move p, 1, 1000)'),
]


def catalogue():
    items = []
    for index, (pid, family, stage, old, new) in enumerate(EXPLICIT + CONTROLS, 1):
        text = (G.HERE / 'solutions' / pid / 'root.pruv').read_text()
        assert text.count(old) == 1, (pid, family, old)
        source = text.replace(old, new)
        items.append(dict(id=f'm{index:03}', pid=pid, family=family, expected=stage, old=old, new=new,
                          source=source, intent='incorrect' if index <= len(EXPLICIT) else 'control',
                          sha256=hashlib.sha256(source.encode()).hexdigest()))
    assert len({x['sha256'] for x in items}) == len(items)
    return items


def classify(r):
    if r['status'] == 'ok':
        return 'accepted'
    if r['status'] == 'wrong':
        return 'wrong_but_proved'
    if r['status'] == 'pruv_error':
        if r['rc'] == -1:
            return 'timeout'
        errors = r.get('errors', [])
        if not errors or r['rc'] != 1:
            return 'compiler_crash'
        if any(e.get('stage') == 'internal' for e in errors):
            return 'compiler_crash'
        if any(e.get('code') in ('solver_unknown', 'solver_timeout', 'inference_timeout', 'verify_unknown') for e in errors):
            return 'unknown'
        if any(e.get('stage') == 'codegen' for e in errors):
            return 'unsupported'
        return 'rejected_verify' if all(e.get('stage') == 'verify' for e in errors) else 'rejected_check'
    return 'unresolved_' + r['status']


EXPECTED = {'static': ('rejected_check', 'rejected_verify'), 'oracle': ('wrong_but_proved',), 'accepted': ('accepted',)}


def measure(item, compiler, backends):
    with tempfile.TemporaryDirectory(prefix=item['id'] + '_', dir=G.WORK) as tmp:
        src = Path(tmp) / 'root.pruv'
        src.write_text(item['source'])
        rows = {}
        for backend in backends:
            r = G.grade_one(src, item['pid'], compiler, backend)
            rows[compiler + '-' + backend] = {'class': classify(r), 'grader': r}
        return {k: v for k, v in item.items() if k != 'source'} | {'runs': rows}


def main():
    p = argparse.ArgumentParser(__doc__)
    p.add_argument('out', type=Path, nargs='?')
    p.add_argument('--self-test', action='store_true')
    p.add_argument('--list', action='store_true')
    p.add_argument('--compiler', choices=['py', 'rs'], default='py')
    p.add_argument('--backend', choices=['rust', 'llvm', 'both'], default='both')
    p.add_argument('--jobs', type=int, default=8)
    a = p.parse_args()
    items = catalogue()
    if a.self_test:
        assert len(EXPLICIT) == 16
        for item in items:
            assert G.HEADERS[item['pid']] in ' '.join(item['source'].split())
        assert G.ref_scan('bin', 10) == 300000255 and G.ref_scan('blank', 10) == 300000000
        print('self-test: PASS;', len(EXPLICIT), 'explicit incorrect mutants;', len(CONTROLS), 'controls')
        return 0
    if a.list:
        for item in items:
            print(item['id'], item['pid'], item['intent'], item['expected'], item['family'])
        return 0
    G.child_exe()
    backends = ['rust', 'llvm'] if a.backend == 'both' else [a.backend]
    with ThreadPoolExecutor(max_workers=max(1, a.jobs)) as ex:
        rows = list(ex.map(lambda i: measure(i, a.compiler, backends), items))
    unexpected = [i['id'] for i in rows if any(r['class'] not in EXPECTED[i['expected']] for r in i['runs'].values())]
    summary = {'total': len(rows), 'explicit_incorrect': len(EXPLICIT), 'controls': len(CONTROLS),
               'by_expected': {s: dict(Counter(r['class'] for i in rows if i['expected'] == s for r in i['runs'].values()))
                               for s in EXPECTED},
               'unexpected': unexpected}
    out = a.out or G.HERE / 'results' / f'mutants_{a.compiler}_{a.backend}.json'
    out.write_text(json.dumps({'tools': G.G2.COMPILERS, 'summary': summary, 'items': rows}, indent=2))
    print(json.dumps(summary, indent=2))
    for i in rows:
        print(i['id'], i['pid'], i['family'], i['expected'], {k: v['class'] for k, v in i['runs'].items()})
    return int(bool(unexpected))


if __name__ == '__main__':
    sys.exit(main())
