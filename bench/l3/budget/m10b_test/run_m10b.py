#!/usr/bin/env python3
"""Compare budget raise and decide with the TypeScript oracle and benchmark against hand Rust."""
import argparse
from collections import Counter
import difflib
import json
import os
from pathlib import Path
import re
import statistics
import subprocess
import sys

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "m10_test"))
import run_m10 as m10

WORK = HERE / ".m10b-work"
KEYS = ("timeMinutes", "delegations", "commands", "reviews", "checks")


def checked(cmd, **kwargs):
    result = subprocess.run(cmd, capture_output=True, text=True, **kwargs)
    if result.returncode:
        raise SystemExit(result.stdout + result.stderr)
    return result


def build_decide_facts(work, facts):
    rlib = work / "libdecide_facts.rlib"
    checked([m10.rustc(), "-O", "--edition", "2021", "--crate-type", "rlib", "--crate-name", "decide_facts",
             "--extern", f"bud_facts={facts}", HERE / "decide_facts.rs", "-o", rlib])
    return rlib


def generate(compiler, target, facts):
    root = HERE / "root_m10b_typed.pruv"
    cmd = [m10.PY_DEFAULT, m10.PRUVC_PY] if compiler == "py" else [compiler]
    backend = ["--backend", "rust"] if compiler == "py" else []
    checked(cmd + [str(root), "-o", str(target)] + backend + m10.LIBS
            + ["--rust-extern", f"bud_facts={facts}",
               "--rust-extern", f"decide_facts={build_decide_facts(target.parent, facts)}", "--proof-cache",
               str(WORK / ("proof-py" if compiler == "py" else "proof-rs"))], cwd=m10.ROOT)


def build(args):
    work = WORK / ("py" if args.py else "rs")
    facts = m10.build_facts(work)
    generate("py" if args.py else args.pruvc, work / "gen.rs", facts)
    main = work / "main.rs"
    codec = json.dumps(str(Path(m10.M7) / "record_text.rs"))
    driver = json.dumps(str(HERE / "driver_m10b.rs"))
    main.write_text('#![allow(dead_code, unused_imports, unused_variables, unused_mut)]\n'
                    f'#[path = {codec}] mod record_text;\nmod gen;\n'
                    'mod sol { pub use crate::gen::*;\n'
                    'pub fn t_text(input: &[u8], offset: i64, count: i64, out: &mut [u8]) -> i64 {\n'
                    '    let start = offset as usize;\n'
                    '    out.copy_from_slice(&input[start..start + count as usize]);\n'
                    '    0\n'
                    '} }\n'
                    f'include!({driver});\n')
    exe = work / "driver_m10b"
    checked([m10.rustc(), "-O", "--edition", "2021", "--extern", f"bud_facts={facts}",
             "--extern", f"decide_facts={build_decide_facts(work, facts)}", "-o", exe, main])
    return exe, facts


def wire_unit(value):
    answer = m10.jv_decode(value)
    return [0] + [answer[key] for key in KEYS]


def size_code(size):
    """0 is the unlimited code; an absent size is 1 (Node `size ?? 1`); any other value outside the size type goes as
    3 (status 2, KD-P30-4)."""
    if size == "unlimited":
        return 0
    if size is None or size is m10.UNDEF:
        return 1
    return size if type(size) is int and size in (1, 2, 4, 8) else 3


def call_body(event):
    if event["op"] == "decide":
        a = list(map(m10.jv_decode, event["args"]))
        flag = size_code(a[3] if len(a) > 3 else None) if a[2] == "extend" else -1
        return [14] + [0] * 10 + [flag] + m10.key(a[0]) + m10.key(a[1]) + [0]
    if event["op"] != "raise":
        return m10.call_body(event["op"], event["args"])
    ref, size = map(m10.jv_decode, event["args"])
    code = 0 if size == "unlimited" else (size if type(size) is int and size in (1, 2, 4, 8) else 3)
    return [13] + [0] * 10 + [code] + m10.key(ref["sessionId"]) + m10.key(ref["budgetId"]) + [0]


