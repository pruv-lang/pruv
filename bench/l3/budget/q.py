"""L3-10 Q runner (spec 11): generate cases with q_gen.gen, run the Node oracle twice, check it, optionally compare an implementation.
  python3 q.py [--seed 20261117] [--families 1,2,...,8] [--per-family 250] [--count N] [--batch 500] [--impl "CMD"]
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
import subprocess
import time
from collections import Counter, defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import compare as compare_mod  # noqa: E402
from common import HERE, strip_case  # noqa: E402
from oracle import run_oracle  # noqa: E402

WORK = HERE / 'work' / 'q'
FAIL_DIR = WORK / 'fail'
DEFAULT_SEED = 20261117


def batches(items, size):
    for i in range(0, len(items), size):
        yield items[i:i + size]


def run_batches(cases, size):
    out = []
    for chunk in batches(cases, size):
        out.extend(run_oracle(chunk))
    return out


def run_impl(cmd, cases, size):
    """-> one entry per case: an observation dict, or a str describing a crash / malformed / missing line."""
    results = []
    for chunk in batches(cases, size):
        stdin = ''.join(json.dumps(strip_case(c), ensure_ascii=True) + '\n' for c in chunk)
        try:
            proc = subprocess.run(shlex.split(cmd), input=stdin, capture_output=True, text=True, encoding='utf-8', errors='replace', cwd=HERE, timeout=1800)
        except Exception as exc:  # noqa: BLE001
            results.extend([f'crash: runner failed to run: {exc}'] * len(chunk))
            continue
        lines = [ln for ln in proc.stdout.split('\n') if ln.strip()]
        got = []
        for ln in lines[:len(chunk)]:
            try:
                got.append(json.loads(ln))
            except ValueError:
                got.append('malformed: ' + ln[:200])
        missing = 'crash: exit %d: %s' % (proc.returncode, proc.stderr[-300:]) if proc.returncode else 'missing line'
        got.extend([missing] * (len(chunk) - len(got)))
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
           'oracle': oracle, 'implementation': impl, 'firstDifference': first_difference, 'pair': pair}
    (FAIL_DIR / f"{case['id']}.json").write_text(json.dumps(art, ensure_ascii=False, indent=1), encoding='utf-8')


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--seed', type=int, default=DEFAULT_SEED)
    ap.add_argument('--families', default='1,2,3,4,5,6,7,8', help='comma list of family numbers (1-8, "Q1" spelling accepted)')
    ap.add_argument('--per-family', type=int, default=250)
    ap.add_argument('--count', type=int, default=0, help='N cases total, round-robin over the shuffled families')
    ap.add_argument('--batch', type=int, default=500)
    ap.add_argument('--impl', help='implementation command (cases JSONL on stdin, observations JSONL on stdout)')
    args = ap.parse_args(argv)

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
    if args.impl:
        impl_obs = run_impl(args.impl, cases, args.batch)
        counts = Counter()
        shown = 0
        for c, o, im in zip(cases, first, impl_obs):
            if isinstance(im, str):
                cls, detail = 'unexplained', im
            else:
                cls, detail = compare_mod.compare(o, im)
            counts[cls] += 1
            if cls == 'unexplained':
                write_fail(c, args.seed, args.impl, o, im, detail)
                if shown < 10:
                    shown += 1
                    print(f"DIFF {c['id']}: {detail[:300]}")
        unexplained += counts['unexplained']
        print('implementation compare: ' + ' '.join(f'{k}={counts[k]}' for k in compare_mod.CLASSES) + f'  ({sum(counts.values())} compared)')
        if counts['unexplained']:
            print(f'artifacts in {FAIL_DIR}')
    else:
        print('implementation: not implemented (0 compared, not counted as pass)')
    print(f'oracle: {len(cases)} cases x2 in {t1 - t0:.1f}s, {oracle_failures} oracle failures, {unexplained} total unexplained/failed')
    return 1 if unexplained else 0


if __name__ == '__main__':
    sys.exit(main())
