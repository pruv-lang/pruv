#!/usr/bin/env python3
"""P24: synchronous halt (bud_halt.pruv) against isolated mutants, plus the G fixtures.

  python3 bench/l3/budget/m10_test/mutant_halt.py [--pruvc PATH] [--jobs N]

Each variant (the unmutated control and every mutant) gets its own copy of bench/l3/budget with a mutated
bud_halt.pruv and its own build directory under work/m10/mut/campaign-*/ (P23 H6). A variant is compiled with
root_m10 against the copy; a mutant the verifier rejects is reported REJECTED (caught by proof). A built variant runs:
  q        run_m10.py q on cases.jsonl (Q4/Q5) and cases_halt.jsonl (derived cuts) against the TS oracle,
           which compares the actual query trace: runningDelegationIds after timer.clear (SETTLE), its arguments
           (sessionId, budgetId) and the stops built from the answer;
  G        the fixtures below on the variant's driver.
The control must pass everything (CLEAN); every mutant must fail something (KILLED) or be REJECTED.

G fixtures (each starts from a corpus halt whose request clears a timer before the query):
  G1 answer  the answer the host gave is the one the stops carry, in order (2 ids, 0 ids)
  G2 bad     a malformed answer (count says 2, one key) after SETTLE: the log is exactly clear, rdi, X 2; the session
             is not halted (isHalted false) and its record keeps SETTLE's result (words 8, 16, 17 as the halted control)
  G3 big     an answer whose stops do not fit fx (2100 ids): the same with status 6
  G3b long   an answer that fits fx but not the store room left after the query (1000 ids x 64 units): the same, 6
  G4 once    the halt request itself hits Grow (arena filled just before it, driver audit): the Grow attempt asks
             nothing and changes nothing, the retry asks exactly once
query_before_settle must build and fail q (the trace order); a variant that does not build (rustc) is BROKEN.
"""

import argparse
from concurrent.futures import ThreadPoolExecutor
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
BUD = HERE.parent
WORK = BUD / "work" / "m10"
sys.path.insert(0, str(HERE))
import run_m10  # noqa: E402

RUN = str(HERE / "run_m10.py")
QUERY = "  let v: Vec<i64> = running_ids(sk, ss, sn, bk, bs, bn, rd(fx, 0));\n"
SETTLE = re.compile(r"  let _a: i64 = crate::bud_clock::settle_at\([^\n]*\);\n")

# Mutants that must build (pass the verifier) and fail the q trace against the oracle.
MUST_BUILD = {"query_before_settle"}

# (name, pattern, replacement); each pattern must match exactly once inside fn halt.
MUTANTS = (
    # the query moved before SETTLE: the trace must show runningDelegationIds before timer.clear
    ("query_before_settle", r"(  let _a: i64 = crate::bud_clock::settle_at\([^\n]*\);\n)(  let v: Vec<i64> = running_ids\([^\n]*\);\n)",
     r"\2\1"),
    ("no_settle", r"  let _a: i64 = crate::bud_clock::settle_at\([^\n]*\);\n", ""),
    ("query_budget_as_session", r"running_ids\(sk, ss, sn, bk, bs, bn,", "running_ids(bk, bs, bn, bk, bs, bn,"),
    ("query_session_as_budget", r"running_ids\(sk, ss, sn, bk, bs, bn,", "running_ids(sk, ss, sn, sk, ss, sn,"),
    ("bad_answer_ok", r"  if iw < 0 \{ return fail_settled\([^\n]*\); \}\n", "  if iw < 0 { return 0; }\n"),
    # a terminal status after SETTLE that drops SETTLE's result (the P24 audit's MF1)
    ("bad_answer_drops_settle", r"  if iw < 0 \{ return fail_settled\([^\n]*\); \}\n", "  if iw < 0 { return 2; }\n"),
    ("big_answer_drops_settle", r"  if g2 != 0 \{ return fail_settled\([^\n]*\); \}\n", "  if g2 != 0 { return 6; }\n"),
    ("no_stop_loop", r"    push\(inout fx, 33\);\n    push\(inout fx, i\);\n", ""),
    ("stop_reversed", r"    push\(inout fx, i\);\n", "    push(inout fx, d - 1 - i);\n"),
    ("cancel_after_stopmain", r"  push\(inout fx, 31\);\n  push\(inout fx, 32\);\n",
     "  push(inout fx, 32);\n  push(inout fx, 31);\n"),
    ("no_save", r"  push\(inout fx, 36\);\n", ""),
    ("big_answer_continues", r"  if g2 != 0 \{ return fail_settled\([^\n]*\); \}\n", "  if g2 != 0 { return 0; }\n"),
)


