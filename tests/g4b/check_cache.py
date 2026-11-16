"""G4b-3 (docs/G4B_SPEC.md section 7): `--proof-cache DIR` on the Python compiler and, with --rs, on the Rust driver.

usage: check_cache.py WORK [--rs PATH] [--py PATH] [--only py|rs] [--case NAME]...
  --rs PATH: the Rust driver (adds the rs compiler); --py PATH: the pruvc.py to test (default pruvc/pruvc.py; its
  directory also serves the Python trace tool); --only: run one compiler; --case: run only the named cases
  (case_cold_warm, case_contract_change, case_never_stored, case_corrupt, case_readonly, case_concurrent, case_keys,
  case_off, case_noverify), repeatable. The Python compiler always runs unless --only rs.
Every case runs on each selected compiler (own cache directories per compiler). Compiles use `pruvc --proof-cache`;
traces use `tests/diff_py_adapter.py trace ... --proof-cache DIR` (py) and `pruv-trace-dump [--infer]
[--proof-cache DIR] FILE` next to the Rust driver (rs). Prints one line per failing check and `cache: N/N passed`;
exit 0 only if all pass."""
from pathlib import Path
import json
import os
import re
import stat
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]

VALID = b"pruv-proof-cache 1\nunsat\n"
TOTAL = [0]
FAILS = []
SKIPS = []
IS_ROOT = hasattr(os, "geteuid") and os.geteuid() == 0


# ---- Pruv sources -------------------------------------------------------------------------------------------------
def single(callee_ens="result == x + 1"):
    """one file: `inc` is called twice by `f`, the caller's proof uses the callee's ensures"""
    return ("fn inc(x: i64) -> i64\n  requires 0 <= x && x <= 10\n  ensures %s\n{ return x + 1; }\n\n"
            "pub fn f(x: i64) -> i64\n  requires 0 <= x && x <= 5\n  ensures result == x + 2\n"
            "{ return inc(inc(x)); }\n" % callee_ens)


def dep_util(callee_ens="result == x + 1"):
    return "pub fn inc(x: i64) -> i64\n  requires 0 <= x && x <= 10\n  ensures %s\n{ return x + 1; }\n" % callee_ens


APP = ("mod util;\npub fn f(x: i64) -> i64\n  requires 0 <= x && x <= 5\n  ensures result == x + 2\n"
       "{ return crate::util::inc(crate::util::inc(x)); }\n")

# a failing obligation: the postcondition is false
FAILING = ("pub fn f(x: i64) -> i64\n  requires 0 <= x && x <= 5\n  ensures result == x + 3\n{ return x + 1; }\n"
           "pub fn g(x: i64) -> i64\n  requires 0 <= x && x <= 5\n  ensures result == x + 1\n{ return x + 1; }\n")
# an obligation the solver cannot decide (a timeout or resource unknown, never sat or unsat), next to a proved one
UNKNOWN = ("pub fn h(x: i64, y: i64, z: i64) -> i64\n  requires 1 <= x && 1 <= y && 1 <= z && z <= 10000000\n"
           "  requires x * x * x + y * y * y == z * z * z\n  ensures result == 1\n{ return 0; }\n"
           "pub fn g(x: i64) -> i64\n  requires 0 <= x && x <= 5\n  ensures result == x + 1\n{ return x + 1; }\n")

# from tests/inference: loops that need --infer; both are accepted, after failing candidates
INFER_OK = ("fn count_less(a: [i64], x: i64) -> i64\n  ensures 0 <= result && result <= len(a)\n"
            "  ensures result == count(k in 0..len(a): a[k] < x)\n{\n  let mut c: i64 = 0;\n  let mut i: i64 = 0;\n"
            "  while i < len(a)\n    decreases len(a) - i\n  {\n    if a[i] < x {\n      c = c + 1;\n    }\n"
            "    i = i + 1;\n  }\n  return c;\n}\n")
INFER_BAD = ("fn drift(a: i64, n: i64) -> i64\n  requires 0 <= a && a <= 100\n  requires 0 <= n && n <= 10\n"
             "  ensures result <= 100\n{\n  let mut x: i64 = a;\n  let mut i: i64 = 0;\n  while i < n\n"
             "    decreases n - i\n  {\n    if x > 0 {\n      x = x - 1;\n    } else {\n      x = 0 - 3;\n    }\n"
             "    i = i + 1;\n  }\n  return x;\n}\n")


