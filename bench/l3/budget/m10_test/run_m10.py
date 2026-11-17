#!/usr/bin/env python3
"""L3-10 M10a runner: halt (bud_halt.pruv) and the calls that reach it, against the TS oracle.

  python3 bench/l3/budget/m10_test/run_m10.py q      [--pruvc PATH | --py] [--cases F] [--oracle F]
  python3 bench/l3/budget/m10_test/run_m10.py derive (writes work/m10/cases_halt.jsonl + oracle_halt.jsonl)
  python3 bench/l3/budget/m10_test/run_m10.py same   (Python pruvc vs Rust pruvc: byte-identical Rust)
  python3 bench/l3/budget/m10_test/run_m10.py perf   [--rounds N] [--reps N]   (Pruv only; hand Rust / TS: later)

q: each case becomes a driver program over the M7 record (bud_lifecycle.pruv) with the halt section codec in `rest`
(bud_halt.pruv header) and the M9 clock (driver_m10.rs runs the timer scheduler). Compared with the oracle per request:
the value (charge {ok, reason}, current's budget, booleans, undefined) and the effect sequence (runningDelegationIds,
cancelRecovery, stopMain, stopDelegation with its sessionId and id, stopAttachedTurns, holdQueued, save, changed,
announce, isQuiet with its answer, uuid, carryHeld, timer.set with token and delay, timer.clear with token; now / unit /
timer.unref are inputs or host detail). For whole cases: every stored session's budget as a JV object (key order not
compared, halt.at rendered from the instant here), the clocks (M9 fields) and the active timers (token, due).
Classes:
  exact            whole case ran and everything matched
  not-implemented  the case reaches a call owned by another step (decide / raise: M10b, hasCard / setCard: M11).
                   The requests before it are compared and must match; nothing after the cut is filled by host code.
  fail             anything else
"""
import argparse, datetime, json, os, shutil, statistics, subprocess, sys

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
BUD = os.path.dirname(HERE)
ROOT = os.path.abspath(os.path.join(BUD, "..", "..", ".."))
WORK = os.path.join(BUD, "work", "m10")
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
REASONS = ["time", "delegations", "commands", "reviews", "checks"]
WHAT = ["delegations", "commands", "reviews", "checks", "retries"]
CODE = {"current": 1, "isHalted": 2, "reopen": 5, "maybeClose": 6, "charge": 7, "runStarted": 8, "runWaiting": 9,
        "runEnded": 10, "halt": 12}
OWNER = {"decide": "M10b", "raise": "M10b", "hasCard": "M11", "setCard": "M11", "attach": "M7 (not in this root)",
         "replaceForUserMessage": "M7 (not in this root)"}
FX = ("uuid", "changed", "carryHeld", "isQuiet", "timer.set", "timer.clear", "timer.fire", "runningDelegationIds",
      "cancelRecovery", "stopMain", "stopDelegation", "stopAttachedTurns", "holdQueued", "save", "announce",
      "queueContinue", "releaseHeld", "dropHeld")
MINE_FX = {"rdi": "runningDelegationIds", "cancel": "cancelRecovery", "stopMain": "stopMain",
           "stopTurns": "stopAttachedTurns", "hold": "holdQueued", "save": "save", "changed": "changed",
           "announce": "announce", "uuid": "uuid"}


def rustc():
    return shutil.which("rustc") or os.path.expanduser("~/.cargo/bin/rustc")


def compile_gen(which, gen):
    root = "bench/l3/budget/m10_test/root_m10.pruv"
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
    if os.path.exists(gen):
        os.remove(gen)
    compile_gen("py" if args.py else args.pruvc, gen)
    main = os.path.join(WORK, "main_m10.rs")
    with open(main, "w") as f:
        f.write('#![allow(dead_code, unused_imports, unused_variables, unused_mut)]\n#[path = "gen.rs"] mod sol;\n')
        f.write(open(os.path.join(HERE, "driver_m10.rs")).read())
    exe = os.path.join(WORK, "driver_m10")
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


