"""G4b-2 (docs/G4B_SPEC.md section 6): `--trust-report PATH` on the Python compiler and, with --rs, on the Rust driver.

usage: check_trust.py WORK [--rs PATH]     (PATH: the Rust driver; the Python compiler always runs)
Every case runs on each selected compiler. Per report: content checks, the canonical format (keys sorted, indent 2,
non-ASCII kept, final newline), the `compiler` object. With --rs the two reports must also be identical after removing
the `compiler` object. Prints one line per failing check and `trust: N/N passed`; exit 0 only if all pass."""
from pathlib import Path
import hashlib
import json
import os
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]

# ---- completeness table (case 9): every trust construct must have a report case ---------------------------------
TRUST_KINDS = {"trusted_rust_fn"}
# catalog op prefix -> name of the case that must produce at least one op of that family
CATALOG_FAMILIES = {"vec_": "collections", "map_": "collections", "string_": "collections",
                    "file_": "file", "proc_": "process", "net_": "net", "ev_": "loop"}

KINDS_SEEN = set()
FAMILIES_SEEN = {}          # (case name, compiler label) -> set of catalog prefixes seen in its reports
TOTAL = [0]
FAILS = []
SKIPS = []

# ---- Pruv sources (small known-good programs reused from the existing tests) -------------------------------------
MIN_ENS = "  ensures result <= a && result <= b\n  ensures result == a || result == b\n"


def trusted_min(name, path="::std::cmp::min"):
    """a `trusted rust fn` as in tests/g6/proof/p13_trusted_scalar_min.pruv"""
    return "trusted rust fn %s(a: i64, b: i64) -> i64\n%s  = \"%s\";\n" % (name, MIN_ENS, path)


# program package with one called and one never called trusted fn, mounting `util`
APP1 = ("mod util;\n" + trusted_min("rust_min") + trusted_min("spare_min") +
        "fn clamp(x: i64) -> i64\n  ensures result <= x\n{ return rust_min(x, 10); }\n"
        "pub fn f(x: i64) -> i64\n  requires 0 <= x && x <= 10\n  ensures result <= x\n"
        "{ return crate::util::lo(clamp(x)); }\n")
UTIL1 = (trusted_min("dep_min") +
         "pub fn lo(x: i64) -> i64\n  requires 0 <= x && x <= 10\n  ensures result <= x\n{ return dep_min(x, 5); }\n")

# proof fails in the program package (ensures is false); the dependency is fine
APP4 = ("mod util;\n" + trusted_min("rust_min") +
        "fn g(x: i64) -> i64\n  ensures result <= x\n{ return rust_min(x, 10); }\n"
        "pub fn f(x: i64) -> i64\n  requires 0 <= x && x <= 10\n  ensures result == x + 2\n{ return x + 1; }\n")
UTIL4 = (trusted_min("dep_min") +
         "pub fn lo(x: i64) -> i64\n  ensures result <= x\n{ return dep_min(x, 5); }\n")

# catalog families (g6_m02_replace, p25, g10_r03_missing, g10_proc args_builder, g10_net fetch, g10_async update)
COLLECTIONS = (
    "fn vecs() -> i64 ! alloc {\n  let mut v: Vec<i64> = vec_new::<i64>();\n  v = vec_push::<i64>(move v, 5);\n"
    "  return vec_get::<i64>(v, 0);\n}\n"
    "fn strs() -> i64 ! alloc {\n  let mut s: String = string_new();\n  s = string_push_str(move s, \"ab\");\n"
    "  return string_byte(s, 0);\n}\n"
    "fn maps(k: i64, a: i64) -> i64 ! alloc {\n  let mut m: HashMap<i64, i64> = map_new::<i64, i64>();\n"
    "  m = map_insert::<i64, i64>(move m, k, a);\n  let found: Option<i64> = map_get::<i64, i64>(m, k);\n"
    "  match found {\n    Some(x) => { return x; }\n    None => { return a; }\n  }\n}\n")
FILE = ("fn missing(path: str) -> bool ! fs {\n  let f: File = file_open_read(path);\n"
        "  let failed: bool = !file_ok(f) && !file_readable(f) && !file_writable(f) && file_size(f) == 0;\n"
        "  let closed: bool = file_close(move f);\n  return failed && !closed;\n}\n")
PROCESS = ("fn args2(first: str, second: str) -> ProcessArgs ! alloc {\n  let mut a: ProcessArgs = proc_args_new();\n"
           "  a = proc_args_push(move a, first);\n  a = proc_args_push(move a, second);\n  return move a;\n}\n\n"
           "fn count(first: str, second: str) -> i64 ! alloc {\n  let a: ProcessArgs = args2(first, second);\n"
           "  return proc_args_len(a);\n}\n")
