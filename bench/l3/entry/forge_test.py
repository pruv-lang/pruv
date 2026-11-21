#!/usr/bin/env python3
"""WA forged-replay (R1), exact pool length and deep-path request count (R3) tests.

Runs work/dev/forge_harness (see forge.rs, forge_build.sh) once per scenario. The harness tampers with the prev
packet of one entry_step call. Host probes forge the input or a step completion's pool. Honest controls
must finish with terminal ok; every forgery must end in
terminal boundary_error / InvalidPacket.  Usage: python3 forge_test.py
"""
import json, os, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.join(HERE, 'work/dev/forge_harness')
ACC = 'acc_0123456789abcdef'
C01 = {"op": "codex_account_folder",
       "args": {"dataDir": "/data", "account": {"id": ACC, "home": "/data/engine-homes/codex/" + ACC}},
       "script": {"homedir": [{"ok": "/home/test"}], "cwd": [{"ok": "/work"}],
                  "realpath": {"/data": [{"ok": "/real/data"}]}, "ambient": [{"ok": "/amb"}]}}
# op 1 with an unset ambient CODEX_HOME: step 4 resumes the Codex home item with the ambient answer (seq 3) in its log
C1 = {"op": "import_sources", "args": {"dataDir": "/data", "env": {}},
      "script": {"homedir": [{"ok": "/home/test"}], "cwd": [{"ok": "/work"}],
                 "accounts": {"claude": {"ok": []}, "codex": {"ok": []}}}}
OK = ('ok', None)
BAD = ('boundary_error', {"code": "InvalidPacket"})
C9 = {"op": "bound_session_index", "args": {"sessions": [
    {"id": "a", "temporary": False, "externalThread": {"engine": "claude", "thread": "a"}},
    {"id": "b", "temporary": True, "externalThread": {"engine": "claude", "thread": "b"}},
]}}

def run(case, tamper=None, stats=False):
    env = dict(os.environ)
    if tamper is not None:
        env['FORGE_TAMPER'] = json.dumps(tamper)
    if stats:
        env['FORGE_STATS'] = '1'
    p = subprocess.run([BIN], input=json.dumps(case), capture_output=True, text=True, env=env)
    out = json.loads(p.stdout.strip().splitlines()[-1])
    return out, p.stderr

def T(at, *ops):
    return {"at": at, "ops": list(ops)}

# (name, tamper, expected). Step 8 of C01 has 3 log entries, step 12 has 4.
SCENARIOS = [
    ('control: untouched replay', None, OK),
    ('control: no-op tamper at step 8', T(8), OK),
    ('control: no-op tamper at step 12', T(12), OK),
    # R1a: recorded outstanding request vs regenerated
    ('R1a record word changed', T(8, {"rec": [0, 1]}), BAD),
    ('R1a request code changed', T(8, {"code": 3}), BAD),
    ('R1a last entry request code changed', T(8, {"entry_rc": [2, 7]}), BAD),
    # R1b: op/seq continuity; forged entries
    ('R1b entries swapped (reordered)', T(8, {"swap": [0, 1]}), BAD),
    ('R1b first and last swapped', T(12, {"swap": [0, 3]}), BAD),
    ('R1b entry dropped (never answered)', T(8, {"drop": 0}), BAD),
    ('R1b entry duplicated', T(8, {"dup": 0}), BAD),
    ('R1b entry for a never issued request (seq far ahead)', T(8, {"entry_seq": [2, 99]}), BAD),
    ('R1b entry seq zero', T(8, {"entry_seq": [0, 0]}), BAD),
    ('R1b entry with foreign op', T(8, {"entry_op": [1, 9]}), BAD),
    ('R1b appended entry for an unissued seq', T(8, {"push": [0, 40]}), BAD),
    ('R1b appended entry reusing a seq', T(8, {"push": [2, 3]}), BAD),
    # R1c: leftover entries (the appended entries above also leave the log unconsumed)
    ('R1c extra entry after the last', T(12, {"push": [3, 6]}), BAD),
    # R1d: checkpoint consistency
    ('R1d phase out of range', T(8, {"ck": [0, 5]}), BAD),
    ('R1d idx beyond count', T(8, {"ck": [1, 9]}), BAD),
    ('R1d reserved ck6 nonzero', T(8, {"ck": [6, 1]}), BAD),
    ('R1d climb depth without matching drops', T(8, {"ck": [3, 7]}), BAD),
    ('R1d dropped-count word inconsistent', T(8, {"ck": [7, 100]}), BAD),
]

def deep_case(n, rel=False):
    d = '/'.join('d%d' % i for i in range(n))
    if not rel:
        d = '/' + d
    return {"op": "codex_account_folder",
            "args": {"dataDir": d, "account": {"id": ACC, "home": d + "/engine-homes/codex/" + ACC}},
            "script": {"homedir": [{"ok": "/home/test"}], "cwd": [{"ok": "rel" if rel else "/work"}],
                       "ambient": [{"ok": "/amb"}]}}

def deep_stats(n, rel=False):
    out, err = run(deep_case(n, rel), stats=True)
    rows = [l.split() for l in err.splitlines() if l.startswith('step ')]
    reqs = len(out.get('trace') or [])
    mx = max(int(r[7]) for r in rows) if rows else 0
    msl = max(int(r[5]) for r in rows) if rows else 0
    return out['terminal'], len(rows), reqs, mx, msl