def truthy(v):
    return not (v is None or v is False or v == 0 or v == "" or v is UNDEF)


# ---- record codec (bud_lifecycle.pruv header + bud_halt.pruv halt section) ----
def halt_prefix(h):
    """A well-formed halt object -> the binary prefix of `rest` (atKind 1, stored text), else None (stays JSON)."""
    if not isinstance(h, dict) or set(h) != {"reason", "at", "stoppedDelegationIds"}:
        return None
    if h["reason"] not in REASONS or not is_str(h["at"]) or not isinstance(h["stoppedDelegationIds"], list):
        return None
    ids = h["stoppedDelegationIds"]
    if not all(is_str(x) for x in ids):
        return None
    at = units(h["at"])
    w = [-2, REASONS.index(h["reason"]), 1, 0, len(at)] + at + [len(ids)]
    for x in ids:
        w += key(x)
    return w


def encode_budget(b):
    """Budget (decoded python dict, or None/UNDEF) -> record words. Fields the record has no header slot for (or holds
    with a value the header cannot carry) go into the opaque rest text as canonical JV JSON; a well-formed halt becomes
    the rest's binary prefix."""
    if not isinstance(b, dict):
        return [-1]
    h = [0] * 24
    rest = {}
    pre = []
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
        elif k == "halt" and halt_prefix(v) is not None:
            pre = halt_prefix(v)
        else:
            rest[k] = v
    for k in ("id", "state", "origin", "limits", "used", "openedAt"):
        if k not in b or (k in ("state", "origin") and b[k] not in (STATE if k == "state" else ORIGIN)):
            raise ValueError("budget field %s not representable: %r" % (k, b.get(k)))
    js = units(json.dumps(jv_encode(rest), ensure_ascii=False, separators=(",", ":"))) if rest else []
    strs[5] = pre + js
    w = list(h)
    for s in strs:
        w += [len(s)] + s
    return w


def prefix_ok(r):
    """Mirror of bud_halt.pruv prefix_len: True iff the units r start with a well-formed halt prefix (otherwise the
    whole rest is JSON-opaque). Raw values, nothing is normalized."""
    n = len(r)
    if n < 6 or r[0] != -2 or n > 4294967296:
        return False
    if not 0 <= r[1] <= 4 or r[2] not in (1, 2):
        return False
    al = r[4]
    if al < 0 or al > n - 6:
        return False
    q = 5 + al
    cnt = r[q]
    if cnt < 0 or cnt > n:
        return False
    p = q + 1
    for _ in range(cnt):
        if p >= n:
            return False
        ln = r[p]
        if ln < 0 or ln > n - p - 1:
            return False
        p += 1 + ln
    return p <= n


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
    r = strs[5]
    halt = None
    if prefix_ok(r):
        reason, kind, ms, al = r[1], r[2], r[3], r[4]
        if reason not in range(5) or kind not in (1, 2) or al < 0 or 5 + al >= len(r):
            raise ValueError("bad halt prefix %r" % (r[:5],))
        at = text(r[5:5 + al]) if kind == 1 else iso(ms)
        q = 5 + al
        if r[q] < 0:
            raise ValueError("bad halt prefix id count")
        ids = []
        for _ in range(r[q]):
            if q + 1 >= len(r):
                raise ValueError("halt prefix id past the end")
            n = r[q + 1]
            if n < 0 or q + 2 + n > len(r):
                raise ValueError("halt prefix id length past the end")
            ids.append(text(r[q + 2:q + 2 + n]))
            q += 1 + n
        halt = {"reason": REASONS[reason], "at": at, "stoppedDelegationIds": ids}
        r = r[q + 1:]
    rest = jv_decode(json.loads(text(r))) if r else {}
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
    if halt is not None:
        b["halt"] = halt
    return b