NET = ("pub fn fetch(host: str, port: i64, request: [u8]) -> i64 ! net, alloc\n  requires 1 <= port && port <= 65535\n{\n"
       "  let mut s: Socket = net_connect(host, port);\n  if !net_ok(s) {\n    let r: Result<i64> = net_close(move s);\n"
       "    return -1;\n  }\n\n  s = net_write_bytes(move s, request);\n"
       "  if !net_ok(s) { let r: Result<i64> = net_close(move s); return -1; }\n  s = net_shutdown_write(move s);\n"
       "  if !net_ok(s) { let r: Result<i64> = net_close(move s); return -1; }\n  s = net_read_all(move s);\n"
       "  if !net_ok(s) { let r: Result<i64> = net_close(move s); return -1; }\n\n"
       "  let reply: [u8] = net_bytes(s);\n  let n: i64 = len(reply);\n  let r: Result<i64> = net_close(move s);\n"
       "  return n;\n}\n")
LOOP = ("fn f() -> Result<i64> ! async, net, proc, alloc {\n  let mut l: Loop = ev_new(4);\n"
        "  l = ev_add_timer(move l, 1);\n  l = ev_wait(move l, 0);\n  return ev_close(move l);\n}\n")

# different ops in the program (String) and in a dependency (Vec)
APP2 = ("mod ds;\nfn text() -> i64 ! alloc {\n  let mut s: String = string_new();\n"
        "  s = string_push_str(move s, \"ab\");\n  return string_byte(s, 0) + crate::ds::count();\n}\n")
DS2 = ("pub fn count() -> i64 ! alloc {\n  let mut v: Vec<i64> = vec_new::<i64>();\n  v = vec_push::<i64>(move v, 5);\n"
       "  return vec_get::<i64>(v, 0);\n}\n")

# flags
INC = "fn inc(x: i64) -> i64\n  requires 0 <= x && x <= 10\n  ensures result == x + 1\n{ return x + 1; }\n"
EXT_PROG = ("trusted rust fn one(a: i64) -> i64\n  ensures result == a\n  = \"::ext_a::one\";\n"
            "trusted rust fn two(a: i64) -> i64\n  ensures result == a\n  = \"::ext_b::two\";\n"
            "fn both(a: i64) -> i64 { return two(one(a)); }\n")

# no manifest
LEGACY = (trusted_min("rust_min") + trusted_min("spare_min") +
          "fn clamp(x: i64) -> i64\n  ensures result <= x\n{ return rust_min(x, 10); }\n")
EXPL_ROOT = ("mod helper;\n" + trusted_min("rust_min") +
             "pub fn f(x: i64) -> i64\n  requires 0 <= x && x <= 10\n{ return rust_min(crate::helper::lo(x), 3); }\n")
EXPL_HELPER = trusted_min("helper_min") + "pub fn lo(x: i64) -> i64 { return helper_min(x, 5); }\n"


# ---- helpers ------------------------------------------------------------------------------------------------------
def man(name, version="0.1.0", root=None, deps=None):
    d = {"name": name, "version": version, "root": root if root is not None else name + ".pruv"}
    if deps is not None:
        d["dependencies"] = deps
    return json.dumps(d)


def dep(path, version="0.1"):
    return {"path": path, "version": version}


def pkg_files(prefix, name, text, deps=None):
    return {prefix + "/pruv.json": man(name, deps=deps), prefix + "/" + name + ".pruv": text}


def lineno(text, needle):
    """1-based line of the first line containing needle"""
    for i, ln in enumerate(text.split("\n")):
        if needle in ln:
            return i + 1
    raise AssertionError("needle not in text: " + needle)


def canon(rep):
    return json.dumps(rep, sort_keys=True, indent=2, ensure_ascii=False) + "\n"


def strip(rep):
    d = dict(rep)
    d.pop("compiler", None)
    return canon(d)


def sha_of(files):
    """files: [(display path, raw bytes)] in load order -> hex SHA-256 as spec section 6 rule 2"""
    h = hashlib.sha256()
    for disp, raw in files:
        h.update(disp.encode("utf-8") + b"\0" + str(len(raw)).encode("ascii") + b"\0" + raw)
    return h.hexdigest()


def tr(item, file, line, reachable):
    return {"kind": "trusted_rust_fn", "item": item, "file": file, "line": line, "reachable": reachable}


def flags_obj(verify=True, trust_callers=False, foreign_checks="requires", infer=False, rust_extern=(), rust_search=()):
    return {"verify": verify, "trust_callers": trust_callers, "foreign_checks": foreign_checks, "infer": infer,
            "rust_extern": list(rust_extern), "rust_search": list(rust_search), "proof_cache": False}


def check(name, ok, detail=""):
    TOTAL[0] += 1
    if not ok:
        FAILS.append("FAIL %s: %s" % (name, str(detail).strip()[-700:]))


def prefix_of(op):
    for p in CATALOG_FAMILIES:
        if op.startswith(p):
            return p
    return op.split("_")[0] + "_"


