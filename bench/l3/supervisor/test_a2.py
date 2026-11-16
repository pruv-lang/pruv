"""L3-2 A2: the capped output writer over real files (limit 0, exact fit, many chunks, create failure)."""
import random
import sys

from common import EXPAND, HERE, enc, run_lines

HARNESS = r'''
#[path="@SOL@"] mod sol;
use std::io::BufRead;
''' + EXPAND + r'''
fn main() {
 let stdin = std::io::stdin();
 for line in stdin.lock().lines() {
  let line = line.unwrap();
  let mut it = line.split_whitespace();
  let path = it.next().unwrap().to_string();
  let limit: i64 = it.next().unwrap().parse().unwrap();
  let v = expand(it);
  let out = sol::run(&path, limit, &v);
  let file = match std::fs::read(&path) {
   Ok(b) => b.iter().map(|x| format!("{:02x}", x)).collect::<String>(),
   Err(_) => "-".to_string(),
  };
  println!("{} | {}", show(&out), file);
 }
}
'''

FILES = HERE / 'work' / 'a2' / 'files'


def oracle(limit, chunks, create_ok):
    written = truncated = total = 0
    error = 0 if create_ok else 1
    data = b''
    for c in chunks:
        total += len(c)
        if error or truncated:
            continue
        room = limit - written
        n = min(len(c), room)
        if len(c) > room:
            truncated = 1
        written += n
        data += c[:n]
    head = f'{written} {truncated} {error} {limit} {total} {int(create_ok)}'
    return head + ' | ' + (data.hex() if create_ok else '-')


def script(chunks):
    out = []
    for c in chunks:
        out += [len(c)] + list(c)
    return out


def main():
    FILES.mkdir(parents=True, exist_ok=True)
    rng = random.Random(2)
    cases = []

    def add(label, limit, chunks, path=None, create_ok=True):
        p = path or str(FILES / f'f{len(cases)}.bin')
        cases.append((label, f'{p} {limit} {enc(script(chunks))}', oracle(limit, chunks, create_ok)))

    add('limit0', 0, [b'abc', b'de'])
    add('limit0 empty', 0, [])
    add('no chunks', 10, [])
    add('empty chunks', 5, [b'', b'', b'ab', b''])
    add('exact fit one', 5, [b'hello'])
    add('exact fit two', 5, [b'he', b'llo'])
    add('exact fit then more', 5, [b'hello', b'x', b'yz'])
    add('over by one', 4, [b'hello'])
    add('split chunk', 7, [b'abcd', b'efgh', b'ij'])
    add('big chunk', 65536, [bytes(rng.randrange(256) for _ in range(65536))])
    add('big over', 100000, [bytes([i % 251 for i in range(65536)]), bytes([7] * 65536)])
    add('huge limit', 1 << 40, [b'abc'])
    for i in range(60):
        chunks = [bytes(rng.randrange(256) for _ in range(rng.choice([0, 1, 2, 5, 17, 300])))
                  for _ in range(rng.randrange(0, 8))]
        add(f'random {i}', rng.choice([0, 1, 3, 10, 50, 400, 2000]), chunks)
    add('create fail dir', 10, [b'abc', b'def'], str(FILES / 'missing' / 'x.bin'), False)
    add('create fail is dir', 3, [b'abcdef'], str(FILES), False)
    # bad input: negative limit, limit over 2^40, chunk length past the end, byte out of range
    cases.append(('bad limit', f'{FILES / "b0.bin"} -1 1 65', '-1 | -'))
    cases.append(('big limit', f'{FILES / "b1.bin"} {(1 << 40) + 1} 1 65', '-1 | -'))
    cases.append(('short script', f'{FILES / "b2.bin"} 5 3 65 66', '-1 | '))
    cases.append(('bad byte', f'{FILES / "b3.bin"} 5 1 256', '-1 | '))
    for p in FILES.glob('*.bin'):
        p.unlink()
    return run_lines('a2', HERE / 'a2_root.pruv', HARNESS, cases)


if __name__ == '__main__':
    sys.exit(main())
