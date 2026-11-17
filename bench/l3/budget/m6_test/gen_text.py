#!/usr/bin/env python3
"""Regenerate the hot-path template writers of bud_text.pruv.

refusal/continue_text run on every budget refusal, so they must not decode UTF-8 literals per call. This script
takes the template texts from the `dec("...")` literals that bud_text.pruv already holds (the single source of the
texts) and emits, between the GENERATED markers, writer functions that store the UTF-16 units directly, three
units packed per i64 constant. Run after changing any template text:

  python3 bench/l3/budget/m6_test/gen_text.py

With --check it only fails (exit 1) when the generated region does not match the literals; run_m6.py calls that.
"""
import re
import sys
from pathlib import Path

SRC = Path(__file__).resolve().parent.parent / 'bud_text.pruv'
BEGIN = '// BEGIN GENERATED (m6_test/gen_text.py): template writers, do not edit by hand\n'
END = '// END GENERATED\n'


def lits(body):
    out = re.findall(r'dec\("((?:[^"\\]|\\.)*)"\)', body)
    for s in out:
        if '\\' in s:
            raise SystemExit(f'escape sequences are not supported in template literals: {s!r}')
    return out


def fn_body(text, name):
    m = re.search(r'\nfn ' + name + r'\(|\npub fn ' + name + r'\(', text)
    if not m:
        raise SystemExit(f'function {name} not found')
    i = text.index('{\n', m.end())
    j = text.index('\n}\n', i)
    return text[i:j]


def units(s):
    out = []
    for ch in s:
        c = ord(ch)
        if c > 0xFFFF:
            raise SystemExit('template text must be BMP only')
        out.append(c)
    return out


def writer(name, s):
    u = units(s)
    lines = [f'// {s!r} ({len(u)} units)',
             f'fn put_{name}(inout o: [i64], at: i64) -> ()',
             '  requires 0 <= at && at <= 1000000',
             '  ensures len(o) == len(old(o))',
             '{']
    k = 0
    while k + 3 <= len(u):
        w = u[k] + u[k + 1] * 65536 + u[k + 2] * 4294967296
        lines.append(f'  put3(inout o, at + {k}, {w});')
        k += 3
    while k < len(u):
        lines.append(f'  crate::ent_u16::wr(inout o, at + {k}, {u[k]});')
        k += 1
    lines.append('}')
    return '\n'.join(lines) + '\n', len(u)


def main():
    text = SRC.read_text(encoding='utf-8')
    head, rest = text.split(BEGIN) if BEGIN in text else (None, None)
    if head is None:
        raise SystemExit('GENERATED markers missing in bud_text.pruv')
    tail = rest.split(END, 1)[1]
    src = head + tail
    # texts: (piece name, literal) taken from the cold decoding functions
    ref_en_pre, ref_en_post, ref_ko_pre, ref_ko_post = lits(fn_body(src, 'refusal_cold'))
    con_en_pre, con_en_post, con_ko_pre, con_ko_post = lits(fn_body(src, 'continue_cold'))
    nl_en, nl_ko = lits(fn_body(src, 'no_limit'))
    rn = lits(fn_body(src, 'reason_name'))
    cn_ko = lits(fn_body(src, 'counter_name'))
    pieces = [('pre_0_0', ref_ko_pre), ('pre_0_1', ref_en_pre), ('post_0_0', ref_ko_post), ('post_0_1', ref_en_post),
              ('pre_1_0', con_ko_pre), ('pre_1_1', con_en_pre), ('post_1_0', con_ko_post), ('post_1_1', con_en_post),
              ('nl_0', nl_ko), ('nl_1', nl_en)]
    pieces += [(f'cn_{r}_0', cn_ko[r]) for r in range(5)] + [(f'cn_{r}_1', rn[r]) for r in range(5)]
    out = [BEGIN]
    lens = {}
    for name, s in pieces:
        code, n = writer(name, s)
        lens[name] = n
        out.append(code)

    def dispatch(fname, keys, params, reqs):
        # keys: list of (condition, piece)
        ln = [f'fn len_{fname}({params}) -> i64', *reqs, '  ensures 0 <= result && result <= 1000', '{']
        pt = [f'fn put_{fname}(inout o: [i64], at: i64, {params}) -> ()', *reqs,
              '  requires 0 <= at && at <= 1000000', '  ensures len(o) == len(old(o))', '{']
        for cond, p in keys[:-1]:
            ln.append(f'  if {cond} {{ return {lens[p]}; }}')
            pt.append(f'  if {cond} {{ put_{p}(inout o, at); return; }}')
        ln.append(f'  return {lens[keys[-1][1]]};')
        pt.append(f'  put_{keys[-1][1]}(inout o, at);')
        return '\n'.join(ln + ['}']) + '\n' + '\n'.join(pt + ['}']) + '\n'

    tl = ['  requires 0 <= t && t <= 1', '  requires 0 <= lang && lang <= 1']
    for part in ('pre', 'post'):
        keys = [(f't == {t} && lang == {l}', f'{part}_{t}_{l}') for t in (0, 1) for l in (0, 1)]
        out.append(dispatch(part, keys, 't: i64, lang: i64', tl))
    out.append(dispatch('nl', [('lang == 0', 'nl_0'), ('lang == 1', 'nl_1')], 'lang: i64',
                        ['  requires 0 <= lang && lang <= 1']))
    keys = [(f'reason == {r} && lang == {l}', f'cn_{r}_{l}') for r in range(5) for l in (0, 1)]
    out.append(dispatch('cn', keys, 'reason: i64, lang: i64',
                        ['  requires 0 <= reason && reason <= 4', '  requires 0 <= lang && lang <= 1']))
    out.append(END)
    new = head + ''.join(out) + tail
    if '--check' in sys.argv[1:]:
        if new != text:
            raise SystemExit(f'{SRC}: generated template writers are stale; run m6_test/gen_text.py')
        return
    SRC.write_text(new, encoding='utf-8')
    print(f'wrote {len(pieces)} template writers into {SRC}')


if __name__ == '__main__':
    main()
