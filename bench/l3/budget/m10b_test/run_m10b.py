#!/usr/bin/env python3
"""Compare budget raise with the TypeScript oracle and benchmark against hand Rust."""
import argparse
from collections import Counter
import json
from pathlib import Path
import re
import statistics
import subprocess
import sys

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "m10_test"))
import run_m10 as m10

WORK = Path(m10.ROOT) / "work/p30"
KEYS = ("timeMinutes", "delegations", "commands", "reviews", "checks")


def checked(cmd, **kwargs):
    result = subprocess.run(cmd, capture_output=True, text=True, **kwargs)
    if result.returncode:
        raise SystemExit(result.stdout + result.stderr)
    return result


def generate(compiler, target, facts):
    root = HERE / "root_m10b.pruv"
    cmd = [m10.PY_DEFAULT, m10.PRUVC_PY] if compiler == "py" else [compiler]
    backend = ["--backend", "rust"] if compiler == "py" else []
    checked(cmd + [str(root), "-o", str(target)] + backend + m10.LIBS
            + ["--rust-extern", f"bud_facts={facts}", "--proof-cache",
               str(WORK / ("proof-py" if compiler == "py" else "proof-rs"))], cwd=m10.ROOT)


def build(args):
    work = WORK / ("py" if args.py else "rs")
    facts = m10.build_facts(work)
    generate("py" if args.py else args.pruvc, work / "gen.rs", facts)
    main = work / "main.rs"
    codec = json.dumps(str(Path(m10.M7) / "record_text.rs"))
    driver = json.dumps(str(HERE / "driver_m10b.rs"))
    main.write_text('#![allow(dead_code, unused_imports, unused_variables, unused_mut)]\n'
                    f'#[path = {codec}] mod record_text;\nmod gen;\nuse gen as sol;\n'
                    f'include!({driver});\n')
    exe = work / "driver_m10b"
    checked([m10.rustc(), "-O", "--edition", "2021", "--extern", f"bud_facts={facts}",
             "-o", exe, main])
    return exe, facts


def wire_unit(value):
    answer = m10.jv_decode(value)
    return [0] + [answer[key] for key in KEYS]


def call_body(event):
    if event["op"] != "raise":
        return m10.call_body(event["op"], event["args"])
    ref, size = map(m10.jv_decode, event["args"])
    # 0 is the unlimited code; any other value outside the size type goes as 3 (status 2, KD-P30-4).
    code = 0 if size == "unlimited" else (size if type(size) is int and size in (1, 2, 4, 8) else 3)
    return [13] + [0] * 10 + [code] + m10.key(ref["sessionId"]) + m10.key(ref["budgetId"]) + [0]


def program(case):
    world = case["world"]
    # Use the M10 framing for the initial world, including existing fact streams.
    setup, _ = m10.program({**case, "script": []})
    lines = setup[:-1] + ["unit " + " ".join(map(str, wire_unit(world["unit"])))]
    for event in case["script"]:
        kind = event["ev"]
        if kind == "advance":
            lines.append(f'adv {event["ms"]}')
        elif kind == "setUnit":
            lines.append("unit " + " ".join(map(str, wire_unit(event["unit"]))))
        elif kind == "call":
            op = event["op"]
            if op not in m10.CODE and op != "raise":
                return lines + ["end"], (event["req"], op)
            lines.append(f'call {event["req"]} ' + " ".join(map(str, call_body(event))))
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
    # Use the M10 classifier with this lane's header precedence for both
    # current() views and final observations.
    m10.decode_budget = decode_budget
    try:
        return m10.classify(case, oracle, run, cut)
    finally:
        m10.decode_budget = decode_m10_budget


def read_lines(path):
    with open(path) as file:
        return [json.loads(line) for line in file]


def write_lines(path, rows):
    with open(path, "w") as file:
        for row in rows:
            file.write(json.dumps(row, ensure_ascii=True) + "\n")