def mutate(source, name, pattern, replacement):
    m = list(re.finditer(r"^pub fn halt\(.*?\n\{\n(.*?)^\}", source, re.MULTILINE | re.DOTALL))
    assert len(m) == 1, f"{name}: expected one halt function"
    body, n = re.subn(pattern, replacement, m[0][1], flags=re.MULTILINE | re.DOTALL)
    assert n == 1, f"{name}: expected one mutation site, found {n}"
    return source[:m[0].start(1)] + body + source[m[0].end(1):]


# ---- G fixtures ----
def drive(exe, lines, env=None):
    r = subprocess.run([exe, "q"], input="\n".join(lines) + "\n", capture_output=True, text=True,
                       env=dict(os.environ, **(env or {})))
    if r.returncode != 0:
        raise AssertionError("driver exited %d: %s" % (r.returncode, r.stderr[-400:]))
    return r.stdout.splitlines(), r.stderr


def blocks(out):
    """request id -> its log lines (after the '@ r' line, up to the next '@')."""
    res, cur = {}, None
    for line in out:
        if line.startswith("@ "):
            cur = line[2:]
            res[cur] = []
        elif cur is not None and line != "end" and not line.startswith(("case ", "O ")):
            res[cur].append(line)
    return res


def base_program(exe):
    """First derived-cut case whose halt request clears a timer and then asks: (program, halt request id)."""
    for l in open(WORK / "cases_halt.jsonl"):
        lines, _ = run_m10.program(json.loads(l))
        if sum(x.startswith("rdi ") for x in lines) != 1:
            continue
        out, _ = drive(exe, lines)
        for rq, b in blocks(out).items():
            if len(b) > 1 and b[0].startswith("E clear") and b[1].startswith("E rdi"):
                return lines, rq
    raise AssertionError("no corpus halt clears a timer before its query")


def with_answer(lines, words):
    return [("rdi " + " ".join(map(str, words))) if x.startswith("rdi ") else x for x in lines]


def halt_call(lines, rq):
    return next(x for x in lines if x.startswith("call %s " % rq))


def is_halted_after(lines, rq, code=2, before=False):
    """The program cut after (before) the halt request, plus a call `code` (2 isHalted, 1 current) on the same ref."""
    call = halt_call(lines, rq).split()
    i = lines.index(" ".join(call))
    probe = ["call", "rq"] + [str(code)] + call[3:]
    return lines[:i + (0 if before else 1)] + [" ".join(probe), "end"]


def record_after(exe, lines, rq, before=False):
    """The budget record (current, code 1) on the halt's ref after (before) the halt request: its words."""
    v = blocks(drive(exe, is_halted_after(lines, rq, 1, before))[0])["rq"]
    assert len(v) == 1 and v[0].startswith("V 4 "), f"current {v}"
    return [int(x) for x in v[0].split()[3:]]


