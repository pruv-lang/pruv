"""Mutant runner for the L3-9 request handlers: each mutant M01..M17 is a small edit of the Pruv sources
(or, for the two host-side rules, of the Rust host leaf), built on rs/rust only.

  python3 q_mutants.py [--only M08,M09] [--jobs N]

For a mutant: copy bench/l3/req/*.pruv and bench/l3/lib to work/mut/Mxx/ (and, for a host mutant, the host crate with a mutated copy of
leaf.rs), apply the (file, old, new) edits (every old text must occur exactly once; a Pruv site also carries a
`// MUTANT Mxx:` marker comment in the product source), verify + compile with the rs compiler, link harness.rs, run the killer
fixture (the fixture whose `killer` key is the mutant id) through the mutant harness and compare with the pinned-TypeScript oracle
using the piece's own compare.compare_batch. Probe mutants instead link forge.rs and check whether the core still refuses forged packets.
Classes: killed (an unexplained fixture difference excluding crash / InternalInvariant, or a forgery no longer refused),
verification-rejected (pruvc verify fails), survived, site-missing (old text not found / no killer fixture or probe),
build-error (compile or link failed), crash (runner failed or
only crash / InternalInvariant answers). Exit 0 only if every selected mutant is killed or verification-rejected.
Output: work/mut/report.json and a table. M10 and M11 only exist in the Rust host (the window timer length and the thrown-name
classification are host duties), so they edit the host leaf copy."""
import sys
sys.dont_write_bytecode = True
import argparse
import json
import re
import shutil
import time
import traceback
from concurrent.futures import ThreadPoolExecutor

import compare
import forge_test
import test_impl
from common import HERE, load_all
from oracle import run_oracle

MUT = HERE / 'work' / 'mut'
LEAF = 'leaf.rs'  # the host leaf copy (edits to this file are host edits)

U = 'crate::ent_u16::'
L = 'crate::req_list::'