class R:
    """result of one compile on one compiler"""

    def __init__(self, rc, text, rep, raw, out, od):
        self.rc, self.text, self.rep, self.raw, self.out, self.od = rc, text, rep, raw, out, od


class Harness:
    def __init__(self, work, binary):
        self.work = Path(work) / "g4b_trust"
        self.work.mkdir(parents=True)
        self.comps = [("py", [sys.executable, str(ROOT / "pruvc" / "pruvc.py")])]
        if binary:
            self.comps.append(("rs", [binary]))
        self.n = 0

    def project(self, files):
        """files: {relative path: text or bytes}; returns the project directory"""
        self.n += 1
        d = self.work / ("p%d" % self.n)
        d.mkdir(parents=True, exist_ok=True)
        for rel, data in files.items():
            p = d / rel
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_bytes(data if isinstance(data, bytes) else data.encode("utf-8"))
        return d

    def compile(self, root, flags=("--no-verify",), manifest=None, outdir=None, stem="out", llvm=False):
        """run every compiler on root; outputs go to OUTDIR/<label>/ ; -> {label: R}"""
        self.n += 1
        outdir = Path(outdir) if outdir is not None else self.work / ("o%d" % self.n)
        res = {}
        for label, cmd in self.comps:
            od = outdir / label
            od.mkdir(parents=True, exist_ok=True)
            out = od / (stem + (".ll" if llvm else ".rs"))
            report = od / "trust.json"
            args = list(cmd) + [str(root), "-o", str(out)] + list(flags)
            if llvm:
                args += ["--backend", "llvm", "--rust-bindings", str(od / "bindings.rs")]
            if manifest is not None:
                args += ["--manifest", str(manifest)]
            args += ["--trust-report", str(report)]
            c = subprocess.run(args, capture_output=True, text=True, timeout=300)
            raw, rep = None, None
            if report.is_file():
                raw = report.read_bytes()
                try:
                    rep = json.loads(raw.decode("utf-8"))
                except ValueError:
                    rep = None
            res[label] = R(c.returncode, c.stdout + c.stderr, rep, raw, out, od)
        return res


def schema_errors(rep, label):
    errs = []
    if not isinstance(rep, dict) or set(rep) != {"format", "compiler", "flags", "packages", "proof_cache"}:
        return ["top-level keys: %s" % (sorted(rep) if isinstance(rep, dict) else rep)]
    if rep["format"] != 1:
        errs.append("format %r" % (rep["format"],))
    c = rep["compiler"]
    if not isinstance(c, dict) or set(c) != {"impl", "version", "solver"}:
        errs.append("compiler keys %r" % (c,))
    else:
        if c["impl"] != label:
            errs.append("compiler.impl %r != %r" % (c["impl"], label))
        for k in ("version", "solver"):
            if not isinstance(c[k], str) or not c[k]:
                errs.append("compiler.%s not a non-empty string: %r" % (k, c[k]))
    f = rep["flags"]
    if not isinstance(f, dict) or set(f) != set(flags_obj()):
        errs.append("flags keys %r" % (f,))
    elif f["proof_cache"] is not False:
        errs.append("flags.proof_cache %r" % (f["proof_cache"],))
    if rep["proof_cache"] != {"hits": 0, "stores": 0}:
        errs.append("proof_cache %r" % (rep["proof_cache"],))
    pk = rep["packages"]
    if not isinstance(pk, list) or not pk:
        return errs + ["packages empty"]
    if pk[0].get("display") != "":
        errs.append("first package display %r" % (pk[0].get("display"),))
    names = [p.get("name") for p in pk[1:]]
    if names != sorted(names):
        errs.append("dependencies not sorted by name: %r" % (names,))
    for p in pk:
        if set(p) != {"name", "version", "display", "sha256", "trusted", "catalog"}:
            errs.append("package keys %r" % (sorted(p),))
            continue
        if p is not pk[0] and p["display"] != "@pkg/" + p["name"]:
            errs.append("display %r for %r" % (p["display"], p["name"]))
        if not re.fullmatch(r"[0-9a-f]{64}", str(p["sha256"])):
            errs.append("sha256 %r" % (p["sha256"],))
        keys = []
        for t in p["trusted"]:
            if set(t) != {"kind", "item", "file", "line", "reachable"} or not isinstance(t["reachable"], bool) \
                    or not isinstance(t["line"], int):
                errs.append("trusted entry %r" % (t,))
                continue
            KINDS_SEEN.add(t["kind"])
            keys.append((t["file"], t["line"], t["item"]))
        if keys != sorted(keys):
            errs.append("trusted not sorted in %r" % (p["name"],))
        ck = []
        for e in p["catalog"]:
            if set(e) != {"op", "type_args"} or not isinstance(e["type_args"], list) \
                    or not all(isinstance(x, str) for x in e["type_args"]):
                errs.append("catalog entry %r" % (e,))
                continue
            ck.append((e["op"], e["type_args"]))
        if ck != sorted(ck) or len(set((o, tuple(a)) for o, a in ck)) != len(ck):
            errs.append("catalog not sorted/deduplicated in %r" % (p["name"],))
    return errs


