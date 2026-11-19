#!/usr/bin/env python3
"""Unit tests for tools/health.py (standard library only, pure functions only).

Covers: parse_headroom, parse_time, compare, last_log_row, rs_branches / strip_rs / py_branches, row.
Nothing here runs git, gh, du, the network or the --full path.

Run:  python3 tests/health_tool.py        (exit code 0 when every test passes)
"""
import os
import sys
import tempfile
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "tools"))
import health  # noqa: E402


# ---- parse_headroom ---------------------------------------------------------------------------------------------

HEADROOM = """# PERF_HEADROOM

## Summary (latest)

| item | Pruv | hand Rust | TS | Pruv/hand | cause | status |
|---|---|---|---|---|---|---|
| fast | 1 | 1 | 1 | 0.69 / 0.75 / 0.74 | none | ok |
| slow | 1 | 1 | 1 | 1.22 / 1.13 / 1.14 (pass 3; was 1.40 / 1.30) | alloc | open |
| matched | 1 | 1 | 1 | P05b matched: typed 1.02-1.04, wire 0.54-0.55 | none | watch |
| equal | 1 | 1 | 1 | 1.00 / 0.99 | none | ok |

## Other

| item | Pruv | hand Rust | TS | Pruv/hand | cause | status |
|---|---|---|---|---|---|---|
| ignored | 1 | 1 | 1 | 3.0 | none | ok |
"""


class ParseHeadroomTest(unittest.TestCase):
    def test_listed_items(self):
        got = health.parse_headroom(HEADROOM)
        self.assertEqual([(i, r, s) for i, r, s in got],
                         [("slow", 1.22, "open"), ("matched", 1.04, "watch")])

    def test_parenthesised_history_ignored(self):
        got = dict((i, r) for i, r, _ in health.parse_headroom(HEADROOM))
        self.assertEqual(got["slow"], 1.22)

    def test_exactly_one_not_listed(self):
        names = [i for i, _, _ in health.parse_headroom(HEADROOM)]
        self.assertNotIn("equal", names)
        self.assertNotIn("fast", names)

    def test_later_section_ignored(self):
        names = [i for i, _, _ in health.parse_headroom(HEADROOM)]
        self.assertNotIn("ignored", names)

    def test_no_summary_heading(self):
        self.assertEqual(health.parse_headroom("# T\n\n| item | Pruv/hand |\n|---|---|\n| a | 2.0 |\n"), [])


# ---- parse_time -------------------------------------------------------------------------------------------------

MAC = "        6.16 real         5.90 user         0.20 sys\n 127926272  maximum resident set size\n"
GNU_WALL = "Elapsed (wall clock) time (h:mm:ss or m:ss): %s\n"
GNU_RSS = "Maximum resident set size (kbytes): 124928\n"


class ParseTimeTest(unittest.TestCase):
    def test_macos(self):
        self.assertEqual(health.parse_time(MAC), (6.16, 127926272))

    def test_gnu_rss_in_kib(self):
        self.assertEqual(health.parse_time(GNU_RSS)[1], 124928 * 1024)

    def test_gnu_simple_elapsed(self):
        # Elapsed label without the "(h:mm:ss or m:ss)" part.
        self.assertEqual(health.parse_time("Elapsed (wall clock) time: 0:06.16\n" + GNU_RSS), (6.16, 124928 * 1024))
        self.assertEqual(health.parse_time("Elapsed (wall clock) time: 1:02:03.5\n")[0], 3723.5)

    # The real GNU label carries "(h:mm:ss or m:ss)" before the colon.
    def test_gnu_real_label_minutes(self):
        self.assertEqual(health.parse_time(GNU_WALL % "0:06.16" + GNU_RSS), (6.16, 124928 * 1024))

    def test_gnu_real_label_hours(self):
        self.assertEqual(health.parse_time(GNU_WALL % "1:02:03.5")[0], 3723.5)

    def test_empty(self):
        self.assertEqual(health.parse_time(""), (None, None))


# ---- compare ----------------------------------------------------------------------------------------------------

def make_full(pruvc_s=8.0, pruv_rss=100, ns=(1.0, 1.0, 1.0), hand_ns=2.0):
    """A full-mode block with identical numbers on every axis unless overridden."""
    m4 = {}
    for axis, n in zip(health.AXES, ns):
        m4[axis] = {"pruv": {"ns": n, "rss": pruv_rss, "allocs": 5, "ns_range": [n, n]},
                    "hand": {"ns": hand_ns, "rss": 100, "allocs": 5, "ns_range": [hand_ns, hand_ns]}}
    return {"pruvc_s": pruvc_s, "rustc_s": 1.0, "binary_bytes": 1000, "pruvc_rss": 100, "rustc_rss": 100, "m4": m4}


