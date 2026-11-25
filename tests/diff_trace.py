#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Check, per input, the trace of Rust pruv-trace-dump against a saved golden file (M3 stage 3).

  python tests/diff_trace.py [--rs-path P] [--only GROUP[,GROUP]] [--seeds 41,56,57]
                             [--fuzz-count 40] [--jobs N] [--save-dir DIR] [--input FILE ...]
                             [--flags "STR"] [--golden PATH] [--update-golden] [--prune-golden]
                             [--vs-py [--python P]]

Default mode (golden): only the Rust binary is run (no Python compiler, no z3). The trace of each input is hashed
  (tests/golden.py) and compared with tests/golden/diff_trace.txt (with --flags: tests/golden/diff_trace_<flags>.txt,
  e.g. --flags=--infer -> diff_trace_infer.txt; override with --golden PATH). Key per case:
  "<group>|<repo-relative input path or fuzz name>|<flags, or "default">".
  Machine/run dependent parts are normalized before hashing: for attempts whose result is "unknown" the z3
  reason_unknown text and the status (resource_unknown / timeout / ...; they depend on where an rlimit or wall clock
  stopped z3), and the status of dropped inference candidates (not_proved / resource_unknown / timeout). Temp
  directories become <TMP> and the repo root <ROOT> (golden.normalize). An inference_timeout result is still
  wall-clock dependent and shows up as a golden mismatch on a slow machine.
  Per group: same (golden match), golden_mismatch, golden_missing, rs_crash (crash, timeout, not one JSON line,
  or wrong "options"). Exit 0 only if everything matches and nothing crashed.
--update-golden: rewrite the golden file from this run's Rust output and print every CHANGED / NEW / REMOVED key
  (exit 0 unless the Rust side crashed). --prune-golden (with --update-golden) also drops keys this run did not produce.
--vs-py: the OLD behaviour below, Python against Rust. Frozen Python reference, not run by CI (needs z3; pruvc/ is frozen
  since bundle PF). With --vs-py --update-golden both sides are run, the golden file is written from the Rust output,
  and py-vs-rs disagreements are reported as before (exit 0 only if there is no diff/crash).

Old (--vs-py) behaviour:
Groups: must_reject examples runtime bench check_edge g6 g9 g4 g4_reject fuzz input
  (g6 = tests/g6/*/*.pruv, g9 = tests/g9/{frontend,proof}/*.pruv,
   g4 = tests/g4/{frontend,proof}/*/root.pruv, g4_reject = tests/must_reject/g4_*/root.pruv)
  (if only --input is given and there is no --only, only the input group runs)
For each input, Python (`PY tests/diff_py_adapter.py trace FILE`) and Rust (`pruv-trace-dump FILE`) are each
run in a new process (no shell, timeout 300). The two runs for one input happen one after the other in the same worker.

