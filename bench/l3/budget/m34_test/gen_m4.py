#!/usr/bin/env python3
"""Python model of the M4 op table (CONTRACT.md) and a deterministic generator of driver programs.

Python dicts keep insertion order, which equals JS Map semantics: re-setting a key keeps its position, deleting
and setting again appends. The model is the expected-output oracle for driver_m4 `q`.

gen_programs(seed, count=300) -> list of {"lines": [...], "expected": [...], "init": L}
  lines     driver input lines: `init <L>`, `op <words>`, `observe`
  expected  one expected output line per input line:
              `init ok`
              `ok <result words>` | `invalid`
              `stats <live non-root entries>`   (only that count and len(st) are compared; see run.py)
              `observe <n> <words>`
Keys are `len, units...`, in ops and in results. Python 3.9, standard library only.
"""
import random


def u16(s):
    b = s.encode("utf-16-le", "surrogatepass")
    return tuple(b[i] | (b[i + 1] << 8) for i in range(0, len(b), 2))


BASE_KEYS = [u16(s) for s in ["a", "b", "ab", "ba", "aa", "", "é", "日本", "\U0001F600", "x y", "K", "k"]] + \
            [(0xD800,), (0xDC00,), (0xFFFF,), (0,)]


def kw(k):
    return [len(k)] + list(k)


def klist(keys):
    out = [len(keys)]
    for k in keys:
        out += kw(k)
    return out


class Model:
    def __init__(self):
        self.sessions = {}     # key -> list of words
        self.clocks = {}       # key -> {"sid": key, "f": [since_present, since, armed, deadline], "runs": {key: waiting}}
        self.turns = {}        # key -> {activity key: True}
        self.ghosts = 0        # children cut off by clock_del / tool_forget so far (an upper bound on ghosts in st)

    def live_nonroot(self):
        return (len(self.sessions) + len(self.clocks) + sum(len(c["runs"]) for c in self.clocks.values())
                + len(self.turns) + sum(len(t) for t in self.turns.values()))

    def apply(self, code, p):
        """Returns the result word list (stats: None)."""
        if code == 1:
            new = p[0] not in self.sessions
            self.sessions[p[0]] = list(p[1])
            return [1 if new else 0]
        if code == 2:
            b = self.sessions.get(p[0])
            return [0] if b is None else [1, len(b)] + b
        if code == 3:
            return [1] if self.sessions.pop(p[0], None) is not None else [0]
        if code == 4:
            return klist(list(self.sessions))
        if code == 5:
            c = self.clocks.get(p[0])
            if c is None:
                self.clocks[p[0]] = {"sid": p[1], "f": [0, 0, 0, 0], "runs": {}}
                return [1]
            c["sid"] = p[1]     # replaced, position and the other fields kept
            return [0]
        if code == 6:
            c = self.clocks.pop(p[0], None)
            if c is None:
                return [0]
            self.ghosts += len(c["runs"])
            return [1]
        if code == 7:
            c = self.clocks.get(p[0])
            if c is None:
                return [0]
            c["f"] = [p[1], p[2], p[3], p[4]]
            return [1]
        if code == 8:
            c = self.clocks.get(p[0])
            if c is None:
                return [0]
            return [1] + kw(c["sid"]) + c["f"] + [len(c["runs"])]
        if code == 9:
            c = self.clocks.get(p[0])
            if c is None:
                return [2]
            new = p[1] not in c["runs"]
            c["runs"][p[1]] = p[2]
            return [1 if new else 0]
        if code == 10:
            c = self.clocks.get(p[0])
            if c is None:
                return [0]
            return [1] if c["runs"].pop(p[1], None) is not None else [0]
        if code == 11:
            c = self.clocks.get(p[0])
            if c is None:
                return [0]
            out = [1, len(c["runs"])]
            for k, wv in c["runs"].items():
                out += kw(k) + [wv]
            return out
        if code == 12:
            return klist(list(self.clocks))
        if code == 13:
            t = self.turns.get(p[0])
            if t is None:
                t = self.turns[p[0]] = {}
            new = p[1] not in t
            t[p[1]] = True
            return [1 if new else 0]
        if code == 14:
            t = self.turns.pop(p[0], None)
            if t is None:
                return [0]
            self.ghosts += len(t)
            return [1]
        if code == 15:
            t = self.turns.get(p[0])
            return [len(t) if t else 0]
        if code == 16:
            t = self.turns.get(p[0])
            return klist(list(t)) if t else [0]
        if code == 17:
            return klist(list(self.turns))
        if code == 18:
            return None
        raise ValueError(code)

    def observe(self):
        w = [len(self.sessions)]
        for k, b in self.sessions.items():
            w += kw(k) + [len(b)] + b
        w.append(len(self.clocks))
        for k, c in self.clocks.items():
            w += kw(k) + kw(c["sid"]) + c["f"] + [len(c["runs"])]
            for rk, wv in c["runs"].items():
                w += kw(rk) + [wv]
        w.append(len(self.turns))
        for k, t in self.turns.items():
            w += kw(k) + [len(t)]
            for a in t:
                w += kw(a)
        return w


