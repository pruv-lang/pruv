"""L3-10 Q runner (spec 11): generate cases with q_gen.gen, run the Node oracle twice, check it, optionally compare an implementation.
  python3 q.py [--seed 20261117] [--families 1,2,...,8] [--per-family 250] [--count N] [--batch 500] [--impl PATH [ARGS...]]
                [--impl-arg ARG ...] [--lane native|hand|ts] [--profile S|M|SM] [--pruvc P] [--rustc R] [--backend B] [--build-flags "F"] [--json OUT]
--backend / --build-flags describe how the --impl binary was built (default: lane ts -> "ts-oracle" / "n/a", else <impl>.build.json,
else "unknown"); the metadata never assumes a backend or flags.
Exit status: 0 only when an implementation ran and every case was compared and matched; 1 = any failure (unexplained difference, oracle
failure, non-zero implementation exit status, extra output lines, uncompared cases); 3 = no implementation (oracle determinism only,
the last line is "RESULT oracle-only" and is never a Pruv success). Every run ends with a "RESULT <pass|fail|oracle-only>" line; the
JSON summary and failure artifacts carry run_meta(...) under "meta".
Oracle checks (any failure -> exit 1): the two oracle runs are identical (determinism), no driver violation, every request terminal
is ok or thrown. Reports the terminal distribution per family and the number of excluded-domain cases.
--impl CMD: CMD (a shell-split command line, run in this directory) reads the scenarios as JSONL on stdin (one batch of at most
--batch cases per process) and writes one observation JSON per line on stdout; every line is classified with compare.compare
(exact / P6 / excluded-domain / unexplained). Without --impl the run prints "implementation: not implemented (0 compared, not
counted as pass)". Failure artifacts: work/q/fail/<id>.json (seed, family, input, script, oracle, implementation,
firstDifference, pair). Exit 1 on any unexplained difference or oracle failure."""
import sys
sys.dont_write_bytecode = True
import argparse
import json
import shlex
import shutil
import subprocess
import time
from collections import Counter, defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import compare as compare_mod  # noqa: E402
from common import HERE, strip_case  # noqa: E402
from oracle import run_oracle  # noqa: E402
from run_meta import PROFILES, SEED, UNKNOWN, impl_argv, run_meta  # noqa: E402

WORK = HERE / 'work' / 'q'
FAIL_DIR = WORK / 'fail'
DEFAULT_SEED = SEED
LANES = ('native', 'hand', 'ts')
EXIT_OK, EXIT_FAIL, EXIT_NO_IMPL = 0, 1, 3
_META = None  # set by main(); written into every failure artifact


def batches(items, size):
    for i in range(0, len(items), size):
        yield items[i:i + size]


def run_batches(cases, size):
    out = []
    for chunk in batches(cases, size):
        out.extend(run_oracle(chunk))
    return out


def impl_from_args(impl, impl_arg=None):
    """argparse values -> argv list or None. `--impl PATH [ARGS...]` (several values) is taken as the argv itself; one value goes
    through run_meta.impl_argv (an existing path with spaces stays one element, otherwise shlex rules). --impl-arg values are appended."""
    if not impl:
        return None
    if isinstance(impl, str):
        impl = [impl]
    base = impl_argv(impl[0]) if len(impl) == 1 else list(impl)
    return base + list(impl_arg or [])


def add_common_args(ap):
    """Options shared by q.py / test_impl.py / perf_budget.py."""
    ap.add_argument('--impl', nargs='+', metavar='PATH_OR_ARG',
                    help='implementation: PATH [ARGS...] (argv, spaces in PATH are safe) or one quoted command string')
    ap.add_argument('--impl-arg', action='append', default=[], metavar='ARG',
                    help='extra implementation argument, repeatable; write --impl-arg=--flag for values starting with "-"')
    ap.add_argument('--lane', choices=LANES, default='native', help='native = Pruv-generated binary given by --impl; hand = hand-written Rust; ts = oracle')
    ap.add_argument('--profile', choices=sorted(PROFILES), help='S=100, M=500, SM=both (recorded in the metadata)')
    ap.add_argument('--pruvc', help='path of the Pruv compiler used to build the implementation (recorded)')
    ap.add_argument('--rustc', help='rustc path (recorded; default: rustc on PATH)')
    ap.add_argument('--backend', help='backend the --impl binary was built with (recorded as given; default: read <impl>.build.json, else "unknown")')
    ap.add_argument('--build-flags', help='build flags of the --impl binary, one string split with shlex (write --build-flags="-C opt-level=3")')


def _sidecar_build_info(argv):
    """-> (backend, flags) read from `<file>.build.json` next to the first argv element that has one, else (None, None).
    The JSON needs the keys backend / flags (flags: list or one string)."""
    for item in argv or []:
        side = Path(str(item) + '.build.json')
        if not side.is_file():
            continue
        try:
            info = json.loads(side.read_text(encoding='utf-8'))
        except (OSError, ValueError):
            continue
        if isinstance(info, dict) and info.get('backend') is not None and info.get('flags') is not None:
            flags = info['flags']
            return str(info['backend']), shlex.split(flags) if isinstance(flags, str) else [str(f) for f in flags]
    return None, None