def basic(name, res, rc=0):
    """exit code, report written, canonical bytes, schema, and equality across compilers"""
    for label, r in res.items():
        tag = "%s [%s]" % (name, label)
        check(tag + ": exit code", r.rc == rc, "exit %d (expected %d)\n%s" % (r.rc, rc, r.text))
        check(tag + ": report written", r.rep is not None, r.text)
        if r.rep is None:
            continue
        check(tag + ": canonical bytes", r.raw == canon(r.rep).encode("utf-8"), r.raw[:300])
        errs = schema_errors(r.rep, label)
        check(tag + ": schema", not errs, "; ".join(errs))
    reps = [r.rep for r in res.values()]
    if len(res) == 2 and all(x is not None for x in reps):
        a, b = strip(reps[0]), strip(reps[1])
        check(name + ": reports equal across compilers", a == b, "py: %s\nrs: %s" % (
            json.dumps(reps[0]["packages"])[:350], json.dumps(reps[1]["packages"])[:350]))


def expect(name, res, pred):
    """pred(rep) -> True/False or an error string, for the report of every compiler"""
    for label, r in res.items():
        if r.rep is None:
            check("%s [%s]" % (name, label), False, "no report\n" + r.text)
            continue
        ok = pred(r.rep)
        check("%s [%s]" % (name, label), ok is True, ok if isinstance(ok, str) else json.dumps(r.rep)[:700])


def pk(rep, name):
    for p in rep["packages"]:
        if p["name"] == name:
            return p
    return None


def fam_note(case, res):
    for label, r in res.items():
        if r.rep is None:
            continue
        s = FAMILIES_SEEN.setdefault((case, label), set())
        for p in r.rep["packages"]:
            for e in p["catalog"]:
                s.add(prefix_of(e["op"]))


def fam_ops(rep, pkg_name, prefixes):
    """True if the package has catalog entries and every entry has one of the prefixes"""
    p = pk(rep, pkg_name)
    return p is not None and bool(p["catalog"]) and all(e["op"].startswith(tuple(prefixes)) for e in p["catalog"])


# ---- the cases ----------------------------------------------------------------------------------------------------
def case_trusted(h):
    """1. trusted kind: called / never called program fns and a dependency's trusted fn"""
    d = h.project(dict({"app/pruv.json": man("app", deps={"util": dep("../util")}), "app/app.pruv": APP1},
                       **pkg_files("util", "util", UTIL1)))
    res = h.compile(d / "app" / "app.pruv", manifest=d / "app" / "pruv.json")
    basic("trusted", res)
    uf = "@pkg/util/util.pruv"
    expect("trusted: packages and entries", res, lambda rep: (
        [p["name"] for p in rep["packages"]] == ["app", "util"]
        and pk(rep, "app")["trusted"] == [tr("crate::rust_min", "app.pruv", lineno(APP1, "fn rust_min"), True),
                                          tr("crate::spare_min", "app.pruv", lineno(APP1, "fn spare_min"), False)]
        and pk(rep, "util")["trusted"] == [tr("@util::dep_min", uf, lineno(UTIL1, "fn dep_min"), True)]) or json.dumps(
        rep["packages"])[:700])
    expect("trusted: version, display, sha256", res, lambda rep: (
        pk(rep, "app")["version"] == "0.1.0" and pk(rep, "util")["display"] == "@pkg/util"
        and pk(rep, "app")["sha256"] == sha_of([("app.pruv", (d / "app" / "app.pruv").read_bytes())])
        and pk(rep, "util")["sha256"] == sha_of([(uf, (d / "util" / "util.pruv").read_bytes())])) or "version/hash")
    return d, res


def case_catalog(h):
    """2. one case per catalog family; ops of a dependency and of the program land in their own package"""
    for case, fam, text, prefixes in (("collections", "collections", COLLECTIONS, ("vec_", "map_", "string_")),
                                      ("file", "file", FILE, ("file_",)), ("process", "process", PROCESS, ("proc_",)),
                                      ("net", "net", NET, ("net_",)), ("loop", "loop", LOOP, ("ev_",))):
        d = h.project({"prog.pruv": text})
        res = h.compile(d / "prog.pruv")
        basic("catalog " + case, res)
        fam_note(fam, res)
        expect("catalog %s: ops of %s" % (case, "/".join(prefixes)), res, lambda rep, pf=prefixes: (
            len(rep["packages"]) == 1 and all(any(e["op"].startswith(x) for x in pf) for e in rep["packages"][0]["catalog"])
            and all(any(e["op"].startswith(x) for e in rep["packages"][0]["catalog"]) for x in pf)) or
               json.dumps(rep["packages"])[:700])
    # collections: every collection family has at least one op with its element types
    d = h.project({"prog.pruv": COLLECTIONS})
    res = h.compile(d / "prog.pruv")
    expect("catalog collections: type_args", res, lambda rep: any(
        e["op"].startswith("map_") and len(e["type_args"]) == 2 for e in rep["packages"][0]["catalog"]) or "map type_args")
    # a dependency and the program use different ops
    d = h.project(dict({"app/pruv.json": man("app", deps={"ds": dep("../ds")}), "app/app.pruv": APP2},
                       **pkg_files("ds", "ds", DS2)))
    res = h.compile(d / "app" / "app.pruv", manifest=d / "app" / "pruv.json")
    basic("catalog split", res)
    fam_note("collections", res)
    expect("catalog split: ops in the right package", res, lambda rep: (
        fam_ops(rep, "app", ("string_",)) and fam_ops(rep, "ds", ("vec_",))) or json.dumps(rep["packages"])[:700])


