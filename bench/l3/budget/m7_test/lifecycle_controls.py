"""Raw-state lifecycle controls shared by the M7 and M10 drivers."""
from diff_hand_m7 import BID, SK, Prog, put_line, quiet_line, rec, run_exe, split_case, kw


def logs(exe, lines):
    rc, stdout, stderr = run_exe(exe, "\n".join(lines) + "\n")
    if rc:
        raise RuntimeError(f"driver exited {rc}: {stderr}")
    return split_case(stdout)[0]


def check_quiet(exe):
    from diff_hand_m7 import controls
    _, lines, expected = next(c for c in controls() if c[0] == "maybeClose two session keys")
    got = [line for line in logs(exe, lines) if line.startswith(("@ ", "E quiet "))]
    want = [line for line in expected if line.startswith(("@ ", "E quiet "))]
    assert got == want, f"quiet log {got!r} != {want!r}"
    print("PASS quiet keys and effect count")


def check_attach(exe):
    bad = rec(BID)[:30]
    bad[24] = 24
    p = Prog("attach-overrun").add(put_line(SK, bad)).add(put_line([116], rec([99])))
    p.add("dump").add("audit").call(3, SK, origin=0).add("dump")
    before, req, attempt, status, after = logs(exe, p.end())
    assert before.startswith("S ") and before == after, "attach changed raw state"
    assert (req, attempt, status) == ("@ r0", "A 2 0 true true true -99 []", "X 2")
    print("PASS attach id overrun preserves both records and emits no ref")


def rewrite_record():
    return rec(BID, since=("t", [97]))


def check_tombstones(exe):
    # An opaque blob forces word-arena growth even when budget text lives in the byte arena.
    p = Prog("tombstones").add(put_line([120], [7] + [0] * 180))
    p.add("del " + " ".join(map(str, kw([120]))))
    p.add(put_line(SK, rewrite_record())).add("tombstones").add("quiet 2 0 1").add("dump")
    p.call(6, SK).add("dump").call(6, SK).add("dump").call(6, SK).call(1, SK, BID)
    got = logs(exe, p.end())
    assert got[0].startswith("S ") and got[0] == got[4] == got[8], "quiet rejection changed raw state"
    assert got[1:4] == ["@ r0", quiet_line(2, SK), "X 2"]
    assert got[5:8] == ["@ r1", quiet_line(0, SK), "V 1"]
    assert got[9:13] == ["@ r2", quiet_line(1, SK), "E changed", "V 2"]
    assert got[13] == "@ r3" and got[14].split()[0:2] == ["V", "4"]
    assert got[14].split()[3] == "2", "budget did not close"
    print("PASS tombstone rewrite preserves raw state on answers 2 and 0 and closes on 1")


def check_grow(exe, m10=False):
    for answer in (0, 1, 2):
        p = Prog("close-grow").add(put_line(SK, rewrite_record()))
        p.add("fill_arena").add("audit").add(f"quiet {answer}")
        if m10:
            p.call_words([6] + [0] * 11 + kw(SK) + [0, 0])
        else:
            p.call(6, SK)
        got = logs(exe, p.end())
        first = got[1].split()
        assert first[0] == "A" and int(first[1]) < 0 and first[2:] == [
            "0", "true", "true", "true", "-99", "[]"], got
        retry = {
            0: ["A 0 1 true true false 1 []", quiet_line(0, SK), "V 1"],
            1: ["A 0 1 false false false 2 [20]", quiet_line(1, SK), "E changed", "V 2"],
            2: ["A 2 1 true true true -99 []", quiet_line(2, SK), "X 2"],
        }
        assert got[2:] == retry[answer], got
        print(f"PASS M{10 if m10 else 7} Grow {first[1]} answer {answer}, queries 0 then 1")
