#!/usr/bin/env python3
"""R08 Claude JSONL performance grid against equal-semantics hand Rust.

Both runners own Vec<i64> packets and use the same rustc -O harness, including
the same flag (0), one warm-up and five timed calls per process. File I/O,
full packet comparisons and build time are outside the call timer. Fresh
processes alternate order per round; report the median of seven process
medians and peak RSS readings. --quick uses one rs/rust round.

Usage: python3 bench/l3/claude/q_measure.py [--quick] [--only INPUT_SUBSTRING]
       [--pairs py/rust,py/llvm,rs/rust,rs/llvm] [--out PERF_CELLS_JSON]
Set PRUV_CLANG to the matching LLVM toolchain for the LLVM pairs.
"""
import argparse
from array import array
import base64
import importlib.util
import json
import os
from pathlib import Path
import platform
import re
import shutil
import signal
import statistics
import subprocess
import sys
import tempfile
import time

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
DEFAULT_OUT = HERE / 'work' / 'r08' / 'perf_cells.json'
HAND_SRC = HERE / 'handrust' / 'src'
OPS = ('window', 'head', 'tail')
FLAG = 0

_spec = importlib.util.spec_from_file_location('l3_test_a4', HERE / 'test_a4.py')
t4 = importlib.util.module_from_spec(_spec)
sys.modules['l3_test_a4'] = t4
_spec.loader.exec_module(t4)
oracle = t4.oracle
EDGE, WINDOW, LINE = t4.EDGE, t4.WINDOW, t4.LINE


# ---------------- inputs ----------------
def build_inputs():
    user, asst, L, tx, tool_use, exact = t4.user, t4.asst, t4.L, t4.tx, t4.tool_use, t4.exact

    small = L(*[x for i in range(10) for x in (
        user(f'question number {i} about the build', cwd='/work', timestamp=1700000000000 + i * 2000),
        asst([tx(f'answer {i} with some detail'), tool_use('Bash', f't{i}')], model='claude-x', timestamp=1700000001000 + i * 2000))])
    assert len(small) <= EDGE

    # 8 MiB window of many short messages, just under oracle.WINDOW
    parts, total, i = [], 0, 0
    while True:
        chunk = L(user(f'q{i}', timestamp=1700000000000 + i * 1000), asst(f'a{i}', model='claude-x', timestamp=1700000000500 + i * 1000))
        if total + len(chunk) > WINDOW - 64:
            break
        parts.append(chunk)
        total += len(chunk)
        i += 1
    win = b''.join(parts)

    # lines just under the per-line cap (LINE - 1 raw bytes + LF); 7 lines stay inside the window
    apre = b'{"type":"assistant","message":{"model":"claude-x","content":"'
    long_lines = []
    for k in range(7):
        pre = t4.USER_PRE if k % 2 == 0 else apre
        long_lines.append(exact(pre, t4.USER_SUF, LINE - 1, b'a') + b'\n')
    long_text = b''.join(long_lines)

    # assistant messages with thousands of small blocks
    blocks = lambda n: [tx('x') if j % 2 == 0 else tool_use('Read', f't{j}') for j in range(n)]
    block_arrays = L(*[x for i in range(20) for x in (user(f'q{i}', timestamp=1700000000000 + i), asst(blocks(4000), model='claude-x', timestamp=1700000000500 + i))])

    # 2000 short pairs: the 200-draft ring wraps ten times
    ring = L(*[x for i in range(2000) for x in (user(f'p{i}', timestamp=1700000000000 + i), asst(f'r{i}', model='claude-x', timestamp=1700000000500 + i))])

    # lines nested to depth 128 (object = depth 1, 127 arrays)
    n = 127
    deep = b'[' * n + b'0' + b']' * n
    dlines = []
    for i in range(3000):
        if i % 2 == 0:
            dlines.append(b'{"type":"user","message":{"content":"d%d"},"x":' % i + deep + b'}')
        else:
            dlines.append(b'{"type":"assistant","message":{"model":"claude-x","content":"e%d"},"x":' % i + deep + b'}')
    depth = b'\n'.join(dlines) + b'\n'

    inputs = [('small', small), ('window_8MiB', win), ('long_text', long_text),
              ('block_arrays', block_arrays), ('ring_wrap', ring), ('depth_limit', depth)]
    for n in (0, 1, 10, 16, 17, 200):
        # turns16 ends with the assistant for user 16. The window collector
        # checks the next slot before consuming this object, so it grows here.
        data = L(*[row for i in range(n) for row in (
            user(f'q{i}', timestamp=1700000000000 + 2000 * i),
            asst(f'a{i}', model='claude-x', timestamp=1700000001000 + 2000 * i),
        )]) if n else b''
        assert len(data) <= EDGE
        inputs.append((f'turns{n}', data))
    for name, data in inputs:
        assert len(data) <= WINDOW, (name, len(data))
    return inputs