def case_flags(h):
    """3. each flag in `flags`"""
    d = h.project({"prog.pruv": INC, "ext.pruv": EXT_PROG, "a.rlib": b"dummy a", "b.rlib": b"dummy b",
                   "s1/keep": "", "s2/keep": ""})
    prog, ext = d / "prog.pruv", d / "ext.pruv"
    a, b = str(d / "a.rlib"), str(d / "b.rlib")
    s1, s2 = str(d / "s1"), str(d / "s2")
    table = [
        ("default", prog, [], flags_obj()),
        ("--no-verify", prog, ["--no-verify"], flags_obj(verify=False)),
        ("--trust-callers", prog, ["--trust-callers"], flags_obj(trust_callers=True)),
        ("--foreign-checks all", prog, ["--foreign-checks", "all"], flags_obj(foreign_checks="all")),
        ("--foreign-checks off", prog, ["--foreign-checks", "off"], flags_obj(foreign_checks="off")),
        ("--infer", prog, ["--infer"], flags_obj(infer=True)),
        ("--rust-extern x2", ext, ["--no-verify", "--rust-extern", "ext_a=" + a, "--rust-extern", "ext_b=" + b],
         flags_obj(verify=False, rust_extern=["ext_a=" + a, "ext_b=" + b])),
        ("--rust-extern reversed", ext, ["--no-verify", "--rust-extern", "ext_b=" + b, "--rust-extern", "ext_a=" + a],
         flags_obj(verify=False, rust_extern=["ext_b=" + b, "ext_a=" + a])),
        ("--rust-search x2", prog, ["--no-verify", "--rust-search", s2, "--rust-search", s1],
         flags_obj(verify=False, rust_search=[s2, s1])),
        ("all flags", ext, ["--trust-callers", "--infer", "--foreign-checks", "all", "--rust-extern", "ext_a=" + a,
                            "--rust-extern", "ext_b=" + b, "--rust-search", s1],
         flags_obj(trust_callers=True, foreign_checks="all", infer=True, rust_extern=["ext_a=" + a, "ext_b=" + b],
                   rust_search=[s1])),
    ]
    for name, root, flags, want in table:
        res = h.compile(root, flags=flags)
        basic("flags " + name, res)
        expect("flags %s: flags object" % name, res, lambda rep, w=want: rep["flags"] == w or json.dumps(rep["flags"]))
    # the extern program's trusted fns are listed with their plain names (legacy single file)
    res = h.compile(ext, flags=["--no-verify", "--rust-extern", "ext_a=" + a, "--rust-extern", "ext_b=" + b])
    expect("flags: extern program trusted", res, lambda rep: (
        [t["item"] for t in rep["packages"][0]["trusted"]] == ["one", "two"]
        and all(t["reachable"] for t in rep["packages"][0]["trusted"])) or json.dumps(rep["packages"])[:500])


def case_fail(h):
    """4. failing proof: exit 1, report written with the right packages"""
    d = h.project(dict({"app/pruv.json": man("app", deps={"util": dep("../util")}), "app/app.pruv": APP4},
                       **pkg_files("util", "util", UTIL4)))
    res = h.compile(d / "app" / "app.pruv", flags=[], manifest=d / "app" / "pruv.json")
    basic("failing proof", res, rc=1)
    expect("failing proof: packages", res, lambda rep: (
        [p["name"] for p in rep["packages"]] == ["app", "util"] and rep["flags"]["verify"] is True
        and pk(rep, "app")["trusted"] == [tr("crate::rust_min", "app.pruv", lineno(APP4, "fn rust_min"), True)]
        and pk(rep, "util")["trusted"] == [tr("@util::dep_min", "@pkg/util/util.pruv", lineno(UTIL4, "fn dep_min"), True)])
           or json.dumps(rep["packages"])[:700])
    for label, r in res.items():
        check("failing proof [%s]: diagnostic on stdout" % label, '"ok": true' not in r.text, r.text)


