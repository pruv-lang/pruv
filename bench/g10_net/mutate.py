"""Explicit G10 net source mutants; never count crashes/timeouts as rejection.

Each mutant names the stage expected to catch it: `static` (the checker or the verifier rejects the
source) or `oracle` (the source is accepted, because it only mishandles what the peer sends, and the
driver reports wrong results). The two are reported separately (G10_NET_SPEC Q gate 4).
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
 ('q01', 'missing_close', 'static', 'if plen > 1000000 { return close_with(move s, -5); }', 'if plen > 1000000 { return -5; }'),
 ('q02', 'missing_close', 'static', 'let r0: Result<i64> = net_listener_close(move l);\n    return -1;', 'return -1;'),
 ('q01', 'overwrite', 'static', 'let mut s: Socket = net_connect(host, port);',
  'let mut s: Socket = net_connect(host, port);\n  s = net_connect(host, port);'),
 ('q01', 'branch', 'static', 'if !net_ok(s) { return close_with(move s, -1); }',
  'if !net_ok(s) { let z: i64 = close_with(move s, -1); }'),
 ('q02', 'temporary', 'static', 'c = net_write_bytes(move c, msg);',
  'let t: i64 = net_size(net_write_bytes(move c, msg));'),
 ('q01', 'unchecked_io', 'static', 's = net_set_timeout(move s, 2000);\n  if !net_ok(s) { return close_with(move s, -2); }',
  's = net_set_timeout(move s, 2000);'),
 # (dropping the read check in q02 is equivalent: a failed read has size 0 and takes the EOF branch)
 ('q02', 'unchecked_io', 'static', 'let mut c: Socket = net_connect(host, ctl);\n  if !net_ok(c) {\n'
  '    let r1: Result<i64> = net_close(move c);\n    let r2: Result<i64> = net_listener_close(move l);\n    return -2;\n  }',
  'let mut c: Socket = net_connect(host, ctl);'),
 ('q02', 'write_after_shutdown', 'static', 's = net_write_str(move s, "!\\n");',
  's = net_shutdown_write(move s);\n    s = net_write_str(move s, "!\\n");'),
 ('q01', 'range', 'static', 'hb[got + i] = net_byte(s, i);', 'hb[got + i] = net_byte(s, i + 1);'),
 ('q02', 'range', 'static', 'line[have + j] = net_byte(s, j);', 'line[have + j + 1] = net_byte(s, j);'),
 ('q02', 'eof_misread', 'oracle', 'if m == 0 { return done(move s, 0); }', 'if m < 1025 - have { return done(move s, 0); }'),
 ('q01', 'eof_misread', 'oracle', 's = net_read(move s, 4 - got);\n    if !net_ok(s) { return close_with(move s, -3); }\n'
  '    let m: i64 = net_size(s);\n    if m == 0 {',
  's = net_read(move s, 4 - got);\n    if !net_ok(s) { return close_with(move s, -3); }\n'
  '    let m: i64 = net_size(s);\n    if m < 4 - got {'),
 ('q01', 'read_all_duplication', 'oracle', 'if plen > 1000000 { return close_with(move s, -5); }',
  'if plen > 1000000 { return close_with(move s, -5); }\n  s = net_read_all(move s);\n'
  '  if !net_ok(s) { return close_with(move s, -3); }'),
 ('q01', 'framing', 'oracle', 'let plen: i64 = load_be(hb, 0, 4);', 'let plen: i64 = load_be(hb, 0, 2);'),
 ('q02', 'framing', 'static', 'let mut out: [u8] = alloc(n + 1, 0);', 'let mut out: [u8] = alloc(n, 0);'),
 ('q02', 'line_boundary', 'oracle', 'net_byte(s, k) != 10', 'net_byte(s, k) != 13'),
]
CONTROLS = [
 ('q01', 'smaller_chunks', 'accepted', 'if mx > 4096 { mx = 4096; }', 'if mx > 1000 { mx = 1000; }'),
 ('q02', 'longer_timeout', 'accepted', 's = net_set_timeout(move s, 2000);', 's = net_set_timeout(move s, 5000);'),
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
    p.add_argument('--jobs', type=int, default=4)
    a = p.parse_args()
    items = catalogue()
    if a.self_test:
        assert len(EXPLICIT) == 16
        for item in items:
            assert G.HEADERS[item['pid']] in ' '.join(item['source'].split())
        assert G.ref_call(b'') == 1 and G.ref_call(b'a') == 1000000 + (256 + 98) % 999983
        print('self-test: PASS;', len(EXPLICIT), 'explicit incorrect mutants;', len(CONTROLS), 'controls')
        return 0
    if a.list:
        for item in items:
            print(item['id'], item['pid'], item['intent'], item['expected'], item['family'])
        return 0
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
