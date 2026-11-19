"""Boundary tests for the shared codegen constants, run on the selected compiler.

The numbers below are hard-coded on purpose (never read from pruvc/data/*.json), so a wrong
JSON value makes this test fail.

    INLINE_SMALL=8, INLINE_BUDGET=1500   (inline_policy.json)
    MAX_TYPE_APPS=4096, MAX_TYPE_NODES=65536   (mono_limits.json)

Usage: python tests/check_codegen_boundaries.py WORK (--py|--rs) PATH
"""
from pathlib import Path
import json
import re
import shutil
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import compiler_driver as CD  # noqa: E402

INLINE_SMALL = 8
INLINE_BUDGET = 1500
MAX_TYPE_APPS = 4096
MAX_TYPE_NODES = 65536


def inline_source(lines, sites):
    """probe has a body of exactly `lines` Rust lines and is called from `sites` call sites."""
    lets = "\n".join(f"  let v{i}: i64 = x;" for i in range(lines - 1))
    calls = "\n".join(f"  let r{i}: i64 = probe(0);" for i in range(sites))
    return (f"fn probe(x: i64) -> i64 {{\n{lets}\n  return x;\n}}\n"
            f"fn caller() -> i64 {{\n{calls}\n  return 0;\n}}\n")


def apps_source(n):
    """n distinct applications of a 13-parameter generic struct (bool/i64 per bit of the index)."""
    params = ", ".join(f"T{b}" for b in range(13))
    parts = [f"struct Tag<{params}> {{ id: i64 }}"]
    for start in range(0, n, 64):
        parts.append(f"fn block{start}() -> i64 {{")
        for i in range(start, min(start + 64, n)):
            args = ", ".join("bool" if (i >> b) & 1 else "i64" for b in range(13))
            parts.append(f"  let v{i}: Tag<{args}> = Tag::<{args}> {{ id: 0 }};")
        parts.append("  return 0;\n}")
    return "\n".join(parts)


def nodes_source(wrappers):
    """d15's T is 65535 nodes; each Option wrapper adds one node."""
    parts = ["struct P<A, B> { a: A, b: B }", "fn leaf<T>() -> i64 { return 0; }"]
    arg = "Option<" * wrappers + "T" + ">" * wrappers
    parts.append(f"fn d15<T>() -> i64 {{ return leaf::<{arg}>(); }}")
    for i in reversed(range(15)):
        parts.append(f"fn d{i}<T>() -> i64 {{ return d{i+1}::<P<T,T>>(); }}")
    parts.append("fn root() -> i64 { return d0::<i64>(); }")
    return "\n".join(parts)


def compile_case(drv, work, name, source):
    d = work / "codegen_boundaries" / name
    if d.exists():
        shutil.rmtree(d)
    d.mkdir(parents=True, exist_ok=True)
    src, out = d / "src.pruv", d / "out.rs"
    src.write_text(source)
    res = drv.run(drv.argv(src, out, ["--no-verify"], "rust"), timeout=600)
    return res, out


def diagnostics(res):
    diags = []
    for line in res.stdout.splitlines():
        try:
            d = json.loads(line)
        except ValueError:
            continue
        if isinstance(d, dict):
            diags.append(d)
    return diags


def check_inline(drv, work, name, lines, sites, expect):
    res, out = compile_case(drv, work, name, inline_source(lines, sites))
    assert res.returncode == 0, f"exit {res.returncode} (expected 0): {res.stdout}{res.stderr}"
    text = out.read_text()
    found = re.findall(r"^(#\[inline(?:\(always\))?\])\n(?:pub )?fn probe\(", text, re.M)
    assert len(found) == 1, f"expected exactly one attribute before probe, got {found}"
    assert found[0] == expect, f"expected {expect}, got {found[0]}"


def check_apps_ok(drv, work, name):
    res, out = compile_case(drv, work, name, apps_source(MAX_TYPE_APPS))
    assert res.returncode == 0, f"exit {res.returncode} (expected 0): {res.stdout[-600:]}{res.stderr[-600:]}"
    n = len(re.findall(r"^(?:pub )?struct pruv_g3_t_", out.read_text(), re.M))
    assert n == MAX_TYPE_APPS, f"expected {MAX_TYPE_APPS} specialized struct definitions, got {n}"


def check_limit_reject(drv, work, name, source, fragment):
    res, _ = compile_case(drv, work, name, source)
    assert res.returncode == 1, f"exit {res.returncode} (expected 1): {res.stdout[-600:]}{res.stderr[-600:]}"
    diags = diagnostics(res)
    good = [d for d in diags if d.get("stage") == "codegen" and d.get("code") == "specialization_limit"
            and fragment in str(d.get("message", ""))]
    assert good, (f"expected stage=codegen code=specialization_limit message containing {fragment!r}; "
                  f"got {[(d.get('stage'), d.get('code'), str(d.get('message'))[:120]) for d in diags]}")


def check_nodes_ok(drv, work, name):
    res, out = compile_case(drv, work, name, nodes_source(1))
    assert res.returncode == 0, f"exit {res.returncode} (expected 0): {res.stdout[-600:]}{res.stderr[-600:]}"
    assert out.exists(), "out.rs not written"


def main(work, kind, path):
    work = Path(work)
    drv = CD.make_driver(kind, path)
    cases = [
        ("inline_small_8", lambda n: check_inline(drv, work, n, INLINE_SMALL, 189, "#[inline(always)]")),
        ("inline_small_9", lambda n: check_inline(drv, work, n, INLINE_SMALL + 1, 189, "#[inline]")),
        ("inline_budget_1500", lambda n: check_inline(drv, work, n, INLINE_BUDGET, 2, "#[inline(always)]")),
        ("inline_budget_1501", lambda n: check_inline(drv, work, n, INLINE_BUDGET + 1, 2, "#[inline]")),
        ("type_apps_4096", lambda n: check_apps_ok(drv, work, n)),
        ("type_apps_4097", lambda n: check_limit_reject(
            drv, work, n, apps_source(MAX_TYPE_APPS + 1),
            f"more than {MAX_TYPE_APPS} distinct generic type applications")),
        ("type_nodes_65536", lambda n: check_nodes_ok(drv, work, n)),
        ("type_nodes_65537", lambda n: check_limit_reject(
            drv, work, n, nodes_source(2), f"more than {MAX_TYPE_NODES} nodes")),
    ]
    failures = []
    for name, fn in cases:
        try:
            fn(name)
            print(f"{name}: OK")
        except AssertionError as e:
            print(f"{name}: FAIL {e}")
            failures.append(name)
    print(f"{len(cases) - len(failures)}/{len(cases)} boundary cases OK")
    return 1 if failures else 0


if __name__ == "__main__":
    if len(sys.argv) != 4 or sys.argv[2] not in ("--py", "--rs"):
        sys.exit("usage: check_codegen_boundaries.py WORK (--py|--rs) PATH")
    sys.exit(main(sys.argv[1], sys.argv[2][2:], sys.argv[3]))