# mutant id -> list of (file, old text, new text)
MUTANTS = {
    # sourceUnknown becomes an empty list success
    'M01': [('root.pruv',
             '    return done_req(world, h, isnew, seq, crate::req_error::unknown_pool(), crate::req_error::unknown_rec(), crate::ent_u16::empty(), crate::ent_u16::empty());\n',
             f'    return list_done(world, h, isnew, seq, np, {L}w_new(0));\n')],
    # always the folder fallback text instead of the thrown Error message
    'M02': [('root.pruv', f'  if {U}rd(crec, 7) == 3 {{\n', '  if false {\n')],
    # the last bound match wins: a hit is remembered (slot 21) and the search goes on; the initial search ends with the remembered one
    'M03': [('root.pruv',
             '      return open_done(world, h, false, seq, crate::req_packet::sec(comp, 0), crec, true);\n',
             f'      let p4: [i64] = {U}cat(p, crate::req_packet::sec(comp, 0));\n'
             f'      let mut w4: [i64] = copy(wk);\n'
             f'      crate::req_value::put(inout w4, 21, 6, {U}clamp({U}rd(crec, 2), 1073741823) + len(p), {U}rd(crec, 3));\n'
             f'      {L}hset(inout w4, 3, {U}clamp({L}hget(wk, 3), 1000000000) + 1);\n'
             f'      return next_call(world, h, false, owner, seq, p4, rec, rows, w4);\n'),
            ('root.pruv',
             '    let key: [i64] = crate::req_open::key_of(p, rows, found, rec);\n',
             f'    if {U}rd(wk, 21) == 6 {{\n'
             f'      let mut cr2: [i64] = {U}zeros(4);\n'
             f'      crate::req_value::copy_slot(inout cr2, 1, wk, 21);\n'
             f'      return open_done(world, h, isnew, seq, p, cr2, true);\n'
             f'    }}\n'
             '    let key: [i64] = crate::req_open::key_of(p, rows, found, rec);\n')],
    # the titles map is first-write-wins (the smallest equal slot instead of the last)
    'M04': [('req_list.pruv',
             '        if i < mid && (j >= hi || scmp(pool, w, sb(m, si), sb(m, sj)) <= 0) {\n',
             '        if i < mid && (j >= hi || scmp(pool, w, sb(m, si), sb(m, sj)) < 0) {\n')],
    # `||` becomes `??`: an empty (falsy but not nullish) mapped title is kept
    'M05': [('req_list.pruv', '  if crate::req_value::truthy(w, tp) { return tp; }\n', '  if !crate::req_value::nullish(w, tp) { return tp; }\n')],
    # the same-mode refusal of an app action is dropped
    'M06': [('root.pruv', f'    if {U}rd(crec, 1) == 1 {{ return wire_req(world, h, false, seq, 8); }}\n', '')],
    # the Grok target account check is skipped
    'M07': [('root.pruv', '    if !isdef {\n      let mut c: [i64] = crate::ent_u16::zeros(3);\n', '    if false && !isdef {\n      let mut c: [i64] = crate::ent_u16::zeros(3);\n')],
    # the folder is part of the opening key
    'M08': [('root.pruv',
             '    let key: [i64] = crate::req_open::key_of(p, rows, found, rec);\n',
             f'    let key: [i64] = {U}cat({U}cat(crate::req_open::key_of(p, rows, found, rec), {U}one(124)), crate::req_value::units(p, wk, 15));\n')],
    # the finally (done_ev and the failure path) no longer reports the opening delete event
    'M09': [('req_open.pruv', '  if k < 0 { return eb0(); }\n  return eb(1, ev(2, 1,', '  if k < 0 || true { return eb0(); }\n  return eb(1, ev(2, 1,')],
    # the 20000 ms window timer becomes 19999 (host leaf)
    'M10': [(LEAF,
             '        let due = self.now + 20000;\n        self.active_timers.push((tid, due));\n'
             '        self.events.push(Ev { c: "timer.set".to_string(), a: format!("{{\\"id\\":{},\\"delay\\":20000,\\"due\\":{}}}", tid, due), o: None, ..Default::default() });\n',
             '        let due = self.now + 19999;\n        self.active_timers.push((tid, due));\n'
             '        self.events.push(Ev { c: "timer.set".to_string(), a: format!("{{\\"id\\":{},\\"delay\\":19999,\\"due\\":{}}}", tid, due), o: None, ..Default::default() });\n')],
    # the AbortError name test needs an Error instance: a plain object named AbortError no longer counts (host leaf facts)
    'M11': [(LEAF, '                } else if let Some(n) = &spec.name {\n                    Some(n.clone())\n',
             '                } else if let (Some(n), true) = (&spec.name, spec.class != Class::Plain) {\n                    Some(n.clone())\n')],
    # the cancel reason no longer takes precedence in the catch (three handler steps)
    'M12': [('root.pruv', f'    if {U}rd(crec, 8) == 1 {{ return cancel_done(world, h, seq, crec); }}\n    return wire_req(world, h, false, seq, 5);\n',
             '    return wire_req(world, h, false, seq, 5);\n'),
            ('root.pruv', f'    if {U}rd(crec, 8) == 1 {{ return cancel_done(world, h, seq, crec); }}\n    return wire_req(world, h, false, seq, 6);\n',
             '    return wire_req(world, h, false, seq, 6);\n'),
            ('root.pruv', f'    if {U}rd(crec, 8) == 1 {{ return cancel_done(world, h, seq, crec); }}\n    let wi: i64',
             '    let wi: i64')],
    # no race re-search after the async read: the create follows at once
    'M13': [('root.pruv', f'  {L}hset(inout w2, 2, 0);\n  {L}hset(inout w2, 3, 0);\n  return next_call(world, h, false, owner, seq, p, rec, rows, w2);\n',
             '  return open_cont(world, h, false, owner, seq, p, rec, rows, w2);\n')],
    # the race rm of the copied path is dropped (the open completes without call 36)
    'M14': [('root.pruv',
             '        return emit_call(world, h, false, owner, seq, 33, 3, crate::ent_value::pool_cat(p, cpool), rec, rows, w3, call2(36, 0));\n',
             f'        let mut cr3: [i64] = {U}zeros(4);\n'
             f'        crate::req_value::copy_slot(inout cr3, 1, w3, 21);\n'
             '        return open_done(world, h, false, seq, crate::ent_value::pool_cat(p, cpool), cr3, true);\n')],
    # a trailing high surrogate is dropped from the 80-unit title cut
    'M15': [('req_fill.pruv', '    let cut: i64 = 80;\n',
             '    let mut cut: i64 = 80;\n'
             '    let u79: i64 = crate::ent_u16::rd(t, 79);\n'
             '    if u79 >= 55296 && u79 <= 56319 { cut = 79; }\n')],
    # the thread slot name ignores the account
    'M16': [('req_fill.pruv', '  if len(acc) > 0 { return crate::ent_u16::cat(', '  if false && len(acc) > 0 { return crate::ent_u16::cat(')],
    # an extra pool word passes the shared core length check
    'M17': [('lib/ent_wire.pruv', '  if l0 != 2 + (nb + 6) / 7 { return false; }',
             '  if l0 < 2 + (nb + 6) / 7 { return false; }')],
}

PROBES = {'M17': ['F62', 'F64']}

VERIFY_WORDS = re.compile(r'verif|prove|proof|ensures|requires|invariant|decreases|overflow|out of bounds|assert|obligation|termination', re.I)


