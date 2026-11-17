#!/usr/bin/env python3
"""L3-10 M7 runner: budget lifecycle (bud_lifecycle.pruv) against the TS oracle.

  python3 bench/l3/budget/m7_test/run_m7.py q      [--pruvc PATH | --py] [--cases F] [--oracle F]
  python3 bench/l3/budget/m7_test/run_m7.py derive (writes work/m7/cases_life.jsonl + oracle_life.jsonl)
  python3 bench/l3/budget/m7_test/run_m7.py same   (Python pruvc vs Rust pruvc: byte-identical Rust)
  python3 bench/l3/budget/m7_test/run_m7.py perf   [--rounds N] [--reps N]

q: each Q1 case (work/m7/cases.jsonl) becomes a driver program; per request the driver prints the M7 effects in order
(uuid, changed, carryHeld, isQuiet, timerClear) and the result words; at the end the stored session records.
Compared with the oracle (work/m7/oracle.jsonl): per request the result value (refs, booleans, the budget of current
in canonical JV space) and the effect sequence (uuid / changed / carryHeld with its old id / isQuiet / timer.*;
now and unit are inputs, KD-1), then for whole cases every stored session's budget as a JV object (key order is not
compared, KD-M7-1). Instants the record holds as numbers are rendered with toISOString here (the boundary).
Classes:
  exact            whole case ran and everything matched
  not-implemented  the case reaches a call owned by another milestone (halt / decide: M10, hasCard: M11, charge: M8).
                   The requests before it are compared and must match; nothing after the cut is filled by host code.
  known-difference a difference listed in M7_NOTES.md (none expected; the class is kept for the table)
  fail             anything else
"""
import argparse, datetime, json, os, shutil, statistics, subprocess, sys

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
BUD = os.path.dirname(HERE)
ROOT = os.path.abspath(os.path.join(BUD, "..", "..", ".."))
WORK = os.path.join(BUD, "work", "m7")
PRUVC_DEFAULT = "/Users/spica4345/Projects/pruv/pruvc-rs/target/release/pruvc"
PY_DEFAULT = "/Users/spica4345/Projects/pruv/.venv/bin/python"
PRUVC_PY = "/Users/spica4345/Projects/pruv/pruvc/pruvc.py"
LIBS = ["--lib", "bench/l3/budget", "--lib", "bench/l3/lib"]
sys.path.insert(0, BUD)
from compare import jv_decode, jv_encode, UNDEF  # noqa: E402

STATE = {"open": 0, "halted": 1, "closed": 2}
STATE_N = {v: k for k, v in STATE.items()}
ORIGIN = {"user": 0, "schedule": 1, "direct": 2, "release": 3}
ORIGIN_N = {v: k for k, v in ORIGIN.items()}
HOW = {"quiet": 1, "user-end": 2, "user-message": 3}
HOW_N = {v: k for k, v in HOW.items()}
LIM = ["timeMinutes", "delegations", "commands", "reviews", "checks"]
USED = ["activeMs", "delegations", "commands", "reviews", "checks", "tools", "failures", "retries"]
CODE = {"current": 1, "isHalted": 2, "attach": 3, "replaceForUserMessage": 4, "reopen": 5, "maybeClose": 6}
OWNER = {"halt": "M10", "decide": "M10", "raise": "M10", "hasCard": "M11", "setCard": "M11", "charge": "M8"}
FX = ("uuid", "changed", "carryHeld", "isQuiet", "timer.set", "timer.clear", "timer.fire")


def rustc():
    return shutil.which("rustc") or os.path.expanduser("~/.cargo/bin/rustc")


def compile_gen(which, gen):
    root = "bench/l3/budget/m7_test/root.pruv"
    if which == "py":
        cmd = [PY_DEFAULT, PRUVC_PY, root, "-o", gen, "--backend", "rust"] + LIBS
    else:
        cmd = [which, root, "-o", gen] + LIBS
    c = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
    if c.returncode != 0 or not os.path.exists(gen):
        sys.exit("pruvc failed:\n" + c.stdout[-2000:] + c.stderr[-2000:])


