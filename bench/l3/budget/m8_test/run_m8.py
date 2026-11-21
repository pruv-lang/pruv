#!/usr/bin/env python3
"""L3-10 M8 runner: activity accounting of the request budget (bud_charge.pruv, bud_tools.pruv) against the TS oracle.

  python3 bench/l3/budget/m8_test/run_m8.py q    [--pruvc PATH | --gen GEN.rs] [--cases F] [--oracle F] [--hand16 | --hand NAME]
  python3 bench/l3/budget/m8_test/run_m8.py q --typed [--py] [--lane direct|adapter] [--cases F] [--oracle F]
  python3 bench/l3/budget/m8_test/run_m8.py perf --typed [--py] [--sizes 100,500 | --profile S|M|SM] [--reps N]
  python3 bench/l3/budget/m8_test/run_m8.py churn --typed [--py]
  python3 bench/l3/budget/m8_test/run_m8.py derive  (writes work/m8/cases_acct.jsonl + oracle_acct.jsonl)
  python3 bench/l3/budget/m8_test/run_m8.py perf [--pruvc PATH | --gen GEN.rs] [--rounds N] [--reps N] [--no-instr]
                                                 [--sizes 100,500 | --profile S|M|SM]
perf lanes: pruv, hand16, hand16w, fxmix, fxmixw, oa, oaw, hand (old), ts (see cmd_perf); writes work/m8/perf_meta.json.

q: converts each Q2 case (m8_test/cases_q2.jsonl, committed: q_gen.py --families Q2 --count 300 --seed 20261117) into the
driver program, runs it, and compares with the oracle output (m8_test/oracle_q2.jsonl, committed: oracle.py on those cases). Compared per call: the return value of every charge ({ok} / {ok:false, reason}) and that the
void methods (noteFailure, noteTool, forgetTurn) return undefined. For cases that run to the end: every final
session's budget presence, state, halt reason and counters (delegations, commands, reviews, checks, tools, failures,
retries), and toolSeen (turns in order, activities in order).
Classes:
  exact            whole case ran and everything matched
  not-implemented  the case reaches a path owned by another milestone and is cut there; the prefix before the cut
                   must match. Cuts: a call of reopen / halt / current / isHalted / hasCard (M7 / M10 / M11), and an
                   over-limit charge that dispatches the halt sub-operation (M10). At a halt dispatch the halt frame
                   is checked against the oracle: the request answers {ok:false, reason:k} with the same k, the
                   oracle's halt ran for the same ref (runningDelegationIds(ref)), and the counters at the halt equal
                   the oracle's at the halt's findSession (nothing was charged). Nothing after the cut is filled by
                   host code.
  known-difference none in M8 (the class is kept for the table).
  fail             anything else.
"""
import argparse, json, os, shutil, statistics, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
BUD = os.path.dirname(HERE)
ROOT = os.path.abspath(os.path.join(BUD, "..", "..", ".."))
WORK = os.path.join(BUD, "work", "m8")
BUILD = WORK