Classification (exactly one per input):
  option_mismatch the "options" of the two outputs differ from the expected value (expected = {"infer": "--infer" in --flags,
                  "no_verify": "--no-verify" in --flags}) (fail).
                  Checked before any other classification.
  same          the one stdout line of each is byte-identical, no attempt result is unknown/error, and errors has no
                verify_unknown or inference_timeout
  same_unknown  byte-identical (or differing only in counterexample values or in the reason_unknown text of a resource_unknown attempt, or in the reason text and status of
                inference attempts that both answered unknown to the same query), but the
                unknown/error/verify_unknown/inference_timeout above is present. Not counted as success. If the py or rs errors contain inference_timeout it is a fail;
                otherwise it is not treated as a fail but is reported separately
  cex_only      only the counterexample values in errors differ and everything else is identical, provided neither side has unknown/error/verify_unknown/
                inference_timeout (counted as pass; the count is reported separately). If any is present it is classified as same_unknown
  flaky_unknown either side's attempt result is "unknown" and that causes the difference, or either side's errors contain
                inference_timeout (wall-clock 5 s limit, depends on run time) and that causes the difference -> only that input is re-run
                once serially (--jobs 1) and re-judged. If it resolves to same/cex_only it is counted in this column (not added to same in the success column;
                the first run's difference summary is kept and reported as "differences resolved by retry (first result)"). If it still differs, it is diff.
                Also: a function's "inference" entry has status "timeout" (5 s wall clock) on either side. If it
                still differs after the serial retry it stays flaky_unknown (noted; not comparable on a machine
                this slow) instead of diff.
  py_recursion  Python recursion-limit internal_error (nothing to compare)
  diff          any other difference
  py_crash / rs_crash   exit code != 0, no output line, or not JSON
  timeout       either side timed out
Separate tally verified: the number of inputs whose status is same or cex_only and whose Python errors are empty (both implementations agree and verification completed).
--flags=STR (an option-shaped value, so attach it with `=`, e.g. --flags=--infer): the whitespace-split tokens are passed identically to Python (`adapter trace FLAGS FILE`) and Rust (`pruv-trace-dump FLAGS FILE`).
Exit codes: 0 = diff, crash, timeout and option_mismatch are all 0 and there is no same_unknown containing inference_timeout,
          1 = anything else, 2 = bad arguments / binary missing.
Uses only the Python 3.9 standard library.
"""
import argparse
import collections
import concurrent.futures
import glob
import json
import os
import re
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, HERE)

import golden  # noqa: E402

PROC_TIMEOUT = 300
MAX_SHOWN = 10
CEX_KEY = "counterexample"
GROUP_DIRS = [
    ("must_reject", "tests/must_reject", "*.pruv"),
    ("examples", "examples", "*.pruv"),
    ("runtime", "tests/runtime", "*.pruv"),
    ("bench", "bench/solutions/pruv_agg", "*.pruv"),
    ("bench_refine", "bench/solutions/pruv_refine", "*.pruv"),
    ("check_edge", "tests/check_edge", "ok_*.pruv"),
    ("inference", "tests/inference", "inf_*.pruv"),
    ("bench_noinv", "bench/inference/agg_noinv", "*.pruv"),
    ("g6", "tests/g6", "*/*.pruv"),
    ("g9", "tests/g9", "*/*.pruv"),
    # G4 A1: project roots only (companion modules are never standalone inputs): tests/g4/{frontend,proof}/<project>/root.pruv
    # and the rejection projects tests/must_reject/g4_*/root.pruv
    ("g4", "tests/g4", "*/*/root.pruv"),
    ("g4_reject", "tests/must_reject", "g4_*/root.pruv"),
]
ALL_GROUPS = [g for g, _, _ in GROUP_DIRS] + ["fuzz", "input"]
COLS = ("inputs", "same", "same_unknown", "cex_only", "flaky_unknown", "py_recursion", "diff", "option_mismatch",
        "py_crash", "rs_crash", "timeout")
GOLDEN_COLS = ("inputs", "same", "golden_mismatch", "golden_missing", "rs_crash")


class Case(object):
    def __init__(self, group, name, path):
        self.group, self.name, self.path = group, name, path
        self.status = None
        self.note = ""
        self.py_raw = None      # bytes (one line, newline excluded) or None
        self.rs_raw = None
        self.py_obj = None
        self.rs_obj = None
        self.flaky_first = False   # was it first judged flaky_unknown and retried?
        self.first_summary = None  # diff_summary result of the first run (kept across the retry)
        self.first_status = None   # status of the first run


def rel(path):
    return os.path.relpath(path, ROOT).replace(os.sep, "/")


def safe(name):
    return re.sub(r"[^A-Za-z0-9_.+-]", "_", name)[:80]


def parse_seed(tok):
    return int(tok) if re.fullmatch(r"-?[0-9]+", tok) else tok


# ---------------------------------------------------------------------------
# Collecting inputs
# ---------------------------------------------------------------------------
def build_cases(selected, args, tmp):
    per_group = collections.OrderedDict((g, []) for g in ALL_GROUPS)
    notes = []
    for g, sub, pat in GROUP_DIRS:
        if g not in selected:
            continue
        files = sorted(glob.glob(os.path.join(ROOT, *sub.split("/"), pat)))
        if not files:
            notes.append("warning: no %s in %s (%s)" % (g, pat, sub))
        for p in files:
            per_group[g].append(Case(g, rel(p), p))
    if "fuzz" in selected:
        d = os.path.join(tmp, "fuzz")
        os.makedirs(d)
        sys.path.insert(0, os.path.join(ROOT, "bench", "fuzz"))
        try:
            import generate
        except Exception as e:   # noqa: BLE001
            print("error: could not load the fuzzer generator: %s: %s" % (type(e).__name__, e), file=sys.stderr)
            raise SystemExit(2)
        skipped = 0
        for seed in args.seeds:
            for idx in range(args.fuzz_count):
                try:
                    data = generate.generate_case(seed, idx)["src"].encode("utf-8")
                except Exception:   # noqa: BLE001
                    skipped += 1
                    continue
                p = os.path.join(d, "s%s_%05d.pruv" % (safe(str(seed)), idx))
                with open(p, "wb") as fh:
                    fh.write(data)
                per_group["fuzz"].append(Case("fuzz", "fuzz:%s:%d" % (seed, idx), p))
        notes.append("fuzz: %d indexes skipped due to generator exceptions (seeds=%s, count=%d)" % (
            skipped, ",".join(str(s) for s in args.seeds), args.fuzz_count))
    if "input" in selected:
        for p in args.input:
            per_group["input"].append(Case("input", os.path.abspath(p), os.path.abspath(p)))
    return per_group, notes


# ---------------------------------------------------------------------------
# Running
# ---------------------------------------------------------------------------
def run_one_proc(argv, env):
    """-> (kind, raw_line|None, obj|None, note)  kind: ok / crash / timeout"""
    try:
        p = subprocess.run(argv, capture_output=True, timeout=PROC_TIMEOUT, env=env)
    except subprocess.TimeoutExpired:
        return "timeout", None, None, "timeout (%d s)" % PROC_TIMEOUT
    except OSError as e:
        return "crash", None, None, "failed to start: %s" % e
    tail = p.stderr.decode("utf-8", "replace")[-300:].strip()
    if p.returncode != 0:
        return "crash", None, None, "exit code %d: %s" % (p.returncode, tail)
    out = p.stdout
    if out.endswith(b"\n"):
        out = out[:-1]
    if not out or b"\n" in out:
        return "crash", None, None, "stdout is not a single line (%d bytes) %s" % (len(p.stdout), tail)
    try:
        obj = json.loads(out.decode("utf-8"))
    except (ValueError, RecursionError):
        return "crash", None, None, "not JSON: %s" % out[:200].decode("utf-8", "replace")
    if not isinstance(obj, dict):
        return "crash", None, None, "not a JSON object"
    return "ok", out, obj, ""


def evaluate(case, py_argv, rs_argv, env, expected):
    case.py_raw = case.rs_raw = case.py_obj = case.rs_obj = None
    case.note = ""
    k, case.py_raw, case.py_obj, note = run_one_proc(py_argv + [case.path], env)
    if k != "ok":
        case.status = "timeout" if k == "timeout" else "py_crash"
        case.note = "py: " + note
        # run Rust too, so it is kept in the saved case
    k2, case.rs_raw, case.rs_obj, note2 = run_one_proc(rs_argv + [case.path], env)
    if k != "ok":
        return
    if k2 != "ok":
        case.status = "timeout" if k2 == "timeout" else "rs_crash"
        case.note = "rs: " + note2
        return
    case.status = classify(case, expected)


def evaluate_rs(case, rs_argv, env, expected):
    """Golden mode: run only Rust. status: "rs_ok" (placeholder until the golden check) or "rs_crash"."""
    case.py_raw = case.rs_raw = case.py_obj = case.rs_obj = None
    case.note = ""
    k, case.rs_raw, case.rs_obj, note = run_one_proc(rs_argv + [case.path], env)
    if k != "ok":
        case.status = "rs_crash"
        case.note = "rs: " + note
    elif case.rs_obj.get("options") != expected:
        case.status = "rs_crash"
        case.note = "rs: options %s, expected %s" % (short(case.rs_obj.get("options")), json.dumps(expected, sort_keys=True))
    else:
        case.status = "rs_ok"


def rs_usable(case, expected):
    return case.rs_raw is not None and isinstance(case.rs_obj, dict) and case.rs_obj.get("options") == expected


def golden_normalize(obj):
    """Copy of a Rust trace object without the parts that depend on the machine / run (not on the compiler):
    - attempts whose result is "unknown": reason_unknown and status (z3 words the stop by where its rlimit or wall
      clock ran out: "max. resource limit exceeded", "push canceled", "timeout", ...);
    - inference "dropped" candidates with status not_proved / resource_unknown / timeout: status (same cause).
    - counterexample values in errors (z3 may return another valid model; presence is kept, as strip_cex does).
    Only fields that are present and statuses z3 can produce for "unknown" are replaced, so a dropped field or a wrong
    status class still changes the digest. Everything else (smt2, results, errors) is kept."""
    o = strip_cex(obj)
    obs = []
    for ob in o.get("obligations", []):
        if isinstance(ob, dict) and ob.get("attempts"):
            ob = dict(ob)
            ob["attempts"] = [_unknown_attempt(a) for a in ob["attempts"]]
        obs.append(ob)
    o["obligations"] = obs
    if isinstance(o.get("inference"), list):
        o["inference"] = [dict(e, dropped=[dict(d, status="<dropped>")
                                           if isinstance(d, dict) and d.get("status") in DROP_STATUSES + ("timeout",) else d
                                           for d in e["dropped"]])
                          if isinstance(e, dict) and e.get("dropped") else e for e in o["inference"]]
    return o


UNKNOWN_STATUSES = ("resource_unknown", "timeout")   # query_status of an "unknown" z3 answer


def _unknown_attempt(a):
    if not (isinstance(a, dict) and a.get("result") == "unknown"):
        return a
    a = dict(a)
    if "reason_unknown" in a:
        a["reason_unknown"] = "<reason>"
    if a.get("status") in UNKNOWN_STATUSES:
        a["status"] = "<status>"
    return a


def golden_text(case):
    """The text that is hashed: the raw Rust line, or its canonical dump when golden_normalize changed something."""
    norm = golden_normalize(case.rs_obj)
    if norm == case.rs_obj:
        return case.rs_raw
    return canon(norm)


def golden_digest(case, gold):
    """golden.digest of golden_text, computed once per Rust result (retry and check both use it)."""
    if getattr(case, "gdig_raw", None) is not case.rs_raw:
        case.gdig_raw, case.gdig = case.rs_raw, golden.digest(golden_text(case), gold.tmp_dirs)
    return case.gdig


def golden_summary(obj):
    return "obl=%d errs=%d" % (len(obj.get("obligations", [])), len(obj.get("errors", [])))


def golden_default_name(flag_toks):
    tag = re.sub(r"_+", "_", re.sub(r"[^a-z0-9_]", "_", "_".join(flag_toks).lower())).strip("_")
    return "diff_trace_%s.txt" % tag if tag else "diff_trace.txt"


def golden_keys(all_cases, expected, label):
    """[(case, golden key)] for every case with a usable Rust trace, in case order."""
    keys, out = {}, []
    for c in all_cases:
        if not rs_usable(c, expected):
            continue
        name = rel(c.path) if c.group == "input" else c.name   # never an absolute / temp path
        key = "%s|%s|%s" % (c.group, name, label)
        n = keys.get(key, 0)
        keys[key] = n + 1
        if n:
            key += "~%d" % (n + 1)   # deterministic disambiguation of a repeated case name
        out.append((c, key))
    return out


RETRY_MAX = 10   # golden mismatches rerun serially at most (solver stops under load hit a few cases, not hundreds)


def golden_retry_mismatches(gold, all_cases, expected, label, rs_argv, env):
    """Golden mode, before golden_check_all: rerun serially (--jobs 1) each case whose digest differs from the file.
    A z3 query that hits its wall-clock timeout_ms on a loaded machine changes a trace (seen on
    g10a_r10_proc_drain while other captures ran in parallel); a real change still differs after the retry."""
    retried = []
    differ = [c for c, key in golden_keys(all_cases, expected, label)
              if key in gold.saved and golden_digest(c, gold) != gold.saved[key][0]]
    if len(differ) > RETRY_MAX:
        # many cases changed: a real (e.g. format) change, not a solver stop under load; rerunning them serially
        # would only make the failing run slow
        print("golden: %d cases differ from the file; not rerun (more than %d)" % (len(differ), RETRY_MAX))
        return retried
    for c in differ:
        evaluate_rs(c, rs_argv, env, expected)
        c.note = (c.note + " " if c.note else "") + "(golden retry)"
        retried.append(c)
    return retried


def golden_check_all(gold, all_cases, expected, label, vs_py):
    """g.check for every case with a usable Rust trace, in case order. Not vs_py: sets the final status."""
    for c, key in golden_keys(all_cases, expected, label):
        miss = len(gold.missing)
        ok = gold.check(key, None, golden_summary(c.rs_obj), dg=golden_digest(c, gold))
        if not vs_py:
            c.status = "same" if ok else ("golden_missing" if len(gold.missing) > miss else "golden_mismatch")


def strip_cex(obj):
    """Copy with the counterexample values in errors replaced by a placeholder (key presence is still compared)"""
    o = dict(obj)
    if isinstance(o.get("errors"), list):   # a missing or non-list errors field stays as it is (it must change the digest)
        errs = []
        for e in o["errors"]:
            if isinstance(e, dict) and CEX_KEY in e:
                e = dict(e)
                e[CEX_KEY] = "<cex>"
            errs.append(e)
        o["errors"] = errs
    return o


RLIMIT_REASONS = ("max. resource limit exceeded", "push canceled")   # z3 rlimit-exhaustion texts (same cause)


def strip_reason(obj):
    """Copy with the reason_unknown text of rlimit-exhausted (RLIMIT_REASONS) resource_unknown attempts replaced by a placeholder. For the same SMT,
    z3 emits different texts such as "max. resource limit exceeded" / "push canceled" depending on where the rlimit ran out (status and result are still compared)"""
    o = dict(obj)
    obs = []
    for ob in o.get("obligations", []):
        if isinstance(ob, dict) and ob.get("attempts"):
            ob = dict(ob)
            ob["attempts"] = [dict(a, reason_unknown="<reason>") if isinstance(a, dict) and a.get("result") == "unknown"
                              and a.get("status") == "resource_unknown" and a.get("reason_unknown") in RLIMIT_REASONS
                              else a for a in ob["attempts"]]
        obs.append(ob)
    o["obligations"] = obs
    return o


def canon(obj):
    return json.dumps(obj, sort_keys=True, ensure_ascii=False)


def results_of(o):
    for ob in o.get("obligations", []):
        for a in ob.get("attempts", []):
            yield a.get("result")


def unknown_involved(py, rs):
    """Is "unknown" involved in an attempt pair whose results differ, or does a count appear to differ while an unknown is present?"""
    po, ro = py.get("obligations", []), rs.get("obligations", [])
    seen_unknown = "unknown" in set(results_of(py)) or "unknown" in set(results_of(rs))
    if not seen_unknown:
        return False
    if len(po) != len(ro):
        return True
    for a, b in zip(po, ro):
        aa, ba = a.get("attempts", []), b.get("attempts", [])
        if len(aa) != len(ba):
            return True
        for x, y in zip(aa, ba):
            if x.get("result") != y.get("result") and "unknown" in (x.get("result"), y.get("result")):
                return True
    return False


DROP_STATUSES = ("not_proved", "resource_unknown")


def same_query_dropped(py, rs):
    """Do the two sides differ only in inference candidate queries whose SMT text is byte-identical, where one side
    answered sat and the other stopped at the rlimit, so both dropped the candidate? The compilers built the same
    query; the rlimit stop depends on z3's internal term order (G10 async Loop queries with `--infer`: same text,
    py sat / rs "max. resource limit exceeded"). The final inference result and errors must still be identical."""
    po, ro = py.get("obligations", []), rs.get("obligations", [])
    if len(po) != len(ro):
        return False
    py2, rs2 = dict(py), dict(rs)
    obs_p, obs_r, seen = [], [], False
    for a, b in zip(po, ro):
        aa, ba = a.get("attempts", []), b.get("attempts", [])
        if len(aa) != len(ba):
            return False
        na = []
        for x, y in zip(aa, ba):
            if x.get("result") != y.get("result"):
                pair = {x.get("result"), y.get("result")}
                unk = x if x.get("result") == "unknown" else y
                if (a.get("phase") != "inference" or pair != {"sat", "unknown"} or x.get("smt2") != y.get("smt2")
                        or unk.get("status") != "resource_unknown" or unk.get("reason_unknown") not in RLIMIT_REASONS):
                    return False
                seen = True
                na.append(x)
            else:
                na.append(y)
        obs_p.append(a)
        obs_r.append(dict(b, attempts=na))
    if not seen:
        return False

    def drops(inf):
        return [dict(e, dropped=[dict(d, status="<dropped>") if d.get("status") in DROP_STATUSES else d
                                 for d in (e.get("dropped") or [])]) for e in (inf or [])]
    py2["obligations"], rs2["obligations"] = obs_p, obs_r
    py2["inference"], rs2["inference"] = drops(py.get("inference")), drops(rs.get("inference"))
    return canon(strip_cex(py2)) == canon(strip_cex(rs2))


def same_query_unknown(py, rs):
    """Do the two sides differ only in the reason_unknown text and status of inference attempts that both answered
    unknown to a byte-identical query? Under an rlimit, z3 words the stop by where it ran out, and that depends on the
    context's history: verify.py's global context gives "incomplete quantifiers" or "canceled" (status timeout) where
    the Rust verifier, which checks in its own shared context and retries in a fresh one, gives "push canceled" or
    "max. resource limit exceeded" (LG21, CI run of 6d767fae: g10a_v_havoc_eof, inf_budget). Everything else, the
    inference result and the errors included, must be identical."""
    po, ro = py.get("obligations", []), rs.get("obligations", [])
    if len(po) != len(ro):
        return False
    obs_r, seen = [], False
    for a, b in zip(po, ro):
        aa, ba = a.get("attempts", []), b.get("attempts", [])
        if len(aa) != len(ba):
            return False
        na = []
        for x, y in zip(aa, ba):
            if x != y and x.get("result") == y.get("result") == "unknown" and a.get("phase") == "inference" \
                    and x.get("smt2") == y.get("smt2"):
                seen = True
                na.append(x)
            else:
                na.append(y)
        obs_r.append(dict(b, attempts=na))
    if not seen:
        return False
    return canon(strip_cex(py)) == canon(strip_cex(dict(rs, obligations=obs_r)))


def py_recursion(obj):
    """Did Python emit internal_error because of the recursion limit (RecursionError)? (nothing to compare; same category as in diff_parse)"""
    return any(isinstance(e, dict) and e.get("code") == "internal_error"
               and "maximum recursion depth" in str(e.get("message", "")) for e in obj.get("errors", []))


def error_codes(obj):
    return [e.get("code") for e in obj.get("errors", []) if isinstance(e, dict)]


def has_unknown_or_error(obj):
    """Is an attempt result unknown/error, or does errors contain verify_unknown/inference_timeout?"""
    if any(r in ("unknown", "error") for r in results_of(obj)):
        return True
    return any(c in ("verify_unknown", "inference_timeout") for c in error_codes(obj))


def classify(case, expected):
    if case.py_obj.get("options") != expected or case.rs_obj.get("options") != expected:
        return "option_mismatch"
    if case.py_raw == case.rs_raw:
        if has_unknown_or_error(case.py_obj) or has_unknown_or_error(case.rs_obj):
            return "same_unknown"
        return "same"
    if py_recursion(case.py_obj):
        return "py_recursion"
    if canon(strip_cex(case.py_obj)) == canon(strip_cex(case.rs_obj)):
        if has_unknown_or_error(case.py_obj) or has_unknown_or_error(case.rs_obj):
            return "same_unknown"
        return "cex_only"
    if canon(strip_reason(strip_cex(case.py_obj))) == canon(strip_reason(strip_cex(case.rs_obj))):
        return "same_unknown"   # only the z3 text of resource_unknown differs (that attempt is unknown, so it is not counted as success)
    if same_query_dropped(case.py_obj, case.rs_obj) or same_query_unknown(case.py_obj, case.rs_obj):
        return "same_unknown"
    if unknown_involved(case.py_obj, case.rs_obj):
        return "flaky_unknown"
    if ("inference_timeout" in error_codes(case.py_obj) or "inference_timeout" in error_codes(case.rs_obj)) \
            and case.py_obj.get("errors") != case.rs_obj.get("errors"):
        return "flaky_unknown"   # a wall-clock limit (depends on run time) is involved in the difference -> serial retry
    if inference_wall_timeout(case):
        # A function's inference hit the wall-clock limit (slow or busy machine, e.g. a CI runner):
        # how far each side got depends on speed -> retry serially like inference_timeout.
        return "flaky_unknown"
    return "diff"


def solver_wall_timeout(case):
    """Did a solver attempt on either side end "unknown" because of the per-query wall-clock timeout (z3
    reason_unknown "timeout")? Unlike an rlimit stop, that depends on the speed of the machine."""
    return any(isinstance(a, dict) and a.get("result") == "unknown" and "timeout" in str(a.get("reason_unknown") or "")
               for o in (case.py_obj, case.rs_obj) if o
               for ob in o.get("obligations", []) for a in ob.get("attempts", []))


def inference_wall_timeout(case):
    """Did either side stop a function's invariant inference at the wall-clock limit (status "timeout"), or drop a
    candidate because its query hit the per-query wall-clock timeout (dropped status "timeout"; the other side may
    report resource_unknown for the same candidate, seen on p2_rt_kernels in CI runs of 36f73c9 and 6f0b835)?"""
    return any(isinstance(e, dict) and (e.get("status") == "timeout" or any(
                   isinstance(d, dict) and d.get("status") == "timeout" for d in (e.get("dropped") or [])))
               for o in (case.py_obj, case.rs_obj) if o for e in (o.get("inference") or []))


# ---------------------------------------------------------------------------
# diff summary
# ---------------------------------------------------------------------------
def smt_context(a, b):
    la, lb = a.split("\n"), b.split("\n")
    i = 0
    n = min(len(la), len(lb))
    while i < n and la[i] == lb[i]:
        i += 1
    lo, hi = max(0, i - 3), i + 4
    out = ["    first differing line number %d (py %d lines, rs %d lines)" % (i + 1, len(la), len(lb))]
    out.append("    py:")
    out += ["      %s%s" % ("> " if lo + k == i else "  ", s) for k, s in enumerate(la[lo:hi])]
    out.append("    rs:")
    out += ["      %s%s" % ("> " if lo + k == i else "  ", s) for k, s in enumerate(lb[lo:hi])]
    return out


def short(v, n=200):
    s = json.dumps(v, ensure_ascii=False, sort_keys=True)
    return s if len(s) <= n else s[:n] + "..."


def diff_summary(py, rs):
    lines = []
    po, ro = py.get("obligations", []), rs.get("obligations", [])
    lines.append("obligation count: py %d / rs %d" % (len(po), len(ro)))
    for k in sorted(set(py) | set(rs)):
        if k in ("errors", "obligations"):
            continue
        if py.get(k) != rs.get(k):
            lines.append("top-level key %s differs: py=%s rs=%s" % (k, short(py.get(k)), short(rs.get(k))))
    pe, re_ = py.get("errors", []), rs.get("errors", [])
    pes, res = strip_cex(py).get("errors", []), strip_cex(rs).get("errors", [])
    if pes != res:
        lines.append("errors count: py %d / rs %d" % (len(pe), len(re_)))
        for i in range(max(len(pes), len(res))):
            a = pes[i] if i < len(pes) else None
            b = res[i] if i < len(res) else None
            if a != b:
                keys = sorted(set(a or {}) | set(b or {})) if isinstance(a, dict) and isinstance(b, dict) else []
                lines.append("first differing error #%d (compared excluding counterexample values)" % i)
                if keys:
                    for k in keys:
                        if a.get(k) != b.get(k):
                            lines.append("  key %s: py=%s rs=%s" % (k, short(a.get(k)), short(b.get(k))))
                else:
                    lines.append("  py=%s rs=%s" % (short(a), short(b)))
                break
    pm = {o.get("seq"): o for o in po}
    rm = {o.get("seq"): o for o in ro}
    seqs = sorted(set(pm) | set(rm), key=lambda x: (x is None, x))
    shown = False
    for sq in seqs:
        a, b = pm.get(sq), rm.get(sq)
        if a is None or b is None:
            lines.append("first differing obligation seq=%s: present only in %s" % (sq, "py" if b is None else "rs"))
            lines.append("  the side that has it: %s" % short({k: v for k, v in (a or b).items() if k != "attempts"}))
            shown = True
            break
        if a == b:
            continue
        lines.append("first differing obligation seq=%s (fn=%s line=%s)" % (sq, a.get("fn"), a.get("line")))
        for k in ("code", "line", "col", "fn", "skipped"):
            if a.get(k) != b.get(k):
                lines.append("  %s: py=%s rs=%s" % (k, short(a.get(k)), short(b.get(k))))
        aa, ba = a.get("attempts", []), b.get("attempts", [])
        if len(aa) != len(ba):
            lines.append("  attempt count: py %d / rs %d" % (len(aa), len(ba)))
        for j in range(min(len(aa), len(ba))):
            x, y = aa[j], ba[j]
            if x == y:
                continue
            lines.append("  attempt #%d differs:" % j)
            for k in ("lemmas", "result"):
                if x.get(k) != y.get(k):
                    lines.append("    %s: py=%s rs=%s" % (k, short(x.get(k)), short(y.get(k))))
            if x.get("smt2") != y.get("smt2"):
                lines.append("    smt2 differs:")
                lines += smt_context(str(x.get("smt2")), str(y.get("smt2")))
            break
        shown = True
        break
    if not shown and not any(l.startswith("first differing error") for l in lines):
        lines.append("(no difference found in the structural summary: may be a byte-level difference such as key order or whitespace)")
    return lines


def case_summary(case):
    lines = ["group: %s" % case.group, "name: %s" % case.name, "status: %s" % case.status]
    if case.flaky_first:
        lines.append("first judged flaky_unknown, so it was retried serially")
    if case.note:
        lines.append("note: " + case.note)
    if case.status == "diff":
        lines += diff_summary(case.py_obj, case.rs_obj)
    return lines


def save_case(save_dir, seq, case):
    d = os.path.join(save_dir, case.group, "%04d_%s" % (seq, safe(case.name)))
    os.makedirs(d, exist_ok=True)
    with open(case.path, "rb") as fh, open(os.path.join(d, "input.pruv"), "wb") as out:
        out.write(fh.read())
    for fn, raw in (("py.json", case.py_raw), ("rs.json", case.rs_raw)):
        if raw is not None:
            with open(os.path.join(d, fn), "wb") as out:
                out.write(raw + b"\n")
    with open(os.path.join(d, "diff.txt"), "wb") as out:
        out.write(("\n".join(case_summary(case)) + "\n").encode("utf-8"))


# ---------------------------------------------------------------------------
def default_rs_path():
    base = os.path.join(ROOT, "pruvc-rs", "target", "release", "pruv-trace-dump")
    if not os.path.isfile(base) and os.path.isfile(base + ".exe"):
        return base + ".exe"
    return base


def default_python(explicit):
    want = explicit or os.environ.get("PRUV_PYTHON")
    if want:
        return want
    for r in (".venv/bin/python", ".venv/Scripts/python.exe"):
        p = os.path.join(ROOT, r)
        if os.path.isfile(p):
            return p
    return sys.executable


def main(argv):
    ap = argparse.ArgumentParser(prog="diff_trace.py", description="Compare Python/Rust verifier traces")
    ap.add_argument("--rs-path", default=None, help="path to pruv-trace-dump")
    ap.add_argument("--python", default=None, help="Python used to run the canonical implementation (default .venv/bin/python)")
    ap.add_argument("--only", default=None, help="groups (comma-separated list): " + " ".join(ALL_GROUPS))
    ap.add_argument("--seeds", default="41,56,57")
    ap.add_argument("--fuzz-count", type=int, default=40)
    ap.add_argument("--jobs", type=int, default=max(1, (os.cpu_count() or 2) // 2))
    ap.add_argument("--save-dir", default=None)
    ap.add_argument("--input", action="append", default=[], metavar="FILE", help="file for the input group (may be repeated)")
    ap.add_argument("--flags", default="", help="options passed identically to both implementations (whitespace-separated), e.g. --flags=--infer")
    ap.add_argument("--vs-py", action="store_true",
                    help="old behaviour: compare with the Python verifier (frozen Python reference, not run by CI)")
    golden.add_args(ap)
    args = ap.parse_args(argv)
    vs_py = args.vs_py
    capture = vs_py and args.update_golden       # --vs-py --update-golden: run both, write golden from Rust
    use_golden = (not vs_py) or capture
    if args.prune_golden and not args.update_golden:
        ap.error("--prune-golden needs --update-golden")
    flag_toks = args.flags.split()
    expected = {"infer": "--infer" in flag_toks, "no_verify": "--no-verify" in flag_toks}

    args.seeds = [parse_seed(t) for t in args.seeds.split(",") if t.strip() != ""]
    if args.fuzz_count < 0 or args.jobs < 1:
        ap.error("--fuzz-count must be >= 0 and --jobs must be >= 1")
    if args.only is None:
        selected = ["input"] if args.input else [g for g in ALL_GROUPS if g != "input"]
    else:
        selected = [t.strip() for t in args.only.split(",") if t.strip()]
        bad = [t for t in selected if t not in ALL_GROUPS]
        if bad or not selected:
            ap.error("--only: unknown group %s (valid: %s)" % (bad, " ".join(ALL_GROUPS)))
    if "input" in selected and not args.input:
        ap.error("the input group needs --input FILE")
    for p in args.input:
        if not os.path.isfile(p):
            print("error: --input file not found: %s" % p, file=sys.stderr)
            return 2

    rs = os.path.abspath(args.rs_path) if args.rs_path else default_rs_path()
    if not os.path.isfile(rs) or not os.access(rs, os.X_OK):
        print("error: Rust binary not found or not executable: %s\n"
              "  build it first, or pass the pruv-trace-dump path with --rs-path." % rs, file=sys.stderr)
        return 2
    py, py_argv = None, None
    if vs_py:
        py = default_python(args.python)
        if os.sep in py or "/" in py:
            if not os.path.isfile(py):
                print("error: Python not found: %s" % py, file=sys.stderr)
                return 2
        adapter = os.path.join(HERE, "diff_py_adapter.py")
        if not os.path.isfile(adapter):
            print("error: %s not found" % adapter, file=sys.stderr)
            return 2
        py_argv = [py, adapter, "trace"] + flag_toks
    rs_argv = [rs] + flag_toks
    env = dict(os.environ)
    env.setdefault("PYTHONUTF8", "1")
    gold = None
    if use_golden:
        gold = golden.Golden.from_args(
            args, golden_default_name(flag_toks),
            "diff_trace%s: Rust pruv-trace-dump trace per (group, input)" % (
                " " + " ".join(flag_toks) if flag_toks else ""))
    label = " ".join(flag_toks) or "default"

    t0 = time.time()
    with tempfile.TemporaryDirectory(prefix="diff_trace_") as tmp:
        if gold is not None:
            gold.add_tmp(tmp)
        per_group, notes = build_cases(set(selected), args, tmp)
        all_cases = [c for g in ALL_GROUPS for c in per_group[g]]

        def work(c):
            if vs_py:
                evaluate(c, py_argv, rs_argv, env, expected)
            else:
                evaluate_rs(c, rs_argv, env, expected)
            return c

        with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as ex:
            list(ex.map(work, all_cases))

        # flaky_unknown retry: serial (--jobs 1). The first run's difference summary and status are kept in the case
        flaky = [c for c in all_cases if c.status == "flaky_unknown"]
        for c in flaky:
            c.flaky_first = True
            c.first_status = c.status
            c.first_summary = diff_summary(c.py_obj, c.rs_obj)
            evaluate(c, py_argv, rs_argv, env, expected)
            if c.status in ("same", "cex_only"):
                c.status = "flaky_unknown"     # resolved by the retry (not added to the success column)
            elif c.status == "flaky_unknown" and inference_wall_timeout(c):
                # Still differs, but a function's inference stopped at the 5 s wall clock: the result only says
                # how fast this machine is (slow CI runner), not that the compilers disagree. Not a failure.
                c.note = (c.note + " " if c.note else "") + "(inference wall-clock timeout: not comparable here)"
            elif c.status == "flaky_unknown" and solver_wall_timeout(c):
                # Same for a solver query that hit its wall-clock timeout on one side (busy CI runner; seen on
                # bench p06 in CI run of 77fc943): sat/unsat on one side and "unknown (timeout)" on the other.
                c.note = (c.note + " " if c.note else "") + "(solver wall-clock timeout: not comparable here)"
            elif c.status == "flaky_unknown":
                c.status = "diff"              # still differs
            c.note = (c.note + " " if c.note else "") + "(after retry)"

        if gold is not None:
            if not vs_py and not gold.update:
                retried = golden_retry_mismatches(gold, all_cases, expected, label, rs_argv, env)
                if retried:
                    print("golden: %d case(s) differed from the file and were rerun serially" % len(retried))
            golden_check_all(gold, all_cases, expected, label, vs_py)

        def verified(c):
            if not vs_py:   # golden mode: the Rust trace matches the golden file and verification completed
                return c.status == "same" and c.rs_obj is not None and c.rs_obj.get("errors") == []
            return c.status in ("same", "cex_only") and c.py_obj is not None and c.py_obj.get("errors") == []

        def inf_timeout(c):
            return c.status == "same_unknown" and any(
                "inference_timeout" in error_codes(o) for o in (c.py_obj, c.rs_obj) if o is not None)

        diffs = [c for c in all_cases if c.status == "diff"]
        crashes = [c for c in all_cases if c.status in ("py_crash", "rs_crash", "timeout")]
        saved = 0
        if args.save_dir:
            if vs_py:
                to_save = diffs + crashes
            else:   # golden mode: mismatching / missing cases with the Rust output (and crashes)
                to_save = [c for c in all_cases if c.status in ("golden_mismatch", "golden_missing", "rs_crash")]
            for c in to_save:
                save_case(args.save_dir, saved, c)
                saved += 1

        cols = COLS if vs_py else GOLDEN_COLS
        print("rs: %s" % rs)
        if vs_py:
            print("py: %s" % py)
        print("mode: %s" % ("vs-py (frozen Python reference)" if vs_py else "golden"))
        print("jobs: %d" % args.jobs)
        print("")
        print("flags: %s (expected options=%s)" % (args.flags or "(none)", json.dumps(expected, sort_keys=True)))
        print("%-12s" % "group" + "".join("%16s" % c for c in cols) + "%15s" % "| verified")
        total = collections.Counter()
        for g in ALL_GROUPS:
            if g not in selected:
                continue
            st = collections.Counter(c.status for c in per_group[g])
            st["inputs"] = len(per_group[g])
            st["verified"] = sum(1 for c in per_group[g] if verified(c))
            print("%-12s" % g + "".join("%16d" % st[c] for c in cols) + "%15d" % st["verified"])
            total.update(st)
        print("%-12s" % "TOTAL" + "".join("%16d" % total[c] for c in cols) + "%15d" % total["verified"])
        if vs_py:
            print("(%d cex_only are counted as pass; flaky_unknown is the number resolved by the serial retry, not added to the success column)" % total["cex_only"])
            print("(verified = inputs that are same or cex_only and have empty py errors: both implementations agree and verification completed)")
            print("(%d same_unknown: byte-identical in both implementations but containing unknown/error/verify_unknown/inference_timeout, "
                  "not counted as success; of these, %d with inference_timeout are failures)" % (
                      total["same_unknown"], sum(1 for c in all_cases if inf_timeout(c))))
        else:
            print("(verified = inputs that match the golden file and have empty errors: verification completed)")
        for n in notes:
            print(n)

        # Tally: based on the canonical (py) output with --vs-py, else on the Rust output
        n_obl = n_att = n_retry = n_skipped = 0
        res = collections.Counter()
        codes = collections.Counter()
        stages = collections.Counter()
        for c in all_cases:
            o = c.py_obj if vs_py else c.rs_obj
            if not o:
                continue
            for ob in o.get("obligations", []):
                n_obl += 1
                if ob.get("skipped"):
                    n_skipped += 1
                for a in ob.get("attempts", []):
                    n_att += 1
                    if a.get("lemmas"):
                        n_retry += 1
                    res[a.get("result")] += 1
            for e in o.get("errors", []):
                if isinstance(e, dict):
                    codes[e.get("code")] += 1
                    stages[e.get("stage")] += 1
        print("")
        print("== tally based on the %s output ==" % ("canonical (py)" if vs_py else "Rust"))
        print("obligations %d (skipped %d), attempts %d, retries (lemmas true) %d" % (n_obl, n_skipped, n_att, n_retry))
        print("by result: sat=%d unsat=%d unknown=%d other=%d" % (
            res["sat"], res["unsat"], res["unknown"],
            sum(v for k, v in res.items() if k not in ("sat", "unsat", "unknown"))))
        print("%d error codes, %d errors:" % (len(codes), sum(codes.values())))
        for code, n in sorted(codes.items(), key=lambda kv: (-kv[1], str(kv[0]))):
            print("  %-24s %d" % (code, n))
        print("error stage: %s" % (", ".join("%s=%d" % kv for kv in sorted(stages.items(), key=lambda kv: str(kv[0]))) or "none"))

        if diffs:
            print("")
            print("== diff (up to %d / %d in total) ==" % (MAX_SHOWN, len(diffs)))
            for c in diffs[:MAX_SHOWN]:
                print("[%s] %s" % (c.group, c.name))
                for ln in diff_summary(c.py_obj, c.rs_obj):
                    print("  " + ln)
        mism = [c for c in all_cases if c.status == "option_mismatch"]
        if mism:
            print("")
            print("== option_mismatch (up to %d / %d in total) ==" % (MAX_SHOWN, len(mism)))
            for c in mism[:MAX_SHOWN]:
                print("[%s] %s: expected %s py=%s rs=%s" % (c.group, c.name, json.dumps(expected, sort_keys=True),
                                                    short(c.py_obj.get("options")), short(c.rs_obj.get("options"))))
        retried = [c for c in all_cases if c.first_summary is not None and c.status != "diff"]
        if retried:
            print("")
            print("== differences resolved by retry (first result) (up to %d / %d in total) ==" % (MAX_SHOWN, len(retried)))
            for c in retried[:MAX_SHOWN]:
                print("[%s] %s: first status %s -> after retry %s" % (c.group, c.name, c.first_status, c.status))
                for ln in c.first_summary:
                    print("  " + ln)
        if crashes:
            print("")
            print("== crash/timeout (up to %d / %d in total) ==" % (MAX_SHOWN, len(crashes)))
            for c in crashes[:MAX_SHOWN]:
                print("[%s] %s: %s %s" % (c.group, c.name, c.status, c.note))
        if args.save_dir:
            print("")
            print("saved: %d cases -> %s" % (saved, args.save_dir))
        print("")
        print("elapsed: %.1f s" % (time.time() - t0))
        gold_ok = True
        if gold is not None:
            print("")
            gold_ok = gold.finish()

    if vs_py:
        clean = all(total[k] == 0 for k in ("diff", "py_crash", "rs_crash", "timeout", "option_mismatch")) \
            and not any(inf_timeout(c) for c in all_cases)
    else:   # golden mismatches are judged by finish() (an --update-golden run accepts them)
        clean = total["rs_crash"] == 0
    clean = clean and gold_ok
    print("result: %s" % ("success" if clean else "fail"))
    return 0 if clean else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