def build(args):
    os.makedirs(WORK, exist_ok=True)
    gen = os.path.join(WORK, "gen.rs")
    compile_gen("py" if args.py else args.pruvc, gen)
    main = os.path.join(WORK, "main_m7.rs")
    with open(main, "w") as f:
        f.write('#![allow(dead_code, unused_imports, unused_variables, unused_mut)]\n#[path = "gen.rs"] mod sol;\n')
        f.write(open(os.path.join(HERE, "driver_m7.rs")).read())
    exe = os.path.join(WORK, "driver_m7")
    c = subprocess.run([rustc(), "-C", "opt-level=3", "--edition", "2021", "-o", exe, main], capture_output=True,
                       text=True)
    if c.returncode != 0:
        sys.exit("rustc failed:\n" + c.stderr[-4000:])
    return exe


# ---- values ----
def units(s):
    if isinstance(s, dict) and "u16" in s:
        return list(s["u16"])
    b = s.encode("utf-16-le", "surrogatepass")
    return [b[i] | (b[i + 1] << 8) for i in range(0, len(b), 2)]


def text(u):
    return b"".join(bytes([x & 0xff, x >> 8]) for x in u).decode("utf-16-le", "surrogatepass")


def key(s):
    u = units(s)
    return [len(u)] + u


def iso(ms):
    d = datetime.datetime(1970, 1, 1) + datetime.timedelta(milliseconds=ms)
    return d.strftime("%Y-%m-%dT%H:%M:%S.") + "%03dZ" % (ms % 1000)


def is_str(v):
    return isinstance(v, str)


# ---- record codec (bud_lifecycle.pruv header comment) ----
def encode_budget(b):
    """Budget (decoded python dict, or None/UNDEF) -> record words. Fields the record has no header slot for (or holds
    with a value the header cannot carry) go into the opaque rest text as canonical JV JSON."""
    if not isinstance(b, dict):
        return [-1]
    h = [0] * 24
    rest = {}
    strs = [[], [], [], [], [], []]
    for k, v in b.items():
        if k == "id" and is_str(v):
            strs[0] = units(v)
        elif k == "state" and v in STATE:
            h[0] = STATE[v]
        elif k == "origin" and v in ORIGIN:
            h[1] = ORIGIN[v]
        elif k == "unlimited":
            h[2] = 1 if v is True else 2
            if v is not True:
                rest[k] = v
        elif k == "limits" and isinstance(v, dict) and all(isinstance(v.get(n), int) for n in LIM) and len(v) == 5:
            h[3:8] = [v[n] for n in LIM]
        elif k == "used" and isinstance(v, dict) and all(isinstance(v.get(n), int) for n in USED) and len(v) == 8:
            h[8:16] = [v[n] for n in USED]
        elif k == "activeSince" and is_str(v):
            h[16] = 1
            strs[2] = units(v)
        elif k == "ended" and isinstance(v, dict) and v.get("how") in HOW and is_str(v.get("at")) and len(v) == 2:
            h[18] = HOW[v["how"]]
            h[19] = 1
            strs[3] = units(v["at"])
        elif k == "openedAt" and is_str(v):
            h[20] = 1
            strs[1] = units(v)
        elif k == "follows":
            h[23] = 1 if is_str(v) else 2
            if is_str(v):
                strs[4] = units(v)
            else:
                rest[k] = v
        else:
            rest[k] = v
    for k in ("id", "state", "origin", "limits", "used", "openedAt"):
        if k not in b or (k in ("state", "origin") and b[k] not in (STATE if k == "state" else ORIGIN)):
            raise ValueError("budget field %s not representable: %r" % (k, b.get(k)))
    strs[5] = units(json.dumps(jv_encode(rest), ensure_ascii=False, separators=(",", ":"))) if rest else []
    w = list(h)
    for s in strs:
        w += [len(s)] + s
    return w