# ---- helpers ------------------------------------------------------------------------------------------------------
def man(name, version="0.1.0", root=None, deps=None):
    d = {"name": name, "version": version, "root": root if root is not None else name + ".pruv"}
    if deps is not None:
        d["dependencies"] = deps
    return json.dumps(d)


def pkg_files(prefix, name, text, deps=None):
    return {prefix + "/pruv.json": man(name, deps=deps), prefix + "/" + name + ".pruv": text}


def app_files(callee_ens="result == x + 1"):
    return dict({"app/pruv.json": man("app", deps={"util": {"path": "../util", "version": "0.1"}}),
                 "app/app.pruv": APP}, **pkg_files("util", "util", dep_util(callee_ens)))


def check(name, ok, detail=""):
    TOTAL[0] += 1
    if not ok:
        FAILS.append("FAIL %s: %s" % (name, str(detail).strip()[-700:]))


def entries(d):
    """all regular files below the cache directory DIR (sorted)"""
    d = Path(d)
    if not d.is_dir():
        return []
    return sorted(p for p in d.rglob("*") if p.is_file())


def entry_ok(p, root):
    """a file is a valid entry: DIR/<key[:2]>/<key>, 64 lowercase hex, exact bytes"""
    try:
        return (re.fullmatch(r"[0-9a-f]{64}", p.name) is not None and p.parent.name == p.name[:2]
                and p.parent.parent == Path(root) and p.read_bytes() == VALID)
    except OSError:
        return False


def attempts(tr):
    return [a for o in tr.get("obligations", []) for a in o.get("attempts", [])]


def strip_cached(x):
    """remove every "cached" key (the only allowed difference between a cold and a warm trace)"""
    if isinstance(x, dict):
        return {k: strip_cached(v) for k, v in x.items() if k != "cached"}
    if isinstance(x, list):
        return [strip_cached(v) for v in x]
    return x


def has_cached_key(x):
    if isinstance(x, dict):
        return "cached" in x or any(has_cached_key(v) for v in x.values())
    if isinstance(x, list):
        return any(has_cached_key(v) for v in x)
    return False


def strip_options(smt2):
    return "\n".join(ln for ln in smt2.split("\n") if not ln.lstrip().startswith("(set-option"))


def live_proved(tr):
    """attempts that called the solver and got unsat: each one stored an entry (an attempt whose key was already
    stored in the run is a hit, `cached`)"""
    return [a for a in attempts(tr) if a.get("status") == "proved" and a.get("cached") is not True]


def proved_texts(tr):
    return set(a["smt2"] for a in attempts(tr) if a.get("status") == "proved")


WRAPPER = '''import importlib.abc
import importlib.machinery
import runpy
import sys

FAULT_DIR, ADAPTER = sys.argv[1], sys.argv[2]


class First(importlib.abc.MetaPathFinder):
    def find_spec(self, name, path=None, target=None):
        if path is None:
            return importlib.machinery.PathFinder.find_spec(name, [FAULT_DIR])
        return None


sys.meta_path.insert(0, First())
sys.path.insert(0, FAULT_DIR)
sys.argv = [ADAPTER] + sys.argv[3:]
runpy.run_path(ADAPTER, run_name="__main__")
'''


class R:
    """result of one compile"""

    def __init__(self, rc, out, err, gen, rep, od):
        self.rc, self.out, self.err, self.gen, self.rep, self.od = rc, out, err, gen, rep, od

    @property
    def text(self):
        return self.out + self.err

    @property
    def counts(self):
        return (self.rep or {}).get("proof_cache", {})


