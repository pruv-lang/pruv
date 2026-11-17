"""Mutant runner for the L3-8 entry: each mutant M01..M16 is a small edit (one or more sites) of the Pruv sources, built on rs/rust only.

  python3 q_mutants.py [--only M08,M09] [--jobs N]

For a mutant: copy root.pruv + pruv.json and a copy of bench/l3/lib to work/mut/Mxx/, apply the (file, old, new) edits to the lib copy (ent_*.pruv) (every old text must occur exactly
once; the site also carries a `// MUTANT Mxx:` marker), verify + compile with the rs compiler, link harness.rs, run the
killing fixtures (gen_fixtures MUTANT_KILLERS, selected through the `mutant` key of the loaded cases).
Classes: killed (a killer differs from its expect, not by crash / InternalInvariant), verification-rejected (pruvc verify
fails), survived, site-missing (TODO entry or the old text is not found), build-error, crash (runner failed or only
InternalInvariant answers). Exit 0 only if all selected mutants are killed or verification-rejected.
Output: work/mut/report.json and a table."""
import sys
sys.dont_write_bytecode = True
import argparse
import json
import shutil
import traceback
from concurrent.futures import ThreadPoolExecutor

import test_impl
from common import HERE, WORK, load_cases

MUT = WORK / 'mut'

