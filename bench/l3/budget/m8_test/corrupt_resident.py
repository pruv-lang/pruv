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
  a  stored session key word corrupted: a non-zero tail lane (NON-CANONICAL stored key; op with the original key), and
     a different valid word (positive twin: the op with the matching key hits, plus the original key)
  b  stored budget-id word corrupted, same two kinds (the id words are record words 15, 16 of a 5-unit id)
  c  record state 3 / -2; reason 5 / -2 / 7 with state 1, reason 7 with state 0 and 2; unlimited 2
  d  stored word 14 (bn) = len+1, len-1, 0, len+4 (word count mismatch)
  e  stored turn-key / activity-key word (after noteTool) corrupted, same two kinds; noteTool and forgetTurn with the
     matching key (dedup hit) and with the original
  f  positive twins: no poke, and pokes that write the value already there
Every poke must print `Z <i>`; `Z -` is a control error.

Separate P21a expectations exercise a high entry top, a record overlapping the header, and reuse after lazy
forget. These check NEW against the expected status and observation instead of comparing with OLD.

The "different valid word" variants of the map keys (a-sk-*-valid, e-turn-valid, e-act-valid) use the driver `rekey`
line instead of `poke`: a bare poke changes the stored key words but leaves the stored hash (entry word 6) and the slot
table at the old key, so the "hit" op would miss and never reach the hit path. rekey checks that the stored hash equals
a Rust mirror of bud_keys::hash for the old words (`Z -` otherwise), writes the word, stores the new key's hash and
forces one grow, which rebuilds the slot table from word 6, so the matching key really hits. The b-bid variants change
record words, not map keys, and keep poke.

P05 meaning change: keys are packed-u16 words (4 lanes per i64 word, bud_keys.pruv), so the old "unit = 65536 / -1 /
70000" corruptions cannot be represented (every word is 4 valid lanes). The same store invariant ("every stored key
passed ingress") is now violated by a NON-CANONICAL tail lane, which no wire key can reproduce, so those cases run the
original key only; "different valid word" cases replace the old "same corrupt unit would hit" variants. Pokes anchor on
the packed words (an arena key is stored packed, a record id is its raw packed words). NOTE: the default --old-rev
(c4d3a63^) predates the packed wire and cannot run these programs; pass --old-rev with a revision that already has the
packed wire (and the checks under test removed or added) for the NEW vs OLD comparison to be meaningful.

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
        # driver_m8.rs keeps its state in the host's StateBuf (state_buf.rs), as run_m8.build writes it
        f.write('include!("%s");\n' % os.path.join(os.path.dirname(HERE), "state_buf.rs").replace("\\", "/"))
        f.write(open(os.path.join(HERE, "driver_m8.rs")).read())
    exe = os.path.join(d, "driver")
    run([run_m8.rustc(), "-C", "opt-level=3", "--edition", "2021", "-o", exe, main], text=True)
    return exe


# ---- controls ----
SK = [7001, 7002, 7003, 7004, 7005, 7006]  # session key units (6 units = 2 packed words, 2 tail lanes in the last)
BID = [8101, 8102, 8103, 8104, 8105]  # budget id (5 units = 2 packed words, 3 tail lanes in the last)
LIM = [777000001, 777000002, 777000003, 777000004]  # limits: record words 3..6, the poke anchor
TURN = [5551, 5552, 5553]
ACT = [6661, 6662]
ACT_N1 = [6663, 6664]
ACT_N2 = [6665, 6666, 6667]
WHAT = [0, 0, 2, 1, 0, 0, 0, 0, 2, 1]  # commands 1, retries 1


def kw(u):
    """Wire key `n, packed words(n)`."""
    return [len(u)] + run_m8.pack(list(u))


def line(tag, words):
    return tag + " " + " ".join(map(str, words))


def record(bid):
    return [0, 0, -1] + LIM + [0] * 4 + [0, 0, 0] + kw(bid)


def put_line(sk=SK, bid=BID):
    r = record(bid)
    return line("put", kw(sk) + [len(r)] + r)


def poke(pattern, delta, value):
    return line("poke", [len(pattern)] + list(pattern) + [delta, value])


def rekey(pattern, delta, value):
    """poke on a stored map-key word that also stores the new key's hash and rebuilds the slots (driver `rekey`)."""
    return line("rekey", [len(pattern)] + list(pattern) + [delta, value])


# word offsets from the first limit word (record word 3)
def rec_poke(word, value):
    return poke(LIM, word - 3, value)


def pk(u):
    return run_m8.pack(list(u))


