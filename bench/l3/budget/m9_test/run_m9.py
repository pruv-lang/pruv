#!/usr/bin/env python3
"""L3-10 M9 runner: clock paths of the request budget (bud_clock.pruv) against the TS oracle.

  python3 bench/l3/budget/m9_test/run_m9.py q    [--typed [--py]] [--pruvc PATH | --gen GEN.rs] [--cases F] [--oracle F]
  python3 bench/l3/budget/m9_test/run_m9.py derive [--family q3|q4] [--cases F]
  python3 bench/l3/budget/m9_test/run_m9.py dc [--typed [--py]] [--count N] [--seed S]
  python3 bench/l3/budget/m9_test/run_m9.py perf [--typed [--py]] [--pruvc PATH | --gen GEN.rs] [--rounds N]
  python3 bench/l3/budget/m9_test/run_m9.py churn --typed [--py]

q: converts each Q3 case (work/m9/cases.jsonl) into the driver program, runs it, and compares with the oracle output
(work/m9/oracle.jsonl). Compared: the timer event sequence (set token + requested delay, clear token, fire token,
halt), then for cases that run to the end the final budget clock fields (activeMs, activeSince, state), the clocks
(owner, runs in order with waiting, since, timer token) and the active timers (token, delay due).
Classes:
  exact           whole case ran and everything matched
  not-implemented the case reaches a non-run call (e.g. charge, raise or lifecycle) or a time halt (M10).
                  The event prefix before that point must match;
                  at a time halt the halt instant and the halted session's activeMs / activeSince are compared too
                  (when no raise/charge call follows, the oracle's final values of that budget are the ones at the
                  halt: later clock-only steps cannot change a halted budget). The rest is not counted. Nothing
                  after the cut is filled by host code.
  known-difference KD-M6-1: the oracle holds a number outside the safe integral range (JS rounded it); excluded
                  domain (spec §8.4), not compared.
  fail            anything else.

dc checks --count randomized DROP/CLOSE cases plus one 20-cycle reclamation case for each run count from 0 to 12.
"""
import argparse, datetime, json, os, random, re, shutil, statistics, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
BUD = os.path.dirname(HERE)
ROOT = os.path.abspath(os.path.join(BUD, "..", "..", ".."))
WORK = os.path.join(BUD, "work", "m9")
TYPED_WORK = os.path.join(HERE, "target", "m9")
BUDLIB_WORK = os.path.join(WORK, "budlib")
PRUVC_DEFAULT = os.path.join(ROOT, "pruvc-rs", "target", "release", "pruvc")
PRUVC_PY_DEFAULT = os.path.join(ROOT, "pruvc", "pruvc.py")
STATE = {"open": 0, "halted": 1, "closed": 2}
RUN_OPS = {"runStarted": 1, "runWaiting": 2, "runEnded": 3}
HALT_MARK = ("runningDelegationIds", "cancelRecovery", "stopMain")


def rustc():
    return shutil.which("rustc") or os.path.expanduser("~/.cargo/bin/rustc")


