"""L3-9 integration cases (spec 9.2): 12 cases, run through BOTH the Node oracle and work/dev/harness.

Part A, injected-reader integration (8 cases I01-I08): Codex, Grok, OpenCode, Qwen list and open. The piece's own built runner
(bench/l3/<piece>/work/rs_rust/runner) runs once on an exact-domain fixture of that piece (same PICK choices as
bench/l3/entry/integrate.py; fake child / exporter / app-server, throwaway home, no account, token or network). The captured rows /
record become the engine reader result of a req scenario (world.readers.<engine>.<list|read>.ok). INJECTING A READER RESULT IS NOT
REAL CANCEL FORWARDING: no abort signal ever reaches the engine runner.
Part B, Grok callback lifecycle (3 cases I09-I11): copy/load success, verify failure, race after copy (copy / loadCheck are the
scenario fakes; the Grok record is the captured runner result).
Part C (1 case I12): an engine error captured from a piece runner is rejected from the reader and carried by the real pinned entry
wrapper (importFailure) to a request WireError.
Claude keeps the L3-1 "no driver" limit (reported, never stubbed).

For every case: oracle observation vs harness observation through compare.py (exact, or an explained class; unexplained fails) plus
semantic assertions on the oracle observation (result shape, host calls).

  python3 integrate.py [--case I01] [--list]     exit 0 all asserted, 1 an assertion failed, 2 only limits / a runner is missing
"""
import sys
sys.dont_write_bytecode = True
import argparse
import json
import os
import shutil
import subprocess
import tempfile
from pathlib import Path

import common
import compare
import oracle
from dev_compare import run_harness, first_diff

BENCH = common.HERE.parent
GROK_OPENCODE = r'''
import sys, json
sys.path.insert(0, %(piece)r)
import test_impl as t
r = t.load_fixtures()
cases = r[0] if isinstance(r, tuple) else r
case = [c for c in cases if c['name'] == %(name)r][0]
run = t.run_pair(case, 'req_int', %(exe)r)
f = case.get('folder'); th = case.get('thread')
print(json.dumps({'result': run['result'], 'returncode': run['returncode'], 'folder': f if isinstance(f, str) else None, 'thread': th if isinstance(th, str) else None}))
'''
CODEX = r'''
import sys, json
sys.path.insert(0, %(piece)r)
import common as c
cases = json.load(open(%(piece)r + '/fixtures/cases.json'))['cases']
case = [x for x in cases if x['name'] == %(name)r][0]
req = c.request_for(case, 'req_int', 0, c.fake_binary())
res = c.run_lines([%(exe)r], [req])[0]
print(json.dumps({'result': res, 'returncode': 0, 'folder': req['folder'], 'thread': req['thread'] if case['mode'] == 'read' else None}))
'''
# Same fixture choices as bench/l3/entry/integrate.py (exact domain of each piece).
PICK = {
    'codex': {'list': 'list_basic', 'read': 'read_basic', 'list_error': 'list_error'},
    'grok': {'list': 'listing-mixed-missing-3', 'read': 'budgets-200-turns-no-omit'},
    'opencode': {'list': 'F04-limit-49', 'read': 'F06-extra-slashes'},
    'qwen': {'list': 'fixtures/normal.jsonl in a throwaway home (runner --list home /project)', 'read': 'fixtures/normal.jsonl (runner <path>)'},
}
LIMITS = {'claude': 'L3-1 "no driver": bench/l3/claude is the pure packet API (head/tail/window packet); it has no listing or read driver '
                    'that returns import rows or an ExternalRecord, so there is no reader result to inject.'}


def norm(v):
    """JSON from the piece runners: integral floats (messages: 1.0) become ints so the scenario carries the same number to both sides."""
    if isinstance(v, float) and v == int(v):
        return int(v)
    if isinstance(v, list):
        return [norm(x) for x in v]
    if isinstance(v, dict):
        return {k: norm(x) for k, x in v.items()}
    return v


def run_py(code, piece, **kw):
    p = subprocess.run([sys.executable, '-c', code % dict(kw, piece=str(piece))], capture_output=True, text=True, cwd=piece, timeout=600)
    if p.returncode:
        raise RuntimeError(f'{piece.name} driver: exit {p.returncode}\n{p.stdout[-600:]}\n{p.stderr[-1000:]}')
    return json.loads(p.stdout.strip().split('\n')[-1])