def resolve_build(args):
    """-> (backend, flags) describing how the implementation was built. Explicit --backend / --build-flags win; lane ts is the
    oracle ("ts-oracle", "n/a"); otherwise <impl>.build.json; anything still missing is UNKNOWN (never guessed)."""
    backend = getattr(args, 'backend', None)
    text = getattr(args, 'build_flags', None)
    flags = shlex.split(text) if text is not None else None
    if getattr(args, 'lane', None) == 'ts':
        return backend if backend is not None else 'ts-oracle', flags if flags is not None else 'n/a'
    if backend is None or flags is None:
        try:
            argv = impl_from_args(getattr(args, 'impl', None), getattr(args, 'impl_arg', None))
        except ValueError:
            argv = None
        side_backend, side_flags = _sidecar_build_info(argv)
        backend = backend if backend is not None else side_backend
        flags = flags if flags is not None else side_flags
    return (backend if backend is not None else UNKNOWN), (flags if flags is not None else UNKNOWN)


def build_meta(args, seed=SEED, extra=None):
    backend, flags = resolve_build(args)
    return run_meta(pruvc=getattr(args, 'pruvc', None), backend=backend, rustc=getattr(args, 'rustc', None) or shutil.which('rustc'),
                    flags=flags, seed=seed, profile=getattr(args, 'profile', None), lanes=[args.lane], extra=extra)


def run_impl(cmd, cases, size):
    """cmd: argv list (or a string, see impl_argv). -> one entry per case: an observation dict, or a str describing a failure.
    A non-zero exit status or any output line beyond one per case fails every case of that batch."""
    argv = impl_argv(cmd)
    results = []
    for chunk in batches(cases, size):
        stdin = ''.join(json.dumps(strip_case(c), ensure_ascii=True) + '\n' for c in chunk)
        try:
            proc = subprocess.run(argv, input=stdin, capture_output=True, text=True, encoding='utf-8', errors='replace', cwd=HERE, timeout=1800)
        except Exception as exc:  # noqa: BLE001
            results.extend([f'crash: runner failed to run: {exc}'] * len(chunk))
            continue
        if proc.returncode:
            results.extend(['crash: exit %d: %s' % (proc.returncode, proc.stderr[-300:])] * len(chunk))
            continue
        lines = [ln for ln in proc.stdout.split('\n') if ln.strip()]
        if len(lines) > len(chunk):
            results.extend([f'malformed: {len(lines) - len(chunk)} extra output line(s) (got {len(lines)} for {len(chunk)} cases)'] * len(chunk))
            continue
        got = []
        for ln in lines:
            try:
                got.append(json.loads(ln))
            except ValueError:
                got.append('malformed: ' + ln[:200])
        got.extend(['missing line'] * (len(chunk) - len(got)))
        results.extend(got)
    return results


def families_arg(text, gen_names):
    """'1,2' or 'Q1,Q2' -> list of ints (q_gen ids are Q<f>-<seed>-<n>)."""
    out = []
    for part in text.split(','):
        part = part.strip().upper().lstrip('Q')
        if part:
            out.append(int(part))
    return out


def generate(seed, fams, per_family, count):
    """q_gen.gen with families spelled 'Q<f>'; --count is round-robin over the shuffled families (spec 11)."""
    import q_gen
    return q_gen.gen(seed, ['Q%d' % f for f in fams], per_family, count or None)


def oracle_problems(obs):
    problems = []
    if obs.get('driver') is not None:
        problems.append(f"driver violations: {obs['driver']}")
    for r in obs.get('requests', []):
        if r.get('terminal') not in ('ok', 'thrown'):
            problems.append(f"request {r.get('id')}: terminal {r.get('terminal')!r}")
    return problems


def write_fail(case, seed, pair, oracle, impl, first_difference):
    FAIL_DIR.mkdir(parents=True, exist_ok=True)
    art = {'seed': seed, 'family': case.get('family'), 'case': case.get('id'), 'input': case.get('world'), 'script': case.get('script'),
           'oracle': oracle, 'implementation': impl, 'firstDifference': first_difference, 'pair': pair, 'meta': _META}
    (FAIL_DIR / f"{case['id']}.json").write_text(json.dumps(art, ensure_ascii=False, indent=1), encoding='utf-8')


