#!/usr/bin/env python3
"""Measure Codex read throughput and peak RSS against hand Rust on macOS."""

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import signal
from statistics import median_low as median
import subprocess
import tempfile
from time import perf_counter_ns

HERE = Path(__file__).resolve().parent
WORK = HERE / 'work' / 'r09'
PAIRS = ('py_rust', 'py_llvm', 'rs_rust', 'rs_llvm')
BATCH = 20
THREAD = 'r09-thread'
FOLDER = '/work/r09'


def build_hand():
    cargo_bin = Path.home() / '.cargo/bin'
    rustc = shutil.which('rustc') or cargo_bin / 'rustc'
    cargo_target = WORK / 'cargo'
    env = dict(os.environ, TMPDIR=str(WORK))
    subprocess.run([cargo_bin / 'cargo', 'build', '--release', '--offline', '--locked',
                    '--manifest-path', HERE / 'host/Cargo.toml', '--target-dir', cargo_target],
                   env=env, check=True)
    deps = cargo_target / 'release/deps'
    serde = next(deps.glob('libserde_json-*.rlib'))
    for name in ('reader', 'replay'):
        subprocess.run([rustc, '-O', '--edition', '2021', HERE / f'hand/{name}.rs',
                        '--extern', f'serde_json={serde}', '-L', f'dependency={deps}',
                        '-o', WORK / name], env=env, check=True)
    # Pruv runners: relink the test_impl harness (main.rs, already verified Pruv emission) against the same release
    # codex_host and serde_json as the hand side, so both lanes use one dependency profile.
    host = next(deps.glob('libcodex_host-*.rlib'))
    adapter = WORK / 'libpruv_str.rlib'
    subprocess.run([rustc, '-O', '--crate-name', 'pruv_str', '--crate-type', 'rlib', HERE / 'str_adapter.rs',
                    '-o', adapter], env=env, check=True)
    runners = {}
    for pair in PAIRS:
        src = HERE / 'work' / pair
        main_rs = src / 'main.rs'
        # The emission (and LLVM objects) must be newer than every Pruv source of this fragment and its packages.
        built = [main_rs, src / 'root_gen.rs']
        objs = [src / 'codex_gen.o', src / 'codex_rt.o'] if pair.endswith('llvm') else []
        sources = list(HERE.glob('*.pruv')) + list((HERE.parent / 'pkgs').rglob('*.pruv')) + [HERE / 'harness.rs']
        newest = max(p.stat().st_mtime for p in sources)
        if any(not p.is_file() or p.stat().st_mtime < newest for p in built + objs) \
                or 'sol::codex_read_wire(' not in main_rs.read_text():
            raise RuntimeError(f'{src} is missing or stale. Run test_impl.py --only read_pages_40 first.')
        extra = [arg for obj in objs for arg in ('-C', f'link-arg={obj}')]
        exe = WORK / f'{pair}_runner'
        subprocess.run([rustc, '-O', '--edition', '2021', main_rs, '-o', exe, '--extern', f'codex_host={host}',
                        '--extern', f'pruv_str={adapter}', '--extern', f'serde_json={serde}',
                        '-L', f'dependency={deps}', *extra], env=env, check=True)
        runners[pair] = exe
    return runners


def encode(value):
    return json.dumps(value, ensure_ascii=False, separators=(',', ':')).encode('utf-8') + b'\n'


