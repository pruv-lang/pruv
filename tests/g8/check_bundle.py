"""G8 A0 resource/surface checks, invoked only by run_all's g8_frontend group.

    python tests/g8/check_bundle.py WORK [--rs PATH] [--vs-py]

Default: checks the Rust compiler (--rs, default pruvc-rs/target/release/pruvc). The std bundle is embedded in that
binary, so the bundle-resource cases run against patched COPIES of the binary (same technique as check_proof.py).
Nothing from pruvc/ is imported or run.

PF: since bundle PF the Python compiler (pruvc/) is frozen. --vs-py is an opt-in extra that ALSO runs the old checks on
the frozen Python distribution (never used by CI); the Python modules are imported only inside that code path.
`canonical` stays importable at module level because check_proof.py uses it.
"""
import argparse
import dataclasses
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_RS = ROOT / "pruvc-rs" / "target" / "release" / ("pruvc.exe" if os.name == "nt" else "pruvc")


def canonical(value):
    if dataclasses.is_dataclass(value):
        return (type(value).__name__, {f.name: canonical(getattr(value, f.name))
                for f in dataclasses.fields(value) if f.name not in ("line", "col", "body")})
    if isinstance(value, (list, tuple)):
        return [canonical(v) for v in value]
    return value


def spec_headers():
    """The approved public headers of std, from docs/G8_SPEC.md section 4."""
    listing = (ROOT / "docs/G8_SPEC.md").read_text().split("## 4.")[1].split("```pruv\n")[1].split("```")[0]
    return listing.strip().split("\n\n")


# ---------------------------------------------------------------- Rust compiler (default)

