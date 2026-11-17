#!/usr/bin/env python3
"""Reference encoder and deterministic case generator for the M3 flat test encoding (CONTRACT.md).

Value model (tuples): ('absent',) ('undef',) ('null',) ('false',) ('true',) ('int', v) ('str', [units])
('bits', signed64) ('arr', [V..]) ('obj', [([keyunits], V)..]) ('ref', [units]) and ('raw', [words]) for
hand-made malformed fragments.

make_cases(seed, count=300) -> list of dicts:
  kind    'call' | 'resume' | 'outcome'
  words   the input words
  expect  ('ok',) : status == len(words) and out == words      |  ('err', code): status == code
  cat     category label
  size    for valid cases: the encoded size from the contract size formula (None otherwise)
Python 3.9, standard library only.
"""
import random

MAGIC_CALL = 0x42554443
MAGIC_RESUME = 0x42554452
MAGIC_OUT = 0x4255444F
VERSION = 1
MAX_WORDS = 16777216
INT_MAX = 2 ** 53 - 1
NOW_MAX = 8640000000000000
VALID_OPS = list(range(1, 26)) + list(range(30, 36))
KINDS = ("call", "resume", "outcome")
MAGIC = {"call": MAGIC_CALL, "resume": MAGIC_RESUME, "outcome": MAGIC_OUT}


def sgn64(x):
    x &= (1 << 64) - 1
    return x - (1 << 64) if x >= (1 << 63) else x


# ---- encoder -------------------------------------------------------------------------------------------------------

def enc(v):
    t = v[0]
    if t == "absent":
        return [0]
    if t == "undef":
        return [1]
    if t == "null":
        return [2]
    if t == "false":
        return [3]
    if t == "true":
        return [4]
    if t == "int":
        return [5, v[1]]
    if t == "str":
        return [6, len(v[1])] + list(v[1])
    if t == "bits":
        return [7, v[1]]
    if t == "arr":
        out = [8, len(v[1])]
        for c in v[1]:
            out += enc(c)
        return out
    if t == "obj":
        out = [9, len(v[1])]
        for k, c in v[1]:
            out += [len(k)] + list(k) + enc(c)
        return out
    if t == "ref":
        return [10, len(v[1])] + list(v[1])
    if t == "raw":
        return list(v[1])
    raise ValueError(t)


def size(v):
    """Encoded size by the contract's size formula (independent of enc)."""
    t = v[0]
    if t in ("absent", "undef", "null", "false", "true"):
        return 1
    if t in ("int", "bits"):
        return 2
    if t in ("str", "ref"):
        return 2 + len(v[1])
    if t == "arr":
        return 2 + sum(size(c) for c in v[1])
    if t == "obj":
        return 2 + sum(1 + len(k) + size(c) for k, c in v[1])
    if t == "raw":
        return len(v[1])
    raise ValueError(t)


def effect_words(e):
    code, args = e
    out = [code, len(args)]
    for a in args:
        out += enc(a)
    return out


def effect_size(e):
    return 2 + sum(size(a) for a in e[1])


def call_msg(op, now, uuid, args):
    w = [MAGIC_CALL, VERSION, 0, op, now, len(uuid)] + list(uuid) + [len(args)]
    for a in args:
        w += enc(a)
    w[2] = len(w)
    # Contract: call = 7 + uuid_len + sum(size(V)).
    return w, 7 + len(uuid) + sum(size(a) for a in args)


def resume_msg(seq, now, ans):
    w = [MAGIC_RESUME, VERSION, 0, seq, now] + enc(ans)
    w[2] = len(w)
    # The words are magic, version, total_len, seq, now_ms = 5 header words. CONTRACT.md says "6 + size(V)", which
    # does not match its own layout; this harness uses 5 + size(V) and flags the difference.
    return w, 5 + size(ans)