def case_nomanifest(h):
    """5. no manifest: legacy single file, and an explicit-mode multi-file program"""
    d = h.project({"prog.pruv": LEGACY})
    res = h.compile(d / "prog.pruv")
    basic("legacy", res)
    raw = (d / "prog.pruv").read_bytes()
    expect("legacy: one package, plain item names", res, lambda rep: (
        len(rep["packages"]) == 1 and rep["packages"][0]["name"] == "" and rep["packages"][0]["version"] == ""
        and rep["packages"][0]["display"] == "" and rep["packages"][0]["sha256"] == sha_of([("prog.pruv", raw)])
        and [(t["item"], t["line"], t["reachable"]) for t in rep["packages"][0]["trusted"]] == [
            ("rust_min", lineno(LEGACY, "fn rust_min"), True), ("spare_min", lineno(LEGACY, "fn spare_min"), False)]
        and all(t["file"] in ("", "prog.pruv") for t in rep["packages"][0]["trusted"])) or json.dumps(rep["packages"])[:700])
    d = h.project({"root.pruv": EXPL_ROOT, "helper.pruv": EXPL_HELPER})
    res = h.compile(d / "root.pruv")
    basic("explicit no manifest", res)
    expect("explicit no manifest: items, files, hash", res, lambda rep: (
        len(rep["packages"]) == 1 and rep["packages"][0]["name"] == ""
        and rep["packages"][0]["sha256"] == sha_of([("root.pruv", (d / "root.pruv").read_bytes()),
                                                     ("helper.pruv", (d / "helper.pruv").read_bytes())])
        and rep["packages"][0]["trusted"] == [
            tr("crate::helper::helper_min", "helper.pruv", lineno(EXPL_HELPER, "fn helper_min"), True),
            tr("crate::rust_min", "root.pruv", lineno(EXPL_ROOT, "fn rust_min"), True)]) or json.dumps(rep["packages"])[:700])


def case_std_and_paths(h):
    """the bundled std is not hashed; a report path equal to the input or an output path is a usage error"""
    text = "mod std;\npub fn f() -> i64 { return 1; }\n"
    d = h.project({"root.pruv": text})
    res = h.compile(d / "root.pruv")
    basic("mod std", res)
    expect("mod std: hash of root.pruv only", res, lambda rep: (
        rep["packages"][0]["sha256"] == sha_of([("root.pruv", text.encode("utf-8"))])) or json.dumps(rep["packages"]))
    root = str(d / "root.pruv")
    out = str(d / "out.rs")
    for name, extra, rpath in (("input", [], root), ("-o", [], out), ("foreign manifest", [], out + ".foreign.json"),
                               ("bindings", ["--backend", "llvm", "--rust-bindings", str(d / "b.rs")], str(d / "b.rs"))):
        for label, cmd in h.comps:
            o = str(d / "out.ll") if extra else out
            c = subprocess.run(list(cmd) + [root, "-o", o, "--no-verify", "--trust-report", rpath] + extra,
                               capture_output=True, text=True, timeout=300)
            check("report path = %s [%s]: exit 2" % (name, label), c.returncode == 2, c.stdout + c.stderr)
    check("report path = input: input kept", (d / "root.pruv").read_text(encoding="utf-8") == text, "")
    # aliases: a link followed before `..`, a link to an output not written yet, an empty --rust-bindings (= default)
    d2 = h.project({"real/root.pruv": text})
    (d2 / "real" / "sub").mkdir()
    os.symlink(str(d2 / "real" / "sub"), str(d2 / "alias"))
    os.symlink(str(d2 / "real" / "out.rs"), str(d2 / "report.json"))
    root2 = str(d2 / "real" / "root.pruv")
    for name, o, rpath, extra in (
            ("link then ..", str(d2 / "real" / "out.rs"), str(d2 / "alias" / ".." / "root.pruv"), []),
            ("link to unwritten -o", str(d2 / "real" / "out.rs"), str(d2 / "report.json"), []),
            ("empty bindings", str(d2 / "real" / "out.ll"), str(d2 / "real" / "out.rs"),
             ["--backend", "llvm", "--rust-bindings", ""])):
        for label, cmd in h.comps:
            c = subprocess.run(list(cmd) + [root2, "-o", o, "--no-verify", "--trust-report", rpath] + extra,
                               capture_output=True, text=True, timeout=300)
            check("report path = %s [%s]: exit 2" % (name, label), c.returncode == 2, c.stdout + c.stderr)
    check("report path aliases: input kept", (d2 / "real" / "root.pruv").read_text(encoding="utf-8") == text, "")