def apply(mid, d):
    """-> None, or a 'site-missing: ...' reason."""
    texts = {}
    for fname, old, new in MUTANTS[mid]:
        path = d / fname
        text = texts.get(fname)
        if text is None:
            text = path.read_text(encoding='utf-8')
        n = text.count(old)
        if n != 1:
            return f'site-missing: {fname}: old text found {n} times: {old[:70]!r}'
        texts[fname] = text.replace(old, new)
    for fname, text in texts.items():
        (d / fname).write_text(text, encoding='utf-8')
    return None


def sites(mid):
    """['file:line', ...] of the `// MUTANT Mxx:` markers in the product sources (host mutants have none: leaf.rs is read-only)."""
    out = []
    sources = {f.name: f for f in HERE.glob('*.pruv')}
    sources.update({name: HERE.parent / name for name, _, _ in MUTANTS[mid] if name.startswith('lib/')})
    for name, f in sorted(sources.items()):
        for i, line in enumerate(f.read_text(encoding='utf-8').split('\n'), 1):
            if f'// MUTANT {mid}:' in line:
                out.append(f'{name}:{i}')
    return out


def host_sites(mid):
    out = []
    for fname, old, _ in MUTANTS[mid]:
        if fname == LEAF:
            text = (HERE / LEAF).read_text(encoding='utf-8')
            at = text.find(old)
            if at >= 0:
                out.append(f'{LEAF}:{text.count(chr(10), 0, at) + 1}')
    return out


def build_host_copy(d):
    """Build the production host wrapper with only leaf.rs replaced by the mutant."""
    hc = d / 'hostcrate'
    (hc / 'src').mkdir(parents=True)
    shutil.copy(HERE / 'host' / 'Cargo.toml', hc / 'Cargo.toml')
    shutil.copy(HERE / 'host' / 'Cargo.lock', hc / 'Cargo.lock')
    wrapper = (HERE / 'host' / 'src' / 'lib.rs').read_text(encoding='utf-8')
    includes = {
        '../../../entry/host.rs': HERE.parent / 'entry' / 'host.rs',
        '../../leaf.rs': d / LEAF,
        '../../host.rs': HERE / 'host.rs',
    }
    for relative, path in includes.items():
        old = f'include!("{relative}");'
        if wrapper.count(old) != 1:
            raise RuntimeError(f'expected one {old} in the host wrapper')
        wrapper = wrapper.replace(old, f'include!({json.dumps(str(path.resolve()), ensure_ascii=False)});')
    (hc / 'src' / 'lib.rs').write_text(wrapper, encoding='utf-8')
    target = d / 'hosttarget'
    test_impl.sh([test_impl.tool('cargo'), 'build', '--locked', '--offline', '--release', '--manifest-path', hc / 'Cargo.toml', '--target-dir', target])
    deps = target / 'release/deps'
    libs = list(deps.glob('libreq_host-*.rlib'))
    if not libs:
        raise RuntimeError('no libreq_host rlib after the mutant host build')
    return deps, max(libs, key=lambda p: p.stat().st_mtime)


def run_probes(probes, exe):
    accepted, other = [], []
    for probe in probes:
        if probe == 'F64':
            v = forge_test.begin_pool('O01', forge_test.extra_pool_word, harness=exe, verdict=True)
        else:
            _, case_id, step, mutate, _ = next(row for row in forge_test.F if row[0] == probe)
            v = forge_test.core_refused(case_id, step, mutate, strict=True, harness=exe, verdict=True)
        if v == 'accepted':
            accepted.append(probe)
        elif v != 'refused':
            other.append(probe)
    # only a well-formed core packet that accepts the forgery counts as a kill; malformed core output is 'other'
    if accepted:
        return 'killed', 'core accepted ' + ', '.join(accepted)
    if other:
        return 'other', 'malformed core output on ' + ', '.join(other)
    return 'survived', 'core refused all probes'