def make_m(**kw):
    m = {"lines": {"pruvc": 100, "pruvc-rs": 100}, "above_hand": [], "ci": [], "disk_free": 100 * 10 ** 9,
         "backed_up": True}
    m.update(kw)
    return m


LAST = {"commit": "old1234", "lines": {"pruvc": 100, "pruvc-rs": 100}}


class CompareTest(unittest.TestCase):
    def joined(self, m, last):
        return "\n".join(health.compare(m, last))

    def test_quiet_baseline(self):
        self.assertEqual(health.compare(make_m(), LAST), [])
        self.assertEqual(health.compare(make_m(), None), [])

    def test_lines_jump(self):
        out = health.compare(make_m(lines={"pruvc": 1101, "pruvc-rs": 100}), LAST)
        self.assertEqual(len(out), 1)
        self.assertIn("lines: pruvc +1,001", out[0])
        out = health.compare(make_m(lines={"pruvc": 100, "pruvc-rs": 1101}), LAST)
        self.assertIn("lines: pruvc-rs +1,001", out[0])

    def test_lines_exactly_threshold_not_flagged(self):
        self.assertEqual(health.compare(make_m(lines={"pruvc": 1100, "pruvc-rs": 1100}), LAST), [])

    def test_above_hand_listed_without_new_marker(self):
        m = make_m(above_hand=[("slow", 1.22, "open")])
        for last in (None, LAST, {"above_hand": [("slow", 1.3, "x")]}):
            out = health.compare(m, last)
            self.assertEqual(len(out), 1)
            self.assertIn("perf: slow 1.22x hand", out[0])
            self.assertNotIn("(new)", out[0])

    def test_above_hand_new_marker(self):
        m = make_m(above_hand=[("slow", 1.22, "open"), ("old", 1.1, "x")])
        out = health.compare(m, {"above_hand": [("old", 1.2, "x")]})
        self.assertEqual(len(out), 2)
        self.assertIn("(new)", out[0])
        self.assertNotIn("(new)", out[1])
        # empty previous list still counts as "has above_hand"
        self.assertIn("(new)", health.compare(m, {"above_hand": []})[0])

    def test_proof_time_growth(self):
        last = dict(LAST, full=make_full(pruvc_s=8.0))
        self.assertIn("m4 proof time", self.joined(make_m(full=make_full(pruvc_s=10.0)), last))
        self.assertNotIn("m4 proof time", self.joined(make_m(full=make_full(pruvc_s=9.5)), last))

    def test_m4_rss_growth(self):
        last = dict(LAST, full=make_full())
        f = make_full()
        f["m4"]["sessions"]["pruv"]["rss"] = 121
        out = self.joined(make_m(full=f), last)
        self.assertIn("m4 sessions peak RSS", out)
        self.assertNotIn("m4 tools", out)
        f["m4"]["sessions"]["pruv"]["rss"] = 110
        self.assertEqual(health.compare(make_m(full=f), last), [])

    def test_m4_slower_than_hand(self):
        out = health.compare(make_m(full=make_full(ns=(1.0, 3.0, 1.0), hand_ns=2.0)), None)
        self.assertEqual(len(out), 1)
        self.assertIn("perf: M4 tools", out[0])
        self.assertEqual(health.compare(make_m(full=make_full(ns=(2.0, 1.0, 1.0), hand_ns=2.0)), None), [])

    def test_ci(self):
        out = health.compare(make_m(ci=[(1, "failure", "abc1234")]), None)
        self.assertEqual(out, ["ci: abc1234 failure"])
        ok = [(1, "success", "a"), (2, "cancelled", "b"), (3, "in_progress", "c")]
        self.assertEqual(health.compare(make_m(ci=ok), None), [])
        self.assertEqual(health.compare(make_m(ci=None), None), [])

    def test_backup(self):
        self.assertIn("work/: changed after the last backup", self.joined(make_m(backed_up=False), None))
        self.assertEqual(health.compare(make_m(backed_up=None), None), [])

    def test_disk(self):
        self.assertIn("disk:", self.joined(make_m(disk_free=19 * 10 ** 9), None))
        self.assertEqual(health.compare(make_m(disk_free=20 * 10 ** 9), None), [])


# ---- last_log_row -----------------------------------------------------------------------------------------------