def capture_qwen(mode):
    exe = BENCH / 'qwen/work/rs_rust/runner'
    src = BENCH / 'qwen/fixtures/normal.jsonl'
    with tempfile.TemporaryDirectory(prefix='req-int-qwen-') as tmp:
        home = Path(tmp) / 'home'
        chats = home / 'projects/-project/chats'
        chats.mkdir(parents=True)
        target = chats / 'thread1.jsonl'
        shutil.copy(src, target)
        os.utime(target, (1700000000, 1700000000))
        if mode == 'list':
            out = subprocess.run([str(exe), '--list', str(home), '/project'], capture_output=True, text=True, timeout=120)
        else:
            out = subprocess.run([str(exe), str(target)], capture_output=True, text=True, timeout=120)
    if out.returncode:
        raise RuntimeError(f'qwen runner exit {out.returncode}: {out.stderr[-600:]}')
    got = json.loads(out.stdout.strip().split('\n')[0])
    if mode == 'list':
        for r in got:  # the temp path is not part of the scenario (non-deterministic): keep a stable stand-in
            if 'path' in r:
                r['path'] = '/home/projects/-project/chats/' + r['thread'] + '.jsonl'
        return {'result': got, 'folder': '/project', 'thread': None}
    return {'result': got['record'], 'folder': '/project', 'thread': 'thread1'}


def capture(route, kind):
    """-> {fixture, folder, thread, rows|record|error}. record is an ExternalRecord ({title?, turns, omittedBefore, model?})."""
    piece = BENCH / route
    exe = str(piece / 'work/rs_rust/runner')
    if not Path(exe).is_file():
        raise RuntimeError(f'{route}: runner artifact missing ({exe}); build with bench/l3/{route}/test_impl.py')
    if route == 'qwen':
        got = capture_qwen(kind)
    else:
        got = run_py(CODEX if route == 'codex' else GROK_OPENCODE, piece, name=PICK[route][kind], exe=exe)
    res = got['result']
    out = {'fixture': PICK[route][kind], 'folder': got.get('folder') or '/work', 'thread': got.get('thread') or 't-1', 'runner': f'bench/l3/{route}/work/rs_rust/runner'}
    base = kind.split('_')[0]
    if route == 'codex':
        if isinstance(res, dict) and 'error' in res:
            out['error'] = {'code': res['error'], 'message': res['error']}
        elif base == 'list':
            out['rows'] = norm(res)
        else:
            out['record'] = norm(res)
    elif route in ('grok', 'opencode'):
        if res is None or res.get('status') != 'ok':
            e = (res or {}).get('error') or {}
            out['error'] = {'code': e.get('code') or 'read-failed', 'message': e.get('message') or str(e.get('code'))}
        elif base == 'list':
            out['rows'] = norm(res.get('rows'))
        else:
            rec = {'turns': norm(res.get('turns') if route == 'opencode' else res.get('rows')), 'omittedBefore': bool(res.get('omittedBefore'))}
            for k in ('title', 'model'):
                if res.get(k) is not None:
                    rec[k] = res[k]
            out['record'] = rec
    else:
        if base == 'list':
            out['rows'] = norm(res)
        else:
            out['record'] = norm(res)
    return out


# ---- scenarios -----------------------------------------------------------------------------------------------------------
def src(engine):
    return f'{engine}:home'


def bound_session(engine, thread, sid='s1', title='Lair title'):
    return {'id': sid, 'title': title, 'externalThread': {'engine': engine, 'source': src(engine), 'thread': thread, 'mode': 'fork',
                                                           'importedAt': '2023-11-14T22:13:20.000Z'}}


def list_scenario(cid, engine, cap):
    rows = cap['rows']
    # one row is bound to a Lair session (title replaced), one other session is unrelated; the rest stay unbound.
    sessions = [bound_session(engine, rows[0]['thread']), bound_session(engine, '__other__', 's2', 'Other')]
    return {'id': cid, 'world': {'readers': {engine: {'list': [{'ok': rows}]}}, 'sessions': sessions},
            'script': [{'ev': 'start', 'req': 'r1', 'op': 'list', 'input': {'source': src(engine), 'folder': cap['folder']}}]}