def outcome_msg(o):
    k = o[0]
    w = [MAGIC_OUT, VERSION, 0]
    if k == "done":
        _, res, effs = o
        w += [1] + enc(res) + [len(effs)]
        for e in effs:
            w += effect_words(e)
        sz = 4 + size(res) + 1 + sum(effect_size(e) for e in effs)
    elif k == "need":
        _, effs, qcode, qargs, seq = o
        w += [2, len(effs)]
        for e in effs:
            w += effect_words(e)
        w += [qcode, len(qargs)]
        for a in qargs:
            w += enc(a)
        w += [seq]
        sz = 4 + 1 + sum(effect_size(e) for e in effs) + 2 + sum(size(a) for a in qargs) + 1
    elif k == "invalid":
        w += [3, o[1]]
        sz = 5
    elif k == "bound":
        w += [4, o[1]]
        sz = 5
    elif k == "grow":
        w += [5, o[1]]
        sz = 5
    else:
        raise ValueError(k)
    w[2] = len(w)
    return w, sz


# ---- random values -------------------------------------------------------------------------------------------------

def rand_unit(rng):
    r = rng.random()
    if r < 0.55:
        return rng.randint(32, 126)
    if r < 0.65:
        return rng.choice([0xE9, 0x20AC, 0xFF, 0x100, 0x7FF, 0x800])
    if r < 0.78:
        return rng.randint(0x4E00, 0x9FFF)
    if r < 0.85:
        return rng.randint(0xD800, 0xDBFF)      # lone high surrogate
    if r < 0.92:
        return rng.randint(0xDC00, 0xDFFF)      # lone low surrogate
    return rng.choice([0, 0xFFFF, 0xFFFE, 0xD7FF, 0xE000])


def rand_units(rng, maxn=10):
    n = rng.choice([0, 0, 1, 1, 2, 3, 4, 6, maxn])
    u = []
    while len(u) < n:
        if rng.random() < 0.1:
            u += [rng.randint(0xD800, 0xDBFF), rng.randint(0xDC00, 0xDFFF)]    # a proper pair
        else:
            u.append(rand_unit(rng))
    return u[:n]


def rand_int(rng):
    return rng.choice([0, 1, -1, INT_MAX, -INT_MAX, rng.randint(-10 ** 6, 10 ** 6), rng.randint(-INT_MAX, INT_MAX)])


def rand_bits(rng):
    return rng.choice([0, -1, -(1 << 63), (1 << 63) - 1, sgn64(rng.getrandbits(64)), sgn64(rng.getrandbits(64))])


def rand_value(rng, depth=1, maxd=4, absent=False):
    ch = ["undef", "null", "false", "true", "int", "int", "str", "str", "bits", "ref"]
    if depth < maxd:
        ch += ["arr", "obj", "arr", "obj"]
    if absent and depth == 1:
        ch += ["absent"]
    t = rng.choice(ch)
    if t in ("undef", "null", "false", "true", "absent"):
        return (t,)
    if t == "int":
        return ("int", rand_int(rng))
    if t == "str":
        return ("str", rand_units(rng))
    if t == "bits":
        return ("bits", rand_bits(rng))
    if t == "ref":
        return ("ref", rand_units(rng))
    if t == "arr":
        return ("arr", [rand_value(rng, depth + 1, maxd) for _ in range(rng.choice([0, 1, 2, 3]))])
    return ("obj", [(rand_units(rng, 5), rand_value(rng, depth + 1, maxd)) for _ in range(rng.choice([0, 1, 2, 3]))])


def chain(depth, leaf=None):
    """A value whose deepest container sits at nesting depth `depth` (top-level value is depth 1)."""
    v = ("arr", [] if leaf is None else [leaf])
    for i in range(depth - 1):
        v = ("arr", [v]) if i % 2 == 0 else ("obj", [([97 + i % 20], v)])
    return v


def rand_uuid(rng):
    if rng.random() < 0.3:
        return []
    return [rng.choice(b"0123456789abcdef-") if rng.random() < 0.95 else rand_unit(rng) for _ in range(36)]


def rand_now(rng):
    return rng.choice([0, 1, NOW_MAX, rng.randint(0, NOW_MAX), 1700000000000])


def rand_effect(rng):
    return (rng.randint(1, 13), [rand_value(rng, 1, 3) for _ in range(rng.randint(0, 4))])


def rand_call(rng, args=None, op=None, now=None, uuid=None):
    if args is None:
        args = [rand_value(rng, 1, 4, absent=True) for _ in range(rng.randint(0, 8))]
    return call_msg(rng.choice(VALID_OPS) if op is None else op, rand_now(rng) if now is None else now,
                    rand_uuid(rng) if uuid is None else uuid, args)