def shared_checkout():
    """Main checkout of this repository (the worktree's git common dir parent), or ROOT."""
    if not os.path.exists(os.path.join(ROOT, ".git")) or not shutil.which("git"):
        return ROOT
    result = subprocess.run(["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
                            cwd=ROOT, capture_output=True, text=True)
    return os.path.dirname(result.stdout.strip()) if result.returncode == 0 else ROOT


def checkout_file(*parts):
    local = os.path.join(ROOT, *parts)
    return local if os.path.isfile(local) else os.path.join(shared_checkout(), *parts)


PRUVC_DEFAULT = checkout_file("pruvc-rs", "target", "release", "pruvc")
PYPRUVC_DEFAULT = checkout_file("pruvc", "pruvc.py")
PYTHON_VENV = checkout_file(".venv", "bin", "python")
LIBS = ["--lib", "bench/l3/budget", "--lib", "bench/l3/lib"]
STATE = {"open": 0, "halted": 1, "closed": 2}
REASONS = ["time", "delegations", "commands", "reviews", "checks"]
KEYS = ["delegations", "commands", "reviews", "checks"]
OWN = {"charge": 1, "noteFailure": 2, "noteTool": 3, "forgetTurn": 4}
OUT_OF_SCOPE = ("reopen", "halt", "current", "isHalted", "hasCard")
HALT_CHECKED = [0]
CHK_OK = [0]
CALLS = {}
BUILDS = {}  # binary name -> the rustc argv that built it in this run (recorded into the perf meta)


def rustc():
    return shutil.which("rustc") or os.path.expanduser("~/.cargo/bin/rustc")


def build(args, typed=False):
    os.makedirs(BUILD, exist_ok=True)
    suffix = ("_typed_py" if args.py else "_typed") if typed else ""
    gen = os.path.join(BUILD, "gen%s.rs" % suffix)
    if args.gen and (typed or not getattr(args, "typed", False)):
        shutil.copyfile(args.gen, gen)
    else:
        root = os.path.relpath(os.path.join(HERE, "root_m8%s.pruv" % ("_typed" if typed else "")), ROOT)
        python = PYTHON_VENV if os.path.isfile(PYTHON_VENV) else sys.executable
        cmd = [python, args.pypruvc] if typed and args.py else [args.pruvc]
        backend = ["--backend", "rust"] if typed and args.py else ["--emit", "rs"] if typed else LIBS
        if typed:
            backend += LIBS
        c = subprocess.run(cmd + [root, "-o", gen] + backend, cwd=ROOT, capture_output=True, text=True)
        if c.returncode != 0:
            sys.exit("pruvc failed:\n" + c.stdout[-2000:] + c.stderr[-2000:])
    main = os.path.join(BUILD, "main_m8%s.rs" % suffix)
    driver = os.path.join(HERE, "driver_m8%s.rs" % ("_typed" if typed else ""))
    with open(main, "w") as f:
        f.write('#![allow(dead_code, unused_imports, unused_variables, unused_mut)]\n'
                '#[path = "gen%s.rs"] mod sol;\n' % suffix)
        if not typed:
            f.write('include!("%s");\n' % os.path.join(BUD, "state_buf.rs").replace("\\", "/"))
        with open(driver) as source:
            f.write(source.read())
    exe = os.path.join(BUILD, "driver_m8%s" % suffix)
    argv = [rustc(), "-C", "opt-level=3", "--edition", "2021", "-o", exe, main]
    c = subprocess.run(argv, capture_output=True, text=True)
    if c.returncode != 0:
        sys.exit("rustc failed:\n" + c.stderr[-4000:])
    BUILDS[os.path.basename(exe)] = argv
    return exe


# Hand baselines: binary name -> (source file in m8_test, extra rustc flags). hand_m8_fx is hand_m8_u16.rs built with
# --cfg fxmix (Fx-style hasher + murmur3 fmix64 finalizer instead of SipHash).
HANDS = {
    "hand_m8": ("hand_m8.rs", []),
    "hand_m8_u16": ("hand_m8_u16.rs", []),
    "hand_m8_fx": ("hand_m8_u16.rs", ["--cfg", "fxmix"]),
    "hand_m8_oa": ("hand_m8_oa.rs", []),
}
HAND_ALIAS = {"hand16": "hand_m8_u16", "fxmix": "hand_m8_fx", "oa": "hand_m8_oa", "hand": "hand_m8"}


def hand_name(name):
    name = HAND_ALIAS.get(name, name)
    if name not in HANDS:
        sys.exit("unknown hand %r (known: %s)" % (name, ", ".join(sorted(HANDS))))
    return name


def build_hand(name):
    """Compiles the hand baseline <name> (see HANDS) into work/m8/<name> and returns the path."""
    name = hand_name(name)
    src, extra = HANDS[name]
    os.makedirs(BUILD, exist_ok=True)
    exe = os.path.join(BUILD, name)
    argv = [rustc(), "-C", "opt-level=3", "--edition", "2021"] + extra + ["-o", exe, os.path.join(HERE, src)]
    c = subprocess.run(argv, capture_output=True, text=True)
    if c.returncode != 0:
        sys.exit("rustc failed (%s):\n%s" % (name, c.stderr[-3000:]))
    BUILDS[name] = argv
    return exe


# ---- values ----
def units(s):
    if isinstance(s, dict) and "u16" in s:
        return list(s["u16"])
    b = s.encode("utf-16-le", "surrogatepass")
    return [b[i] | (b[i + 1] << 8) for i in range(0, len(b), 2)]


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


def key(s):
    u = units(s)
    return [len(u)] + pack(u)


def obj(v):
    return dict(v["obj"]) if isinstance(v, dict) and "obj" in v else None


def undef(v):
    return isinstance(v, dict) and "undef" in v


def truthy(v):
    return not (v is None or v is False or v == 0 or v == "" or undef(v))


def text(v):
    return "".join(chr(u) for u in units(v)) if isinstance(v, dict) else v


def budget_rec(b):
    """PLACEHOLDER record words (root_m8.pruv) for a session's budget value."""
    if b is None:
        return [-1] + [0] * 13 + [0]
    lim = obj(b["limits"])
    used = obj(b["used"])
    bid = units(b["id"])
    h = obj(b.get("halt")) if b.get("halt") is not None else None
    reason = REASONS.index(text(h["reason"])) if h and h.get("reason") is not None else -1
    return ([STATE[b["state"]], 1 if truthy(b.get("unlimited")) else 0, reason] + [lim[k] for k in KEYS]
            + [used[k] for k in KEYS] + [used["tools"], used["failures"], used["retries"], len(bid)] + pack(bid))


def what_words(w):
    d = obj(w) or {}
    out = []
    for k in KEYS + ["retries"]:
        v = d.get(k, {"undef": 1})
        if undef(v):
            out += [0, 0]
        elif v is None:
            out += [1, 0]
        else:
            out += [2, int(v)]
    return out


def ref_words(r):
    r = obj(r)
    return key(r["sessionId"]) + key(r["budgetId"])


# ---- driver program ----
def program(case):
    """Returns (lines, steps, cut): steps maps script index -> request id; cut is None or the first out-of-scope
    request id."""
    out = ["case " + case["id"]]
    for s in case["world"]["sessions"]:
        d = obj(s)
        rec = budget_rec(obj(d.get("budget")) if d.get("budget") is not None else None)
        out.append("put " + " ".join(map(str, key(d["id"]) + [len(rec)] + rec)))
    steps, cut = {}, None
    for i, e in enumerate(case["script"]):
        if e["ev"] != "call":
            raise ValueError("unknown event " + e["ev"])
        op, a = e["op"], e["args"]
        if op not in OWN:
            cut = e["req"]
            break
        steps[i] = e["req"]
        out.append("step %d" % i)
        if op == "charge":
            w = [1] + ref_words(a[0]) + what_words(a[1])
        elif op == "noteFailure":
            w = [2] + ref_words(a[0])
        elif op == "noteTool":
            w = [3] + ref_words(a[0]) + key(a[1]) + key(a[2])
        else:
            w = [4] + key(a[0])
        out.append("call " + " ".join(map(str, w)))
    out.append("end")
    return out, steps, cut


def parse_driver(text_):
    res = {}
    cur = None
    for line in text_.splitlines():
        if line.startswith("case "):
            cur = {"log": [], "obs": None}
            res[line[5:]] = cur
        elif line == "end":
            cur = None
        elif line.startswith("O"):
            cur["obs"] = line
        else:
            cur["log"].append(line)
    return res


def parse_obs(line):
    w = list(map(int, line[1:].split()))
    p = 0

    def rd():
        nonlocal p
        p += 1
        return w[p - 1]

    def rkey():
        n = rd()
        u = w[p:p + n]
        for _ in range(n):
            rd()
        return tuple(u)

    sessions = {}
    for _ in range(rd()):
        k = rkey()
        sessions[k] = list(rkey())
    for _ in range(rd()):
        rkey(); rkey(); rd(); rd(); rd(); rd()
        for _ in range(rd()):
            rkey(); rd()
    turns = []
    for _ in range(rd()):
        t = rkey()
        turns.append((t, [rkey() for _ in range(rd())]))
    return sessions, turns


def counters(b):
    used = obj(b["used"])
    return [used[k] for k in KEYS] + [used["tools"], used["failures"], used["retries"]]


def mine_counters(rec):
    return rec[7:14]


def compare_sessions(want_sessions, sessions):
    diffs = []
    for d in want_sessions:
        sk = tuple(units(d["id"]))
        b = d.get("budget")
        b = obj(b["v"]) if b is not None else None
        mine = sessions.get(sk)
        if mine is None:
            diffs.append("session missing %r" % (d["id"],))
            continue
        if b is None:
            if mine[0] != -1:
                diffs.append("budget presence")
            continue
        if mine[0] != STATE[b["state"]]:
            diffs.append("state %s %s" % (mine[0], b["state"]))
        h = obj(b.get("halt")) if b.get("halt") is not None else None
        reason = REASONS.index(text(h["reason"])) if h and h.get("reason") is not None else -1
        if mine[2] != reason:
            diffs.append("halt reason %s %s" % (mine[2], reason))
        if mine_counters(mine) != counters(b):
            diffs.append("counters %r != %r for %r" % (mine_counters(mine), counters(b), d["id"]))
    return diffs


def charge_value(code):
    if code in (0, 1):
        return {"obj": [["ok", True]]}
    if 16 <= code <= 20:
        return {"obj": [["ok", False], ["reason", REASONS[code - 16]]]}
    return None


def classify(case, o, run, steps, cut):
    reqs = {r["id"]: r for r in o["requests"]}
    ops = {e["req"]: e["op"] for e in case["script"]}
    log = run["log"]
    cur = None
    for l in log:
        if l.startswith("P X"):  # a Q2 case's put was rejected as malformed input: never expected
            return "fail", "put rejected: %s" % l
        if l.startswith("@ "):
            cur = steps[int(l[2:])]
            continue
        rq = reqs.get(cur)
        if rq is None:
            return "fail", "no oracle request for %s" % cur
        if l.startswith("X"):
            return "fail", "driver status %s at %s" % (l, cur)
        if l.startswith("H "):
            f = list(map(int, l[2:].split()))
            # [20, k, sessionId key, budgetId key]: both keys raw, i.e. in packed form (n, packed words), as ref_words
            if f[0] != 20:
                return "fail", "halt frame %r" % f
            k = f[1]
            want = {"obj": [["ok", False], ["reason", REASONS[k]]]}
            if rq["value"] != want:
                return "fail", "halt %s: value %r != %r" % (cur, want, rq["value"])
            tr = [t for t in o["trace"] if t.get("r") == cur]
            rdi = [t for t in tr if t["c"] == "runningDelegationIds"]
            if not rdi or ref_words(rdi[0]["a"][0]) != f[2:]:
                return "fail", "halt %s: oracle halt ref %r != %r" % (cur, rdi[:1], f[2:])
            fs = [t for t in tr[:tr.index(rdi[0])] if t["c"] == "findSession"]
            snap = obj(fs[-1]["o"]["v"])
            sessions, _ = parse_obs(run["obs"])
            d = compare_sessions([snap], sessions)
            HALT_CHECKED[0] += 1
            if d:
                return "fail", "at halt %s: %s" % (cur, "; ".join(d))
            CALLS["halt"] = CALLS.get("halt", 0) + 1
            return "not-implemented", "halt dispatch (M10)"
        v = l[2:]
        CALLS[ops[cur]] = CALLS.get(ops[cur], 0) + 1
        if v == "-":
            if not undef(rq["value"]):
                return "fail", "%s: void != %r" % (cur, rq["value"])
        else:
            got = charge_value(int(v))
            if got != rq["value"]:
                return "fail", "%s: charge %s -> %r != %r" % (cur, v, got, rq["value"])
    if cut:
        return "not-implemented", "%s call" % ops[cut]
    sessions, turns = parse_obs(run["obs"])
    diffs = compare_sessions([obj(s["v"]) for s in o["finalSessions"]], sessions)
    want_t = [(tuple(units(t)), [tuple(units(a)) for a in acts]) for t, acts in o["finalToolSeen"].get("A", [])]
    if turns != want_t:
        diffs.append("toolSeen %r != %r" % (turns, want_t))
    if diffs:
        return "fail", "; ".join(diffs)
    return "exact", ""


def enc(v):
    """Plain fixture JSON -> the tagged value form of Q cases (objects as {"obj": [[k, v], ...]}); values already
    tagged ({"obj"}, {"undef"}, {"u16"}) keep their tag, object members are encoded recursively."""
    if isinstance(v, list):
        return [enc(x) for x in v]
    if isinstance(v, dict):
        if set(v) == {"obj"}:
            return {"obj": [[k, enc(x)] for k, x in v["obj"]]}
        if set(v) <= {"undef", "u16"}:
            return v
        return {"obj": [[k, enc(x)] for k, x in v.items()]}
    return v


def fixture_cases(groups):
    """Fixture groups (e.g. "C,D") as tagged Q cases plus their oracle output, written to work/m8/fixtures_*.jsonl."""
    sys.path.insert(0, BUD)
    from common import load_group, strip_case
    from oracle import run_oracle
    cases = []
    for g in groups.split(","):
        for f in load_group(g.strip()):
            c = strip_case(f)
            c["world"] = {"sessions": enc(c["world"]["sessions"])}
            c["script"] = [dict(e, args=enc(e["args"])) for e in c["script"]]
            cases.append(c)
    obs = run_oracle(cases)
    os.makedirs(BUILD, exist_ok=True)
    cp, op = os.path.join(BUILD, "fixtures_cases.jsonl"), os.path.join(BUILD, "fixtures_oracle.jsonl")
    with open(cp, "w") as f:
        for c in cases:
            f.write(json.dumps(c, ensure_ascii=False) + "\n")
    with open(op, "w") as f:
        for o in obs:
            f.write(json.dumps(o, ensure_ascii=False) + "\n")
    return cp, op


# ---- negative / boundary controls (fixed; outside the TS oracle's exact range, so the expected answers are here) ----
MAX = 9007199254740991  # 2^53 - 1
BID = "11111111-1111-4111-8111-000000000001"


def ctl_session(sid="s1", state="open", used=None, limits=None, unlimited=None, reason=None, bid=BID):
    u = {"activeMs": 0, "delegations": 0, "commands": 0, "reviews": 0, "checks": 0, "tools": 0, "failures": 0,
         "retries": 0}
    u.update(used or {})
    lim = {"timeMinutes": 120, "delegations": 24, "commands": 3, "reviews": 2, "checks": 1}
    lim.update(limits or {})
    b = {"id": bid, "openedAt": "2023-11-14T22:13:20.000Z", "origin": "user", "state": state, "limits": lim,
         "used": u}
    if unlimited:
        b["unlimited"] = True
    if reason is not None:
        b["halt"] = {"reason": reason, "at": "2023-11-14T22:13:20.000Z"}
    return {"id": sid, "budget": b}


def REF(sid="s1", bid=BID):
    return {"sessionId": sid, "budgetId": bid}


def ctl(cid, sessions, calls, logs, counters_, turns=0, acts=None):
    """A control: script of (op, args); `logs` = the driver lines expected per call ("R 0", "R -", "X 5", "H 20 k"
    prefix); `counters_` = s1's final [delegations, commands, reviews, checks, tools, failures, retries]; `turns` =
    the number of toolSeen turns at the end; `acts` (optional) = the recorded activity keys at the end, as
    [(turn, [activity, ...]), ...] in observe order. A rejected call stops the case (later calls do not run)."""
    script = [{"ev": "call", "req": "r%d" % (i + 1), "op": op, "args": enc(a)} for i, (op, a) in enumerate(calls)]
    return {"id": cid, "world": {"sessions": enc(sessions)}, "script": script, "logs": logs, "counters": counters_,
            "turns": turns, "acts": acts}


def controls():
    z = [0] * 7
    big = {"delegations": MAX, "commands": MAX, "reviews": MAX, "checks": MAX}
    return [
        # ResourceBound refuses atomically: nothing written, not even the keys that fit
        ctl("N01-max-counter-unlimited", [ctl_session(used={"delegations": MAX}, unlimited=True)],
            [("charge", [REF(), {"delegations": 1}])], ["X 5"], [MAX, 0, 0, 0, 0, 0, 0]),
        ctl("N02-retries-last-key-refuses-all", [ctl_session(used={"retries": MAX}, limits=big)],
            [("charge", [REF(), {"delegations": 1, "commands": 1, "retries": 1}])], ["X 5"], [0, 0, 0, 0, 0, 0, MAX]),
        ctl("N03-exact-max-is-allowed", [ctl_session(used={"checks": MAX - 1}, limits=big)],
            [("charge", [REF(), {"checks": 1}]), ("charge", [REF(), {"checks": 1}])], ["R 0", "X 5"],
            [0, 0, 0, MAX, 0, 0, 0]),
        ctl("N04-unsafe-amount", [ctl_session(unlimited=True)],
            [("charge", [REF(), {"commands": MAX + 1}])], ["X 5"], z),
        ctl("N05-unsafe-negative-amount", [ctl_session()],
            [("charge", [REF(), {"reviews": -MAX - 1}])], ["X 5"], z),
        ctl("N06-negative-to-min-then-below", [ctl_session()],
            [("charge", [REF(), {"reviews": -MAX}]), ("charge", [REF(), {"reviews": -1}])], ["R 0", "X 5"],
            [0, 0, -MAX, 0, 0, 0, 0]),
        ctl("N07-negative-amount-lowers-counter", [ctl_session(used={"commands": 1})],
            [("charge", [REF(), {"commands": -3, "retries": -1}])], ["R 0"], [0, -2, 0, 0, 0, 0, -1]),
        ctl("N08-limit-equality-then-over", [ctl_session(used={"commands": 1})],
            [("charge", [REF(), {"commands": 2}]), ("charge", [REF(), {"commands": 1}])], ["R 0", "H 20 2"],
            [0, 3, 0, 0, 0, 0, 0]),
        ctl("N09-over-limit-before-later-bound", [ctl_session(used={"commands": MAX}, limits={"commands": MAX})],
            [("charge", [REF(), {"delegations": 25, "commands": 1}])], ["H 20 1"], [0, MAX, 0, 0, 0, 0, 0]),
        ctl("N10-bound-before-later-over-limit", [ctl_session(used={"delegations": MAX}, limits={"delegations": MAX})],
            [("charge", [REF(), {"delegations": 1, "commands": 9}])], ["X 5"], [MAX, 0, 0, 0, 0, 0, 0]),
        ctl("N11-stored-counter-outside-range", [ctl_session(used={"retries": MAX + 10})],
            [("charge", [REF(), {"commands": 1}])], ["X 5"], [0, 0, 0, 0, 0, 0, MAX + 10]),
        ctl("N12-nullish-amounts", [ctl_session(used={"checks": 1})],
            [("charge", [REF(), {"obj": [["checks", None], ["reviews", {"undef": 1}]]}])], ["R 0"],
            [0, 0, 0, 1, 0, 0, 0]),
        ctl("N13-halted-answers-before-sums", [ctl_session(state="halted", reason="commands", used={"commands": MAX})],
            [("charge", [REF(), {"commands": MAX}])], ["R 18"], [0, MAX, 0, 0, 0, 0, 0]),
        ctl("N14-closed-answers-before-sums", [ctl_session(state="closed", used={"retries": MAX})],
            [("charge", [REF(), {"retries": MAX}])], ["R 1"], [0, 0, 0, 0, 0, 0, MAX]),
        ctl("N15-failure-at-max-halted", [ctl_session(state="halted", reason="time", used={"failures": MAX})],
            [("noteFailure", [REF()])], ["X 5"], [0, 0, 0, 0, 0, MAX, 0]),
        ctl("N16-failure-to-max", [ctl_session(used={"failures": MAX - 1})],
            [("noteFailure", [REF()]), ("noteFailure", [REF()])], ["R -", "X 5"], [0, 0, 0, 0, 0, MAX, 0]),
        ctl("N17-failure-negative-min", [ctl_session(used={"failures": -MAX})],
            [("noteFailure", [REF()])], ["R -"], [0, 0, 0, 0, 0, -MAX + 1, 0]),
        ctl("N18-stale-ref-at-max-is-no-op", [ctl_session(used={"failures": MAX})],
            [("noteFailure", [REF(bid="11111111-1111-4111-8111-000000000063")])], ["R -"], [0, 0, 0, 0, 0, MAX, 0]),
        ctl("N19-tool-to-max", [ctl_session(used={"tools": MAX - 1})],
            [("noteTool", [REF(), "t1", "a1"]), ("noteTool", [REF(), "t1", "a1"])], ["R -", "R -"],
            [0, 0, 0, 0, MAX, 0, 0], turns=1),
        # a refused noteTool leaves the dedup set untouched too (the activity is not recorded)
        ctl("N20-tool-at-max-new-activity", [ctl_session(used={"tools": MAX})],
            [("noteTool", [REF(), "t1", "a1"])], ["X 5"], [0, 0, 0, 0, MAX, 0, 0], turns=0),
        # a refused NEW activity on an EXISTING turn: the turn keeps only a1 (a2 is not recorded)
        ctl("N21-tool-at-max-existing-turn", [ctl_session(used={"tools": MAX - 1})],
            [("noteTool", [REF(), "t1", "a1"]), ("noteTool", [REF(), "t1", "a2"])], ["R -", "X 5"],
            [0, 0, 0, 0, MAX, 0, 0], turns=1, acts=[("t1", ["a1"])]),
        # negative stored tools counter: the -MAX side of the single range test, mirrored from N17
        ctl("N22-tool-negative-min", [ctl_session(used={"tools": -MAX})],
            [("noteTool", [REF(), "t1", "a1"])], ["R -"], [0, 0, 0, 0, -MAX + 1, 0, 0], turns=1,
            acts=[("t1", ["a1"])]),
        # missing budget (§15 #6): the dedup key is consumed without a current budget, so the later call with the
        # real budget is a dedup hit and charges nothing (P21)
        ctl("N23-no-budget-consumes-key", [ctl_session(), {"id": "s2"}],
            [("noteTool", [REF(sid="s2"), "t1", "a1"]), ("noteTool", [REF(sid="s2"), "t1", "a1"]),
             ("noteTool", [REF(), "t1", "a1"])], ["R -", "R -", "R -"], z, turns=1, acts=[("t1", ["a1"])]),
        # stale budget id: same, the key is consumed and the real ref then dedups
        ctl("N24-stale-budget-consumes-key", [ctl_session()],
            [("noteTool", [REF(bid="11111111-1111-4111-8111-000000000063"), "t1", "a1"]),
             ("noteTool", [REF(), "t1", "a1"])], ["R -", "R -"], z, turns=1, acts=[("t1", ["a1"])]),
        # no budget, new activity on an existing turn (the known-turn insert path without a counter)
        ctl("N25-no-budget-existing-turn", [ctl_session(), {"id": "s2"}],
            [("noteTool", [REF(), "t1", "a1"]), ("noteTool", [REF(sid="s2"), "t1", "a2"]),
             ("noteTool", [REF(), "t1", "a2"])], ["R -", "R -", "R -"], [0, 0, 0, 0, 1, 0, 0], turns=1,
            acts=[("t1", ["a1", "a2"])]),
    ]


def cmd_controls(args):
    """Runs the fixed negative / boundary controls through the Pruv driver (or --hand) and checks every call's
    driver line, s1's final counters (atomic refusal: the record is unchanged) and the toolSeen turn count."""
    hand = "hand_m8_u16" if args.hand16 else args.hand
    exe = build_hand(hand) if hand else build(args)
    cs = controls()
    prog = []
    for c in cs:
        prog += program(c)[0]
    r = subprocess.run([exe, "q"], input="\n".join(prog) + "\n", capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit("driver failed:\n" + r.stderr[-3000:])
    runs = parse_driver(r.stdout)
    fails = []
    for c in cs:
        run = runs[c["id"]]
        got = [l for l in run["log"] if not l.startswith("@ ")]
        ok = len(got) == len(c["logs"]) and all(g == w or (w.startswith("H ") and g.startswith(w + " "))
                                                  for g, w in zip(got, c["logs"]))
        sessions, turns = parse_obs(run["obs"])
        cnt = mine_counters(sessions[tuple(units("s1"))])
        acts = None if c["acts"] is None else [(tuple(units(t)), [tuple(units(a)) for a in al]) for t, al in c["acts"]]
        if not ok or cnt != c["counters"] or len(turns) != c["turns"] or (acts is not None and turns != acts):
            fails.append("%s: lines %r want %r; counters %r want %r; turns %r want %d / %r"
                         % (c["id"], got, c["logs"], cnt, c["counters"], turns, c["turns"], acts))
    print(json.dumps({"controls": len(cs), "pass": len(cs) - len(fails), "fail": len(fails)}))
    for f in fails:
        print("FAIL", f)
    return 1 if fails else 0


# ---- memory: peak RSS and allocation count (P20; the P13b memory gate's accounting) ----
# Every mem binary is the lane's own source with this prelude in front: a counting global allocator (alloc,
# alloc_zeroed and realloc each count one) that prints `allocs N` to stderr at exit. The timed lanes are built
# without it, so wall time is not affected.
MEM_PRELUDE = r'''
struct CountingAlloc;
static ALLOC_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
unsafe impl std::alloc::GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, l: std::alloc::Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::alloc::System.alloc(l)
    }
    unsafe fn alloc_zeroed(&self, l: std::alloc::Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::alloc::System.alloc_zeroed(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: std::alloc::Layout) {
        std::alloc::System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: std::alloc::Layout, n: usize) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::alloc::System.realloc(p, l, n)
    }
}
#[global_allocator]
static GLOBAL_ALLOC: CountingAlloc = CountingAlloc;
extern "C" {
    fn atexit(cb: extern "C" fn()) -> i32;
}
extern "C" fn report_allocs() {
    eprintln!("allocs {}", ALLOC_COUNT.load(std::sync::atomic::Ordering::Relaxed));
}
extern "C" fn register_allocs() {
    unsafe {
        atexit(report_allocs);
    }
}
#[used]
#[link_section = "__DATA,__mod_init_func"]
static REGISTER_ALLOCS: extern "C" fn() = register_allocs;
'''
MEM_LANES = [("pruv", None, "perf"), ("hand16", "hand_m8_u16", "perf"), ("hand16w", "hand_m8_u16", "perfw"),
             ("oaw", "hand_m8_oa", "perfw")]


def build_mem(name):
    """The mem binary of the Pruv driver (name None; needs build() first: uses work/m8/main_m8.rs) or of a hand."""
    if name is None:
        src = os.path.join(WORK, "main_m8.rs")
        body = open(src).read().split("\n", 1)
        # keep the crate-level #![allow] line first; mapped state buffers count like allocator calls (STATE_MAPS, as
        # driver_m4.rs counts them)
        pre = MEM_PRELUDE.replace('eprintln!("allocs {}", ALLOC_COUNT.load(std::sync::atomic::Ordering::Relaxed));',
                                  'eprintln!("allocs {}", ALLOC_COUNT.load(std::sync::atomic::Ordering::Relaxed)'
                                  ' + STATE_MAPS.load(std::sync::atomic::Ordering::Relaxed));')
        assert pre != MEM_PRELUDE
        text_ = body[0] + "\n" + pre + body[1]
        out, extra = os.path.join(WORK, "driver_m8_mem"), []
        path = os.path.join(WORK, "main_m8_mem.rs")
    else:
        src, extra = HANDS[name]
        text_ = MEM_PRELUDE + 'include!("%s");\n' % os.path.join(HERE, src)
        out = os.path.join(WORK, name + "_mem")
        path = out + ".rs"
    with open(path, "w") as f:
        f.write(text_)
    argv = [rustc(), "-C", "opt-level=3", "--edition", "2021"] + extra + ["-o", out, path]
    c = subprocess.run(argv, capture_output=True, text=True)
    if c.returncode != 0:
        sys.exit("rustc failed (mem %s):\n%s" % (name, c.stderr[-3000:]))
    BUILDS[os.path.basename(out)] = argv
    return out


def mem_run(cmd):
    r = subprocess.run(["/usr/bin/time", "-l"] + cmd, capture_output=True, text=True, cwd=HERE)
    if r.returncode != 0:
        sys.exit("%s failed:\n%s" % (cmd[0], r.stderr[-2000:]))
    rss = allocs = None
    for l in r.stderr.splitlines():
        f = l.split()
        if "maximum resident set size" in l:
            rss = int(f[0])
        elif len(f) == 2 and f[0] == "allocs":
            allocs = int(f[1])
    return rss, allocs


def mem_pass(sizes, reps, runs, items=("charge", "failure")):
    """Peak RSS (bytes) and allocation count per lane, item and size: `runs` runs each, lanes interleaved per run;
    prints median [min-max]."""
    exes = [(lane, [build_mem(name), mode]) for lane, name, mode in MEM_LANES]
    res = {}
    for k in range(runs):
        order = exes[k % len(exes):] + exes[:k % len(exes)]
        for size in sizes:
            for item in items:
                for lane, cmd in order:
                    rss, al = mem_run(cmd + [str(size), str(reps), item])
                    d = res.setdefault((item, size), {}).setdefault(lane, ([], []))
                    d[0].append(rss)
                    d[1].append(al)

    def mm(xs, scale=1):
        xs = [x / scale for x in xs]
        return "%.0f [%.0f-%.0f]" % (statistics.median(xs), min(xs), max(xs))

    print("memory (%d runs, reps %d): peak RSS KiB and allocation count, median [min-max]"
          " (toolgrow covers toolnew and toolforget)" % (runs, reps))
    print("item | size | " + " | ".join("%s RSS KiB | %s allocs" % (l, l) for l, _, _ in MEM_LANES))
    for (item, size), d in sorted(res.items(), key=lambda x: (x[0][0], x[0][1])):
        print("%s | %d | %s" % (item, size, " | ".join("%s | %s" % (mm(d[l][0], 1024), mm(d[l][1])) for l, _, _ in
                                                        MEM_LANES)))
    return res


def check_malformed_keys(exe):
    # Every payload is the final packed key of a forgetTurn call.
    keys = {"length": "-1", "padding": "1 1", "truncated": "5 0", "extra": "0 0"}
    source = "".join(f"case {name}\ncall 4 {words}\nend\n" for name, words in keys.items())
    expected = "".join(f"case {name}\nX 2\nO 0 0 0\nend\n" for name in keys)
    # The payload contains fourteen counters and an empty budget key (15 words).
    source += "case put-length\nput 0 14" + " 0" * 15 + "\nend\n"
    expected += "case put-length\nP X 2\nO 0 0 0\nend\n"
    source += "case failure-extra\ncall 2 0 0 0\nend\n"
    expected += "case failure-extra\nX 2\nO 0 0 0\nend\n"
    result = subprocess.run([exe, "q"], input=source, capture_output=True, text=True, check=True)
    if result.stdout != expected:
        sys.exit("packed key admission mismatch:\n" + result.stdout)
    print("packed key and frame admission PASS (6 cases)")


def cmd_q(args):
    # --hand16: run the hand-written UTF-16 Rust baseline (hand_m8_u16.rs q mode) instead of the Pruv driver; the
    # comparison / classification below is the same, so mismatches vs the oracle are counted the same way.
    if args.fixtures:
        args.cases, args.oracle = fixture_cases(args.fixtures)
    hand = "hand_m8_u16" if args.hand16 else args.hand
    exe = build_hand(hand) if hand else build(args, typed=args.typed)
    if args.typed:
        check_malformed_keys(exe)
    cases = [json.loads(l) for l in open(args.cases)]
    oracle = {}
    for l in open(args.oracle):
        o = json.loads(l)
        oracle[o["id"]] = o
    prog, meta = [], {}
    for c in cases:
        lines, steps, cut = program(c)
        prog += lines
        meta[c["id"]] = (steps, cut)
    r = subprocess.run([exe, "q"], input="\n".join(prog) + "\n", capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit("driver failed:\n" + r.stderr[-3000:])
    runs = parse_driver(r.stdout)
    count = {"exact": 0, "known-difference": 0, "not-implemented": 0, "fail": 0}
    why, fails = {}, []
    for c in cases:
        o = oracle[c["id"]]
        if o.get("excluded"):
            why["excluded"] = why.get("excluded", 0) + 1
            continue
        cls, note = classify(c, o, runs[c["id"]], *meta[c["id"]])
        count[cls] += 1
        if cls in ("known-difference", "not-implemented"):
            why[note] = why.get(note, 0) + 1
        if cls == "fail":
            fails.append((c["id"], note))
    print(json.dumps({"cases": len(cases), "count": count, "reasons": why, "compared_calls": CALLS,
                      "halt_checked": HALT_CHECKED[0]}, ensure_ascii=False))
    for i, n in fails[:15]:
        print("FAIL", i, n[:600])
    return 1 if fails else 0


def cmd_derive(args):
    """Accounting-only variant of each Q2 case: the same world and script without the reopen / halt / current /
    isHalted / hasCard calls (ids get an '-acct' suffix), run through the TS oracle so every remaining call is an M8
    method (an over-limit charge still dispatches halt)."""
    sys.path.insert(0, BUD)
    from oracle import run_oracle
    cases = []
    for l in open(args.cases):
        c = json.loads(l)
        c["id"] += "-acct"
        c["script"] = [e for e in c["script"] if not (e["ev"] == "call" and e["op"] in OUT_OF_SCOPE)]
        cases.append(c)
    obs = run_oracle(cases)
    with open(os.path.join(WORK, "cases_acct.jsonl"), "w") as f:
        for c in cases:
            f.write(json.dumps(c, ensure_ascii=False) + "\n")
    with open(os.path.join(WORK, "oracle_acct.jsonl"), "w") as f:
        for o in obs:
            f.write(json.dumps(o, ensure_ascii=False) + "\n")
    print("derived %d cases" % len(cases))
    return 0


def instr_retired(cmd, env):
    """Instructions retired by one run of cmd (`/usr/bin/time -l`, macOS); None when the counter could not be read
    (the report prints 'unavailable', never 0). A failing command still aborts."""
    try:
        r = subprocess.run(["/usr/bin/time", "-l"] + cmd, capture_output=True, text=True, env=env, cwd=HERE)
    except OSError:
        return None
    if r.returncode != 0:
        sys.exit("%s failed:\n%s" % (cmd[0], r.stderr[-2000:]))
    for l in r.stderr.splitlines():
        if "instructions retired" in l:
            try:
                v = int(l.split()[0])
            except ValueError:
                return None
            return v if v > 0 else None  # a zero counter is a failed read, not a measurement
    return None


def calls_of(item, size, reps):
    return (max(1, reps // size) * size) if item == "toolgrow" else reps


TIME_ITEMS = ("charge", "failure", "tooldup", "toolnew", "toolforget")
INSTR_ITEMS = ("charge", "failure", "tooldup", "toolgrow")
CHK_LANES = ["pruv", "hand16", "hand16w", "fxmix", "fxmixw", "oa", "oaw"]  # lanes that print chk lines


def cmd_perf(args):
    """Wall time: every lane runs once per round and size, and the lane order rotates every round; prints the median ns
    per call of each item. Lanes (LANES): pruv = driver_m8 (i64 op words, frame parsed per call); hand16 / fxmix / oa =
    the three hand implementations with pre-decoded keys (UNMATCHED boundary); hand16w / fxmixw / oaw = the same with
    the matched wire boundary (`perfw`: the driver's i64 op words in packed-u16 form, O(1) canonical key checks and a
    lane unpack into a reused scratch buffer); hand = hand_m8 (old UTF-8 String keys, informational); ts = the TS oracle. Every lane except
    hand and ts also prints `chk <item> <size> <value>` (the full observed final state folded into a u64); the same size
    and reps go to every lane and any chk difference from Pruv aborts the run. Instructions (every lane except ts,
    unless --no-instr): one item per run (the drivers' item argument), retired-instruction difference between --reps
    and 5 x --reps, divided by the extra calls; median of 3. The instruction count does not depend on machine load. A
    counter that could not be read is reported as 'unavailable', never 0. Sizes: --profile S|M|SM (run_meta.PROFILES)
    or --sizes. run_meta(...) is written to work/m8/perf_meta.json."""
    sys.path.insert(0, BUD)
    from common import NODE, TS_ENV
    from run_meta import run_meta, UNAVAILABLE, PROFILES, RUSTC_FLAGS
    exe = build(args)
    hand = build_hand("hand_m8")
    hand16 = build_hand("hand_m8_u16")
    fx = build_hand("hand_m8_fx")
    oa = build_hand("hand_m8_oa")
    env = dict(os.environ)
    env.update(TS_ENV())
    ts = [NODE, "--experimental-strip-types", "--no-warnings", "--import", os.path.join(BUD, "reference-hooks.mjs"),
          os.path.join(HERE, "perf_ts.mjs")]
    sides = [("pruv", [exe, "perf"]), ("hand16", [hand16, "perf"]), ("hand16w", [hand16, "perfw"]),
             ("fxmix", [fx, "perf"]), ("fxmixw", [fx, "perfw"]), ("oa", [oa, "perf"]), ("oaw", [oa, "perfw"]),
             ("hand", [hand]), ("ts", ts)]
    if args.only:
        keep = set(args.only.split(",")) | {"pruv"}
        sides = [x for x in sides if x[0] in keep]
    lanes = [s for s, _ in sides]
    if args.profile:
        sizes = PROFILES[args.profile]
    else:
        sizes = tuple(int(x) for x in args.sizes.split(","))
    res = {}
    for rd in range(args.rounds):
        k = rd % len(sides)
        order = sides[k:] + sides[:k]
        for size in sizes:
            chks = {}  # side -> {(item, size): value} of this run; every side gets the same size and reps
            for side, cmd in order:
                r = subprocess.run(cmd + [str(size), str(args.reps)], capture_output=True, text=True, env=env,
                                   cwd=HERE)
                if r.returncode != 0:
                    sys.exit("%s perf failed:\n%s" % (side, r.stderr[-2000:]))
                for l in r.stdout.splitlines():
                    f = l.split()
                    if f[0] == "chk":  # chk <item> <size> <value>: full final-state checksum (every lane but hand, ts)
                        chks.setdefault(side, {})[(f[1], int(f[2]))] = f[3]
                        continue
                    item, sz, ns = f
                    res.setdefault((item, int(sz)), {}).setdefault(side, []).append(float(ns))
            # the full observed final state must be identical between Pruv and every hand lane
            a = chks.get("pruv", {})
            if not a:
                sys.exit("CHK MISMATCH (round %d, size %d, reps %d): pruv printed no chk" % (rd, size, args.reps))
            for side in [l for l in CHK_LANES[1:] if l in lanes]:
                b = chks.get(side, {})
                if a.keys() != b.keys():
                    sys.exit("CHK MISMATCH (round %d, size %d, reps %d): item sets differ: pruv %r %s %r"
                             % (rd, size, args.reps, sorted(a), side, sorted(b)))
                for k in a:
                    if a[k] != b[k]:
                        sys.exit("CHK MISMATCH (round %d, item %s, size %d, reps %d): pruv %s != %s %s"
                                 % (rd, k[0], k[1], args.reps, a[k], side, b[k]))
            CHK_OK[0] += len(a)
    ins = {}  # (item, size) -> lane -> instructions per call, or None (unavailable)
    if not args.no_instr:
        r1, r2 = args.reps, 5 * args.reps
        for size in sizes:
            for item in INSTR_ITEMS:
                for side, cmd in sides:
                    if side == "ts":
                        continue
                    d = []
                    for _ in range(3):
                        lo = instr_retired(cmd + [str(size), str(r1), item], env)
                        hi = instr_retired(cmd + [str(size), str(r2), item], env)
                        if lo is None or hi is None:
                            d = None
                            break
                        d.append((hi - lo) / (calls_of(item, size, r2) - calls_of(item, size, r1)))
                    ins.setdefault((item, size), {})[side] = statistics.median(d) if d else None

    def fmt(x, f):
        return UNAVAILABLE if x is None else f % x

    def ratio(a, b):
        return UNAVAILABLE if a is None or b is None or b <= 0 else "%.2f" % (a / b)

    print("chk: %d (item, size, round) checksums equal between Pruv and each of %s (hand / TS print no chk)"
          % (CHK_OK[0], ", ".join(l for l in CHK_LANES[1:] if l in lanes)))
    print("rounds %d, reps %d, sizes %s; Pruv/lane = Pruv / lane (>1.0x: Pruv is slower); w = matched wire boundary"
          % (args.rounds, args.reps, ",".join(map(str, sizes))))
    print("ns per call (median of rounds)")
    print("item | size | " + " | ".join(l + " ns" for l in lanes) + " | " + " | ".join("Pruv/" + l for l in lanes[1:]))
    ns_rows = {}
    for item in TIME_ITEMS:
        for sz in sizes:
            m = {k: statistics.median(x) for k, x in res[(item, sz)].items()}
            ns_rows[(item, sz)] = m
            print("%s | %d | %s | %s" % (item, sz, " | ".join(fmt(m.get(l), "%.1f") for l in lanes),
                                          " | ".join(ratio(m.get("pruv"), m.get(l)) for l in lanes[1:])))
    dl = [l for l in ["pruv", "hand16", "hand16w", "oaw"] if l in lanes]
    print("ns per call distribution, median [min-max] over rounds (interleaved; overlap = noise)")
    print("item | size | " + " | ".join(dl) + " | pruv max <= lane min")
    for item in TIME_ITEMS:
        for sz in sizes:
            x = res[(item, sz)]
            print("%s | %d | %s | %s" % (item, sz, " | ".join("%.1f [%.1f-%.1f]" % (statistics.median(x[l]), min(x[l]),
                                                                                       max(x[l])) for l in dl),
                                          " ".join("%s:%s" % (l, "below" if max(x["pruv"]) <= min(x[l]) else
                                                              ("above" if min(x["pruv"]) > max(x[l]) else "overlap"))
                                                   for l in dl[1:])))
    if not args.no_mem:
        mem_pass(sizes, args.reps, args.mem_runs, items=INSTR_ITEMS)
    print("retired instructions per call (/usr/bin/time -l, difference of two rep counts)" +
          (" -- skipped (--no-instr)" if args.no_instr else ""))
    print("item | size | " + " | ".join(l + " instr" for l in lanes) + " | " +
          " | ".join("Pruv/" + l for l in lanes[1:]))
    for item in INSTR_ITEMS:
        for sz in sizes:
            i = ins.get((item, sz), {})
            print("%s | %d | %s | %s" % (item, sz, " | ".join(fmt(i.get(l), "%.0f") for l in lanes),
                                          " | ".join(ratio(i.get("pruv"), i.get(l)) for l in lanes[1:])))
    unavail = sorted("%s/%s/%d" % (l, it, sz) for (it, sz), d in ins.items() for l, v in d.items() if v is None)
    if args.no_instr or not ins:
        unavail = ["all (--no-instr)"]
    meta = run_meta(pruvc=None if args.gen else args.pruvc, backend="rust", rustc=rustc(), flags=RUSTC_FLAGS,
                    profile=args.profile or ("custom:" + ",".join(map(str, sizes))), lanes=lanes,
                    extra={"sizes": list(sizes), "rounds": args.rounds, "reps": args.reps,
                           "instr_unavailable": unavail or [],
                           "lane_cmds": {s: [os.path.basename(c) for c in cmd[:2]] for s, cmd in sides[:-1]},
                           "fxmix_extra_flags": HANDS["hand_m8_fx"][1], "chk_lanes": CHK_LANES,
                           "chk_checked": CHK_OK[0], "builds": BUILDS})
    os.makedirs(WORK, exist_ok=True)
    with open(os.path.join(WORK, "perf_meta.json"), "w") as f:
        json.dump(meta, f, indent=2, ensure_ascii=False)
    print("run_meta written to %s" % os.path.join(WORK, "perf_meta.json"))
    return 0


def cmd_perf_typed(args):
    sys.path.insert(0, BUD)
    from run_meta import PROFILES

    sizes = PROFILES[args.profile] if args.profile else tuple(map(int, args.sizes.split(",")))
    if args.reps <= 0 or any(size <= 0 for size in sizes):
        sys.exit("perf requires positive sizes and reps")
    sides = [
        ("typed", [build(args, typed=True), "perf"]),
        ("flat", [build(args), "perf"]),
        ("hand16w", [build_hand("hand_m8_u16"), "perfw"]),
    ]
    samples = {}
    for round_ in range(5):
        index = round_ % len(sides)
        for size in sizes:
            checksums = {}
            for lane, cmd in sides[index:] + sides[:index]:
                run = subprocess.run(cmd + [str(size), str(args.reps)], capture_output=True, text=True, cwd=HERE)
                if run.returncode:
                    sys.exit("%s perf failed:\n%s" % (lane, run.stderr[-2000:]))
                checksums[lane] = {}
                for line in run.stdout.splitlines():
                    fields = line.split()
                    if fields[0] == "chk":
                        _, item, n, value = fields
                        checksums[lane][item, int(n)] = value
                    else:
                        item, n, ns = fields
                        samples.setdefault((item, int(n), lane), []).append(float(ns))
            expected = {(item, size) for item in TIME_ITEMS}
            if (checksums["flat"].keys() != expected or checksums["typed"] != checksums["flat"]
                    or checksums["hand16w"] != checksums["flat"]):
                sys.exit("perf checksum mismatch at size %d, round %d: %r" % (size, round_ + 1, checksums))
    os.makedirs(WORK, exist_ok=True)
    raw = os.path.join(WORK, "perf_typed_samples.json")
    with open(raw, "w") as f:
        json.dump([{"item": item, "size": n, "lane": lane, "ns": values}
                   for (item, n, lane), values in sorted(samples.items())], f, indent=1)
    print("raw samples: %s" % raw)
    print("Median ns/call over 5 rounds, %d reps, packed wire input; all checksums matched" % args.reps)
    print("item | size | typed | flat | hand16w")
    for size in sizes:
        for item in TIME_ITEMS:
            values = [statistics.median(samples[item, size, lane]) for lane, _ in sides]
            print("%s | %d | %.1f | %.1f | %.1f" % (item, size, *values))
    return 0


def cmd_churn_typed(args):
    exe = build(args, typed=True)
    return subprocess.run([exe, "churn"], cwd=HERE).returncode


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", choices=["q", "perf", "churn", "derive", "controls"])
    ap.add_argument("--fixtures", help="q: run these fixture groups (e.g. C,D) through the oracle instead of --cases")
    ap.add_argument("--typed", action="store_true", help="q, perf or churn: use resident typed state")
    ap.add_argument("--py", action="store_true", help="--typed: use the Python compiler Rust backend")
    ap.add_argument("--lane", choices=["direct", "adapter"], default="direct",
                    help="--typed: account codes through the direct entries or the test-only t_call adapter")
    ap.add_argument("--pruvc", default=PRUVC_DEFAULT)
    ap.add_argument("--pypruvc", type=os.path.abspath, default=PYPRUVC_DEFAULT,
                    help="--typed --py: path to pruvc.py")
    ap.add_argument("--gen")
    ap.add_argument("--cases", default=os.path.join(HERE, "cases_q2.jsonl"))
    ap.add_argument("--oracle", default=os.path.join(HERE, "oracle_q2.jsonl"))
    ap.add_argument("--rounds", type=int, default=7)
    ap.add_argument("--no-instr", action="store_true")
    ap.add_argument("--hand16", action="store_true", help="q: run hand_m8_u16 instead of the Pruv driver")
    ap.add_argument("--hand", help="q: run this hand baseline (hand_m8_u16 | hand_m8_fx | hand_m8_oa, or hand16 | "
                    "fxmix | oa) instead of the Pruv driver")
    ap.add_argument("--sizes", default="100,500", help="perf: comma-separated session counts")
    ap.add_argument("--profile", choices=sorted(["S", "M", "SM"]), help="perf: size profile S=100 M=500 SM=100,500 "
                    "(run_meta.PROFILES); overrides --sizes")
    ap.add_argument("--reps", type=int, default=200000)
    ap.add_argument("--no-mem", action="store_true", help="perf: skip the peak RSS / allocation count pass")
    ap.add_argument("--mem-runs", type=int, default=5)
    ap.add_argument("--only", help="perf: comma-separated lanes to time (default all); pruv is always kept")
    a = ap.parse_args()
    if a.py and not a.typed:
        ap.error("--py requires --typed")
    typed_dispatch = {"q": cmd_q, "perf": cmd_perf_typed, "churn": cmd_churn_typed}
    if a.cmd == "churn" and not a.typed:
        ap.error("churn requires --typed")
    if a.typed and (a.cmd not in typed_dispatch or a.hand16 or a.hand):
        ap.error("--typed requires q, perf or churn and cannot use --hand or --hand16")
    dispatch = {"q": cmd_q, "perf": cmd_perf, "derive": cmd_derive, "controls": cmd_controls}
    if a.typed:
        os.environ["M8_LANE"] = a.lane
        global BUILD
        with tempfile.TemporaryDirectory(prefix=".m8-typed-", dir=HERE) as build_dir:
            BUILD = build_dir
            env_tmp = os.environ.get("TMPDIR")
            os.environ["TMPDIR"] = build_dir
            try:
                rc = typed_dispatch[a.cmd](a)
            finally:
                if env_tmp is None:
                    os.environ.pop("TMPDIR")
                else:
                    os.environ["TMPDIR"] = env_tmp
        sys.exit(rc)
    sys.exit(dispatch[a.cmd](a))


if __name__ == "__main__":
    main()
