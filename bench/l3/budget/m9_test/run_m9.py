#!/usr/bin/env python3
"""L3-10 M9 runner: clock paths of the request budget (bud_clock.pruv) against the TS oracle.

  python3 bench/l3/budget/m9_test/run_m9.py q    [--pruvc PATH | --gen GEN.rs] [--cases F] [--oracle F]
  python3 bench/l3/budget/m9_test/run_m9.py derive  (writes work/m9/cases_clock.jsonl + oracle_clock.jsonl)
  python3 bench/l3/budget/m9_test/run_m9.py perf [--pruvc PATH | --gen GEN.rs] [--rounds N]

q: converts each Q3 case (work/m9/cases.jsonl) into the driver program, runs it, and compares with the oracle output
(work/m9/oracle.jsonl). Compared: the timer event sequence (set token + requested delay, clear token, fire token,
halt), then for cases that run to the end the final budget clock fields (activeMs, activeSince, state), the clocks
(owner, runs in order with waiting, since, timer token) and the active timers (token, delay due).
Classes:
  exact           whole case ran and everything matched
  not-implemented the case reaches a path owned by another milestone: a raise/charge call (M5/M6 charge, M7
                  lifecycle) or a time halt (M10). The event prefix before that point is compared and must match;
                  the rest is not counted. Nothing after the cut is filled by host code.
  known-difference KD-M9-1: a captured delivery of a timer the host already cleared or fired. TS runs the stale
                  callback; the Pruv host drops it. The prefix before it must match.
  fail            anything else.
"""
import argparse, datetime, json, os, shutil, statistics, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
BUD = os.path.dirname(HERE)
ROOT = os.path.abspath(os.path.join(BUD, "..", "..", ".."))
WORK = os.path.join(BUD, "work", "m9")
PRUVC_DEFAULT = os.path.join(ROOT, "pruvc-rs", "target", "release", "pruvc")
LIBS = ["--lib", "bench/l3/budget", "--lib", "bench/l3/lib"]
STATE = {"open": 0, "halted": 1, "closed": 2}
HALT_MARK = ("runningDelegationIds", "cancelRecovery", "stopMain")
COMPARED = {}


def rustc():
    return shutil.which("rustc") or os.path.expanduser("~/.cargo/bin/rustc")


def build(args):
    os.makedirs(WORK, exist_ok=True)
    gen = os.path.join(WORK, "gen.rs")
    if args.gen:
        shutil.copyfile(args.gen, gen)
    else:
        c = subprocess.run([args.pruvc, "bench/l3/budget/m9_test/root_m9.pruv", "-o", gen] + LIBS, cwd=ROOT,
                           capture_output=True, text=True)
        if c.returncode != 0:
            sys.exit("pruvc failed:\n" + c.stdout[-2000:] + c.stderr[-2000:])
    main = os.path.join(WORK, "main_m9.rs")
    with open(main, "w") as f:
        f.write('#![allow(dead_code, unused_imports, unused_variables, unused_mut)]\n#[path = "gen.rs"] mod sol;\n')
        f.write(open(os.path.join(HERE, "driver_m9.rs")).read())
    exe = os.path.join(WORK, "driver_m9")
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


def key(s):
    u = units(s)
    return [len(u)] + u


def obj(v):
    return dict(v["obj"]) if isinstance(v, dict) and "obj" in v else None


def iso(ms):
    d = datetime.datetime(1970, 1, 1) + datetime.timedelta(milliseconds=ms)
    return d.strftime("%Y-%m-%dT%H:%M:%S.") + "%03dZ" % (ms % 1000)


def truthy(v):
    return not (v is None or v is False or v == 0 or v == "" or (isinstance(v, dict) and "undef" in v))


def budget_rec(b):
    """PLACEHOLDER record words (root_m9.pruv) for a session's budget value."""
    if b is None:
        return [-1, 0, 0, 0, 0, 0, 0]
    lim = obj(b["limits"])
    used = obj(b["used"])
    bid = units(b["id"])
    kind = 1 if "activeSince" in b else 0
    return [STATE[b["state"]], 1 if truthy(b.get("unlimited")) else 0, lim["timeMinutes"], used["activeMs"], kind, 0,
            len(bid)] + bid