def put_words(sess):
    d = jv_decode(sess)
    rec = encode_budget(d.get("budget"))
    return key(d["id"]) + [len(rec)] + rec


# ---- driver program ----
def call_body(op, args):
    """[code, what x10, flag, sessionId key, budgetId key, runKey key] (the driver inserts now and quiet after the code
    and appends its next runningDelegationIds answer)."""
    a = [x if (isinstance(x, dict) and "sess" in x) else jv_decode(x) for x in args]
    what = [0] * 10
    flag, bid, rk = 0, "", ""
    if op == "maybeClose":
        sid = a[0]["sess"]
    else:
        sid, bid = a[0]["sessionId"], a[0]["budgetId"]
    if op == "charge":
        d = a[1] if isinstance(a[1], dict) else {}
        for i, k in enumerate(WHAT):
            v = d.get(k, UNDEF)
            if v is UNDEF:
                what[2 * i:2 * i + 2] = [0, 0]
            elif v is None:
                what[2 * i:2 * i + 2] = [1, 0]
            else:
                what[2 * i:2 * i + 2] = [2, int(v)]
    elif op in ("runStarted", "runWaiting", "runEnded"):
        rk = a[1]
        if op == "runWaiting":
            flag = 1 if truthy(a[2] if len(a) > 2 else UNDEF) else 0
    elif op == "halt":
        flag = REASONS.index(a[1])
    return [CODE[op]] + what + [flag] + key(sid) + key(bid) + key(rk)


def default_uuid(n):
    return "00000000-0000-4000-8000-%012x" % n


def program(case):
    """Returns (lines, cut) where cut is None or (req, op) of the first call owned by another step."""
    w = case["world"]
    out = ["case " + case["id"], "now %d" % w.get("clockStart", 1700000000000)]
    deps = w.get("deps") or {}
    out.append("quiet " + " ".join("1" if x else "0" for x in (deps.get("isQuiet") or [])))
    for ans in deps.get("runningDelegationIds") or []:
        ids = jv_decode(ans)
        out.append("rdi " + " ".join(map(str, [len(ids)] + [x for i in ids for x in key(i)])))
    for s in w["sessions"]:
        out.append("put " + " ".join(map(str, put_words(s))))
    cut = None
    for e in case["script"]:
        ev = e["ev"]
        if ev == "advance":
            out.append("adv %d" % e["ms"])
        elif ev == "setUnit":
            pass  # the unit only feeds attach / replaceForUserMessage, not in this root
        elif ev == "call":
            if e["op"] not in CODE:
                cut = (e["req"], e["op"])
                break
            out.append("call %s %s" % (e["req"], " ".join(map(str, call_body(e["op"], e["args"])))))
        else:
            raise ValueError("unknown event " + ev)
    out.append("end")
    return out, cut


def parse_driver(t):
    res = {}
    cur = None
    r = None
    for line in t.splitlines():
        if line.startswith("case "):
            cur = {"reqs": {}, "order": [], "obs": None, "x": [], "between": []}
            res[line[5:]] = cur
            r = None
        elif line == "end":
            cur = None
        elif line.startswith("O"):
            cur["obs"] = line
        elif line.startswith("@ "):
            r = line[2:]
            if r != "-":
                cur["order"].append(r)
                cur["reqs"][r] = {"fx": [], "v": None}
        elif line.startswith("X"):
            cur["x"].append((r, line))
        elif line.startswith("F "):
            if r != "-":
                cur["x"].append((r, "fire inside a request " + line))
            else:
                cur["between"].append(("timer.fire", line[2:].strip()))
        elif line.startswith("E "):
            if r == "-":
                cur["between"].append(mine_effect(line[2:].split(), None))  # effect of a timer fire between requests
                continue
            cur["reqs"][r]["fx"].append(line[2:].split())
        elif line.startswith("V "):
            cur["reqs"][r]["v"] = list(map(int, line[2:].split()))
    return res


