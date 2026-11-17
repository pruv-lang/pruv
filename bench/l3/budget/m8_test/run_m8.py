#!/usr/bin/env python3
"""L3-10 M8 runner: activity accounting of the request budget (bud_charge.pruv, bud_tools.pruv) against the TS oracle.

  python3 bench/l3/budget/m8_test/run_m8.py q    [--pruvc PATH | --gen GEN.rs] [--cases F] [--oracle F] [--hand16]
  python3 bench/l3/budget/m8_test/run_m8.py derive  (writes work/m8/cases_acct.jsonl + oracle_acct.jsonl)
  python3 bench/l3/budget/m8_test/run_m8.py perf [--pruvc PATH | --gen GEN.rs] [--rounds N] [--reps N] [--no-instr] [--sizes 100,500,1000]

q: converts each Q2 case (work/m8/cases.jsonl) into the driver program, runs it, and compares with the oracle output
(work/m8/oracle.jsonl). Compared per call: the return value of every charge ({ok} / {ok:false, reason}) and that the
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
import argparse, json, os, shutil, statistics, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
BUD = os.path.dirname(HERE)
ROOT = os.path.abspath(os.path.join(BUD, "..", "..", ".."))
WORK = os.path.join(BUD, "work", "m8")
PRUVC_DEFAULT = "/Users/spica4345/Projects/pruv/pruvc-rs/target/release/pruvc"
LIBS = ["--lib", "bench/l3/budget", "--lib", "bench/l3/lib"]
STATE = {"open": 0, "halted": 1, "closed": 2}
REASONS = ["time", "delegations", "commands", "reviews", "checks"]
KEYS = ["delegations", "commands", "reviews", "checks"]
OWN = {"charge": 1, "noteFailure": 2, "noteTool": 3, "forgetTurn": 4}
OUT_OF_SCOPE = ("reopen", "halt", "current", "isHalted", "hasCard")
HALT_CHECKED = [0]
CHK_OK = [0]
CALLS = {}


def rustc():
    return shutil.which("rustc") or os.path.expanduser("~/.cargo/bin/rustc")


def build(args):
    os.makedirs(WORK, exist_ok=True)
    gen = os.path.join(WORK, "gen.rs")
    if args.gen:
        shutil.copyfile(args.gen, gen)
    else:
        c = subprocess.run([args.pruvc, "bench/l3/budget/m8_test/root_m8.pruv", "-o", gen] + LIBS, cwd=ROOT,
                           capture_output=True, text=True)
        if c.returncode != 0:
            sys.exit("pruvc failed:\n" + c.stdout[-2000:] + c.stderr[-2000:])
    main = os.path.join(WORK, "main_m8.rs")
    with open(main, "w") as f:
        f.write('#![allow(dead_code, unused_imports, unused_variables, unused_mut)]\n#[path = "gen.rs"] mod sol;\n')
        f.write(open(os.path.join(HERE, "driver_m8.rs")).read())
    exe = os.path.join(WORK, "driver_m8")
    c = subprocess.run([rustc(), "-C", "opt-level=3", "--edition", "2021", "-o", exe, main], capture_output=True,
                       text=True)
    if c.returncode != 0:
        sys.exit("rustc failed:\n" + c.stderr[-4000:])
    return exe


def build_hand(name):
    """Compiles bench/l3/budget/m8_test/<name>.rs (a standalone hand-written Rust baseline) into work/m8/<name>."""
    os.makedirs(WORK, exist_ok=True)
    exe = os.path.join(WORK, name)
    c = subprocess.run([rustc(), "-C", "opt-level=3", "--edition", "2021", "-o", exe, os.path.join(HERE, name + ".rs")],
                       capture_output=True, text=True)
    if c.returncode != 0:
        sys.exit("rustc failed (%s):\n%s" % (name, c.stderr[-3000:]))
    return exe


# ---- values ----
def units(s):
    if isinstance(s, dict) and "u16" in s:
        return list(s["u16"])
    b = s.encode("utf-16-le", "surrogatepass")
    return [b[i] | (b[i + 1] << 8) for i in range(0, len(b), 2)]


def key(s):
    u = units(s)
    return [len(u)] + u


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
            + [used[k] for k in KEYS] + [used["tools"], used["failures"], used["retries"], len(bid)] + bid)


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
            # [20, k, sessionId key, budgetId key]
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


def cmd_q(args):
    # --hand16: run the hand-written UTF-16 Rust baseline (hand_m8_u16.rs q mode) instead of the Pruv driver; the
    # comparison / classification below is the same, so mismatches vs the oracle are counted the same way.
    exe = build_hand("hand_m8_u16") if args.hand16 else build(args)
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
    """Instructions retired by one run of cmd (`/usr/bin/time -l`, macOS)."""
    r = subprocess.run(["/usr/bin/time", "-l"] + cmd, capture_output=True, text=True, env=env, cwd=HERE)
    if r.returncode != 0:
        sys.exit("%s failed:\n%s" % (cmd[0], r.stderr[-2000:]))
    for l in r.stderr.splitlines():
        if "instructions retired" in l:
            return int(l.split()[0])
    sys.exit("no instruction count from /usr/bin/time -l")


def calls_of(item, size, reps):
    return (max(1, reps // size) * size) if item == "toolgrow" else reps


ITEMS = ("charge", "failure", "tooldup", "toolgrow")


def cmd_perf(args):
    """Wall time: Pruv / hand Rust / TS runs alternate per round and size, and the side order rotates every round;
    prints the median ns per call of each item. Pruv and hand16 also print `chk <item> <size> <value>` (the full
    observed final state folded into a u64); the same size and reps go to every side and a chk difference between
    Pruv and hand16 aborts the run. Instructions (Pruv and hand, unless --no-instr): one item per run
    (the drivers' item argument), retired-instruction difference between --reps and 5 x --reps, divided by the extra
    calls; median of 3. The instruction count does not depend on machine load."""
    sys.path.insert(0, BUD)
    from common import NODE, TS_ENV
    exe = build(args)
    hand = build_hand("hand_m8")
    hand16 = build_hand("hand_m8_u16")
    env = dict(os.environ)
    env.update(TS_ENV())
    ts = [NODE, "--experimental-strip-types", "--no-warnings", "--import", os.path.join(BUD, "reference-hooks.mjs"),
          os.path.join(HERE, "perf_ts.mjs")]
    sides = [("pruv", [exe, "perf"]), ("hand16", [hand16, "perf"]), ("hand", [hand]), ("ts", ts)]
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
                    if f[0] == "chk":  # chk <item> <size> <value>: full final-state checksum (pruv, hand16 only)
                        chks.setdefault(side, {})[(f[1], int(f[2]))] = f[3]
                        continue
                    item, sz, ns = f
                    res.setdefault((item, int(sz)), {}).setdefault(side, []).append(float(ns))
            # the full observed final state must be identical between Pruv and the hand UTF-16 baseline
            a, b = chks.get("pruv", {}), chks.get("hand16", {})
            if not a or a.keys() != b.keys():
                sys.exit("CHK MISMATCH (round %d, size %d, reps %d): item sets differ: pruv %r hand16 %r"
                         % (rd, size, args.reps, sorted(a), sorted(b)))
            for k in a:
                if a[k] != b[k]:
                    sys.exit("CHK MISMATCH (round %d, item %s, size %d, reps %d): pruv %s != hand16 %s"
                             % (rd, k[0], k[1], args.reps, a[k], b[k]))
            CHK_OK[0] += len(a)
    ins = {}
    if not args.no_instr:
        r1, r2 = args.reps, 5 * args.reps
        for size in sizes:
            for item in ITEMS:
                for side, cmd in sides[:3]:  # pruv, hand16, hand
                    d = []
                    for _ in range(3):
                        lo = instr_retired(cmd + [str(size), str(r1), item], env)
                        hi = instr_retired(cmd + [str(size), str(r2), item], env)
                        d.append((hi - lo) / (calls_of(item, size, r2) - calls_of(item, size, r1)))
                    ins.setdefault((item, size), {})[side] = statistics.median(d)
    print("chk: %d (item, size, round) checksums equal between Pruv and hand16 (hand / TS print no chk)" % CHK_OK[0])
    print("item | size | Pruv ns | hand16 ns | hand(old) ns | TS ns | Pruv/hand16 (bar) | Pruv/hand (info) | Pruv/TS"
          " | Pruv instr | hand16 instr | hand instr | instr Pruv/hand16 | instr Pruv/hand")
    for item in ITEMS:
        for sz in sizes:
            m = {k: statistics.median(x) for k, x in res[(item, sz)].items()}
            i = ins.get((item, sz), {})
            ir = ("%.0f | %.0f | %.0f | %.2f | %.2f" % (i["pruv"], i["hand16"], i["hand"], i["pruv"] / i["hand16"],
                                                        i["pruv"] / i["hand"])) if i else "- | - | - | - | -"
            print("%s | %d | %.1f | %.1f | %.1f | %.1f | %.2f | %.2f | %.2f | %s" % (
                item, sz, m["pruv"], m["hand16"], m["hand"], m["ts"], m["pruv"] / m["hand16"], m["pruv"] / m["hand"],
                m["pruv"] / m["ts"], ir))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", choices=["q", "perf", "derive"])
    ap.add_argument("--pruvc", default=PRUVC_DEFAULT)
    ap.add_argument("--gen")
    ap.add_argument("--cases", default=os.path.join(WORK, "cases.jsonl"))
    ap.add_argument("--oracle", default=os.path.join(WORK, "oracle.jsonl"))
    ap.add_argument("--rounds", type=int, default=7)
    ap.add_argument("--no-instr", action="store_true")
    ap.add_argument("--hand16", action="store_true", help="q: run hand_m8_u16 instead of the Pruv driver")
    ap.add_argument("--sizes", default="100,500,1000", help="perf: comma-separated session counts")
    ap.add_argument("--reps", type=int, default=200000)
    a = ap.parse_args()
    sys.exit({"q": cmd_q, "perf": cmd_perf, "derive": cmd_derive}[a.cmd](a))


if __name__ == "__main__":
    main()