def program(case):
    world = case["world"]
    # Use the M10 framing for the initial world, including existing fact streams.
    setup, _ = m10.program({**case, "script": []})
    lines = setup[:-1] + ["unit " + " ".join(map(str, wire_unit(world["unit"])))]
    held = (world.get("deps") or {}).get("releaseHeld") or []
    lines.append("rel " + " ".join("1" if m10.truthy(m10.jv_decode(x)) else "0" for x in held))
    for event in case["script"]:
        kind = event["ev"]
        if kind == "advance":
            lines.append(f'adv {event["ms"]}')
        elif kind == "setUnit":
            lines.append("unit " + " ".join(map(str, wire_unit(event["unit"]))))
        elif kind == "call":
            op = event["op"]
            if op not in m10.CODE and op not in ("raise", "decide"):
                return lines + ["end"], (event["req"], op)
            lines.append(f'call {event["req"]} ' + " ".join(map(str, call_body(event))))
        elif kind == "snapshot" and event is case["script"][-1]:
            pass  # a closing snapshot is the end state, which the classifier compares anyway
        else:
            raise ValueError(f"unknown event {kind}")
    return lines + ["end"], None


decode_m10_budget = m10.decode_budget


def decode_budget(words):
    budget = decode_m10_budget(words)
    # A raise can promote word 2 from 2 to 1. The old non-true value in opaque
    # rest only belongs to the word-2 == 2 representation.
    if budget is not None and words[2] == 1:
        budget["unlimited"] = True
    return budget


HELD = {"rel": "releaseHeld", "qc": "queueContinue", "drop": "dropHeld"}
mine_m10_value = m10.mine_value


def mine_value(words):
    if words and words[0] == 6:
        codes = {0: {"ok": True}, 1: {"ok": False, "code": "not-found"}, 2: {"ok": False, "code": "not-halted"}}
        if words[1] not in codes:
            raise SystemExit(f"decide result code outside 0..2: {words[:2]}")
        return codes[words[1]]
    return mine_m10_value(words)


def held_mine(f):
    if f[0] == "rel":
        return ("releaseHeld", tuple(int(x) for x in f[1][2:].split(",") if x),
                tuple(int(x) for x in f[2][2:].split(",") if x), int(f[3]))
    if f[0] == "qc":
        return ("queueContinue", int(f[1]), int(f[2]))
    return ("dropHeld",)


def held_oracle(entry):
    c, a = entry["c"], entry.get("a")
    if c == "releaseHeld":
        sess = m10.jv_decode(a[0]["v"]) if isinstance(a[0], dict) and "v" in a[0] else m10.jv_decode(a[0])
        return (c, tuple(m10.units(sess["id"])), tuple(m10.units(m10.jv_decode(a[1]))),
                1 if m10.truthy(m10.jv_decode(entry.get("o"))) else 0)
    if c == "queueContinue":
        reason, wake = m10.jv_decode(a[2]), m10.jv_decode(a[3])
        return (c, -1 if reason is None else m10.REASONS.index(reason), 1 if wake else 0)
    return (c,)


def compare(case, oracle, run, cut):
    # Existing M10 effects are compared by its classifier. Verify unit's returned data
    # and position relative to those effects before passing the remaining stream through.
    traces = {}
    for entry in oracle["trace"]:
        traces.setdefault(entry.get("r"), []).append(entry)
    for req in run["order"]:
        stream = run["reqs"][req]["fx"]
        got = [(i, list(map(int, f[1:]))) for i, f in enumerate(stream) if f[0] == "unit"]
        want = []
        position = 0
        for entry in traces.get(req, []):
            if entry["c"] == "unit":
                want.append((position, wire_unit(entry["o"])))
                position += 1
            elif entry["c"] in m10.FX:
                position += 1
        if got != want:
            return "fail", f"{req} unit effects {got!r} != {want!r}"
        stream[:] = [f for f in stream if f[0] != "unit"]
        # decide's host calls (P31): check their arguments here; the classifier checks their order by name.
        got = [held_mine(f) for f in stream if f[0] in HELD]
        want = [held_oracle(entry) for entry in traces.get(req, []) if entry["c"] in HELD.values()]
        if got != want:
            return "fail", f"{req} decide host calls {got!r} != {want!r}"
        stream[:] = [[f[0]] if f[0] in HELD else f for f in stream]
    # Use the M10 classifier with this lane's header precedence for both
    # current() views and final observations, and with decide's value and host calls.
    m10.decode_budget = decode_budget
    m10.mine_value = mine_value
    m10.MINE_FX.update(HELD)
    # decide takes the session id as its first argument; the classifier reads it from a `{sess}` argument
    script = [{**e, "args": [{"sess": m10.jv_decode(e["args"][0])}] + e["args"][1:]} if e.get("op") == "decide" else e
              for e in case["script"]]
    try:
        return m10.classify({**case, "script": script}, oracle, run, cut)
    finally:
        m10.decode_budget = decode_m10_budget
        m10.mine_value = mine_m10_value
        for name in HELD:
            del m10.MINE_FX[name]