def size_class(name):
    if name in ('small', 'turns0', 'turns1', 'turns10'):
        return 'S'
    if name in ('turns16', 'turns17'):
        return 'M'
    return 'L'


def check_packet(exe, path, op, flag, want, data=None):
    result = subprocess.run([exe, path, 'check', str(flag), op], input=data,
                            capture_output=True, check=True, timeout=120)
    got = array('q')
    got.frombytes(result.stdout)
    if sys.byteorder != 'little':
        got.byteswap()
    if got != want:
        pos = next((i for i, (g, w) in enumerate(zip(got, want)) if g != w), min(len(got), len(want)))
        raise RuntimeError(f'{exe}: {op}/{flag} word {pos}: '
                           f'{list(got[pos:pos + 4])} != {list(want[pos:pos + 4])} (oracle)')


RSS_RE = re.compile(r'(\d+)\s+maximum resident set size')


def mac_time_available():
    if sys.platform != 'darwin' or not Path('/usr/bin/time').is_file():
        return False
    probe = subprocess.run(['/usr/bin/time', '-l', '/usr/bin/true'], capture_output=True, text=True)
    return probe.returncode == 0 and RSS_RE.search(probe.stderr) is not None


def run_measured(cmd, use_time, timeout=120):
    if use_time:
        p = subprocess.Popen(['/usr/bin/time', '-l', *cmd], stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, text=True, start_new_session=True)
        try:
            out, err = p.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            os.killpg(p.pid, signal.SIGKILL)
            p.communicate()
            raise
        rss = RSS_RE.search(err)
        if p.returncode or not rss:
            raise RuntimeError(f'{cmd}: exit {p.returncode}, RSS missing or failed\n{err[-2000:]}')
        return out, int(rss.group(1))

    # wait4 returns this process's high-water mark. Separate files keep a
    # failing child's stderr from filling a pipe while its parent waits.
    with tempfile.TemporaryFile(mode='w+t') as stdout, tempfile.TemporaryFile(mode='w+t') as stderr:
        p = subprocess.Popen(cmd, stdout=stdout, stderr=stderr)
        deadline = time.monotonic() + timeout
        while True:
            pid, status, usage = os.wait4(p.pid, os.WNOHANG)
            if pid:
                p.returncode = os.waitstatus_to_exitcode(status)
                break
            if time.monotonic() >= deadline:
                p.kill()
                p.wait()
                raise subprocess.TimeoutExpired(cmd, timeout)
            time.sleep(0.001)
        stdout.seek(0)
        stderr.seek(0)
        if p.returncode:
            raise RuntimeError(f'{cmd}: exit {p.returncode}\n{stderr.read()[-2000:]}')
        return stdout.read(), usage.ru_maxrss * (1 if sys.platform == 'darwin' else 1024)


def measure_one(exe, path, op, want, use_time):
    out, rss = run_measured([exe, path, op, str(FLAG)], use_time)
    rows = [line.split() for line in out.splitlines()]
    if len(rows) != 6 or [r[0] for r in rows] != ['warm'] + ['run'] * 5:
        raise RuntimeError(f'unexpected harness output: {out!r}')
    if any((int(r[2]), int(r[3])) != (len(want), want[0]) for r in rows):
        raise RuntimeError(f'{exe}: timed packet shape differs from oracle: {out!r}')
    warm, *ms = [float(r[1]) for r in rows]
    return dict(median_ms=statistics.median(ms), warmup_ms=warm, samples_ms=ms, rss_bytes=rss)


def build_runner(rustc, work, module, *flags):
    work.mkdir(parents=True, exist_ok=True)
    harness = work / 'main.rs'
    harness.write_text((HAND_SRC / 'main.rs').read_text().replace(
        'use l3_claude_handrust as sol;', f'#[path = {json.dumps(str(module), ensure_ascii=False)}] mod sol;'))
    exe = work / 'runner'
    subprocess.run([rustc, '-O', '--edition', '2021', harness, '-o', exe, *flags], check=True, timeout=120)
    return exe


