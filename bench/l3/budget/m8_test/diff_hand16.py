#!/usr/bin/env python3
"""P01 Q2 Unicode/numeric control: differential check of hand_m8_u16 (q mode) against the Pruv driver_m8 (q mode).

Q2 (q_gen) values never reach the safe-integer edge and rarely use lone surrogates, so this generates deterministic
driver programs aimed at those edges and diffs the two executables' output byte for byte:
  * keys: empty, NUL units, ASCII, non-ASCII BMP, astral pairs, lone high/low surrogates, tails of every length 0..7,
    near-miss budget ids (differ in the last unit);
  * numbers: counters and amounts at 0, +-1, +-(2^53-1), +-2^53, limits at the boundary, negative amounts, unlimited;
  * states open / halted (every reason, absent reason) / closed; noteTool dedup, forgetTurn, failures at the edge.

  python3 bench/l3/budget/m8_test/diff_hand16.py [--count 300] [--seed 20261117]
(needs work/m8/driver_m8 from `run_m8.py q` and work/m8/hand_m8_u16 from `run_m8.py q --hand16`)
"""
import argparse
import os
import random
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
WORK = os.path.join(HERE, "..", "work", "m8")
M = 2 ** 53 - 1
NUMS = [0, 1, -1, 2, M, M - 1, -M, -M + 1, M + 1, -M - 1, 2 ** 53, 10 ** 12, 7]


def key_units(rng):
    k = rng.randrange(9)
    n = rng.randrange(0, 18)
    if k == 0:
        return []
    if k == 1:
        return [0] * max(1, n % 4) + [ord("a")] * (n % 3)
    if k == 2:
        return [rng.randrange(32, 127) for _ in range(n)]
    if k == 3:
        return [rng.choice([0xAC00, 0x0441, 0x00E9, 0x4E2D, 0xFFFF, 0x80]) for _ in range(max(1, n))]
    if k == 4:
        return [0xD83D, 0xDE00] * max(1, n // 2)
    if k == 5:
        return [0xD800] + [ord("x")] * n
    if k == 6:
        return [ord("y")] * n + [0xDC00]
    if k == 7:
        return [0xDC00, 0xD800] + [rng.randrange(0, 65536) for _ in range(n)]
    return [rng.randrange(0, 65536) for _ in range(n)]


def kw(u):
    return [len(u)] + u


def record(rng, bid):
    st = rng.choice([0, 0, 0, 1, 2])
    unl = rng.choice([0, 0, 1])
    reason = rng.choice([-1, 0, 1, 2, 3, 4]) if st == 1 else -1
    lim = [rng.choice([10 ** 12, M, 5, 0, M - 1]) for _ in range(4)]
    used = [rng.choice(NUMS[:8] + [3]) for _ in range(4)]
    tools, fails, retr = (rng.choice([0, 3, M, M - 1, -M]) for _ in range(3))
    return [st, unl, reason] + lim + used + [tools, fails, retr] + kw(bid)


def what(rng):
    w = []
    for _ in range(5):
        if rng.random() < 0.6:
            w += [2, rng.choice(NUMS)]
        else:
            w += [rng.choice([0, 1]), 0]
    return w


def program(rng, case):
    out = ["case %d" % case]
    sess = [key_units(rng) for _ in range(rng.randrange(1, 4))]
    bids = [key_units(rng) for _ in sess]
    for s, b in zip(sess, bids):
        r = record(rng, b)
        out.append("put " + " ".join(map(str, kw(s) + [len(r)] + r)))
    turns = [key_units(rng) for _ in range(2)]
    acts = [key_units(rng) for _ in range(3)]
    for i in range(rng.randrange(1, 12)):
        out.append("step %d" % i)
        j = rng.randrange(len(sess))
        b = bids[j]
        if rng.random() < 0.2 and b:
            b = b[:-1] + [(b[-1] + 1) % 65536]  # near-miss budget id
        c = rng.choice([1, 1, 1, 2, 3, 3, 4])
        if c == 1:
            op = [1] + kw(sess[j]) + kw(b) + what(rng)
        elif c == 2:
            op = [2] + kw(sess[j]) + kw(b)
        elif c == 3:
            op = [3] + kw(sess[j]) + kw(b) + kw(rng.choice(turns)) + kw(rng.choice(acts))
        else:
            op = [4] + kw(rng.choice(turns))
        out.append("call " + " ".join(map(str, op)))
    out.append("end")
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--count", type=int, default=300)
    ap.add_argument("--seed", type=int, default=20261117)
    a = ap.parse_args()
    rng = random.Random(a.seed)
    prog = []
    for c in range(a.count):
        prog += program(rng, c)
    text = "\n".join(prog) + "\n"
    outs = []
    for exe in ("driver_m8", "hand_m8_u16"):
        r = subprocess.run([os.path.join(WORK, exe)], input=text, capture_output=True, text=True)
        if r.returncode != 0:
            sys.exit("%s failed rc %d: %s" % (exe, r.returncode, r.stderr[-1000:]))
        outs.append(r.stdout)
    split = [o.split("case ")[1:] for o in outs]
    bad = [i for i, (x, y) in enumerate(zip(*split)) if x != y]
    lines = outs[0].splitlines()
    tally = {t: sum(1 for l in lines if l.startswith(t)) for t in ("R ", "H ", "X 5", "X 2", "X 3")}
    print("cases %d, mismatching cases %d, outcome lines %s" % (len(split[0]), len(bad) + abs(len(split[0]) - len(split[1])), tally))
    for i in bad[:5]:
        print("--- case", i, "\npruv:", split[0][i][:600], "\nhand16:", split[1][i][:600])
    return 1 if bad or len(split[0]) != len(split[1]) else 0


if __name__ == "__main__":
    sys.exit(main())
