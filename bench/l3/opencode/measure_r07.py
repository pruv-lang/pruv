#!/usr/bin/env python3
"""Measure the R07 OpenCode begin and step lanes against hand Rust on macOS."""

import argparse
import json
from pathlib import Path
import re
from statistics import median
import subprocess
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.dont_write_bytecode = True
import test_impl

HAND = HERE / 'hand'
WORK = HERE / 'work' / 'r07'
PAIRS = ('rs_rust', 'rs_llvm')


def timed(exe, lane, env, steps):
    result = subprocess.run(
        ['/usr/bin/time', '-l', str(exe), lane, str(env), str(steps)],
        capture_output=True, text=True, timeout=600,
    )
    if result.returncode:
        raise RuntimeError(f'{exe}: exit {result.returncode}\n{result.stderr}')
    rss = re.search(r'^\s*(\d+)\s+maximum resident set size\s*$', result.stderr, re.M)
    if rss is None:
        raise RuntimeError(f'{exe}: missing macOS max RSS in {result.stderr!r}')
    ns, digest = result.stdout.split()
    return (int(ns), int(rss[1])), digest


def measure(pruv, hand, name, lane, size, env, steps, rounds):
    samples = {'pruv': [], 'hand': []}
    digests = {}
    for index in range(rounds):
        order = [('pruv', pruv), ('hand', hand)]
        if index % 2:
            order.reverse()
        for side, exe in order:
            sample, digest = timed(exe, lane, env, steps)
            if digest != digests.setdefault(side, digest):
                raise RuntimeError(f'{name} {size} round {index + 1}: {side} digest changed')
            samples[side].append(sample)

    cells = []
    for index, (axis, unit) in enumerate([('time', 'ns'), ('rss', 'bytes')]):
        p, h = (median(sample[index] for sample in samples[side]) for side in ('pruv', 'hand'))
        cells.append(dict(name=name, axis=axis, size=size, kind='proportional',
                          pruv=p, hand=h, ratio=p / h, unit=unit, runs=rounds))
        print(f'{name:25} {size} {axis:4} {p:12g} {h:12g} {p / h:8.3f}x', flush=True)
    return cells, digests


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--quick', action='store_true', help='run one round at size M')
    parser.add_argument('--rounds', type=int, default=7, help='alternating rounds per cell (default 7)')
    args = parser.parse_args()
    quick = args.quick
    for pair in PAIRS:
        root = test_impl.WORK / pair / 'root_gen.rs'
        assert root.is_file() and (pair != 'rs_llvm' or root.with_suffix('.ll').is_file()), (
            'run test_impl.py --pairs rs/rust,rs/llvm first'
        )

    _, g, _ = test_impl.configure()
    host = test_impl.build_host(g.RUSTC)
    hand_dir = WORK / 'hand'
    hand_dir.mkdir(parents=True, exist_ok=True)
    hand_main = hand_dir / 'main.rs'
    hand_main.write_text((HAND / 'hand_main.rs').read_text().replace('@SOL@', str(HAND / 'begin_hand.rs')))
    hand = hand_dir / 'runner'
    test_impl.sh([g.RUSTC, '-O', '--edition', '2021', hand_main, '-o', hand])

    sizes = [('M', 1000, 20000)]
    if not quick:
        sizes.append(('L', 4000, 80000))
    rounds = 1 if quick else args.rounds
    cells = []
    print(f'{"case":25} S {"axis":4} {"Pruv":>12} {"hand":>12} {"Pruv/hand":>9}', flush=True)
    for pair in PAIRS:
        root = test_impl.WORK / pair / 'root_gen.rs'
        out = WORK / pair
        out.mkdir(parents=True, exist_ok=True)
        entry = out / 'main.rs'
        entry.write_text((HAND / 'perf_main.rs').read_text().replace('@SOL@', str(root)))
        extra = []
        if pair == 'rs_llvm':
            extra, error = g.build_llvm_objects(str(root.with_suffix('.ll')), 'opencode', str(out))
            assert not error, error
        pruv = out / 'runner'
        test_impl.rustc_link(g.RUSTC, entry, pruv, host, extra)

        label = pair.replace('_', '/')
        for size, env, steps in sizes:
            begin, _ = measure(pruv, hand, f'begin/{label}', 'begin', size, env, 0, rounds)
            checked, checked_digests = measure(pruv, hand, f'step_checked/{label}',
                                              'step_checked', size, 1000, steps, rounds)
            admitted, admitted_digests = measure(pruv, hand, f'step_admitted/{label}',
                                                'step_admitted', size, 1000, steps, rounds)
            if checked_digests != admitted_digests:
                raise RuntimeError(f'{label} {size}: checked and admitted step digests differ')
            if checked_digests['pruv'] != checked_digests['hand']:
                raise RuntimeError(f'{label} {size}: Pruv and hand step packets differ')
            cells.extend(begin + checked + admitted)

    (WORK / 'perf_cells.json').write_text(json.dumps({'source': 'opencode/r07', 'cells': cells}, indent=2) + '\n')


if __name__ == '__main__':
    main()
