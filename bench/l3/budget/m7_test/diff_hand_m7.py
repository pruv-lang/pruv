#!/usr/bin/env python3
"""P02 substep 4: differential check of hand_m7_u16 (q mode) against the Pruv driver_m7 (q mode).

Q1 cases (work/m7/cases.jsonl) hold ordinary keys and well-formed requests, so this generates deterministic driver
programs aimed at the edges and diffs the two executables' output byte for byte (every `@`, `E`, `V`, `X` line and the
final `O` observation of the stored session records):
  * keys (session ids, budget ids, uuids): empty, NUL units, ASCII, non-ASCII BMP, astral pairs, lone high / low
    surrogates, tails of every length 0..7, near-miss budget ids (last unit +1, a unit appended, the last unit dropped),
    a budget id of another session;
  * records: open / halted / closed, every origin, unlimited 0/1/2, activeSince / ended / openedAt as stored text or ms
    or absent, follows as id / other / absent, rest text, extreme counters, `[-1]` (no budget), the empty blob, short
    blobs (< 30 words), a state word outside 0..2 with >= 30 words;
  * calls: current / isHalted / attach / replaceForUserMessage / reopen / maybeClose with valid and invalid origin and
    size words, unit words from 0 to 2^63-1 and negative, the isQuiet queue (0, 1, 2, -1), uuid queue exhausted (empty
    id), `adv` / `now` / `put` over an existing key / `del` + re-put (observation order).
After the random cases (numbered 0..count-1, seed 20261117) a fixed set of deterministic CONTROLS is appended:
  * edge-key programs (one per fixed edge key, every call kind);
  * malformed frames: short body, missing keys, trailing word, key length overrun / negative, unknown code, a frame that
    swallows the uuid key as an extra "key", invalid origin / size, `del` with a malformed key (`X del 2`), maybeClose
    with quiet -1 (`X 5`), call keys with a unit outside 0..65535 (see DOMAIN below), absent sessions;
  * out-buffer limits: budget / uuid ids of 4094 and 4095 units (status 3 above 4096 result words);
  * fatal driver inputs, each in its own process (compared on exit status and stdout, plus stderr for status 2): put
    with n too big / too small / negative, key length overrun / negative, no n word, no words; an unknown command;
    `call` with no request id / no body, `now` without a number, `uuids` with a key running past the line.
Besides byte equality of the two sides, the frame controls have an EXPECTED Pruv log (ABI: malformed host input is
status 2, the case goes on; the driver prints `X 2`).

Finally the hand is run over work/m7/cases.jsonl through `run_m7.py q --hand16` (the same oracle classification as the
Pruv root); 0 fail is required. `--no-q` skips that step.

DOMAIN (documented difference, not tested for equality): the hand stores keys as UTF-16 units and refuses (exit 3) a put
or uuid with a unit outside 0..65535 and a budget record with state 0..2 and >= 30 words that is not canonical; the Pruv
store keeps raw words. A CALL key with such a unit can match nothing in either implementation; the hand answers it as a
miss and the controls "call key unit" check that (the Pruv answer is not assumed by an expectation, only compared).

  python3 bench/l3/budget/m7_test/diff_hand_m7.py [--count 300] [--seed 20261117] [--rebuild] [--no-q]
Builds work/m7/driver_m7 (when missing or --rebuild) and work/m7/hand_m7_u16 through run_m7.py. Exit status 1 on any
mismatch, expectation failure or a failing q classification.
"""
import argparse
import os
import random
import subprocess
import sys

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import run_m7  # noqa: E402

MAXS = 2 ** 53 - 1
I64 = 2 ** 63 - 1
NUMS = [0, 1, -1, 2, 7, 24, 120, 10 ** 12, MAXS, MAXS - 1, -MAXS, 2 ** 53, 2 ** 62, -2 ** 62, I64]
UNITS = [0, 1, 2, 3, 24, 120, -1, -7, MAXS, 2 ** 53, 2 ** 62, I64]
NOWS = [1700000000000, 0, 1, 99, 2 ** 53, 2 ** 53 + 1, 2 ** 62, 1234567890123, -5]
DEF_UNIT = (120, 24, 3, 2, 1, 0)


def kw(u):
    return [len(u)] + list(u)


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


def stamp_text(rng):
    return rng.choice([[], list(map(ord, "2023-11-14T22:13:20.000Z")), key_units(rng)])


