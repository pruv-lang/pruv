#!/usr/bin/env python3
"""L3-2 Q measurement: a child writes 64 MiB in 64 KiB chunks; wall time of one supervised run, median of 5.

Pruv: the q.py runners (rs/rust, rs/llvm), one fixture line per process. TS: reference/drive.mjs with one case per
process, so the time includes Node's start and the forked supervisor's start; the 'ts start only' row runs an
empty child to show that part. Run q.py first (it builds the runners). Writes results/measure.json.
"""
import json
import statistics
import subprocess
import sys
import time

sys.dont_write_bytecode = True
import common  # noqa: E402
import test_a3  # noqa: E402

HERE = common.HERE
WORK = HERE / 'work' / 'q'
N = 5
BIG = 'import sys; b=bytes(range(256))*256\nfor i in range(1024): sys.stdout.buffer.write(b)'


def line(code, out):
    return ' '.join(['wait', '0', test_a3.hx(sys.executable), test_a3.hx(test_a3.blob(['-c', code])), '-',
                     test_a3.hx(out), str(1 << 30), '4096'])


def timed(cmd, inp=None):
    t = time.perf_counter()
    r = subprocess.run(cmd, input=inp.encode() if inp else None, capture_output=True, timeout=600)
    dt = time.perf_counter() - t
    if r.returncode != 0:
        raise RuntimeError(r.stderr[-300:])
    return dt, r.stdout.decode('utf-8', 'replace')


def main():
    rows = {}
    out = str(WORK / 'measure.bin')
    for key in ('rs_rust', 'rs_llvm'):
        exe = str(WORK / key / 'runner')
        ts = []
        for _ in range(N):
            dt, o = timed([exe], line(BIG, out) + '\n')
            assert o.split(' | ')[2].startswith('67108864:'), o[-100:]
            ts.append(dt)
        rows[key.replace('_', '/')] = ts
    for name, code in (('ts', BIG), ('ts start only', 'pass')):
        cf, rf = WORK / 'm_cases.json', WORK / 'm_results.json'
        cf.write_text(json.dumps([dict(shell=sys.executable, args=['-c', code], cwd='', out=out + '.ts',
                                       limit=1 << 30, tail=4096, mode='wait', delay=0)]))
        ts = []
        for _ in range(N):
            dt, _ = timed(['node', str(HERE / 'reference' / 'drive.mjs'), str(cf), str(rf)])
            ts.append(dt)
        if code == BIG:
            assert json.loads(rf.read_text())[0]['exit']['fileBytes'] == 1 << 26
        rows[name] = ts
    rows['child alone'] = [timed([sys.executable, '-c', BIG])[0] for _ in range(N)]
    res = {k: dict(median_s=round(statistics.median(v), 3), runs=[round(x, 3) for x in v]) for k, v in rows.items()}
    for k, v in res.items():
        print(f'{k}: {v["median_s"]} s', flush=True)
    (HERE / 'results' / 'measure.json').write_text(json.dumps(res, indent=1) + '\n')
    return 0


if __name__ == '__main__':
    sys.exit(main())
