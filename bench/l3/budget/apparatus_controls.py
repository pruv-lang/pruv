"""Apparatus self-test (P02 substep 2): proves the runners cannot report a Pruv success they did not earn. Needs no Pruv build,
only python3 and the Node oracle the runners already use.
  python3 apparatus_controls.py
Controls:
  (a) an implementation path containing a space runs as one argv element (as `--impl PYTHON SCRIPT ...` and, when the interpreter path
      allows a shebang, as a directly executed script)
  (b) q.py without an implementation prints "RESULT oracle-only" and exits 3
  (c) a fake implementation that prints correct lines and then exits non-zero is a failure (exit 1)
  (d) a fake implementation that prints an extra line is a failure (exit 1)
  (e) perf_budget.py without an implementation reports "unavailable" and exits 3
  (f) metadata keys are present and counters that were not read are the string "unavailable", never 0
  (+) positive control: the faithful fake passes, so (c)/(d) fail because of the injected fault and not because the fake is broken
Exit 0 when every control holds, 1 otherwise."""
import sys
sys.dont_write_bytecode = True
import json
import os
import stat
import subprocess
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import q as q_mod  # noqa: E402
import perf_budget  # noqa: E402
from run_meta import UNAVAILABLE, impl_argv, num  # noqa: E402

COUNT = 8  # small: the oracle runs a handful of cases, the whole script takes seconds
META_KEYS = ('pruvc', 'backend', 'rustc', 'flags', 'seed', 'profile', 'lanes')

FAKE = '''import sys
replay, mode = sys.argv[1], sys.argv[2]
n = sum(1 for ln in sys.stdin if ln.strip())
lines = [ln for ln in open(replay, encoding='utf-8').read().split('\\n') if ln.strip()][:n]
for ln in lines:
    print(ln)
if mode == 'extra':
    print(lines[-1])
sys.stdout.flush()
if mode == 'exit1':
    sys.exit(1)
'''

results = []


def check(name, ok, detail=''):
    results.append((name, bool(ok)))
    print(('PASS ' if ok else 'FAIL ') + name + ((' -- ' + detail) if detail and not ok else ''), flush=True)


def run(argv):
    p = subprocess.run(argv, capture_output=True, text=True, encoding='utf-8', errors='replace', cwd=HERE, timeout=600)
    return p.returncode, p.stdout + p.stderr


def tail(out, n=3):
    return ' | '.join(out.strip().splitlines()[-n:])