def rec(bid, state=0, origin=0, unl=0, limits=(120, 24, 3, 2, 1), used=(0,) * 8, since=None, ended=None,
        opened=("ms", 1700000000000), follows=None, rest=()):
    """Canonical record words (bud_lifecycle.pruv header comment). since / opened: None | ("t", units) | ("ms", n);
    ended: None | (how 1..3, ("t", units) | ("ms", n)); follows: None | "other" | units."""
    h = [0] * 24
    strs = [list(bid), [], [], [], [], list(rest)]
    h[0], h[1], h[2] = state, origin, unl
    h[3:8] = list(limits)
    h[8:16] = list(used)
    if since:
        if since[0] == "t":
            h[16], strs[2] = 1, list(since[1])
        else:
            h[16], h[17] = 2, since[1]
    if ended:
        how, at = ended
        h[18] = how
        if at[0] == "t":
            h[19], strs[3] = 1, list(at[1])
        else:
            h[19], h[22] = 2, at[1]
    if opened:
        if opened[0] == "t":
            h[20], strs[1] = 1, list(opened[1])
        else:
            h[20], h[21] = 2, opened[1]
    if follows is not None:
        if follows == "other":
            h[23] = 2
        else:
            h[23], strs[4] = 1, list(follows)
    w = list(h)
    for s in strs:
        w += [len(s)] + s
    return w


def random_rec(rng, bid):
    state = rng.choice([0, 0, 1, 1, 2, 2])
    unl = rng.choice([0, 0, 1, 2])
    lim = [rng.choice(NUMS) for _ in range(5)] if rng.random() < 0.3 else [120, 24, 3, 2, 1]
    used = [rng.choice(NUMS) for _ in range(8)] if rng.random() < 0.3 else [0] * 8
    since = rng.choice([None, None, ("t", stamp_text(rng)), ("ms", rng.choice(NUMS))])
    ended = None
    if state == 2 or rng.random() < 0.1:
        how = rng.choice([1, 1, 1, 2, 3])
        ended = (how, rng.choice([("t", stamp_text(rng)), ("ms", rng.choice(NUMS))]))
    opened = rng.choice([None, ("t", stamp_text(rng)), ("ms", rng.choice(NOWS)), ("ms", 1700000000000)])
    follows = rng.choice([None, None, "other", key_units(rng)])
    rest = rng.choice([[], list(map(ord, '{"card":{"state":"done"}}')), key_units(rng)])
    return rec(bid, state, rng.randrange(4), unl, lim, used, since, ended, opened, follows, rest)


def blob(rng, bid):
    r = rng.random()
    if r < 0.72:
        return random_rec(rng, bid)
    if r < 0.82:
        return [-1]
    if r < 0.87:
        return []
    if r < 0.92:
        return rng.choice([[0], [1, 2, 3], [2] * 29, [0] * 29])
    if r < 0.96:
        return rng.choice([[7] + [0] * 40, [-3] + [1] * 33])
    return [-1] + [5] * 35


def put_line(skey, words, n=None):
    return "put " + " ".join(map(str, kw(skey) + [len(words) if n is None else n] + list(words)))


class Prog:
    """A driver program of one case: put / call / adv lines with consecutive request ids."""

    def __init__(self, cid):
        self.lines = ["case %s" % cid]
        self.n = 0

    def add(self, line):
        self.lines.append(line)
        return self

    def call_words(self, words):
        self.lines.append("call r%d %s" % (self.n, " ".join(map(str, words))))
        self.n += 1
        return self

    def call(self, code, sk, bk=(), origin=-1, size=-1, unit=DEF_UNIT):
        return self.call_words([code, origin, size] + list(unit) + kw(sk) + kw(bk))

    def end(self):
        self.lines.append("end")
        return self.lines


def near_miss(rng, b):
    r = rng.random()
    if b and r < 0.4:
        return b[:-1] + [(b[-1] + 1) % 65536]
    if r < 0.7:
        return b + [rng.randrange(0, 65536)]
    if b:
        return b[:-1]
    return [rng.randrange(0, 65536)]