def build_pair(g2, pair, scratch, str_adapter):
    compiler, backend = pair.split('/')
    work = scratch / f'{compiler}_{backend}'
    work.mkdir()
    start = time.monotonic()
    extern = f'pruv_str={str_adapter}'
    compiled = g2.compile_pruv(compiler, str(HERE / 'a4_root.pruv'), str(work), backend,
                               ['--rust-extern', extern, '--rust-search', str(str_adapter.parent)])
    if compiled['rc'] != 0:
        raise RuntimeError(f'{pair}: compile/verify failed: {compiled["errors"] or compiled["stderr"]}')
    compile_s = time.monotonic() - start
    objects = []
    if compiled['ll']:
        objects, error = g2.build_llvm_objects(compiled['ll'], 'a4', str(work))
        if error:
            raise RuntimeError(error)
    exe = build_runner(g2.RUSTC, work, compiled['rs'], *objects, '--extern', extern)
    return exe, {'compile_verify_seconds': compile_s, 'build_seconds': time.monotonic() - start}


def compare(pair, name, op, path, want, pruv, hand, rounds, use_time):
    # Each expected word (including all widened text bytes) is compared outside
    # timing. Both comparisons to the same oracle packet imply pairwise equality.
    check_packet(pruv, path, op, FLAG, want)
    samples = {'pruv': [], 'hand': []}
    for i in range(rounds):
        lanes = [('pruv', pruv), ('hand', hand)]
        for side, exe in (lanes if i % 2 == 0 else reversed(lanes)):
            samples[side].append(measure_one(exe, path, op, want, use_time))

    size = size_class(name)
    name_key = f'{name}/{op}/{pair}'
    fixed = op != 'window' and path.stat().st_size > EDGE
    tiny = all(statistics.median(s['warmup_ms'] + sum(s['samples_ms']) for s in lane) < 1
               for lane in samples.values())
    cells = []
    for axis, metric, unit in [('time', 'median_ms', 'ms'), ('rss', 'rss_bytes', 'bytes')]:
        p, h = [statistics.median(s[metric] for s in samples[side]) for side in ('pruv', 'hand')]
        cells.append(dict(name=name_key, axis=axis, size='S' if axis == 'time' and tiny else size,
                          kind='fixed' if axis == 'time' and fixed else 'proportional',
                          pruv=p, hand=h, unit=unit, runs=rounds, ratio=p / h if h else None))
    return cells, samples


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--only', default='', help='measure only inputs whose name contains this')
    ap.add_argument('--pairs', default='py/rust,py/llvm,rs/rust,rs/llvm')
    ap.add_argument('--quick', action='store_true', help='one fresh-process round, rs/rust only')
    ap.add_argument('--out', type=Path, default=DEFAULT_OUT, help='path for perf_cells.json')
    args = ap.parse_args()
    inputs = [(name, data) for name, data in build_inputs() if args.only in name]
    if not inputs:
        ap.error('--only matched no inputs')
    rounds = 1 if args.quick else 7
    pairs = ['rs/rust'] if args.quick else args.pairs.split(',')
    if not all(p in ('py/rust', 'py/llvm', 'rs/rust', 'rs/llvm') for p in pairs):
        ap.error('unknown compiler/backend pair')

    use_time = mac_time_available()
    rss_method = '/usr/bin/time -l' if use_time else 'wait4 ru_maxrss'
    print(f'RSS: {rss_method}', flush=True)
    report = dict(source='claude/r08', cells=[], builds={}, samples={}, errors=[], flag=FLAG,
                  rss_method=rss_method, platform=platform.platform(), scratch_rust_flags='rustc -O',
                  notes=[
                      'Both lanes use rustc -O (opt-level 3); neither requests LTO or a codegen-units override.',
                      'Each process reads input, warms once, times five calls and drops each owned packet outside the timer.',
                      'RSS covers the whole process including input; oversized edge rejection time is fixed.',
                      'The ring starts at 16 slots (1,049,344 bytes) and can grow to 200 (13,116,800 bytes).',
                      'turns16 ends with the 16th user reply to exercise growth on that parsed object.',
                      'S includes turns0/1/10; M includes turns16/17; turns200 and the heavy historical inputs are L.',
                      'Time cells with median warm-up plus five calls under 1 ms on both lanes are S.',
                  ])
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='claude-r08-') as temporary:
        scratch = Path(temporary)
        t4.WORK = scratch
        os.environ['TMPDIR'] = str(scratch)
        rustc = Path(os.environ.get('RUSTC', str(Path.home() / '.cargo/bin/rustc')))
        # Use the same rustc entry point as the Pruv build when it is on PATH.
        rustc = shutil.which('rustc') or rustc
        hand = build_runner(rustc, scratch / 'hand', HAND_SRC / 'lib.rs')
        pruv = {}
        try:
            g2, _ = t4.configure()
            report['rustc'] = subprocess.check_output([g2.RUSTC, '--version', '--verbose'], text=True).strip()
            report['clang'] = subprocess.run([g2.CLANG, '--version'], capture_output=True, text=True, timeout=30).stdout.strip()
            adapter = scratch / 'libpruv_str.rlib'
            subprocess.run([g2.RUSTC, '-O', '--edition', '2021', '--crate-name', 'pruv_str',
                            '--crate-type', 'rlib', t4.JSON_DIR / 'str_adapter/src/lib.rs', '-o', adapter],
                           check=True, timeout=120)
            for pair in pairs:
                try:
                    print(f'{pair} 빌드 중', flush=True)
                    pruv[pair], report['builds'][pair] = build_pair(g2, pair, scratch, adapter)
                except (RuntimeError, OSError, subprocess.SubprocessError) as exc:
                    report['errors'].append(str(exc))
                    print(exc, file=sys.stderr, flush=True)
        except (RuntimeError, OSError, subprocess.SubprocessError) as exc:
            report['errors'].append(str(exc))
            print(exc, file=sys.stderr, flush=True)

        print('| 입력 | 연산 | 페어 | Hand ms | Pruv ms | 시간 배율 | Hand MiB | Pruv MiB | RSS 배율 |', flush=True)
        print('|---|---|---|---:|---:|---:|---:|---:|---:|', flush=True)
        hand_checks = 0
        pair_checks = 0
        with (args.out.parent / 'fixtures.json').open('w') as fixtures:
            fixtures.write('[\n')
            for name, data in inputs:
                path = scratch / f'{name}.jsonl'
                path.write_bytes(data)
                encoded = base64.b64encode(data).decode()
                for op in OPS:
                    expected = oracle.api(op, FLAG, data)
                    if hand_checks:
                        fixtures.write(',\n')
                    json.dump(dict(name=f'{op}{FLAG}: q_{name}', op=op, flag=FLAG, data=encoded, expected=expected), fixtures)
                    want = array('q', expected)
                    check_packet(hand, path, op, FLAG, want)
                    hand_checks += 1
                    if not pruv:
                        # A broken or concurrently edited Pruv root still leaves
                        # usable hand smoke evidence and no invented ratios.
                        sample = measure_one(hand, path, op, want, use_time)
                        print(f'| {name} | {op} | hand | {sample["median_ms"]:.6f} | — | — | '
                              f'{sample["rss_bytes"] / 1048576:.3f} | — | — |', flush=True)
                    for pair, exe in pruv.items():
                        try:
                            cells, samples = compare(pair, name, op, path, want, exe, hand, rounds, use_time)
                            pair_checks += 1
                        except (RuntimeError, OSError, subprocess.SubprocessError, ValueError) as exc:
                            report['errors'].append(f'{name}/{op}/{pair}: {exc}')
                            print(report['errors'][-1], file=sys.stderr, flush=True)
                            continue
                        report['cells'].extend(cells)
                        report['samples'][cells[0]['name']] = samples
                        t, r = cells
                        print(f'| {name} | {op} | {pair} | {t["hand"]:.6f} | {t["pruv"]:.6f} | {t["ratio"]:.3f} | '
                              f'{r["hand"] / 1048576:.3f} | {r["pruv"] / 1048576:.3f} | {r["ratio"]:.3f} |', flush=True)
            fixtures.write('\n]\n')
        report['packet_checks'] = dict(hand_vs_oracle=hand_checks, pruv_vs_hand_vs_oracle=pair_checks)
        args.out.write_text(json.dumps(report, indent=2, ensure_ascii=False) + '\n')
        print(f'완전 일치: hand/oracle {hand_checks}개, Pruv/hand/oracle {pair_checks}개', flush=True)
        print(f'{args.out} ({len(report["cells"])}개 셀)', flush=True)
    return int(bool(report['errors']))


if __name__ == '__main__':
    raise SystemExit(main())