def make_turn(number):
    prompt = (
        f'Review deployment #{number} for the 서울 and Montréal regions.\n\n'
        + ''.join(f'- Shard {shard:02}: compare latency, cache misses, and the "queue depth" counter.\n'
                  for shard in range(23))
        + '\nSummarize the regressions and propose a rollback threshold.'
    )
    reply = (
        f'## Deployment #{number}\n\nThe largest change was confined to the 서울 region.\n\n'
        + ''.join(f'- Shard {shard:02}: p95 rose from {80 + shard} ms to {100 + shard} ms. '
                  'The extra delay appears after cache lookup and before queue admission.\n'
                  for shard in range(23))
        + '\nSet the threshold at 125 ms and watch both regions for the next 30 minutes.'
    )
    return {
        'id': f'turn-{number}',
        'startedAt': '2026-10-12T12:00:00.000Z', 'completedAt': '2026-10-12T12:00:05.000Z',
        'items': [
            {'type': 'userMessage', 'content': [{'type': 'text', 'text': prompt}]},
            {'type': 'agentMessage', 'phase': 'commentary', 'text': 'Checking deployment logs.'},
            {'type': 'commandExecution', 'command': 'compare-metrics'},
            {'type': 'agentMessage', 'phase': 'final_answer', 'text': reply},
        ],
    }, len(prompt.encode('utf-8')) + len(reply.encode('utf-8'))


def scenario(size, page_count, page_size):
    script = WORK / f'{size}.jsonl'
    with script.open('wb') as f:
        def exchange(request, result):
            f.write(encode(request))
            f.write(b'\n' if result is None else encode({'id': request['id'], 'result': result}))

        exchange({'id': 1, 'method': 'initialize', 'params': {
            'clientInfo': {'name': 'lair', 'title': 'Lair', 'version': '0.0.1'},
            'capabilities': {'experimentalApi': True, 'requestAttestation': False},
        }}, {'userAgent': 'r09-replay'})
        exchange({'method': 'initialized', 'params': {}}, None)
        exchange({'id': 2, 'method': 'thread/read', 'params': {'threadId': THREAD, 'includeTurns': False}},
                 {'thread': {'id': THREAD, 'cwd': FOLDER, 'name': 'Deployment review', 'model': 'codex-r09'}})
        count = page_count * page_size
        text_bytes = 0
        for page in range(page_count):
            params = {'threadId': THREAD, 'limit': 20, 'sortDirection': 'desc', 'itemsView': 'summary'}
            if page:
                params['cursor'] = f'page-{page}'
            start = count - page * page_size
            data = []
            for number in range(start, start - page_size, -1):
                turn, length = make_turn(number)
                data.append(turn)
                text_bytes += length
            exchange({'id': page + 3, 'method': 'thread/turns/list', 'params': params}, {
                'data': data, 'nextCursor': f'page-{page + 1}' if page + 1 < page_count else None,
            })
    assert count <= 200 and text_bytes <= 2 * 1024 * 1024
    request = encode({'mode': 'read', 'binary': str(WORK / 'replay'), 'args': [str(script)],
                      'cwd': str(HERE), 'folder': FOLDER, 'thread': THREAD, 'timeout': 20000})
    print(f'{size}: {count} turns, {page_count} pages, {text_bytes:,} UTF-8 text bytes', flush=True)
    return request, count


def invoke(cmd, payload):
    # The private group contains only this invocation and its children.
    with subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                          start_new_session=True, env=dict(os.environ, LC_ALL='C')) as child:
        try:
            stdout, stderr = child.communicate(payload, timeout=600)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            child.communicate()
            raise
        if child.returncode:
            raise RuntimeError(f'{cmd}: exit {child.returncode}\n{stderr.decode(errors="replace")}')
    return stdout, stderr


def rss_timer():
    probe = subprocess.run(['/usr/bin/time', '-l', '/usr/bin/true'], capture_output=True)
    if probe.returncode == 0:
        return time_command
    if b'kern.clockrate: Operation not permitted' in probe.stderr:
        print('time -l blocked by sandbox; measuring peak RSS with wait4', flush=True)
        return time_wait4
    raise RuntimeError(probe.stderr.decode(errors='replace'))


def time_command(runner, payload):
    start = perf_counter_ns()
    stdout, stderr = invoke(['/usr/bin/time', '-l', runner], payload)
    ns = perf_counter_ns() - start
    rss = re.search(rb'^\s*(\d+)\s+maximum resident set size\s*$', stderr, re.M)
    if rss is None:
        raise RuntimeError(f'{runner}: missing macOS max RSS in {stderr!r}')
    return ns, int(rss[1]), stdout