def encode(code, p):
    w = [code]
    if code == 1:
        w += kw(p[0]) + [len(p[1])] + list(p[1])
    elif code in (2, 3, 6, 8, 11, 14, 15, 16):
        w += kw(p[0])
    elif code in (4, 12, 17, 18):
        pass
    elif code in (5, 10, 13):
        w += kw(p[0]) + kw(p[1])
    elif code == 7:
        w += kw(p[0]) + [p[1], p[2], p[3], p[4]]
    elif code == 9:
        w += kw(p[0]) + kw(p[1]) + [p[2]]
    else:
        raise ValueError(code)
    return w


STYLES = {
    "mixed": {1: 5, 2: 4, 3: 3, 4: 1, 5: 4, 6: 2, 7: 3, 8: 3, 9: 5, 10: 3, 11: 2, 12: 1, 13: 6, 14: 2, 15: 2, 16: 2, 17: 1, 18: 2},
    "sessions": {1: 12, 2: 4, 3: 5, 4: 2, 18: 2},
    "tools": {13: 14, 14: 3, 15: 3, 16: 3, 17: 2, 18: 2},
    "clocks": {5: 6, 6: 2, 7: 4, 8: 3, 9: 8, 10: 4, 11: 3, 12: 2, 18: 2},
    "churn": {1: 6, 3: 6, 5: 4, 6: 4, 9: 4, 10: 3, 13: 6, 14: 5, 18: 2},
}
BLOB_LENS = [0, 1, 2, 3, 5, 8, 20, 40]


def rand_word(rng):
    return rng.choice([0, 1, -1, 7, rng.randint(-1000, 1000), rng.randint(-(1 << 53), 1 << 53), (1 << 62), -(1 << 62)])


def gen_program(rng):
    L = rng.choice([64, 96, 128, 200, 256, 320, 384, 512])
    style = rng.choice(list(STYLES))
    if rng.random() < 0.4:
        big = [u16("k%d" % i) for i in range(rng.randint(20, 60))]
        pools = {n: big for n in ("s", "c", "t", "a", "r")}
        pools["r"] = [u16("r%d" % i) for i in range(rng.randint(3, 12))]
    else:
        pools = {n: rng.sample(BASE_KEYS, rng.randint(4, 10)) for n in ("s", "c", "t", "a", "r")}
    model = Model()
    lines = ["init %d" % L]
    exp = ["init ok"]
    weights = STYLES[style]
    codes = list(weights)
    wts = [weights[c] for c in codes]
    blob_max = rng.choice([3, 5, 6, 7])
    state = {"act": 0}

    def emit(code, p):
        w = encode(code, p)
        lines.append("op " + " ".join(str(x) for x in w))
        r = model.apply(code, p)
        if r is None:
            exp.append("stats %d %d" % (model.live_nonroot(), model.ghosts))
        else:
            exp.append("ok " + " ".join(str(x) for x in r))

    def emit_raw(w):
        lines.append("op " + " ".join(str(x) for x in w))
        exp.append("invalid")

    def random_op():
        code = rng.choices(codes, wts)[0]
        k = lambda n: rng.choice(pools[n])
        if code == 1:
            n = BLOB_LENS[rng.randint(0, blob_max)]
            emit(1, (k("s"), [rand_word(rng) for _ in range(n)]))
        elif code in (2, 3):
            emit(code, (k("s"),))
        elif code in (4, 12, 17, 18):
            emit(code, ())
        elif code == 5:
            emit(5, (k("c"), k("s")))
        elif code in (6, 8, 11):
            emit(code, (k("c"),))
        elif code == 7:
            emit(7, (k("c"), rng.randint(0, 1), rand_word(rng), rng.randint(0, 1), rand_word(rng)))
        elif code == 9:
            emit(9, (k("c"), k("r"), rng.randint(0, 1)))
        elif code == 10:
            emit(10, (k("c"), k("r")))
        elif code == 13:
            emit(13, (k("t"), k("a")))
        else:
            emit(code, (k("t"),))

    def forget_replay():
        # A turn with many activities is forgotten, then a new turn with the same id gets a few notes.
        t = rng.choice(pools["t"])
        for _ in range(rng.randint(8, 25)):
            state["act"] += 1
            emit(13, (t, u16("act%d" % state["act"])))
        emit(15, (t,))
        emit(14, (t,))
        emit(16, (t,))
        for _ in range(rng.randint(1, 4)):
            emit(13, (t, rng.choice(pools["a"])))
        emit(16, (t,))
        emit(17, ())

    n_ops = rng.randint(40, 140)
    replay_at = rng.randint(0, n_ops) if rng.random() < 0.35 else -1
    for i in range(n_ops):
        if i == replay_at:
            forget_replay()
        if rng.random() < 0.01:
            emit_raw(rng.choice([[99], [0], [19, 1, 97], [3, 5, 97]]))
        else:
            random_op()
    emit(18, ())
    lines.append("observe")
    ow = model.observe()
    exp.append("observe %d %s" % (len(ow), " ".join(str(x) for x in ow)) if ow else "observe 0")
    return {"lines": lines, "expected": exp, "init": L, "style": style}


def gen_programs(seed, count=300):
    rng = random.Random(seed)
    return [gen_program(random.Random(rng.getrandbits(64))) for _ in range(count)]


if __name__ == "__main__":
    import sys
    seed = int(sys.argv[1]) if len(sys.argv) > 1 else 1
    for pr in gen_programs(seed, 3):
        print("\n".join(pr["lines"]))
        print("--")