ALT4 = [65535, 1, 0x8000, 2]  # four lanes with the top lane >= 0x8000: a negative packed word


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
    SKP, BP, TP, AP = pk(SK), pk(BID), pk(TURN), pk(ACT)
    # a) session-key words (arena key, packed): a different valid word (twin, the matching key hits) and non-zero tail
    #    lanes of the last word (non-canonical stored key: no wire key matches it, original key only)
    yield ("a-sk-w0-valid", [], [rekey(SKP, 0, pk(ALT4)[0])], [("hit", ALT4 + SK[4:], BID, TURN, ACT), orig])
    yield ("a-sk-w1-valid", [], [rekey(SKP, 1, pk([40000, 41000])[0])], [("hit", SK[:4] + [40000, 41000], BID, TURN, ACT), orig])
    yield ("a-sk-tail2", [], [poke(SKP, 1, SKP[1] | (1 << 16))], [orig])
    yield ("a-sk-tail3", [], [poke(SKP, 1, SKP[1] | 0xFFFF)], [orig])
    # b) budget-id words (record words 15, 16)
    yield ("b-bid-w0-valid", [], [rec_poke(15, pk(ALT4)[0])], [("hit", SK, ALT4 + BID[4:], TURN, ACT), orig])
    yield ("b-bid-w1-valid", [], [rec_poke(16, pk([9999])[0])], [("hit", SK, BID[:4] + [9999], TURN, ACT), orig])
    yield ("b-bid-tail1", [], [rec_poke(16, BP[1] | (1 << 32))], [orig])
    yield ("b-bid-tail3", [], [rec_poke(16, BP[1] | 0xFFFF)], [orig])
    # c) state / unlimited / reason
    for v in (3, -2):
        yield ("c-state%d" % v, [], [rec_poke(0, v)], [orig])
    for r in (5, -2, 7):
        yield ("c-st1-reason%d" % r, [], [rec_poke(0, 1), rec_poke(2, r)], [orig])
    for s in (0, 2):
        yield ("c-st%d-reason7" % s, [], [rec_poke(0, s), rec_poke(2, 7)], [orig])
    yield ("c-unl2", [], [rec_poke(1, 2)], [orig])
    # d) word 14 (bn); the record holds 2 id words. bn = len+1 / len-1 keep the word count (len+1 stays canonical
    #    for these words, len-1 does not); len+4 and 0 break the word count.
    for v in (len(BID) + 1, len(BID) - 1, 0, len(BID) + 4):
        yield ("d-w14-%d" % v, [], [rec_poke(14, v)], [orig])
    # e) turn / activity key words after noteTool (TURN: 3 units, tail lane 3; ACT: 2 units, tail lanes 2, 3)
    yield ("e-turn-valid", SETUP, [rekey(TP, 0, pk([65535, 5552, 5553])[0])], [("hit", SK, BID, [65535, 5552, 5553], ACT), orig])
    yield ("e-turn-tail3", SETUP, [poke(TP, 0, TP[0] | 0xFFFF)], [orig])
    yield ("e-act-valid", SETUP, [rekey(AP, 0, pk([9999, 1])[0])], [("hit", SK, BID, TURN, [9999, 1]), orig])
    yield ("e-act-tail2", SETUP, [poke(AP, 0, AP[0] | (1 << 16))], [orig])
    yield ("e-act-tail3", SETUP, [poke(AP, 0, AP[0] | 0xFFFF)], [orig])
    # f) positive twins: no poke, and pokes writing the value already there
    yield ("f-none", [], [], [orig])
    yield ("f-none-tools", SETUP, [], [orig])
    yield ("f-same-state", [], [rec_poke(0, 0)], [orig])
    yield ("f-same-reason", [], [rec_poke(2, -1)], [orig])
    yield ("f-same-unl", [], [rec_poke(1, 0)], [orig])
    yield ("f-same-w14", [], [rec_poke(14, len(BID))], [orig])
    for j in range(len(SKP)):
        yield ("f-same-sk-w%d" % j, [], [poke(SKP, j, SKP[j])], [orig])
    for j in range(len(BP)):
        yield ("f-same-bid-w%d" % j, [], [rec_poke(15 + j, BP[j])], [orig])
    yield ("f-same-turn-w0", SETUP, [poke(TP, 0, TP[0])], [orig])
    yield ("f-same-act-w0", SETUP, [poke(AP, 0, AP[0])], [orig])


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


def run_cases(exe, program):
    r = run([exe, "q"], input=program, text=True)
    return split_cases(r.stdout)