# ---- driver program ----
def program(case):
    """Returns (lines, cut) where cut is None or ('call', req) for the first raise/charge call."""
    out = ["case " + case["id"]]
    for s in case["world"]["sessions"]:
        d = obj(s)
        rec = budget_rec(obj(d.get("budget")) if d.get("budget") is not None else None)
        out.append("put " + " ".join(map(str, key(d["id"]) + [len(rec)] + rec)))
    out.append("now 1700000000000")
    cut = None
    for i, e in enumerate(case["script"]):
        out.append("step %d" % i)
        ev = e["ev"]
        if ev == "advance":
            out.append("adv %d" % e["ms"])
        elif ev == "setTime":
            out.append("settime %d" % e["now"])
        elif ev == "deliverTimer":
            out.append("deliver %s %d" % (e["token"], 1 if e.get("captured") else 0))
        elif ev == "call":
            op = e["op"]
            if op not in ("runStarted", "runWaiting", "runEnded"):
                cut = ("call", e["req"])
                break
            r = obj(e["args"][0])
            code = {"runStarted": 1, "runWaiting": 2, "runEnded": 3}[op]
            flag = 1 if (op == "runWaiting" and e["args"][2]) else 0
            w = [code] + key(r["sessionId"]) + key(r["budgetId"]) + key(e["args"][1]) + [flag]
            out.append("call " + " ".join(map(str, w)))
        else:
            raise ValueError("unknown event " + ev)
    out.append("end")
    return out, cut


def parse_driver(text):
    res = {}
    cur = None
    for line in text.splitlines():
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


def oracle_events(o, cut):
    """Flat event list from the oracle trace, up to the cut call."""
    ev = []
    halted = False
    for t in o["trace"]:
        c = t["c"]
        if cut and c == "call.start" and t["r"] == cut[1]:
            break
        a = obj(t.get("a")) if isinstance(t.get("a"), dict) else None
        if c == "timer.set":
            ev.append("S %s %d" % (a["token"], a["delay"]))
        elif c == "timer.clear":
            ev.append("C %s" % a["token"])
        elif c == "timer.fire":
            ev.append("F %s" % a["token"])
        elif c in HALT_MARK and not halted:
            ev.append("H")
            halted = True
    return ev


def parse_obs(line):
    body, _, tl = line[1:].partition("| T")
    w = list(map(int, body.split()))
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
        b = rkey()
        sessions[k] = list(b)
    clocks = []
    for _ in range(rd()):
        k = rkey()
        owner = rkey()
        sp, since, armed, dl = rd(), rd(), rd(), rd()
        runs = []
        for _ in range(rd()):
            rk = rkey()
            runs.append((rk, rd()))
        clocks.append((k, owner, sp, since, armed, dl, runs))
    timers = []
    for t in tl.split():
        tok, due, bid = t.split(":")
        timers.append((tok, int(due), tuple(int(x) for x in bid.split(",") if x)))
    return sessions, clocks, timers


def compare_final(case, o, obs):
    sessions, clocks, timers = parse_obs(obs)
    diffs = []
    for s in o["finalSessions"]:
        d = obj(s["v"])
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
        used = obj(b["used"])
        if mine[0] != STATE[b["state"]]:
            diffs.append("state %s %s" % (mine[0], b["state"]))
        if mine[3] != used["activeMs"]:
            diffs.append("activeMs %s %s" % (mine[3], used["activeMs"]))
        want = b.get("activeSince")
        if mine[4] == 0:
            got = None
        elif mine[4] == 1:
            init = [obj(x) for x in case["world"]["sessions"] if tuple(units(obj(x)["id"])) == sk][0]
            got = obj(init["budget"]).get("activeSince")
        else:
            got = iso(mine[5])
        if want is not None:
            want = "".join(chr(u) for u in units(want)) if isinstance(want, dict) else want
        if got != want:
            diffs.append("activeSince %r %r" % (got, want))
    tok_of = {t[2]: t[0] for t in timers}
    want_clocks = []
    for bid, cv in o["finalClocks"].get("A", []):
        c = obj(cv)
        runs = [(tuple(units(rk)), 1 if obj(rv)["waiting"] else 0) for rk, rv in c["runs"]]
        since = c.get("since")
        since = None if (since is None or (isinstance(since, dict) and "undef" in since)) else since
        tm = c.get("timer")
        tm = None if (tm is None or (isinstance(tm, dict) and "undef" in tm)) else tm
        want_clocks.append((tuple(units(bid)), tuple(units(c["sessionId"])), since, tm, runs))
    got_clocks = []
    for k, owner, sp, since, armed, dl, runs in clocks:
        got_clocks.append((k, owner, since if sp else None, tok_of.get(k) if armed else None, runs))
    if got_clocks != want_clocks:
        diffs.append("clocks %r != %r" % (got_clocks, want_clocks))
    want_t = sorted((t["token"], t["due"]) for t in o["activeTimers"])
    got_t = sorted((t[0], t[1]) for t in timers)
    if want_t != got_t:
        diffs.append("timers %r != %r" % (got_t, want_t))
    return diffs