def main():
    py = sys.executable
    q = str(HERE / 'q.py')
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        spaced = tmp / 'a b'
        spaced.mkdir()
        fake = spaced / 'fake impl.py'
        fake.write_text(FAKE, encoding='utf-8')
        fake.chmod(fake.stat().st_mode | stat.S_IXUSR)

        # precomputed oracle output for exactly the cases q.py --count COUNT will generate
        fams = q_mod.families_arg('1,2,3,4,5,6,7,8', None)
        cases = q_mod.generate(q_mod.DEFAULT_SEED, fams, 250, COUNT)
        replay = spaced / 'replay file.jsonl'
        replay.write_text(''.join(json.dumps(o, ensure_ascii=True) + '\n' for o in q_mod.run_batches(cases, 500)), encoding='utf-8')
        base = [py, q, '--count', str(COUNT), '--profile', 'S']

        def fake_run(mode, extra=()):
            return run(base + ['--impl', py, str(fake), str(replay), mode] + list(extra))

        # (+) positive control and (a) spaced path as one argv element
        rc, out = fake_run('ok')
        check('(+) faithful fake passes: exit 0 and RESULT pass', rc == 0 and 'RESULT pass' in out, f'rc={rc} {tail(out)}')
        check('(a1) spaced script path and spaced replay path run as single argv elements (--impl PATH ARGS...)',
              rc == 0 and f'({COUNT} compared)' in out, f'rc={rc} {tail(out)}')

        # (a2) one existing spaced path executed directly (run_meta.impl_argv keeps it one element); needs a shebang
        check('(a0) impl_argv keeps an existing spaced path as one element', impl_argv(str(fake)) == [str(fake)])
        check('(a0) impl_argv passes a list through unchanged', impl_argv([str(fake), 'x y']) == [str(fake), 'x y'])
        if ' ' not in py and os.name == 'posix':
            fake.write_text('#!' + py + '\n' + FAKE, encoding='utf-8')
            rc, out = run(base + ['--impl', str(fake), '--impl-arg', str(replay), '--impl-arg', 'ok'])
            check('(a2) directly executed spaced script (--impl PATH --impl-arg ...) passes', rc == 0 and 'RESULT pass' in out, f'rc={rc} {tail(out)}')
        else:
            print('SKIP (a2) interpreter path has a space or platform is not posix: no shebang possible')

        # (b) no implementation
        jpath = tmp / 'q summary.json'
        rc, out = run(base + ['--json', str(jpath)])
        check('(b) q.py without --impl exits 3', rc == 3, f'rc={rc} {tail(out)}')
        check('(b) q.py without --impl prints a line starting "RESULT oracle-only"',
              any(ln.startswith('RESULT oracle-only') for ln in out.splitlines()), tail(out))
        check('(b) the oracle-only summary is not a pass', json.loads(jpath.read_text(encoding='utf-8')).get('result') == 'oracle-only')

        # (c) non-zero exit after correct lines
        rc, out = fake_run('exit1')
        check('(c) implementation exiting non-zero after correct output is a failure (exit 1, RESULT fail)',
              rc == 1 and 'RESULT fail' in out and 'RESULT pass' not in out, f'rc={rc} {tail(out)}')

        # (d) extra output line
        rc, out = fake_run('extra')
        check('(d) implementation printing an extra line is a failure (exit 1, RESULT fail)',
              rc == 1 and 'RESULT fail' in out and 'RESULT pass' not in out, f'rc={rc} {tail(out)}')

        # (e) perf_budget without implementation
        pjson = tmp / 'perf.json'
        rc, out = run([py, str(HERE / 'perf_budget.py'), '--axes', 'sessions', '--sizes', '10', '--repeat', '1', '--json', str(pjson)])
        check('(e) perf_budget without --impl exits 3', rc == 3, f'rc={rc} {tail(out)}')
        check('(e) perf_budget without --impl reports "unavailable" and no "not implemented" zero column',
              UNAVAILABLE in out and 'RESULT unavailable' in out, tail(out))
        pdata = json.loads(pjson.read_text(encoding='utf-8')) if pjson.exists() else {}
        rows = [r for rs in pdata.get('axes', {}).values() for r in rs]
        check('(e) every row marks the implementation column unavailable',
              bool(rows) and all(r.get('impl') == UNAVAILABLE and r.get('implNsPerCall') == UNAVAILABLE for r in rows))

        # (f) metadata
        qmeta = json.loads(jpath.read_text(encoding='utf-8')).get('meta', {})
        check('(f) q.py summary carries meta keys', all(k in qmeta for k in META_KEYS), str(sorted(qmeta)))
        check('(f) q.py meta: pruvc unavailable without --pruvc, profile S, lane native, seed recorded',
              qmeta.get('pruvc') == UNAVAILABLE and qmeta.get('profile') == 'S' and qmeta.get('lanes') == ['native']
              and qmeta.get('seed') == q_mod.DEFAULT_SEED, str(qmeta))
        pmeta = pdata.get('meta', {})
        check('(f) perf_budget JSON carries meta keys', all(k in pmeta for k in META_KEYS), str(sorted(pmeta)))
        check('(f) num(None) is "unavailable", num(0) stays 0', num(None) == UNAVAILABLE and num(0) == 0)

        # (f) missing counters from the oracle become "unavailable", not 0 (patched oracle that reports no counters)
        real = perf_budget.run_oracle
        perf_budget.run_oracle = lambda cases, **kw: [{'driver': None} for _ in cases]
        try:
            pj2 = tmp / 'perf2.json'
            saved = sys.stdout
            with open(os.devnull, 'w') as sink:
                sys.stdout = sink
                try:
                    rc2 = perf_budget.main(['--axes', 'sessions', '--sizes', '10', '--repeat', '1', '--json', str(pj2)])
                finally:
                    sys.stdout = saved
            d2 = json.loads(pj2.read_text(encoding='utf-8'))
            r0 = d2['axes']['sessions'][0]
            check('(f) missing calls/coreNs/fireNs are "unavailable", not 0',
                  all(r0[k] == UNAVAILABLE for k in ('calls', 'coreNs', 'fireNs', 'nsPerCall')) and rc2 == 3, str(r0))
        finally:
            perf_budget.run_oracle = real

    failed = [n for n, ok in results if not ok]
    print(f'apparatus controls: {len(results) - len(failed)}/{len(results)} passed')
    for n in failed:
        print('  FAILED ' + n)
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main())