def shared_checkout():
    if not os.path.exists(os.path.join(ROOT, ".git")) or not shutil.which("git"):
        return ROOT
    result = subprocess.run(["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
                            cwd=ROOT, capture_output=True, text=True)
    return os.path.dirname(result.stdout.strip()) if result.returncode == 0 else ROOT


def compiler_path(explicit, default):
    if explicit is not None:
        return explicit
    if os.path.isfile(default):
        return default
    return os.path.join(shared_checkout(), os.path.relpath(default, ROOT))


def compiler(args, root, gen, typed):
    if typed and args.py:
        python = os.path.join(shared_checkout(), ".venv", "bin", "python")
        script = compiler_path(args.pruvc, PRUVC_PY_DEFAULT)
        return [python if os.path.isfile(python) else sys.executable, "-B", script, root, "-o", gen, "--backend", "rust"]
    pruvc = compiler_path(args.pruvc, PRUVC_DEFAULT)
    if typed:
        return [pruvc, root, "-o", gen, "--emit", "rs"]
    return [pruvc, root, "-o", gen, "--lib", args.budlib or "bench/l3/budget", "--lib", "bench/l3/lib"]


def build(args):
    typed = getattr(args, "typed", False)
    if args.out:
        work = os.path.abspath(args.out)
    elif typed:
        work = TYPED_WORK
    elif args.budlib is None:
        work = os.path.abspath(WORK)
    else:
        # a --budlib build without --out gets its own directory, so concurrent campaigns never share binaries (P23 H6)
        os.makedirs(BUDLIB_WORK, exist_ok=True)
        work = tempfile.mkdtemp(prefix="run-", dir=BUDLIB_WORK)
    if typed:
        work = os.path.join(work, "typed_py" if args.py else "typed")
    suffix = "_typed" if typed else ""
    os.makedirs(work, exist_ok=True)
    gen = os.path.join(work, "gen.rs")
    if args.gen:
        if os.path.abspath(args.gen) != gen:
            shutil.copyfile(args.gen, gen)
    else:
        c = subprocess.run(compiler(args, os.path.join(HERE, f"root_m9{suffix}.pruv"), gen, typed),
                           cwd=ROOT, capture_output=True, text=True)
        if c.returncode != 0:
            sys.exit("pruvc failed:\n" + c.stdout[-2000:] + c.stderr[-2000:])
    main = os.path.join(work, "main_m9.rs")
    with open(main, "w") as f:
        f.write('#![allow(dead_code, unused_imports, unused_variables, unused_mut)]\n#[path = "gen.rs"] mod sol;\n')
        driver = json.dumps(os.path.join(HERE, f"driver_m9{suffix}.rs"), ensure_ascii=False)
        f.write(f"include!({driver});\n")
    churn = typed and getattr(args, "cmd", None) == "churn"
    exe = os.path.join(work, "churn_m9" if churn else "driver_m9")
    cfg = ["--cfg", "m9_typed"] if typed else []
    if churn:
        cfg.append("--test")
    c = subprocess.run([rustc(), *cfg, "-C", "opt-level=3", "--edition", "2021", "-o", exe, main], capture_output=True,
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


def key(s):
    """Wire key, packed once here at ingress: `n, words(n) packed words`."""
    u = units(s)
    return [len(u)] + pack(u)


def obj(v):
    return dict(v["obj"]) if isinstance(v, dict) and "obj" in v else None


def iso(ms):
    d = datetime.datetime(1970, 1, 1) + datetime.timedelta(milliseconds=ms)
    return d.strftime("%Y-%m-%dT%H:%M:%S.") + "%03dZ" % (ms % 1000)


def truthy(v):
    return not (v is None or v is False or v == 0 or v == "" or (isinstance(v, dict) and "undef" in v))


def budget_rec(b):
    """PLACEHOLDER record words (root_m9.pruv) for a session's budget value; the budget id is packed (word 6 = its
    unit count, then words(n) packed words)."""
    if b is None:
        return [-1, 0, 0, 0, 0, 0, 0]
    lim = obj(b["limits"])
    used = obj(b["used"])
    kind = 1 if "activeSince" in b else 0
    return [STATE[b["state"]], 1 if truthy(b.get("unlimited")) else 0, lim["timeMinutes"], used["activeMs"], kind, 0
            ] + key(b["id"])


# ---- driver program ----
def put_line(s):
    d = obj(s)
    rec = budget_rec(obj(d.get("budget")) if d.get("budget") is not None else None)
    return "put " + " ".join(map(str, key(d["id"]) + [len(rec)] + rec))


def program(case):
    """Return the driver lines and the first unsupported call, if any."""
    out = ["case " + case["id"]]
    for s in case["world"]["sessions"]:
        out.append(put_line(s))
    out.append("now %d" % case["world"].get("clockStart", 1700000000000))
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
        elif ev == "putSession":
            out.append(put_line(e["session"]))
        elif ev == "call":
            op = e["op"]
            if op not in RUN_OPS:
                cut = ("call", e["req"], op)
                break
            r = obj(e["args"][0])
            code = RUN_OPS[op]
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
        sp, since, armed, act = rd(), rd(), rd(), rd()
        runs = []
        for _ in range(rd()):
            rk = rkey()
            runs.append((rk, rd()))
        clocks.append((k, owner, sp, since, armed, act, runs))
    timers = []
    for t in tl.split():
        tok, due, bid, own = t.split(":")
        timers.append((tok, int(due), tuple(int(x) for x in bid.split(",") if x), own == "1"))
    return sessions, clocks, timers


def last_put(case, sk):
    # The session record the driver last stored for sk: the last putSession before the cut (compare_final runs only
    # without a halt, so every such put ran), else the initial world session.
    rec = None
    for x in case["world"]["sessions"]:
        if tuple(units(obj(x)["id"])) == sk:
            rec = obj(x)
    for e in case["script"]:
        if e["ev"] == "call" and e["op"] not in RUN_OPS:
            break
        if e["ev"] == "putSession" and tuple(units(obj(e["session"])["id"])) == sk:
            rec = obj(e["session"])
    return rec


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
            got = obj(last_put(case, sk)["budget"]).get("activeSince")
        else:
            got = iso(mine[5])
        # both sides as text: a lone surrogate arrives as {u16: [...]} from the oracle and from the stored put
        got = text(got) if got is not None else None
        want = text(want) if want is not None else None
        if got != want:
            diffs.append("activeSince %r %r" % (got, want))
    tok_of = {t[2]: t[0] for t in timers if t[3]}
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
    for k, owner, sp, since, armed, act, runs in clocks:
        # field 17 is the active-run count REFRESH reads; it must equal the runs with waiting = 0
        if act != sum(1 for _, wv in runs if wv == 0):
            diffs.append("active count %s for %r" % (act, runs))
        # The delivered timer may already be absent from the scheduler. Check the clock's flag independently.
        timer = tok_of.get(k, "<armed without a pending timer>") if armed else None
        got_clocks.append((k, owner, since if sp else None, timer, runs))
    if got_clocks != want_clocks:
        diffs.append("clocks %r != %r" % (got_clocks, want_clocks))
    want_t = sorted((t["token"], t["due"]) for t in o["activeTimers"])
    got_t = sorted((t[0], t[1]) for t in timers)
    if want_t != got_t:
        diffs.append("timers %r != %r" % (got_t, want_t))
    return diffs


def text(v):
    return "".join(chr(u) for u in units(v)) if isinstance(v, dict) else v


def compare_halt(case, o, obs, h):
    """At a time halt (driver line `H <now> <sessionId units>`) with no raise/charge call after it: the halted
    budget's halt instant, state and settled clock fields equal the oracle's final ones (halt(ref, 'time') settles,
    which adds nothing after FIRE's SETTLE, and sets state / halt.at; later clock-only steps leave a halted budget
    alone)."""
    _, t, su = h.split(" ")
    sk = tuple(int(x) for x in su.split(",") if x)
    sessions, _, _ = parse_obs(obs)
    mine = sessions.get(sk)
    diffs = []
    for sv in o["finalSessions"]:
        d = obj(sv["v"])
        if tuple(units(d["id"])) != sk:
            continue
        b = obj(d["budget"]["v"]) if d.get("budget") is not None else None
        if b is None or mine is None or mine[0] == -1:
            return ["halted budget missing"]
        if b["state"] != "halted":
            diffs.append("oracle state %s" % b["state"])
        at = text(obj(b["halt"])["at"]) if obj(b.get("halt")) else None
        if at != iso(int(t)):
            diffs.append("halt.at %r %r" % (iso(int(t)), at))
        if mine[3] != obj(b["used"])["activeMs"]:
            diffs.append("activeMs %s %s" % (mine[3], obj(b["used"])["activeMs"]))
        if mine[4] != 0 or b.get("activeSince") is not None:
            diffs.append("activeSince kind %s %r" % (mine[4], b.get("activeSince")))
        return diffs
    return ["halted session missing"]


def unsafe_number(v):
    """True if the oracle record holds a number outside the safe integral range (encoded {num: bits})."""
    if isinstance(v, dict):
        return "num" in v or any(unsafe_number(x) for x in v.values())
    if isinstance(v, list):
        return any(unsafe_number(x) for x in v)
    return False


def put_after_call(case, sk):
    """True if the script stores session sk again after its first call (the oracle's final state then is not the
    one at a halt during the calls)."""
    called = False
    for e in case["script"]:
        if e["ev"] == "call":
            called = True
        elif called and e["ev"] == "putSession" and tuple(units(obj(e["session"])["id"])) == sk:
            return True
    return False


def classify(case, o, run, cut, compared, halt_checked):
    if unsafe_number(o):
        # KD-M6-1: JS rounds arithmetic that leaves the safe range; Pruv keeps exact i64 (requested delay) or
        # saturates (activeMs). Spec §8.4 makes it excluded-domain, so it is not compared.
        return "known-difference", "KD-M6-1 unsafe arithmetic"
    mine = [l for l in run["log"] if not l.startswith(("@", "I2 "))]
    xs = [l for l in mine if l.startswith("X")]
    if xs:
        return "fail", "driver status %s" % xs[0]
    want = oracle_events(o, cut)
    ev = ["H" if l.startswith("H") else l for l in mine]
    for l in ev[:ev.index("H") + 1] if "H" in ev else ev:
        compared[l[0]] = compared.get(l[0], 0) + 1
    if "H" in ev:
        h = ev.index("H")
        if want[:h + 1] != ev[:h + 1]:
            return "fail", "events %r != %r" % (ev, want[:h + 2])
        if not cut and not put_after_call(case, tuple(int(x) for x in mine[h].split(" ")[2].split(",") if x)):
            d = compare_halt(case, o, run["obs"], mine[h])
            halt_checked[0] += 1
            if d:
                return "fail", "at halt: " + "; ".join(d)
        return "not-implemented", "time halt (M10)"
    if ev != want:
        return "fail", "events %r != %r" % (ev, want)
    if cut:
        reason = "raise/charge call" if cut[2] in ("raise", "charge") else "non-run call"
        return "not-implemented", reason
    d = compare_final(case, o, run["obs"])
    if d:
        return "fail", "; ".join(d)
    return "exact", ""


def cmd_q(args):
    return check_q(args, build(args))


def cmd_churn(args):
    return subprocess.run([build(args), "--exact", "churn_order"]).returncode


def check_q(args, exe, report=print):
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
    if r.returncode not in (0, 3):
        sys.exit("driver failed:\n" + r.stderr[-3000:])
    i2_summary = re.search(r"^i2_checks=(\d+) i2_bad=(\d+)$", r.stderr, re.MULTILINE)
    if i2_summary is None:
        sys.exit("driver missing I2 summary:\n" + r.stderr[-3000:])
    i2_checks, i2_bad = map(int, i2_summary.groups())
    if cases and i2_checks == 0:
        sys.exit("driver ran no I2 checks on a non-empty case set")
    runs = parse_driver(r.stdout)
    count = {"exact": 0, "known-difference": 0, "not-implemented": 0, "fail": 0}
    compared, halt_checked = {}, [0]
    why = {}
    fails = []
    for c in cases:
        cls, note = classify(c, oracle[c["id"]], runs[c["id"]], cuts[c["id"]], compared, halt_checked)
        count[cls] += 1
        if cls in ("known-difference", "not-implemented"):
            why[note] = why.get(note, 0) + 1
        if cls == "fail":
            fails.append((c["id"], note))
    report(json.dumps({"cases": len(cases), "count": count, "reasons": why, "compared_events": compared,
                      "halt_state_checked": halt_checked[0], "i2_checks": i2_checks, "i2_bad": i2_bad},
                     ensure_ascii=False))
    if r.returncode == 3:
        for case_id, run in runs.items():
            for line in run["log"]:
                if line.startswith("I2 "):
                    report(f"I2 FAIL {case_id} {line[3:]}")
    for i, n in fails[:15]:
        report(f"FAIL {i} {n[:600]}")
    return 1 if fails or r.returncode == 3 else 0


def dc_program(case_id, rng, size, closing=False, cycles=1):
    """Script and expected checkpoints for a single limited budget."""
    sid, bid = rng.choice(("session", "s-é", "s-한글")), "budget"
    owner, clock_key = tuple(units(sid)), tuple(units(bid))
    initial_ms = rng.randrange(100)
    now = rng.randrange(1000)
    rec = [0, 0, 1, initial_ms, 0, 0] + key(bid)
    lines = [f"case {case_id}", "put " + " ".join(map(str, key(sid) + [len(rec)] + rec)), f"now {now}"]
    expected = []
    seq = 0
    used, since = initial_ms, None
    runs = {}

    def call(code, run="", flag=0):
        lines.append("call " + " ".join(map(str, [code] + key(sid) + key(bid) + key(run) + [flag])))

    def advance(ms):
        nonlocal now
        lines.append(f"adv {ms}")
        now += ms

    def checkpoint(events, state, has_clock, timer, kind):
        lines.append("inspect")
        budget = [state, 0, 1, used, 2 if since is not None else 0, since or 0]
        clock = [(clock_key, owner, since, int(timer is not None), sorted(runs.items()))] if has_clock else []
        timers = [(timer, due, clock_key, True)] if timer else []
        expected.append((events, budget, clock, timers, 4 + (len(runs) + 1 if has_clock else 0), kind))

    for cycle in range(cycles):
        events = []
        # Starting at least one run also creates the empty clock tested at R=0.
        for n in range(max(1, size)):
            run = f"run-{n:02}"
            call(1, run)
            runs[tuple(units(run))] = 0
        seq += 1
        timer = f"t{seq}"
        since = now
        due = now + 60000 - used
        events.append(f"S {timer} {due - now}")
        advance(rng.randint(1, 20))
        if cycles == 1 and rng.choice((True, False)):
            # Early delivery settles and re-arms with the remaining delay.
            lines.append(f"deliver {timer} 0")
            events.append(f"F {timer}")
            used += now - since
            since = now
            seq += 1
            timer = f"t{seq}"
            due = now + 60000 - used
            events.append(f"S {timer} {due - now}")
        last_timer = timer
        if size == 0:
            call(3, "run-00")
            runs.clear()
        else:
            wait = rng.randrange(size + 1) if cycles == 1 else 0
            for n in range(wait):
                run = f"run-{n:02}"
                call(2, run, 1)
                runs[tuple(units(run))] = 1
        if all(runs.values()):
            used += now - since
            since = None
            events.append(f"C {timer}")
            timer = None
        if cycle == 0 and cycles == 1 and rng.choice((True, False)):
            # Cover CLOSE with an interval whose start lies in the future.
            now -= rng.randint(1, 25)
            lines.append(f"settime {now}")
        else:
            advance(rng.randint(1, 20))
        checkpoint(events, 0, True, timer, "build")
        call(6 if closing else 5)
        if closing and since is not None:
            used += max(0, now - since)
            since = None
        checkpoint([f"C {timer}"] if timer else [], 2 if closing else 0, False, None,
                   "close" if closing else "drop")
        runs.clear()

    lines.append(f"deliver {last_timer} 1")
    events = [f"F {last_timer}"]
    if not closing:
        events.append(f"H {now} " + ",".join(map(str, owner)))
    expected.append((events, [2 if closing else 0, 0, 1, used, 2 if since is not None else 0, since or 0],
                     [], [], None, "closed_delivery" if closing else "open_delivery"))
    lines.append("end")
    return lines, expected


def check_dc(args, exe, report=print):
    typed = getattr(args, "typed", False)
    rng = random.Random(args.seed)
    lines, model = [], {}
    for index in range(args.count + 13):
        reclaim = index >= args.count
        size = index - args.count if reclaim else index % 13
        closing = not reclaim and (index // 13) % 2 == 1
        case_id = f"dc-{index}"
        script, model[case_id] = dc_program(case_id, rng, size, closing, 20 if reclaim else 1)
        lines.extend(script)
    result = subprocess.run([exe, "q"], input="\n".join(lines) + "\n", capture_output=True, text=True)
    summary = re.search(r"^i2_checks=(\d+) i2_bad=(\d+)$", result.stderr, re.MULTILINE)
    if result.returncode not in (0, 3) or summary is None:
        sys.exit("dc driver failed:\n" + result.stderr[-3000:])
    i2_checks, i2_bad = map(int, summary.groups())
    observed = parse_driver(result.stdout)
    checks, failures = {}, {}
    for case_id, expected in model.items():
        run = observed[case_id]
        events, snapshots, live = [], [], None
        for line in run["log"]:
            if line.startswith("L "):
                live = tuple(map(int, line.split()[1:]))
            elif line.startswith("V "):
                snapshots.append((events, live, line[2:]))
                events, live = [], None
            else:
                events.append(line)
        snapshots.append((events, None, run["obs"]))
        errors = []
        if len(snapshots) != len(expected):
            errors.append(f"checkpoints {len(snapshots)} != {len(expected)}")
        first_capacity = None
        before = None
        for step, (got, want) in enumerate(zip(snapshots, expected)):
            events, live, obs = got
            want_events, budget, clocks, timers, count, kind = want
            checks[kind] = checks.get(kind, 0) + 1
            if events != want_events:
                errors.append(f"{step} {kind} events {events!r} != {want_events!r}")
            if not obs or obs.startswith("O X"):
                errors.append(f"{step} {kind} invalid observation {obs!r}")
                continue
            sessions, entries, active = parse_obs(obs)
            actual = [(k, owner, since if sp else None, armed, sorted(runs))
                      for k, owner, sp, since, armed, _, runs in entries]
            budgets = [rec[:6] for rec in sessions.values()]
            if (budgets, actual, active) != ([budget], clocks, timers):
                errors.append(f"{step} {kind} observation {(budgets, actual, active)!r}"
                              f" != {([budget], clocks, timers)!r}")
            if count is not None and (live is None or live[0] != count):
                errors.append(f"{step} {kind} live entries {live!r}, expected {count}")
            # The flat entry walk in root_m9.pruv is bounded by c32(entry capacity).
            if typed and live is not None and max(live) > (1 << 32):
                errors.append(f"{step} typed store exceeds the flat entry bound: {live!r}")
            if kind == "build":
                before = live
                if first_capacity is not None:
                    checks["reclaim"] = checks.get("reclaim", 0) + 1
                    if live is None or live[1] != first_capacity:
                        errors.append(f"{step} store capacity grew: {live!r} vs {first_capacity}")
            elif kind in ("drop", "close"):
                checks["deletion"] = checks.get("deletion", 0) + 1
                removed = len(expected[step - 1][2][0][-1]) + 1
                if before is None or live is None or before[0] - live[0] != removed:
                    errors.append(f"{step} expected {removed} reclaimed entries: {before!r} -> {live!r}")
                if live is not None:
                    if first_capacity is None:
                        first_capacity = live[1]
                    elif live[1] != first_capacity:
                        errors.append(f"{step} store capacity grew: {live[1]} vs {first_capacity}")
        if errors:
            failures[case_id] = errors
    report(json.dumps({"cases": len(model), "ok": len(model) - len(failures), "fail": len(failures),
                      "checks": checks, "i2_checks": i2_checks, "i2_bad": i2_bad}))
    for case_id, errors in list(failures.items())[:15]:
        report(f"FAIL {case_id} " + "\n  ".join(errors[:3]))
    return int(bool(failures or i2_bad))


def cmd_dc(args):
    return check_dc(args, build(args))


def cmd_derive(args):
    """Filter the script to clock paths and run it through the TS oracle."""
    sys.path.insert(0, BUD)
    from oracle import run_oracle
    cases = []
    for l in open(args.cases):
        c = json.loads(l)
        c["id"] += "-clock"
        if args.family == "q4":
            c["script"] = [e for e in c["script"] if e["ev"] in ("advance", "setTime", "deliverTimer", "putSession")
                           or (e["ev"] == "call" and e["op"] in RUN_OPS)]
        else:
            c["script"] = [e for e in c["script"] if not (e["ev"] == "call" and e["op"] in ("raise", "charge"))]
        cases.append(c)
    obs = run_oracle(cases)
    os.makedirs(WORK, exist_ok=True)
    suffix = "q4clock" if args.family == "q4" else "clock"
    with open(os.path.join(WORK, f"cases_{suffix}.jsonl"), "w") as f:
        for c in cases:
            f.write(json.dumps(c, ensure_ascii=False) + "\n")
    with open(os.path.join(WORK, f"oracle_{suffix}.jsonl"), "w") as f:
        for o in obs:
            f.write(json.dumps(o, ensure_ascii=False) + "\n")
    print("derived %d cases" % len(cases))
    return 0


def cmd_perf(args):
    """Alternates Pruv / hand Rust / TS runs per round and size; prints the median ns per call of each item."""
    if args.rounds < 1 or args.reps < 1:
        sys.exit("perf needs --rounds >= 1 and --reps >= 1")
    sys.path.insert(0, BUD)
    from common import NODE, TS_ENV
    exe = build(args)
    hand = os.path.join(os.path.dirname(exe), "hand_m9")
    c = subprocess.run([rustc(), "-C", "opt-level=3", "--edition", "2021", "-o", hand, os.path.join(HERE, "hand_m9.rs")],
                       capture_output=True, text=True)
    if c.returncode != 0:
        sys.exit("rustc failed (hand):\n" + c.stderr[-3000:])
    env = dict(os.environ)
    if args.typed:
        flat_args = vars(args) | {"typed": False, "py": False, "gen": None,
                                  "pruvc": None if args.py else args.pruvc, "out": args.out or TYPED_WORK}
        flat = build(argparse.Namespace(**flat_args))
        sides = {"typed": [exe, "perf"], "flat": [flat, "perf"], "hand": [hand]}
    else:
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
                    if item not in ("refresh", "drop", "close") and size != 10:
                        continue
                    res.setdefault((item, int(sz)), {}).setdefault(side, []).append(float(ns))
    # every item must be measured on both sides; a median above 1.0x hand whose Pruv and hand distributions do not
    # overlap (min Pruv above max hand) is a hold and makes the command fail (P23 audit)
    want = {(i, s) for i in ("refresh", "drop", "close") for s in (10, 50, 100)}
    want |= {(i, 1) for i in ("cycle", "fire", "stale", "staleu")}
    if args.typed:
        print(json.dumps({f"{i}/{z}": v for (i, z), v in sorted(res.items())}), file=sys.stderr)
        print("item | size | typed ns | flat ns | hand Rust ns")
        for item, size in sorted(want):
            sample = res.get((item, size), {})
            if any(len(sample.get(side, [])) != args.rounds for side in sides):
                sys.exit(f"missing or duplicate perf samples for {item}/{size}")
            medians = " | ".join(f"{statistics.median(sample[side]):.1f}" for side in sides)
            print(f"{item} | {size} | {medians}")
        return 0
    missing = sorted(k for k in want if not {"pruv", "hand"} <= set(res.get(k, {})))
    holds = []
    print("item | size | Pruv ns | hand Rust ns | TS ns | Pruv/hand | overlap")
    for (item, sz), v in sorted(res.items()):
        if not {"pruv", "hand"} <= set(v):
            continue
        m = {k: statistics.median(x) for k, x in v.items()}
        ts_ns = f'{m["ts"]:.1f}' if "ts" in m else "n/a"
        ratio = m["pruv"] / m["hand"]
        overlap = min(v["pruv"]) <= max(v["hand"])
        if ratio > 1.0 and not overlap:
            holds.append((item, sz))
        print("%s | %d | %.1f | %.1f | %s | %.2f | %s" % (item, sz, m["pruv"], m["hand"], ts_ns, ratio,
                                                         "yes" if overlap else "no"))
    if missing:
        print("MISSING " + " ".join("%s/%d" % k for k in missing))
    if holds:
        print("HOLD " + " ".join("%s/%d" % k for k in holds))
    return 1 if missing or holds else 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", choices=["q", "perf", "derive", "dc", "churn"])
    ap.add_argument("--pruvc", help="compiler executable, or Python compiler script with --py")
    ap.add_argument("--typed", action="store_true", help="use the resident typed state lane")
    ap.add_argument("--py", action="store_true", help="compile the typed lane with the Python Rust backend")
    ap.add_argument("--budlib")
    ap.add_argument("--out", help="directory for generated Rust and driver binaries")
    ap.add_argument("--family", choices=["q3", "q4"], default="q3")
    ap.add_argument("--count", type=int, default=300, help="number of randomized dc cases")
    ap.add_argument("--seed", type=int, default=20261117)
    ap.add_argument("--gen")
    ap.add_argument("--cases")
    ap.add_argument("--oracle")
    ap.add_argument("--rounds", type=int, default=5)
    ap.add_argument("--reps", type=int, default=200000)
    a = ap.parse_args()
    if a.typed and (a.cmd not in ("q", "perf", "churn", "dc") or a.budlib):
        ap.error("--typed supports q, perf, churn and dc without --budlib")
    if a.cmd == "churn" and not a.typed:
        ap.error("churn requires --typed")
    if a.py and (not a.typed or a.gen):
        ap.error("--py requires --typed without --gen")
    def fixture(filename):
        local = os.path.join(WORK, filename)
        if a.cmd != "q" or os.path.isfile(local):
            return local
        return os.path.join(shared_checkout(), os.path.relpath(local, ROOT))
    if a.cases is None:
        a.cases = fixture("cases_q4.jsonl" if a.cmd == "derive" and a.family == "q4" else "cases.jsonl")
    if a.oracle is None:
        a.oracle = fixture("oracle.jsonl")
    sys.exit({"q": cmd_q, "perf": cmd_perf, "derive": cmd_derive, "dc": cmd_dc, "churn": cmd_churn}[a.cmd](a))


if __name__ == "__main__":
    main()
