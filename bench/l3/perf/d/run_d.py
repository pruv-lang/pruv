"""Leg D: one in-process call of the bound kernel, Pruv vs hand Rust vs JS (node), on the req list no-match workload.

  sh build_d.sh && python3 run_d.py [--sizes 2000,20000,50000] [--m 10] [--reps 5]

Writes ../work/d/d.json and prints one JSON row per (size, variant, impl). All impls must agree on the checksum.
"""
import argparse, json, os, subprocess

HERE = os.path.dirname(os.path.abspath(__file__))
WORK = os.path.join(HERE, '..', 'work', 'd')


def run(cmd):
    return json.loads(subprocess.run(cmd, capture_output=True, text=True, check=True).stdout.strip().splitlines()[-1])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--sizes', default='2000,20000,50000')
    ap.add_argument('--m', type=int, default=10)
    ap.add_argument('--reps', type=int, default=5)
    a = ap.parse_args()
    rows = []
    for S in map(int, a.sizes.split(',')):
        pre = os.path.join(WORK, f'data_{S}_{a.m}')
        subprocess.run(['python3', os.path.join(HERE, 'gen.py'), str(S), str(a.m), pre], check=True)
        res = {}
        for v in ('int', 'str'):
            for imp in ('pruv', 'hand'):
                res[(v, imp)] = run([os.path.join(WORK, 'main'), v, imp, f'{pre}.{v}.txt', str(a.reps)])
            res[(v, 'ts')] = run(['node', os.path.join(HERE, 'ts_kernel.mjs'), v, f'{pre}.{v}.txt', str(a.reps)])
        res[('obj', 'ts')] = run(['node', os.path.join(HERE, 'ts_kernel.mjs'), 'obj', f'{pre}.str.txt', str(a.reps)])
        sums = {r['checksum'] for r in res.values()}
        for (v, imp), r in res.items():
            row = {'S': S, 'M': a.m, **r, 'agree': len(sums) == 1}
            rows.append(row)
            print(json.dumps(row), flush=True)
    with open(os.path.join(WORK, 'd.json'), 'w') as f:
        json.dump(rows, f, indent=1)


if __name__ == '__main__':
    main()
