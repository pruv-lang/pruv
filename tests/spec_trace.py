"""T1 (docs/TIDY_SPEC.md, "T1 spec sync"): trace SPEC.md rule IDs to tests and diagnostic codes to the code table.

usage: spec_trace.py [--rs PATH] [--only-rs | --vs-py [--py PATH] | --only-py [--py PATH]] [--work DIR] [--no-run]

Default: the Rust compiler only (pruvc/ is frozen since bundle PF). --vs-py also runs the frozen Python compiler
(the old default: a rule passes only if it passes on both); --only-py runs only the frozen Python compiler.

Checks (each failure prints one line; the last line is `spec_trace: N/N passed`):
  1. every rule ID `R-<section>-<n>` that SPEC.md names has at least one accept and one reject test under tests/spec/,
     and each of them passed with every compiler run here (Rust pruvc-rs; with --vs-py also the frozen Python pruvc);
  2. every rule ID a test names exists in SPEC.md;
  3. every diagnostic `stage/code` that a test expects (any JSON object with string "stage" and "code" under tests/)
     or that a compiler emitted while running tests/spec is a row of the diagnostic code table in SPEC.md section 8.
  4. every SPEC.md section (## heading) names at least one rule ID, unless its heading is in EXEMPT_SECTIONS (the
     sections that had no rule ID when this check was added, TIDY 4.16 HC-9). An exempt section is printed as
     `untracked (exempt)`; an exemption whose section now names an ID or no longer exists is printed as a note.

Test format (tests/spec/README.md): a file `NAME.pruv` or a project `NAME/root.pruv` whose first line is
`// spec: R-x-n [R-y-m ...] accept|reject`, with an optional `NAME.json` (`NAME/root.json`) next to it:
  flags            extra compiler flags (list of strings)
  backend          "rust" (default), "llvm", or "none" (no --backend flag; for native --emit)
  stage, code      reject: one emitted diagnostic must have this stage and code (exit 1)
  line, col        optional expected location of that diagnostic
  exit             expected exit code (default 0 accept, 1 reject)
  gen_contains, gen_absent           strings that must / must not appear in the generated output
  gen_regex, gen_regex_absent        regular expressions (re.M) that must / must not match the generated output
  out_contains     strings that must appear in stdout+stderr
  max_seconds      upper bound on the compiler's wall time for this test (a compile-time bound, R-6g9-4)
  cache            {"prime": "OTHER.pruv" (optional, relative to the test), "hits": "positive"|"zero"}: compile prime
                   (default: the test itself) with a fresh --proof-cache, then compile the test with the same cache
                   and --trust-report; proof_cache.hits must be > 0 (positive) or == 0 (zero)
  py, rs           objects merged over the keys above for that compiler only; a null value drops the key (a
                   driver-only rule, such as native --emit, expects different results per compiler)
"""
import glob
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
if HERE not in sys.path:
    sys.path.insert(0, HERE)
import compiler_driver as CD  # noqa: E402

SPEC = os.path.join(ROOT, "SPEC.md")
SPEC_DIR = os.path.join(HERE, "spec")
RULE = r"R-[0-9]+(?:[a-z][0-9]*)?-[0-9]+"
RULE_ID = re.compile(r"\b" + RULE + r"\b")
HEADER = re.compile(r"^//\s*spec:\s*((?:" + RULE + r"\s+)+)(accept|reject)\s*$")
TABLE_ROW = re.compile(r"^\|\s*`([a-z_]+)/([a-z0-9_]+)`\s*\|\s*([a-z_]+)\s*\|")
# SPEC.md sections without any rule ID at a206043a (HC-9). Give a new section rule IDs instead of adding it here;
# drop a name once its section names an ID.
EXEMPT_SECTIONS = (
    "2. Types",
    "3. Functions",
    "6c. Generics (G3)",
    "6h. Files (G10)",
    "6i. Parallel calls: par (G7)",
    "6j. Cost bounds: cost and spent (Stage 5)",
    "6k. The standard module: std (G8)",
    "6n. Readiness loops: async (G10)",
    "7a. Refinement types (STAGE 1, bundle A)",
    "9. Example",
)
CLASSES = ("false_reject", "false_accept", "wrong_location", "timeout", "other")