def open_scenario(cid, engine, cap, extra_world=None, script_tail=None):
    world = {'readers': {engine: {'read': [{'ok': cap['record']}]}}}
    if engine == 'grok':
        world['copy'] = [{'ok': {'path': '/grok/copy1', 'skipped': 0}}]
    world.update(extra_world or {})
    script = [{'ev': 'start', 'req': 'r1', 'op': 'open', 'input': {'source': src(engine), 'folder': cap['folder'], 'thread': cap['thread'], 'mode': 'fork'}}]
    return {'id': cid, 'world': world, 'script': script + (script_tail or [])}


def build_cases(caps):
    """-> [(id, group, label, scenario, checker)]"""
    cases = []
    n = 0
    for op in ('list', 'open'):
        for engine in ('codex', 'grok', 'opencode', 'qwen'):
            n += 1
            cid = f'I{n:02d}'
            cap = caps[(engine, 'list' if op == 'list' else 'read')]
            sc = list_scenario(cid, engine, cap) if op == 'list' else open_scenario(cid, engine, cap)
            cases.append((cid, 'A', f'{engine} {op}', sc, cap, check_list if op == 'list' else check_open))
    gcap = caps[('grok', 'read')]
    cases.append(('I09', 'B', 'grok copy+load success', open_scenario('I09', 'grok', gcap, {'copy': [{'ok': {'path': '/grok/copy1', 'skipped': 0}, 'verify': True}]}), gcap, check_grok_ok))
    cases.append(('I10', 'B', 'grok verify failure', open_scenario('I10', 'grok', gcap, {
        'copy': [{'ok': {'path': '/grok/copy1', 'skipped': 0}, 'verify': True}],
        'loadCheck': [{'reject': {'class': 'Error', 'ref': 'e1', 'message': 'cannot load'}}], 'errs': [{'class': 'Error', 'ref': 'e1', 'message': 'cannot load'}]}), gcap, check_grok_verify_fail))
    race_world = {'copy': [{'ok': {'path': '/grok/copy1', 'skipped': 0}, 'verify': True, 'defer': 'cp'}]}
    race_tail = [{'ev': 'mutate', 'op': 'add', 'session': bound_session('grok', gcap['thread'], 's9', 'Raced')}, {'ev': 'settle', 'label': 'cp'}]
    cases.append(('I11', 'B', 'grok race after copy', open_scenario('I11', 'grok', gcap, race_world, race_tail), gcap, check_grok_race))
    ecap = caps[('codex', 'list_error')]
    err = ecap['error']
    sc = {'id': 'I12', 'world': {'readers': {'codex': {'list': [{'reject': {'class': 'EngineImportError', 'ref': 'e1', 'code': err['code'], 'message': err['message']}}]}},
                                 'errs': [{'class': 'EngineImportError', 'ref': 'e1', 'code': err['code'], 'message': err['message']}], 'sessions': []},
          'script': [{'ev': 'start', 'req': 'r1', 'op': 'list', 'input': {'source': src('codex'), 'folder': ecap['folder']}}]}
    cases.append(('I12', 'C', f'codex list error ({err["code"]}) -> WireError', sc, ecap, check_error))
    return cases


# ---- semantic checks on the oracle observation ---------------------------------------------------------------------------
def req(obs, rid='r1'):
    return next(r for r in obs['requests'] if r['id'] == rid)


def calls(obs, name):
    return [t for t in obs['trace'] if t.get('c') == name]


def jv_items(v):
    """The list result {obj:[[items,{arr:[{obj:[[k,v],...]}]}]]} -> list of plain dicts (values left in JV form)."""
    try:
        arr = dict(v['obj'])['items']['arr']
        return [dict(x['obj']) for x in arr]
    except (KeyError, TypeError):
        return v


def check_list(obs, cap):
    bad = []
    r = req(obs)
    if r['terminal'] != 'ok':
        return [f'terminal {r["terminal"]}']
    items = jv_items(r['value'])
    if not isinstance(items, list) or len(items) != len(cap['rows']):
        return [f'{len(items) if isinstance(items, list) else items} items for {len(cap["rows"])} rows']
    if [i.get('thread') for i in items] != [x['thread'] for x in cap['rows']]:
        bad.append('thread order differs')
    if len(calls(obs, 'reader')) != 1:
        bad.append('reader not called exactly once')
    return bad


def check_open(obs, cap):
    r = req(obs)
    bad = [] if r['terminal'] == 'ok' else [f'terminal {r["terminal"]}']
    if r['terminal'] == 'ok' and 'new-1' not in json.dumps(r['value']):
        bad.append('no created session id')
    if len(obs['createdSessions']) != 1:
        bad.append(f'{len(obs["createdSessions"])} created sessions')
    return bad


