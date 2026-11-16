"""G4b-0 (docs/G4B_LIB_SPEC.md): `--lib DIR` library directories, on the selected compiler.

usage: check_lib.py WORK [--rs PATH]     (PATH: the Rust driver; default: the Python compiler)
Prints one line per failing case; exit 0 only if every case passes."""
from pathlib import Path
import json
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]

INC = ("pub fn inc(x: i64) -> i64\n  requires 0 <= x && x <= 10\n  ensures result == x + 1\n"
       "{ return x + 1; }\n")
ROOT_A = ("mod a;\npub fn f(x: i64) -> i64\n  requires 0 <= x && x <= 10\n  ensures result == x + 1\n"
          "{ return crate::a::inc(x); }\n")


class Runner:
    def __init__(self, work, binary):
        self.work = Path(work) / "g4_lib"
        self.work.mkdir()
        self.cmd = [binary] if binary else [sys.executable, str(ROOT / "pruvc" / "pruvc.py")]
        self.n = 0

    def project(self, files):
        """files: {relative path: text}; returns the project directory"""
        self.n += 1
        d = self.work / ("p%d" % self.n)
        for rel, text in files.items():
            p = d / rel
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(text)
        return d

    def run(self, root, libs=(), flags=("--no-verify",), out=None, raw_libs=None):
        out = out or (root.parent / (root.name + ".rs"))
        args = list(self.cmd) + [str(root), "-o", str(out)] + list(flags)
        for d in (raw_libs if raw_libs is not None else [str(x) for x in libs]):
            args += ["--lib", d]
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