def spec_ids_and_sections(text):
    ids = set()
    untracked = []
    section, section_ids = None, 0
    for line in text.splitlines():
        if line.startswith("## "):
            if section is not None and section_ids == 0:
                untracked.append(section)
            section, section_ids = line[3:].strip(), 0
        found = RULE_ID.findall(line)
        ids.update(found)
        section_ids += len(found)
    if section is not None and section_ids == 0:
        untracked.append(section)
    return ids, untracked


def code_table(text):
    """rows of the diagnostic code table: `stage/code` | stage | meaning"""
    rows = set()
    bad = []
    for line in text.splitlines():
        m = TABLE_ROW.match(line)
        if m:
            if m.group(1) != m.group(3):
                bad.append("%s/%s listed under stage %s" % (m.group(1), m.group(2), m.group(3)))
            rows.add((m.group(1), m.group(2)))
    return rows, bad


def expected_codes():
    """(stage, code) -> first test JSON that expects it"""
    out = {}
    for f in sorted(glob.glob(os.path.join(HERE, "**", "*.json"), recursive=True)):
        try:
            d = json.load(open(f, encoding="utf-8"))
        except (OSError, ValueError, UnicodeDecodeError):
            continue
        stack = [d]
        while stack:
            x = stack.pop()
            if isinstance(x, dict):
                if isinstance(x.get("stage"), str) and isinstance(x.get("code"), str):
                    out.setdefault((x["stage"], x["code"]), os.path.relpath(f, ROOT))
                stack.extend(x.values())
            elif isinstance(x, list):
                stack.extend(x)
    return out


def spec_tests():
    """[(rel path, source path, json path, ids, kind)] and header errors"""
    tests, errors = [], []
    cands = sorted(glob.glob(os.path.join(SPEC_DIR, "*.pruv")) + glob.glob(os.path.join(SPEC_DIR, "*", "root.pruv")))
    for f in cands:
        rel = os.path.relpath(f, ROOT).replace(os.sep, "/")
        first = open(f, encoding="utf-8").readline().rstrip("\n")
        m = HEADER.match(first)
        if not m:
            # a companion module or a cache prime file has no header and is not a test
            if os.path.basename(f) == "root.pruv":
                errors.append("%s: first line must be `// spec: R-x-n ... accept|reject`" % rel)
            continue
        jf = os.path.splitext(f)[0] + ".json"
        tests.append((rel, f, jf if os.path.exists(jf) else None, m.group(1).split(), m.group(2)))
    return tests, errors


def diags_of(text):
    out = []
    for line in text.splitlines():
        try:
            d = json.loads(line)
        except ValueError:
            continue
        if isinstance(d, dict) and "code" in d:
            out.append(d)
    return out


