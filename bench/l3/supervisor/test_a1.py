"""L3-2 A1: the tail decoder and cut against Python's incremental 'replace' decoder over random chunkings."""
import random
import sys

from common import EXPAND, HERE, manifest_flags, enc, run_lines

HARNESS = r'''
#[path="@SOL@"] mod sol;
use std::io::BufRead;
''' + EXPAND + r'''
fn main() {
 let stdin = std::io::stdin();
 for line in stdin.lock().lines() {
  let line = line.unwrap();
  let mut it = line.split_whitespace();
  let t: i64 = it.next().unwrap().parse().unwrap();
  let v = expand(it);
  println!("{}", show(&sol::run(t, &v)));
 }
}
'''


def whatwg(chunks):
    """Reference WHATWG UTF-8 decoder, one per stream, emitting in read order (the Python incremental decoder
    holds back some invalid bytes at a chunk end, so its cross-stream order differs; its per-stream text is
    checked against this model in main)."""
    fresh = [0, 0, 0, 0x80, 0xBF]
    st = {1: list(fresh), 2: list(fresh)}
    out = []

    def lead(d, b):
        if b <= 0x7F:
            out.append(chr(b))
        elif 0xC2 <= b <= 0xDF:
            d[0], d[2] = 1, b & 0x1F
        elif 0xE0 <= b <= 0xEF:
            d[3] = 0xA0 if b == 0xE0 else 0x80
            d[4] = 0x9F if b == 0xED else 0xBF
            d[0], d[2] = 2, b & 0xF
        elif 0xF0 <= b <= 0xF4:
            d[3] = 0x90 if b == 0xF0 else 0x80
            d[4] = 0x8F if b == 0xF4 else 0xBF
            d[0], d[2] = 3, b & 7
        else:
            out.append('\ufffd')

    for s, c in chunks:
        d = st[s]
        for b in c:
            if d[0] == 0:
                lead(d, b)
            elif not d[3] <= b <= d[4]:
                d[:] = fresh
                out.append('\ufffd')
                lead(d, b)
            else:
                d[1] += 1
                d[2] = d[2] * 64 + (b & 0x3F)
                d[3], d[4] = 0x80, 0xBF
                if d[1] == d[0]:
                    out.append(chr(d[2]))
                    d[:] = fresh
    for s in (1, 2):
        if st[s][0]:
            out.append('\ufffd')
    return ''.join(out)


def check_model(chunks):
    for s in (1, 2):
        mine = whatwg([(x, c) for x, c in chunks if x == s])
        whole = b''.join(c for x, c in chunks if x == s).decode('utf-8', 'replace')
        if mine != whole:
            raise SystemExit(f'model differs from Python on stream {s}: {chunks!r}')


def oracle(t, chunks):
    check_model(chunks)
    units = whatwg(chunks).encode('utf-16-le', 'surrogatepass')
    units = units[-2 * t:]
    if len(units) >= 2 and 0xDC00 <= int.from_bytes(units[:2], 'little') <= 0xDFFF:
        units = units[2:]  # policy P5
    return list(units.decode('utf-16-le').encode())


PIECES = [b'a', b'z', b'\n', 'é'.encode(), '한'.encode(), '\U0001f600'.encode(), '\U0010ffff'.encode(),
          b'\x80', b'\xbf', b'\xc0', b'\xc1', b'\xc2', b'\xdf', b'\xe0', b'\xe0\x9f', b'\xe0\xa0', b'\xed', b'\xed\xa0',
          b'\xed\x9f\xbf', b'\xef\xbf\xbd', b'\xf0', b'\xf0\x8f', b'\xf0\x90', b'\xf0\x90\x80', b'\xf4\x8f\xbf\xbf',
          b'\xf4\x90', b'\xf5', b'\xff', b'\xfe']


def script(chunks):
    out = []
    for s, c in chunks:
        out += [s, len(c)] + list(c)
    return out


def main():
    rng = random.Random(4311)
    cases = []

    def add(label, t, chunks):
        cases.append((label, f'{t} {enc(script(chunks))}', ' '.join(map(str, oracle(t, chunks)))))

    add('empty', 5, [])
    add('lone lead at end', 5, [(1, b'ab\xe0')])
    add('two pending', 9, [(1, b'\xf0\x9f'), (2, b'\xe4'), (1, b'\x98\x80')])
    add('pair cut', 1, [(1, 'x\U0001f600'.encode())])
    add('pair cut 3', 3, [(1, 'ab\U0001f600\U0001f600'.encode())])
    for i in range(400):
        t = rng.choice([1, 2, 3, 4, 7, 16, 100, 65536])
        stream = {1: b'', 2: b''}
        for s in (1, 2):
            stream[s] = b''.join(rng.choice(PIECES) for _ in range(rng.randrange(0, 60)))
        chunks, pos = [], {1: 0, 2: 0}
        while pos[1] < len(stream[1]) or pos[2] < len(stream[2]):
            s = rng.choice([1, 2])
            n = rng.choice([0, 1, 1, 2, 3, 5, 40])
            chunks.append((s, stream[s][pos[s]:pos[s] + n]))
            pos[s] += n
        add(f'random {i} T={t}', t, chunks)
    big = ''.join(rng.choice(['a', 'é', '한', '\U0001f600']) for _ in range(90000)).encode()
    for t in (1, 1000, 65536):
        chunks = [(1 + k % 2, big[k * 4096:(k + 1) * 4096]) for k in range(len(big) // 4096 + 1)]
        add(f'big T={t}', t, chunks)
    cases.append(('bad T', '0 1 0', '-1'))
    cases.append(('bad script', '5 1 3 97', '-1'))
    print(f'{len(cases)} fixtures', flush=True)
    return run_lines('a1', HERE / 'a1_root.pruv', HARNESS, cases, flags=manifest_flags(HERE / 'a1_root.pruv'))


if __name__ == '__main__':
    sys.exit(main())