def read_lines(path):
    with open(path) as file:
        return [json.loads(line) for line in file]


def write_lines(path, rows):
    with open(path, "w") as file:
        for row in rows:
            file.write(json.dumps(row, ensure_ascii=True) + "\n")


def derive(args):
    """Q5-derived sets: `raise` keeps P30's raise-only cases (decide/card calls stripped), `decide` keeps decide too
    (card calls stripped), each grown with Q5 cases until it holds 300 calls of its op."""
    from oracle import run_oracle
    from q_gen import SEED, build as make_case

    def derived(case, drop, suffix):
        return {**case, "id": case["id"] + suffix, "script": [e for e in case["script"] if e.get("op") not in drop]}

    def calls(case, op):
        return sum(e.get("op") == op for e in case["script"])

    summary = {}
    for name, op, drop in (("raise", "raise", ("decide", "hasCard", "setCard")), ("decide", "decide", ("hasCard", "setCard"))):
        suffix = "-" + name
        cases = [derived(c, drop, suffix) for c in read_lines(args.cases or m10.input_path("cases.jsonl"))]
        total = sum(calls(c, op) for c in cases)
        ordinal = 1
        ids = {case["id"] for case in cases}
        while total < 300:
            case = derived(make_case("Q5", SEED, ordinal), drop, suffix)
            ordinal += 1
            if case["id"] not in ids and calls(case, op):
                cases.append(case)
                total += calls(case, op)
        write_lines(WORK / f"cases_{name}.jsonl", cases)
        write_lines(WORK / f"oracle_{name}.jsonl", run_oracle(cases))
        summary[name] = {"cases": len(cases), "calls": total}
    print(json.dumps(summary))
    return 0


def check_audit_controls(exe):
    def obs(line):
        # parse_obs reads a missing timer section as no timers, so require the separator first.
        if "| T" not in line:
            raise SystemExit(f"audit controls: O line without timer section: {line!r}")
        return f"O {m10.parse_obs(line)!r}\n"

    def observed(stdout):
        # The flat and typed hosts encode inactive fields and tool counts differently.
        return [obs(line) if line.startswith("O ") else line
                for line in stdout.splitlines(keepends=True)]

    for case in json.loads((HERE / "audit_controls.golden.json").read_text()):
        source = case["input"]
        name = source.splitlines()[0].removeprefix("case ")
        expected = observed(case["stdout"])
        actual = observed(checked([exe, "q"], input=source).stdout)
        if actual != expected:
            diff = "".join(difflib.unified_diff(expected, actual, fromfile="golden", tofile="typed"))
            raise SystemExit(f"{name}:\n{diff}")
        print(f"{name} exact (golden)")


def check_typed_malformed(exe, raise_fixture, decide_fixture):
    def check(prefix, tail, expected):
        source = "\n".join(prefix[:-1] + ["audit", *tail, "end"]) + "\n"
        log = checked([exe, "q"], input=source).stdout.split("@ probe\n", 1)[1].splitlines()
        actual = [line for line in log[:-1] if not line.startswith("O ")]
        if actual != expected:
            raise SystemExit(f"typed malformed {tail}: expected {expected}, got {actual}")

    active, _ = program({**raise_fixture, "script": raise_fixture["script"][:1]})
    halted, _ = program({**decide_fixture, "script": decide_fixture["script"][:-1]})
    ref = m10.jv_decode(raise_fixture["script"][2]["args"][0])

    def frame(code, flag, what, sid, bid):
        return " ".join(map(str, [code, *what, flag, *m10.key(sid), *m10.key(bid), 0]))

    for width in (9, 11):
        body = frame(7, 0, [2, 1] + [0] * (width - 2), ref["sessionId"], ref["budgetId"])
        check(active, [f"call-what probe {width} {body}"], ["A 2 0 true", "X 2"])

    raise_body = frame(13, 2, [0] * 10, ref["sessionId"], ref["budgetId"])
    decide_body = " ".join(map(str, call_body(decide_fixture["script"][-1])))
    for unit in ("0 120 24 3 2", "0 120 24 3 2 1 0", "1 120 24 3 2 1"):
        for prefix, body in ((active, raise_body), (halted, decide_body)):
            check(prefix, ["unit " + unit, "call probe " + body], ["A 2 0 true", "E unit " + unit, "X 2"])

    charge = [2, 1] + [0] * 8
    unknown = frame(7, 0, charge, "missing-session", ref["budgetId"])
    check(active, ["call probe " + unknown], ["A 0 0 true", "V 5 2 0"])

    session = raise_fixture["world"]["sessions"][0]
    # Replace the original budget under the same session before charging its old bid.
    replacement = {**session, "budget": {**session["budget"], "id": "replacement-budget"}}
    stale, _ = program({**raise_fixture, "world": {**raise_fixture["world"], "sessions": [session, replacement]},
                        "script": []})
    old_bid = frame(7, 0, charge, ref["sessionId"], ref["budgetId"])
    check(stale, ["call probe " + old_bid], ["A 0 0 true", "V 5 2 0"])
    print("typed malformed PASS")