def terminal_after_settle(exe, prog, rq, status, settled, tag):
    """A halt ending with `status` after SETTLE: the log is exactly clear, rdi, X status; the session is not halted
    and the record keeps SETTLE's result (activeMs word 8, activeSince kind 16 and ms 17 as the halted control)."""
    b = blocks(drive(exe, prog)[0])[rq]
    assert len(b) == 3 and b[0].startswith("E clear") and b[1].startswith("E rdi") and b[2] == f"X {status}", \
        f"{tag} {b[:4]} .. {b[-1:]}"
    probe = blocks(drive(exe, is_halted_after(prog, rq))[0])["rq"]
    assert probe == ["V 1"], f"{tag} isHalted {probe}"  # out [1] = not halted, [2] = halted
    r = record_after(exe, prog, rq)
    got = (r[0], r[8], r[16], r[17])
    assert got == (0,) + settled, f"{tag} record (halted, activeMs, kind, since) {got} != {(0,) + settled}"


def check_fixtures(exe):
    lines, rq = base_program(exe)
    key_x = [1, 120 << 48]  # one u16 lane "x"
    key_y = [1, 121 << 48]
    # G1: the stops carry the host's answer in order
    for ids in ([key_x, key_y], []):
        words = [len(ids)] + [w for k in ids for w in k]
        b = blocks(drive(exe, with_answer(lines, words))[0])[rq]
        stops = [x for x in b if x.startswith("E stopDel")]
        want = ["E stopDel 120", "E stopDel 121"][:len(ids)]
        assert stops == want, f"G1 stops {stops} != {want}"
        assert b[0].startswith("E clear") and b[1].startswith("E rdi"), f"G1 order {b[:2]}"
        assert sum(x.startswith("E rdi") for x in b) == 1, "G1 asked more than once"
    # SETTLE's result: the halted control's words 8, 16, 17; the fixture must change them (a running clock)
    done = record_after(exe, lines, rq)
    pre = record_after(exe, lines, rq, before=True)
    assert done[0] == 1, f"control not halted {done[:1]}"
    settled = (done[8], done[16], done[17])
    assert settled != (pre[8], pre[16], pre[17]), f"SETTLE changed nothing {settled}"
    good = blocks(drive(exe, is_halted_after(lines, rq))[0])["rq"]
    assert good == ["V 2"], f"G2 control isHalted {good}"
    # G2: malformed answer after SETTLE (count says 2, one key) -> 2
    terminal_after_settle(exe, with_answer(lines, [2] + key_x), rq, 2, settled, "G2")
    # G3: 2100 ids cannot fit fx -> 6
    terminal_after_settle(exe, with_answer(lines, [2100] + key_x * 2100), rq, 6, settled, "G3")
    # G3b: 1000 ids of 64 units fit fx (8 + 2000 words) but not the store room left after the query -> 6
    long_key = [64] + [(120 << 48) | (120 << 32) | (120 << 16) | 120] * 16
    terminal_after_settle(exe, with_answer(lines, [1000] + long_key * 1000), rq, 6, settled, "G3b")
    # G4: the halt request itself hits Grow (arena filled just before it): the first attempt returns Grow with no
    # query, no effect and the store unchanged; the retry asks exactly once and halts
    call = halt_call(lines, rq)
    assert call.split()[2] == "12", f"G4 halt is not a direct halt call: {call[:40]}"
    i = lines.index(call)
    grown = lines[:i] + ["fill_arena", "audit"] + lines[i:]
    b = blocks(drive(exe, grown)[0])[rq]
    att = [x.split() for x in b if x.startswith("A ")]
    assert len(att) >= 2 and int(att[0][1]) < 0, f"G4 halt did not Grow: {att[:1]}"
    assert att[0][2:5] == ["0", "true", "true"], f"G4 Grow attempt asked or wrote: {att[0][:6]}"
    assert all(int(x[1]) < 0 and x[2] == "0" for x in att[:-1]), f"G4 attempts {att}"
    assert int(att[-1][1]) == 0 and att[-1][2] == "1", f"G4 retry {att[-1][:6]}"
    assert sum(x.startswith("E rdi") for x in b) == 1, f"G4 {b}"
    return "G1-G4,G3b"


