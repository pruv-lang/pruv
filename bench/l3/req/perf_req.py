"""L3-9 complexity / performance measurement (spec 13).

Grid: list and open scenarios, S = 1000/5000/20000 sessions, M = 1/10/50 list items (open has no M), K = 8 candidates per session
(original thread, 3 forks, terminal cliSession, 3 engineThreads slots), placements none / first / last:
  none   no session carries the requested thread(s): the whole session list is walked (worst case)
  first  the requested thread(s) sit in the first session(s) (list: item j matches session j; open: session 0)
  last   the requested thread(s) sit in the last session(s)  (list: item j matches session S-M+j; open: session S-1)
The matching candidate is always the LAST of the K candidates of its session (worst case inside the session).
Each scenario runs once on work/dev/harness and once on the Node oracle (oracle skipped past --node-limit seconds, default 120, the
timeout kill is recorded). Wall time is the whole process (start, scenario parse, run, observation print), RSS from /usr/bin/time -l.
Assertion (spec 13): list, no match, all temporary flags false => sessionList calls == M+1 and isTemporary calls == M*S, counted from
the observation trace of every run that finished; a Pruv list that needs more than 1,000,000 external calls (spec 7.2) must end
with core InputTooLarge instead. No pass threshold is declared. Results: work/perf_report.md.

  python3 perf_req.py [--sizes 1000,5000,20000] [--ms 1,10,50] [--ops list,open] [--no-node] [--node-limit 120] [--timeout 600]
  exit 0 asserted counts hold, 1 an assertion failed or a run failed, 2 nothing could be measured
"""
import sys
sys.dont_write_bytecode = True
import argparse
import json
import os
import re
import signal
import subprocess
import tempfile
import time
from collections import Counter
from pathlib import Path

import common

HERE = common.HERE
HARNESS = HERE / 'work' / 'dev' / 'harness'
REPORT = HERE / 'work' / 'perf_report.md'
K = 8
TIME = '/usr/bin/time'
ORACLE_CMD = [common.NODE, '--experimental-strip-types', '--no-warnings', '--import', str(HERE / 'reference-hooks.mjs'), str(HERE / 'reference.mjs')]
IMPORTED_AT = '2023-11-14T22:13:20.000Z'


def make_session(i, match_thread=None):
    """A session with K candidates; match_thread (if any) is the last candidate (an engineThreads slot of engine claude)."""
    sess = {'id': f's{i}', 'title': f'T{i}',
            'externalThread': {'engine': 'claude', 'source': 'claude:home', 'thread': f'o{i}', 'mode': 'fork', 'importedAt': IMPORTED_AT,
                               'forks': [f'f{i}_{k}' for k in range(3)]},
            'terminal': {'cliSession': f'c{i}'},
            'engineThreads': {'claude:w1': {'threadId': f'e{i}_1'}, 'claude:w2': {'threadId': f'e{i}_2'},
                              'claude:w3': {'threadId': match_thread if match_thread is not None else f'e{i}_3'}}}
    return sess


def sessions_for(S, matches):
    """matches: {session index: thread}"""
    return [make_session(i, matches.get(i)) for i in range(S)]


def placement_matches(S, M, placement, op):
    n = M if op == 'list' else 1
    name = (lambda j: f't{j}') if op == 'list' else (lambda j: 'tgt')
    if placement == 'none':
        return {}
    base = 0 if placement == 'first' else S - n
    return {base + j: name(j) for j in range(n)}


def scenario(op, S, M, placement):
    sessions = sessions_for(S, placement_matches(S, M, placement, op))
    world = {'sessions': sessions}
    if op == 'list':
        world['readers'] = {'claude': {'list': [{'ok': [{'thread': f't{j}', 'title': f'Item {j}'} for j in range(M)]}]}}
        start = {'ev': 'start', 'req': 'r1', 'op': 'list', 'input': {'source': 'claude:home', 'folder': '/proj'}}
    else:
        world['readers'] = {'claude': {'read': [{'ok': {'title': 'T', 'turns': [{'prompt': f'p{k}', 'reply': f'r{k}', 'tools': 0, 'clipped': False} for k in range(3)]}}]}}
        start = {'ev': 'start', 'req': 'r1', 'op': 'open', 'input': {'source': 'claude:home', 'folder': '/proj', 'thread': 'tgt', 'mode': 'fork'}}
    return {'id': f'perf-{op}-{S}-{M}-{placement}', 'world': world, 'script': [start]}