def program(rng, case):
    p = Prog(case)
    p.add("now %d" % rng.choice(NOWS))
    sess = [key_units(rng) for _ in range(rng.randrange(1, 4))]
    bids = [key_units(rng) for _ in sess]
    uu = [key_units(rng) for _ in range(rng.randrange(0, 5))]
    p.add("uuids " + " ".join(str(x) for u in uu for x in kw(u)))
    p.add("quiet " + " ".join(str(rng.choice([0, 1, 1, 0, 2, -1])) for _ in range(rng.randrange(0, 5))))
    for s, b in zip(sess, bids):
        p.add(put_line(s, blob(rng, b)))
    for _ in range(rng.randrange(1, 12)):
        r = rng.random()
        if r < 0.07:
            p.add("adv %d" % rng.choice([0, 1, 1000, 86400000, -3]))
        elif r < 0.10:
            p.add("now %d" % rng.choice(NOWS))
        elif r < 0.16:
            j = rng.randrange(len(sess))
            p.add(put_line(sess[j], blob(rng, bids[j])))
        elif r < 0.21:
            p.add("del " + " ".join(map(str, kw(rng.choice(sess + [key_units(rng)])))))
        else:
            j = rng.randrange(len(sess))
            sk = sess[j] if rng.random() > 0.15 else key_units(rng)
            bk = bids[j]
            q = rng.random()
            if q < 0.2:
                bk = near_miss(rng, bk)
            elif q < 0.25:
                bk = rng.choice(bids)
            code = rng.choice([1, 1, 2, 3, 3, 4, 4, 4, 5, 5, 5, 6, 6, 6])
            origin = rng.choice([0, 0, 1, 2, 3, -1, 4, 9]) if code == 3 else rng.choice([-1, 0, 1, 2, 3, 9, -1])
            size = rng.choice([-1, -1, 0, 1, 2, 4, 8, 3, 9, -2])
            unit = [rng.choice(UNITS) if rng.random() < 0.4 else d for d in DEF_UNIT[:5]] + [rng.choice([0, 1, 0, 2])]
            p.call(code, sk, bk, origin, size, unit)
    return p.end()


# ---- fixed controls: (name, lines, expected Pruv log or None) ----
SK = [115]  # session "s"
BID = [98, 99]  # budget id "bc"
EDGE_KEYS = [[], [0], [0, 0], [97], [0xD83D, 0xDE00], [0xD800], [0xDC00], [0xDC00, 0xD800], [0xFFFF], [97] * 7,
             [0xD83D], [0xDE00], [0, 97, 0], [0xFFFF, 0xD800, 0xDC00]]
UU3 = [3, 97, 98, 99]  # uuid key "abc"


def base(cid, state=0, extra=(), quiet="1 1 1", uuids=UU3 + UU3, bid=BID, **kw_):
    p = Prog(cid)
    p.add("now 1700000000000").add("uuids " + " ".join(map(str, uuids))).add("quiet " + quiet)
    p.add(put_line(SK, rec(bid, state, **kw_)))
    for e in extra:
        p.add(e)
    return p