def main(work, binary=None):
    r = Runner(work, binary)
    fails = []

    def check(name, ok, detail=""):
        if not ok:
            fails.append("FAIL %s: %s" % (name, detail.strip()[-300:]))

    def one(diags, code):
        return len(diags) == 1 and diags[0].get("code") == code

    # 1. library-only module
    p = r.project({"proj/root.pruv": ROOT_A, "lib/a.pruv": INC})
    rc, ds, txt, _ = r.run(p / "proj" / "root.pruv", [p / "lib"])
    check("library-only module", rc == 0, txt)

    # 2. library child module (DIR/p/q.pruv)
    p = r.project({"proj/root.pruv": "mod p;\npub fn f(x: i64) -> i64 { return crate::p::g(x); }\n",
                   "lib/p.pruv": "mod q;\npub fn g(x: i64) -> i64 { return crate::p::q::h(x); }\n",
                   "lib/p/q.pruv": "pub fn h(x: i64) -> i64 { return x; }\n"})
    rc, ds, txt, _ = r.run(p / "proj" / "root.pruv", [p / "lib"])
    check("library child module", rc == 0, txt)
    # a child is never taken from the root directory's tree of a library module
    p = r.project({"proj/root.pruv": "mod p;\npub fn f(x: i64) -> i64 { return crate::p::g(x); }\n",
                   "lib/p.pruv": "mod q;\npub fn g(x: i64) -> i64 { return crate::p::q::h(x); }\n",
                   "proj/p/q.pruv": "pub fn h(x: i64) -> i64 { return x; }\n"})
    rc, ds, txt, _ = r.run(p / "proj" / "root.pruv", [p / "lib"])
    check("library child not read from root dir", rc == 1 and one(ds, "module_file")
          and ds[0].get("file") == "@lib/p.pruv", txt)

    # 3. two libraries (each module in a different one)
    p = r.project({"proj/root.pruv": "mod a;\nmod b;\npub fn f(x: i64) -> i64 { return crate::b::g(crate::a::inc(x)); }\n",
                   "l1/a.pruv": INC, "l2/b.pruv": "pub fn g(x: i64) -> i64 { return x; }\n"})
    rc, ds, txt, _ = r.run(p / "proj" / "root.pruv", [p / "l1", p / "l2"])
    check("two libraries", rc == 0, txt)

    # 4. local + library conflict
    p = r.project({"proj/root.pruv": ROOT_A, "proj/a.pruv": INC, "lib/a.pruv": INC})
    rc, ds, txt, _ = r.run(p / "proj" / "root.pruv", [p / "lib"])
    check("local+library conflict", rc == 1 and one(ds, "lib_module_conflict") and ds[0].get("stage") == "parse"
          and ds[0].get("message") == "module a is found in more than one place: a.pruv, @lib/a.pruv"
          and ds[0].get("fix") == "remove the duplicate file or drop one --lib", txt)
    check("conflict never shows DIR", str(p) not in txt, txt)

    # 5. two-library conflict
    p = r.project({"proj/root.pruv": ROOT_A, "l1/a.pruv": INC, "l2/a.pruv": INC})
    rc, ds, txt, _ = r.run(p / "proj" / "root.pruv", [p / "l1", p / "l2"])
    check("two-library conflict", rc == 1 and one(ds, "lib_module_conflict")
          and ds[0].get("message") == "module a is found in more than one place: @lib/a.pruv, @lib/a.pruv", txt)

    # 6. missing module hint (root-level and nested)
    p = r.project({"proj/root.pruv": ROOT_A, "lib/other.pruv": INC})
    rc, ds, txt, _ = r.run(p / "proj" / "root.pruv", [p / "lib"])
    check("missing module hint", rc == 1 and one(ds, "module_file") and ds[0].get("fix") ==
          "create a.pruv next to the root file, pass --lib DIR, or remove `mod a;`", txt)
    p = r.project({"proj/root.pruv": "mod p;\npub fn f(x: i64) -> i64 { return x; }\n", "proj/p.pruv": "mod q;\n"})
    rc, ds, txt, _ = r.run(p / "proj" / "root.pruv")
    check("nested missing hint unchanged", rc == 1 and one(ds, "module_file") and ds[0].get("fix") ==
          "create p/q.pruv next to the root file, or remove `mod q;`", txt)

    # 7. std inside a library is ignored (and is no conflict)
    p = r.project({"proj/root.pruv": "mod std;\npub fn f(x: i64) -> i64 { return crate::std::min_i64(x, 0); }\n",
                   "lib/std.pruv": "this is not Pruv\n"})
    rc, ds, txt, _ = r.run(p / "proj" / "root.pruv", [p / "lib"])
    check("std inside a library ignored", rc == 0, txt)

    # 8. a library module that fails to verify reports file = @lib/NAME.pruv
    p = r.project({"proj/root.pruv": "mod b;\npub fn f(x: i64) -> i64 { return crate::b::bad(x); }\n",
                   "lib/b.pruv": "pub fn bad(x: i64) -> i64\n  ensures result == x + 2\n{ return x + 1; }\n"})
    rc, ds, txt, _ = r.run(p / "proj" / "root.pruv", [p / "lib"], flags=())
    check("library verify error file", rc == 1 and ds and all(d.get("file") == "@lib/b.pruv" for d in ds)
          and str(p) not in txt, txt)

    # 9. legacy root ignores --lib
    p = r.project({"proj/root.pruv": "fn f(x: i64) -> i64 { return x; }\n", "lib/a.pruv": INC})
    rc, ds, txt, _ = r.run(p / "proj" / "root.pruv", [p / "lib"])
    check("legacy root ignores --lib", rc == 0, txt)

    # 10. bad --lib values: usage error, exit 2
    p = r.project({"proj/root.pruv": ROOT_A, "proj/a.pruv": INC, "file.txt": "x"})
    for label, raw in (("missing dir", [str(p / "nope")]), ("empty value", [""]), ("a file", [str(p / "file.txt")])):
        rc, ds, txt, _ = r.run(p / "proj" / "root.pruv", raw_libs=raw)
        check("bad --lib (%s)" % label, rc == 2 and "--lib needs an existing directory" in txt, "exit %d %s" % (rc, txt))
    # --lib is accepted before other flags too
    rc, ds, txt, _ = r.run(p / "proj" / "root.pruv", flags=("--lib", str(p), "--no-verify"))
    check("--lib before other flags", rc == 0, txt)

    # 11. generated Rust identical to the same project with the files copied next to the root
    files = {"mod_root.pruv": "mod a;\nmod p;\npub fn f(x: i64) -> i64\n  requires 0 <= x && x <= 10\n"
                              "{ return crate::p::g(crate::a::inc(x)); }\n"}
    pa = r.project({"proj/root.pruv": files["mod_root.pruv"], "lib/a.pruv": INC,
                    "lib/p.pruv": "mod q;\npub fn g(x: i64) -> i64 { return crate::p::q::h(x); }\n",
                    "lib/p/q.pruv": "pub fn h(x: i64) -> i64 { return x; }\n"})
    pb = r.project({"proj/root.pruv": files["mod_root.pruv"], "proj/a.pruv": INC,
                    "proj/p.pruv": "mod q;\npub fn g(x: i64) -> i64 { return crate::p::q::h(x); }\n",
                    "proj/p/q.pruv": "pub fn h(x: i64) -> i64 { return x; }\n"})
    rc1, _, t1, o1 = r.run(pa / "proj" / "root.pruv", [pa / "lib"], flags=())
    rc2, _, t2, o2 = r.run(pb / "proj" / "root.pruv", flags=())
    check("identical generated Rust", rc1 == 0 and rc2 == 0 and o1.read_text() == o2.read_text(), t1 + t2)

    for line in fails:
        print(line)
    if fails:
        return 1
    print("--lib: lookup, children, conflicts, hints, std, display path, legacy, CLI, identical Rust: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1], sys.argv[3] if len(sys.argv) == 4 and sys.argv[2] == "--rs" else None))