# mutant id -> list of (file, old text, new text); None = TODO entry (reported as site-missing)
MUTANTS = {
    'M01': [('ent_u16.pruv', "  if u == 32 || u == 160 { return true; }\n", "  if u == 32 { return true; }\n"),
            ('ent_u16.pruv', "  if u == 5760 || u == 65279 { return true; }\n", ""),
            ('ent_u16.pruv', "  if u >= 8192 && u <= 8202 { return true; }\n", ""),
            ('ent_u16.pruv', "  if u == 8232 || u == 8233 || u == 8239 || u == 8287 || u == 12288 { return true; }\n", "")],
    'M02': [('ent_sources.pruv', '  let cx: crate::ent_state::Ctx = crate::ent_canon::host_home(cx0);\n  if crate::ent_canon::early(cx) { return cx; }\n  return adv(home_row(cx, "codex:home"', '  let cx: crate::ent_state::Ctx = env_or_home(cx0, 6, ".codex");\n  if crate::ent_canon::early(cx) { return cx; }\n  return adv(home_row(cx, "codex:home"')],
    'M03': [('ent_sources.pruv', "  if len(xdg) > 0 && crate::ent_u16::rd(xdg, 0) == 47 {", "  if len(xdg) > 0 {")],
    'M04': [('ent_state.pruv', "  if crate::ent_u16::rd(ck, 6) != 0 { return false; }\n", ""),
            ('ent_sources.pruv', '  return adv(emit_row(cy, crate::ent_u16::cat(crate::ent_u16::lit("codex:"), id),',
             '  let mut hh: i64 = 1;\n'
             '  let mut hi: i64 = 0;\n'
             '  while hi < len(id) && hi < 64\n'
             '    invariant 0 <= hi && hi <= 64 && 1 <= hh && hh <= 16777213\n'
             '    decreases 64 - hi\n'
             '  {\n'
             '    hh = (hh * 31 + crate::ent_u16::clamp(crate::ent_u16::rd(id, hi), 65536)) % 16777212 + 1;\n'
             '    hi = hi + 1;\n'
             '  }\n'
             '  if hh == crate::ent_state::get_ck(cy, 6) { return crate::ent_state::set_ck(skip(cy, 5, idx + 1, cnt), 6, hh); }\n'
             '  return crate::ent_state::set_ck(adv(emit_row(cy, crate::ent_u16::cat(crate::ent_u16::lit("codex:"), id),'),
            ('ent_sources.pruv', 'nu, expected), 5, idx + 1, cnt);', 'nu, expected), 5, idx + 1, cnt), 6, hh);')],
    'M05': [('ent_sources.pruv', '!crate::ent_accounts::valid_account_id(cx.ent, rp + 2) { return skip(cx, 2, idx + 1, cnt); }', '!crate::ent_accounts::valid_account_id(cx.ent, rp + 2) { return crate::ent_state::fail(cx); }')],
    'M06': [('ent_canon.pruv', '    if !crate::ent_value::str_eq_lit(cx.ent, rp + 16, "ENOENT") {', '    if true {')],
    'M07': [('ent_canon.pruv', "  if crate::ent_u16::eq(target, dot) { return flag(cx, 1); }\n", "")],
    'M08': [('ent_dispatch.pruv', "      if found < 0 { found = i; }", "      found = i;")],
    'M09': [('ent_dispatch.pruv', "  return 4;\n", "  return 1;\n")],
    'M10': [('ent_dispatch.pruv', "  let cut: i64 = 200;\n", ""),
            ('ent_dispatch.pruv', "      crate::ent_u16::wr(inout tab, tb, 1);\n",
             "      crate::ent_u16::wr(inout tab, tb, 1);\n"
             "      let u199: i64 = crate::ent_u16::rd(cx.ent, crate::ent_value::str_start(cx.ent, rb + 3) + 199);\n"
             "      let mut cut: i64 = 200;\n"
             "      if u199 >= 55296 && u199 <= 56319 { cut = 199; }\n")],
    'M11': [('ent_dispatch.pruv', '"AbortError");\n  let e: [i64]', '"AbortError") && is_err;\n  let e: [i64]')],
    'M12': [('ent_dispatch.pruv', "  let has_detail: bool = is_err;", "  let has_detail: bool = true;")],
    'M13': [('ent_bound.pruv', "    if crate::ent_value::nullish(w, p + 6) {", "    if !crate::ent_value::truthy(w, p + 6) {")],
    'M14': [('ent_bound.pruv', "  let mut i: i64 = 0;\n  // MUTANT M14", "  let mut i: i64 = 0;\n  let mut last: i64 = n;\n  // MUTANT M14"),
            ('ent_bound.pruv', "    invariant 0 <= i && i <= n && n <= 16777216 && 0 <= s && s <= 67108864\n    decreases n - i\n  {\n    if crate::ent_u16::rd(w, s + i) == 58 { return i; }",
             "    invariant 0 <= i && i <= n && n <= 16777216 && 0 <= s && s <= 67108864 && 0 <= last && last <= n\n    decreases n - i\n  {\n    if crate::ent_u16::rd(w, s + i) == 58 { last = i; }"),
            ('ent_bound.pruv', "  return n;\n}\n\n// om = packet::new_meta(w)", "  return last;\n}\n\n// om = packet::new_meta(w)")],
    'M15': [('ent_bound.pruv', "  return crate::ent_value::tag_at(w, crate::ent_u16::clamp(p, 4294967296) + 3) == 3;",
             "  return crate::ent_value::truthy(w, crate::ent_u16::clamp(p, 4294967296) + 3);")],
    'M16': [('ent_bound.pruv', "      // MUTANT M16: a later write overwrites the stored session (last write wins)\n",
             "      // MUTANT M16: a later write overwrites the stored session (last write wins)\n      crate::ent_u16::wr(inout val, hit, crate::ent_u16::rd(sp, f));\n")],
}


def apply(mid, d):
    """-> None, or a 'site-missing: ...' reason."""
    edits = MUTANTS[mid]
    if edits is None:
        return 'site-missing: TODO entry (no edit table yet)'
    texts = {}
    for fname, old, new in edits:
        text = texts.get(fname) or (d / 'lib' / fname).read_text(encoding='utf-8')
        n = text.count(old)
        if n != 1:
            return f'site-missing: {fname}: old text found {n} times: {old[:60]!r}'
        texts[fname] = text.replace(old, new)
    for fname, text in texts.items():
        (d / 'lib' / fname).write_text(text, encoding='utf-8')
    return None