def check_raise_controls(exe, fixture):
    setup = {**fixture, "script": fixture["script"][:1]}
    prefix, _ = program(setup)
    event = fixture["script"][2]
    ref = m10.jv_decode(event["args"][0])

    def call(size):
        body = call_body({"op": "raise", "args": [ref, size]})
        return "call probe " + " ".join(map(str, body))

    def run(tail):
        return checked([exe, "q"], input="\n".join(prefix[:-1] + tail + ["end"]) + "\n").stdout

    for size, unit, query in (
        (2, "0 120 24 3 2", True),
        (2, "1 120 24 3 2 1", True),
        (2, "0 120 24 3 2 -1", True),
        (2, "0 120 24 3 2 9007199254740992", True),
        (2, "0 120 24 3 2 9007199254740991", True),
        (3, "0 120 24 3 2 1", False),
    ):
        log = run(["unit " + unit, "audit", call(size)]).split("@ probe\n", 1)[1].splitlines()
        expected = ["A 2 0 true"]
        if query:
            expected.append("E unit " + unit)
        expected.append("X 2")
        if log[:len(expected)] != expected or not log[len(expected)].startswith("O"):
            raise SystemExit(f"raise rejection changed state or effects: {log[:5]}")

    log = run(["now 1700014400001", call(2)]).split("@ probe\n", 1)[1].splitlines()
    effects = [line.split()[1] for line in log if line.startswith("E ")]
    expected = ["unit", "clear", "rdi", "cancel", "stopMain", "stopTurns", "hold", "save",
                "changed", "announce", "changed"]
    if effects != expected or "V 2" not in log:
        raise SystemExit(f"raise time-halt path diverged: {log[:15]}")
    print("PASS raise controls: 6 rejection paths and 1 time-halt path")


def check_decide_controls(exe, fixture):
    """G10's halted s3: rejections leave state and effects untouched, and an
    invalid releaseHeld answer is status 2 after the store (KD-P31-3)."""
    prefix, _ = program({**fixture, "script": fixture["script"][:-1]})
    body = call_body(fixture["script"][-1])

    def run_all(tail, flag):
        b = list(body)
        b[11] = flag
        out = checked([exe, "q"], input="\n".join(prefix[:-1] + tail + ["audit", "call probe " + " ".join(map(str, b)),
                                                                        "end"]) + "\n").stdout
        return out.split("@ probe\n", 1)[1].splitlines()

    def run(tail, flag):
        return [line for line in run_all(tail, flag) if not line.startswith("O")]

    untouched = "A 2 0 true"
    # size 3; product above 2^53-1; sum only above 2^53-1 (size 1, existing limit >= 1); malformed unit answer
    for tail, flag, unit in (([], 3, None), (["unit 0 1 24 3 2 9007199254740991"], 8, "0 1 24 3 2 9007199254740991"),
                             (["unit 0 9007199254740991 24 3 2 1"], 1, "0 9007199254740991 24 3 2 1"),
                             (["unit 1 1 24 3 2 1"], 2, "1 1 24 3 2 1")):
        expected = [untouched] + (["E unit " + unit] if unit else []) + ["X 2"]
        log = run(tail, flag)
        if log[:len(expected)] != expected:
            raise SystemExit(f"decide rejection changed state or effects: {log[:5]}")
    # invalid releaseHeld answer: status 2 after the store, and the stored state (O) equals a valid answer's
    full = run_all(["rel 2"], 1)
    log = [line for line in full if not line.startswith("O")][:-1]  # drop the driver's closing "end"
    if not log[-2].startswith("E rel ") or not log[-2].endswith(" 2") or log[-1] != "X 2" or "E save" not in log:
        raise SystemExit(f"decide invalid releaseHeld answer: {log[-4:]}")
    if [line for line in full if line.startswith("O")] != [line for line in run_all(["rel 1"], 1) if line.startswith("O")]:
        raise SystemExit("decide invalid releaseHeld answer: stored state differs from a valid answer's")
    print("PASS decide controls: 4 rejection paths and 1 invalid releaseHeld answer")


