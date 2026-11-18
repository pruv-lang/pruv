"""T1 (docs/TIDY_SPEC.md, "T1 spec sync"): trace SPEC.md rule IDs to tests and diagnostic codes to the code table.

usage: spec_trace.py [--py PATH] [--rs PATH] [--only-py | --only-rs] [--work DIR] [--no-run]

Checks (each failure prints one line; the last line is `spec_trace: N/N passed`):
  1. every rule ID `R-<section>-<n>` that SPEC.md names has at least one accept and one reject test under tests/spec/,
     and each of them passed with both compilers (Python pruvc and Rust pruvc-rs) in this run;
  2. every rule ID a test names exists in SPEC.md;
  3. every diagnostic `stage/code` that a test expects (any JSON object with string "stage" and "code" under tests/)
     or that a compiler emitted while running tests/spec is a row of the diagnostic code table in SPEC.md section 8.
SPEC.md sections (## headings) that name no rule ID are listed as untracked; that is information, not a failure.

Test format (tests/spec/README.md): a file `NAME.pruv` or a project `NAME/root.pruv` whose first line is
`// spec: R-x-n [R-y-m ...] accept|reject`, with an optional `NAME.json` (`NAME/root.json`) next to it:
  flags            extra compiler flags (list of strings)
  backend          "rust" (default), "llvm", or "none" (no --backend flag; for native --emit)
  stage, code      reject: one emitted diagnostic must have this stage and code (exit 1)
  exit             expected exit code (default 0 accept, 1 reject)
  gen_contains, gen_absent           strings that must / must not appear in the generated output
  gen_regex, gen_regex_absent        regular expressions (re.M) that must / must not match the generated output
  out_contains     strings that must appear in stdout+stderr
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
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
if HERE not in sys.path:
    sys.path.insert(0, HERE)
import compiler_driver as CD  # noqa: E402

SPEC = os.path.join(ROOT, "SPEC.md")
SPEC_DIR = os.path.join(HERE, "spec")
RULE_ID = re.compile(r"\bR-[0-9]+[a-z]?-[0-9]+\b")
HEADER = re.compile(r"^//\s*spec:\s*((?:R-[0-9]+[a-z]?-[0-9]+\s+)+)(accept|reject)\s*$")
TABLE_ROW = re.compile(r"^\|\s*`([a-z_]+)/([a-z0-9_]+)`\s*\|\s*([a-z_]+)\s*\|")


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
    """(ok, detail)"""
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
        c0 = drv.run(drv.argv(prime, out + ".prime", ["--proof-cache", cdir], backend), timeout=300)
        if c0.returncode != 0:
            return False, "prime exit %d %s" % (c0.returncode, (c0.stdout + c0.stderr)[-300:])
        report = os.path.join(work, tag + "_trust.json")
        flags += ["--proof-cache", cdir, "--trust-report", report]
    if backend == "none":   # no --backend flag (native --emit picks its own)
        argv = drv.argv_raw([src, "-o", out] + flags)
    else:
        argv = drv.argv(src, out, flags, backend)
    c = drv.run(argv, timeout=300).raise_if_timeout()
    text = c.stdout + c.stderr
    ds = diags_of(c.stdout)
    for d in ds:
        if isinstance(d.get("stage"), str) and isinstance(d.get("code"), str):
            seen.setdefault((d["stage"], d["code"]), os.path.relpath(src, ROOT))
    exit_want = want.get("exit", 0 if kind == "accept" else 1)
    if c.returncode != exit_want:
        return False, "exit %d (expected %d) %s" % (c.returncode, exit_want, text[-400:])
    if "code" in want:
        if not any(d.get("stage") == want.get("stage") and d.get("code") == want["code"] for d in ds):
            return False, "expected %s/%s; got %s" % (want.get("stage"), want["code"],
                                                     [(d.get("stage"), d.get("code")) for d in ds])
    elif kind == "reject" and exit_want == 1:
        return False, "a reject test needs stage and code in its JSON"
    for s in want.get("out_contains", []):
        if s not in text:
            return False, "output lacks %r: %s" % (s, text[-300:])
    gen_keys = ("gen_contains", "gen_absent", "gen_regex", "gen_regex_absent")
    if any(k in want for k in gen_keys):
        try:
            gen = open(out, encoding="utf-8").read()
        except OSError as e:
            return False, "no generated output: %r" % e
        for s in want.get("gen_contains", []):
            if s not in gen:
                return False, "generated output lacks %r" % s
        for s in want.get("gen_absent", []):
            if s in gen:
                return False, "generated output has %r" % s
        for r in want.get("gen_regex", []):
            if not re.search(r, gen, re.M):
                return False, "generated output does not match /%s/" % r
        for r in want.get("gen_regex_absent", []):
            if re.search(r, gen, re.M):
                return False, "generated output matches /%s/" % r
    if cache is not None:
        try:
            hits = json.load(open(report, encoding="utf-8"))["proof_cache"]["hits"]
        except (OSError, ValueError, KeyError, TypeError) as e:
            return False, "no proof_cache.hits in the trust report: %r" % e
        if (hits > 0) != (cache.get("hits") == "positive"):
            return False, "proof_cache.hits = %d (expected %s)" % (hits, cache.get("hits"))
    return True, ""


def main(argv):
    py_path = rs_path = None
    kinds = ["py", "rs"]
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
    drivers = [CD.make_driver(k, py_path if k == "py" else rs_path) for k in kinds] if run else []
    own_work = work is None
    work = work or tempfile.mkdtemp(prefix="spec_trace_")
    os.makedirs(work, exist_ok=True)
    seen = {}
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
                    ok, detail = run_test(drv, src, want, kind, work, seen)
                except Exception as e:  # timeout or spawn failure is a failed test, not a crash
                    ok, detail = False, repr(e)
                if ok:
                    for t in tids:
                        passed.setdefault((t, kind), set()).add(drv.kind)
                else:
                    fails.append("%s [%s] %s: %s" % (rel, drv.kind, kind, detail))
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
        print("untracked: " + s)
    print("rule IDs: %d, tests: %d, compilers: %s" % (len(ids), len(tests), ",".join(kinds) if run else "none"))
    print("spec_trace: %d/%d passed" % (checks - len(fails), checks))
    return 0 if not fails else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