def est_packet_words(sc):
    """ESTIMATE (the harness does not expose packet words): the largest single session as 3-word value slots plus one word per
    UTF-16 string unit (an upper bound of the pool), by walking its JSON. Sessions are streamed one per sessionList.next, so one
    session is the largest value packet; this is not a measurement of the real packet length."""
    def walk(v):
        if isinstance(v, dict):
            return sum(walk(x) + 3 + len(k) for k, x in v.items()) + 3
        if isinstance(v, list):
            return sum(walk(x) for x in v) + 3
        if isinstance(v, str):
            return 3 + len(v.encode('utf-16-le')) // 2
        return 3
    return max(walk(s) for s in sc['world']['sessions'])


def run_timed(cmd, infile, outfile, timeout):
    """-> (status, wall seconds, peak RSS bytes|None). status ok | timeout | fail:<rc>."""
    errfile = outfile.with_suffix('.err')
    t0 = time.perf_counter()
    with open(infile, 'rb') as fi, open(outfile, 'wb') as fo, open(errfile, 'wb') as fe:
        proc = subprocess.Popen([TIME, '-l'] + cmd, stdin=fi, stdout=fo, stderr=fe, cwd=HERE, start_new_session=True)
        try:
            proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            os.killpg(proc.pid, signal.SIGKILL)  # kill time AND the measured child (an orphan would skew later timings)
            proc.wait()
            return 'timeout', time.perf_counter() - t0, None
        p = proc
    wall = time.perf_counter() - t0
    err = errfile.read_text(errors='replace')
    m = re.search(r'(\d+)\s+maximum resident set size', err)
    rss = int(m.group(1)) if m else None
    return ('ok' if p.returncode == 0 else f'fail:{p.returncode}'), wall, rss


def read_obs(path):
    with open(path, encoding='utf-8') as f:
        line = f.readline()
    return json.loads(line)


def counts(obs):
    c = Counter(t.get('c') for t in obs['trace'])
    return {'sessionList': c['sessionList'], 'isTemporary': c['isTemporary'], 'next': c['sessionList.next'], 'trace': len(obs['trace']),
            'reader': c['reader'], 'createImported': c['createImported'],
            'too_large': any(isinstance(r, dict) and r.get('terminal') == 'core' and isinstance(r.get('value'), dict)
                             and r['value'].get('code') == 'InputTooLarge' for r in obs.get('requests', []))}


# spec 7.2: at most 1,000,000 external calls per scenario. A no-match list needs at least M*S isTemporary calls and M+1
# sessionList calls, so past this bound Pruv must end with core InputTooLarge (P6); Node has no bound.
CALL_BOUND = 1000000


def fmt_s(x):
    return '-' if x is None else f'{x:.2f}'