def decode_budget(w):
    if w[0] == -1:
        return None
    h = w[:24]
    p = 24
    strs = []
    for _ in range(6):
        n = w[p]
        strs.append(w[p + 1:p + 1 + n])
        p += 1 + n
    rest = jv_decode(json.loads(text(strs[5]))) if strs[5] else {}
    b = {"id": text(strs[0]), "state": STATE_N.get(h[0], h[0]), "origin": ORIGIN_N.get(h[1], h[1])}
    b["openedAt"] = text(strs[1]) if h[20] == 1 else iso(h[21])
    b["limits"] = dict(zip(LIM, h[3:8]))
    b["used"] = dict(zip(USED, h[8:16]))
    if h[2] == 1:
        b["unlimited"] = True
    if h[16] == 1:
        b["activeSince"] = text(strs[2])
    elif h[16] == 2:
        b["activeSince"] = iso(h[17])
    if h[18]:
        b["ended"] = {"how": HOW_N[h[18]], "at": text(strs[3]) if h[19] == 1 else iso(h[22])}
    if h[23] == 1:
        b["follows"] = text(strs[4])
    b.update(rest)
    return b


def put_words(sess):
    d = jv_decode(sess)
    rec = encode_budget(d.get("budget"))
    return key(d["id"]) + [len(rec)] + rec


# ---- driver program ----
def call_body(op, args, unit):
    """[code, origin, size, unit x6, sessionId key, budgetId key]."""
    a = [jv_decode(x) if not (isinstance(x, dict) and "sess" in x) else x for x in args]
    origin, size, sid, bid = -1, -1, None, ""
    if op in ("current", "isHalted", "reopen"):
        sid, bid = a[0]["sessionId"], a[0]["budgetId"]
    else:
        sid = a[0]["sess"]
        if op == "attach":
            o = a[1] if len(a) > 1 else UNDEF
            origin = ORIGIN.get(o, 9) if is_str(o) else 9
            z = a[2] if len(a) > 2 else UNDEF
        elif op == "replaceForUserMessage":
            z = a[1] if len(a) > 1 else UNDEF
            o = a[2] if len(a) > 2 else UNDEF
            origin = -1 if o is UNDEF else (ORIGIN.get(o, 9) if is_str(o) else 9)
        if op in ("attach", "replaceForUserMessage"):
            size = -1 if z is UNDEF else 0 if z == "unlimited" else z if z in (1, 2, 4, 8) else 9
    u = [unit.get(n, 0) for n in LIM] + [1 if unit.get("defaultUnlimited") is True else 0]
    return [CODE[op], origin, size] + u + key(sid) + key(bid)


def default_uuid(n):
    return "00000000-0000-4000-8000-%012x" % n


def program(case):
    """Returns (lines, cut) where cut is None or (req, op) of the first call owned by another milestone."""
    w = case["world"]
    out = ["case " + case["id"], "now %d" % w.get("clockStart", 1700000000000)]
    us = w.get("uuids") or [default_uuid(i) for i in range(1, 200)]
    out.append("uuids " + " ".join(str(x) for u in us for x in key(u)))
    q = (w.get("deps") or {}).get("isQuiet") or []
    out.append("quiet " + " ".join("1" if x else "0" for x in q))
    for s in w["sessions"]:
        out.append("put " + " ".join(map(str, put_words(s))))
    unit = jv_decode(w["unit"])
    cut = None
    for e in case["script"]:
        ev = e["ev"]
        if ev == "advance":
            out.append("adv %d" % e["ms"])
        elif ev == "setUnit":
            unit = jv_decode(e["unit"])
        elif ev == "putSession":
            out.append("put " + " ".join(map(str, put_words(e["session"]))))
        elif ev == "removeSession":
            out.append("del " + " ".join(map(str, key(e["id"]))))
        elif ev == "call":
            if e["op"] not in CODE:
                cut = (e["req"], e["op"])
                break
            out.append("call %s %s" % (e["req"], " ".join(map(str, call_body(e["op"], e["args"], unit)))))
        else:
            raise ValueError("unknown event " + ev)
    out.append("end")
    return out, cut


