#!/usr/bin/env python3
"""Generate UTF-8 byte writers and byte-length dispatchers from bud_text.pruv's template comments.

Run after changing a template. With --check, fail if the generated region is stale.
"""
import re
import sys
from pathlib import Path

SRC = Path(__file__).resolve().parent.parent / 'bud_text.pruv'
BEGIN = '// BEGIN GENERATED (m6_test/gen_text.py): template writers, do not edit by hand\n'
END = '// END GENERATED\n'


def writer(name, value):
    data = value.encode('utf-8')
    lines = [f'fn put_{name}(inout o: [u8], at: i64) -> ()',
             '  requires 0 <= at && at <= 1000000',
             '  ensures len(o) == len(old(o))',
             '{',
             f'  if at + {len(data)} > len(o) {{ return; }}']
    lines.extend(f'  o[at + {k}] = {byte};' for k, byte in enumerate(data))
    lines.append('}')
    return '\n'.join(lines) + '\n', len(data)


def main():
    text = SRC.read_text(encoding='utf-8')
    head, rest = text.split(BEGIN) if BEGIN in text else (None, None)
    if head is None:
        raise SystemExit('GENERATED markers missing in bud_text.pruv')
    tail = rest.split(END, 1)[1]
    pieces = re.findall(r'^// text (\w+): (.+)$', head, re.MULTILINE)
    out = [BEGIN]
    lens = {}
    for name, s in pieces:
        code, n = writer(name, s)
        lens[name] = n
        out.append(code)

    def dispatch(fname, keys, params, reqs):
        # keys: list of (condition, piece)
        ln = [f'fn len_{fname}({params}) -> i64', *reqs, '  ensures 0 <= result && result <= 1000', '{']
        pt = [f'fn put_{fname}(inout o: [u8], at: i64, {params}) -> ()', *reqs,
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
    out.append(dispatch('cm', [('lang == 0', 'cm_0'), ('lang == 1', 'cm_1')], 'lang: i64',
                        ['  requires 0 <= lang && lang <= 1']))
    # Dispatch before allocation so each fixed-size error can be initialized directly.
    errors = ['pub fn card_error(kind: i64, lang: i64) -> [u8] ! alloc',
              '  requires 0 <= kind && kind <= 6',
              '  requires 0 <= lang && lang <= 1', '{']
    for k in range(7):
        for lang in range(2):
            name = f'ce_{k}_{lang}'
            last = k == 6 and lang == 1
            if not last:
                errors.append(f'  if kind == {k} && lang == {lang} {{')
            indent = '  ' if last else '    '
            errors.extend([f'{indent}let mut o: [u8] = alloc({lens[name]}, 0);',
                           f'{indent}put_{name}(inout o, 0);',
                           f'{indent}return o;'])
            if not last:
                errors.append('  }')
    out.append('\n'.join(errors + ['}']) + '\n')
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