def check_malformed_keys(exe):
    # Keep the op, what, flag, session, and budget fields valid. Corrupt the final run key.
    keys = {"length": "-1", "padding": "1 1", "truncated": "5 0", "extra": "0 0"}
    body = "8 " + "0 " * 13
    source, expected = [], []
    for name, words in keys.items():
        source.append(f"case {name}")
        expected.append(f"case {name}")
        if name != "extra":
            source.append(f"put {words}")
            expected.append("X 2")
        source.extend(("audit", f"call probe {body}{words}", "end"))
        expected.extend(("@ probe", "X 2", "O 0 0 | T", "end"))
    records = (
        ("put-length", "0 2 -1"),
        ("short-budget", "0 25" + " 0" * 25),
        ("truncated-text", "0 30" + " 0" * 25 + " 6 0 0 0 0 |"),
        ("text-outside-buffer", "0 32" + " 0" * 25 + " 2 2 0 0 0 0 0 | 97"),
        ("truncated-halt", "0 35" + " 0" * 29 + " 5 -2 0 1 0 0 |"),
    )
    for name, words in records:
        source.extend((f"case {name}", f"put {words}", "put 0 1 -1", f"call probe {body}0", "end"))
        expected.extend((f"case {name}", "X 2", "@ probe", "V 0", "O 1 0 1 -1 0 | T", "end"))
    log = checked([exe, "q"], input="\n".join(source) + "\n").stdout.splitlines()
    if log != expected:
        raise SystemExit(f"packed key admission mismatch: {log}")
    print("packed key and frame admission PASS (4 malformed calls, 8 malformed puts, 5 recovery calls)")


def run_cases(args, cases, observations, fixtures=False):
    exe, _ = build(args)
    check_malformed_keys(exe)
    if fixtures and any(case["id"] == "H10" for case in cases):
        fixture = next(case for case in cases if case["id"] == "H10")
        check_raise_controls(exe, fixture)
        check_audit_controls(exe)
        from common import load_group
        decide_fixture = next(case for case in load_group("G") if case["id"] == "G10")
        check_typed_malformed(exe, fixture, decide_fixture)
    if fixtures and any(case["id"] == "G10" for case in cases):
        check_decide_controls(exe, next(case for case in cases if case["id"] == "G10"))
    inputs, cuts = [], {}
    for case in cases:
        lines, cut = program(case)
        inputs.extend(lines)
        cuts[case["id"]] = cut
    runs = m10.parse_driver(checked([exe, "q"], input="\n".join(inputs) + "\n").stdout)
    observations = {o["id"]: o for o in observations}
    counts = Counter({"exact": 0, "not-implemented": 0, "fail": 0})
    compared = Counter({"raise": 0, "decide": 0})
    for case in cases:
        cid = case["id"]
        oracle = observations[cid]
        run = runs[cid]
        if fixtures:
            from check_oracle import check
            errors, checker_errors = check(case, oracle)
            if errors or checker_errors:
                print("FAIL", cid, errors + checker_errors)
                counts["fail"] += 1
                continue
        result, note = compare(case, oracle, run, cuts[cid])
        counts[result] += 1
        if result == "fail":
            print("FAIL", cid, note)
        else:
            for e in case["script"]:
                if e.get("op") in ("raise", "decide") and e["req"] in run["reqs"]:
                    compared[e["op"]] += 1
        if fixtures:
            print(cid, result)
    print(json.dumps({"cases": len(cases), "count": counts, "compared_calls": compared}))
    return int(counts["fail"] > 0 or (fixtures and counts["exact"] != len(cases)))


def q(args):
    return run_cases(args, read_lines(args.cases or WORK / f"cases_{args.set}.jsonl"),
                     read_lines(args.oracle or WORK / f"oracle_{args.set}.jsonl"))


def h(args):
    from common import load_group
    from oracle import run_oracle
    cases = load_group(args.group)
    return run_cases(args, cases, run_oracle(cases), fixtures=True)