def rand_resume(rng, ans=None):
    return resume_msg(rng.choice([1, 2, 7, rng.randint(1, 10 ** 6)]), rand_now(rng),
                      rand_value(rng, 1, 4) if ans is None else ans)


def rand_outcome(rng, k=None):
    k = k or rng.choice(["done", "need", "invalid", "bound", "grow"])
    if k == "done":
        return outcome_msg(("done", rand_value(rng, 1, 4), [rand_effect(rng) for _ in range(rng.randint(0, 4))]))
    if k == "need":
        return outcome_msg(("need", [rand_effect(rng) for _ in range(rng.randint(0, 3))], rng.randint(1, 3),
                            [rand_value(rng, 1, 3) for _ in range(rng.randint(0, 4))], rng.randint(1, 1000)))
    if k == "invalid":
        return outcome_msg(("invalid", rng.randint(1, 8)))
    if k == "bound":
        return outcome_msg(("bound", rng.randint(1, 3)))
    return outcome_msg(("grow", rng.choice([1, 64, 4096, rng.randint(1, 10 ** 7)])))


def rand_valid(rng, kind):
    if kind == "call":
        return rand_call(rng)
    if kind == "resume":
        return rand_resume(rng)
    return rand_outcome(rng)


def with_value(rng, kind, v):
    """A structurally valid message of `kind` that contains the value v at a top-level value position."""
    if kind == "call":
        pre = [rand_value(rng, 1, 3) for _ in range(rng.randint(0, 3))]
        post = [rand_value(rng, 1, 3) for _ in range(rng.randint(0, 2))]
        return rand_call(rng, args=pre + [v] + post)[0]
    if kind == "resume":
        return rand_resume(rng, ans=v)[0]
    if rng.random() < 0.5:
        return outcome_msg(("done", v, []))[0]
    return outcome_msg(("need", [], rng.randint(1, 3), [v], 1))[0]


def fix_len(w):
    w[2] = len(w)
    return w


# ---- valid specials -------------------------------------------------------------------------------------------------

def special_valid(rng):
    out = []   # (kind, (words, size), cat)
    u = lambda s: [ord(c) for c in s]
    uu = [ord("a") + i % 26 for i in range(36)]
    out.append(("call", call_msg(6, 0, [], []), "valid-min"))
    out.append(("call", call_msg(35, NOW_MAX, uu, [("absent",)] * 8), "valid-absent8-nowmax"))
    out.append(("call", call_msg(1, 5, uu, [("undef",), ("null",), ("false",), ("true",), ("absent",)]), "valid-simple-tags"))
    out.append(("call", call_msg(8, 7, [], [("int", INT_MAX), ("int", -INT_MAX), ("int", 0), ("int", -1)]), "valid-int-edges"))
    out.append(("call", call_msg(9, 7, [], [("str", []), ("str", u("")), ("ref", []), ("arr", []), ("obj", [])]), "valid-empties"))
    out.append(("call", call_msg(3, 7, [], [("str", [0xD800]), ("str", [0xDC00, 0xD800]), ("str", [0xD83D, 0xDE00]),
                                            ("str", [0xFFFF, 0, 0xE9, 0x65E5])]), "valid-surrogates"))
    out.append(("call", call_msg(4, 9, uu, [("bits", 0), ("bits", -1), ("bits", -(1 << 63)), ("bits", (1 << 63) - 1)]), "valid-bits"))
    out.append(("call", call_msg(5, 9, [], [chain(32)]), "valid-depth32"))
    out.append(("call", call_msg(5, 9, [], [chain(31, ("int", INT_MAX))]), "valid-depth32-leaf"))
    out.append(("call", call_msg(25, 9, [], [("obj", [(u("a"), ("arr", [("obj", [(u(""), ("null",))])])), (u("a"), ("true",))]),
                                             ("ref", u("session-1"))]), "valid-nested-dupkeys"))
    for v in [("undef",), ("null",), ("false",), ("true",), ("int", -INT_MAX), ("str", [0xD800]), ("bits", -1),
              ("arr", [("int", 1), ("str", [])]), ("obj", [([], ("null",))]), ("ref", u("s")), chain(32)]:
        out.append(("resume", resume_msg(1, 0, v), "valid-resume-special"))
    out.append(("outcome", outcome_msg(("done", ("null",), [])), "valid-out-done-min"))
    out.append(("outcome", outcome_msg(("done", ("int", INT_MAX), [(1, [("str", u("s1")), ("obj", [(u("k"), ("int", -INT_MAX))])]),
                                                                   (13, [("str", u("b"))])])), "valid-out-done-effects"))
    out.append(("outcome", outcome_msg(("need", [(12, [("str", u("b")), ("int", 1000)])], 3, [("str", u("s")), ("str", u("b"))], 4)),
                "valid-out-need"))
    out.append(("outcome", outcome_msg(("need", [], 1, [("ref", u("r"))], 1)), "valid-out-need-min"))
    for k in ("invalid", "bound", "grow"):
        out.append(("outcome", outcome_msg((k, 1)), "valid-out-" + k))
    return out


