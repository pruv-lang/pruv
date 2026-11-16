"""G4b-1 (docs/G4B_SPEC.md sections 3, 4, 5): manifests, package namespaces, identity, versions, on the selected compiler.

usage: check_pkg.py WORK [--rs PATH] [--py PATH]
  --rs PATH: the Rust driver (default: the Python compiler)
  --py PATH: the pruvc.py to run when --rs is absent (default: pruvc/pruvc.py of this repository)
Prints one line per failing case; exit 0 only if every case passes."""
from pathlib import Path
import json
import os
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]

INC = ("pub fn inc(x: i64) -> i64\n  requires 0 <= x && x <= 10\n  ensures result == x + 1\n"
       "{ return x + 1; }\n")
APP_JSON = ("mod json;\npub fn f(x: i64) -> i64\n  requires 0 <= x && x <= 10\n  ensures result == x + 1\n"
            "{ return crate::json::inc(x); }\n")
APP_PLAIN = "pub fn f(x: i64) -> i64 { return x; }\n"
FIX3 = "see docs/G4B_SPEC.md section 3"


def man(name, version="0.1.0", root=None, deps=None, **extra):
    """manifest text; root defaults to NAME.pruv"""
    d = {"name": name, "version": version, "root": root if root is not None else name + ".pruv"}
    if deps is not None:
        d["dependencies"] = deps
    d.update(extra)
    return json.dumps(d)


def dep(path, version="0.1"):
    return {"path": path, "version": version}


def pkg_files(prefix, name, root_text, version="0.1.0", deps=None, extra_files=None):
    """files of one package directory `prefix`: pruv.json + NAME.pruv (+ others)"""
    f = {prefix + "/pruv.json": man(name, version, deps=deps), prefix + "/" + name + ".pruv": root_text}
    for k, v in (extra_files or {}).items():
        f[prefix + "/" + k] = v
    return f


def app_js_text():
    """program root that mounts the dependency under the alias `js`"""
    return APP_JSON.replace("mod json;", "mod js;").replace("crate::json::inc", "crate::js::inc")


def code_of(d):
    return str(d.get("code", "")).split("/")[-1]


class Runner:
    def __init__(self, work, binary, py=None):
        self.work = Path(work) / "g4b_pkg"
        self.work.mkdir(parents=True)
        pyc = str(Path(py).resolve()) if py else str(ROOT / "pruvc" / "pruvc.py")
        self.cmd = [binary] if binary else [sys.executable, pyc]
        self.n = 0

    def project(self, files):
        """files: {relative path: text}; returns the project directory"""
        self.n += 1
        d = self.work / ("p%d" % self.n)
        d.mkdir(parents=True, exist_ok=True)
        for rel, text in files.items():
            p = d / rel
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(text)
        return d

    def run(self, root, manifest=None, flags=("--no-verify",), extra=(), out=None):
        out = out or (root.parent / (root.name + ".rs"))
        args = list(self.cmd) + [str(root), "-o", str(out)] + list(flags)
        if manifest is not None:
            args += ["--manifest", str(manifest)]
        args += list(extra)
        c = subprocess.run(args, capture_output=True, text=True, timeout=300)
        diags = []
        for line in c.stdout.splitlines():
            try:
                d = json.loads(line)
            except ValueError:
                continue
            if isinstance(d, dict) and "code" in d:
                diags.append(d)
        return c.returncode, diags, c.stdout + c.stderr, out