def parse_driver(t):
    res = {}
    cur = None
    for line in t.splitlines():
        if line.startswith("case "):
            cur = {"reqs": {}, "order": [], "obs": None, "x": []}
            res[line[5:]] = cur
            r = None
        elif line == "end":
            cur = None
        elif line.startswith("O"):
            cur["obs"] = line
        elif line.startswith("@ "):
            r = line[2:]
            cur["order"].append(r)
            cur["reqs"][r] = {"fx": [], "v": None}
        elif line.startswith("X"):
            cur["x"].append((r, line))
        elif line.startswith("E "):
            cur["reqs"][r]["fx"].append(line[2:].split())
        elif line.startswith("V "):
            cur["reqs"][r]["v"] = list(map(int, line[2:].split()))
    return res


def parse_obs(line):
    w = list(map(int, line[1:].split()))
    p = 0
    sessions = {}
    n = w[p]
    p += 1
    for _ in range(n):
        k = w[p + 1:p + 1 + w[p]]
        p += 1 + w[p]
        b = w[p + 1:p + 1 + w[p]]
        p += 1 + w[p]
        sessions[text(k)] = decode_budget(b)
    return sessions


# ---- compare ----
def oracle_fx(o, req):
    ev = []
    on = False
    for t in o["trace"]:
        if t.get("r") != req:
            continue
        c = t["c"]
        if c == "call.start":
            on = True
        elif c == "call.end":
            break
        elif on and c in FX:
            if c == "carryHeld":
                a = t["a"]
                sess = jv_decode(a[0]["v"]) if isinstance(a[0], dict) and "v" in a[0] else jv_decode(a[0])
                ev.append(("carryHeld", sess["id"], jv_decode(a[1]), jv_decode(a[2])["budgetId"]))
            elif c == "isQuiet":
                ev.append(("isQuiet", 1 if jv_decode(t.get("o")) else 0))
            elif c.startswith("timer."):
                ev.append((c,))
            else:
                ev.append((c,))
    return ev


def unwrap(v):
    """Oracle request value: budgets come as {bref, v}."""
    if isinstance(v, dict) and "bref" in v and "v" in v:
        return jv_decode(v["v"])
    if isinstance(v, dict) and "obj" in v:
        d = jv_decode(v)
        if isinstance(d, dict) and "bref" in d:
            x = d["v"]
            return jv_decode(x) if isinstance(x, dict) and "obj" in x else x
        return d
    return jv_decode(v)


def mine_value(op, sid, words):
    tag = words[0]
    if tag == 0:
        return UNDEF, None
    if tag in (1, 2):
        return tag == 2, None
    if tag == 3:
        n = words[1]
        ref = {"sessionId": sid, "budgetId": text(words[2:2 + n])}
        old = None
        if len(words) > 2 + n:
            m = words[2 + n]
            old = text(words[3 + n:3 + n + m])
        return ref, old
    if tag == 4:
        return decode_budget(words[2:2 + words[1]]), None
    return ("tag", tag), None


def same(a, b):
    return json.dumps(jv_encode(a), sort_keys=True) == json.dumps(jv_encode(b), sort_keys=True)


def norm_obj(x):
    if isinstance(x, dict):
        return {k: norm_obj(v) for k, v in sorted(x.items())}
    if isinstance(x, list):
        return [norm_obj(v) for v in x]
    return x