def rs_main(work, binary):
    source_text = (ROOT / "stdlib/std.pruv").read_text()
    headers = spec_headers()
    # Text-level surface check (no parser): 12 public functions, headers exactly as approved, no escape hatches.
    pubs = re.findall(r"^pub fn (\w+)", source_text, re.M)
    assert len(pubs) == len(headers) == 12, (len(pubs), len(headers))
    assert not re.search(r"^(?!pub fn )[ \t]*fn\b", source_text, re.M), "non-public function in std"
    assert not re.search(r"^[ \t]*mod\b", source_text, re.M), "std declares modules"
    header_names = []
    for h in headers:
        m = re.match(r"pub fn (\w+)\s*\(", h)
        assert m, h
        header_names.append(m.group(1))
        assert h + "\n{" in source_text, "header text changed: " + m.group(1)
        assert not re.match(r"pub fn \w+\s*<", h), "type parameters in " + m.group(1)
        assert not re.search(r"\b(cost|decreases)\b", h), "cost/decreases in " + m.group(1)
    assert header_names == pubs, "public function order changed"
    assert not re.search(r"\b(trusted|rust|crate|mod)\b", source_text)

    binary = Path(binary).resolve()
    work = Path(work) / "g8_resources_rs"
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True)
    source = (ROOT / "stdlib/std.pruv").read_bytes()
    image = binary.read_bytes()
    assert image.count(source) == 1, "embedded bundle not found exactly once in " + str(binary)

    def variant(name, text):
        assert len(text) == len(source) and text != source  # same length keeps every offset valid
        path = work / name
        path.write_bytes(image.replace(source, text))
        path.chmod(0o755)
        if sys.platform == "darwin":
            subprocess.run(["codesign", "-f", "-s", "-", str(path)], check=True, capture_output=True)
        return path

    def padded(text):
        assert len(text) <= len(source)
        return text.ljust(len(source), b" ")

    app = work / "app"
    app.mkdir()
    root = app / "root.pruv"
    root.write_text("mod std;\npub fn f() -> i64 { return 1; }\n")
    cwd = work / "unrelated"
    cwd.mkdir()
    out = work / "out"

    def clean():
        for p in (out, out.with_suffix(".rs"), out.with_suffix(".ll")):
            if p.exists():
                p.unlink()

    def compile_with(exe, src=None, backend="rust", flags=()):
        clean()
        return subprocess.run([str(exe), str(src or root), "-o", str(out), "--backend", backend, *flags],
                              cwd=cwd, text=True, capture_output=True, timeout=300)

    def diagnostics(c):
        return [json.loads(line) for line in (c.stdout + c.stderr).splitlines() if line.startswith("{")]

    def expect(exe, stage, code, file, backends=("rust", "llvm")):
        for backend in backends:
            c = compile_with(exe, backend=backend)
            ds = diagnostics(c)
            assert c.returncode == 1 and len(ds) == 1, (backend, c.stdout, c.stderr)
            d = ds[0]
            assert (d["stage"], d["code"], d.get("file")) == (stage, code, file), d
            assert str(work) not in c.stdout, "absolute path leaked into diagnostics"
            assert not out.exists()

    for backend in ("rust", "llvm"):
        c = compile_with(binary, backend=backend)
        assert c.returncode == 0 and out.exists(), c.stdout + c.stderr
    # Module order: the bundle and ordinary modules coexist in any position.
    root.write_text("mod child;\nmod std;\nmod after;\n")
    (app / "child.pruv").write_text("pub fn f() -> () {}")
    (app / "after.pruv").write_text("pub fn f() -> () {}")
    c = compile_with(binary)
    assert c.returncode == 0, c.stdout + c.stderr
    root.write_text("mod std;\npub fn f() -> i64 { return 1; }\n")

    # Unreadable bundle: an embedded resource can be absent only as invalid UTF-8 (patched copy).
    bad_utf8 = variant("pruvc_utf8", b"\xff" + source[1:])
    expect(bad_utf8, "parse", "std_source", "root.pruv")

    # A local file/directory/dangling link named std.pruv conflicts with the bundled module.
    for kind in ("file", "directory", "dangling"):
        local = app / "std.pruv"
        if kind == "file":
            local.write_text("invalid")
        elif kind == "directory":
            local.mkdir()
        else:
            local.symlink_to("missing")
        expect(binary, "parse", "std_module_conflict", "root.pruv")
        local.rmdir() if kind == "directory" else local.unlink()
    root.write_text("mod child;\nmod std;\n")
    (app / "child.pruv").write_text("mod std;\n")
    expect(binary, "parse", "std_import_location", "child.pruv")
    root.write_text("mod std;\n")

    # A broken bundle is reported with the bundle label, not as std_source.
    broken_parse = variant("pruvc_parse", padded(b"pub fn broken("))
    c = compile_with(broken_parse)
    ds = diagnostics(c)
    assert c.returncode == 1 and len(ds) == 1, c.stdout + c.stderr
    assert ds[0]["stage"] == "parse" and ds[0]["code"] != "std_source", ds
    assert ds[0]["file"] == "@std/std.pruv", ds
    broken_type = variant("pruvc_type", padded(b"pub fn broken() -> i64 { return true; }"))
    c = compile_with(broken_type)
    ds = diagnostics(c)
    assert c.returncode == 1 and len(ds) == 1, c.stdout + c.stderr
    assert ds[0]["stage"] == "type" and ds[0]["file"] == "@std/std.pruv", ds

    # A broken bundle must not affect programs that do not import it (legacy and ordinary explicit modules).
    for program in ("fn f() -> i64 { return 1; }", "pub fn f() -> i64 { return 1; }"):
        root.write_text(program)
        for backend in ("rust", "llvm"):
            c = compile_with(broken_type, backend=backend, flags=("--no-verify",))
            assert c.returncode == 0, c.stdout + c.stderr

    # Frozen pre-G8 outputs (tests/g8/compatibility.json) cover legacy, explicit modules, and diagnostics.
    samples = {
        "legacy": "fn f(x: i64) -> i64 { return x; }",
        "modules": "mod child;\npub fn f(x: i64) -> i64 { return crate::child::f(x); }",
        "diagnostic": "fn f() -> i64 { return true; }",
    }
    (app / "child.pruv").write_text("pub fn f(x: i64) -> i64 { return x; }")
    expected = json.loads((ROOT / "tests/g8/compatibility.json").read_text())
    for name, program in samples.items():
        root.write_text(program)
        for backend in ("rust", "llvm"):
            for mode, flags in (("default", ()), ("safe", ("--no-verify",))):
                key = "/".join((name, backend, mode))
                bindings = work / "bindings.rs"
                if bindings.exists():
                    bindings.unlink()
                extra = ("--rust-bindings", str(bindings)) if backend == "llvm" else ()
                c = compile_with(binary, backend=backend, flags=(*flags, *extra))
                assert c.returncode == (1 if name == "diagnostic" else 0), c.stdout + c.stderr
                got = {"exit": c.returncode, "stdout": c.stdout, "stderr": c.stderr,
                       "output": hashlib.sha256(out.read_bytes()).hexdigest() if out.exists() else None,
                       "bindings": hashlib.sha256(bindings.read_bytes()).hexdigest() if bindings.exists() else None}
                assert got == expected[key], (key, got, expected[key])
    print("rs: 12 text headers; relocated cwd; invalid-UTF-8/conflicting/broken bundles and labels; compatibility: OK")