def time_wait4(runner, payload):
    # Files let the runner finish without a pipe reader reaping it before wait4.
    with (tempfile.TemporaryFile(dir=WORK) as stdin,
          tempfile.TemporaryFile(dir=WORK) as stdout,
          tempfile.TemporaryFile(dir=WORK) as stderr):
        stdin.write(payload)
        stdin.seek(0)
        start = perf_counter_ns()
        with subprocess.Popen([runner], stdin=stdin, stdout=stdout, stderr=stderr) as child:
            _, status, usage = os.wait4(child.pid, 0)
            ns = perf_counter_ns() - start
            child.returncode = os.waitstatus_to_exitcode(status)
        if child.returncode:
            stderr.seek(0)
            raise RuntimeError(f'{runner}: exit {child.returncode}\n{stderr.read().decode(errors="replace")}')
        stdout.seek(0)
        return ns, usage.ru_maxrss, stdout.read()


def measure(name, size, pruv, hand, request, expected, rounds, timed, base=None):
    samples = {'pruv': [], 'hand': [], 'base': []}
    payload = request * BATCH
    expected_batch = expected * BATCH
    sides = [('pruv', pruv), ('hand', hand)] + ([('base', base)] if base else [])
    for index in range(rounds):
        order = sides[index % len(sides):] + sides[:index % len(sides)]
        for side, runner in order:
            ns, rss, stdout = timed(runner, payload)
            if stdout != expected_batch:
                raise RuntimeError(f'{name} {size} round {index + 1}: {side} batch differs from preflight')
            samples[side].append((ns, rss))

    cells, base_cells = [], []
    for index, (axis, unit) in enumerate((('time', 'ns'), ('rss', 'bytes'))):
        p, h = (median(sample[index] for sample in samples[side]) for side in ('pruv', 'hand'))
        cells.append(dict(name=name, axis=axis, size=size, kind='proportional',
                          pruv=p, hand=h, ratio=p / h, unit=unit, runs=rounds))
        line = f'{name:16} {size} {axis:4} {unit:5} {p:15,d} {h:15,d} {p / h:8.3f}x'
        if base:
            b = median(sample[index] for sample in samples['base'])
            base_cells.append(dict(name=name, axis=axis, size=size, kind='proportional',
                                   pruv=b, hand=h, ratio=b / h, unit=unit, runs=rounds))
            line += f'   base {b:15,d} {b / h:8.3f}x'
        print(line, flush=True)
    return cells, base_cells