def controls():
    out = []

    def add(name, lines, exp=None):
        out.append((name, lines, exp))

    # edge keys: every call kind on a halted / closed-quiet / open budget whose session id and budget id are the key
    for i, k in enumerate(EDGE_KEYS):
        for st, how in ((1, None), (2, 1), (0, None)):
            p = Prog("edge")
            p.add("now 1700000000000").add("uuids " + " ".join(map(str, kw(k) + kw(k[::-1] + [1]))))
            p.add("quiet 1 0 1")
            p.add(put_line(k, rec(k, st, ended=(1, ("ms", 5)) if how else None)))
            nm = near_miss(random.Random(i), k)
            for code in (1, 2, 5, 6, 3, 4):
                p.call(code, k, k, 0 if code in (3, 4) else -1, 2 if code == 4 else -1)
                p.call(code, k, nm, 0 if code in (3, 4) else -1, 2 if code == 4 else -1)
            p.call(1, k, k)
            add("edge key %r state %d" % (k, st), p.end())

    # malformed frames (non-fatal): `X 2`, the case goes on
    U = list(DEF_UNIT)
    s_, b_ = kw(SK), kw(BID)
    frame = [
        ("short body", [1]),
        ("no keys", [1, -1, -1] + U),
        ("only session key", [1, -1, -1] + U + s_),
        ("trailing word", [1, -1, -1] + U + s_ + b_ + [0]),
        ("session key length overrun", [1, -1, -1] + U + [50, 115] + b_),
        ("session key length negative", [1, -1, -1] + U + [-1, 115] + b_),
        ("budget key length overrun", [1, -1, -1] + U + s_ + [50, 98]),
        ("budget key length negative", [1, -1, -1] + U + s_ + [-2, 98]),
        ("unknown code 0", [0, -1, -1] + U + s_ + b_),
        ("unknown code 7", [7, -1, -1] + U + s_ + b_),
        ("unknown code -1", [-1, -1, -1] + U + s_ + b_),
        ("unknown code 100", [100, -1, -1] + U + s_ + b_),
        ("attach origin -1", [3, -1, -1] + U + s_ + b_),
        ("attach origin 4", [3, 4, -1] + U + s_ + b_),
        ("attach origin 9", [3, 9, -1] + U + s_ + b_),
        ("attach size 3", [3, 0, 3] + U + s_ + b_),
        ("attach size 9", [3, 0, 9] + U + s_ + b_),
        ("attach size -2", [3, 0, -2] + U + s_ + b_),
        ("replace origin 4", [4, 4, 2] + U + s_ + b_),
        ("replace size 3", [4, 0, 3] + U + s_ + b_),
        ("replace size -2", [4, -1, -2] + U + s_ + b_),
    ]
    for name, body in frame:
        for st in (0, 1, 2):
            p = base("frame", st, quiet="1")
            p.call_words(body)
            add("frame %s (state %d)" % (name, st), p.end(), ["@ r0", "X 2"])
    # no uuid left: the host appends an empty key; the missing keys then parse differently but stay status 2
    p = base("frame", 0, uuids=[])
    p.call_words([1, -1, -1] + U + s_)
    add("frame only session key, uuids exhausted", p.end(), ["@ r0", "X 2"])
    # a frame that swallows the uuid key as an extra "key" is well formed: the new budget id is the 4 swallowed words
    for code, st in ((3, 2), (4, 1), (3, 0)):
        p = base("swallow", st, ended=(1, ("ms", 7)) if st == 2 else None)
        p.call_words([code, 0, 2 if code == 4 else -1] + U + s_ + b_ + [4])
        add("frame swallows the uuid key (code %d, state %d)" % (code, st), p.end())
    # session absent
    for code in (1, 2, 3, 4, 5, 6):
        p = Prog("absent")
        p.add("uuids " + " ".join(map(str, UU3))).add("quiet 1")
        p.call(code, SK, BID, 0 if code in (3, 4) else -1, 2 if code == 4 else -1)
        exp = {1: ["@ r0", "V 0"], 2: ["@ r0", "V 1"], 5: ["@ r0", "V 1"]}.get(code, ["@ r0", "X 2"])
        add("session absent code %d" % code, p.end(), exp)
    # maybeClose: quiet -1 on an open budget is `X 5`, on halted / closed / no-budget it is `V 1` (not asked)
    for st, exp in ((0, ["@ r0", "X 5"]), (1, ["@ r0", "V 1"]), (2, ["@ r0", "V 1"])):
        p = base("mcq", st, quiet="-1", ended=(1, ("ms", 7)) if st == 2 else None)
        p.call(6, SK)
        add("maybeClose quiet -1 state %d" % st, p.end(), exp)
    for qv in ("0", "1", "2", "-1 1", "1 0"):
        p = base("mcq", 0, quiet=qv)
        p.call(6, SK).call(5, SK, BID).call(6, SK).call(5, SK, BID).call(6, SK)
        add("maybeClose quiet queue %s" % qv, p.end())
    # del with a malformed key frame logs `X del 2` and the case goes on; trailing words are ignored
    for name, line in (("no words", "del"), ("key length overrun", "del 5 1"), ("key length negative", "del -1"),
                       ("trailing words ignored", "del 1 115 999 999"), ("valid", "del 1 115")):
        p = base("del", 0, extra=[line])
        p.call(1, SK, BID)
        exp = None
        if name in ("no words", "key length overrun", "key length negative"):
            exp = ["X del 2", "@ r0", "V 4"]  # V 4 line is the record; compared by prefix below
        add("del %s" % name, p.end(), exp)
    # delete + re-put moves the session to the end of the observation; put over an existing key keeps its place
    p = Prog("order")
    p.add("uuids " + " ".join(map(str, UU3)))
    for k in ([97], [98], [99]):
        p.add(put_line(k, rec([100 + k[0]])))
    p.add("del " + " ".join(map(str, kw([97])))).add(put_line([97], rec([1]))).add(put_line([98], rec([2], 1)))
    add("observation order after del / re-put", p.end())
    # call keys with a unit outside 0..65535 (domain note: both sides must answer a miss)
    for u in (-1, 65536, 70000):
        bad = [2, 115, u]  # key words `len, units...` with one unit outside 0..65535
        for code in (1, 2, 5):
            for which in ("session", "budget"):
                p = base("badunit", 0, quiet="1")
                sk, bk = (bad, kw(BID)) if which == "session" else (kw(SK), bad)
                p.call_words([code, -1, -1] + U + sk + bk)
                add("call key unit %d, %s key, code %d" % (u, which, code), p.end())
        for code in (3, 4, 6):
            p = base("badunit", 1, quiet="1")
            p.call_words([code, 0, 2 if code == 4 else -1] + U + bad + kw(BID))
            add("call key unit %d, session key, code %d" % (u, code), p.end())
    # out-buffer limits: results of more than 4096 words answer status 3
    for il, exp in ((4094, "V 3"), (4095, "X 3")):
        p = base("outbig", 0, bid=[97] * il)
        p.call(3, SK, [97] * il, 0)
        add("attach ref, budget id %d units" % il, p.end(), ["@ r0", exp] if exp == "X 3" else None)
    for ul, ex in ((4094, "ok"), (4095, "X 3")):
        p = base("outbig", 2, uuids=kw([98] * ul), ended=(1, ("ms", 7)))
        p.call(3, SK, BID, 0)
        add("attach open, uuid %d units" % ul, p.end(), ["@ r0", "X 3"] if ex == "X 3" else None)
    for ol, ex in ((4090, "ok"), (4091, "X 3")):
        p = base("outbig", 1, bid=[97] * ol)
        p.call(4, SK, [97] * ol, 0, 2)
        add("replace halted, budget id %d units, uuid 3" % ol, p.end(), ["@ r0", "X 3"] if ex == "X 3" else None)
    # short / opaque blobs
    for name, w in (("empty blob", []), ("no budget [-1]", [-1]), ("29 words state 0", [0] * 29),
                    ("29 words state 2", [2] * 29), ("state 7, 40 words", [7] + [0] * 39), ("state -3, 40 words", [-3] + [1] * 39)):
        p = Prog("blob")
        p.add("uuids " + " ".join(map(str, UU3))).add("quiet 1")
        p.add(put_line(SK, w))
        for code in (1, 2, 5, 6, 3, 4):
            p.call(code, SK, BID, 0 if code in (3, 4) else -1, 2 if code == 4 else -1)
        p.call(1, SK, BID)
        add("blob %s" % name, p.end())
    return out