def classify(case, o, run, cut):
    if run["x"]:
        return "fail", "driver %r" % (run["x"][0],)
    req_o = {r["id"]: r for r in o["requests"]}
    calls = {e["req"]: e for e in case["script"] if e["ev"] == "call"}
    for r in run["order"]:
        mine = run["reqs"][r]
        e = calls[r]
        a0 = e["args"][0]
        sid = a0["sess"] if isinstance(a0, dict) and "sess" in a0 else jv_decode(a0)["sessionId"]
        v, old = mine_value(e["op"], sid, mine["v"])
        want = req_o[r]
        if want["terminal"] != "ok":
            return "fail", "%s terminal %s" % (r, want["terminal"])
        wv = unwrap(want["value"])
        if e["op"] == "current" and isinstance(v, dict) and isinstance(wv, dict):
            if jv_encode(norm_obj(v)) != jv_encode(norm_obj(wv)):
                return "fail", "%s value %r != %r" % (r, v, wv)
        elif not same(v, wv):
            return "fail", "%s value %r != %r" % (r, v, wv)
        got = []
        for f in mine["fx"]:
            if f[0] == "carry":
                got.append(("carryHeld", sid, old, v["budgetId"]))
            elif f[0] == "quiet":
                got.append(("isQuiet", int(f[1])))
            elif f[0] == "clear":
                got.append(("timer.clear",))
            else:
                got.append((f[0],))
        wfx = oracle_fx(o, r)
        if got != wfx:
            return "fail", "%s effects %r != %r" % (r, got, wfx)
    if cut:
        return "not-implemented", "%s (%s)" % (cut[1], OWNER.get(cut[1], "?"))
    sessions = parse_obs(run["obs"])
    for s in o["finalSessions"]:
        d = jv_decode(s["v"])
        b = d.get("budget")
        if isinstance(b, dict) and "bref" in b:
            b = b["v"]
            if isinstance(b, dict) and "obj" in b:
                b = jv_decode(b)
        if d["id"] not in sessions:
            return "fail", "session %r missing" % d["id"]
        mine = sessions[d["id"]]
        if not isinstance(b, dict):
            if mine is not None:
                return "fail", "budget presence %r" % d["id"]
            continue
        if jv_encode(norm_obj(mine)) != jv_encode(norm_obj(b)):
            return "fail", "final %r: %r != %r" % (d["id"], mine, b)
    if len(sessions) != len(o["finalSessions"]):
        return "fail", "session count %d != %d" % (len(sessions), len(o["finalSessions"]))
    return "exact", ""