def bad_answer(actual):
    return 'InternalInvariant' in json.dumps(actual)


def run_one(mid, g, host, cases):
    d = MUT / mid
    if d.exists():
        shutil.rmtree(d)
    d.mkdir(parents=True)
    for f in HERE.glob('*.pruv'):
        shutil.copy(f, d / f.name)
    shutil.copy(HERE / 'pruv.json', d / 'pruv.json')
    shutil.copytree(HERE.parent / 'lib', d / 'lib')  # temporary lib copy: the mutant edits land here
    rec = {'id': mid, 'status': None, 'killers': [], 'detail': ''}
    why = apply(mid, d)
    if why:
        rec.update(status='site-missing', detail=why)
        return rec
    mine = [c for c in cases if c.get('mutant') == mid]
    rec['killers'] = [c['id'] for c in mine]
    if not mine:
        rec.update(status='site-missing', detail='no killing fixture selected')
        return rec
    out = d / 'out'
    out.mkdir()
    try:
        built = g.compile_pruv('rs', str(d / 'root.pruv'), str(out), 'rust', ['--lib', str(d / 'lib')])
        if built['rc']:
            rec.update(status='verification-rejected', detail=str(built.get('errors') or built.get('stderr') or '')[:300])
            return rec
        main = out / 'main.rs'
        main.write_text((HERE / 'harness.rs').read_text(encoding='utf-8').replace('@SOL@', built['rs']), encoding='utf-8')
        deps, lib = host
        exe = d / 'harness'
        test_impl.sh([g.RUSTC, '-O', '--edition', '2021', main, '-o', exe, '--extern', f'entry_host={lib}', '-L', f'dependency={deps}'])
    except Exception as exc:
        rec.update(status='build-error', detail=str(exc)[:400])
        return rec
    try:
        got = test_impl.run_runner(exe, mine)
    except Exception as exc:
        rec.update(status='crash', detail=str(exc)[:400])
        return rec
    diffs, invariant = [], []
    for case, actual in zip(mine, got):
        if bad_answer(actual):
            invariant.append(case['id'])
            continue
        cls, field = test_impl.judge(case, actual, None)
        if cls != 'exact':
            diffs.append(f'{case["id"]}:{field}')
    if diffs:
        rec.update(status='killed', detail='differs: ' + ', '.join(diffs[:6]))
    elif invariant:
        rec.update(status='crash', detail='only InternalInvariant answers: ' + ', '.join(invariant))
    else:
        rec.update(status='survived', detail='all killers matched expect')
    return rec


def main(argv=None):
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
    cases = [c for c in load_cases() if c.get('mutant') in ids]
    g = test_impl.configure()
    host = test_impl.build_host()
    MUT.mkdir(parents=True, exist_ok=True)

    def safe(mid):
        try:
            return run_one(mid, g, host, cases)
        except Exception:
            return {'id': mid, 'status': 'crash', 'killers': [], 'detail': traceback.format_exc()[-400:]}

    with ThreadPoolExecutor(max_workers=max(1, args.jobs)) as ex:
        results = list(ex.map(safe, ids))
    (MUT / 'report.json').write_text(json.dumps({'mutants': results}, indent=1), encoding='utf-8')
    print(f'{"mutant":<7}{"status":<22}killers / detail')
    for r in results:
        print(f'{r["id"]:<7}{r["status"]:<22}{",".join(r["killers"][:3])}{"..." if len(r["killers"]) > 3 else ""}  {r["detail"][:110]}')
    ok = all(r['status'] in ('killed', 'verification-rejected') for r in results)
    print(f'{sum(r["status"] == "killed" for r in results)} killed, '
          f'{sum(r["status"] == "verification-rejected" for r in results)} verification-rejected, '
          f'{sum(r["status"] not in ("killed", "verification-rejected") for r in results)} other of {len(results)}')
    return 0 if ok else 1


if __name__ == '__main__':
    sys.exit(main())