# ---------------------------------------------------------------- frozen Python distribution (--vs-py only)

def py_main(work):
    sys.path.insert(0, str(ROOT / "pruvc"))
    from vparser import parse
    from modules import load_program

    source = (ROOT / "stdlib/std.pruv").read_text()
    headers = spec_headers()
    approved = parse("\n\n".join(h + "\n{}" for h in headers))
    actual = parse(source)
    assert len(actual.functions) == len(actual.items) == 12 and not actual.mods
    assert canonical(actual) == canonical(approved), "parsed public headers changed"
    for f, h in zip(actual.functions, headers):
        assert f.pub and f.body
        assert not f.tparams and f.cost is None and f.decreases is None
        assert h + "\n{" in source, "header text changed"
    assert not re.search(r"\b(trusted|rust|crate|mod)\b", source)

    work = Path(work) / "g8_resources"
    work.mkdir()
    package = work / "distribution"
    shutil.copytree(ROOT / "pruvc", package / "pruvc", ignore=shutil.ignore_patterns("__pycache__"))
    (package / "stdlib").mkdir()
    bundle = package / "stdlib/std.pruv"
    bundle.write_text(source)
    app = work / "app"
    app.mkdir()
    root = app / "root.pruv"
    root.write_text("mod std;\npub fn f() -> i64 { return 1; }\n")
    cwd = work / "unrelated"
    cwd.mkdir()

    def compile_with(compiler, src=root, backend="rust", flags=()):
        return subprocess.run([sys.executable, str(compiler), str(src), "-o", str(work / "out"),
                               "--backend", backend, *flags], cwd=cwd, text=True,
                              capture_output=True, timeout=60)

    def expect(stage, code, file, message=None):
        for backend in (("llvm",) if stage == "codegen" else ("rust", "llvm")):
            c = compile_with(package / "pruvc/pruvc.py", backend=backend)
            ds = [json.loads(line) for line in c.stdout.splitlines() if line.startswith("{")]
            assert c.returncode == 1 and len(ds) == 1, c.stdout + c.stderr
            d = ds[0]
            assert (d["stage"], d["code"], d.get("file")) == (stage, code, file), d
            if message:
                assert d["message"] == message, d
            assert str(package) not in c.stdout
            assert not (work / "out").exists()

    c = compile_with(package / "pruvc/pruvc.py")
    assert c.returncode == 0 and (work / "out").exists(), c.stdout + c.stderr
    (work / "out").unlink()
    c = compile_with(package / "pruvc/pruvc.py", backend="llvm")
    assert c.returncode == 0 and (work / "out").exists(), c.stdout + c.stderr
    (work / "out").unlink()
    root.write_text("mod child;\nmod std;\nmod after;\n")
    (app / "child.pruv").write_text("pub fn f() -> () {}")
    (app / "after.pruv").write_text("pub fn f() -> () {}")
    loaded = load_program(str(root))
    assert [m.path for m in loaded.modules] == [(), ("child",), ("std",), ("after",)]
    assert loaded.modules[2].file == "@std/std.pruv"
    assert all(f.file == "@std/std.pruv" for f in loaded.modules[2].items)
    root.write_text("mod std;\npub fn f() -> i64 { return 1; }\n")
    bundle.unlink()
    expect("parse", "std_source", "root.pruv")
    bundle.mkdir()  # read failure, independent of permission bits/root privileges
    expect("parse", "std_source", "root.pruv")
    bundle.rmdir()
    bundle.write_bytes(b"\xff")
    expect("parse", "std_source", "root.pruv")
    for kind in ("file", "directory", "dangling"):
        local = app / "std.pruv"
        if kind == "file":
            local.write_text("invalid")
        elif kind == "directory":
            local.mkdir()
        else:
            local.symlink_to("missing")
        expect("parse", "std_module_conflict", "root.pruv")
        local.rmdir() if kind == "directory" else local.unlink()
    root.write_text("mod child;\nmod std;\n")
    (app / "child.pruv").write_text("mod std;\n")
    expect("parse", "std_import_location", "child.pruv")
    root.write_text("mod std;\n")
    bundle.write_text("pub fn broken(")
    c = compile_with(package / "pruvc/pruvc.py")
    d = json.loads(c.stdout.strip())
    assert c.returncode == 1 and d["stage"] == "parse" and d["code"] != "std_source"
    assert d["file"] == "@std/std.pruv", d
    bundle.write_text("pub fn broken() -> i64 { return true; }")
    c = compile_with(package / "pruvc/pruvc.py")
    d = json.loads(c.stdout.strip())
    assert c.returncode == 1 and d["stage"] == "type" and d["file"] == "@std/std.pruv", d
    # Missing std must not affect legacy or ordinary explicit modules.
    bundle.unlink()
    for program in ("fn f() -> i64 { return 1; }", "pub fn f() -> i64 { return 1; }"):
        root.write_text(program)
        for backend in ("rust", "llvm"):
            c = compile_with(package / "pruvc/pruvc.py", backend=backend, flags=("--no-verify",))
            assert c.returncode == 0, c.stdout + c.stderr
            (work / "out").unlink()
    # Frozen pre-G8 outputs cover legacy, explicit modules, and diagnostics.
    compatibility = ROOT / "tests/g8/compatibility.json"
    samples = {
        "legacy": "fn f(x: i64) -> i64 { return x; }",
        "modules": "mod child;\npub fn f(x: i64) -> i64 { return crate::child::f(x); }",
        "diagnostic": "fn f() -> i64 { return true; }",
    }
    (app / "child.pruv").write_text("pub fn f(x: i64) -> i64 { return x; }")
    expected = json.loads(compatibility.read_text())
    for name, program in samples.items():
        root.write_text(program)
        for backend in ("rust", "llvm"):
            for mode, flags in (("default", ()), ("safe", ("--no-verify",))):
                key = "/".join((name, backend, mode))

                def artifacts(compiler):
                    bindings = work / "bindings.rs"
                    output = work / "out"
                    for path in (bindings, output):
                        if path.exists():
                            path.unlink()
                    extra = ("--rust-bindings", str(bindings)) if backend == "llvm" else ()
                    c = compile_with(compiler, backend=backend, flags=flags + extra)
                    assert c.returncode == (1 if name == "diagnostic" else 0), c.stdout + c.stderr
                    return {"exit": c.returncode, "stdout": c.stdout, "stderr": c.stderr,
                            "output": hashlib.sha256(output.read_bytes()).hexdigest() if output.exists() else None,
                            "bindings": hashlib.sha256(bindings.read_bytes()).hexdigest() if bindings.exists() else None}

                got = artifacts(package / "pruvc/pruvc.py")
                assert got == expected[key], (key, got, expected[key])
    print("py: 12 parsed/text headers; relocated cwd; missing/unreadable/UTF-8 resources; conflicts and labels: OK")


def main(argv):
    ap = argparse.ArgumentParser(description="G8 bundle/resource checks on the Rust compiler")
    ap.add_argument("work", help="existing scratch directory")
    ap.add_argument("--rs", default=str(DEFAULT_RS), metavar="PATH", help="Rust compiler binary (default: release build)")
    ap.add_argument("--vs-py", action="store_true",
                    help="opt-in, not CI: also run the old checks on the frozen Python distribution")
    try:
        args = ap.parse_args(argv)
    except SystemExit as e:
        return 2 if e.code else 0
    if not os.path.isfile(args.rs):
        print("Rust compiler not found: %s" % args.rs, file=sys.stderr)
        return 2
    rs_main(args.work, args.rs)
    if args.vs_py:
        py_main(args.work)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
