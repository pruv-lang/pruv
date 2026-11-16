"""L3-2 A0: argument spans, argument count and packet builder on py/rs x rust/llvm."""
import random
import sys

from common import EXPAND, HERE, enc, run_lines
from checker import Bad, parse_packet

HARNESS = r'''
#[path="@SOL@"] mod sol;
use std::io::BufRead;
''' + EXPAND + r'''
fn main() {
 let stdin = std::io::stdin();
 for line in stdin.lock().lines() {
  let line = line.unwrap();
  let mut it = line.split_whitespace();
  let op = it.next().unwrap().to_string();
  let v = expand(it);
  let out: Vec<i64> = match op.as_str() {
   "spans" => sol::spans(&text(&v)),
   "count" => sol::arg_count(&text(&v)),
   "pack" => { let k = v.len().min(11); sol::pack(&v[..k], &bytes(&v[k..])) }
   "bad" => sol::bad(),
   _ => panic!("op"),
  };
  println!("{}", show(&out));
 }
}
'''


def oracle_spans(b):
    if not b:
        return [0, 0]
    if b[-1] != 0 or len(b) > 1048576:
        return [1, 0]
    out, start = [], 0
    for i, c in enumerate(b):
        if c == 0:
            out += [start, i]
            start = i + 1
    return [0, len(out) // 2] + out


def oracle_count(b):
    s = oracle_spans(b)
    return [s[1]] if s[0] == 0 else [-1]


def blob(args):
    return b''.join(a.encode() + b'\0' for a in args)


def main():
    rng = random.Random(4302)
    alphabet = ['a', 'b', ' ', '-', '"', 'é', '한', '😀', '\n', '$']
    samples = [b'', b'\0', b'\0\0', b'abc', b'abc\0d', blob(['echo', 'hi']), blob(['', 'x', '']),
               blob(['-c', 'printf "%s" 한글 😀']), b'a\0b\0c\0', blob(['x' * 5000])]
    for _ in range(200):
        args = [''.join(rng.choice(alphabet) for _ in range(rng.randrange(0, 8))) for _ in range(rng.randrange(0, 6))]
        b = blob(args)
        if rng.random() < 0.2 and b:
            b = b[:-1]
        samples.append(b)
    cases = []
    for b in samples:
        try:
            b.decode()
        except UnicodeDecodeError:
            continue  # the host passes valid UTF-8 only
        cases.append(('spans', f'spans {enc(list(b))}', ' '.join(map(str, oracle_spans(b)))))
        cases.append(('count', f'count {enc(list(b))}', ' '.join(map(str, oracle_count(b)))))
    for _ in range(100):
        h = [rng.randrange(-5, 1 << 40) for _ in range(11)]
        tail = [rng.randrange(256) for _ in range(rng.choice([0, 1, 7, 300]))]
        cases.append(('pack', f'pack {enc(h + tail)}', ' '.join(map(str, h + [len(tail)] + tail))))
    cases.append(('pack bad len', 'pack ' + ' '.join(['1'] * 10), '-1'))
    bad = [1, 5] + [0] * 9 + [0]
    cases.append(('bad', 'bad', ' '.join(map(str, bad))))
    # The checker accepts the bad-input packet and rejects a broken one.
    parse_packet(bad)
    try:
        parse_packet([1, 1] + [0] * 10)
        raise SystemExit('checker accepted status 1 with kind 1')
    except Bad:
        pass
    print(f'{len(cases)} fixtures', flush=True)
    return run_lines('a0', HERE / 'a0_root.pruv', HARNESS, cases)


if __name__ == '__main__':
    sys.exit(main())