def three_deps(extra_alpha=""):
    files = {"app/pruv.json": man("app", deps={"zeta": dep("../zeta"), "alpha": dep("../alpha"), "mid": dep("../mid")}),
             "app/app.pruv": "mod zeta;\nmod alpha;\nmod mid;\npub fn all(x: i64) -> i64 "
                             "{ return crate::zeta::fz(crate::alpha::fa(crate::mid::fm(x))); }\n"}
    for name, fn in (("zeta", "fz"), ("alpha", "fa"), ("mid", "fm")):
        text = "pub fn %s(x: i64) -> i64 { return x; }\n" % fn
        files.update(pkg_files(name, name, text + (extra_alpha if name == "alpha" else "")))
    return files


def case_sorting(h):
    """6. packages sorted by name; sha256 recomputed from the files; one changed byte changes one hash only"""
    d1 = h.project(three_deps())
    res1 = h.compile(d1 / "app" / "app.pruv", manifest=d1 / "app" / "pruv.json")
    basic("sorting", res1)
    want = {"app": sha_of([("app.pruv", (d1 / "app" / "app.pruv").read_bytes())])}
    for n in ("alpha", "mid", "zeta"):
        want[n] = sha_of([("@pkg/%s/%s.pruv" % (n, n), (d1 / n / (n + ".pruv")).read_bytes())])
    expect("sorting: order and hashes", res1, lambda rep: (
        [p["name"] for p in rep["packages"]] == ["app", "alpha", "mid", "zeta"]
        and {p["name"]: p["sha256"] for p in rep["packages"]} == want
        and all(p["trusted"] == [] and p["catalog"] == [] for p in rep["packages"])) or json.dumps(rep["packages"])[:700])
    d2 = h.project(three_deps("// one more comment line\n"))
    res2 = h.compile(d2 / "app" / "app.pruv", manifest=d2 / "app" / "pruv.json")
    basic("sorting changed", res2)
    for label in res1:
        a, b = res1[label].rep, res2[label].rep
        if a is None or b is None:
            check("sorting changed [%s]" % label, False, "no report")
            continue
        ha = {p["name"]: p["sha256"] for p in a["packages"]}
        hb = {p["name"]: p["sha256"] for p in b["packages"]}
        check("sorting changed [%s]: only alpha differs" % label,
              [n for n in ha if ha[n] != hb.get(n)] == ["alpha"], "%s vs %s" % (ha, hb))
        check("sorting changed [%s]: rest of the report equal" % label, [
            {k: v for k, v in p.items() if k != "sha256"} for p in a["packages"]] == [
            {k: v for k, v in p.items() if k != "sha256"} for p in b["packages"]], "")


def case_determinism(h):
    """7. the same input twice (in two directories) gives the same bytes"""
    files = dict({"app/pruv.json": man("app", deps={"util": dep("../util")}), "app/app.pruv": APP1},
                 **pkg_files("util", "util", UTIL1))
    ds = [h.project(files), h.project(files)]
    rs = [h.compile(d / "app" / "app.pruv", manifest=d / "app" / "pruv.json") for d in ds]
    for label in rs[0]:
        a, b = rs[0][label], rs[1][label]
        check("determinism [%s]" % label, a.raw is not None and a.raw == b.raw, "reports differ or are missing\n" + a.text)


def case_backend(h):
    """8. the LLVM backend writes the same report as the Rust source backend"""
    d = h.project(dict({"app/pruv.json": man("app", deps={"util": dep("../util")}), "app/app.pruv": APP1},
                       **pkg_files("util", "util", UTIL1)))
    root, mf = d / "app" / "app.pruv", d / "app" / "pruv.json"
    rust = h.compile(root, manifest=mf, outdir=d / "o_rust")
    llvm = h.compile(root, manifest=mf, outdir=d / "o_llvm", llvm=True)
    for label, r in llvm.items():
        if r.rc == 2:
            SKIPS.append("SKIP backend [%s]: the llvm backend is unavailable (exit 2): %s" % (label, r.text.strip()[-200:]))
            continue
        tag = "backend [%s]" % label
        check(tag + ": llvm exit code", r.rc == 0, "exit %d\n%s" % (r.rc, r.text))
        check(tag + ": llvm report equals the rust report", r.rep is not None and rust[label].rep is not None
              and strip(r.rep) == strip(rust[label].rep), r.text)
        check(tag + ": llvm canonical bytes", r.raw is not None and r.rep is not None and r.raw == canon(r.rep).encode("utf-8"), "")
    live = {k: v for k, v in llvm.items() if v.rc != 2}
    if len(live) == 2:
        check("backend: llvm reports equal across compilers", strip(live["py"].rep or {}) == strip(live["rs"].rep or {}), "")


def scan_ops():
    """catalog op names from pruvc/foreign.py (_add calls) and foreign.rs (CATALOG_NAMES)"""
    py = (ROOT / "pruvc" / "foreign.py").read_text(encoding="utf-8")
    rs = (ROOT / "pruvc-rs" / "crates" / "pruv-core" / "src" / "foreign.rs").read_text(encoding="utf-8")
    py_ops = set(re.findall(r"""^_add\(\s*["'](\w+)["']""", py, re.M))
    i = rs.index("CATALOG_NAMES: [&str;")
    j = rs.index("];", i)
    rs_ops = set(re.findall(r'"(\w+)"', rs[i:j]))
    return py_ops, rs_ops