def fmt_mb(x):
    return '-' if x is None else f'{x / 1048576:.0f}'


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--sizes', default='1000,5000,20000')
    ap.add_argument('--ms', default='1,10,50')
    ap.add_argument('--ops', default='list,open')
    ap.add_argument('--no-node', action='store_true')
    ap.add_argument('--node-limit', type=float, default=120)
    ap.add_argument('--timeout', type=float, default=120, help='harness timeout, seconds (a timed-out cell is reported, larger S of the group is not run)')
    ap.add_argument('--report', default=str(REPORT))
    args = ap.parse_args(argv)
    if not HARNESS.is_file():
        print(f'harness missing: {HARNESS} (build with sh dev_build.sh)')
        return 2
    sizes = [int(x) for x in args.sizes.split(',')]
    ms = [int(x) for x in args.ms.split(',')]
    rows, failures = [], []
    with tempfile.TemporaryDirectory(prefix='req-perf-') as tmp:
        tmp = Path(tmp)
        for op in args.ops.split(','):
            for M in (ms if op == 'list' else [0]):
                for placement in ('none', 'first', 'last'):
                    timed_out = False
                    for S in sizes:
                        if timed_out:  # a smaller S already hit the harness timeout: larger S is not run
                            rows.append({'op': op, 'S': S, 'M': M, 'placement': placement, 'h_status': 'not run (smaller S timed out)', 'h_wall': None, 'packet_est': '-'})
                            continue
                        sc = scenario(op, S, M, placement)
                        infile = tmp / 'in.jsonl'
                        infile.write_text(json.dumps(sc, ensure_ascii=True) + '\n')
                        row = {'op': op, 'S': S, 'M': M, 'placement': placement, 'packet_est': est_packet_words(sc)}
                        st, wall, rss = run_timed([str(HARNESS)], infile, tmp / 'h.out', args.timeout)
                        row.update(h_status=st, h_wall=wall if st == 'ok' else None, h_rss=rss)
                        if st == 'ok':
                            row['h_counts'] = counts(read_obs(tmp / 'h.out'))
                        if not args.no_node:
                            st, wall, rss = run_timed(ORACLE_CMD, infile, tmp / 'n.out', args.node_limit)
                            row.update(n_status=st, n_wall=wall if st == 'ok' else None, n_rss=rss)
                            if st == 'ok':
                                row['n_counts'] = counts(read_obs(tmp / 'n.out'))
                        if op == 'list' and placement == 'none':
                            for side in ('h', 'n'):
                                c = row.get(f'{side}_counts')
                                if c is None:
                                    continue
                                if side == 'h' and M * S + M + 1 > CALL_BOUND:
                                    row['bound'] = True
                                    if not c['too_large']:
                                        failures.append(f'{op} S={S} M={M} h: needs more than {CALL_BOUND} calls but did not end with InputTooLarge')
                                    continue
                                if c['sessionList'] != M + 1 or c['isTemporary'] != M * S:
                                    failures.append(f'{op} S={S} M={M} {side}: sessionList {c["sessionList"]} (want {M + 1}), isTemporary {c["isTemporary"]} (want {M * S})')
                        timed_out = row['h_status'] == 'timeout'
                        if row['h_status'] not in ('ok', 'timeout'):
                            failures.append(f'{op} S={S} M={M} {placement}: harness {row["h_status"]}')
                        rows.append(row)
                        print(f'  done {op} S={S} M={M} {placement}: pruv {fmt_s(row["h_wall"])}s', file=sys.stderr, flush=True)
    return report(rows, failures, args)


