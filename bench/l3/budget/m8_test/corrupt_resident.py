#!/usr/bin/env python3
"""M8 control: t_call on a CORRUPT resident state buffer, NEW Pruv vs OLD Pruv (pre-c4d3a63), byte for byte.

Commit c4d3a63 dropped the key-unit checks on hit paths, the state / reason rechecks and the word-14 compare in
load_cv, assuming a store invariant (every stored key / record passed t_put) that is not checked. This control builds
resident states that violate it (driver_m8 `poke` lines flip single words of the buffer after valid puts / noteTools)
and diffs the q-mode output of two builds on the same program text:

  NEW  the working tree (or --new-rev REV: the same files extracted from a git revision);
  OLD  `git show <old-rev>:<path>` of every .pruv file under bench/l3/budget and bench/l3/lib (default c4d3a63^).

Both are compiled with pruvc (cwd = the tree root, same --lib paths as run_m8.build) and linked with the CURRENT
working-tree driver_m8.rs (it has the poke line). Builds go to work/m8/corrupt/{new,old}/ (work/m8/gen.rs and
work/m8/driver_m8 are not touched).

Controls (deterministic, no rng), each a q-mode case: put one session, optional noteTool setup, poke(s), then a call
sequence (charge, noteFailure, noteTool new turn / same turn new activity / duplicate / existing activity, forgetTurn,
charge, end) as ONE case, and every call alone as its own case (a rejected call stops a case, so the alone cases keep the
later calls covered):
  a  stored session-key unit = 65536 / -1 / 70000; op with the same corrupt unit (would hit) and with the original key
  b  stored budget-id unit = 65536 / -1; op budget key with the same value, and the original
  c  record state 3 / -2; reason 5 / -2 / 7 with state 1, reason 7 with state 0 and 2; unlimited 2
  d  stored word 14 = n-15+1, n-15-1, 0 (non-empty id)
  e  stored turn-key / activity-key unit (after noteTool) = 65536 / -1; noteTool and forgetTurn with the same corrupt
     key (dedup hit) and with the original
  f  positive twins: no poke, and pokes that write the value already there
Every poke must print `Z <i>`; `Z -` is a control error.

  python3 bench/l3/budget/m8_test/corrupt_resident.py [--pruvc PATH] [--old-rev REV] [--new-rev REV]
Prints: controls N, mismatches M, poke-not-found P, how many controls bite on the OLD side, and per mismatch the case
name with both outputs (first 20 lines). Exit 1 on any mismatch or poke-not-found.
"""
import argparse
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import run_m8  # noqa: E402

ROOT = run_m8.ROOT
OUT = os.path.join(run_m8.WORK, "corrupt")
ROOT_PRUV = "bench/l3/budget/m8_test/root_m8.pruv"
DIRS = ["bench/l3/budget", "bench/l3/lib"]


def run(cmd, **kw):
    r = subprocess.run(cmd, capture_output=True, **kw)
    if r.returncode != 0:
        err = r.stderr if isinstance(r.stderr, str) else r.stderr.decode("utf-8", "replace")
        sys.exit("%s failed:\n%s" % (" ".join(cmd[:4]), err[-3000:]))
    return r


def extract(rev, dest):
    """Writes every .pruv file under the repo dirs at git revision rev into dest, mirroring repo paths."""
    names = run(["git", "-C", ROOT, "ls-tree", "-r", "--name-only", rev, "--"] + DIRS, text=True).stdout.split("\n")
    names = [n for n in names if n.endswith(".pruv")]
    if ROOT_PRUV not in names:
        sys.exit("%s has no %s" % (rev, ROOT_PRUV))
    for n in names:
        data = run(["git", "-C", ROOT, "show", "%s:%s" % (rev, n)]).stdout
        p = os.path.join(dest, n)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "wb") as f:
            f.write(data)
    return len(names)


def build(label, pruvc, rev):
    """Builds the q driver for label ('new' / 'old'): rev None = working tree, else that git revision."""
    d = os.path.join(OUT, label)
    os.makedirs(d, exist_ok=True)
    if rev is None:
        cwd = ROOT
    else:
        cwd = os.path.join(OUT, label + "_src")
        extract(rev, cwd)
    gen = os.path.join(d, "gen.rs")
    run([pruvc, ROOT_PRUV, "-o", gen] + run_m8.LIBS, cwd=cwd, text=True)
    main = os.path.join(d, "main.rs")
    with open(main, "w") as f:
        f.write('#![allow(dead_code, unused_imports, unused_variables, unused_mut)]\n#[path = "gen.rs"] mod sol;\n')
        f.write(open(os.path.join(HERE, "driver_m8.rs")).read())
    exe = os.path.join(d, "driver")
    run([run_m8.rustc(), "-C", "opt-level=3", "--edition", "2021", "-o", exe, main], text=True)
    return exe