def same(args):
    facts = m10.build_facts(WORK)
    rs, py = WORK / "gen_rs.rs", WORK / "gen_py.rs"
    generate(args.pruvc, rs, facts)
    generate("py", py, facts)
    a, b = rs.read_bytes(), py.read_bytes()
    print(json.dumps({"rs_bytes": len(a), "py_bytes": len(b), "identical": a == b}))
    return int(a != b)


def rss_runner():
    time_cmd = ["/usr/bin/time", "-l"]
    rss = re.compile(r"(\d+)\s+maximum resident set size")
    probe = subprocess.run(time_cmd + ["/usr/bin/true"], capture_output=True, text=True)
    if probe.returncode == 0 and rss.search(probe.stderr):
        def sample(cmd):
            result = checked(time_cmd + cmd)
            return (*map(float, result.stdout.split()), int(rss.search(result.stderr)[1]))
        return sample

    # macOS time -l asks for kern.clockrate before printing RSS. A sandbox may
    # deny that sysctl while allowing getrusage. Each wrapper waits for one child.
    print("Peak RSS from getrusage(RUSAGE_CHILDREN); /usr/bin/time -l RSS unavailable.")
    wrapper = (
        "import resource, subprocess, sys\n"
        "subprocess.run(sys.argv[1:], check=True)\n"
        "rss = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss\n"
        "print(rss if sys.platform == 'darwin' else rss * 1024)\n"
    )

    def sample(cmd):
        ns, alloc, peak = checked([sys.executable, "-c", wrapper] + cmd).stdout.split()
        return float(ns), float(alloc), int(peak)

    return sample


def perf(args):
    if args.rounds < 1 or args.reps < 1:
        raise SystemExit("perf requires positive rounds and reps")
    exe, facts = build(args)
    hand = exe.with_name("hand_m10b")
    checked([m10.rustc(), "-O", "--edition", "2021", "--extern", f"bud_facts={facts}",
             "--extern", f"decide_facts={build_decide_facts(exe.parent, facts)}", "-o", hand, HERE / "hand_m10b.rs"])
    commands = {"pruv": [exe, "perf"], "hand": [hand]}
    sample = rss_runner()
    holds = []
    print("| Path | D | Pruv ns/op | Hand ns/op | Pruv alloc/op | Hand alloc/op | Pruv RSS bytes | Hand RSS bytes | Hold |")
    print("|---|---:|---:|---:|---:|---:|---:|---:|---|")
    for item in args.items.split(","):
        for d in (10, 50, 100):
            samples = {side: [] for side in commands}
            for round_index in range(args.rounds):
                sides = list(commands)
                if round_index % 2:
                    sides.reverse()
                for side in sides:
                    samples[side].append(sample(commands[side] + [item, str(d), str(args.reps)]))
            p, h = [tuple(map(statistics.median, zip(*samples[side]))) for side in commands]
            print(json.dumps({"raw": item, "d": d, "samples": samples}), file=sys.stderr)
            hold = p[0] > h[0] and min(s[0] for s in samples["pruv"]) > max(s[0] for s in samples["hand"])
            if hold:
                holds.append((item, d))
            print(f"| {item} | {d} | {p[0]:.1f} | {h[0]:.1f} | {p[1]:.2f} | {h[1]:.2f} | {p[2]:.0f} | {h[2]:.0f} | {'yes' if hold else 'no'} |")
    if holds:
        print("HOLD " + " ".join(f"{item}/{d}" for item, d in holds))
    return int(bool(holds))


def main():
    commands = {fn.__name__: fn for fn in (derive, q, h, same, perf)}
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cmd", choices=commands)
    parser.add_argument("--pruvc", default=m10.PRUVC_DEFAULT)
    parser.add_argument("--py", action="store_true")
    parser.add_argument("--lane", choices=("direct", "adapter"), default="adapter")
    parser.add_argument("--set", choices=("raise", "decide"), default="decide")
    parser.add_argument("--group", default="H")
    parser.add_argument("--cases")
    parser.add_argument("--oracle")
    parser.add_argument("--rounds", type=int, default=5)
    parser.add_argument("--reps", type=int, default=10000)
    parser.add_argument("--items", default="time,limits,unlimited,extend,end")
    args = parser.parse_args()
    WORK.mkdir(parents=True, exist_ok=True)
    os.environ["M10B_LANE"] = args.lane
    os.environ["TMPDIR"] = str(WORK)
    return commands[args.cmd](args)


if __name__ == "__main__":
    sys.exit(main())
