"""L3-10 M5 driver (M5_NOTES.md): Q cases -> oracle -> effect programs -> Rust host compare; then the timer/storage perf.
  python3 m5_test/run_m5.py [--count 300] [--seed S] [--build] [--perf 100,500,1000] [--reps 5]
Run from bench/l3/budget. Outputs go to work/m5/ (cases, oracle observations, programs) and work/m5build/ (harness).
The compare itself is q.py --impl, so it reruns the oracle twice and classifies every case exactly as for any other implementation."""
import sys
sys.dont_write_bytecode = True
import argparse
import json
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(HERE))
from common import strip_case  # noqa: E402
from oracle import run_oracle  # noqa: E402
import q_gen  # noqa: E402
import q  # noqa: E402

WORK = HERE / 'work' / 'm5'
HARNESS = HERE / 'work' / 'm5build' / 'harness'


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument('--seed', type=int, default=q.DEFAULT_SEED)
    ap.add_argument('--count', type=int, default=300)
    ap.add_argument('--build', action='store_true', help='run dev_build.sh first')
    ap.add_argument('--perf', default='100,500,1000')
    ap.add_argument('--reps', type=int, default=5)
    args = ap.parse_args(argv)
    WORK.mkdir(parents=True, exist_ok=True)

    cases = q_gen.gen(args.seed, [f'Q{k}' for k in range(1, 9)], 250, args.count)
    (WORK / 'cases.jsonl').write_text(''.join(json.dumps(strip_case(c), ensure_ascii=True) + '\n' for c in cases), encoding='utf-8')
    obs = run_oracle(cases)
    (WORK / 'oracle.jsonl').write_text(''.join(json.dumps(o, ensure_ascii=True) + '\n' for o in obs), encoding='utf-8')
    subprocess.run([sys.executable, str(HERE / 'm5_test' / 'gen_prog.py'), str(WORK / 'oracle.jsonl'), str(WORK / 'prog.jsonl')],
                   check=True, cwd=HERE)
    if args.build:
        subprocess.run(['sh', str(HERE / 'dev_build.sh')], check=True, cwd=HERE)

    print('== Q compare (host replays generated effect programs)', flush=True)
    rc = q.main(['--seed', str(args.seed), '--count', str(args.count),
                 '--impl', f'{HARNESS} q --prog {WORK / "prog.jsonl"}'])

    print('== perf: Rust host (ns per op)', flush=True)
    for n in args.perf.split(','):
        subprocess.run([str(HARNESS), 'perf', n, str(args.reps)], check=True, cwd=HERE)
    print('== perf: TS oracle world (ns per op)', flush=True)
    subprocess.run(['node', str(HERE / 'm5_test' / 'perf_ts.mjs'), args.perf, str(args.reps)], check=True, cwd=HERE)
    return rc


if __name__ == '__main__':
    sys.exit(main())