# ---- rejections ----------------------------------------------------------------------------------------------------

def rej_magic(rng, kind):
    w = rand_valid(rng, kind)[0]
    other = [m for k, m in MAGIC.items() if k != kind]
    w[0] = rng.choice([0, 1, other[0], other[1], MAGIC[kind] ^ 1, -1, 1 << 40])
    return w, -301


def rej_version(rng, kind):
    w = rand_valid(rng, kind)[0]
    w[1] = rng.choice([0, 2, -1, 256, 1 << 33])
    return w, -301


def rej_length(rng, kind):
    w = rand_valid(rng, kind)[0]
    d = rng.choice([-3, -2, -1, 1, 2, 5, 100])
    w[2] = len(w) + d
    return w, -302


def rej_toolong(rng, kind):
    w = rand_valid(rng, kind)[0]
    w[2] = MAX_WORDS + rng.choice([1, 2, 1000, 1 << 40])
    return w, -401


def rej_tag(rng, kind):
    return with_value(rng, kind, ("raw", [rng.choice([11, 12, 99, -1, 1 << 40])])), -303


def rej_unit(rng, kind):
    bad = rng.choice([65536, 70000, -1, 1 << 40])
    v = rng.choice([("raw", [6, 2, 65, bad]), ("raw", [10, 1, bad]), ("raw", [6, 1, bad]),
                    ("arr", [("raw", [6, 3, 1, bad, 2])])])
    if v[0] == "arr":
        v = ("raw", [8, 1, 6, 3, 1, bad, 2])
    return with_value(rng, kind, v), -303


def rej_nested_absent(rng, kind):
    v = rng.choice([("arr", [("absent",)]), ("arr", [("int", 1), ("absent",)]), ("obj", [([97], ("absent",))])])
    return with_value(rng, kind, v), -303


def rej_overflow(rng, kind):
    # A count larger than the remaining words (the fragment is placed anywhere; 10**6 exceeds every message here).
    t = rng.choice([6, 8, 9, 10])
    return with_value(rng, kind, ("raw", [t, rng.choice([10 ** 6, 1 << 40])])), -303


def rej_negcount(rng, kind):
    t = rng.choice([6, 8, 9, 10])
    return with_value(rng, kind, ("raw", [t, rng.choice([-1, -5, -(1 << 40)])])), -303


def rej_op(rng, kind):
    bad = rng.choice([0, 26, 29, 36, 100, -1, 1 << 33])
    return rand_call(rng, op=bad)[0], -304


def rej_trailing(rng, kind):
    w = rand_valid(rng, kind)[0]
    w += [rng.choice([0, 1, 5, 99]) for _ in range(rng.randint(1, 6))]
    return fix_len(w), -305


def rej_depth(rng, kind):
    d = rng.choice([33, 33, 34, 40])
    return with_value(rng, kind, chain(d)), -306


def rej_int(rng, kind):
    bad = rng.choice([INT_MAX + 1, -INT_MAX - 1, 1 << 62, -(1 << 62), (1 << 63) - 1, -(1 << 63)])
    v = rng.choice([("int", bad), ("arr", [("int", 3), ("int", bad)]), ("obj", [([98], ("int", bad))])])
    return with_value(rng, kind, v), -402


def rej_now(rng, kind):
    bad = rng.choice([-1, NOW_MAX + 1, 1 << 62, -(1 << 40)])
    return rand_call(rng, now=bad)[0], -402