def parse_obs(line):
    body, _, tl = line[1:].partition("| T")
    w = list(map(int, body.split()))
    p = 0

    def rd():
        nonlocal p
        p += 1
        return w[p - 1]

    def rwords():
        n = rd()
        u = w[p:p + n]
        for _ in range(n):
            rd()
        return u

    sessions = {}
    for _ in range(rd()):
        k = rwords()
        sessions[text(k)] = decode_budget(rwords())
    clocks = []
    for _ in range(rd()):
        k = tuple(rwords())
        owner = tuple(rwords())
        sp, since, armed, act = rd(), rd(), rd(), rd()
        runs = []
        for _ in range(rd()):
            rk = tuple(rwords())
            runs.append((rk, rd()))
        clocks.append((k, owner, sp, since, armed, act, runs))
    timers = []
    for t in tl.split():
        tok, due, bid, own = t.split(":")
        timers.append((tok, int(due), tuple(int(x) for x in bid.split(",") if x), own == "1"))
    return sessions, clocks, timers


# ---- compare ----
def oracle_ev(t, between=False):
    """One oracle trace entry (c in FX) -> effect tuple. Outside a request (between) stopDelegation carries no session."""
    c = t["c"]
    a = t.get("a")
    if c == "carryHeld":
        sess = jv_decode(a[0]["v"]) if isinstance(a[0], dict) and "v" in a[0] else jv_decode(a[0])
        return ("carryHeld", sess["id"], jv_decode(a[1]), jv_decode(a[2])["budgetId"])
    if c == "isQuiet":
        return ("isQuiet", 1 if jv_decode(t.get("o")) else 0)
    if c == "stopDelegation":
        return ("stopDelegation", None if between else tuple(units(a[0])), tuple(units(a[1])))
    if c in ("timer.set", "timer.clear", "timer.fire"):
        d = jv_decode(a)
        return (c, d["token"]) + ((d["delay"],) if c == "timer.set" else ())
    return (c,)


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
            ev.append(oracle_ev(t))
    return ev


def oracle_between(o, stop=None):
    """Effects outside any request (timer fires between requests and what they emit), in trace order. With stop set,
    only those before the first trace entry of request stop."""
    out = []
    for t in o["trace"]:
        if stop is not None and t.get("r") == stop:
            break
        if t.get("r") is None and t["c"] in FX:
            out.append(oracle_ev(t, True))
    return out


def mine_effect(f, sid):
    """Driver effect words (f = split `E` line) -> effect tuple; sid = the request's session id units, None outside a
    request."""
    if f[0] == "quiet":
        return ("isQuiet", int(f[1]))
    if f[0] == "set":
        return ("timer.set", f[1], int(f[3]))
    if f[0] == "clear":
        return ("timer.clear", f[1])
    if f[0] == "stopDel":
        u = tuple(int(x) for x in f[1].split(",")) if len(f) > 1 else ()
        return ("stopDelegation", None if sid is None else tuple(units(sid)), u)
    if f[0] == "carry":
        return ("carryHeld",)
    if f[0] in MINE_FX:
        return (MINE_FX[f[0]],)
    return ("?" + f[0],)


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


def mine_value(words):
    tag = words[0]
    if tag == 0:
        return UNDEF
    if tag in (1, 2):
        return tag == 2
    if tag == 4:
        return decode_budget(words[2:2 + words[1]])
    if tag == 5:
        return {"ok": True} if words[1] == 2 else {"ok": False, "reason": REASONS[words[2]]}
    return ("tag", tag)


def same(a, b):
    return json.dumps(jv_encode(a), sort_keys=True) == json.dumps(jv_encode(b), sort_keys=True)


def norm_obj(x):
    if isinstance(x, dict):
        return {k: norm_obj(v) for k, v in sorted(x.items())}
    if isinstance(x, list):
        return [norm_obj(v) for v in x]
    return x