def probe(runners, hand, rounds, timed, bases=None):
    """G-DB38-3: one-process fixed-share probe. E0 = empty stdin (no request: runtime, code pages, startup),
    E1 = one read of a thread with no turns (adds the per-read scratch), M1 = one read of the M thread."""
    empty, _ = scenario('E', 1, 0)
    medium, medium_count = scenario('M', 5, 10)
    cases = [('E0', b''), ('E1', empty), ('M1', medium)]
    sides = [('hand', hand)] + [(pair, runner) for pair, runner in runners.items()]
    sides += [(f'old/{pair}', runner) for pair, runner in (bases or {}).items()]
    expected = {}
    for case, payload in cases:
        expected[case] = invoke([hand], payload)[0] if payload else b''
        if case == 'M1' and len(json.loads(expected[case]).get('turns', [])) != medium_count:
            raise RuntimeError(f'M1: hand reader did not keep all {medium_count} turns')
        for side, runner in sides:
            if invoke([runner], payload)[0] != expected[case]:
                raise RuntimeError(f'{side} {case}: stdout differs from hand')
    print(f'probe: {rounds} round(s), one process per sample, median_low peak RSS (bytes)', flush=True)
    print(f'{"side":12} {"size":>9} ' + ' '.join(f'{case:>11}' for case, _ in cases), flush=True)
    rss = {side: {case: [] for case, _ in cases} for side, _ in sides}
    for index in range(rounds):
        order = sides[index % len(sides):] + sides[:index % len(sides)]
        for case, payload in cases:
            for side, runner in order:
                _, peak, out = timed(runner, payload)
                if out != expected[case]:
                    raise RuntimeError(f'{side} {case} round {index + 1}: stdout differs from hand')
                rss[side][case].append(peak)
    for side, runner in sides:
        print(f'{side:12} {Path(runner).stat().st_size:9,d} '
              + ' '.join(f'{median(rss[side][case]):11,d}' for case, _ in cases), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--quick', action='store_true', help='run one round at size M')
    parser.add_argument('--rounds', type=int, default=7, help='alternating rounds (default 7)')
    parser.add_argument('--save', help='copy the Pruv runners into this directory (a base for --base)')
    parser.add_argument('--base', help='also time the runners saved in this directory by --save')
    parser.add_argument('--probe', type=int, metavar='ROUNDS',
                        help='only run the one-read fixed-share probe (G-DB38-3) for this many rounds')
    args = parser.parse_args()
    if args.probe is not None and args.probe < 1:
        parser.error('--probe needs at least one round')
    quick = args.quick
    WORK.mkdir(parents=True, exist_ok=True)
    runners = build_hand()
    hand = WORK / 'reader'
    if args.save:
        Path(args.save).mkdir(parents=True, exist_ok=True)
        for pair, runner in runners.items():
            shutil.copy2(runner, Path(args.save) / runner.name)
        print(f'saved runners to {args.save}', flush=True)
        return
    bases = {pair: Path(args.base) / runner.name for pair, runner in runners.items()} if args.base else {}
    if args.probe:
        probe(runners, hand, args.probe, rss_timer(), bases)
        return
    sizes = [('M', 5, 10)] if quick else [('M', 5, 10), ('L', 40, 5)]
    checked = {}
    for size, pages, per_page in sizes:
        request, count = scenario(size, pages, per_page)
        expected, _ = invoke([hand], request)
        result = json.loads(expected)
        if len(result.get('turns', [])) != count or result.get('omittedBefore') is not False:
            raise RuntimeError(f'{size}: hand reader did not keep all {count} turns')
        for pair, runner in runners.items():
            actual, _ = invoke([runner], request)
            if actual != expected:
                raise RuntimeError(f'{pair} {size}: Pruv and hand stdout differ before timing')
            if pair in bases and invoke([bases[pair]], request)[0] != expected:
                raise RuntimeError(f'{pair} {size}: base and hand stdout differ before timing')
        checked[size] = request, expected

    rounds = 1 if quick else args.rounds
    timed = rss_timer()
    print(f'{BATCH} reads per sample, {rounds} round(s), preflight stdout matched', flush=True)
    print(f'{"case":16} S {"axis":4} {"unit":5} {"Pruv":>15} {"hand":>15} {"Pruv/hand":>9}', flush=True)
    cells, base_cells = [], []
    for pair, runner in runners.items():
        for size, (request, expected) in checked.items():
            name = f'read/{pair.replace("_", "/")}'
            got, got_base = measure(name, size, runner, hand, request, expected, rounds, timed, bases.get(pair))
            cells.extend(got)
            base_cells.extend(got_base)
    method = 'wait4' if timed is time_wait4 else 'time -l'
    for cell_list, stem in ((cells, 'perf_cells'), (base_cells, 'perf_cells_base')):
        if cell_list:
            out = WORK / (stem + ('_quick.json' if quick else '.json'))
            out.write_text(json.dumps({'source': 'codex/r09', 'timing': f'wall perf_counter_ns, peak RSS via {method}',
                                       'rounds': rounds, 'cells': cell_list}, indent=2) + '\n')


if __name__ == '__main__':
    main()
