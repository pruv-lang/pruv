"""L3-10 implementation test: the fixed fixtures and a Q sample, run through a Pruv implementation command and compared with the
Node oracle (compare.compare: exact / P6 / excluded-domain / unexplained).
  python3 test_impl.py [--impl PATH [ARGS...]] [--impl-arg ARG] [--lane native|hand|ts] [--profile S|M|SM] [--json OUT]
                       [--group A,B] [--q-count 300] [--seed 20261117] [--batch 500]
(a single quoted command string is still accepted.) The command is --impl, else the environment variable BUDGET_IMPL, else work/build/budget_impl when that file exists. CMD reads scenario
JSONL on stdin and writes one observation JSON per line (the same contract as q.py --impl).
Without an implementation (no command, or the command's executable is missing) each pair (fixtures, q) is printed as
"not implemented" and the exit code is 2. Exit 0 is only possible when both pairs compared at least one case and nothing is
unexplained; exit 1 = unexplained difference or an oracle failure; exit 2 = not implemented / nothing compared. A not-implemented run
is never a pass."""
import sys
sys.dont_write_bytecode = True
import argparse
import json
import os
import shutil
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import compare as compare_mod  # noqa: E402
import q as q_mod  # noqa: E402
from common import GROUPS, HERE, load_all  # noqa: E402
from oracle import run_oracle  # noqa: E402

BUILD_FILE = HERE / 'work' / 'build' / 'budget_impl'
OUT = HERE / 'work' / 'test_impl'
PAIRS = ('fixtures', 'q')


def resolve_impl(arg, impl_arg=None):
    """arg: argparse --impl values (list), a string, or None. -> argv list or None (not implemented).
    The auto-discovered BUILD_FILE is returned as a one-element list, never as a string."""
    try:
        argv = q_mod.impl_from_args(arg, impl_arg)
        if argv is None and os.environ.get('BUDGET_IMPL'):
            argv = q_mod.impl_from_args(os.environ['BUDGET_IMPL'], impl_arg)
    except ValueError:
        return None
    if not argv:
        return [str(BUILD_FILE)] + list(impl_arg or []) if BUILD_FILE.is_file() else None
    exe = argv[0]
    if os.sep in exe or exe.startswith('.'):
        path = Path(exe) if os.path.isabs(exe) else HERE / exe
        return argv if path.is_file() else None
    return argv if shutil.which(exe) else None


def run_pair(name, cases, impl, batch, seed, meta=None):
    """-> (counts Counter, unexplained list, oracle_failed bool)."""
    first = q_mod.run_batches(cases, batch)
    second = q_mod.run_batches(cases, batch)
    nondet = [c['id'] for c, a, b in zip(cases, first, second)
              if json.dumps(a, sort_keys=True, ensure_ascii=True) != json.dumps(b, sort_keys=True, ensure_ascii=True)]
    oracle_bad = [(c['id'], p) for c, o in zip(cases, first) for p in q_mod.oracle_problems(o)]
    print(f'{name}: oracle {len(cases)} cases, nondeterministic {len(nondet)}, driver/terminal problems {len(oracle_bad)}')
    for i in nondet[:5]:
        print('  nondeterministic:', i)
    for i, p in oracle_bad[:5]:
        print('  oracle problem', i, p[:200])
    impl_obs = q_mod.run_impl(impl, cases, batch)
    counts = Counter()
    bad = []
    out_dir = OUT / name
    out_dir.mkdir(parents=True, exist_ok=True)
    for old in out_dir.glob('*.json'):
        old.unlink()
    for c, o, im in zip(cases, first, impl_obs):
        if isinstance(im, str):
            cls, detail = 'unexplained', im
        else:
            cls, detail = compare_mod.compare(o, im)
        counts[cls] += 1
        if cls == 'unexplained':
            bad.append((c['id'], detail))
            (out_dir / f"{c['id']}.json").write_text(json.dumps(
                {'seed': seed, 'family': c.get('family'), 'case': c['id'], 'input': c.get('world'), 'script': c.get('script'),
                 'oracle': o, 'implementation': im, 'firstDifference': detail, 'pair': name, 'meta': meta}, ensure_ascii=False, indent=1), encoding='utf-8')
    for i, d in bad[:10]:
        print(f'  DIFF {i}: {d[:300]}')
    print(f'{name}: ' + ' '.join(f'{k}={counts[k]}' for k in compare_mod.CLASSES) + f'  ({sum(counts.values())} compared)')
    return counts, bad, bool(nondet or oracle_bad)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    q_mod.add_common_args(ap)
    ap.add_argument('--json', help='write a summary (result, per-pair counts, meta) to this path')
    ap.add_argument('--group', help='fixture groups, comma list (' + ' '.join(GROUPS) + ')')
    ap.add_argument('--q-count', type=int, default=300)
    ap.add_argument('--seed', type=int, default=q_mod.DEFAULT_SEED)
    ap.add_argument('--batch', type=int, default=500)
    args = ap.parse_args(argv)

    meta = q_mod.build_meta(args, seed=args.seed)

    def finish(rc, result, pairs=None):
        print('RESULT ' + result)
        if args.json:
            out = Path(args.json)
            out.parent.mkdir(parents=True, exist_ok=True)
            out.write_text(json.dumps({'result': result, 'exit': rc, 'pairs': pairs or {}, 'meta': meta}, indent=1), encoding='utf-8')
        return rc

    impl = resolve_impl(args.impl, args.impl_arg)
    if impl is None:
        for name in PAIRS:
            print(f'{name}: not implemented')
        print('no implementation command (--impl / BUDGET_IMPL) and no ' + str(BUILD_FILE.relative_to(HERE.parent.parent.parent)) + '; nothing compared (exit 2, never a pass)')
        return finish(2, 'not-implemented')

    fixtures = load_all()
    if args.group:
        wanted = {g.strip().upper() for g in args.group.split(',') if g.strip()}
        unknown = wanted - set(GROUPS)
        if unknown:
            print('unknown group(s): ' + ','.join(sorted(unknown)))
            return finish(2, 'fail')
        fixtures = [f for f in fixtures if f['id'][0] in wanted]
    fams = q_mod.families_arg('1,2,3,4,5,6,7,8', None)
    per_family = max(1, -(-args.q_count // len(fams)))
    qcases = q_mod.generate(args.seed, fams, per_family, args.q_count)

    failed = False
    per_pair = {}  # name -> compared count (only pairs that ran)
    for name, cases in (('fixtures', fixtures), ('q', qcases)):
        if not cases:
            print(f'{name}: not implemented (no cases selected)')
            continue
        try:
            counts, bad, oracle_bad = run_pair(name, cases, impl, args.batch, args.seed, meta)
        except Exception as exc:  # noqa: BLE001
            print(f'{name}: ERROR {exc}', file=sys.stderr)
            failed = True
            continue
        per_pair[name] = sum(counts.values())
        if bad or oracle_bad or per_pair[name] != len(cases):
            failed = True
    if failed:
        return finish(1, 'fail', per_pair)
    if any(per_pair.get(name, 0) <= 0 for name in PAIRS):
        print('not every pair ran with compared > 0: not a pass')
        return finish(2, 'not-implemented', per_pair)
    return finish(0, 'pass', per_pair)


if __name__ == '__main__':
    sys.exit(main())