def main():
    fails = 0
    for corrupt in ('extra_word', 'missing_word'):
        for at, phase in [(0, 'begin'), (1, 'step')]:
            case = dict(C01, probe={'at': at, 'corrupt': corrupt})
            out, _ = run(case)
            got = (out.get('terminal'), out.get('value'))
            ok = got == BAD and len(out['trace']) == at
            fails += not ok
            print('%-4s %s-%s -> %s' % ('ok' if ok else 'FAIL', corrupt, phase, got))
    # The synthetic separator takes one byte but no logical UTF-16 unit.
    for name, session, units in [
        ('C3 separated surrogates', {'id': {'u16': [0xD800]}, 'temporary': {'u16': [0xDC00]}}, 2),
        ('C3 literal NUL', {'id': {'u16': [0xD800, 0, 0xDC00]}}, 3),
    ]:
        case = {"op": "bound_session_index", "args": {"sessions": [session]}}
        out, err = run(case, stats=True)
        table = 10 + units + 3
        want = f'index state {table} {units} {table + 30} progress {table}'
        actual = next((line for line in err.splitlines() if line.startswith('index state ')), '')
        ok = out.get('terminal') == 'ok' and actual == want
        fails += not ok
        print('%-4s %s -> %s' % ('ok' if ok else 'FAIL', name, actual))
    # C3 recheck: a literal [D800, 0000, DC00] ID next to an unreferenced high-separator-low tail in the pool. The repack
    # drops the tail and must keep the same unit count as the input without it; the separator byte is 0xFF (FORGE_SEP)
    sep = int(os.environ.get('FORGE_SEP', '255'))
    lit = {'u16': [0xD800, 0, 0xDC00]}
    for name, session in [('ID only', {'id': lit}),
                          ('ID with a thread', {'id': lit, 'temporary': False, 'externalThread': {'engine': 'claude', 'thread': 'a'}})]:
        case = {"op": "bound_session_index", "args": {"sessions": [session]}}
        base, berr = run(case, stats=True)
        out, err = run(case, dict(T(999), index_tail=sep), stats=True)
        # table and unit counts and progress; the third number is the raw input size, which the tail makes larger
        line = lambda e: next((' '.join(l.split()[:4] + l.split()[5:]) for l in e.splitlines() if l.startswith('index state ')), '')
        ok = out.get('terminal') == 'ok' and out.get('value') == base.get('value') and line(err) == line(berr) != ''
        fails += not ok
        print('%-4s C3 literal NUL %s, separated tail in the pool -> %s %s' % ('ok' if ok else 'FAIL', name, out.get('terminal'), line(err)))
    control, _ = run(C9)
    out, _ = run(C9, dict(T(1), index_swap=True))
    ok = (control.get('terminal'), control.get('value'), out.get('terminal'),
          out.get('value'), out.get('trace')) == ('ok', [['claude:a', 'a']], *BAD, [['isTemporary', 'b']])
    fails += not ok
    print('%-4s op 9 progress from A with the await from B -> %s' % ('ok' if ok else 'FAIL', out.get('terminal')))
    # R1 on op 1 (audit R1 forgery): the ambient answer (first request of its item, seq 3) is relabelled seq 1; the item start
    # seq kept in the state makes (start seq + dropped + kept log + current seq) inconsistent
    for name, tam, exp in [('control: op 1 untouched replay', None, OK),
                           ('control: op 1 no-op tamper at step 4', T(4), OK),
                           ('R1b op 1 entry seq below the item start', T(4, {"entry_seq": [0, 1]}), BAD),
                           ('R1b op 1 entry seq above the current request', T(4, {"entry_seq": [0, 9]}), BAD),
                           ('R1b op 1 dropped count claimed without a drop', T(4, {"ck": [7, 1]}), BAD),
                           ('R1b op 1 item start seq changed', T(4, {"ck": [0, 3 + 16 * 2]}), BAD)]:
        out, _ = run(C1, tam)
        got = (out.get('terminal'), out.get('value') if out.get('terminal') == 'boundary_error' else None)
        ok = got == exp
        fails += not ok
        print('%-4s %s -> %s' % ('ok' if ok else 'FAIL', name, got[0]))
    for name, tam, exp in SCENARIOS:
        out, _ = run(C01, tam)
        got = (out.get('terminal'), out.get('value') if out.get('terminal') == 'boundary_error' else None)
        ok = got == exp
        fails += not ok
        print('%-4s %s -> %s' % ('ok' if ok else 'FAIL', name, got[0]))
    print('deep path (segments, terminal, steps, host requests, max entries, max state words):')
    prev = None
    for n in (10, 20, 40, 80, 160):
        term, steps, reqs, mx, msl = deep_stats(n)
        print('  n=%-3d %s steps=%d requests=%d max_entries=%d max_state=%d' % (n, term, steps, reqs, mx, msl))
        if prev is not None and reqs > 3 * prev[1] * n / prev[0]:
            fails += 1
            print('FAIL request count grows faster than linear')
        prev = (n, reqs)
    print('relative deep path (cwd "rel"), bounded log and state (n, terminal, steps, requests, max entries, max state words):')
    for n in (10, 20, 40):
        term, steps, reqs, mx, msl = deep_stats(n, rel=True)
        _, _, _, amx, amsl = deep_stats(n)
        print('  n=%-3d %s steps=%d requests=%d max_entries=%d max_state=%d (absolute: %d, %d)' % (n, term, steps, reqs, mx, msl, amx, amsl))
        # constant log (not growing with n) and state within the path-size growth of the absolute run
        if term != 'ok' or mx > 12 or msl > amsl + 100:
            fails += 1
            print('FAIL relative deep climb: log entries or state size not bounded')
    print('failures:', fails)
    return 1 if fails else 0

if __name__ == '__main__':
    sys.exit(main())
