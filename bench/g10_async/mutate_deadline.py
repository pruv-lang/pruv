"""Small G10-ASYNC-DEADLINE source mutant set over ev_proc_wait_for and ev_proc_write_for.

Contract mutants edit the verified proof programs tests/g10_async/proof/ev_proc_{wait,write}_for.pruv and are
compiled with the verifier on: expected stage `static` (the checker or the verifier rejects the source). Harness
mutants edit the runtime programs tests/runtime/g10a_r23_wait_for.pruv / g10a_r24_write_for.pruv (which carry no loop
invariants, so they are compiled with --no-verify) and run their drivers with the fixture child: expected stage
`oracle` (the driver exits non-zero). Controls (`accepted`) must verify, or compile and exit 0. Signal exits
(`crash`), timeouts and build failures never count as detection. The two catch kinds are summarised separately.
"""
import sys
sys.dont_write_bytecode = True
import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import os
from pathlib import Path
import tempfile
import run as G

WAIT_P = 'tests/g10_async/proof/ev_proc_wait_for.pruv'
WRITE_P = 'tests/g10_async/proof/ev_proc_write_for.pruv'
WAIT_R = 'tests/runtime/g10a_r23_wait_for.pruv'
WRITE_R = 'tests/runtime/g10a_r24_write_for.pruv'

# target, family, expected stage, old text, replacement. The old text must occur exactly once in the target.
EXPLICIT = [
 # contract catches
 (WAIT_P, 'wait_for_not_process', 'static', '  requires ev_kind(l, a) == 3\n  requires 0 <= b', '  requires 0 <= b'),
 (WAIT_P, 'wait_for_deadline_too_long', 'static', 'return ev_proc_wait_for(move l, a, 50);',
  'return ev_proc_wait_for(move l, a, 86400001);'),
 (WAIT_P, 'wait_after_free', 'static', '  if ev_kind(q, a) == 3 { return ev_proc_wait(move q, a); }\n  return move q;',
  '  return ev_proc_wait(move q, a);'),
 (WAIT_P, 'wait_for_claims_stdin_kept', 'static', '  ensures ev_write_open(result, a) == false',
  '  ensures ev_write_open(result, a) == ev_write_open(l, a)'),
 (WRITE_P, 'write_for_after_close', 'static', '  requires ev_write_open(l, a)\n  ensures ev_ok(result)\n  ensures ev_kind(result, a) == 3\n  ensures ev_eof',
  '  ensures ev_ok(result)\n  ensures ev_kind(result, a) == 3\n  ensures ev_eof'),
 (WRITE_P, 'close_after_half_close', 'static', '  if ev_write_open(q, a) { return ev_proc_close_stdin(move q, a); }\n  return move q;',
  '  return ev_proc_close_stdin(move q, a);'),
 (WRITE_P, 'write_for_claims_stdin_kept', 'static', '  ensures ev_count(result) == ev_count(l)\n',
  '  ensures ev_count(result) == ev_count(l)\n  ensures ev_write_open(result, a)\n'),
 (WRITE_P, 'write_for_negative_deadline', 'static', 'ev_proc_write_for(move l, a, data, 100)', 'ev_proc_write_for(move l, a, data, -1)'),
 # harness catches
 (WAIT_R, 'wait_past_exit', 'oracle', 'l = ev_proc_wait_for(move l, ps, 20);', 'l = ev_proc_wait_for(move l, ps, 2000);'),
 (WAIT_R, 'kept_slot_not_removed', 'oracle', '        l = ev_remove(move l, ps);\n', ''),
 (WAIT_R, 'exit_read_too_early', 'oracle', 'if ev_ok(l) && ev_kind(l, ps) == 0 && ev_count(l) == 0 { out = ev_exit(l); }',
  'if ev_ok(l) && ev_count(l) == 0 { out = e0; }'),
 (WRITE_R, 'unbounded_write', 'oracle', 'l = ev_proc_write_for(move l, ps, data, ms);', 'l = ev_proc_write(move l, ps, data);'),
 (WRITE_R, 'stdin_never_closed', 'oracle', '    l = ev_proc_write(move l, ps, tail);\n    l = ev_proc_close_stdin(move l, ps);\n',
  '    l = ev_proc_write(move l, ps, tail);\n'),
 (WRITE_R, 'empty_write_closes', 'oracle', '  if mode == 4 && !half {\n    l = ev_proc_close_stdin(move l, ps);\n  }\n', ''),
 (WRITE_R, 'half_close_ignored', 'oracle', 'let half: bool = !ev_write_open(l, ps);', 'let half: bool = false;'),
]
CONTROLS = [
 (WAIT_P, 'longer_wait', 'accepted', 'return ev_proc_wait_for(move l, a, 50);', 'return ev_proc_wait_for(move l, a, 86400000);'),
 (WRITE_P, 'zero_deadline', 'accepted', 'ev_proc_write_for(move l, a, data, 100)', 'ev_proc_write_for(move l, a, data, 0)'),
 (WAIT_R, 'failed_slot_any_deadline', 'accepted', 'l = ev_proc_wait_for(move l, ps, 86400000);', 'l = ev_proc_wait_for(move l, ps, 0);'),
 (WRITE_R, 'tail_bounded_write', 'accepted', 'l = ev_proc_write(move l, ps, tail);', 'l = ev_proc_write_for(move l, ps, tail, 20000);'),
]
EXPECTED = {'static': ('rejected_check', 'rejected_verify'), 'oracle': ('wrong_but_proved',), 'accepted': ('accepted',)}


