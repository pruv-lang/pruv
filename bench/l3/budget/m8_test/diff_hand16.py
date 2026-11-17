#!/usr/bin/env python3
"""P01 Q2 Unicode/numeric control: differential check of hand_m8_u16 (q mode) against the Pruv driver_m8 (q mode).

Q2 (q_gen) values never reach the safe-integer edge and rarely use lone surrogates, so this generates deterministic
driver programs aimed at those edges and diffs the two executables' output byte for byte:
  * keys: empty, NUL units, ASCII, non-ASCII BMP, astral pairs, lone high/low surrogates, tails of every length 0..7,
    near-miss budget ids (differ in the last unit);
  * numbers: counters and amounts at 0, +-1, +-(2^53-1), +-2^53, limits at the boundary, negative amounts, unlimited;
  * states open / halted (every reason, absent reason) / closed; noteTool dedup, forgetTurn, failures at the edge.

After the random cases (same seed / stream as before, numbered 0..count-1) a fixed set of deterministic MALFORMED
controls is appended (numbered count..): records with state 3 / -2, unlimited 2 / -1, reason 5 / -2, n 0 / 14 words,
word 14 != n-15 (too big / too small), id unit 65536 / -1; key units -1 / 65536 / 70000 in the put session key, the
call session / budget key, the noteTool turn / activity key and the forgetTurn turn key; put framing (n mismatch,
n negative, key length overrun / negative, missing n). Each bad put is run twice (no session / a valid record
present) followed by charge, noteFailure, noteTool on that ref, a valid put on the same key and a charge. Besides
byte equality of the two sides, each control has an EXPECTED Pruv log (malformed put -> `P X 2`, state unchanged;
malformed call key -> `X 2`). Policy (ABI.md, REQUEST_BUDGET_SPEC section 12): malformed host input is InvalidInput.

  python3 bench/l3/budget/m8_test/diff_hand16.py [--count 300] [--seed 20261117]
Prints: random cases + mismatches; malformed controls + mismatches between sides + expectation failures; the outcome
tally (R / H / X 5 / X 2 / X 3 / P X). Exit status 1 on any mismatch or expectation failure.
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


# ---- malformed fixed controls (deterministic, no rng): each is a full program plus the EXPECTED Pruv log ----
# The expected log is the driver output between `case <id>` and the `O ...` line: `P X 2` for a rejected put, `@ i`
# per step, `R 1` / `R -` / `R 0` per call result, `X 2` for a call with a malformed key (the case stops there).
SK = [115]  # session key "s"
BID = [98, 99]  # budget id "bc"
TURN = [116]
ACT = [97]
WHAT = [0, 0, 2, 1, 0, 0, 0, 0, 0, 0]  # charge commands 1
BAD_UNITS = [-1, 65536, 70000]


def good_rec(bid=BID):
    return [0, 0, -1] + [10 ** 12] * 4 + [0] * 7 + [len(bid)] + bid


def put_line(skw, rec, n=None):
    """skw = the session key words `len, units...`; n = the record word count word (default len(rec))."""
    return "put " + " ".join(map(str, skw + [len(rec) if n is None else n] + rec))


def call_line(op):
    return "call " + " ".join(map(str, op))


SKW = kw(SK)
REF = SKW + kw(BID)
CHARGE = [1] + REF + WHAT
FAIL = [2] + REF
TOOL = [3] + REF + kw(TURN) + kw(ACT)


def bad_put_controls(name, bad_line):
    """Two programs per bad put line. A: the session does not exist; B: it holds a valid record. After the bad put
    (expected `P X 2`, state unchanged) charge / noteFailure / noteTool on that ref, then a valid put on the same key
    and a charge."""
    valid = put_line(SKW, good_rec())
    a =[bad_line, "step 0", call_line(CHARGE), "step 1", call_line(FAIL), "step 2", call_line(TOOL), valid,
         "step 3", call_line(CHARGE), "end"]
    ea = ["P X 2", "@ 0", "R 1", "@ 1", "R -", "@ 2", "R -", "@ 3", "R 0"]
    b = [valid, bad_line, "step 0", call_line(CHARGE), "step 1", call_line(FAIL), "step 2", call_line(TOOL), valid,
         "step 3", call_line(CHARGE), "end"]
    eb = ["P X 2", "@ 0", "R 0", "@ 1", "R -", "@ 2", "R -", "@ 3", "R 0"]
    return [(name + " (no session)", a, ea), (name + " (valid record kept)", b, eb)]


def controls():
    out = []
    g = good_rec()

    def rec_with(i, v):
        r = list(g)
        r[i] = v
        return r

    # record words
    bad_recs = [
        ("state 3", rec_with(0, 3)), ("state -2", rec_with(0, -2)),
        ("unlimited 2", rec_with(1, 2)), ("unlimited -1", rec_with(1, -1)),
        ("reason 5", rec_with(2, 5)), ("reason -2", rec_with(2, -2)),
        ("n 0", []), ("n 14", g[:14]),
        ("idLen too big", rec_with(14, len(BID) + 1)), ("idLen too small", rec_with(14, len(BID) - 1)),
        ("id unit 65536", g[:15] + [98, 65536]), ("id unit -1", g[:15] + [98, -1]),
    ]
    for name, r in bad_recs:
        out += bad_put_controls("record " + name, put_line(SKW, r))
    # key units of the put session key
    for u in BAD_UNITS:
        out += bad_put_controls("put session key unit %d" % u, put_line([2, 115, u], g))
    # framing
    out += bad_put_controls("framing n too big", put_line(SKW, g, len(g) + 1))
    out += bad_put_controls("framing n too small", put_line(SKW, g, len(g) - 1))
    out += bad_put_controls("framing n negative", put_line(SKW, g, -1))
    out += bad_put_controls("framing key length overrun", "put " + " ".join(map(str, [5, 115, 98] + [len(g)] + g)))
    out += bad_put_controls("framing key length negative", "put " + " ".join(map(str, [-1, 115] + [len(g)] + g)))
    out += bad_put_controls("framing no n word", "put 1 115")
    # call keys: a valid session exists, the call must answer X 2 and stop the case
    valid = put_line(SKW, g)
    for u in BAD_UNITS:
        bk = [2, 115, u]  # a malformed key `len, units...` (the unit u is out of 0..65535)
        calls = [
            ("call session key charge", [1] + bk + kw(BID) + WHAT),
            ("call session key noteFailure", [2] + bk + kw(BID)),
            ("call session key noteTool", [3] + bk + kw(BID) + kw(TURN) + kw(ACT)),
            ("call budget key charge", [1] + SKW + bk + WHAT),
            ("call budget key noteFailure", [2] + SKW + bk),
            ("call budget key noteTool", [3] + SKW + bk + kw(TURN) + kw(ACT)),
            ("noteTool turn key", [3] + REF + bk + kw(ACT)),
            ("noteTool activity key", [3] + REF + kw(TURN) + bk),
            ("forgetTurn turn key", [4] + bk),
        ]
        for name, op in calls:
            out.append(("%s unit %d" % (name, u), [valid, "step 0", call_line(op), "end"], ["@ 0", "X 2"]))
    return out


def split_case(chunk):
    """chunk of `case ` split: (log lines, observation line)"""
    lines = chunk.splitlines()
    k = next(i for i, l in enumerate(lines) if l.startswith("O"))
    return lines[1:k], lines[k]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--count", type=int, default=300)
    ap.add_argument("--seed", type=int, default=20261117)
    a = ap.parse_args()
    rng = random.Random(a.seed)
    prog = []
    for c in range(a.count):
        prog += program(rng, c)
    ctl = controls()
    for i, (name, lines, exp) in enumerate(ctl):
        prog += ["case %d" % (a.count + i)] + lines
    text = "\n".join(prog) + "\n"
    outs = []
    for exe in ("driver_m8", "hand_m8_u16"):
        r = subprocess.run([os.path.join(WORK, exe)], input=text, capture_output=True, text=True)
        if r.returncode != 0:
            sys.exit("%s failed rc %d: %s" % (exe, r.returncode, r.stderr[-1000:]))
        outs.append(r.stdout)
    split = [o.split("case ")[1:] for o in outs]
    want = a.count + len(ctl)
    count_bad = len(split[0]) != want or len(split[1]) != want
    pairs = list(zip(*split))
    bad_rand = [i for i, (x, y) in enumerate(pairs[:a.count]) if x != y]
    bad_ctl = [i for i, (x, y) in enumerate(pairs[a.count:]) if x != y]
    expect_fail = []
    for i, (name, lines, exp) in enumerate(ctl):
        if a.count + i >= len(split[0]):
            break
        log, _ = split_case(split[0][a.count + i])
        if log != exp:
            expect_fail.append((i, name, exp, log))
    lines = outs[0].splitlines()
    tally = {t: sum(1 for l in lines if l.startswith(t)) for t in ("R ", "H ", "X 5", "X 2", "X 3", "P X")}
    print("random cases %d, mismatching cases %d" % (a.count, len(bad_rand)))
    print("malformed controls %d, mismatches between sides %d, expectation failures %d"
          % (len(ctl), len(bad_ctl), len(expect_fail)))
    print("outcome lines (Pruv) %s" % tally)
    for i in bad_rand[:5]:
        print("--- random case", i, "\npruv:", pairs[i][0][:600], "\nhand16:", pairs[i][1][:600])
    for i in bad_ctl[:5]:
        print("--- control", i, ctl[i][0], "\npruv:", pairs[a.count + i][0][:600], "\nhand16:",
              pairs[a.count + i][1][:600])
    for i, name, exp, log in expect_fail[:5]:
        print("--- expectation", i, name, "\nexpected:", exp, "\npruv log:", log)
    if count_bad:
        print("case count differs: pruv %d, hand16 %d, expected %d" % (len(split[0]), len(split[1]), want))
    return 1 if bad_rand or bad_ctl or expect_fail or count_bad else 0


if __name__ == "__main__":
    sys.exit(main())
