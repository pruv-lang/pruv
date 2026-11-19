#!/usr/bin/env python3
"""Compile isolated M7 mutants and require their specific lifecycle checks to fail."""
import argparse
from pathlib import Path
import shutil
import subprocess
import tempfile

import run_m7
from lifecycle_controls import check_quiet, check_tombstones


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pruvc", default=run_m7.PRUVC_DEFAULT)
    args = ap.parse_args()
    Path(run_m7.WORK).mkdir(parents=True, exist_ok=True)
    source = Path(run_m7.BUD)
    query = "  let q: i64 = is_quiet(sk, ss, sn, rd(fx, 0));"
    plan = "    p = crate::bud_map::room_plan(st, 0, wd);"
    mutants = [
        ("unmutated", None, None, (check_quiet, check_tombstones)),
        ("effect-before-query", query, "  push(inout fx, 20);\n" + query, (check_quiet,)),
        ("wrong-key", query, query.replace("ss, sn", "ss, 0"), (check_quiet,)),
        ("rehash-before-query", plan, "    p = crate::bud_lifecycle::reserve(inout st, wd);", (check_tombstones,)),
    ]
    with tempfile.TemporaryDirectory(dir=run_m7.WORK, prefix="quiet-mutants-") as tmp:
        tmp = Path(tmp)
        budget = tmp / "budget"
        shutil.copytree(source, budget, ignore=shutil.ignore_patterns("work", "__pycache__"))
        lifecycle = budget / "bud_lifecycle.pruv"
        original = lifecycle.read_text()
        rlib = tmp / "libbud_facts.rlib"
        subprocess.run([run_m7.rustc(), "-O", "--edition", "2021", "--crate-type", "rlib", "--crate-name",
                        "bud_facts", str(budget / "m7_test/bud_facts.rs"), "-o", str(rlib)], check=True)
        gen = tmp / "gen.rs"
        main_rs = tmp / "main.rs"
        main_rs.write_text('#![allow(dead_code, unused_imports, unused_variables, unused_mut)]\n'
                           'mod gen;\nuse gen as sol;\n' + run_m7.driver_source(str(budget / "m7_test")))
        for name, old, new, checks in mutants:
            exe = tmp / name
            if old is not None:
                assert original.count(old) == 1, old
            lifecycle.write_text(original if old is None else original.replace(old, new, 1))
            subprocess.run([args.pruvc, str(budget / "m7_test/root.pruv"), "-o", str(gen),
                            "--lib", str(budget), "--lib", "bench/l3/lib",
                            "--rust-extern", f"bud_facts={rlib}", "--proven-entries"],
                           cwd=run_m7.ROOT, check=True)
            subprocess.run([run_m7.rustc(), "-O", "--edition", "2021", "--extern", f"bud_facts={rlib}",
                            "-o", str(exe), str(main_rs)], check=True)
            for check in checks:
                try:
                    check(exe)
                except AssertionError as err:
                    if old is None:
                        raise
                    print(f"PASS caught {name}: {err}")
                else:
                    if old is not None:
                        raise SystemExit(f"FAIL mutant survived: {name}")
            if old is None:
                print("PASS unmutated tree")


if __name__ == "__main__":
    main()
