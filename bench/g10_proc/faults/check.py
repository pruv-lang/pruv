"""Six isolated G10 proc compiler faults, each applied alone to a disposable copy of `pruvc/`.

The source compiler is never edited. A fault counts as detected when a named probe that passes on the clean copy fails
on the faulty copy. Probes: `reject` (a tests/must_reject row, matched by its stage/code JSON oracle), `local_reject` /
`local_accept` (a small program in this folder that must be rejected / verified), and `runtime` (a tests/runtime g10p
driver built on the LLVM backend, whose Rust shim carries the fault, run with the fixture child; exit 0 expected).
Timeouts and startup failures never count as detection. Copies and outputs stay under bench/g10_proc/.work.
"""
import sys
sys.dont_write_bytecode = True
sys.path.insert(0, str(__import__('pathlib').Path(__file__).resolve().parents[1]))
import argparse
import difflib
import hashlib
import json
import os
from pathlib import Path
import shutil
import tempfile
import run as G

FAULTS = [
 ('f1_must_consume', 'foreign.py',
  [('            for n in st:       # every owned handle leaving', '            for n in ():       # FAULT: every owned handle leaving')],
  [('reject', 'g10p_early_return')]),
 ('f2_catalog_requires', 'verify.py',
  [('            need(_proc_stream_ok(args[1]))\n            need(z3.And(args[2] > 0',
    '            # FAULT: no stream requires\n            need(z3.And(args[2] > 0')],
  [('reject', 'g10p_v_bad_stream')]),
 ('f3_snapshot_keep', 'verify.py',
  [('z3.Implies(z3.And(ok, sm == 1), _proc_keep_err(res, p))', 'z3.Implies(z3.And(ok, sm == 1), _proc_keep_out(res, p))'),
   ('z3.Implies(z3.And(ok, sm == 2), _proc_keep_out(res, p))', 'z3.Implies(z3.And(ok, sm == 2), _proc_keep_err(res, p))')],
  [('local_accept', 'p_keep_other'), ('local_reject', 'p_keep_read')]),
 ('f4_result_tag', 'foreign_shim.py',
  [('{ *pruv_t = 1; *pruv_v = pruv_x; } "\n                   "::std::result::Result::Err(pruv_e) => { *pruv_t = 0; *pruv_v = pruv_e; } } } })" % (\n                       _proc_consume("h", P)',
    '{ *pruv_t = 0; *pruv_v = pruv_x; } "\n                   "::std::result::Result::Err(pruv_e) => { *pruv_t = 1; *pruv_v = pruv_e; } } } })" % (\n                       _proc_consume("h", P)')],
  [('runtime', 'g10p_r10_exit_kill')]),
 # an edit is (old, new) and must match exactly once, or (old, new, N) and must match exactly N times
 ('f5_env_ignored', 'codegen.py',
  [('        cmd.env(k, v);', '        let _ = (k, v); // FAULT: env ignored', 2)],
  [('runtime', 'g10p_r22_env')]),
 ('f6_env_validation', 'codegen.py',
  [("names.iter().any(|k| k.is_empty() || k.contains('=') || k.contains('\\0'))",
    "names.iter().any(|k| k.is_empty() || k.contains('\\0'))", 2)],
  [('runtime', 'g10p_r22_env')]),
]
LOCAL_EXPECTED = {'stage': 'verify', 'code': 'assert'}


def diagnostics(stdout):
    out = []
    for line in stdout.splitlines():
        try:
            out.append(json.loads(line))
        except ValueError:
            pass
    return out


def test_child():
    # the tests/runtime drivers expect the A2 fixture child, not the bench child
    exe = G.WORK / 'test_child'
    src = G.ROOT / 'tests/g10_proc/child.rs'
    if not exe.exists() or exe.stat().st_mtime < src.stat().st_mtime:
        r = G.run([G.G2.RUSTC, '-O', '--edition=2021', src, '-o', exe])
        if r['rc']:
            raise SystemExit('test child build failed: ' + r['stderr'][-800:])
    return exe


def probe(compiler, kind, name, wd):
    py = G.G2.COMPILERS['py'][0]
    if kind == 'runtime':
        src = G.ROOT / 'tests/runtime' / (name + '.pruv')
        ll, rs = wd / (name + '.ll'), wd / (name + '_gen.rs')
        cmd = [py, compiler, src, '-o', ll, '--backend', 'llvm', '--rust-bindings', rs]
        r = G.run(cmd, timeout=600)
        row = dict(r, command=[str(x) for x in cmd])
        if r['rc']:
            return dict(row, passed=False, reason='compile', internal_error=False, started=False)
        exe, error = G.build({'rs': str(rs), 'll': str(ll)}, wd, (G.ROOT / 'tests/runtime' / (name + '.rs')).read_text())
        if error:
            return dict(row, passed=False, reason='build: ' + error[-400:], internal_error=False, started=False)
        run_dir = wd / ('run_' + name)
        run_dir.mkdir()
        env = dict(os.environ, PRUV_PROC_CHILD=str(test_child()))
        x = G.run([exe], timeout=300, env=env, cwd=run_dir)
        return dict(row, run=x, passed=x['rc'] == 0, reason='exit %d' % x['rc'], internal_error=False,
                    started=x['rc'] > 0)
    if kind == 'reject':
        src = G.ROOT / 'tests/must_reject' / (name + '.pruv')
        expected = json.loads(src.with_suffix('.json').read_text())
    else:
        src = G.HERE / 'faults' / (name + '.pruv')
        expected = LOCAL_EXPECTED if kind == 'local_reject' else None
    cmd = [py, compiler, src, '--backend', 'rust', '-o', wd / (name + '.rs')]
    r = G.run(cmd, timeout=600)
    diags = diagnostics(r['stdout'])
    if expected is None:
        passed = r['rc'] == 0
    else:
        passed = r['rc'] == 1 and any(all(d.get(k) == expected[k] for k in ('stage', 'code')) for d in diags)
    return dict(r, command=[str(x) for x in cmd], expected=expected, passed=passed,
                internal_error=any(d.get('stage') == 'internal' for d in diags), started=r['rc'] not in (-1, -2))


