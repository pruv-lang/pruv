"""Leg D data: the req list no-match workload (bench/l3/req perf_req.py, placement none) as flat i64 arrays.

  python3 gen.py S M out_prefix   -> out_prefix.int.txt, out_prefix.str.txt (lines: sess, items[, pool])

Session i has K = 8 candidates in sessionThreads order: externalThread o{i} and forks f{i}_0..2 (engine claude), terminal
cliSession c{i} (engine codex: the session has no engine), engineThreads e{i}_1..3 (engine claude). Items are t{j}, engine
claude, so nothing matches and every item walks every session. Engine ids: claude 1, codex 2. No session is temporary.
"""
import sys

K = 8
ENG = {'claude': 1, 'codex': 2}


def cands(i):
    return ([('claude', f'o{i}')] + [('claude', f'f{i}_{k}') for k in range(3)] + [('codex', f'c{i}')]
            + [('claude', f'e{i}_{k}') for k in (1, 2, 3)])


def main():
    S, M, prefix = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
    ids, sess_int, sess_str, pool = {}, [], [], []

    def intern(t):
        return ids.setdefault(t, len(ids) + 1)

    def put(t):
        off = len(pool)
        pool.extend(ord(ch) for ch in t)  # ASCII: one UTF-16 unit per character
        return off, len(t)

    for i in range(S):
        sess_int.append(0)
        sess_str.append(0)
        for eng, t in cands(i):
            sess_int += [ENG[eng], intern(t)]
            off, ln = put(t)
            sess_str += [ENG[eng], off, ln]
    items_int, items_str = [], []
    for j in range(M):
        t = f't{j}'
        items_int += [ENG['claude'], intern(t)]
        off, ln = put(t)
        items_str += [ENG['claude'], off, ln]
    line = lambda a: ' '.join(map(str, a)) + '\n'
    with open(prefix + '.int.txt', 'w') as f:
        f.write(line(sess_int) + line(items_int))
    with open(prefix + '.str.txt', 'w') as f:
        f.write(line(sess_str) + line(items_str) + line(pool))


if __name__ == '__main__':
    main()