def main(work, binary=None, py=None):
    r = Runner(work, binary, py)
    fails = []

    def check(name, ok, detail=""):
        if not ok:
            fails.append("FAIL %s: %s" % (name, str(detail).strip()[-300:]))

    def app(p, flags=("--no-verify",), extra=(), out=None, name="app", manifest=None):
        return r.run(p / name / (name + ".pruv"), manifest or (p / name / "pruv.json"), flags, extra, out)

    def pkg_diag(rc, ds, code, message=None, fix=None, file=None, line=None):
        """exactly one diagnostic: stage pkg, the given code (prefix `pkg/` optional), exit 1"""
        if rc != 1 or len(ds) != 1:
            return False
        d = ds[0]
        if code_of(d) != code or d.get("stage") != "pkg":
            return False
        if message is not None and d.get("message") != message:
            return False
        if fix is not None and d.get("fix") != fix:
            return False
        if file is not None and d.get("file") != file:
            return False
        if line is not None and (d.get("line"), d.get("col")) != line:
            return False
        return True

    def no_path(p, txt):
        return str(p) not in txt and str(p.resolve()) not in txt

    # ---- manifest_invalid: program manifest -------------------------------------------------------------------------
    def bad_manifest(label, text, message, file="pruv.json", extra_files=None):
        files = {"app/pruv.json": text, "app/app.pruv": APP_PLAIN}
        files.update(extra_files or {})
        p = r.project(files)
        rc, ds, txt, _ = app(p)
        check("manifest_invalid: " + label,
              pkg_diag(rc, ds, "manifest_invalid", message, FIX3, file, (1, 1)) and no_path(p, txt), "exit %d %s" % (rc, txt))

    good = {"name": "app", "version": "0.1.0", "root": "app.pruv"}

    def without(key):
        d = dict(good)
        del d[key]
        return json.dumps(d)

    bad_manifest("not an object", "[1]", "invalid manifest: not a UTF-8 JSON object")
    bad_manifest("not JSON", "this is {not json", "invalid manifest: not a UTF-8 JSON object")
    for k in ("name", "version", "root"):
        bad_manifest("missing " + k, without(k), "invalid manifest key %s: missing" % k)
    bad_manifest("unknown key", man("app", extra=1), "invalid manifest key extra: unknown key")
    bad_manifest("duplicate key", '{"name": "app", "name": "app", "version": "0.1.0", "root": "app.pruv"}',
                 "invalid manifest key name: duplicate key")
    bad_manifest("version not a string", json.dumps(dict(good, version=5)), "invalid manifest key version: expected a string")
    bad_manifest("dependencies not an object", json.dumps(dict(good, dependencies=[])),
                 "invalid manifest key dependencies: expected an object")
    for bad in ("Json", "a__b", "pruv_x", "std", "crate"):
        bad_manifest("bad name " + bad, man(bad, root="app.pruv"), "invalid manifest key name: bad package name")
    for bad in ("1.0", "01.0.0", "1.0.0-rc1", "2147483648.0.0"):
        bad_manifest("bad version " + bad, man("app", bad, root="app.pruv"), "invalid manifest key version: bad version")
    for bad in ("../x.pruv", "/abs.pruv", "x.txt"):
        bad_manifest("bad root " + bad, man("app", root=bad), "invalid manifest key root: bad root path")
    bad_manifest("root file not found", man("app", root="nope.pruv"), "invalid manifest key root: root file not found")
    jd = pkg_files("json", "json", INC)
    bad_manifest("dependency path without pruv.json", man("app", root="app.pruv", deps={"json": dep("../nodir")}),
                 "invalid manifest key dependencies.json.path: no pruv.json in the dependency directory")
    bad_manifest("bad mount name Json", man("app", root="app.pruv", deps={"Json": dep("../json")}),
                 "invalid manifest key dependencies.Json: bad mount name", extra_files=jd)
    bad_manifest("bad mount name std", man("app", root="app.pruv", deps={"std": dep("../json")}),
                 "invalid manifest key dependencies.std: bad mount name", extra_files=jd)
    bad_manifest("bad requirement", man("app", root="app.pruv", deps={"json": dep("../json", "x")}),
                 "invalid manifest key dependencies.json.version: bad version requirement", extra_files=jd)
    bad_manifest("unknown dependency entry key",
                 man("app", root="app.pruv", deps={"json": {"path": "../json", "version": "0.1", "extra": 1}}),
                 "invalid manifest key dependencies.json.extra: unknown key", extra_files=jd)

    # manifest_invalid inside a dependency reports @pkg/KEY/pruv.json
    f = {"app/pruv.json": man("app", deps={"json": dep("../json")}), "app/app.pruv": APP_JSON,
         "json/pruv.json": json.dumps({"version": "0.1.0", "root": "json.pruv"}), "json/json.pruv": INC}
    p = r.project(f)
    rc, ds, txt, _ = app(p)
    check("manifest_invalid in a dependency",
          pkg_diag(rc, ds, "manifest_invalid", "invalid manifest key name: missing", FIX3, "@pkg/json/pruv.json", (1, 1))
          and no_path(p, txt), "exit %d %s" % (rc, txt))

    # (a) a root file that is a symlink leading outside the package directory
    p = r.project({"app/pruv.json": man("app"), "outside.pruv": APP_PLAIN})
    try:
        os.symlink("../outside.pruv", str(p / "app" / "app.pruv"))
        linked = True
    except (AttributeError, NotImplementedError, OSError):
        linked = False  # symlinks unavailable: skip the case
    if linked:
        rc, ds, txt, _ = app(p)
        check("manifest_invalid: root symlink outside the package",
              pkg_diag(rc, ds, "manifest_invalid", "invalid manifest key root: root file is outside the package directory",
                       FIX3, "pruv.json", (1, 1)) and no_path(p, txt), "exit %d %s" % (rc, txt))

    # (b) mount keys that `mod KEY;` cannot declare
    for key in ("crate", "fn", "pruv_x", "a__b"):
        bad_manifest("bad mount name " + key, man("app", root="app.pruv", deps={key: dep("../json")}),
                     "invalid manifest key dependencies.%s: bad mount name" % key, extra_files=jd)

    # (c) a dependency mounted under an alias is reported under its real name
    f = {"app/pruv.json": man("app", deps={"js": dep("../json")}), "app/app.pruv": app_js_text()}
    f.update(pkg_files("json", "json", INC, deps={"lex": dep("../lex")}))
    f.update(pkg_files("lex", "lex", INC))
    p = r.project(f)
    rc, ds, txt, _ = app(p)
    check("alias dependency reports the package name",
          pkg_diag(rc, ds, "unused_dependency", file="@pkg/json/pruv.json") and no_path(p, txt), "exit %d %s" % (rc, txt))

    # ---- CLI usage errors -------------------------------------------------------------------------------------------
    p = r.project({"app/pruv.json": man("app"), "app/app.pruv": APP_PLAIN, "lib/a.pruv": INC})
    rc, ds, txt, _ = app(p, extra=("--lib", str(p / "lib")))
    check("--manifest with --lib", rc == 2 and "--manifest and --lib cannot be used together" in txt, "exit %d %s" % (rc, txt))
    p = r.project({"app/pruv.json": man("app", root="other.pruv"), "app/app.pruv": APP_PLAIN, "app/other.pruv": APP_PLAIN})
    rc, ds, txt, _ = app(p)
    check("root mismatch", rc == 2 and "--manifest root does not match the input file" in txt, "exit %d %s" % (rc, txt))

    # ---- dependency cycles ------------------------------------------------------------------------------------------
    f = {"app/pruv.json": man("app", deps={"a": dep("../a")}), "app/app.pruv": "mod a;\n" + APP_PLAIN}
    f.update(pkg_files("a", "a", "mod b;\n" + INC, deps={"b": dep("../b")}))
    f.update(pkg_files("b", "b", "mod a;\n" + INC, deps={"a": dep("../a")}))
    p = r.project(f)
    rc, ds, txt, _ = app(p)
    check("cycle of two", pkg_diag(rc, ds, "dependency_cycle", "dependency cycle: a -> b -> a") and no_path(p, txt),
          "exit %d %s" % (rc, txt))
    f = {"app/pruv.json": man("app", deps={"a": dep("../a")}), "app/app.pruv": "mod a;\n" + APP_PLAIN}
    f.update(pkg_files("a", "a", "mod b;\n" + INC, deps={"b": dep("../b")}))
    f.update(pkg_files("b", "b", "mod c;\n" + INC, deps={"c": dep("../c")}))
    f.update(pkg_files("c", "c", "mod a;\n" + INC, deps={"a": dep("../a")}))
    p = r.project(f)
    rc, ds, txt, _ = app(p)
    check("cycle of three", pkg_diag(rc, ds, "dependency_cycle", "dependency cycle: a -> b -> c -> a") and no_path(p, txt),
          "exit %d %s" % (rc, txt))

    # ---- identity conflicts -----------------------------------------------------------------------------------------
    f = {"app/pruv.json": man("app", deps={"json": dep("../j1", "0.1"), "lines": dep("../lines", "0.1")}),
         "app/app.pruv": "mod json;\nmod lines;\n" + APP_PLAIN}
    f.update({"j1/pruv.json": man("json", "0.1.0", root="json.pruv"), "j1/json.pruv": INC})
    f.update({"j2/pruv.json": man("json", "0.2.0", root="json.pruv"), "j2/json.pruv": INC})
    f.update(pkg_files("lines", "lines", "mod json;\n" + INC, deps={"json": dep("../j2", "0.2")}))
    p = r.project(f)
    rc, ds, txt, _ = app(p)
    check("identity conflict: two versions",
          pkg_diag(rc, ds, "identity_conflict",
                   "package json is loaded from two places: required by app as json (version 0.1.0) "
                   "and by lines as json (version 0.2.0)") and no_path(p, txt), "exit %d %s" % (rc, txt))
    f["j2/pruv.json"] = man("json", "0.1.0", root="json.pruv")
    f["lines/pruv.json"] = man("lines", deps={"json": dep("../j2", "0.1")})
    p = r.project(f)
    rc, ds, txt, _ = app(p)
    check("identity conflict: same version, two directories",
          pkg_diag(rc, ds, "identity_conflict") and no_path(p, txt), "exit %d %s" % (rc, txt))
    f = {"app/pruv.json": man("app", deps={"other": dep("../other")}), "app/app.pruv": "mod other;\n" + APP_PLAIN}
    f.update({"other/pruv.json": man("app", "0.1.0", root="other.pruv"), "other/other.pruv": INC})
    p = r.project(f)
    rc, ds, txt, _ = app(p)
    check("identity conflict: dependency named like the program",
          pkg_diag(rc, ds, "identity_conflict") and no_path(p, txt), "exit %d %s" % (rc, txt))

    # ---- versions ---------------------------------------------------------------------------------------------------
    def version_case(dep_version, requirement):
        f = {"app/pruv.json": man("app", deps={"json": dep("../json", requirement)}), "app/app.pruv": APP_JSON}
        f.update(pkg_files("json", "json", INC, version=dep_version))
        p = r.project(f)
        rc, ds, txt, _ = app(p)
        return p, rc, ds, txt

    p, rc, ds, txt = version_case("0.2.0", "0.1")
    check("version unsatisfied: 0.1 vs 0.2.0",
          pkg_diag(rc, ds, "version_unsatisfied", "package json version 0.2.0 does not satisfy: app requires 0.1")
          and no_path(p, txt), "exit %d %s" % (rc, txt))
    for found, req in (("0.1.5", "0.1"), ("1.4.0", "1")):
        p, rc, ds, txt = version_case(found, req)
        check("version accepted: %s accepts %s" % (req, found), rc == 0, "exit %d %s" % (rc, txt))
    for found, req in (("1.1.9", "1.2"), ("2.0.0", "1")):
        p, rc, ds, txt = version_case(found, req)
        check("version rejected: %s rejects %s" % (req, found), pkg_diag(rc, ds, "version_unsatisfied"),
              "exit %d %s" % (rc, txt))

    # ---- mount conflict, missing module, unused dependency, std -----------------------------------------------------
    f = {"app/pruv.json": man("app", deps={"json": dep("../json")}), "app/app.pruv": APP_JSON, "app/json.pruv": INC}
    f.update(pkg_files("json", "json", INC))
    p = r.project(f)
    rc, ds, txt, _ = app(p)
    check("mount conflict",
          pkg_diag(rc, ds, "mount_conflict", "module json is found in more than one place: json.pruv, @pkg/json",
                   "remove the local file or rename the dependency key") and no_path(p, txt), "exit %d %s" % (rc, txt))

    p = r.project({"app/pruv.json": man("app"), "app/app.pruv": "mod a;\n" + APP_PLAIN})
    rc, ds, txt, _ = app(p)
    check("missing module hint",
          rc == 1 and len(ds) == 1 and code_of(ds[0]) == "module_file" and ds[0].get("stage") == "parse"
          and ds[0].get("fix") == "create a.pruv next to the root file, add a to dependencies, or remove `mod a;`",
          "exit %d %s" % (rc, txt))

    f = {"app/pruv.json": man("app", deps={"json": dep("../json")}), "app/app.pruv": APP_PLAIN}
    f.update(pkg_files("json", "json", INC))
    p = r.project(f)
    rc, ds, txt, _ = app(p)
    check("unused dependency",
          pkg_diag(rc, ds, "unused_dependency",
                   "dependency json is declared but not mounted by `mod json;` in the package root", file="pruv.json")
          and no_path(p, txt), "exit %d %s" % (rc, txt))

    # legacy root (no `mod`, no `pub`) that declares a dependency: it mounts nothing, so the dependency is unused
    f = {"app/pruv.json": man("app", deps={"json": dep("../json")}), "app/app.pruv": "fn f(x: i64) -> i64 { return x; }\n"}
    f.update(pkg_files("json", "json", INC))
    p = r.project(f)
    rc, ds, txt, _ = app(p)
    check("unused dependency: legacy root",
          pkg_diag(rc, ds, "unused_dependency",
                   "dependency json is declared but not mounted by `mod json;` in the package root", file="pruv.json")
          and no_path(p, txt), "exit %d %s" % (rc, txt))

    f = {"app/pruv.json": man("app", deps={"lines": dep("../lines")}), "app/app.pruv": "mod lines;\n" + APP_PLAIN}
    f.update(pkg_files("lines", "lines", INC, deps={"json": dep("../json")}))
    f.update(pkg_files("json", "json", INC))
    p = r.project(f)
    rc, ds, txt, _ = app(p)
    check("unused dependency inside a dependency",
          pkg_diag(rc, ds, "unused_dependency", file="@pkg/lines/pruv.json") and no_path(p, txt), "exit %d %s" % (rc, txt))

    f = {"app/pruv.json": man("app", deps={"json": dep("../json")}), "app/app.pruv": APP_JSON}
    f.update(pkg_files("json", "json", "mod std;\n" + INC))
    p = r.project(f)
    rc, ds, txt, _ = app(p)
    check("std in a dependency",
          pkg_diag(rc, ds, "std_in_dependency", file="@pkg/json/json.pruv") and no_path(p, txt), "exit %d %s" % (rc, txt))

    # ---- positive and semantic cases --------------------------------------------------------------------------------
    f = {"app/pruv.json": man("app", deps={"json": dep("../json")}), "app/app.pruv": APP_JSON}
    f.update(pkg_files("json", "json", INC))
    p = r.project(f)
    rc, ds, txt, _ = app(p)
    check("simple mount", rc == 0, "exit %d %s" % (rc, txt))

    # mount alias: the key differs from the package name; generated Rust is the same as with key = name
    app_js = APP_JSON.replace("mod json;", "mod js;").replace("crate::json::inc", "crate::js::inc")
    fa = {"app/pruv.json": man("app", deps={"js": dep("../json")}), "app/app.pruv": app_js}
    fa.update(pkg_files("json", "json", INC))
    pa = r.project(fa)
    fb = {"app/pruv.json": man("app", deps={"json": dep("../json")}), "app/app.pruv": APP_JSON}
    fb.update(pkg_files("json", "json", INC))
    pb = r.project(fb)
    rc1, _, t1, o1 = app(pa, flags=())
    rc2, _, t2, o2 = app(pb, flags=())
    same = rc1 == 0 and rc2 == 0 and o1.read_text() == o2.read_text()
    check("mount alias: identical generated Rust", same, t1 + t2)
    check("mount alias: pruv_p_f_ symbols", rc1 == 0 and "pruv_p_f_" in o1.read_text(), t1)

    # dependency child module
    f = {"app/pruv.json": man("app", deps={"json": dep("../json")}),
         "app/app.pruv": "mod json;\npub fn f(x: i64) -> i64 { return crate::json::g(x); }\n"}
    f.update(pkg_files("json", "json", "mod q;\npub fn g(x: i64) -> i64 { return crate::q::h(x); }\n",
                       extra_files={"q.pruv": "pub fn h(x: i64) -> i64 { return x; }\n"}))
    p = r.project(f)
    rc, ds, txt, _ = app(p)
    check("dependency child module", rc == 0, "exit %d %s" % (rc, txt))

    # a private item of a dependency is not usable from the program
    f = {"app/pruv.json": man("app", deps={"json": dep("../json")}),
         "app/app.pruv": "mod json;\npub fn f(x: i64) -> i64 { return crate::json::hid(x); }\n"}
    f.update(pkg_files("json", "json", INC + "fn hid(x: i64) -> i64 { return x; }\n"))
    p = r.project(f)
    rc, ds, txt, _ = app(p)
    check("private item of a dependency", rc == 1 and ds, "exit %d %s" % (rc, txt))

    # a dependency's dependency is not reachable
    f = {"app/pruv.json": man("app", deps={"lines": dep("../lines")}),
         "app/app.pruv": "mod lines;\npub fn f(x: i64) -> i64 { return crate::lines::json::inc(x); }\n"}
    f.update(pkg_files("lines", "lines", "mod json;\n" + INC, deps={"json": dep("../json")}))
    f.update(pkg_files("json", "json", INC))
    p = r.project(f)
    rc, ds, txt, _ = app(p)
    check("dependency of a dependency not reachable", rc == 1 and ds, "exit %d %s" % (rc, txt))

    # a package that is not mounted is not reachable
    f = {"app/pruv.json": man("app", deps={"lines": dep("../lines")}),
         "app/app.pruv": "mod lines;\npub fn f(x: i64) -> i64 { return crate::json::inc(x); }\n"}
    f.update(pkg_files("lines", "lines", "mod json;\n" + INC, deps={"json": dep("../json")}))
    f.update(pkg_files("json", "json", INC))
    p = r.project(f)
    rc, ds, txt, _ = app(p)
    check("unmounted package not reachable", rc == 1 and ds, "exit %d %s" % (rc, txt))

    # diamond: the program and `lines` both mount json (same directory); one Value type, emitted once
    json_root = ("pub struct Value { n: i64 }\n"
                 "pub fn get(v: Value) -> i64\n  requires 0 <= v.n\n  ensures result == v.n\n{ return v.n; }\n")
    lines_root = ("mod json;\npub fn make() -> crate::json::Value\n  ensures result.n == 5\n"
                  "{ return crate::json::Value { n: 5 }; }\n")
    app_d = ("mod json;\nmod lines;\npub fn f() -> i64\n  ensures result == 5\n"
             "{ return crate::json::get(crate::lines::make()); }\n")
    f = {"app/pruv.json": man("app", deps={"json": dep("../json"), "lines": dep("../lines")}), "app/app.pruv": app_d}
    f.update(pkg_files("json", "json", json_root))
    f.update(pkg_files("lines", "lines", lines_root, deps={"json": dep("../json")}))
    p = r.project(f)
    rc, ds, txt, o = app(p, flags=())
    check("diamond compiles with verification", rc == 0, "exit %d %s" % (rc, txt))
    if rc == 0:
        names = re.findall(r"\bfn\s+(pruv_p_f_\w+)", o.read_text())
        check("diamond: definitions emitted once", names and len(names) == len(set(names)), str(names))

    # failing proof in a dependency reports the dependency's display path
    f = {"app/pruv.json": man("app", deps={"json": dep("../json")}),
         "app/app.pruv": APP_JSON.replace("result == x + 1", "result == x + 2")}
    f.update(pkg_files("json", "json", "pub fn inc(x: i64) -> i64\n  requires 0 <= x && x <= 10\n"
                                       "  ensures result == x + 2\n{ return x + 1; }\n"))
    p = r.project(f)
    rc, ds, txt, _ = app(p, flags=())
    check("failing proof in a dependency",
          rc == 1 and ds and all(d.get("file") == "@pkg/json/json.pruv" for d in ds) and no_path(p, txt),
          "exit %d %s" % (rc, txt))

    # a manifest without dependencies changes nothing in the generated Rust
    f = {"app/pruv.json": man("app"), "app/app.pruv": ("pub fn f(x: i64) -> i64\n  requires 0 <= x && x <= 10\n"
                                                       "  ensures result == x + 1\n{ return x + 1; }\n")}
    p = r.project(f)
    root = p / "app" / "app.pruv"
    rc1, _, t1, o1 = r.run(root, p / "app" / "pruv.json", flags=(), out=p / "with.rs")
    rc2, _, t2, o2 = r.run(root, None, flags=(), out=p / "without.rs")
    check("manifest without dependencies: identical Rust",
          rc1 == 0 and rc2 == 0 and o1.read_text() == o2.read_text(), t1 + t2)

    for line in fails:
        print(line)
    if fails:
        return 1
    print("packages: manifest errors, mounts, cycles, identity, versions, visibility, diamond, proofs, CLI, identical Rust: OK")
    return 0


if __name__ == "__main__":
    args = sys.argv[1:]
    opts = {}
    for flag in ("--rs", "--py"):
        if flag in args:
            i = args.index(flag)
            if i + 1 >= len(args):
                print(__doc__)
                sys.exit(2)
            opts[flag] = args[i + 1]
            del args[i:i + 2]
    if len(args) != 1:
        print(__doc__)
        sys.exit(2)
    sys.exit(main(args[0], opts.get("--rs"), opts.get("--py")))