def catalogue():
    items = []
    for index, (target, family, stage, old, new) in enumerate(EXPLICIT + CONTROLS, 1):
        text = (G.ROOT / target).read_text()
        assert text.count(old) == 1, (family, old, text.count(old))
        source = text.replace(old, new)
        items.append(dict(id=f'd{index:02}', target=target, family=family, expected=stage, old=old, new=new,
                          source=source, intent='incorrect' if index <= len(EXPLICIT) else 'control',
                          catch='contract' if target.startswith('tests/g10_async/proof') else 'harness',
                          sha256=hashlib.sha256(source.encode()).hexdigest()))
    assert len({x['sha256'] for x in items}) == len(items)
    return items


def diagnostics(stdout):
    out = []
    for line in stdout.splitlines():
        try:
            out.append(json.loads(line))
        except ValueError:
            pass
    return out


def grade(src, wd, compiler, backend, child, driver):
    gen = wd / 'gen.rs'
    flags = ['--no-verify'] if driver else []
    if backend == 'llvm':
        ll = wd / 'gen.ll'
        cmd = G.G2.COMPILERS[compiler] + [src, '-o', ll, '--backend', 'llvm', '--rust-bindings', gen] + flags
    else:
        ll = None
        cmd = G.G2.COMPILERS[compiler] + [src, '-o', gen, '--backend', 'rust'] + flags
    r = G.run(cmd, timeout=600)
    if r['rc'] == -1:
        return 'timeout', r
    if r['rc'] != 0:
        diags = diagnostics(r['stdout'])
        if r['rc'] != 1 or not diags or any(d.get('stage') == 'internal' for d in diags):
            return 'compiler_crash', r
        r = dict(r, codes=sorted({'%s/%s' % (d.get('stage'), d.get('code')) for d in diags}))
        return ('rejected_verify' if all(d.get('stage') == 'verify' for d in diags) else 'rejected_check'), r
    if not driver:
        return 'accepted', {'rc': 0}
    exe, error = G.build({'rs': str(gen), 'll': str(ll) if ll else None}, wd, driver.read_text())
    if error:
        return 'build_failure', {'stderr': error[-800:]}
    run_dir = wd / 'run'
    run_dir.mkdir()
    x = G.run([exe], timeout=300, env=dict(os.environ, PRUV_PROC_CHILD=str(child)), cwd=run_dir)
    if x['rc'] in (-1, -2):
        return 'timeout', x
    detail = {'rc': x['rc'], 'stderr': x['stderr'][-400:]}
    if x['rc'] < 0:
        return 'crash', detail
    return ('accepted' if x['rc'] == 0 else 'wrong_but_proved'), detail


def test_child():
    exe = G.WORK / 'test_child'
    src = G.ROOT / 'tests/g10_proc/child.rs'
    if not exe.exists() or exe.stat().st_mtime < src.stat().st_mtime:
        r = G.run([G.G2.RUSTC, '-O', '--edition=2021', src, '-o', exe])
        if r['rc']:
            raise SystemExit('test child build failed: ' + r['stderr'][-800:])
    return exe


def measure(item, compiler, backends, child):
    rows = {}
    driver = (G.ROOT / item['target']).with_suffix('.rs') if item['catch'] == 'harness' else None
    for backend in backends:
        with tempfile.TemporaryDirectory(prefix=item['id'] + '_', dir=G.WORK) as tmp:
            wd = Path(tmp)
            src = wd / Path(item['target']).name
            src.write_text(item['source'])
            cls, detail = grade(src, wd, compiler, backend, child, driver)
            rows[compiler + '-' + backend] = {'class': cls, 'detail': {k: v for k, v in detail.items() if k != 'stdout'}}
    return {k: v for k, v in item.items() if k != 'source'} | {'runs': rows}


def main():
    p = argparse.ArgumentParser(__doc__)
    p.add_argument('out', type=Path, nargs='?')
    p.add_argument('--compiler', choices=['py', 'rs'], default='py')
    p.add_argument('--backend', choices=['rust', 'llvm', 'both'], default='both')
    p.add_argument('--jobs', type=int, default=4)
    p.add_argument('--list', action='store_true')
    a = p.parse_args()
    items = catalogue()
    if a.list:
        for item in items:
            print(item['id'], item['catch'], item['intent'], item['expected'], item['family'])
        return 0
    child = test_child()
    backends = ['rust', 'llvm'] if a.backend == 'both' else [a.backend]
    with ThreadPoolExecutor(max_workers=max(1, a.jobs)) as ex:
        rows = list(ex.map(lambda i: measure(i, a.compiler, backends, child), items))
    unexpected = [i['id'] for i in rows if any(r['class'] not in EXPECTED[i['expected']] for r in i['runs'].values())]
    summary = {'total': len(rows), 'explicit_incorrect': len(EXPLICIT), 'controls': len(CONTROLS),
               'by_catch': {c: {s: dict(Counter(r['class'] for i in rows if i['catch'] == c and i['expected'] == s
                                                for r in i['runs'].values()))
                                for s in EXPECTED if any(i['catch'] == c and i['expected'] == s for i in rows)}
                            for c in ('contract', 'harness')},
               'unexpected': unexpected}
    out = a.out or G.HERE / 'results' / f'mutants_deadline_{a.compiler}_{a.backend}.json'
    tmp = out.with_suffix('.tmp')
    tmp.write_text(json.dumps({'tools': {k: [Path(c).name for c in v] for k, v in G.G2.COMPILERS.items()},
                               'summary': summary, 'items': rows}, indent=2))
    os.replace(tmp, out)
    print(json.dumps(summary, indent=2))
    for i in rows:
        print(i['id'], i['catch'], i['family'], i['expected'], {k: v['class'] for k, v in i['runs'].items()})
    return int(bool(unexpected))


if __name__ == '__main__':
    sys.exit(main())