def main():
    p = argparse.ArgumentParser(__doc__)
    p.add_argument('--generate-only', action='store_true')
    a = p.parse_args()
    G.WORK.mkdir(exist_ok=True)
    rows = []
    for name, filename, edits, probes in FAULTS:
        original = (G.ROOT / 'pruvc' / filename).read_text()
        altered = original
        for old, new, *times in edits:
            assert altered.count(old) == (times[0] if times else 1), (name, old, altered.count(old))
            altered = altered.replace(old, new)
        patch = ''.join(difflib.unified_diff(original.splitlines(True), altered.splitlines(True),
                        fromfile='a/pruvc/' + filename, tofile='b/pruvc/' + filename))
        path = G.HERE / 'faults' / (name + '.diff')
        path.write_text(patch)
        row = {'fault': name, 'patch': path.name, 'sha256': hashlib.sha256(patch.encode()).hexdigest(),
               'source_sha256': hashlib.sha256(original.encode()).hexdigest(), 'probes': {}}
        if a.generate_only:
            row['status'] = 'not_run'
        else:
            with tempfile.TemporaryDirectory(prefix=name + '_', dir=G.WORK) as tmp:
                wd = Path(tmp)
                shutil.copytree(G.ROOT / 'pruvc', wd / 'pruvc', ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
                compiler = wd / 'pruvc/pruvc.py'
                (wd / 'clean').mkdir()
                (wd / 'faulty').mkdir()
                baseline = {n: probe(compiler, k, n, wd / 'clean') for k, n in probes}
                applied = G.run(['patch', '--batch', '-p1', '-d', wd, '-i', path])
                row['patch_apply'] = applied
                if applied['rc']:
                    row['status'] = 'patch_error'
                else:
                    for kind, n in probes:
                        faulty = probe(compiler, kind, n, wd / 'faulty')
                        row['probes'][n] = {'kind': kind, 'baseline': baseline[n], 'faulty': faulty,
                                            'detected': baseline[n]['passed'] and not faulty['passed']
                                                        and faulty.get('started', True)}
                    row['status'] = 'detected' if any(t['detected'] for t in row['probes'].values()) else 'NOT_DETECTED'
        print(name, row['status'], {n: t['detected'] for n, t in row['probes'].items()}, flush=True)
        rows.append(row)
    if not a.generate_only:
        (G.HERE / 'results/faults.json').write_text(json.dumps({'rows': rows}, indent=2))
    table = ['# G10 proc compiler fault injections', '',
             'Each patch is applied alone with `patch -p1` to a disposable copy of the current `pruvc/` under',
             '`bench/g10_proc/.work`; the source compiler is never patched. Run `python3 bench/g10_proc/faults/check.py`.',
             'A fault is detected when a named probe passes on the clean copy and fails on the faulty one (a runtime',
             'probe only counts when the faulty driver ran and exited with a positive code; compile or build failures,',
             'timeouts and signals do not).', '',
             '| Fault | Patch SHA256 prefix | Probe(s) | Result |', '|---|---|---|---|']
    for row, (_, _, _, probes) in zip(rows, FAULTS):
        table.append('| ' + ' | '.join([row['patch'], row['sha256'][:16],
                                        ', '.join('%s `%s`' % (k, n) for k, n in probes), row['status']]) + ' |')
    table += ['', 'Probe kinds: `reject` is a tests/must_reject row (its stage/code JSON); `local_accept` / `local_reject`',
              'are the programs in this folder (verified / rejected with an `assert` diagnostic); `runtime` builds the',
              'tests/runtime driver on the LLVM backend (the Rust shim carries the Result tag) and expects exit 0.',
              'The faults: f1 drops the must-consume check at `return`; f2 drops the `stream` 1-or-2 requires of',
              '`proc_read`; f3 keeps the read stream instead of the other one; f4 swaps the Ok/Err tag of',
              '`proc_wait`/`proc_kill` in the shim; f5 drops the env pairs in `proc_spawn_env`; f6 stops rejecting',
              '`=` in env names. Full commands and outputs are in `../results/faults.json`.', '']
    (G.HERE / 'faults/README.md').write_text('\n'.join(table))
    return int(not a.generate_only and any(r['status'] != 'detected' for r in rows))


if __name__ == '__main__':
    sys.exit(main())
