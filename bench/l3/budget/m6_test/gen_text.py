#!/usr/bin/env python3
"""Generate UTF-8 writers, length dispatchers and return bounds from template comments.

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

    def dispatch(fname, keys, params, reqs, bounds=()):
        # keys: list of (condition, piece)
        maximum = max(lens[p] for _, p in keys)
        ln = [f'fn len_{fname}({params}) -> i64', *reqs,
              f'  ensures 0 <= result && result <= {maximum}',
              *(f'  ensures !({cond}) || result <= {n}' for cond, n in bounds), '{']
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
        out.append(dispatch(part, keys, 't: i64, lang: i64', tl,
                            [(cond, lens[piece]) for cond, piece in keys]))
    out.append(dispatch('nl', [('lang == 0', 'nl_0'), ('lang == 1', 'nl_1')], 'lang: i64',
                        ['  requires 0 <= lang && lang <= 1']))
    keys = [(f'reason == {r} && lang == {l}', f'cn_{r}_{l}') for r in range(5) for l in (0, 1)]
    counters = [max(lens[f'cn_{r}_{l}'] for r in range(5)) for l in (0, 1)]
    out.append(dispatch('cn', keys, 'reason: i64, lang: i64',
                        ['  requires 0 <= reason && reason <= 4', '  requires 0 <= lang && lang <= 1'],
                        [(f'lang == {l}', n) for l, n in enumerate(counters)]))
    out.append(dispatch('cm', [('lang == 0', 'cm_0'), ('lang == 1', 'cm_1')], 'lang: i64',
                        ['  requires 0 <= lang && lang <= 1']))
    # Dispatch before allocation so each fixed-size error can be initialized directly.
    errors = ['pub fn card_error(kind: i64, lang: i64) -> [u8] ! alloc',
              '  requires 0 <= kind && kind <= 6',
              '  requires 0 <= lang && lang <= 1',
              f'  ensures len(result) <= {max(n for name, n in lens.items() if name.startswith("ce_"))}',
              '{']
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
    # UTF-16 unit count of each card error, for V-word emitters that need it before writing (m11 root): a constant
    # per (kind, lang) instead of a count loop over the bytes.
    units = {name: len(value.encode('utf-16-le')) // 2 for name, value in pieces if name.startswith('ce_')}
    counts = ['pub fn card_error_units(kind: i64, lang: i64) -> i64',
              '  requires 0 <= kind && kind <= 6',
              '  requires 0 <= lang && lang <= 1',
              f'  ensures 0 <= result && result <= {max(units.values())}',
              '{']
    for k in range(7):
        for lang in range(2):
            if k == 6 and lang == 1:
                counts.append(f'  return {units[f"ce_{k}_{lang}"]};')
            else:
                counts.append(f'  if kind == {k} && lang == {lang} {{ return {units[f"ce_{k}_{lang}"]}; }}')
    out.append('\n'.join(counts + ['}']) + '\n')
    out.append(END)
    int_bytes = 17
    no_limit = max(lens['nl_0'], lens['nl_1'])
    limit = max(int_bytes, no_limit)
    amount = int_bytes + 1 + limit
    templates = [max(lens[f'pre_{t}_{l}'] + counters[l] + 1 + amount + lens[f'post_{t}_{l}']
                     for l in (0, 1)) for t in (0, 1)]
    key = lens['wire_prefix'] + counters[1]
    bounds = {
        'int_text': int_bytes,
        'reason_name': counters[1],
        'counter_name': max(counters),
        'no_limit': no_limit,
        'limit_text': limit,
        'amount_text': amount,
        'refusal': templates[0],
        'continue_text': templates[1],
        'card_missing': max(lens['cm_0'], lens['cm_1']),
        'refusal_key': key,
        'refusal_wire': key,
        'template': max(templates),
    }
    new = head + ''.join(out) + tail
    # Synchronize generated length clauses on the handwritten builders.
    for name, bound in bounds.items():
        pattern = rf'(^fn {name}\(|^pub fn {name}\()([^\n]*\n(?:  requires [^\n]*\n)*)(?:  ensures .* // Generated text bound\.\n|  ensures len\(result\) <= 17\n)*'
        target = 'result.key' if name == 'refusal_wire' else 'result'
        clause = f'  ensures len({target}) <= {bound}'
        if name == 'template':
            clause += f' && (t != 0 || len(result) <= {templates[0]}) && (t != 1 || len(result) <= {templates[1]})'
        new, count = re.subn(pattern, lambda m: m[1] + m[2] + clause + ' // Generated text bound.\n',
                             new, flags=re.MULTILINE)
        if count != 1:
            raise SystemExit(f'expected one text builder named {name}')
    if '--check' in sys.argv[1:]:
        if new != text:
            raise SystemExit(f'{SRC}: generated template writers are stale; run m6_test/gen_text.py')
        return
    SRC.write_text(new, encoding='utf-8')
    print(f'wrote {len(pieces)} template writers into {SRC}')


if __name__ == '__main__':
    main()
