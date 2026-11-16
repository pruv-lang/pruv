"""Small G10-PROC-ENV source mutant set on tests/runtime/g10p_r22_env.pruv (driver g10p_r22_env.rs).

Each mutant names the stage expected to catch it: `static` (the checker or the verifier rejects the source), `oracle`
(the source compiles, the driver run with the fixture child exits non-zero) or `accepted` (a control: compiles and the
driver exits 0). Signal exits (`crash`), timeouts and build failures never count as detection. Runs on both backends.
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

SRC = G.ROOT / 'tests/runtime/g10p_r22_env.pruv'
DRIVER = G.ROOT / 'tests/runtime/g10p_r22_env.rs'

# family, expected stage, old text, replacement. The old text must occur exactly once.
EXPLICIT = [
 ('missing_consume', 'static', 'let r: Result<i64> = proc_wait(move p);\n  if ok && !open',
  'let r: Result<i64> = Ok(0);\n  if ok && !open'),
 ('unchecked_spawn', 'static', '  if !proc_ok(p) { return abort_with(move p, 100); }\n', ''),
 ('swapped_lists', 'oracle', 'let mut p: Process = proc_spawn_env(program, args, "", names, values)',
  'let mut p: Process = proc_spawn_env(program, args, "", values, names)'),
 ('same_list', 'oracle', 'let mut p: Process = proc_spawn_env(program, args, "", names, values)',
  'let mut p: Process = proc_spawn_env(program, args, "", names, names)'),
 ('dropped_pairs', 'oracle', 'let p: Process = proc_spawn_env(program, args, "", names, values);',
  'let p: Process = proc_spawn(program, args, "");'),
 ('lost_pair', 'oracle', '  if k >= 3 { l = proc_args_push(move l, c); }\n', ''),
 ('reordered_pairs', 'oracle', 'let names: ProcessArgs = pick(kn, n1, n2, n3);',
  'let names: ProcessArgs = pick(kn, n1, n3, n2);'),
]
CONTROLS = [
 ('kill_instead_of_wait', 'accepted', 'if !proc_ok(p) { return abort_with(move p, 100); }',
  'if !proc_ok(p) { let r: Result<i64> = proc_kill(move p); return 100; }'),
]
EXPECTED = {'static': ('rejected_check', 'rejected_verify'), 'oracle': ('wrong_but_proved',), 'accepted': ('accepted',)}


def catalogue():
    text = SRC.read_text()
    items = []
    for index, (family, stage, old, new) in enumerate(EXPLICIT + CONTROLS, 1):
        assert text.count(old) == 1, (family, old, text.count(old))
        source = text.replace(old, new)
        items.append(dict(id=f'e{index:02}', family=family, expected=stage, old=old, new=new, source=source,
                          intent='incorrect' if index <= len(EXPLICIT) else 'control',
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


def grade(src, wd, compiler, backend, child):
    gen = wd / 'gen.rs'
    if backend == 'llvm':
        ll = wd / 'gen.ll'
        cmd = G.G2.COMPILERS[compiler] + [src, '-o', ll, '--backend', 'llvm', '--rust-bindings', gen]
    else:
        ll = None
        cmd = G.G2.COMPILERS[compiler] + [src, '-o', gen, '--backend', 'rust']
    r = G.run(cmd, timeout=600)
    if r['rc'] == -1:
        return 'timeout', r
    if r['rc'] != 0:
        diags = diagnostics(r['stdout'])
        if r['rc'] != 1 or not diags or any(d.get('stage') == 'internal' for d in diags):
            return 'compiler_crash', r
        r = dict(r, codes=sorted({'%s/%s' % (d.get('stage'), d.get('code')) for d in diags}))
        return ('rejected_verify' if all(d.get('stage') == 'verify' for d in diags) else 'rejected_check'), r
    exe, error = G.build({'rs': str(gen), 'll': str(ll) if ll else None}, wd, DRIVER.read_text())
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
    for backend in backends:
        with tempfile.TemporaryDirectory(prefix=item['id'] + '_', dir=G.WORK) as tmp:
            wd = Path(tmp)
            src = wd / 'g10p_r22_env.pruv'
            src.write_text(item['source'])
            cls, detail = grade(src, wd, compiler, backend, child)
            rows[compiler + '-' + backend] = {'class': cls, 'detail': {k: v for k, v in detail.items() if k != 'stdout'}}
    return {k: v for k, v in item.items() if k != 'source'} | {'runs': rows}


def main():
    p = argparse.ArgumentParser(__doc__)
    p.add_argument('out', type=Path, nargs='?')
    p.add_argument('--compiler', choices=['py', 'rs'], default='py')
    p.add_argument('--backend', choices=['rust', 'llvm', 'both'], default='both')
    p.add_argument('--jobs', type=int, default=8)
    p.add_argument('--list', action='store_true')
    a = p.parse_args()
    items = catalogue()
    if a.list:
        for item in items:
            print(item['id'], item['intent'], item['expected'], item['family'])
        return 0
    child = test_child()
    backends = ['rust', 'llvm'] if a.backend == 'both' else [a.backend]
    with ThreadPoolExecutor(max_workers=max(1, a.jobs)) as ex:
        rows = list(ex.map(lambda i: measure(i, a.compiler, backends, child), items))
    unexpected = [i['id'] for i in rows if any(r['class'] not in EXPECTED[i['expected']] for r in i['runs'].values())]
    summary = {'total': len(rows), 'explicit_incorrect': len(EXPLICIT), 'controls': len(CONTROLS),
               'by_expected': {s: dict(Counter(r['class'] for i in rows if i['expected'] == s for r in i['runs'].values()))
                               for s in EXPECTED},
               'unexpected': unexpected}
    out = a.out or G.HERE / 'results' / f'mutants_env_{a.compiler}_{a.backend}.json'
    tmp = out.with_suffix('.tmp')
    tmp.write_text(json.dumps({'tools': {k: [Path(c).name for c in v] for k, v in G.G2.COMPILERS.items()},
                               'summary': summary, 'items': rows}, indent=2))
    os.replace(tmp, out)
    print(json.dumps(summary, indent=2))
    for i in rows:
        print(i['id'], i['family'], i['expected'], {k: v['class'] for k, v in i['runs'].items()})
    return int(bool(unexpected))


if __name__ == '__main__':
    sys.exit(main())