def run_one(mid, g, host, cases, oracle):
    d = MUT / mid
    if d.exists():
        shutil.rmtree(d)
    d.mkdir(parents=True)
    for f in HERE.glob('*.pruv'):
        shutil.copy(f, d / f.name)
    shutil.copy(HERE / LEAF, d / LEAF)
    shutil.copytree(HERE.parent / 'lib', d / 'lib')
    rec = {'id': mid, 'status': None, 'killers': [], 'sites': sites(mid) + host_sites(mid), 'detail': ''}
    why = apply(mid, d)
    if why:
        rec.update(status='site-missing', detail=why)
        return rec
    probes = PROBES.get(mid, [])
    idx = [i for i, c in enumerate(cases) if c.get('killer') == mid]
    rec['killers'] = probes or [cases[i]['id'] for i in idx]
    if not idx and not probes:
        rec.update(status='site-missing', detail='no killer fixture carries this mutant id')
        return rec
    mine = [cases[i] for i in idx]
    out = d / 'out'
    out.mkdir()
    try:
        built = g.compile_pruv('rs', str(d / 'root.pruv'), str(out), 'rust', ['--lib', str(d / 'lib')])
        if built['rc']:
            msg = str(built.get('errors') or built.get('stderr') or built)
            rec.update(status='verification-rejected' if VERIFY_WORDS.search(msg) else 'build-error', detail=msg.replace('\n', ' ')[:300])
            return rec
        deps, lib = build_host_copy(d) if any(f == LEAF for f, _, _ in MUTANTS[mid]) else host
        main = out / 'main.rs'
        source = 'forge.rs' if probes else 'harness.rs'
        main.write_text((HERE / source).read_text(encoding='utf-8').replace('@SOL@', built['rs']), encoding='utf-8')
        exe = d / ('forge_harness' if probes else 'harness')
        cmd = [g.RUSTC, '-O', '--edition', '2021', main, '-o', exe, '--extern', f'req_host={lib}']
        if probes:
            serde_json = max(deps.glob('libserde_json-*.rlib'), key=lambda p: p.stat().st_mtime)
            cmd.extend(['--extern', f'serde_json={serde_json}'])
        test_impl.sh(cmd + ['-L', f'dependency={deps}'])
    except Exception as exc:
        rec.update(status='build-error', detail=str(exc).replace('\n', ' ')[:400])
        return rec
    if probes:
        try:
            status, detail = run_probes(probes, exe)
        except Exception as exc:
            status, detail = 'crash', str(exc)[:400]
        rec.update(status=status, detail=detail)
        return rec
    got = test_impl.run_runner(exe, mine)
    if isinstance(got, BaseException):
        rec.update(status='crash', detail=str(got)[:400])
        return rec
    batch = compare.compare_batch([oracle[i] for i in idx], got)
    real, crashy, notes = [], [], []
    for case, res in zip(mine, batch['results']):
        if res['class'] == 'unexplained':
            (crashy if res.get('reason') in ('crash', 'missing', 'malformed', 'internal-invariant', 'id-mismatch', 'extra') else real).append(res)
            notes.append(f'{case["id"]}: {res.get("reason") or "differs"} at {res.get("path")}')
        elif res['class'] != 'exact':
            notes.append(f'{case["id"]}: explained as {res["class"]}')
    if real:
        rec.update(status='killed', detail='; '.join(notes)[:300])
    elif crashy:
        rec.update(status='crash', detail='only crash-type answers: ' + '; '.join(notes)[:300])
    else:
        rec.update(status='survived', detail='all killers matched the oracle ' + '; '.join(notes)[:200])
    return rec


def main(argv=None):
    t0 = time.time()
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--only', help='comma list of mutant ids, e.g. M08,M09')
    ap.add_argument('--jobs', type=int, default=2)
    args = ap.parse_args(argv)
    ids = sorted(MUTANTS)
    if args.only:
        want = [w.strip().upper() for w in args.only.split(',') if w.strip()]
        unknown = [w for w in want if w not in MUTANTS]
        if unknown:
            raise SystemExit('unknown mutant ' + ','.join(unknown))
        ids = sorted(want)
    fixture_ids = set(ids).difference(PROBES)
    cases = [c for c in load_all() if c.get('killer') in fixture_ids] if fixture_ids else []
    oracle = run_oracle(cases) if cases else []
    g = test_impl.configure()
    host = test_impl.build_host()
    MUT.mkdir(parents=True, exist_ok=True)
    # run_one indexes cases/oracle by position: keep only the selected killers (one list for all mutants)

    def safe(mid):
        try:
            return run_one(mid, g, host, cases, oracle)
        except Exception:
            return {'id': mid, 'status': 'crash', 'killers': [], 'sites': [], 'detail': traceback.format_exc()[-400:]}

    with ThreadPoolExecutor(max_workers=max(1, args.jobs)) as ex:
        results = list(ex.map(safe, ids))
    secs = round(time.time() - t0, 1)
    (MUT / 'report.json').write_text(json.dumps({'seconds': secs, 'mutants': results}, indent=1, ensure_ascii=False), encoding='utf-8')
    print(f'{"mutant":<7}{"status":<22}{"killer":<7}{"site":<28}detail')
    for r in results:
        print(f'{r["id"]:<7}{r["status"]:<22}{",".join(r["killers"][:2]):<7}{",".join(r["sites"][:2])[:27]:<28}{r["detail"][:110]}')
    ok = all(r['status'] in ('killed', 'verification-rejected') for r in results)
    print(f'{sum(r["status"] == "killed" for r in results)} killed, '
          f'{sum(r["status"] == "verification-rejected" for r in results)} verification-rejected, '
          f'{sum(r["status"] not in ("killed", "verification-rejected") for r in results)} other of {len(results)}; {secs}s')
    return 0 if ok else 1


if __name__ == '__main__':
    sys.exit(main())