def rej_combo(rng, i):
    """Several faults at once: the expected code is the first check in the documented order."""
    k = i % 10
    if k == 0:
        w = rand_valid(rng, "call")[0]; w[0] = 7; w[2] += 3
        return "call", w, -301
    if k == 1:
        w = rand_valid(rng, "resume")[0]; w[1] = 2; w[2] -= 1
        return "resume", w, -301
    if k == 2:
        w = rand_valid(rng, "call")[0]; w[0] = 7; w[2] = MAX_WORDS + 1
        return "call", w, -401
    if k == 3:
        w = with_value(rng, "call", ("raw", [11])); w[2] += 2
        return "call", w, -302
    if k == 4:
        w = rand_call(rng, op=99)[0]; w[2] = MAX_WORDS + 5
        return "call", w, -401
    if k == 5:
        w = with_value(rng, "outcome", ("int", INT_MAX + 1)); w[0] = 0
        return "outcome", w, -301
    if k == 6:
        w = rand_call(rng, op=99)[0] + [1, 2]; fix_len(w)
        return "call", w, -304
    if k == 7:
        w = with_value(rng, "resume", ("raw", [11])) + [0]; fix_len(w)
        return "resume", w, -303
    if k == 8:
        w = rand_valid(rng, "outcome")[0]; w[0] = 5; w[2] = MAX_WORDS * 2
        return "outcome", w, -401
    w = with_value(rng, "call", chain(33)) + [4]; fix_len(w)
    return "call", w, -306


def make_cases(seed, count=300):
    rng = random.Random(seed)
    cases = []

    def valid(kind, ws, cat):
        cases.append({"kind": kind, "words": ws[0], "expect": ("ok",), "cat": cat, "size": ws[1]})

    def reject(kind, w, code, cat):
        cases.append({"kind": kind, "words": w, "expect": ("err", code), "cat": cat, "size": None})

    for kind, ws, cat in special_valid(rng):
        valid(kind, ws, cat)
    want_valid = round(count * 0.4)
    quota = {"call": round(want_valid * 0.55), "resume": round(want_valid * 0.2)}
    quota["outcome"] = want_valid - quota["call"] - quota["resume"]
    have = {"call": 0, "resume": 0, "outcome": 0}
    for c in cases:
        have[c["kind"]] += 1
    for kind in KINDS:
        while have[kind] < quota[kind]:
            valid(kind, rand_valid(rng, kind), "valid-random")
            have[kind] += 1

    plan = [
        ("magic", rej_magic, 12, True), ("version", rej_version, 10, True), ("length", rej_length, 16, True),
        ("total_len>max", rej_toolong, 10, True), ("bad-tag", rej_tag, 14, True), ("bad-units", rej_unit, 14, True),
        ("nested-absent", rej_nested_absent, 8, True), ("count-overflow", rej_overflow, 8, True),
        ("negative-count", rej_negcount, 6, True), ("bad-op", rej_op, 16, False), ("trailing", rej_trailing, 16, True),
        ("depth>32", rej_depth, 14, True), ("int-range", rej_int, 14, True), ("now-range", rej_now, 12, False),
    ]
    want_rej = count - len(cases)
    base = sum(p[2] for p in plan) + 10
    for name, fn, n, any_kind in plan:
        n = max(1, round(n * want_rej / base))
        for i in range(n):
            kind = KINDS[i % 3] if any_kind else "call"
            w, code = fn(rng, kind)
            reject(kind, w, code, name)
    i = 0
    while len(cases) < count:
        # Combos first (deterministic cycle), then extra random valid calls if the plan fell short.
        if i < max(1, round(10 * want_rej / base)):
            kind, w, code = rej_combo(rng, i)
            reject(kind, w, code, "combo")
        else:
            valid("call", rand_call(rng), "valid-random")
        i += 1
    cases = cases[:count] if len(cases) > count else cases
    # Deterministic shuffle so that categories are interleaved.
    rng.shuffle(cases)
    return cases


def input_line(c):
    return c["kind"] + " " + " ".join(str(x) for x in c["words"])


def expected_words(c):
    if c["expect"][0] == "ok":
        return [len(c["words"])] + list(c["words"])
    return [c["expect"][1]]


if __name__ == "__main__":
    import sys
    seed = int(sys.argv[1]) if len(sys.argv) > 1 else 1
    for c in make_cases(seed):
        print(input_line(c))