def absent(v):
    return v is None or (isinstance(v, dict) and "undef" in v)


def compare_final(o, obs):
    sessions, clocks, timers = parse_obs(obs)
    for s in o["finalSessions"]:
        d = jv_decode(s["v"])
        b = d.get("budget")
        if isinstance(b, dict) and "bref" in b:
            b = b["v"]
            if isinstance(b, dict) and "obj" in b:
                b = jv_decode(b)
        if d["id"] not in sessions:
            return "session %r missing" % d["id"]
        mine = sessions[d["id"]]
        if not isinstance(b, dict):
            if mine is not None:
                return "budget presence %r" % d["id"]
            continue
        if jv_encode(norm_obj(mine)) != jv_encode(norm_obj(b)):
            return "final %r: %r != %r" % (d["id"], mine, b)
    if len(sessions) != len(o["finalSessions"]):
        return "session count %d != %d" % (len(sessions), len(o["finalSessions"]))
    tok_of = {t[2]: t[0] for t in timers if t[3]}
    want_c = []
    for bid, cv in o["finalClocks"].get("A", []):
        c = jv_decode(cv)
        runs = [(tuple(units(rk)), 1 if rv["waiting"] else 0) for rk, rv in c["runs"]]
        since = None if absent(c.get("since")) else c.get("since")
        tm = None if absent(c.get("timer")) or c.get("timer") is UNDEF else c.get("timer")
        want_c.append((tuple(units(bid)), tuple(units(c["sessionId"])), since, tm, runs))
    got_c = []
    for k, owner, sp, since, armed, act, runs in clocks:
        if act != sum(1 for _, wv in runs if wv == 0):
            return "active count %s for %r" % (act, runs)
        got_c.append((k, owner, since if sp else None, tok_of.get(k) if armed else None, runs))
    if got_c != want_c:
        return "clocks %r != %r" % (got_c, want_c)
    want_t = sorted((t["token"], t["due"]) for t in o["activeTimers"])
    got_t = sorted((t[0], t[1]) for t in timers)
    if want_t != got_t:
        return "timers %r != %r" % (got_t, want_t)
    return None


STATS = {"halts": 0}


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
        v = mine_value(mine["v"])
        want = req_o[r]
        if want["terminal"] != "ok":
            return "fail", "%s terminal %s" % (r, want["terminal"])
        wv = unwrap(want["value"])
        if e["op"] == "current" and isinstance(v, dict) and isinstance(wv, dict):
            if jv_encode(norm_obj(v)) != jv_encode(norm_obj(wv)):
                return "fail", "%s value %r != %r" % (r, v, wv)
        elif not same(v, wv):
            return "fail", "%s %s value %r != %r" % (r, e["op"], v, wv)
        got = [mine_effect(f, sid) for f in mine["fx"]]
        wfx = oracle_fx(o, r)
        if got != wfx:
            return "fail", "%s %s effects %r != %r" % (r, e["op"], got, wfx)
        if ("runningDelegationIds",) in got:
            STATS["halts"] += 1
    wb = oracle_between(o, cut[0] if cut else None)
    if run["between"] != wb:
        return "fail", "between effects %r != %r" % (run["between"], wb)
    if cut:
        return "not-implemented", "%s (%s)" % (cut[1], OWNER.get(cut[1], "?"))
    d = compare_final(o, run["obs"])
    if d:
        return "fail", d
    return "exact", ""