# ---- variants ----
def q(budlib, out, cases, oracle, pruvc):
    r = subprocess.run([sys.executable, RUN, "q", "--pruvc", pruvc, "--budlib", str(budlib), "--out", str(out),
                        "--cases", str(cases), "--oracle", str(oracle)], capture_output=True, text=True,
                       cwd=run_m10.ROOT)
    (out / f"{cases.stem}.log").write_text(r.stdout + r.stderr, encoding="utf-8")
    if "pruvc failed" in r.stderr:
        return "rejected", r.stderr
    if "rustc failed" in r.stderr:
        return "broken", r.stderr[-160:]
    if r.returncode == 0:
        return "pass", ""
    lines = [x for x in (r.stdout + r.stderr).splitlines() if x.strip()]
    why = next((x for x in lines if x.startswith(("FAIL", "AssertionError"))), lines[-1] if lines else "")
    return "fail", why[:160]


def check_variant(name, source, campaign, pruvc):
    lib = campaign / name
    shutil.copytree(BUD, lib, ignore=shutil.ignore_patterns("work", "__pycache__", "m*_test", "*.md"))
    (lib / "bud_halt.pruv").write_text(source, encoding="utf-8")
    out = lib / "build"
    out.mkdir()
    res = {}
    for cases in ("cases", "cases_halt"):
        oracle = "oracle" + cases[len("cases"):]
        res[cases] = q(lib, out, WORK / f"{cases}.jsonl", WORK / f"{oracle}.jsonl", pruvc)
        if res[cases][0] == "broken":  # a variant that does not build is no result: neither killed nor clean
            return (name, "-", "-", "-", "BROKEN  # " + res[cases][1].replace("\n", " "))
        if res[cases][0] == "rejected":
            if name in MUST_BUILD:
                return (name, "-", "-", "-", "SURVIVED  # rejected by proof but must build and fail its run")
            return (name, "-", "-", "-", "REJECTED" if name != "control" else "DIRTY")
    try:
        g = ("pass", check_fixtures(str(out / "driver_m10")))
    except AssertionError as e:
        g = ("fail", str(e)[:160])
    caught = any(r[0] == "fail" for r in (*res.values(), g))
    if name in MUST_BUILD:  # must fail the oracle trace on the query order itself: not only a G fixture, and not a
        # driver crash or a Grow control (the reason names the query: 'X armed' or a runningDelegationIds mismatch)
        caught = any(r[0] == "fail" and ("X armed" in r[1] or "runningDelegationIds" in r[1]) for r in res.values())
    verdict = ("DIRTY" if caught else "CLEAN") if name == "control" else ("KILLED" if caught else "SURVIVED")
    why = next((r[1] for r in (*res.values(), g) if r[0] == "fail"), "")
    return (name, res["cases"][0], res["cases_halt"][0], g[0], verdict + (("  # " + why) if why else ""))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--pruvc", default=run_m10.PRUVC_DEFAULT)
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--only", help="comma-separated variant names")
    a = ap.parse_args()
    src = (BUD / "bud_halt.pruv").read_text(encoding="utf-8")
    assert QUERY in src and SETTLE.search(src), "bud_halt.pruv no longer has the expected query / SETTLE lines"
    variants = {"control": src}
    for name, pattern, repl in MUTANTS:
        variants[name] = mutate(src, name, pattern, repl)
    if a.only:
        keep = set(a.only.split(","))
        variants = {k: v for k, v in variants.items() if k in keep}
    (WORK / "mut").mkdir(parents=True, exist_ok=True)
    campaign = Path(tempfile.mkdtemp(prefix="campaign-", dir=WORK / "mut"))
    print(f"campaign {campaign}", file=sys.stderr, flush=True)
    print("name q cases_halt G verdict", flush=True)
    failed = False
    with ThreadPoolExecutor(max_workers=a.jobs) as pool:
        futs = [pool.submit(check_variant, n, s, campaign, a.pruvc) for n, s in variants.items()]
        for f in futs:
            row = f.result()
            print(*row, flush=True)
            failed |= row[-1].startswith(("DIRTY", "SURVIVED", "BROKEN"))
    return int(failed)


if __name__ == "__main__":
    sys.exit(main())