def cmd_q(args):
    exe = build(args)
    cases = [json.loads(l) for l in open(args.cases)]
    oracle = {}
    for l in open(args.oracle):
        o = json.loads(l)
        oracle[o["id"]] = o
    prog, cuts = [], {}
    for c in cases:
        lines, cut = program(c)
        prog += lines
        cuts[c["id"]] = cut
    r = subprocess.run([exe, "q"], input="\n".join(prog) + "\n", capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit("driver failed:\n" + r.stderr[-3000:])
    runs = parse_driver(r.stdout)
    count = {"exact": 0, "known-difference": 0, "not-implemented": 0, "fail": 0}
    why, ops, fails = {}, {}, []
    for c in cases:
        run = runs[c["id"]]
        for rq in run["order"]:
            op = [e["op"] for e in c["script"] if e["ev"] == "call" and e["req"] == rq][0]
            ops[op] = ops.get(op, 0) + 1
        cls, note = classify(c, oracle[c["id"]], run, cuts[c["id"]])
        count[cls] += 1
        if cls in ("known-difference", "not-implemented"):
            why[note] = why.get(note, 0) + 1
        if cls == "fail":
            fails.append((c["id"], note))
    print(json.dumps({"cases": len(cases), "count": count, "reasons": why, "compared_requests": ops},
                     ensure_ascii=False))
    for i, n in fails[:12]:
        print("FAIL", i, n[:700])
    return 1 if fails else 0


def cmd_derive(args):
    """Lifecycle-only variant of each Q1 case: the same world and script without the calls owned by other
    milestones (ids get a '-life' suffix), run through the TS oracle so every remaining call is an M7 path."""
    from oracle import run_oracle
    cases = []
    for l in open(args.cases):
        c = json.loads(l)
        c["id"] += "-life"
        c["script"] = [e for e in c["script"] if not (e["ev"] == "call" and e["op"] not in CODE)]
        cases.append(c)
    obs = run_oracle(cases)
    with open(os.path.join(WORK, "cases_life.jsonl"), "w") as f:
        for c in cases:
            f.write(json.dumps(c, ensure_ascii=False) + "\n")
    with open(os.path.join(WORK, "oracle_life.jsonl"), "w") as f:
        for o in obs:
            f.write(json.dumps(o, ensure_ascii=False) + "\n")
    print("derived %d cases" % len(cases))
    return 0


def cmd_same(args):
    a, b = os.path.join(WORK, "gen_rs.rs"), os.path.join(WORK, "gen_py.rs")
    compile_gen(args.pruvc, a)
    compile_gen("py", b)
    x, y = open(a, "rb").read(), open(b, "rb").read()
    print(json.dumps({"rs_bytes": len(x), "py_bytes": len(y), "identical": x == y}))
    return 0 if x == y else 1


def cmd_perf(args):
    """Alternates Pruv / hand Rust / TS runs per round and size; prints the median ns per call of each item."""
    from common import NODE, TS_ENV
    exe = build(args)
    hand = os.path.join(WORK, "hand_m7")
    c = subprocess.run([rustc(), "-C", "opt-level=3", "--edition", "2021", "-o", hand, os.path.join(HERE, "hand_m7.rs")],
                       capture_output=True, text=True)
    if c.returncode != 0:
        sys.exit("rustc failed (hand):\n" + c.stderr[-3000:])
    env = dict(os.environ)
    env.update(TS_ENV())
    ts = [NODE, "--experimental-strip-types", "--no-warnings", "--import", os.path.join(BUD, "reference-hooks.mjs"),
          os.path.join(HERE, "perf_ts.mjs")]
    sides = {"pruv": [exe, "perf"], "hand": [hand], "ts": ts}
    res = {}
    for rd in range(args.rounds):
        for size in (100, 500, 1000):
            for side, cmd in sides.items():
                r = subprocess.run(cmd + [str(size), str(args.reps)], capture_output=True, text=True, env=env,
                                   cwd=HERE)
                if r.returncode != 0:
                    sys.exit("%s perf failed:\n%s" % (side, r.stderr[-2000:]))
                for l in r.stdout.splitlines():
                    item, sz, ns = l.split()
                    res.setdefault((item, int(sz)), {}).setdefault(side, []).append(float(ns))
    print("item | S | Pruv ns | hand Rust ns | TS ns | Pruv/hand")
    for (item, sz), v in sorted(res.items()):
        m = {k: statistics.median(x) for k, x in v.items()}
        f = lambda k: ("%.1f" % m[k]) if k in m else "-"
        ratio = ("%.2f" % (m["pruv"] / m["hand"])) if "hand" in m else "-"
        print("%s | %d | %s | %s | %s | %s" % (item, sz, f("pruv"), f("hand"), f("ts"), ratio))
    return 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", choices=["q", "perf", "derive", "same"])
    ap.add_argument("--pruvc", default=PRUVC_DEFAULT)
    ap.add_argument("--py", action="store_true")
    ap.add_argument("--cases", default=os.path.join(WORK, "cases.jsonl"))
    ap.add_argument("--oracle", default=os.path.join(WORK, "oracle.jsonl"))
    ap.add_argument("--rounds", type=int, default=5)
    ap.add_argument("--reps", type=int, default=200000)
    a = ap.parse_args()
    sys.exit({"q": cmd_q, "perf": cmd_perf, "derive": cmd_derive, "same": cmd_same}[a.cmd](a))


if __name__ == "__main__":
    main()
