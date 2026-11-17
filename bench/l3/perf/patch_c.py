"""Leg C patch: in a COPY of req_world.pruv, make dig() return 1 and world_ok() return true (see build_bc.sh)."""
import re
import sys

path = sys.argv[1]
src = open(path, encoding='utf-8').read()


def replace_body(src, header_re, body):
    m = re.search(header_re, src, re.M)
    if not m:
        sys.exit(f'patch_c: header not found: {header_re}')
    start = src.index('{', m.end())  # the body brace follows the header (and its contracts)
    depth, i = 0, start
    while True:
        if src[i] == '{':
            depth += 1
        elif src[i] == '}':
            depth -= 1
            if depth == 0:
                break
        i += 1
    return src[:start] + '{\n  ' + body + '\n}' + src[i + 1:]


src = replace_body(src, r'^pub fn dig\(a: \[i64\], skip: i64\) -> i64\n  ensures 1 <= result\n', 'return 1;')
src = replace_body(src, r'^pub fn world_ok\(w: \[i64\]\) -> bool\n', 'return true;')
open(path, 'w', encoding='utf-8').write(src)
print('patched', path)