def case_completeness(h):
    """9. every trust construct has a report case"""
    py_ops, rs_ops = scan_ops()
    check("completeness: catalog scan found ops", len(py_ops) >= 50 and len(rs_ops) >= 50, "%d / %d" % (len(py_ops), len(rs_ops)))
    check("completeness: both compilers list the same catalog ops", py_ops == rs_ops, sorted(py_ops ^ rs_ops))
    for op in sorted(py_ops | rs_ops):
        p = op.split("_")[0] + "_"
        check("completeness: family of %s has a case" % op, p in CATALOG_FAMILIES, "add %r to CATALOG_FAMILIES" % p)
    for p, case in sorted(CATALOG_FAMILIES.items()):
        for label, _ in h.comps:
            check("completeness: case %s produced %s* ops [%s]" % (case, p, label),
                  p in FAMILIES_SEEN.get((case, label), set()), sorted(FAMILIES_SEEN.get((case, label), set())))
    for path in ("pruvc/vparser.py", "pruvc-rs/crates/pruv-core/src/parser.rs"):
        check("completeness: `trusted rust` parsed in " + path, "trusted rust" in (ROOT / path).read_text(encoding="utf-8"), path)
    check("completeness: every kind seen is known", KINDS_SEEN <= TRUST_KINDS, sorted(KINDS_SEEN - TRUST_KINDS))
    check("completeness: every known kind was seen", TRUST_KINDS <= KINDS_SEEN, sorted(TRUST_KINDS - KINDS_SEEN))


def case_mcp(h):
    """10. L3 mcp piece: the real flags of bench/l3/mcp/test_impl.py (without building the host crates)"""
    base = h.work / "mcp"
    base.mkdir()
    (base / "libmcp_host.rlib").write_bytes(b"dummy")
    (base / "libpruv_str.rlib").write_bytes(b"dummy")
    mdir = ROOT / "bench" / "l3" / "mcp"
    flags = ["--no-verify", "--rust-extern", "mcp_host=" + str(base / "libmcp_host.rlib"),
             "--rust-extern", "pruv_str=" + str(base / "libpruv_str.rlib")]
    res = h.compile(mdir / "root.pruv", flags=flags, manifest=mdir / "pruv.json", outdir=base / "out", stem="root_gen")
    basic("mcp", res)
    for label, r in res.items():
        if r.rep is None:
            continue
        tag = "mcp [%s]" % label
        prog, deps = r.rep["packages"][0], r.rep["packages"][1:]
        check(tag + ": dependency trusted items", sorted((p["name"], t["item"]) for p in deps for t in p["trusted"])
              == [("jstr", "@jstr::utf8_to_string")], json.dumps([(p["name"], p["trusted"]) for p in deps])[:600])
        check(tag + ": program trusted", [t["item"] for t in prog["trusted"]] == ["crate::command_missing"], prog["trusted"])
        fj = Path(str(r.out) + ".foreign.json")
        check(tag + ": foreign manifest written", fj.is_file(), str(fj))
        if fj.is_file():
            doc = json.loads(fj.read_text(encoding="utf-8"))
            want = set((e["op"], tuple(e["type_args"])) for e in doc["catalog"])
            got = set((e["op"], tuple(e["type_args"])) for p in r.rep["packages"] for e in p["catalog"])
            check(tag + ": catalog union equals the foreign manifest", want == got and want,
                  "only manifest: %s; only report: %s" % (sorted(want - got)[:5], sorted(got - want)[:5]))
            fams = set(prefix_of(op) for op, _ in want)
            check(tag + ": catalog families", fams <= {"string_", "proc_", "ev_", "vec_", "map_"}, sorted(fams))


def main(work, binary=None):
    h = Harness(work, binary)
    for fn in (case_trusted, case_catalog, case_flags, case_fail, case_nomanifest, case_std_and_paths, case_sorting,
               case_determinism,
               case_backend, case_mcp, case_completeness):   # completeness last: it needs what the others saw
        try:
            fn(h)
        except Exception as e:  # a broken case is a failure, not a crash of the whole run
            TOTAL[0] += 1
            FAILS.append("FAIL %s: exception %s: %s" % (fn.__name__, type(e).__name__, e))
    for s in SKIPS:
        print(s)
    for f in FAILS:
        print(f)
    print("trust: %d/%d passed" % (TOTAL[0] - len(FAILS), TOTAL[0]))
    return 1 if FAILS else 0


if __name__ == "__main__":
    args = sys.argv[1:]
    rs = None
    if "--rs" in args:
        i = args.index("--rs")
        rs = args[i + 1]
        del args[i:i + 2]
    if len(args) != 1:
        print(__doc__)
        sys.exit(2)
    sys.exit(main(args[0], rs))