def classify(case, o, run, cut):
    mine = [l for l in run["log"] if not l.startswith("@")]
    drop = [i for i, l in enumerate(mine) if l.startswith("D ")]
    xs = [l for l in mine if l.startswith("X")]
    if xs:
        return "fail", "driver status %s" % xs[0]
    want = oracle_events(o, cut)
    # captured stale deliveries: KD-M9-1 when the oracle runs that callback; a dropped non-captured delivery is a
    # no-op on both sides.
    for i in drop:
        tok = mine[i][2:]
        pre = [l for l in mine[:i] if not l.startswith("D ")]
        if want[:len(pre)] == pre and len(want) > len(pre) and want[len(pre)] == "F " + tok:
            return "known-difference", "KD-M9-1 at %s" % tok
    mine = [l for l in mine if not l.startswith("D ")]
    for l in mine[:mine.index("H") + 1] if "H" in mine else mine:
        COMPARED[l[0]] = COMPARED.get(l[0], 0) + 1
    if "H" in mine:
        h = mine.index("H")
        if want[:h + 1] == mine[:h + 1]:
            return "not-implemented", "time halt (M10)"
        return "fail", "events %r != %r" % (mine, want[:h + 2])
    if mine != want:
        return "fail", "events %r != %r" % (mine, want)
    if cut:
        return "not-implemented", "raise/charge call %s" % cut[1]
    d = compare_final(case, o, run["obs"])
    if d:
        return "fail", "; ".join(d)
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
    why = {}
    fails = []
    for c in cases:
        cls, note = classify(c, oracle[c["id"]], runs[c["id"]], cuts[c["id"]])
        count[cls] += 1
        if cls in ("known-difference", "not-implemented"):
            k = note.split(" r")[0].split(" at ")[0] if cls == "not-implemented" else "KD-M9-1"
            k = "raise/charge call" if note.startswith("raise/charge") else k
            why[k] = why.get(k, 0) + 1
        if cls == "fail":
            fails.append((c["id"], note))
    print(json.dumps({"cases": len(cases), "count": count, "reasons": why, "compared_events": COMPARED}, ensure_ascii=False))
    for i, n in fails[:15]:
        print("FAIL", i, n[:600])
    return 1 if fails else 0


def cmd_derive(args):
    """Clock-only variant of each Q3 case: the same world and script without the raise/charge calls (ids get a
    '-clock' suffix), run through the TS oracle so every remaining step is a clock path."""
    sys.path.insert(0, BUD)
    from oracle import run_oracle
    cases = []
    for l in open(args.cases):
        c = json.loads(l)
        c["id"] += "-clock"
        c["script"] = [e for e in c["script"] if not (e["ev"] == "call" and e["op"] in ("raise", "charge"))]
        cases.append(c)
    obs = run_oracle(cases)
    with open(os.path.join(WORK, "cases_clock.jsonl"), "w") as f:
        for c in cases:
            f.write(json.dumps(c, ensure_ascii=False) + "\n")
    with open(os.path.join(WORK, "oracle_clock.jsonl"), "w") as f:
        for o in obs:
            f.write(json.dumps(o, ensure_ascii=False) + "\n")
    print("derived %d cases" % len(cases))
    return 0


def cmd_perf(args):
    """Alternates Pruv / hand Rust / TS runs per round and size; prints the median ns per call of each item."""
    sys.path.insert(0, BUD)
    from common import NODE, TS_ENV
    exe = build(args)
    hand = os.path.join(WORK, "hand_m9")
    c = subprocess.run([rustc(), "-C", "opt-level=3", "--edition", "2021", "-o", hand, os.path.join(HERE, "hand_m9.rs")],
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
        for size in (10, 50, 100):
            for side, cmd in sides.items():
                r = subprocess.run(cmd + [str(size), str(args.reps)], capture_output=True, text=True, env=env,
                                   cwd=HERE)
                if r.returncode != 0:
                    sys.exit("%s perf failed:\n%s" % (side, r.stderr[-2000:]))
                for l in r.stdout.splitlines():
                    item, sz, ns = l.split()
                    if item != "refresh" and size != 10:
                        continue
                    res.setdefault((item, int(sz)), {}).setdefault(side, []).append(float(ns))
    print("item | size | Pruv ns | hand Rust ns | TS ns | Pruv/hand")
    for (item, sz), v in sorted(res.items()):
        m = {k: statistics.median(x) for k, x in v.items()}
        print("%s | %d | %.1f | %.1f | %.1f | %.2f" % (item, sz, m["pruv"], m["hand"], m["ts"], m["pruv"] / m["hand"]))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", choices=["q", "perf", "derive"])
    ap.add_argument("--pruvc", default=PRUVC_DEFAULT)
    ap.add_argument("--gen")
    ap.add_argument("--cases", default=os.path.join(WORK, "cases.jsonl"))
    ap.add_argument("--oracle", default=os.path.join(WORK, "oracle.jsonl"))
    ap.add_argument("--rounds", type=int, default=5)
    ap.add_argument("--reps", type=int, default=200000)
    a = ap.parse_args()
    sys.exit({"q": cmd_q, "perf": cmd_perf, "derive": cmd_derive}[a.cmd](a))


if __name__ == "__main__":
    main()