def derive(args):
    from oracle import run_oracle
    from q_gen import SEED, build as make_case

    def derived(case):
        return {**case, "id": case["id"] + "-raise",
                "script": [e for e in case["script"] if e.get("op") not in ("decide", "hasCard", "setCard")]}

    def raises(case):
        return sum(e.get("op") == "raise" for e in case["script"])

    cases = [derived(c) for c in read_lines(args.cases or m10.input_path("cases.jsonl"))]
    total = sum(map(raises, cases))
    ordinal = 1
    ids = {case["id"] for case in cases}
    while total < 300:
        case = derived(make_case("Q5", SEED, ordinal))
        ordinal += 1
        if case["id"] not in ids and raises(case):
            cases.append(case)
            total += raises(case)
    observations = run_oracle(cases)
    write_lines(WORK / "cases_raise.jsonl", cases)
    write_lines(WORK / "oracle_raise.jsonl", observations)
    print(json.dumps({"cases": len(cases), "raise_calls": total}))
    return 0


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
        expected = ["A 2 0 true true true -99 []"]
        if query:
            expected.append("E unit " + unit)
        expected.append("X 2")
        if log[:len(expected)] != expected or not log[len(expected)].startswith("O"):
            raise SystemExit(f"raise rejection changed state or effects: {log[:5]}")

    log = run(["fill_arena", "audit", "now 1700014400001", call(2)]).split("@ probe\n", 1)[1].splitlines()
    if not re.fullmatch(r"A -[1-9]\d* 0 true true true -99 \[\]", log[0]):
        raise SystemExit(f"raise did not Grow before writes: {log[:2]}")
    effects = [line.split()[1] for line in log if line.startswith("E ")]
    expected = ["unit", "clear", "rdi", "cancel", "stopMain", "stopTurns", "hold", "save",
                "changed", "announce", "changed"]
    if effects != expected or "V 2" not in log:
        raise SystemExit(f"raise Grow/time-halt retry diverged: {log[:15]}")
    print("PASS raise controls: 6 rejection paths and 1 Grow/time-halt retry")


def run_cases(args, cases, observations, fixtures=False):
    exe, _ = build(args)
    if fixtures:
        check_raise_controls(exe, next(case for case in cases if case["id"] == "H10"))
    inputs, cuts = [], {}
    for case in cases:
        lines, cut = program(case)
        inputs.extend(lines)
        cuts[case["id"]] = cut
    runs = m10.parse_driver(checked([exe, "q"], input="\n".join(inputs) + "\n").stdout)
    observations = {o["id"]: o for o in observations}
    counts = Counter({"exact": 0, "not-implemented": 0, "fail": 0})
    compared = 0
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
            compared += sum(e.get("op") == "raise" and e["req"] in run["reqs"] for e in case["script"])
        if fixtures:
            print(cid, result)
    print(json.dumps({"cases": len(cases), "count": counts, "compared_raise_calls": compared}))
    return int(counts["fail"] > 0 or (fixtures and counts["exact"] != len(cases)))


def q(args):
    return run_cases(args, read_lines(args.cases or WORK / "cases_raise.jsonl"),
                     read_lines(args.oracle or WORK / "oracle_raise.jsonl"))


def h(args):
    from common import load_group
    from oracle import run_oracle
    cases = load_group("H")
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
             "-o", hand, HERE / "hand_m10b.rs"])
    commands = {"pruv": [exe, "perf"], "hand": [hand]}
    sample = rss_runner()
    holds = []
    print("| Path | D | Pruv ns/op | Hand ns/op | Pruv alloc/op | Hand alloc/op | Pruv RSS bytes | Hand RSS bytes | Hold |")
    print("|---|---:|---:|---:|---:|---:|---:|---:|---|")
    for item in ("time", "limits", "unlimited"):
        for d in (10, 50, 100):
            samples = {side: [] for side in commands}
            for round_index in range(args.rounds):
                sides = list(commands)
                if round_index % 2:
                    sides.reverse()
                for side in sides:
                    samples[side].append(sample(commands[side] + [item, str(d), str(args.reps)]))
            p, h = [tuple(map(statistics.median, zip(*samples[side]))) for side in commands]
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
    parser.add_argument("--cases")
    parser.add_argument("--oracle")
    parser.add_argument("--rounds", type=int, default=5)
    parser.add_argument("--reps", type=int, default=10000)
    args = parser.parse_args()
    WORK.mkdir(parents=True, exist_ok=True)
    return commands[args.cmd](args)


if __name__ == "__main__":
    sys.exit(main())