# ---- fatal controls: (name, program text, expected exit status) ----
def fatal_controls():
    out = []
    g = rec(BID)

    def prog(*lines):
        return "\n".join(lines) + "\n"

    for name, line in (
        ("put n too big", put_line(SK, g, len(g) + 1)),
        ("put n too small", put_line(SK, g, len(g) - 1)),
        ("put n negative", put_line(SK, g, -1)),
        ("put key length overrun", "put " + " ".join(map(str, [5, 115, 98, len(g)] + g))),
        ("put key length negative", "put " + " ".join(map(str, [-1, 115, len(g)] + g))),
        ("put no n word", "put 1 115"),
        ("put no words", "put"),
    ):
        out.append((name, prog("case f", line, "end"), 2))
    out.append(("unknown command", prog("case f", "bogus 1", "end"), 2))
    out.append(("call without request id", prog("case f", "call"), 101))
    out.append(("call without body", prog("case f", "call r0"), 101))
    out.append(("now without number", prog("case f", "now"), 101))
    out.append(("now not a number", prog("case f", "now x"), 101))
    out.append(("uuids key past the line", prog("case f", "uuids 5 1 2"), 101))
    out.append(("quiet not a number", prog("case f", "quiet 1 x"), 101))
    return out


def run_exe(exe, text):
    r = subprocess.run([exe, "q"], input=text, capture_output=True, text=True)
    return r.returncode, r.stdout, r.stderr


def split_case(chunk):
    lines = chunk.splitlines()
    k = next(i for i, l in enumerate(lines) if l.startswith("O"))
    return lines[1:k], lines[k]


def log_matches(log, exp):
    """exp lists the leading log lines; a `V 4` entry matches any `V 4 ...` line."""
    if len(log) < len(exp):
        return False
    return all(l == e or (e == "V 4" and l.startswith("V 4 ")) for l, e in zip(log, exp)) and (
        len(log) == len(exp) or exp[-1] == "V 4")