def p21a_controls(exe):
    """Check NEW outcomes independently of the revision used for the a-f differential."""
    hand = run_m8.build_hand("hand_m8_u16")
    second_session = put_line([7007])
    two_sessions = [put_line(), second_session]
    # Two sessions force the 256-word initial store to 512 words (16 slots, 8 entries, top 5).
    # Anchoring on all nine header words checks that the fixed offsets used by both pokes are correct.
    header = [1112884307, 1, 512, 16, 16, 32, 8, 5, -1]
    first_session_blob_off = header[5] + 20 * 3 + 12
    add = SETUP[0]
    # Use an empty budget ID so header word 15 (pending-Need, 0) matches the record's ID length at offset 1.
    overlap = [put_line(bid=[]), second_session, poke(header, first_session_blob_off, 1)]
    overlap_call = line("call", [3] + kw(SK) + kw([]) + kw(TURN) + kw(ACT))
    # Fill all eight entries before forgetting. Reinsert the same keys, including a duplicate, to expose stale hits.
    extra = line("call", [3] + kw(SK) + kw(BID) + kw(TURN) + kw(ACT_N1))
    reuse = [put_line(), *SETUP, extra, line("call", [4] + kw(TURN)), *SETUP, extra, add]

    def program(cases):
        return "".join("\n".join(["case " + name, *body, "end", ""]) for name, body in cases.items())

    got = run_cases(exe, program({
        "a-high-top": [*two_sessions, poke(header, 7, header[6] - 1), add],
        "b-header-blob": [*overlap, overlap_call],
        "b-before-call": overlap,
        "c-reuse": reuse,
    }))
    # The hand driver has no resident header. The clean equivalent gives the expected call log and state.
    r = run([hand], input=program({"a-high-top": [*two_sessions, add], "c-reuse": reuse}), text=True)
    ref = split_cases(r.stdout)
    before = got["b-before-call"]
    expected = {
        "a-high-top": ["Z 0", *ref["a-high-top"]],
        "b-header-blob": ["Z 0", "X 2", *before[1:]],
        "c-reuse": ref["c-reuse"],
    }
    failed = 0
    for name, want in expected.items():
        ok = got[name] == want
        if name == "b-header-blob":
            ok = ok and len(before) == 3 and before[0] == "Z 0" and before[1].startswith("O ") and before[1] != "O X 2"
        print("P21a %s: %s" % (name, "pass" if ok else "FAIL"))
        if not ok:
            print("  expected", want)
            print("  got     ", got[name])
            failed += 1
    # Header views (driver `hdr`: buffer length, then header words 4..14) separate a reclaim from a Grow, which
    # the logical observation above cannot: the driver retries Grow internally and `O` serializes reachable state.
    hdr = "hdr"
    reuse_g = [put_line(), *SETUP, extra, hdr, line("call", [4] + kw(TURN)), hdr, *SETUP, extra, hdr]
    g = run_cases(exe, program({
        "a-high-top": [*two_sessions, hdr, poke(header, 7, header[6] - 1), hdr, add, hdr],
        "b-header-blob": [*overlap, hdr, overlap_call, hdr],
        "c-reuse": reuse_g,
    }))

    def views(name):
        return [list(map(int, l.split()[1:])) for l in g[name] if l.startswith("G ") and l != "G -"]

    def word(v, i):  # header word i (4 <= i <= 14) of a view
        return v[1 + i - 4]

    checks = {}
    v = views("a-high-top")
    # the poke raises only word 7 (to cap - 1); the insert then needs two entries and must Grow, not fail
    checks["a-high-top grows"] = (len(v) == 3 and v[1][0] == v[0][0] and word(v[1], 7) == header[6] - 1 and
                                 [x for i, x in enumerate(v[1]) if i != 1 + 7 - 4] ==
                                 [x for i, x in enumerate(v[0]) if i != 1 + 7 - 4] and v[2][0] > v[1][0])
    v = views("b-header-blob")
    checks["b-header-blob header words 4..14 unchanged"] = len(v) == 2 and v[0] == v[1]
    v = views("c-reuse")
    # before forget, after forget, after reinserting the same keys: no Grow, no new entries past the top, and the
    # live entries (12) and live arena words (14) return to their values before the forget
    checks["c-reuse reclaims"] = (len(v) == 3 and v[0][0] == v[1][0] == v[2][0] and
                                  word(v[2], 7) == word(v[0], 7) and word(v[1], 12) < word(v[0], 12) and
                                  word(v[2], 12) == word(v[0], 12) and word(v[2], 14) == word(v[0], 14))
    for name, ok in checks.items():
        print("P21a %s: %s" % (name, "pass" if ok else "FAIL"))
        if not ok:
            print("  views", views(name.split()[0]))
            failed += 1
    total = len(expected) + len(checks)
    print("P21a expectations %d/%d" % (total - failed, total))
    return failed


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pruvc", default=run_m8.PRUVC_DEFAULT)
    ap.add_argument("--old-rev", default="c4d3a63^")
    ap.add_argument("--new-rev", help="build NEW from this git revision instead of the working tree")
    a = ap.parse_args()
    new_exe = build("new", a.pruvc, a.new_rev)
    old_exe = build("old", a.pruvc, a.old_rev)
    prog, pokes = build_program()
    outs = {"new": run_cases(new_exe, prog), "old": run_cases(old_exe, prog)}
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
    failed = p21a_controls(new_exe)
    return 1 if mism or notfound or failed else 0


if __name__ == "__main__":
    sys.exit(main())