# ---- controls ----
SK = [7001, 7002, 7003, 7004]  # session key units (occur nowhere else in the buffer)
BID = [8101, 8102, 8103, 8104, 8105]  # budget id
LIM = [777000001, 777000002, 777000003, 777000004]  # limits: record words 3..6, the poke anchor
TURN = [5551, 5552, 5553]
ACT = [6661, 6662]
ACT_N1 = [6663, 6664]
ACT_N2 = [6665, 6666, 6667]
WHAT = [0, 0, 2, 1, 0, 0, 0, 0, 2, 1]  # commands 1, retries 1


def kw(u):
    return [len(u)] + list(u)


def line(tag, words):
    return tag + " " + " ".join(map(str, words))


def record():
    return [0, 0, -1] + LIM + [0] * 4 + [0, 0, 0] + kw(BID)


def put_line():
    r = record()
    return line("put", kw(SK) + [len(r)] + r)


def poke(pattern, delta, value):
    return line("poke", [len(pattern)] + list(pattern) + [delta, value])


# word offsets from the first limit word (record word 3)
def rec_poke(word, value):
    return poke(LIM, word - 3, value)


def with_unit(u, j, v):
    u = list(u)
    u[j] = v
    return u


def ops_of(sk, bk, t, a):
    """Named ops for one key variant: session sk, budget bk, turn t, activity a (existing activity)."""
    return [
        ("charge1", [1] + kw(sk) + kw(bk) + WHAT),
        ("fail", [2] + kw(sk) + kw(bk)),
        ("toolN1", [3] + kw(sk) + kw(bk) + kw(t) + kw(ACT_N1)),
        ("toolN2", [3] + kw(sk) + kw(bk) + kw(t) + kw(ACT_N2)),
        ("toolN2dup", [3] + kw(sk) + kw(bk) + kw(t) + kw(ACT_N2)),
        ("toolA", [3] + kw(sk) + kw(bk) + kw(t) + kw(a)),
        ("forget", [4] + kw(t)),
        ("charge2", [1] + kw(sk) + kw(bk) + WHAT),
    ]


SEQ = ["charge1", "fail", "toolN1", "toolN2", "toolN2dup", "toolA", "forget", "charge2"]
SETUP = [line("call", [3] + kw(SK) + kw(BID) + kw(TURN) + kw(ACT)),
         line("call", [3] + kw(SK) + kw(BID) + kw(TURN) + kw([6668, 6669]))]


def scenarios():
    """Yields (name, setup lines, poke lines, variants); variant = (label, sk, bk, t, a)."""
    orig = ("orig", SK, BID, TURN, ACT)
    # a) session-key unit
    for v in (65536, -1, 70000):
        for j in range(len(SK)):
            sk = with_unit(SK, j, v)
            yield ("a-sk%d-u%d" % (v, j), [], [poke(SK, j, v)], [("hit", sk, BID, TURN, ACT), orig])
    # b) budget-id unit
    for v in (65536, -1):
        for j in range(len(BID)):
            bk = with_unit(BID, j, v)
            yield ("b-bid%d-u%d" % (v, j), [], [rec_poke(15 + j, v)], [("hit", SK, bk, TURN, ACT), orig])
    # c) state / unlimited / reason
    for v in (3, -2):
        yield ("c-state%d" % v, [], [rec_poke(0, v)], [orig])
    for r in (5, -2, 7):
        yield ("c-st1-reason%d" % r, [], [rec_poke(0, 1), rec_poke(2, r)], [orig])
    for s in (0, 2):
        yield ("c-st%d-reason7" % s, [], [rec_poke(0, s), rec_poke(2, 7)], [orig])
    yield ("c-unl2", [], [rec_poke(1, 2)], [orig])
    # d) word 14
    n = 15 + len(BID)
    for v in (n - 15 + 1, n - 15 - 1, 0):
        yield ("d-w14-%d" % v, [], [rec_poke(14, v)], [orig])
    # e) turn / activity key units after noteTool
    for v in (65536, -1):
        for j in range(len(TURN)):
            t = with_unit(TURN, j, v)
            yield ("e-turn%d-u%d" % (v, j), SETUP, [poke(TURN, j, v)], [("hit", SK, BID, t, ACT), orig])
        for j in range(len(ACT)):
            a = with_unit(ACT, j, v)
            yield ("e-act%d-u%d" % (v, j), SETUP, [poke(ACT, j, v)], [("hit", SK, BID, TURN, a), orig])
    # f) positive twins: no poke, and pokes writing the value already there
    yield ("f-none", [], [], [orig])
    yield ("f-none-tools", SETUP, [], [orig])
    yield ("f-same-state", [], [rec_poke(0, 0)], [orig])
    yield ("f-same-reason", [], [rec_poke(2, -1)], [orig])
    yield ("f-same-unl", [], [rec_poke(1, 0)], [orig])
    yield ("f-same-w14", [], [rec_poke(14, len(BID))], [orig])
    for j in range(len(SK)):
        yield ("f-same-sk-u%d" % j, [], [poke(SK, j, SK[j])], [orig])
    for j in range(len(BID)):
        yield ("f-same-bid-u%d" % j, [], [rec_poke(15 + j, BID[j])], [orig])
    yield ("f-same-turn-u0", SETUP, [poke(TURN, 0, TURN[0])], [orig])
    yield ("f-same-act-u0", SETUP, [poke(ACT, 0, ACT[0])], [orig])