def run_test(drv, src, want, kind, work, seen):
    """(ok, detail, failure class or None)"""
    flags = list(want.get("flags", []))
    backend = want.get("backend", "rust")
    tag = "%s_%s" % (drv.kind, re.sub(r"[^A-Za-z0-9]+", "_", os.path.relpath(src, SPEC_DIR)))
    out = os.path.join(work, tag + (".ll" if backend == "llvm" else ".rs"))
    if backend == "llvm":
        flags = ["--rust-bindings", os.path.join(work, tag + "_b.rs")] + flags
    cache = want.get("cache")
    if cache is not None:
        cdir = os.path.join(work, tag + "_cache")
        shutil.rmtree(cdir, ignore_errors=True)
        prime = os.path.join(os.path.dirname(src), cache["prime"]) if cache.get("prime") else src
        c0 = drv.run(drv.argv(prime, out + ".prime", ["--proof-cache", cdir], backend),
                     timeout=300).raise_if_timeout()
        if c0.returncode != 0:
            return False, "prime exit %d %s" % (c0.returncode, (c0.stdout + c0.stderr)[-300:]), "other"
        report = os.path.join(work, tag + "_trust.json")
        flags += ["--proof-cache", cdir, "--trust-report", report]
    if backend == "none":   # no --backend flag (native --emit picks its own)
        argv = drv.argv_raw([src, "-o", out] + flags)
    else:
        argv = drv.argv(src, out, flags, backend)
    c = drv.run(argv, timeout=300).raise_if_timeout()
    text = c.stdout + c.stderr
    ds = diags_of(c.stdout + "\n" + c.stderr)
    for d in ds:
        if isinstance(d.get("stage"), str) and isinstance(d.get("code"), str):
            seen.setdefault((d["stage"], d["code"]), os.path.relpath(src, ROOT))
    exit_want = want.get("exit", 0 if kind == "accept" else 1)
    if c.returncode != exit_want:
        cls = "other"
        if kind == "accept" and c.returncode == 1 and any("stage" in d for d in ds):
            cls = "false_reject"
        elif kind == "reject" and c.returncode == 0:
            cls = "false_accept"
        return False, "exit %d (expected %d) %s" % (c.returncode, exit_want, text[-400:]), cls
    if "max_seconds" in want and c.elapsed > want["max_seconds"]:
        return False, "compile took %.2fs (max_seconds %s)" % (c.elapsed, want["max_seconds"]), "other"
    if "code" in want:
        matching = [d for d in ds if d.get("stage") == want.get("stage") and d.get("code") == want["code"]]
        if not matching:
            return False, "expected %s/%s; got %s" % (want.get("stage"), want["code"],
                                                     [(d.get("stage"), d.get("code")) for d in ds]), "other"
        location = {k: want[k] for k in ("line", "col") if k in want}
        if location and not any(all(d.get(k) == v for k, v in location.items()) for d in matching):
            return False, "expected location %s; got %s" % (
                location, [{k: d.get(k) for k in location} for d in matching]), "wrong_location"
    elif kind == "reject" and exit_want == 1:
        return False, "a reject test needs stage and code in its JSON", "other"
    for s in want.get("out_contains", []):
        if s not in text:
            return False, "output lacks %r: %s" % (s, text[-300:]), "other"
    gen_keys = ("gen_contains", "gen_absent", "gen_regex", "gen_regex_absent")
    if any(k in want for k in gen_keys):
        try:
            gen = open(out, encoding="utf-8").read()
        except OSError as e:
            return False, "no generated output: %r" % e, "other"
        for s in want.get("gen_contains", []):
            if s not in gen:
                return False, "generated output lacks %r" % s, "other"
        for s in want.get("gen_absent", []):
            if s in gen:
                return False, "generated output has %r" % s, "other"
        for r in want.get("gen_regex", []):
            if not re.search(r, gen, re.M):
                return False, "generated output does not match /%s/" % r, "other"
        for r in want.get("gen_regex_absent", []):
            if re.search(r, gen, re.M):
                return False, "generated output matches /%s/" % r, "other"
    if cache is not None:
        try:
            hits = json.load(open(report, encoding="utf-8"))["proof_cache"]["hits"]
        except (OSError, ValueError, KeyError, TypeError) as e:
            return False, "no proof_cache.hits in the trust report: %r" % e, "other"
        if (hits > 0) != (cache.get("hits") == "positive"):
            return False, "proof_cache.hits = %d (expected %s)" % (hits, cache.get("hits")), "other"
    return True, "", None


