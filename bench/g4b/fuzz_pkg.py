"""G4b-Q: fuzz manifests and package graphs against both compilers (docs/G4B_SPEC.md sections 3-5).

Each case builds a random package graph (DAG, diamond, chain, aliases) in one workspace directory, optionally plants
one fault (cycle, identity conflict, unsatisfied version, unused dependency) and compiles the program with the Python
and the Rust compiler.  Oracle: (1) both compilers agree on exit code, stdout and generated Rust; (2) both match the
planted expectation.

usage: python3 bench/g4b/fuzz_pkg.py [--seed N] [--count N] [--rs BIN] [--work DIR] [--jobs N] [--keep-failures]
"""
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import argparse
import collections
import json
import random
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
TIMEOUT = 120
BASE = 1000000          # every package requires 0 <= x <= BASE - gain, so any call chain stays in range
KINDS = ("cycle", "identity", "version", "unused")
EXPECTED = {"cycle": "dependency_cycle", "identity": "identity_conflict",
            "version": "version_unsatisfied", "unused": "unused_dependency", "clean": "ok"}
VERSIONS = [(0, 1, 0), (0, 1, 5), (0, 2, 3), (1, 0, 0), (1, 3, 2), (2, 0, 0)]


# ---- version requirements -------------------------------------------------------------------------------------------

def sat_req(v, rnd):
    """a requirement string satisfied by version v (same compatibility line, at least)"""
    maj, mi, pa = v
    if maj >= 1:
        opts = ["%d" % maj, "%d.%d" % (maj, mi), "%d.%d.%d" % (maj, mi, pa), "%d.0" % maj]
        if mi > 0:
            opts.append("%d.%d" % (maj, mi - 1))
    else:
        opts = ["0.%d" % mi, "0.%d.%d" % (mi, pa), "0.%d.0" % mi]
    return rnd.choice(opts)


def unsat_req(v, rnd):
    """a requirement string NOT satisfied by version v (other line, or newer than found)"""
    maj, mi, pa = v
    if maj >= 1:
        opts = ["%d" % (maj + 1), "%d.%d" % (maj, mi + 1), "%d.%d.%d" % (maj, mi, pa + 1)]
        opts.append("%d" % (maj - 1) if maj >= 2 else "0.5")
    else:
        opts = ["0.%d" % (mi + 1), "0.%d.%d" % (mi, pa + 1), "1"]
        if mi > 0:
            opts.append("0.%d" % (mi - 1))
    return rnd.choice(opts)


# ---- generator ------------------------------------------------------------------------------------------------------

class Edge:
    def __init__(self, src, dst, key, req, mount=True, call=True):
        self.src, self.dst, self.key, self.req, self.mount, self.call = src, dst, key, req, mount, call


class Node:
    def __init__(self, name, dirname, version, c):
        self.name, self.dir, self.version, self.c = name, dirname, version, c


def reach_sets(n, edges):
    """reach[i] = set of nodes reachable from i through at least one edge (any edge kind)"""
    adj = collections.defaultdict(set)
    for e in edges:
        adj[e.src].add(e.dst)
    reach = []
    for i in range(n):
        seen, stack = set(), list(adj[i])
        while stack:
            x = stack.pop()
            if x not in seen:
                seen.add(x)
                stack.extend(adj[x])
        reach.append(seen)
    return reach


def gen_graph(rnd):
    n = rnd.randint(2, 7)
    shape = rnd.choice(["dag", "diamond", "alias", "chain"])
    if shape == "diamond" and n < 4:
        shape = "dag"
    nodes = [Node("app", "app", (0, 1, 0), rnd.randint(1, 3))]
    for i in range(1, n):
        nodes.append(Node("pk%d" % i, "pk%d" % i, rnd.choice(VERSIONS), rnd.randint(1, 3)))
    pairs = set()                                   # forward edges only (i < j), so the graph is acyclic
    if shape == "chain":
        pairs |= {(i - 1, i) for i in range(1, n)}
    else:
        if shape == "diamond":
            pairs |= {(0, 1), (0, 2), (1, 3), (2, 3)}
        for j in range(1, n):
            if not any(d == j for _, d in pairs):   # every package is reachable from the program
                pairs.add((rnd.randrange(0, j), j))
        p_extra = 0.35 if shape != "chain" else 0.0
        for i in range(n):
            for j in range(i + 1, n):
                if rnd.random() < p_extra:
                    pairs.add((i, j))
    alias_p = 1.0 if shape == "alias" else 0.2
    edges = []
    for (i, j) in sorted(pairs):
        key = ("x" + nodes[j].name) if rnd.random() < alias_p else nodes[j].name
        edges.append(Edge(i, j, key, sat_req(nodes[j].version, rnd)))
    return shape, nodes, edges