class LastLogRowTest(unittest.TestCase):
    HEAD = "# Health log\n\n| date | commit | pruvc lines | pruvc-rs lines | notes |\n|---|---|---|---|---|\n"

    def write(self, body):
        fd, path = tempfile.mkstemp(suffix=".md")
        self.addCleanup(os.remove, path)
        with os.fdopen(fd, "w", encoding="utf-8") as fh:
            fh.write(self.HEAD + body)
        return path

    def test_last_row(self):
        path = self.write("| 2026-11-18 | aaa1111 | 23,000 (0) | 49,000 (+10) | x |\n"
                          "| 2026-11-19 | bbb2222 | 23,357 (0) | 49,551 (+31) | y |\n")
        self.assertEqual(health.last_log_row(path),
                         {"commit": "bbb2222", "lines": {"pruvc": 23357, "pruvc-rs": 49551}})

    def test_dash_row_falls_back(self):
        path = self.write("| 2026-11-18 | aaa1111 | 23,000 (0) | 49,000 (+10) | x |\n"
                          "| 2026-11-19 | bbb2222 | - | - | y |\n")
        self.assertEqual(health.last_log_row(path),
                         {"commit": "aaa1111", "lines": {"pruvc": 23000, "pruvc-rs": 49000}})

    def test_missing_file(self):
        self.assertIsNone(health.last_log_row(os.path.join(tempfile.gettempdir(), "no_such_health_log_xyz.md")))

    def test_no_rows(self):
        self.assertIsNone(health.last_log_row(self.write("")))


# ---- branch counting --------------------------------------------------------------------------------------------

RS_SRC = """fn a(x: i64) -> i64 { if x > 0 && x < 9 { 1 } else { match x { 0 => 2, _ => 3 } } }
trait T {
    fn b(&self);
}
fn c() { let s = "if while =>"; }
"""


class BranchTest(unittest.TestCase):
    def test_rs_branches(self):
        got = health.rs_branches(RS_SRC, "x.rs")
        counts = dict((w.split()[-1], n) for n, w in got)
        self.assertEqual(counts, {"a": 4, "c": 0})   # trait declaration `b` skipped; string literal not counted
        self.assertEqual(len(got), 2)
        self.assertTrue(got[0][1].startswith("x.rs:1 "))

    def test_strip_rs(self):
        s = health.strip_rs('let a = "if while =>"; // if\n/* while\n*/ let c = \'x\';\n')
        self.assertNotIn("if", s)
        self.assertNotIn("while", s)
        self.assertNotIn("=>", s)
        self.assertEqual(s.count("\n"), 3)

    def test_py_branches(self):
        src = ("def f(a, b, c):\n"
               "    if a:\n"
               "        pass\n"
               "    elif b:\n"
               "        pass\n"
               "    for i in a:\n"
               "        pass\n"
               "    return a and b and c\n")
        # if + elif (nested If) + for + BoolOp with 3 values (2)
        self.assertEqual(health.py_branches(src, "m.py"), [(5, "m.py:1 f")])


# ---- row --------------------------------------------------------------------------------------------------------

class RowTest(unittest.TestCase):
    def setUp(self):
        self.m = {"date": "2026-11-19 10:00", "commit": "abc1234", "lines": {"pruvc": 105, "pruvc-rs": 200},
                  "top_files": [("pruvc/a.py", 500), ("pruvc-rs/b.rs", 400)], "above_hand": [],
                  "ci": [(1, "success", "abc1234")], "disk_free": 100 * 10 ** 9, "pruv_bytes": 5 * 2 ** 30,
                  "work_bytes": 2 ** 20, "backed_up": True}
        self.last = {"lines": {"pruvc": 100, "pruvc-rs": 200}}

    def test_shape(self):
        r = health.row(self.m, self.last)
        self.assertTrue(r.startswith("| "))
        self.assertTrue(r.endswith(" |"))
        self.assertEqual(len(r.strip().strip("|").split(" | ")), 10)

    def test_contents(self):
        r = health.row(self.m, self.last)
        self.assertIn("(+5)", r)
        self.assertIn("200 (0)", r)
        self.assertIn("not measured", r)
        self.assertIn("no item above 1.0x hand", r)
        self.assertIn("backed up", r)

    def test_without_last(self):
        r = health.row(self.m, None)
        self.assertIn("| 105 |", r)
        self.assertNotIn("(+", r)

    def test_above_hand_and_ci_unavailable(self):
        self.m["above_hand"] = [("slow", 1.22, "open")]
        self.m["ci"] = None
        r = health.row(self.m, None)
        self.assertIn("slow 1.22", r)
        self.assertIn("gh unavailable", r)
        self.assertEqual(len(r.strip().strip("|").split(" | ")), 10)


if __name__ == "__main__":
    unittest.main()