def main(argv):
    py_path = rs_path = None
    kinds = ["rs"]
    work = None
    run = True
    i = 0
    while i < len(argv):
        a = argv[i]
        if a in ("--py", "--rs", "--work") and i + 1 < len(argv):
            if a == "--py":
                py_path = argv[i + 1]
            elif a == "--rs":
                rs_path = argv[i + 1]
            else:
                work = argv[i + 1]
            i += 2
            continue
        if a == "--only-py":
            kinds = ["py"]
        elif a == "--only-rs":
            kinds = ["rs"]
        elif a == "--vs-py":
            kinds = ["py", "rs"]
        elif a == "--no-run":
            run = False
        else:
            sys.stderr.write(__doc__.split("\n\n")[1] + "\n")
            return 2
        i += 1
    text = open(SPEC, encoding="utf-8").read()
    ids, untracked = spec_ids_and_sections(text)
    table, table_bad = code_table(text)
    tests, errors = spec_tests()
    fails = list(errors) + table_bad
    checks = len(errors) + len(table_bad)
    for sec in untracked:
        checks += 1
        if sec not in EXEMPT_SECTIONS:
            fails.append("SPEC.md section %r names no rule ID (add R-x-n IDs and tests; EXEMPT_SECTIONS is closed)" % sec)
    stale = [sec for sec in EXEMPT_SECTIONS if sec not in untracked]
    drivers = [CD.make_driver(k, py_path if k == "py" else rs_path) for k in kinds] if run else []
    own_work = work is None
    work = work or tempfile.mkdtemp(prefix="spec_trace_")
    os.makedirs(work, exist_ok=True)
    seen = {}
    counts = {drv.kind: dict.fromkeys(CLASSES, 0) for drv in drivers}
    passed = {}   # (id, kind) -> set of compiler kinds where some test passed
    try:
        for rel, src, jf, tids, kind in tests:
            base = json.load(open(jf, encoding="utf-8")) if jf else {}
            for t in tids:
                checks += 1
                if t not in ids:
                    fails.append("%s names %s, which SPEC.md does not define" % (rel, t))
            for drv in drivers:
                want = {k: v for k, v in base.items() if k not in ("py", "rs")}
                for k, v in base.get(drv.kind, {}).items():
                    if v is None:
                        want.pop(k, None)   # null drops a key for this compiler
                    else:
                        want[k] = v
                checks += 1
                try:
                    ok, detail, cls = run_test(drv, src, want, kind, work, seen)
                except subprocess.TimeoutExpired as e:
                    ok, detail, cls = False, repr(e), "timeout"
                except Exception as e:
                    ok, detail, cls = False, repr(e), "other"
                if ok:
                    for t in tids:
                        passed.setdefault((t, kind), set()).add(drv.kind)
                else:
                    counts[drv.kind][cls] += 1
                    fails.append("%s [%s] %s [%s]: %s" % (rel, drv.kind, kind, cls, detail))
    finally:
        if own_work:
            shutil.rmtree(work, ignore_errors=True)
    named = {(t, k) for _, _, _, tids, k in tests for t in tids}
    for t in sorted(ids):
        for kind in ("accept", "reject"):
            checks += 1
            if (t, kind) not in named:
                fails.append("%s has no %s test under tests/spec" % (t, kind))
            elif run and passed.get((t, kind), set()) != set(kinds):
                fails.append("%s: no %s test passed on %s" % (
                    t, kind, ", ".join(sorted(set(kinds) - passed.get((t, kind), set())))))
    codes = expected_codes()
    for k, v in seen.items():
        codes.setdefault(k, v)
    for (stage, code), where in sorted(codes.items()):
        checks += 1
        if (stage, code) not in table:
            fails.append("diagnostic %s/%s (%s) is not in the SPEC.md code table" % (stage, code, where))
    for f in fails:
        print("FAIL " + f)
    for s in untracked:
        if s in EXEMPT_SECTIONS:
            print("untracked (exempt): " + s)
    for s in stale:
        print("note: EXEMPT_SECTIONS entry %r has rule IDs now or is gone; remove it" % s)
    print("rule IDs: %d, tests: %d, compilers: %s" % (len(ids), len(tests), ",".join(kinds) if run else "none"))
    for k, classes in counts.items():
        print("CLASSES %s %s" % (k, " ".join("%s=%d" % item for item in classes.items())))
    print("spec_trace: %d/%d passed" % (checks - len(fails), checks))
    return 0 if not fails else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