class Harness:
    def __init__(self, work, binary, py=None, only=None):
        self.work = Path(work) / "g4b_cache"
        self.work.mkdir(parents=True)
        self.comps = []
        if only != "rs":
            if py:
                pyc = Path(py).resolve()
                # the adapter imports pruvc from a fixed path: run it through a wrapper that serves every top-level
                # module found next to PY first (a meta path finder placed before the normal path finder)
                wrapper = self.work / "py_trace_wrapper.py"
                wrapper.write_text(WRAPPER)
                trace = [sys.executable, str(wrapper), str(pyc.parent), str(ROOT / "tests" / "diff_py_adapter.py"), "trace"]
                self.comps.append(("py", [sys.executable, str(pyc)], trace))
            else:
                self.comps.append(("py", [sys.executable, str(ROOT / "pruvc" / "pruvc.py")],
                                   [sys.executable, str(ROOT / "tests" / "diff_py_adapter.py"), "trace"]))
        if binary:
            dump = str(Path(binary).parent / "pruv-trace-dump")
            self.comps.append(("rs", [binary], [dump]))
        self.n = 0

    def cmd(self, label):
        return [c for c in self.comps if c[0] == label][0][1]

    def project(self, files):
        self.n += 1
        d = self.work / ("p%d" % self.n)
        d.mkdir(parents=True, exist_ok=True)
        for rel, data in files.items():
            p = d / rel
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_bytes(data if isinstance(data, bytes) else data.encode("utf-8"))
        return d

    def source(self, text):
        return self.project({"prog.pruv": text}) / "prog.pruv"

    def newdir(self, create=True):
        self.n += 1
        d = self.work / ("c%d" % self.n)
        if create:
            d.mkdir(parents=True)
        return d

    def launch(self, label, root, cache=None, flags=(), manifest=None):
        self.n += 1
        od = self.work / ("o%d" % self.n) / label
        od.mkdir(parents=True)
        out, report = od / "out.rs", od / "trust.json"
        args = list(self.cmd(label)) + [str(root), "-o", str(out)] + list(flags)
        if cache is not None:
            args += ["--proof-cache", str(cache)]
        if manifest is not None:
            args += ["--manifest", str(manifest)]
        args += ["--trust-report", str(report)]
        p = subprocess.Popen(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        return p, od, out, report

    def finish(self, started):
        p, od, out, report = started
        so, se = p.communicate(timeout=600)
        norm = lambda s: s.replace(str(od), "<OUT>")
        gen = out.read_bytes() if out.is_file() else None
        rep = None
        if report.is_file():
            try:
                rep = json.loads(report.read_text(encoding="utf-8"))
            except ValueError:
                rep = None
        return R(p.returncode, norm(so), norm(se), gen, rep, od)

    def compile(self, label, root, cache=None, flags=(), manifest=None):
        return self.finish(self.launch(label, root, cache, flags, manifest))

    def trace(self, label, file, infer=False, cache=None, no_verify=False):
        """-> parsed trace dict or None"""
        tool = [c for c in self.comps if c[0] == label][0][2]
        args = list(tool)
        if label == "py":
            args += [str(file)]
        if infer:
            args += ["--infer"]
        if no_verify and label == "py":
            args += ["--no-verify"]
        if cache is not None:
            args += ["--proof-cache", str(cache)]
        if label != "py":
            args += [str(file)]
        c = subprocess.run(args, capture_output=True, text=True, timeout=600)
        lines = [ln for ln in c.stdout.split("\n") if ln.strip()]
        try:
            return json.loads(lines[-1])
        except (ValueError, IndexError):
            check("trace [%s] parses" % label, False, "exit %d\n%s%s" % (c.returncode, c.stdout[-300:], c.stderr[-300:]))
            return None


def same_result(name, a, b):
    check(name + ": exit code", a.rc == b.rc, "%d vs %d" % (a.rc, b.rc))
    check(name + ": generated bytes", a.gen == b.gen, "")
    check(name + ": stdout", a.out == b.out, "%s\n---\n%s" % (a.out[-300:], b.out[-300:]))
    check(name + ": stderr", a.err == b.err, "%s\n---\n%s" % (a.err[-300:], b.err[-300:]))


# ---- the cases ----------------------------------------------------------------------------------------------------
def case_cold_warm(h):
    """1. cold then warm: identical output, counts, traces; also on a manifest program"""
    progs = [("single", single(), []), ("infer", INFER_OK, ["--infer"])]
    for label, _, _ in h.comps:
        for pname, text, flags in progs:
            tag = "cold_warm %s [%s]" % (pname, label)
            f = h.source(text)
            cache = h.newdir()
            cold = h.compile(label, f, cache, flags)
            warm = h.compile(label, f, cache, flags)
            check(tag + ": cold exit 0", cold.rc == 0, cold.text)
            same_result(tag, cold, warm)
            cc, wc = cold.counts, warm.counts
            check(tag + ": reports have counts", bool(cc) and bool(wc), "%s / %s" % (cc, wc))
            if cc and wc:
                # a cold compile may already hit: inference asks some queries twice within one run
                check(tag + ": cold stored something", cc["stores"] > 0, cc)
                check(tag + ": warm hits == cold hits + cold stores", wc["hits"] == cc["hits"] + cc["stores"],
                      "%s / %s" % (cc, wc))
                check(tag + ": warm stores == 0", wc["stores"] == 0, wc)
                check(tag + ": warm misses == cold misses - cold stores", wc["misses"] == cc["misses"] - cc["stores"],
                      "%s / %s" % (cc, wc))
                check(tag + ": flags.proof_cache true", cold.rep["flags"]["proof_cache"] is True, cold.rep["flags"])
        # traces: plain and --infer
        for pname, text, infer in (("single", single(), False), ("single", single(), True), ("infer", INFER_OK, True)):
            tag = "cold_warm trace %s infer=%s [%s]" % (pname, infer, label)
            f = h.source(text)
            cache = h.newdir()
            cold = h.trace(label, f, infer, cache)
            warm = h.trace(label, f, infer, cache)
            if cold is None or warm is None:
                continue
            check(tag + ": has attempts", len(attempts(cold)) > 0, "")
            check(tag + ": identical without cached", strip_cached(cold) == strip_cached(warm), "")
            # a cold trace may hit only on a text that an earlier attempt of the same run proved live
            seen, early = set(), []
            for a in attempts(cold):
                if a.get("cached") is True and a["smt2"] not in seen:
                    early.append(a)
                if a.get("status") == "proved" and a.get("cached") is not True:
                    seen.add(a["smt2"])
            check(tag + ": cold hits only on texts proved earlier in the run", not early, "%d early" % len(early))
            if not infer:
                check(tag + ": plain cold trace has no cached", not has_cached_key(cold), "")
            bad = [a for a in attempts(warm) if a.get("status") == "proved" and a.get("cached") is not True]
            check(tag + ": every warm proved attempt is cached", not bad, "%d live" % len(bad))
        # manifest program with a dependency: counts only
        tag = "cold_warm manifest [%s]" % label
        d = h.project(app_files())
        root, mf = d / "app" / "app.pruv", d / "app" / "pruv.json"
        cache = h.newdir()
        cold = h.compile(label, root, cache, (), mf)
        warm = h.compile(label, root, cache, (), mf)
        check(tag + ": cold exit 0", cold.rc == 0, cold.text)
        same_result(tag, cold, warm)
        cc, wc = cold.counts, warm.counts
        if cc and wc:
            check(tag + ": counts", cc["stores"] > 0 and wc["hits"] == cc["hits"] + cc["stores"] and wc["stores"] == 0
                  and wc["misses"] == cc["misses"] - cc["stores"], "%s / %s" % (cc, wc))
        else:
            check(tag + ": reports have counts", False, "")


def case_contract_change(h):
    """2. a changed callee ensures invalidates exactly the queries that mention it; a false one fails as before"""
    for label, _, _ in h.comps:
        tag = "contract_change [%s]" % label
        v1, v2, v3 = h.source(single()), h.source(single("result == x + 1 && x < result")), h.source(
            single("result == x + 5"))
        cache = h.newdir()
        cold = h.trace(label, v1, False, cache)
        warm = h.trace(label, v2, False, cache)
        if cold is not None and warm is not None:
            seen = proved_texts(cold)
            wa = attempts(warm)
            wrong = [a for a in wa if (a.get("cached") is True) != (a["smt2"] in seen)]
            check(tag + ": cached iff text seen proved in the cold trace", not wrong and wa, "%d wrong" % len(wrong))
            check(tag + ": at least one live call", any(a.get("cached") is not True for a in wa), "")
        # false ensures: same outcome as without the cache (the cache holds the V1 answers)
        with_c = h.compile(label, v3, cache)
        without = h.compile(label, v3)
        check(tag + ": false ensures is rejected", without.rc != 0, without.text)
        same_result(tag + " false ensures", with_c, without)

        # the same with a package dependency
        tag = "contract_change manifest [%s]" % label
        cache = h.newdir()
        ds = [h.project(app_files(e)) for e in ("result == x + 1", "result == x + 1 && x < result", "result == x + 5")]
        args = [(d / "app" / "app.pruv", d / "app" / "pruv.json") for d in ds]
        cold = h.compile(label, args[0][0], cache, (), args[0][1])
        check(tag + ": cold exit 0", cold.rc == 0, cold.text)
        warm = h.compile(label, args[1][0], cache, (), args[1][1])
        check(tag + ": changed (true) contract exit 0", warm.rc == 0, warm.text)
        if warm.counts:
            check(tag + ": live solver calls after the change", warm.counts["misses"] > 0, warm.counts)
        else:
            check(tag + ": report has counts", False, "")
        with_c = h.compile(label, args[2][0], cache, (), args[2][1])
        without = h.compile(label, args[2][0], None, (), args[2][1])
        check(tag + ": false contract is rejected", without.rc != 0, without.text)
        same_result(tag + " false contract", with_c, without)
        if with_c.counts:
            check(tag + ": false contract has live calls", with_c.counts["misses"] > 0, with_c.counts)


def case_never_stored(h):
    """3. only unsat answers are stored: failing obligations and failing inference candidates leave no entry"""
    for label, _, _ in h.comps:
        for pname, text, infer in (("failing", FAILING, False), ("unknown", UNKNOWN, False),
                                   ("infer drift", INFER_BAD, True),
                                   ("infer candidates", INFER_OK, True)):
            tag = "never_stored %s [%s]" % (pname, label)
            f = h.source(text)
            cache = h.newdir()
            c = h.compile(label, f, cache, ["--infer"] if infer else [])
            if pname in ("failing", "unknown"):
                check(tag + ": compile fails", c.rc != 0, c.text)
            check(tag + ": report written", c.rep is not None, c.text)
            if c.rep is None:
                continue
            check(tag + ": compile stores are valid entries", all(entry_ok(p, cache) for p in entries(cache)), "")
            check(tag + ": entries == stores", len(entries(cache)) == c.counts.get("stores"),
                  "%d entries, %s" % (len(entries(cache)), c.counts))
            cache2 = h.newdir()
            cold = h.trace(label, f, infer, cache2)
            warm = h.trace(label, f, infer, cache2)
            if cold is None or warm is None:
                continue
            n = len(entries(cache2))
            nonproved = [a for a in attempts(cold) if a.get("status") != "proved"]
            check(tag + ": has non-proved attempts", len(nonproved) > 0, "")
            if pname == "unknown":
                check(tag + ": has undecided attempts", any(a.get("status") in ("timeout", "resource_unknown")
                                                            for a in nonproved), [a.get("status") for a in nonproved])
            check(tag + ": entries == live proved attempts", n == len(live_proved(cold)),
                  "%d entries, %d live proved" % (n, len(live_proved(cold))))
            # a stored non-proved attempt would come back cached (as proved) in the warm trace
            wn = [a for a in attempts(warm) if a.get("status") != "proved"]
            check(tag + ": non-proved attempts are never cached", len(wn) == len(nonproved)
                  and not any(a.get("cached") is True for a in wn), "%d / %d" % (len(wn), len(nonproved)))
            check(tag + ": entries are valid", all(entry_ok(p, cache2) for p in entries(cache2)), "")


def case_corrupt(h):
    """4. garbage, truncated and unreadable entries are misses and are rewritten"""
    for label, _, _ in h.comps:
        tag = "corrupt [%s]" % label
        f = h.source(single())
        cache = h.newdir()
        cold = h.compile(label, f, cache)
        es = entries(cache)
        check(tag + ": cold stored at least 3 entries", len(es) >= 3, len(es))
        if len(es) < 3 or not cold.counts:
            continue
        damaged = 2 if IS_ROOT else 3
        if IS_ROOT:
            SKIPS.append("SKIP %s: chmod 000 part (running as root)" % tag)
        locked = None
        try:
            es[0].write_bytes(b"garbage \xff\x00 not an entry")
            es[1].write_bytes(VALID[:len(VALID) // 2])
            if not IS_ROOT:
                locked = es[2]
                os.chmod(locked, 0o000)
            warm = h.compile(label, f, cache)
        finally:
            if locked is not None:
                try:
                    os.chmod(locked, 0o644)
                except OSError:
                    pass
        check(tag + ": warm exit 0", warm.rc == 0, warm.text)
        same_result(tag, cold, warm)
        if warm.counts:
            check(tag + ": hits == stores - damaged", warm.counts["hits"] == cold.counts["stores"] - damaged,
                  "%s / %s" % (cold.counts, warm.counts))
            check(tag + ": stores == damaged", warm.counts["stores"] == damaged, warm.counts)
        check(tag + ": entries rewritten", [p for p in entries(cache)] == es and all(entry_ok(p, cache) for p in es),
              [str(p) for p in es if not entry_ok(p, cache)][:3])


def case_readonly(h):
    """5. a read-only cache directory: the compile succeeds, nothing is stored or created"""
    if IS_ROOT:
        SKIPS.append("SKIP readonly: running as root (permissions are not enforced)")
        return
    for label, _, _ in h.comps:
        tag = "readonly [%s]" % label
        f = h.source(single())
        base = h.compile(label, f)
        cache = h.newdir()
        os.chmod(cache, 0o555)
        try:
            r = h.compile(label, f, cache)
            left = list(cache.rglob("*"))
        finally:
            os.chmod(cache, 0o755)
        check(tag + ": exit 0", r.rc == 0, r.text)
        check(tag + ": output equals the no-cache output", r.gen == base.gen and r.out == base.out
              and r.err == base.err, r.text)
        check(tag + ": stores == 0", r.counts.get("stores") == 0, r.counts)
        check(tag + ": no files created", not left, [str(p) for p in left][:5])


def case_concurrent(h):
    """6. two compiles of one program into one DIR at the same time"""
    for label, _, _ in h.comps:
        tag = "concurrent [%s]" % label
        f = h.source(INFER_OK)
        cache = h.newdir()
        ps = [h.launch(label, f, cache, ["--infer"]) for _ in range(2)]
        a, b = [h.finish(p) for p in ps]
        check(tag + ": both exit 0", a.rc == 0 and b.rc == 0, a.text + b.text)
        same_result(tag, a, b)
        es = entries(cache)
        check(tag + ": entries exist", len(es) > 0, "")
        bad = [str(p) for p in es if not entry_ok(p, cache)]
        check(tag + ": every file is a valid entry (no temporaries)", not bad and not any(
            ".tmp" in p.name for p in es), bad[:3])
        warm = h.compile(label, f, cache, ["--infer"])
        check(tag + ": warm exit 0", warm.rc == 0, warm.text)
        tr = h.trace(label, f, True, h.newdir())
        if tr is not None and warm.counts:
            live = len([x for x in attempts(tr) if x.get("status") != "proved"])
            check(tag + ": warm misses == non-unsat attempts", warm.counts["misses"] == live,
                  "%s misses, %d non-proved attempts" % (warm.counts, live))
            check(tag + ": warm stores == 0", warm.counts["stores"] == 0, warm.counts)


def case_keys(h):
    """7. keys: the whole query text (with the solver options) is in the key, and py and rs keys never meet"""
    f = h.source(INFER_OK)
    sets = {}
    for label, _, _ in h.comps:
        tag = "keys [%s]" % label
        cache = h.newdir()
        c = h.compile(label, f, cache, ["--infer"])
        check(tag + ": compile ok", c.rc == 0 and c.rep is not None, c.text)
        if c.rep is None:
            continue
        sets[label] = set(p.name for p in entries(cache))
        check(tag + ": compiler id matches the label", c.rep["compiler"]["impl"] == label, c.rep["compiler"])
        # every live unsat attempt stored one entry; plain and --infer traces fill one directory
        cache2 = h.newdir()
        plain = h.trace(label, f, False, cache2)
        inf = h.trace(label, f, True, cache2)
        if plain is None or inf is None:
            continue
        live = live_proved(plain) + live_proved(inf)
        n = len(entries(cache2))
        check(tag + ": entries == live proved attempts", n == len(live), "%d entries, %d live" % (n, len(live)))
        check(tag + ": entries live under their key prefix", all(entry_ok(p, cache2) for p in entries(cache2)), "")
        # the same query asked with two option sets: neither answers the other, so each text was proved live
        groups = {}
        for a in attempts(plain) + attempts(inf):
            if a.get("status") == "proved":
                groups.setdefault(strip_options(a["smt2"]), set()).add(a["smt2"])
        livetexts = set(a["smt2"] for a in live)
        pairs = [ts for ts in groups.values() if len(ts) > 1]
        check(tag + ": some formula was asked with two option sets", len(pairs) > 0, "no such pair in the traces")
        for ts in pairs:
            check(tag + ": option change changes the key", ts <= livetexts, "%d of %d live" % (len(ts & livetexts), len(ts)))
    if "py" in sets and "rs" in sets:
        check("keys: py and rs entry sets are disjoint", not (sets["py"] & sets["rs"]) and sets["py"] and sets["rs"],
              "%d common" % len(sets["py"] & sets["rs"]))


def case_off(h):
    """8. no --proof-cache: flag false, counts 0, no cached key in the trace, nothing created"""
    for label, _, _ in h.comps:
        tag = "off [%s]" % label
        f = h.source(single())
        c = h.compile(label, f)
        check(tag + ": exit 0", c.rc == 0, c.text)
        check(tag + ": flags.proof_cache false", c.rep is not None and c.rep["flags"]["proof_cache"] is False,
              (c.rep or {}).get("flags"))
        check(tag + ": counts all 0", c.counts == {"hits": 0, "misses": 0, "stores": 0}, c.counts)
        for infer in (False, True):
            tr = h.trace(label, f, infer)
            if tr is not None:
                check(tag + ": trace infer=%s has attempts and no cached key" % infer,
                      len(attempts(tr)) > 0 and not has_cached_key(tr), "")


def case_noverify(h):
    """9. --no-verify never touches the cache"""
    for label, _, _ in h.comps:
        tag = "noverify [%s]" % label
        f = h.source(single())
        cache = h.newdir()
        c = h.compile(label, f, cache, ["--no-verify"])
        check(tag + ": exit 0", c.rc == 0, c.text)
        check(tag + ": no entries", not entries(cache), [str(p) for p in entries(cache)][:3])
        check(tag + ": counts all 0", c.counts == {"hits": 0, "misses": 0, "stores": 0}, c.counts)
        check(tag + ": flags.proof_cache true", c.rep is not None and c.rep["flags"]["proof_cache"] is True,
              (c.rep or {}).get("flags"))
        absent = h.newdir(create=False)
        h.compile(label, f, absent, ["--no-verify"])
        check(tag + ": a missing DIR is not created", not absent.exists(), "")


CASES = (case_cold_warm, case_contract_change, case_never_stored, case_corrupt, case_readonly,
         case_concurrent, case_keys, case_off, case_noverify)


def main(work, binary=None, py=None, only=None, cases=()):
    names = [f.__name__ for f in CASES]
    for c in cases:
        if c not in names:
            print("unknown case %s (known: %s)" % (c, ", ".join(names)))
            return 2
    if only == "rs" and not binary:
        print("--only rs needs --rs PATH")
        return 2
    h = Harness(work, binary, py, only)
    for fn in CASES:
        if cases and fn.__name__ not in cases:
            continue
        try:
            fn(h)
        except Exception as e:  # a broken case is a failure, not a crash of the whole run
            TOTAL[0] += 1
            FAILS.append("FAIL %s: exception %s: %s" % (fn.__name__, type(e).__name__, e))
    for s in SKIPS:
        print(s)
    for f in FAILS:
        print(f)
    print("cache: %d/%d passed" % (TOTAL[0] - len(FAILS), TOTAL[0]))
    return 1 if FAILS else 0


if __name__ == "__main__":
    args = sys.argv[1:]
    opts = {}
    cases = []
    for flag in ("--rs", "--py", "--only", "--case"):
        while flag in args:
            i = args.index(flag)
            if i + 1 >= len(args):
                print(__doc__)
                sys.exit(2)
            if flag == "--case":
                cases.append(args[i + 1])
            else:
                opts[flag] = args[i + 1]
            del args[i:i + 2]
    if len(args) != 1 or opts.get("--only") not in (None, "py", "rs"):
        print(__doc__)
        sys.exit(2)
    sys.exit(main(args[0], opts.get("--rs"), opts.get("--py"), opts.get("--only"), tuple(cases)))