def run_q(args, cases_f, oracle_f):
    exe = build(args)
    cases = [json.loads(l) for l in open(cases_f)]
    oracle = {}
    for l in open(oracle_f):
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
    count = {"exact": 0, "not-implemented": 0, "fail": 0}
    why, ops, fails = {}, {}, []
    STATS["halts"] = 0
    for c in cases:
        run = runs[c["id"]]
        for rq in run["order"]:
            op = [e["op"] for e in c["script"] if e["ev"] == "call" and e["req"] == rq][0]
            ops[op] = ops.get(op, 0) + 1
        cls, note = classify(c, oracle[c["id"]], run, cuts[c["id"]])
        count[cls] += 1
        if cls == "not-implemented":
            why[note] = why.get(note, 0) + 1
        if cls == "fail":
            fails.append((c["id"], note))
    print(json.dumps({"cases": len(cases), "count": count, "reasons": why, "compared_requests": ops,
                      "halts_compared": STATS["halts"]}, ensure_ascii=False))
    for i, n in fails[:12]:
        print("FAIL", i, n[:900])
    return 1 if fails else 0


def cmd_q(args):
    return run_q(args, args.cases, args.oracle)


def cmd_derive(args):
    """Halt-path variant of each case: the same world and script without decide / raise / hasCard / setCard calls (ids
    get a '-halt' suffix), run through the TS oracle so every remaining call is an M7 / M8 / M9 / M10a path."""
    from oracle import run_oracle
    cases = []
    for l in open(args.cases):
        c = json.loads(l)
        c["id"] += "-halt"
        c["script"] = [e for e in c["script"] if not (e["ev"] == "call" and e["op"] not in CODE)]
        cases.append(c)
    obs = run_oracle(cases)
    with open(os.path.join(WORK, "cases_halt.jsonl"), "w") as f:
        for c in cases:
            f.write(json.dumps(c, ensure_ascii=False) + "\n")
    with open(os.path.join(WORK, "oracle_halt.jsonl"), "w") as f:
        for o in obs:
            f.write(json.dumps(o, ensure_ascii=False) + "\n")
    reach = sum(1 for o in obs if any(t["c"] == "runningDelegationIds" for t in o["trace"]))
    print("derived %d cases, %d reach halt" % (len(cases), reach))
    return 0


def cmd_same(args):
    a, b = os.path.join(WORK, "gen_rs.rs"), os.path.join(WORK, "gen_py.rs")
    os.makedirs(WORK, exist_ok=True)
    compile_gen(args.pruvc, a)
    compile_gen("py", b)
    x, y = open(a, "rb").read(), open(b, "rb").read()
    print(json.dumps({"rs_bytes": len(x), "py_bytes": len(y), "identical": x == y}))
    return 0 if x == y else 1


def cmd_perf(args):
    """Pruv only for M10a (hand_m10.rs / perf_ts.mjs come with M10b): median ns per halt with D running delegations
    (the timed loop also puts the record back open; `putback` times that put alone)."""
    exe = build(args)
    res = {}
    for rd in range(args.rounds):
        for d in (10, 50, 100):
            r = subprocess.run([exe, "perf", str(d), str(args.reps)], capture_output=True, text=True)
            if r.returncode != 0:
                sys.exit("perf failed:\n" + r.stderr[-2000:])
            for l in r.stdout.splitlines():
                item, sz, ns = l.split()
                res.setdefault((item, int(sz)), []).append(float(ns))
    print("item | D | Pruv ns (median of %d)" % args.rounds)
    for (item, sz), v in sorted(res.items()):
        print("%s | %d | %.1f" % (item, sz, statistics.median(v)))
    return 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", choices=["q", "perf", "derive", "same"])
    ap.add_argument("--pruvc", default=PRUVC_DEFAULT)
    ap.add_argument("--py", action="store_true")
    ap.add_argument("--cases", default=os.path.join(WORK, "cases.jsonl"))
    ap.add_argument("--oracle", default=os.path.join(WORK, "oracle.jsonl"))
    ap.add_argument("--rounds", type=int, default=5)
    ap.add_argument("--reps", type=int, default=100000)
    a = ap.parse_args()
    sys.exit({"q": cmd_q, "perf": cmd_perf, "derive": cmd_derive, "same": cmd_same}[a.cmd](a))


if __name__ == "__main__":
    main()