def plant(kind, rnd, nodes, edges):
    """plant the fault in place; False when the graph has no suitable place"""
    n = len(nodes)
    if kind == "cycle":
        reach = reach_sets(n, edges)
        cand = [(i, j) for i in range(1, n) for j in range(i + 1, n) if j in reach[i]]
        if not cand:
            return False
        i, j = rnd.choice(cand)
        key = ("x" + nodes[i].name) if rnd.random() < 0.3 else nodes[i].name
        edges.append(Edge(j, i, key, sat_req(nodes[i].version, rnd), mount=True, call=False))
        return True
    if kind == "identity":
        reach = reach_sets(n, edges)
        cand = []
        for d in range(1, n):
            for p in range(n):
                if p == d or p in reach[d]:
                    continue
                if any(e.src == p and e.dst == d for e in edges):
                    continue
                cand.append((p, d))
        if not cand:
            return False
        p, d = rnd.choice(cand)
        orig = nodes[d]
        clone = Node(orig.name, orig.dir + "_2", orig.version, orig.c)
        nodes.append(clone)
        ci = len(nodes) - 1
        for e in [e for e in edges if e.src == d]:      # the clone depends on the same directories
            edges.append(Edge(ci, e.dst, e.key, e.req, e.mount, e.call))
        edges.append(Edge(p, ci, "z" + orig.name, sat_req(orig.version, rnd)))
        return True
    if kind == "version":
        if not edges:
            return False
        e = rnd.choice(edges)
        e.req = unsat_req(nodes[e.dst].version, rnd)
        return True
    if kind == "unused":
        if not edges:
            return False
        e = rnd.choice(edges)
        e.mount = False
        e.call = False
        return True
    raise ValueError(kind)


def gains(nodes, edges):
    """gain[i] = own increment plus the gains of the dependencies it calls (call edges form a DAG)"""
    memo = {}

    def g(i):
        if i not in memo:
            memo[i] = nodes[i].c + sum(g(e.dst) for e in edges if e.src == i and e.call)
        return memo[i]
    return [g(i) for i in range(len(nodes))]


def render_files(nodes, edges):
    """{relative path: text} of the whole workspace"""
    gain = gains(nodes, edges)
    files = {}
    for i, nd in enumerate(nodes):
        mine = sorted((e for e in edges if e.src == i), key=lambda e: e.key)
        deps = {e.key: {"path": "../" + nodes[e.dst].dir, "version": e.req} for e in mine}
        man = {"name": nd.name, "version": "%d.%d.%d" % nd.version, "root": nd.name + ".pruv"}
        if deps:
            man["dependencies"] = deps
        files[nd.dir + "/pruv.json"] = json.dumps(man) + "\n"
        text = "".join("mod %s;\n" % e.key for e in mine if e.mount)
        expr = "x"
        for e in mine:
            if e.call:
                expr = "crate::%s::f_%s(%s)" % (e.key, nodes[e.dst].name, expr)
        text += ("pub fn f_%s(x: i64) -> i64\n  requires 0 <= x && x <= %d\n  ensures result == x + %d\n"
                 "{ return %s + %d; }\n") % (nd.name, BASE - gain[i], gain[i], expr, nd.c)
        files[nd.dir + "/" + nd.name + ".pruv"] = text
    return files


def gen_case(seed, i):
    rnd = random.Random(seed * 100000 + i)
    shape, nodes, edges = gen_graph(rnd)
    kind = "clean"
    if rnd.random() >= 0.4:
        order = list(KINDS)
        rnd.shuffle(order)
        for k in order:
            if plant(k, rnd, nodes, edges):
                kind = k
                break
    return {"shape": shape, "kind": kind, "expected": EXPECTED[kind], "files": render_files(nodes, edges),
            "nodes": len(nodes), "edges": len(edges)}


# ---- running --------------------------------------------------------------------------------------------------------

class Timeout(Exception):
    pass


def compile_one(cmd, case_dir, out_dir):
    """returns (exit code, normalized stdout, generated bytes or None, diagnostics)"""
    out_dir.mkdir(parents=True, exist_ok=True)
    out = out_dir / "out.rs"
    args = list(cmd) + [str(case_dir / "app" / "app.pruv"), "--manifest", str(case_dir / "app" / "pruv.json"),
                        "-o", str(out)]
    try:
        c = subprocess.run(args, capture_output=True, text=True, timeout=TIMEOUT)
    except subprocess.TimeoutExpired:
        raise Timeout()
    stdout = c.stdout.replace(str(out), "<OUT>").replace(str(case_dir), "<CASE>")
    diags = []
    for line in c.stdout.splitlines():
        try:
            d = json.loads(line)
        except ValueError:
            continue
        if isinstance(d, dict) and "code" in d:
            diags.append(d)
    data = out.read_bytes() if c.returncode == 0 and out.exists() else None
    return c.returncode, stdout, data, diags