def ensure_exes(rebuild):
    drv = os.path.join(run_m7.WORK, "driver_m7")
    if rebuild or not os.path.exists(drv):
        drv = run_m7.build(argparse.Namespace(py=False, pruvc=run_m7.PRUVC_DEFAULT))
    return drv, run_m7.build_hand16()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--count", type=int, default=300)
    ap.add_argument("--seed", type=int, default=20261117)
    ap.add_argument("--rebuild", action="store_true", help="rebuild driver_m7 from the Pruv root first")
    ap.add_argument("--no-q", action="store_true", help="skip run_m7.py q --hand16")
    a = ap.parse_args()
    drv, hand = ensure_exes(a.rebuild)
    rng = random.Random(a.seed)
    prog = []
    for c in range(a.count):
        prog += program(rng, c)
    ctl = controls()
    for i, (name, lines, exp) in enumerate(ctl):
        prog += ["case %d" % (a.count + i)] + lines[1:]
    text = "\n".join(prog) + "\n"
    outs = []
    for exe in (drv, hand):
        rc, so, se = run_exe(exe, text)
        if rc != 0:
            sys.exit("%s failed rc %d: %s" % (os.path.basename(exe), rc, se[-1000:]))
        outs.append(so)
    split = [o.split("case ")[1:] for o in outs]
    want = a.count + len(ctl)
    count_bad = len(split[0]) != want or len(split[1]) != want
    pairs = list(zip(*split))
    bad_rand = [i for i, (x, y) in enumerate(pairs[:a.count]) if x != y]
    bad_ctl = [i for i, (x, y) in enumerate(pairs[a.count:]) if x != y]
    expect_fail = []
    for i, (name, lines, exp) in enumerate(ctl):
        if exp is None or a.count + i >= len(split[0]):
            continue
        log, _ = split_case(split[0][a.count + i])
        if not log_matches(log, exp):
            expect_fail.append((i, name, exp, log))
    # fatal controls: own process each; exit status and stdout (and stderr for status 2) must agree
    fatal = fatal_controls()
    bad_fatal = []
    for name, text_f, want_rc in fatal:
        rd, rh = run_exe(drv, text_f), run_exe(hand, text_f)
        same = rd[0] == rh[0] and rd[1] == rh[1] and (rd[0] != 2 or rd[2] == rh[2])
        if not same or rd[0] != want_rc:
            bad_fatal.append((name, want_rc, rd, rh))
    lines = outs[0].splitlines()
    tally = {t: sum(1 for l in lines if l.startswith(t)) for t in
             ("V 0", "V 1", "V 2", "V 3", "V 4", "E uuid", "E changed", "E carry", "E quiet", "X 2", "X 3", "X 5",
              "X del")}
    print("random cases %d, mismatching cases %d" % (a.count, len(bad_rand)))
    print("controls %d, mismatches between sides %d, expectation failures %d" % (len(ctl), len(bad_ctl), len(expect_fail)))
    print("fatal controls %d, failures %d" % (len(fatal), len(bad_fatal)))
    print("coverage (Pruv output lines) %s" % tally)
    total_mis = len(bad_rand) + len(bad_ctl)
    print("mismatches: %d" % total_mis)
    for i in bad_rand[:5]:
        print("--- random case", i, "\npruv:", pairs[i][0][:700], "\nhand16:", pairs[i][1][:700])
    for i in bad_ctl[:5]:
        print("--- control", i, ctl[i][0], "\npruv:", pairs[a.count + i][0][:700], "\nhand16:", pairs[a.count + i][1][:700])
    for i, name, exp, log in expect_fail[:5]:
        print("--- expectation", i, name, "\nexpected:", exp, "\npruv log:", log[:6])
    for name, want_rc, rd, rh in bad_fatal[:5]:
        print("--- fatal", name, "want rc", want_rc, "\npruv rc %d stdout %r stderr %r\nhand rc %d stdout %r stderr %r"
              % (rd[0], rd[1][:200], rd[2][:200], rh[0], rh[1][:200], rh[2][:200]))
    if count_bad:
        print("case count differs: pruv %d, hand16 %d, expected %d" % (len(split[0]), len(split[1]), want))
    rc = 1 if (bad_rand or bad_ctl or expect_fail or bad_fatal or count_bad) else 0
    if not a.no_q:
        r = subprocess.run([sys.executable, os.path.join(HERE, "run_m7.py"), "q", "--hand16"], capture_output=True,
                           text=True)
        print("run_m7.py q --hand16 exit %d" % r.returncode)
        print(r.stdout[-1500:], end="" if r.stdout.endswith("\n") else "\n")
        if r.returncode != 0:
            print(r.stderr[-1500:])
            rc = 1
    return rc


if __name__ == "__main__":
    sys.exit(main())