def report(rows, failures, args):
    out = ['# L3-9 request handlers: complexity / performance (spec 13)', '',
           f'Host: macOS, `work/dev/harness` (Pruv, rs/rust) and the Node oracle (`reference.mjs`), one scenario per process, K={K} candidates per session. '
           'Wall time is the whole process (start, scenario parse, run, observation print incl. the full trace); RSS is `/usr/bin/time -l` maximum resident set size. '
           'No pass threshold is declared. Node runs past the limit are killed and marked `skip>Ns`.', '',
           '## Rows', '',
           '| op | S | M | match | TS s | Pruv s | TS RSS MB | Pruv RSS MB | packet words (est.) | sessionList | isTemporary | sessionList.next | trace lines | reader |',
           '|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|']
    for r in rows:
        c = r.get('h_counts') or {}
        n = r.get('n_status', 'off')
        ts = fmt_s(r.get('n_wall')) if n == 'ok' else (f'skip>{args.node_limit:.0f}s' if n == 'timeout' else ('not run' if n == 'off' else n))
        out.append(f'| {r["op"]} | {r["S"]} | {r["M"] if r["op"] == "list" else "-"} | {r["placement"]} | {ts} | {fmt_s(r["h_wall"]) if r["h_status"] == "ok" else ("timeout>" + str(int(args.timeout)) + "s" if r["h_status"] == "timeout" else r["h_status"])} | '
                   f'{fmt_mb(r.get("n_rss"))} | {fmt_mb(r.get("h_rss"))} | {r["packet_est"]} | {c.get("sessionList", "-")} | {c.get("isTemporary", "-")} | {c.get("next", "-")} | {c.get("trace", "-")} | {c.get("reader", "-")} |')
    out += ['', '## Pruv time growth S=5000 -> S=20000 at fixed M (linear = 4.0; ratio > 6 is flagged)', '',
            '| op | M | match | Pruv s at 5000 | Pruv s at 20000 | ratio | flag |', '|---|---:|---|---:|---:|---:|---|']
    idx = {(r['op'], r['M'], r['placement'], r['S']): r for r in rows}
    suspect = []
    for (op, M, pl, S), r in sorted(idx.items()):
        if S != 5000 or (op, M, pl, 20000) not in idx:
            continue
        a, b = r['h_wall'], idx[(op, M, pl, 20000)]['h_wall']
        if a is None or b is None:
            out.append(f'| {op} | {M or "-"} | {pl} | {fmt_s(a)} | {fmt_s(b)} | - | run missing |')
            continue
        ratio = b / a
        flag = 'SUSPECT-QUADRATIC' if ratio > 6 else ''
        if flag:
            suspect.append(f'{op} M={M} {pl}: {ratio:.1f}')
        out.append(f'| {op} | {M or "-"} | {pl} | {a:.2f} | {b:.2f} | {ratio:.2f} | {flag} |')
    out += ['', '## Local growth exponent of Pruv time between consecutive measured S (time ~ S^e; 1.0 linear, 2.0 quadratic)', '',
            '| op | M | match | S pair | Pruv s | exponent |', '|---|---:|---|---|---|---:|']
    import math
    for (op, M, pl, S), r in sorted(idx.items()):
        nxt = [k for k in idx if k[:3] == (op, M, pl) and k[3] > S]
        if not nxt or r['h_wall'] is None or pl == 'first':
            continue
        S2 = min(k[3] for k in nxt)
        r2 = idx[(op, M, pl, S2)]
        if r2['h_wall'] is None or r['h_wall'] < 0.3:
            continue
        out.append(f'| {op} | {M or "-"} | {pl} | {S}->{S2} | {r["h_wall"]:.2f} -> {r2["h_wall"]:.2f} | {math.log(r2["h_wall"] / r["h_wall"]) / math.log(S2 / S):.2f} |')
    out += ['', '## Assertion (spec 13): list, all temporary flags false, no match', '',
            'sessionList calls == M+1 and isTemporary calls == M*S, counted from the observation trace of each finished run (Pruv and Node); '
            'a Pruv cell whose list needs more than 1,000,000 external calls (M*S + M + 1, spec 7.2) must instead end with core InputTooLarge: '
            + ('**FAILED**' if failures else 'holds for every finished run') + '.']
    out += [f'- {f}' for f in failures]
    out += ['', '## Notes', '',
            '- Packet words: the harness does not expose them; the column is an estimate (largest single session as 3-word slots plus one word per UTF-16 unit, sessions being streamed one per `sessionList.next`), not a measurement.',
            '- Timing covers trace printing: the trace has one line per session visited (`sessionList.next`, `isTemporary`), so process time includes trace cost; core cost is not separated by this script (trace on/off is not switchable in the harness).',
            '- Replayed items: not exposed by the harness; not reported.',
            '- A cell that timed out is shown as timeout>Ns; larger S of that group was then not run, so the 5000->20000 ratio is missing there (a timeout is itself evidence of super-linear growth only together with the exponent table).',
            '- Another worker may have run q_mutants concurrently; times are wall time on a shared machine.',
            '- Single run per cell (no repetition), so small-S times are noisy.',
            '- SUSPECT-QUADRATIC rows: ' + (', '.join(suspect) if suspect else 'none')]
    text = '\n'.join(out) + '\n'
    Path(args.report).parent.mkdir(parents=True, exist_ok=True)
    Path(args.report).write_text(text, encoding='utf-8')
    print(text)
    if not any(r['h_status'] == 'ok' for r in rows):
        return 2
    return 1 if failures else 0


if __name__ == '__main__':
    sys.exit(main())