def check_grok_ok(obs, cap):
    bad = check_open(obs, cap)
    for name in ('copy', 'copy.verify', 'loadCheck', 'grokHomeFor'):
        if len(calls(obs, name)) != 1:
            bad.append(f'{name} count {len(calls(obs, name))}')
    if calls(obs, 'rm'):
        bad.append('rm called on success')
    return bad


def check_grok_verify_fail(obs, cap):
    r = req(obs)
    bad = []
    if r['terminal'] != 'wire' or 'grokLoadFailed' not in json.dumps(r['value']):
        bad.append(f'want grokLoadFailed wire, got {r["terminal"]} {json.dumps(r["value"], ensure_ascii=False)[:120]}')
    if calls(obs, 'rm') or obs['createdSessions']:
        bad.append('rm or createImported after a failed verify')
    return bad


def check_grok_race(obs, cap):
    r = req(obs)
    bad = []
    if r['terminal'] != 'ok' or 'existing' not in json.dumps(r['value']) or 's9' not in json.dumps(r['value']):
        bad.append(f'want existing s9, got {r["terminal"]} {json.dumps(r["value"], ensure_ascii=False)[:120]}')
    if len(calls(obs, 'rm')) != 1 or obs['createdSessions']:
        bad.append('want exactly one rm of the copy and no createImported')
    return bad


def check_error(obs, cap):
    r = req(obs)
    if r['terminal'] != 'wire' or 'codexUnsupported' not in json.dumps(r['value']):
        return [f'want wire codexUnsupported, got {r["terminal"]} {json.dumps(r["value"], ensure_ascii=False)[:160]}']
    return []


# ---- main ------------------------------------------------------------------------------------------------------------------
def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--case', action='append', default=[])
    ap.add_argument('--list', action='store_true')
    args = ap.parse_args(argv)
    print('integration (spec 9.2): 12 cases; A = injected-reader (I01-I08, NOT real cancel forwarding), B = Grok callback lifecycle (I09-I11), C = error (I12)')
    needed = [('codex', 'list'), ('codex', 'read'), ('grok', 'list'), ('grok', 'read'), ('opencode', 'list'), ('opencode', 'read'),
              ('qwen', 'list'), ('qwen', 'read'), ('codex', 'list_error')]
    caps, limits, failed = {}, [], False
    for e, why in LIMITS.items():
        limits.append((e, why))
    for key in needed:
        try:
            caps[key] = capture(*key)
        except Exception as exc:
            limits.append((f'{key[0]} {key[1]}', f'runner capture failed: {str(exc)[:300]}'))
    if len(caps) < len(needed):
        for n, w in limits:
            print(f'LIMIT {n}: {w}')
        return 2
    cases = [c for c in build_cases(caps) if not args.case or c[0] in args.case]
    if args.list:
        for c in cases:
            print(c[0], c[1], c[2])
        return 0
    scen = [c[3] for c in cases]
    want = oracle.run_oracle(scen)
    got = run_harness(scen)
    rows = []
    for (cid, grp, label, sc, cap, chk), w, g in zip(cases, want, got):
        cmp = compare.compare_observation(w, g, compare.default_policy(cid))
        sem = chk(w, cap)
        ok = cmp['class'] != 'unexplained' and not sem
        failed |= not ok
        rows.append((cid, grp, label, cap['fixture'][:34], cmp['class'], 'ok' if not sem else '; '.join(sem), 'PASS' if ok else 'FAIL'))
        if cmp['class'] != 'exact':
            print(f'{cid} diff: {json.dumps(cmp, ensure_ascii=False)[:600]}')
            d = first_diff(w, g)
            if d:
                print(f'   first diff {d[0]}: oracle {json.dumps(d[1], ensure_ascii=False)[:200]} harness {json.dumps(d[2], ensure_ascii=False)[:200]}')
    print('\n| id | part | case | runner fixture | oracle vs harness | semantic checks | result |\n|---|---|---|---|---|---|---|')
    for r in rows:
        print('| ' + ' | '.join(r) + ' |')
    for n, w in limits:
        print(f'LIMIT {n}: {w}')
    unexpl = sum(1 for r in rows if r[4] == 'unexplained')
    print(f'{sum(r[6] == "PASS" for r in rows)}/{len(rows)} cases pass, {unexpl} unexplained, {len(limits)} limit(s) (claude: L3-1 no driver)')
    if failed:
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