def main(argv=None, impl=None):
    """impl: optional argv list that overrides --impl (used by run_m5.py so no string quoting is involved)."""
    global _META
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--seed', type=int, default=DEFAULT_SEED)
    ap.add_argument('--families', default='1,2,3,4,5,6,7,8', help='comma list of family numbers (1-8, "Q1" spelling accepted)')
    ap.add_argument('--per-family', type=int, default=250)
    ap.add_argument('--count', type=int, default=0, help='N cases total, round-robin over the shuffled families')
    ap.add_argument('--batch', type=int, default=500)
    ap.add_argument('--json', help='write a summary (result, counts, meta) to this path')
    add_common_args(ap)
    args = ap.parse_args(argv)
    impl_cmd = list(impl) if impl else impl_from_args(args.impl, args.impl_arg)
    _META = build_meta(args, seed=args.seed)

    t0 = time.time()
    fams = families_arg(args.families, None)
    cases = generate(args.seed, fams, args.per_family, args.count)
    if not cases:
        print('no cases generated')
        return 1
    ids = [c['id'] for c in cases]
    if len(set(ids)) != len(ids):
        print('FAIL: duplicate case ids from the generator')
        return 1
    first = run_batches(cases, args.batch)
    second = run_batches(cases, args.batch)
    t1 = time.time()

    for old in FAIL_DIR.glob('*.json') if FAIL_DIR.exists() else []:
        old.unlink()
    nondet = [i for i, (a, b) in enumerate(zip(first, second))
              if json.dumps(a, sort_keys=True, ensure_ascii=True) != json.dumps(b, sort_keys=True, ensure_ascii=True)]
    hist = defaultdict(Counter)
    fam_count = Counter()
    excluded = 0
    oracle_failures = 0
    driver_violations = 0
    for i, (c, o) in enumerate(zip(cases, first)):
        fam = c.get('family', '?')
        fam_count[fam] += 1
        for r in o.get('requests', []):
            hist[fam][r.get('terminal')] += 1
        if o.get('excluded') or o.get('domain') is not None:
            excluded += 1
        probs = oracle_problems(o)
        if o.get('driver') is not None:
            driver_violations += 1
        if probs or i in nondet:
            oracle_failures += 1
            write_fail(c, args.seed, 'oracle', o, second[i] if i in nondet else None,
                       'nondeterministic' if i in nondet else probs[0])
            if oracle_failures <= 10:
                print(f"ORACLE FAIL {c['id']}: {'nondeterministic' if i in nondet else probs[0][:300]}")

    print(f'seed {args.seed}: {len(cases)} cases, families: ' + ' '.join(f'{k}={fam_count[k]}' for k in sorted(fam_count)))
    for fam in sorted(hist):
        print(f'  {fam} terminals: ' + ' '.join(f'{t}={n}' for t, n in sorted(hist[fam].items(), key=lambda kv: str(kv[0]))))
    print(f'excluded-domain cases: {excluded}')
    print(f'driver violations: {driver_violations}')
    print(f"determinism: {'identical' if not nondet else 'DIFFERENT at ' + ', '.join(ids[i] for i in nondet[:10])}")

    unexplained = oracle_failures
    compared = 0
    counts = Counter()
    if impl_cmd:
        impl_obs = run_impl(impl_cmd, cases, args.batch)
        shown = 0
        for c, o, im in zip(cases, first, impl_obs):
            if isinstance(im, str):
                cls, detail = 'unexplained', im
            else:
                cls, detail = compare_mod.compare(o, im)
            counts[cls] += 1
            if cls == 'unexplained':
                write_fail(c, args.seed, ' '.join(impl_cmd), o, im, detail)
                if shown < 10:
                    shown += 1
                    print(f"DIFF {c['id']}: {detail[:300]}")
        compared = sum(counts.values())
        unexplained += counts['unexplained']
        print('implementation compare: ' + ' '.join(f'{k}={counts[k]}' for k in compare_mod.CLASSES) + f'  ({compared} compared)')
        if counts['unexplained']:
            print(f'artifacts in {FAIL_DIR}')
    else:
        print('implementation: not implemented (0 compared, not counted as pass)')
    print(f'oracle: {len(cases)} cases x2 in {t1 - t0:.1f}s, {oracle_failures} oracle failures, {unexplained} total unexplained/failed')

    if unexplained:
        rc, result = EXIT_FAIL, 'fail'
    elif not impl_cmd:
        rc, result = EXIT_NO_IMPL, 'oracle-only'
    elif compared != len(cases):
        rc, result = EXIT_FAIL, 'fail'
    else:
        rc, result = EXIT_OK, 'pass'
    detail = {'oracle-only': ' (no implementation compared; this is not a Pruv result)',
              'fail': f' ({unexplained} unexplained/failed, {compared}/{len(cases)} compared)',
              'pass': f' ({compared} compared, all matched)'}[result]
    print('RESULT ' + result + detail)
    if args.json:
        out = Path(args.json)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps({'result': result, 'exit': rc, 'cases': len(cases), 'compared': compared, 'unexplained': unexplained,
                                   'classes': dict(counts), 'meta': _META}, indent=1), encoding='utf-8')
    return rc


if __name__ == '__main__':
    sys.exit(main())