def build_program():
    """Returns (program text, {case name: number of poke lines})."""
    out, pokes = [], {}

    def case(name, setup, pk, calls):
        out.append("case " + name)
        out.append(put_line())
        out.extend(setup)
        out.extend(pk)
        for c in calls:
            out.append(line("call", c))
        out.append("end")
        pokes[name] = len(pk)

    for name, setup, pk, variants in scenarios():
        for label, sk, bk, t, a in variants:
            ops = ops_of(sk, bk, t, a)
            d = dict(ops)
            base = "%s-%s" % (name, label)
            case(base + "-seq", setup, pk, [d[k] for k in SEQ])
            for k in SEQ:
                case("%s-%s" % (base, k), setup, pk, [d[k]])
    return "\n".join(out) + "\n", pokes


def split_cases(text):
    res, cur = {}, None
    for l in text.splitlines():
        if l.startswith("case "):
            cur = res.setdefault(l[5:], [])
        elif cur is not None:
            cur.append(l)
    return res


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pruvc", default=run_m8.PRUVC_DEFAULT)
    ap.add_argument("--old-rev", default="c4d3a63^")
    ap.add_argument("--new-rev", help="build NEW from this git revision instead of the working tree")
    a = ap.parse_args()
    new_exe = build("new", a.pruvc, a.new_rev)
    old_exe = build("old", a.pruvc, a.old_rev)
    prog, pokes = build_program()
    outs = {}
    for label, exe in (("new", new_exe), ("old", old_exe)):
        r = subprocess.run([exe, "q"], input=prog, capture_output=True, text=True)
        if r.returncode != 0:
            sys.exit("%s driver failed:\n%s" % (label, r.stderr[-3000:]))
        outs[label] = split_cases(r.stdout)
    mism, notfound, bite_x2, bite_any = [], [], 0, 0
    for name in pokes:
        o, n = outs["old"].get(name), outs["new"].get(name)
        if o is None or n is None:
            sys.exit("case %s missing in an output" % name)
        z = [l for l in o if l.startswith("Z ")] + [l for l in n if l.startswith("Z ")]
        if any(l == "Z -" for l in z) or len([l for l in o if l.startswith("Z ")]) != pokes[name]:
            notfound.append(name)
        if o != n:
            mism.append(name)
        if "X 2" in o:
            bite_x2 += 1
        if any(l.startswith(("X ", "H ")) or (l.startswith("R ") and l not in ("R 0", "R -")) for l in o):
            bite_any += 1
    print("old rev %s, new %s" % (a.old_rev, a.new_rev or "working tree"))
    print("controls %d, mismatches %d, poke-not-found %d" % (len(pokes), len(mism), len(notfound)))
    print("OLD side: %d controls with `X 2`, %d with any X / H / non-`R 0` charge outcome (the controls bite)"
          % (bite_x2, bite_any))
    for name in notfound[:20]:
        print("POKE NOT FOUND", name)
    for name in mism:
        print("MISMATCH", name)
        print("  new:")
        for l in outs["new"][name][:20]:
            print("    " + l)
        print("  old:")
        for l in outs["old"][name][:20]:
            print("    " + l)
    return 1 if mism or notfound else 0


if __name__ == "__main__":
    sys.exit(main())
