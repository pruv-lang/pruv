#!/usr/bin/env python3
"""L3-10 M7 runner: budget lifecycle (bud_lifecycle.pruv) against the TS oracle.

  python3 bench/l3/budget/m7_test/run_m7.py q      [--pruvc PATH | --py] [--cases F] [--oracle F] [--hand16]
  python3 bench/l3/budget/m7_test/run_m7.py derive (writes work/m7/cases_life.jsonl + oracle_life.jsonl)
  python3 bench/l3/budget/m7_test/run_m7.py same   (Python pruvc vs Rust pruvc: byte-identical Rust)
  python3 bench/l3/budget/m7_test/run_m7.py perf   [--rounds N] [--reps N] [--profile S|M|SM | --sizes 100,500,1000] [--no-instr]

q --hand16 runs hand_m7_u16.rs (typed UTF-16 hand Rust, q mode) on the same program instead of the Pruv driver, through the
same classification. perf lanes: pruv (driver_m7), hand16 (hand_m7_u16), hand (hand_m7, UTF-8 String, informational), ts.
perf also writes work/m7/perf.json with run_meta.run_meta(...) and the rows (ns, retired instructions, ratios).
perf compares per cell the per-op result checksums rchk, rchk_t (pruv) and rchk, rchk_w (hand16) (untimed passes); a
mismatch or missing rchk fails the cell (no ratio) and the exit status.
If work/m7/cases.jsonl / oracle.jsonl are missing the read-only source SRC_WORK is read instead.

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
SRC_WORK = "/Users/spica4345/Projects/pruv/bench/l3/budget/work/m7"  # read-only source of cases / oracle
sys.path.insert(0, BUD)
from compare import jv_decode, jv_encode, UNDEF  # noqa: E402
from run_meta import run_meta, num, PROFILES, RUSTC_FLAGS, SEED  # noqa: E402
from q_gate import failures as gate_failures, required_exact  # noqa: E402


def input_path(name):
    """work/m7/<name> when present, else the read-only source copy."""
    p = os.path.join(WORK, name)
    return p if os.path.exists(p) else os.path.join(SRC_WORK, name)

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


def build_facts():
    os.makedirs(WORK, exist_ok=True)
    rlib = os.path.join(WORK, "libbud_facts.rlib")
    src = os.path.join(HERE, "bud_facts.rs")
    c = subprocess.run([rustc(), "-O", "--edition", "2021", "--crate-type", "rlib",
                        "--crate-name", "bud_facts", src, "-o", rlib], capture_output=True, text=True)
    if c.returncode != 0:
        sys.exit("rustc failed (bud_facts):\n" + c.stderr)
    return rlib


def compile_gen(which, gen, rlib=None):
    if rlib is None:
        rlib = build_facts()
    root = "bench/l3/budget/m7_test/root.pruv"
    if which == "py":
        cmd = [PY_DEFAULT, PRUVC_PY, root, "-o", gen, "--backend", "rust"] + LIBS
    else:
        cmd = [which, root, "-o", gen] + LIBS
    c = subprocess.run(cmd + ["--rust-extern", f"bud_facts={rlib}"], cwd=ROOT, capture_output=True, text=True)
    if c.returncode != 0 or not os.path.exists(gen):
        sys.exit("pruvc failed:\n" + c.stdout + c.stderr)


def build(args):
    rlib = build_facts()
    gen = os.path.join(WORK, "gen.rs")
    compile_gen("py" if args.py else args.pruvc, gen, rlib)
    main = os.path.join(WORK, "main_m7.rs")
    with open(main, "w") as f:
        f.write('#![allow(dead_code, unused_imports, unused_variables, unused_mut)]\n#[path = "gen.rs"] mod sol;\n')
        f.write(open(os.path.join(HERE, "driver_m7.rs")).read())
    exe = os.path.join(WORK, "driver_m7")
    c = subprocess.run([rustc()] + RUSTC_FLAGS + ["--extern", f"bud_facts={rlib}", "-o", exe, main], capture_output=True, text=True)
    if c.returncode != 0:
        sys.exit("rustc failed:\n" + c.stderr[-4000:])
    return exe


def build_hand(src, name):
    """Compile a hand Rust file of this folder to work/m7/<name> with the shared rustc flags."""
    rlib = build_facts()
    exe = os.path.join(WORK, name)
    c = subprocess.run([rustc()] + RUSTC_FLAGS + ["--extern", f"bud_facts={rlib}",
                                            "-o", exe, os.path.join(HERE, src)], capture_output=True, text=True)
    if c.returncode != 0:
        sys.exit("rustc failed (%s):\n%s" % (src, c.stderr[-4000:]))
    return exe


def build_hand16():
    return build_hand("hand_m7_u16.rs", "hand_m7_u16")


# ---- values ----
def units(s):
    if isinstance(s, dict) and "u16" in s:
        return list(s["u16"])
    b = s.encode("utf-16-le", "surrogatepass")
    return [b[i] | (b[i + 1] << 8) for i in range(0, len(b), 2)]


def text(u):
    return b"".join(bytes([x & 0xff, x >> 8]) for x in u).decode("utf-16-le", "surrogatepass")


def pack(u):
    """Packed-u16 words (bud_keys.pruv): word i = u[4i]<<48 | u[4i+1]<<32 | u[4i+2]<<16 | u[4i+3], as a two's-complement
    i64 (u[4i] >= 0x8000 gives a negative word); lanes past len(u) are zero."""
    out = []
    for i in range(0, len(u), 4):
        x = 0
        for l in range(4):
            x = (x << 16) | (u[i + l] if i + l < len(u) else 0)
        out.append(x - (1 << 64) if x >= (1 << 63) else x)
    return out


def unpack(words, n):
    """Inverse of pack: the n u16 units of packed words."""
    return [(words[i // 4] >> (48 - 16 * (i % 4))) & 0xFFFF for i in range(n)]


def nwords(n):
    """Data words of a packed key of n units: ceil(n / 4)."""
    return (max(n, 0) + 3) // 4


def key(s):
    """Wire key, packed once here at ingress: `n, words(n) packed words`."""
    u = units(s)
    return [len(u)] + pack(u)


def iso(ms):
    d = datetime.datetime(1970, 1, 1) + datetime.timedelta(milliseconds=ms)
    return d.strftime("%Y-%m-%dT%H:%M:%S.") + "%03dZ" % (ms % 1000)


def is_str(v):
    return isinstance(v, str)


# ---- record codec (bud_lifecycle.pruv header comment) ----
PACKED_STRS = (0, 4)  # record strings stored as framed packed keys: 0 id, 4 follows


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
    for i, s in enumerate(strs):
        w += [len(s)] + (pack(s) if i in PACKED_STRS else s)
    return w


def decode_strs(w):
    """The six record strings as unit lists (strings 0 and 4 unpacked from their packed words) and the end position."""
    p = 24
    strs = []
    for i in range(6):
        n = w[p]
        if i in PACKED_STRS:
            strs.append(unpack(w[p + 1:p + 1 + nwords(n)], n))
            p += 1 + nwords(n)
        else:
            strs.append(w[p + 1:p + 1 + n])
            p += 1 + n
    return strs, p


def decode_budget(w):
    if w[0] == -1:
        return None
    h = w[:24]
    strs, _ = decode_strs(w)
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
    """[code, origin, size, unit x6, sessionId key, budgetId key] (keys packed)."""
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
    expected = []
    for e in case["script"]:
        if e["ev"] != "call":
            continue
        if cut and e["req"] == cut[0]:
            break
        expected.append(e["req"])
    if run["order"] != expected:
        return "fail", "requests ran %r != expected %r" % (run["order"], expected)
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
                got.append(("isQuiet", int(f[1]), tuple(map(int, f[2:-1])), f[-1]))
            elif f[0] == "clear":
                got.append(("timer.clear",))
            else:
                got.append((f[0],))
        wfx = [f + (tuple(key(sid)), "@0") if f[0] == "isQuiet" else f for f in oracle_fx(o, r)]
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
    # --hand16: the typed UTF-16 hand Rust (hand_m7_u16.rs q mode) instead of the Pruv driver; same protocol, same
    # classification (the hand is judged against the oracle exactly like the Pruv root).
    required = required_exact(HERE, args.cases)
    exe = build_hand16() if args.hand16 else build(args)
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
    exact = set()
    for c in cases:
        run = runs[c["id"]]
        for rq in run["order"]:
            op = [e["op"] for e in c["script"] if e["ev"] == "call" and e["req"] == rq]
            op = op[0] if op else "<unexpected>"
            ops[op] = ops.get(op, 0) + 1
        cls, note = classify(c, oracle[c["id"]], run, cuts[c["id"]])
        count[cls] += 1
        if cls == "exact":
            exact.add(c["id"])
        if cls in ("known-difference", "not-implemented"):
            why[note] = why.get(note, 0) + 1
        if cls == "fail":
            fails.append((c["id"], note))
    print(json.dumps({"cases": len(cases), "count": count, "reasons": why, "compared_requests": ops},
                     ensure_ascii=False))
    fails.extend(gate_failures(count, exact, required))
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


PERF_ITEMS = ["ishalted", "current", "attach", "close", "closereopen", "reopenmiss", "replace"]  # items the Pruv driver and hand16 both run
PERF_CALLS = {"closereopen": 2}  # calls per rep (ns and instructions are reported per call)
# close: whole-process instruction counts would include the per-batch restore of the closed state, which differs by side
# (Pruv copies st, hand16 rebuilds its records) and is outside the timed CLOSE span; close is measured by time,
# allocations and peak RSS only.
NO_INSTR_ITEMS = {"close"}


def perf_sizes(args):
    """--profile (S = 100, M = 500, SM = both; run_meta.PROFILES) wins over --sizes; default 100,500,1000."""
    if args.profile:
        return list(PROFILES[args.profile])
    return [int(x) for x in (args.sizes or "100,500,1000").split(",")]


def instr_per_call(make_cmd, reps, calls):
    """Retired instructions per call of one lane: (instr(5 x reps) - instr(reps)) / (4 x reps x calls), median of 3.
    The setup, warm-up and the final observation are the same in both runs and cancel. None when unavailable."""
    r1, r2 = reps, 5 * reps
    vals = []
    for _ in range(3):
        lo, hi = instr_retired(make_cmd(r1), soft=True), instr_retired(make_cmd(r2), soft=True)
        if lo is None or hi is None:
            return None
        vals.append((hi - lo) / (r2 - r1) / calls)
    return statistics.median(vals)


def cmd_perf(args):
    """Rotates Pruv / hand16 / hand / TS per round and size; prints the median ns per call of each item, retired
    instructions per call (Pruv, hand16) and Pruv/hand16 (the bar). Writes work/m7/perf.json with run_meta.

    The hand16 lane prints `chk item size value` (final observed state folded); chk only has to be identical across
    rounds (state equality Pruv vs hand16 is checked by diff_hand_m7.py through the q-mode observation).
    Both pruv and hand16 print `rchk item size value`: a checksum of every op's status, effect words and result words
    from an untimed verification pass (fresh identical setup, min(reps, 2000) iterations). Per cell the two must be
    equal; a missing or different value (or one that varies across rounds) marks the cell `RCHK MISSING` /
    `RCHK MISMATCH`, drops its ratios, is recorded in perf.json and makes the command exit non-zero. dispatch and
    rawcall (pruv only) print rchk too but are not compared. Last line: `result checks: N cells compared, M mismatches`.

    Matched lanes (same boundary on both sides): driver_m7 `perf` additionally prints `t <item> <size> <ns>` and
    `rchk_t <item> <size> <value>` (Pruv typed lane: sol::c_* entries, no wire framing, no valid) and hand_m7_u16 `perf`
    prints `w <item> <size> <ns>` and `rchk_w <item> <size> <value>` (hand16 wire lane: Host op build + decode_call +
    dispatch, no logging). They form the extra lanes pruv_t and hand16w. Columns: `Pruv/hand16w` = Pruv wire vs
    hand16 wire (matched), `Pruv typed/hand16` = Pruv typed vs hand16 typed (matched); the old `Pruv/hand16` (Pruv wire
    vs hand16 typed) stays but is UNMATCHED. Per cell rchk, rchk_t (pruv) and rchk, rchk_w (hand16) must be one
    identical value in every round; a missing one (including rchk_t) is RCHK MISSING.

    Instructions: driver_m7 `perf` runs every item in one process (no item argument), so its per-item instructions are
    taken from `driver_m7 boundary S reps item A` (the same World::call path and bodies as perf) minus the same item's
    `observe` row (boundary variants additionally fold the result words into a checksum; observe is that fold alone).
    Pruv typed instr = boundary variant C minus observe. hand16 typed instr uses `hand_m7_u16 perf S reps item typed`,
    hand16w instr `... item wire`. All: (instr(5 reps) - instr(reps)) / (4 reps x calls per rep). Instr ratios:
    Pruv/hand16w (wire), Pruv typed/hand16 (typed), Pruv/hand16 (unmatched).
    No retired-instruction counter (no /usr/bin/time -l line) -> 'unavailable', never 0."""
    from common import NODE, TS_ENV
    exe = build(args)
    hand16 = build_hand16()
    hand = build_hand("hand_m7.rs", "hand_m7")
    env = dict(os.environ)
    env.update(TS_ENV())
    ts = [NODE, "--experimental-strip-types", "--no-warnings", "--import", os.path.join(BUD, "reference-hooks.mjs"),
          os.path.join(HERE, "perf_ts.mjs")]
    sides = [("pruv", [exe, "perf"]), ("hand16", [hand16, "perf"]), ("hand", [hand]), ("ts", ts)]
    sizes = perf_sizes(args)
    res, chk, rchk = {}, {}, {}
    for rd in range(args.rounds):
        for size in (sizes if rd % 2 == 0 else sizes[::-1]):
            k = rd % len(sides)  # rotate the lane order every round
            for side, cmd in sides[k:] + sides[:k]:
                r = subprocess.run(cmd + [str(size), str(args.reps)], capture_output=True, text=True, env=env,
                                   cwd=HERE)
                if r.returncode != 0:
                    sys.exit("%s perf failed:\n%s" % (side, r.stderr[-2000:]))
                for l in r.stdout.splitlines():
                    f = l.split()
                    if f[0] == "chk":  # chk <item> <size> <value> (hand16 only)
                        chk.setdefault((f[1], int(f[2])), set()).add(f[3])
                        continue
                    if f[0] in ("rchk", "rchk_t", "rchk_w"):
                        # rchk <item> <size> <value> (pruv and hand16 typed lane; per-op results, untimed pass);
                        # rchk_t (pruv typed lane) and rchk_w (hand16 wire lane) go to their own lane key
                        # (pruv_t / hand16w); kept per run (one list entry per process run), so a value missing from
                        # one round is seen
                        lk = {"rchk": side, "rchk_t": "pruv_t", "rchk_w": "hand16w"}[f[0]]
                        rchk.setdefault((f[1], int(f[2])), {}).setdefault(lk, {}).setdefault(rd, []).append(f[3])
                        continue
                    if f[0] in ("t", "w"):  # t <item> <size> <ns> (pruv typed lane), w <item> <size> <ns> (hand16 wire)
                        res.setdefault((f[1], int(f[2])), {}).setdefault("pruv_t" if f[0] == "t" else "hand16w", []).append(
                            float(f[3]))
                        continue
                    item, sz, ns = f
                    res.setdefault((item, int(sz)), {}).setdefault(side, []).append(float(ns))
    bad = ["chk %s %d varies across rounds: %s" % (i, s, sorted(v)) for (i, s), v in sorted(chk.items()) if len(v) != 1]
    # Per-cell result check: Pruv vs hand16 rchk of every item both lanes run. Each lane must print exactly one value in
    # EVERY round; a value missing from any round, a value that varies across rounds, or a different value is a FAIL
    # cell (no ratio is reported for it).
    cell_status, n_cmp, n_bad = {}, 0, 0
    for (item, sz) in sorted(res):
        if item not in PERF_ITEMS:
            continue
        n_cmp += 1
        r = rchk.get((item, sz), {})
        # four lanes must each print exactly one value in every round, all identical: pruv rchk, pruv typed rchk_t,
        # hand16 rchk, hand16 wire rchk_w
        lanes = [r.get(k, {}) for k in ("pruv", "pruv_t", "hand16", "hand16w")]
        if any(len(ln.get(rd, [])) != 1 for ln in lanes for rd in range(args.rounds)):
            cell_status[(item, sz)] = "RCHK MISSING"
        elif len({v for ln in lanes for vs in ln.values() for v in vs}) != 1:
            cell_status[(item, sz)] = "RCHK MISMATCH"
        if (item, sz) in cell_status:
            n_bad += 1
    instr = {}
    if not args.no_instr:
        for size in sizes:
            for item in PERF_ITEMS:
                if item in NO_INSTR_ITEMS:
                    continue
                calls = PERF_CALLS.get(item, 1)
                a = instr_per_call(lambda r: [exe, "boundary", str(size), str(r), item, "A"], args.reps, calls)
                o = instr_per_call(lambda r: [exe, "boundary", str(size), str(r), item, "observe"], args.reps, calls)
                instr[("pruv_A", item, size)] = a
                instr[("pruv_observe", item, size)] = o
                instr[("pruv", item, size)] = None if a is None or o is None else a - o
                # typed lane of Pruv: boundary variant C (sol::c_* typed entries) minus observe
                c = instr_per_call(lambda r: [exe, "boundary", str(size), str(r), item, "C"], args.reps, calls)
                instr[("pruv_C", item, size)] = c
                instr[("pruv_t", item, size)] = None if c is None or o is None else c - o
                instr[("hand16", item, size)] = instr_per_call(
                    lambda r: [hand16, "perf", str(size), str(r), item, "typed"], args.reps, calls)
                instr[("hand16w", item, size)] = instr_per_call(
                    lambda r: [hand16, "perf", str(size), str(r), item, "wire"], args.reps, calls)
    fi = lambda x: "unavailable" if x is None else "%.0f" % x
    print("rounds %d, reps %d, sizes %s; ns and instructions per call" % (args.rounds, args.reps, sizes))
    print("Matched pairs: Pruv/hand16w (wire vs wire), Pruv typed/hand16 (typed vs typed). Pruv/hand16 is UNMATCHED "
          "(Pruv wire vs hand16 typed).")
    print("item | S | Pruv ns | hand16 ns | hand(old UTF-8) ns | TS ns | Pruv typed ns | hand16w ns"
          " | Pruv/hand16 (unmatched: wire vs typed) | Pruv/hand16w (wire) | Pruv typed/hand16 (typed) | Pruv/hand(old)"
          " | Pruv instr | hand16 instr | instr Pruv/hand16 (unmatched) | Pruv typed instr | hand16w instr"
          " | instr Pruv/hand16w | instr Pruv typed/hand16")
    rows = []
    for (item, sz), v in sorted(res.items(), key=lambda kv: (kv[0][1], kv[0][0])):
        m = {k: statistics.median(x) for k, x in v.items()}
        f = lambda k: ("%.1f" % m[k]) if k in m else "-"
        rt = lambda a, b: (m[a] / m[b]) if a in m and b in m and m[b] else None
        ip, ih = instr.get(("pruv", item, sz)), instr.get(("hand16", item, sz))
        ipt, ihw = instr.get(("pruv_t", item, sz)), instr.get(("hand16w", item, sz))
        ir = (ip / ih) if ip is not None and ih else None
        irw = (ip / ihw) if ip is not None and ihw else None  # wire vs wire
        irt = (ipt / ih) if ipt is not None and ih else None  # typed vs typed
        fr = lambda x: "-" if x is None else "%.2f" % x
        fail = cell_status.get((item, sz))
        if fail:  # no ratio of a cell whose per-op results are unchecked or differ
            rt = lambda a, b: None
            ir = irw = irt = None
        ci = item in PERF_ITEMS and not args.no_instr
        print("%s | %d | %s | %s | %s | %s | %s | %s | %s | %s | %s | %s | %s | %s | %s | %s | %s | %s | %s%s" % (
            item, sz, f("pruv"), f("hand16"), f("hand"), f("ts"), f("pruv_t"), f("hand16w"),
            fr(rt("pruv", "hand16")), fr(rt("pruv", "hand16w")), fr(rt("pruv_t", "hand16")), fr(rt("pruv", "hand")),
            fi(ip) if ci else "-", fi(ih) if ci else "-", fr(ir),
            fi(ipt) if ci else "-", fi(ihw) if ci else "-", fr(irw), fr(irt),
            " | FAIL " + fail if fail else ""))
        rc = rchk.get((item, sz), {})
        rows.append({
            "item": item, "size": sz,
            "ns": {k: m[k] for k in m}, "raw_ns": v,
            "rchk": {s_: sorted(x) for s_, x in rc.items()} or "unavailable",
            "rchk_status": fail or ("ok" if item in PERF_ITEMS else "not compared (pruv only)"),
            "pruv_over_hand16": num(rt("pruv", "hand16")), "pruv_over_hand_old": num(rt("pruv", "hand")),
            "pruv_over_hand16w": num(rt("pruv", "hand16w")), "pruv_typed_over_hand16": num(rt("pruv_t", "hand16")),
            "pruv_over_hand16_matched": False,  # Pruv wire vs hand16 typed: unmatched boundaries
            "instr_pruv": num(ip), "instr_hand16": num(ih), "instr_pruv_over_hand16": num(ir),
            "instr_pruv_typed": num(ipt), "instr_hand16w": num(ihw),
            "instr_pruv_over_hand16w": num(irw), "instr_pruv_typed_over_hand16": num(irt),
            "instr_pruv_boundary_C": num(instr.get(("pruv_C", item, sz))),
            "instr_pruv_boundary_A": num(instr.get(("pruv_A", item, sz))),
            "instr_pruv_boundary_observe": num(instr.get(("pruv_observe", item, sz))),
            "hand16_chk": sorted(chk.get((item, sz), [])) or "unavailable"})
    os.makedirs(WORK, exist_ok=True)
    meta = run_meta(pruvc=PRUVC_PY if args.py else args.pruvc, backend="rust", rustc=rustc(), flags=RUSTC_FLAGS,
                    seed=SEED, profile=args.profile or "custom:" + ",".join(map(str, sizes)),
                    lanes=[s for s, _ in sides] + ["pruv_t", "hand16w"],
                    extra={"rounds": args.rounds, "reps": args.reps, "sizes": sizes, "instr_enabled": not args.no_instr,
                           "pruv_checksum": "rchk + rchk_t per op (driver_m7), compared with hand16 rchk + rchk_w per cell"})
    with open(os.path.join(WORK, "perf.json"), "w") as f:
        json.dump({"meta": meta, "rows": rows, "checksum_problems": bad,
                   "result_checks": {"compared": n_cmp, "mismatches": n_bad,
                                     "failed_cells": ["%s %d: %s" % (i, s_, t) for (i, s_), t in sorted(cell_status.items())]}},
                  f, indent=1)
    for b in bad:
        print("CHK NONDETERMINISTIC", b)
    for (i, s_), t in sorted(cell_status.items()):
        print("FAIL %s %d: %s" % (i, s_, t))
    print("result checks: %d cells compared, %d mismatches" % (n_cmp, n_bad))
    return 1 if bad or n_bad else 0


def instr_retired(cmd, soft=False):
    """Instructions retired by one run of cmd (`/usr/bin/time -l`, macOS). soft: None instead of exiting when the run
    fails or the counter line is missing."""
    r = subprocess.run(["/usr/bin/time", "-l"] + cmd, capture_output=True, text=True, cwd=HERE)
    if r.returncode != 0:
        if soft:
            return None
        sys.exit("%s failed:\n%s" % (cmd[0], r.stderr[-2000:]))
    for l in r.stderr.splitlines():
        if "instructions retired" in l:
            return int(l.split()[0])
    if soft:
        return None
    sys.exit("no instruction count from /usr/bin/time -l")


B_ITEMS = ["current", "ishalted", "attach", "close", "closereopen", "reopenmiss", "replace"]
B_VARS = ["A", "B", "Bnv", "C", "prep", "observe"]
B_PAIR = {"closereopen": 2}  # calls per rep; the closereopen rows are reported per pair (maybeClose + reopen)


def cmd_boundary(args):
    """P01 boundary experiment: A wrapper + packed root, B prepared packed root (t_call), Bnv B without valid(st),
    C direct typed root, plus noop / prep / observe rows. Rounds alternate separate processes; the variant order
    rotates every round and the size order reverses on odd rounds. ns and instructions are per call, except
    closereopen which is per pair (2 calls per rep). Derived: A-B (wrapper), B-C (packed checks incl. valid),
    B-Bnv (validation only, separate: not part of B-C's decomposition)."""
    exe = build(args)
    sizes = list(PROFILES[args.profile]) if args.profile else [int(x) for x in (args.sizes or "100,500").split(",")]
    combos = [(i, B_VARS) for i in B_ITEMS] + [("noop", ["A", "B"])]
    ns, al, ck = {}, {}, {}
    for rd in range(args.rounds):
        for size in (sizes if rd % 2 == 0 else sizes[::-1]):
            for item, vs in combos:
                k = rd % len(vs)
                for var in vs[k:] + vs[:k]:
                    r = subprocess.run([exe, "boundary", str(size), str(args.reps), item, var], capture_output=True,
                                       text=True, cwd=HERE)
                    if r.returncode != 0:
                        sys.exit("boundary %s %s %d failed (rc %d):\n%s" % (item, var, size, r.returncode, r.stderr[-2000:]))
                    for l in r.stdout.splitlines():
                        it, v, sz, t, c, a = l.split()
                        key_ = (it, int(sz), v)
                        f = B_PAIR.get(it, 1)
                        ns.setdefault(key_, []).append(float(t) * f)
                        al[key_] = float(a) * f
                        ck.setdefault(key_, set()).add(c)
    instr = {}
    if not args.no_instr:
        for (item, size, var) in sorted(ns):
            if item in NO_INSTR_ITEMS:
                continue  # the per-batch restore is in the process count (see NO_INSTR_ITEMS); ns only
            calls = B_PAIR.get(item, 1)  # calls per rep
            r1, r2 = args.reps, 5 * args.reps
            vals = []
            for _ in range(3):
                lo = instr_retired([exe, "boundary", str(size), str(r1), item, var])
                hi = instr_retired([exe, "boundary", str(size), str(r2), item, var])
                # per rep (= per call, or per pair for closereopen)
                vals.append((hi - lo) / (r2 - r1))
            instr[(item, size, var)] = statistics.median(vals)
    # checksums must agree across A/B/Bnv/C of an item and size
    bad = []
    for (item, size, var), c in ck.items():
        if len(c) != 1:
            bad.append("%s %s %d: checksum varies across rounds %s" % (item, var, size, sorted(c)))
        ref = ck.get((item, size, "A"))
        if var in ("B", "Bnv", "C") and ref != c:
            bad.append("%s %s %d: checksum %s != A %s" % (item, var, size, sorted(c), sorted(ref or [])))
    stat = {}
    for key_, v in ns.items():
        m = statistics.median(v)
        stat[key_] = {"median": m, "min": min(v), "max": max(v), "spread": (max(v) - min(v)) / m if m else 0.0,
                      "allocs": al[key_], "instr": instr.get(key_), "checksum": sorted(ck[key_]), "raw_ns": v}
    print("unit: ns / instructions per call; closereopen per pair (maybeClose + reopen)")
    print("item | S | variant | median ns | min | max | spread | allocs | instr")
    print("--- | --- | --- | --- | --- | --- | --- | --- | ---")
    fi = lambda x: "-" if x is None else "%.0f" % x
    for (item, size, var) in sorted(stat, key=lambda k: ((B_ITEMS + ["noop"]).index(k[0]), k[1], B_VARS.index(k[2]))):
        s = stat[(item, size, var)]
        print("%s | %d | %s | %.1f | %.1f | %.1f | %.2f | %.3f | %s" % (
            item, size, var, s["median"], s["min"], s["max"], s["spread"], s["allocs"], fi(s["instr"])))
    derived = []
    print()
    print("derived | S | what | ns | instr")
    print("--- | --- | --- | --- | ---")
    for item in B_ITEMS + ["noop"]:
        for size in sizes:
            for name, x, y in (("A-B wrapper", "A", "B"), ("B-C packed checks incl. valid", "B", "C"),
                               ("B-Bnv validation only", "B", "Bnv")):
                if (item, size, x) not in stat or (item, size, y) not in stat:
                    continue
                dn = stat[(item, size, x)]["median"] - stat[(item, size, y)]["median"]
                di = None
                if stat[(item, size, x)]["instr"] is not None and stat[(item, size, y)]["instr"] is not None:
                    di = stat[(item, size, x)]["instr"] - stat[(item, size, y)]["instr"]
                derived.append({"item": item, "size": size, "what": name, "ns": dn, "instr": di})
                print("%s | %d | %s | %.1f | %s" % (item, size, name, dn, fi(di)))
    os.makedirs(WORK, exist_ok=True)
    with open(os.path.join(WORK, "boundary.json"), "w") as f:
        json.dump({"rounds": args.rounds, "reps": args.reps, "sizes": sizes,
                   "rows": [dict(item=k[0], size=k[1], variant=k[2], **v) for k, v in sorted(stat.items())],
                   "derived": derived, "checksum_problems": bad}, f, indent=1)
    for b in bad:
        print("CHECKSUM MISMATCH", b)
    return 1 if bad else 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", choices=["q", "perf", "derive", "same", "boundary"])
    ap.add_argument("--sizes", default=None)  # perf: 100,500,1000; boundary: 100,500
    ap.add_argument("--profile", choices=sorted(PROFILES), default=None, help="perf/boundary sizes: S=100, M=500, SM")
    ap.add_argument("--no-instr", action="store_true")
    ap.add_argument("--hand16", action="store_true", help="q: run hand_m7_u16 instead of the Pruv driver")
    ap.add_argument("--pruvc", default=PRUVC_DEFAULT)
    ap.add_argument("--py", action="store_true")
    ap.add_argument("--cases", default=None)  # default work/m7/cases.jsonl, else SRC_WORK
    ap.add_argument("--oracle", default=None)
    ap.add_argument("--rounds", type=int, default=None)  # perf 5, boundary 7
    ap.add_argument("--reps", type=int, default=200000)
    a = ap.parse_args()
    a.cases = a.cases or input_path("cases.jsonl")
    a.oracle = a.oracle or input_path("oracle.jsonl")
    if a.rounds is None:
        a.rounds = 7 if a.cmd == "boundary" else 5
    sys.exit({"q": cmd_q, "perf": cmd_perf, "derive": cmd_derive, "same": cmd_same, "boundary": cmd_boundary}[a.cmd](a))


if __name__ == "__main__":
    main()
