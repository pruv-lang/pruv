"""16 hand mutants (single source replacements in work only, rs/rust only). Each must be killed by a verifier
rejection or by a fixed fixture: wrong result against the checked-in expected JSON, or a protocol violation recorded
by the fake server. Run test_impl.py first (it builds the host crate and the string adapter in work/).

    python3 bench/l3/codex/q_mutants.py
"""
import sys
sys.dont_write_bytecode = True
import importlib.util
import json
import os
import shutil
from common import HERE, WORK, fake_binary, request_for, read_log, run_lines, check_case, load_cases

MUTANTS = [
    ('wrong-id-accepted', 'rpc.pruv', 'sp.status == 0 && crate::util::id_is(d.source, sp.start, sp.end, id)', 'sp.status == 0'),
    ('initialized-omitted', 'rpc.pruv', 'else { q = crate::transport::send(move q, slot, crate::util::to_bytes("{\\"method\\":\\"initialized\\",\\"params\\":{}}\\n"), ms); }', 'else { }'),
    ('unsupported-mapping-lost', 'root.pruv', '1 => { ctl.fail = 4; }', '1 => { ctl.fail = 6; }'),
    ('not-found-alternative-dropped', 'rpc.pruv', 'if lit_at(u, n, i, "no rollout") { return true; }', 'if lit_at(u, n, i, "no rollout!") { return true; }'),
    ('list-admission-bypassed', 'list.pruv', 'if crate::names::valid_thread(idv) {', 'if len(idv) > 0 {'),
    ('stop-after-first-page', 'root.pruv', 'else if cu.status != 0 || len(cu.units) == 0 || dn <= 0 { fin = true; }', 'else if cu.status != 0 || len(cu.units) == 0 || dn <= 0 || page >= 1 { fin = true; }'),
    ('newest-first-feeding', 'read.pruv', '    j = j - 1;\n    let b: i64 = 10 * j;', '    let b: i64 = 10 * (count - j);\n    j = j - 1;'),
    ('commentary-admitted', 'item.pruv', 'if crate::access::string_is(d, crate::util::field(d, it, "phase"), "commentary") != 1 { x = item_text(d, it); }', 'if true { x = item_text(d, it); }'),
    ('plan-counted-as-tool', 'item.pruv', '} else if crate::access::string_is(d, ty, "plan") == 1 {', '} else if crate::access::string_is(d, ty, "planx") == 1 {'),
    ('generic-tool-not-counted', 'item.pruv', '        tools = tools + 1;', '        tools = tools + 0;'),
    ('budget-omission-removed', 'root.pruv', 'if hs[4] == 1 { more = true; fin = true; }', 'if hs[4] == 1 { fin = true; }'),
    ('reply-bytes-cap-off-by-one', 'item.pruv', 'if rbytes > 32768 { rbytes = 32768; }', 'if rbytes > 32767 { rbytes = 32767; }'),
    ('later-plan-replaces-first', 'read.pruv', 'if hs[3] == 0 && raw.has_plan {', 'if raw.has_plan {'),
    ('lone-cwd-accepted', 'root.pruv', '!crate::folder::lone(crate::access::string_units(d, cw).units) && ', ''),
    ('code-rounding-lost', 'rpc.pruv', 'let hi: str = "32601000000000001818989403545856475830078125";', 'let hi: str = "32601";'),
    ('code-low-prefix-accepted', 'rpc.pruv', 'if sig < 5 || (on_lo && sig < len(lo)) { return false; }', 'if sig < 5 { return false; }'),
]


def main():
    spec = importlib.util.spec_from_file_location('common_sup', HERE.parent / 'supervisor/common.py')
    c = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(c)
    g, buildrun = c.configure(WORK)
    deps = WORK / 'host/debug/deps'
    lib = next(deps.glob('libcodex_host-*.rlib'))
    serde = next(deps.glob('libserde_json-*.rlib'))
    adapter = WORK / 'libpruv_str.rlib'
    manifest = json.loads((HERE / 'pruv.json').read_text())
    flags = ['--rust-extern', f'pruv_str={adapter}']
    binary = fake_binary()
    cases = load_cases()
    report = []
    for name, file, old, new in MUTANTS:
        w = WORK / 'mutants' / name
        shutil.rmtree(w, ignore_errors=True)
        w.mkdir(parents=True, exist_ok=True)
        for p in HERE.glob('*.pruv'):
            shutil.copy2(p, w / p.name)
        # manifest copy next to the root: dependency paths are relative to the manifest, so point them at the real packages
        mutated = json.loads(json.dumps(manifest))
        for dep in mutated.get('dependencies', {}).values():
            dep['path'] = os.path.relpath((HERE / dep['path']).resolve(), w.resolve())
        (w / 'pruv.json').write_text(json.dumps(mutated) + '\n')
        p = w / file
        s = p.read_text()
        assert s.count(old) == 1, (name, s.count(old))
        p.write_text(s.replace(old, new))
        built = g.compile_pruv('rs', str(w / 'root.pruv'), str(w), 'rust', flags + ['--manifest', str(w / 'pruv.json')])
        (w / 'compile.json').write_text(json.dumps(built, default=str, indent=2))
        row = {'name': name, 'file': file, 'old': old, 'new': new}
        if built['rc']:
            detail = str(built.get('errors') or built.get('stderr'))
            # A parser/tool error is not credited as a semantic kill.
            row.update(result='verifier-rejected' if any(x in detail.lower() for x in ('ensures', 'invariant', 'unproved', 'verification', 'timeout')) else 'build-error', detail=detail[-2000:])
        else:
            harness = w / 'main.rs'
            harness.write_text((HERE / 'harness.rs').read_text().replace('@SOL@', built['rs']))
            exe = w / 'runner'
            buildrun([g.RUSTC, '-O', '--edition', '2021', harness, '-o', exe, '--extern', f'codex_host={lib}', '--extern', f'pruv_str={adapter}',
                      '--extern', f'serde_json={serde}', '-L', f'dependency={deps}'])
            side = f'mut_{name}'
            requests = [request_for(case, side, i, binary) for i, case in enumerate(cases)]
            actual = run_lines([exe], requests)
            kills = [case['name'] for i, case in enumerate(cases) if check_case(case, actual[i], read_log(i, side))]
            row.update(result='differential-killed' if kills else 'survived', fixtures=kills[:20])
            if not kills:
                row['reason'] = 'No fixed fixture distinguishes it; not a claim of equivalence.'
        report.append(row)
        (WORK / 'mutants').mkdir(exist_ok=True)
        (WORK / 'mutants/report.json').write_text(json.dumps(report, indent=2))
        print(name, row['result'], flush=True)
    return int(any(x['result'] in ('build-error', 'survived') for x in report))


if __name__ == '__main__':
    raise SystemExit(main())