def code_of(d):
    return str(d.get("code", "")).split("/")[-1]


def run_case(args, i):
    case = gen_case(args.seed, i)
    case_dir = args.work / ("case_%d" % i)
    if case_dir.exists():
        shutil.rmtree(case_dir)
    for rel, text in case["files"].items():
        p = case_dir / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text)
    res = {"i": i, "kind": case["kind"], "shape": case["shape"], "expected": case["expected"],
           "agree": True, "oracle": True, "reasons": []}
    try:
        py = compile_one([sys.executable, str(ROOT / "pruvc" / "pruvc.py")], case_dir, case_dir / "out_py")
        rs = compile_one([str(args.rs)], case_dir, case_dir / "out_rs")
    except Timeout:
        res["agree"] = res["oracle"] = False
        res["reasons"].append("timeout")
        return finish(args, case_dir, res)
    if py[0] != rs[0]:
        res["agree"] = False
        res["reasons"].append("exit code differs: py=%d rs=%d" % (py[0], rs[0]))
    if py[1] != rs[1]:
        res["agree"] = False
        res["reasons"].append("stdout differs")
    if py[0] == 0 and rs[0] == 0 and py[2] != rs[2]:
        res["agree"] = False
        res["reasons"].append("generated .rs differs")
    for label, r in (("py", py), ("rs", rs)):
        rc, _, _, diags = r
        if case["expected"] == "ok":
            if rc != 0:
                res["oracle"] = False
                res["reasons"].append("%s: expected ok, got exit %d: %s" % (label, rc, [code_of(d) for d in diags]))
        elif rc != 1 or len(diags) != 1 or code_of(diags[0]) != case["expected"]:
            res["oracle"] = False
            res["reasons"].append("%s: expected %s, got exit %d with %s" % (label, case["expected"], rc,
                                                                          [code_of(d) for d in diags]))
    return finish(args, case_dir, res)


def finish(args, case_dir, res):
    failed = not (res["agree"] and res["oracle"])
    if not (failed and args.keep_failures):
        shutil.rmtree(case_dir, ignore_errors=True)
    return res


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--count", type=int, default=200)
    ap.add_argument("--rs", default=None)
    ap.add_argument("--work", default=None)
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--keep-failures", action="store_true")
    args = ap.parse_args()
    args.rs = Path(args.rs).resolve() if args.rs else ROOT / "pruvc-rs" / "target" / "release" / "pruvc"
    args.work = Path(args.work).resolve() if args.work else ROOT / "bench" / "g4b" / ".work" / "fuzz"
    if not args.rs.exists():
        print("Rust compiler not found: %s" % args.rs)
        return 2
    args.work.mkdir(parents=True, exist_ok=True)

    with ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
        results = list(pool.map(lambda i: run_case(args, i), range(args.count)))

    per_kind = {}
    for r in results:
        k = per_kind.setdefault(r["kind"], {"cases": 0, "disagree": 0, "oracle_fail": 0})
        k["cases"] += 1
        k["disagree"] += 0 if r["agree"] else 1
        k["oracle_fail"] += 0 if r["oracle"] else 1
    failures = [r for r in results if not (r["agree"] and r["oracle"])]
    summary = {"seed": args.seed, "count": args.count, "per_kind": per_kind,
               "agree": sum(1 for r in results if r["agree"]), "disagree": sum(1 for r in results if not r["agree"]),
               "oracle_pass": sum(1 for r in results if r["oracle"]),
               "oracle_fail": sum(1 for r in results if not r["oracle"]),
               "failures": [{"case": r["i"], "kind": r["kind"], "shape": r["shape"], "reasons": r["reasons"]}
                            for r in failures]}
    (args.work / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")

    print("cases: %d (seed %d)" % (args.count, args.seed))
    for k in sorted(per_kind):
        v = per_kind[k]
        print("  %-9s cases %-4d py/rs disagree %-3d oracle fail %d" % (k, v["cases"], v["disagree"], v["oracle_fail"]))
    print("py/rs agree: %d, disagree: %d" % (summary["agree"], summary["disagree"]))
    print("oracle pass: %d, fail: %d" % (summary["oracle_pass"], summary["oracle_fail"]))
    for r in failures[:10]:
        print("FAIL case_%d (%s, %s): %s" % (r["i"], r["kind"], r["shape"], "; ".join(r["reasons"])))
    if len(failures) > 10:
        print("... %d more in %s" % (len(failures) - 10, args.work / "summary.json"))
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
