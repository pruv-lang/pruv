#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""inout overlap check test of mutable host entries (docs/RESIDENT_STATE_SPEC.md sections 3-4; standard library only, Python 3.9).

Usage: python3 tests/abi/check_inout_overlap.py [--pruvc PATH] [--py-pruvc PATH] [--keep DIR]
  --pruvc     Rust implementation of pruvc (default pruvc-rs/target/release/pruvc)
  --py-pruvc  Python implementation pruvc.py (default pruvc/pruvc.py; uses the .venv Python if present)
  --keep      keep the artifacts in DIR
Links the .ll that both compilers (py, rs) emit for the same explicit-mode source (root `pub` functions, some with `inout`
parameters) with the plain runtime (pruvc/runtime.c, not RUST_ALLOCATOR) and a C caller that calls `pruv_abi_*` directly.
Every case runs against a build without and a build with --trust-callers (the overlap check does not depend on trust).
The C caller prints `ret=N` (or `out=.. n=..`) and, from an atexit handler, `buf0=.. buf8=..` of its shared buffer, so a
rejected call (exit 101, which runs atexit) shows that the buffer was left untouched. Cases:
  *_disjoint / *_adjacent     the regions do not overlap (exactly adjacent included)  -> exit 0, buffer updated in place
  bump_overlap / _identical / _before   inout [i64] and read-only [i64] overlap       -> exit 101, buffer untouched
  pair_overlap                two inout arrays overlap                                -> exit 101
  fresh_out_in_inout          the result storage `%out` lies inside the inout buffer   -> exit 101
  fresh_overlap_ab            inout and read-only argument overlap (array result)      -> exit 101
  touch_zero_*                zero-length regions never overlap (identical / inside / zero inout) -> exit 0
  touch_negative*             a negative length is rejected like an overlap            -> exit 101
  size_*                      lone inout [i64] (empty-sentinel check): ok / empty -> exit 0; negative length,
                              len*8 multiply overflow, ptr+bytes add overflow     -> exit 101
  bytes_* / tag_*             [u8] and str byte granularity: adjacent -> exit 0, 1-byte overlap -> exit 101
                              (these also print `bb0=.. bb4=..` of a second, byte-sized shared buffer)
  flags_*                     [bool]: __typed overlap -> 101; raw wrapper with overlap and an invalid byte (7) -> 101
                              with the overlap message (overlap check runs before the bool check)
  tri_*                       three pointers, only the last pair (y, inout z) overlaps -> 101; all disjoint -> exit 0
stderr of a rejection contains `requires violated` and `inout argument overlaps another argument`.
For each case, one line `ok <compiler>/<name>` / `FAIL <compiler>/<name>: reason`, and `N/N ok` at the end.
Exit: 0 all pass / 1 some fail / 2 tool or input missing.
"""
import argparse
import os
import platform
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
RUNTIME = REPO / "pruvc" / "runtime.c"
CPU_FLAG = "-mcpu=native" if platform.machine().lower() in ("arm64", "aarch64") else "-march=native"
REQ_MSG = "requires violated"
OVERLAP_MSG = "inout argument overlaps another argument"

SOURCE = """// For the inout overlap check of mutable host entries (explicit mode: root `pub` functions)
pub fn bump(inout a: [i64], b: [i64]) -> i64
  requires len(a) >= 1
  ensures len(a) == len(old(a))
  ensures a[0] == len(b)
  ensures result == len(a)
{
  a[0] = len(b);
  return len(a);
}

pub fn pair(inout a: [i64], inout c: [i64]) -> i64
  requires len(a) >= 1 && len(c) >= 1
  ensures len(a) == len(old(a)) && len(c) == len(old(c))
  ensures a[0] == 1 && c[0] == 2
{
  a[0] = 1;
  c[0] = 2;
  return 3;
}

pub fn fresh(inout a: [i64], b: [i64]) -> [i64] ! alloc
  requires len(a) >= 1
{
  a[0] = 5;
  let w: [i64] = copy(b);
  return w;
}

pub fn touch(inout a: [i64], b: [i64]) -> i64 {
  return len(a);
}

// lone inout: only one pointer-carrying argument, checked against an empty sentinel
pub fn lone(inout a: [i64]) -> i64 {
  return len(a);
}

// byte granularity ([u8] elements are 1 byte)
pub fn bytes(inout a: [u8], b: [u8]) -> i64
  requires len(a) >= 1 && len(b) >= 1
{
  a[0] = b[0];
  return len(b);
}

// str argument inside an inout [u8]
pub fn tag(inout a: [u8], s: str) -> i64 {
  return len(s);
}

// [bool]: raw wrapper pruv_abi_flags checks bytes, pruv_abi_flags__typed does not; overlap runs first in both
pub fn flags(inout f: [bool], g: [bool]) -> i64
  requires len(f) >= 1 && len(g) >= 1
{
  f[0] = g[0];
  return len(g);
}

// three pointers: only the last pair (y, inout z) may overlap
pub fn tri(x: [i64], y: [i64], inout z: [i64]) -> i64
  requires len(x) >= 1 && len(z) >= 1
{
  z[0] = x[0];
  return len(y);
}
"""

CALLER = """#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
extern int64_t pruv_abi_bump(int64_t *a, int64_t an, const int64_t *b, int64_t bn);
extern int64_t pruv_abi_pair(int64_t *a, int64_t an, int64_t *c, int64_t cn);
extern void pruv_abi_fresh(int64_t *a, int64_t an, const int64_t *b, int64_t bn, void *out);
extern int64_t pruv_abi_touch(int64_t *a, int64_t an, const int64_t *b, int64_t bn);
extern int64_t pruv_abi_lone(int64_t *a, int64_t an);
extern int64_t pruv_abi_bytes(uint8_t *a, int64_t an, const uint8_t *b, int64_t bn);
extern int64_t pruv_abi_tag(uint8_t *a, int64_t an, const uint8_t *s, int64_t sn);
extern int64_t pruv_abi_flags(uint8_t *f, int64_t fn, const uint8_t *g, int64_t gn);
extern int64_t pruv_abi_flags__typed(uint8_t *f, int64_t fn, const uint8_t *g, int64_t gn);
extern int64_t pruv_abi_tri(const int64_t *x, int64_t xn, const int64_t *y, int64_t yn, int64_t *z, int64_t zn);
static int64_t buf[16];
static uint8_t bb[16];
static int show_bb = 0;
static void report(void) {
    printf("buf0=%lld buf8=%lld\\n", (long long)buf[0], (long long)buf[8]);
    if (show_bb) printf("bb0=%d bb4=%d\\n", (int)bb[0], (int)bb[4]);
}
struct arr { int64_t *data; int64_t len; };
int main(int argc, char **argv) {
    const char *c = argc > 1 ? argv[1] : "";
    int64_t r = 0;
    for (int k = 0; k < 16; k++) buf[k] = 100 + k;
    for (int k = 0; k < 16; k++) bb[k] = (uint8_t)(50 + k);
    if (!strncmp(c, "bytes_", 6) || !strncmp(c, "tag_", 4)) show_bb = 1;
    if (!strncmp(c, "flags_", 6)) {  /* bool bytes: all 0 except bb[4] = 1 */
        show_bb = 1;
        memset(bb, 0, sizeof bb);
        bb[4] = 1;
        if (!strcmp(c, "flags_raw_overlap_invalid")) bb[5] = 7;  /* invalid bool byte inside g */
    }
    atexit(report);
    if (!strcmp(c, "bump_disjoint")) r = pruv_abi_bump(buf, 4, buf + 8, 3);
    else if (!strcmp(c, "bump_adjacent")) r = pruv_abi_bump(buf, 4, buf + 4, 4);
    else if (!strcmp(c, "bump_overlap")) r = pruv_abi_bump(buf, 4, buf + 3, 4);
    else if (!strcmp(c, "bump_identical")) r = pruv_abi_bump(buf, 4, buf, 4);
    else if (!strcmp(c, "bump_before")) r = pruv_abi_bump(buf + 2, 4, buf, 4);
    else if (!strcmp(c, "pair_disjoint")) r = pruv_abi_pair(buf, 4, buf + 8, 4);
    else if (!strcmp(c, "pair_overlap")) r = pruv_abi_pair(buf, 4, buf + 2, 4);
    else if (!strcmp(c, "fresh_disjoint")) {
        struct arr out = {0, 0};
        pruv_abi_fresh(buf, 4, buf + 8, 4, &out);
        printf("out=%lld n=%lld\\n", (long long)out.data[0], (long long)out.len);
        return 0;
    }
    else if (!strcmp(c, "fresh_out_in_inout")) {
        pruv_abi_fresh(buf, 8, buf + 8, 4, (void *)(buf + 2));
        printf("unreachable\\n");
        return 0;
    }
    else if (!strcmp(c, "fresh_overlap_ab")) {
        struct arr out = {0, 0};
        pruv_abi_fresh(buf, 6, buf + 4, 4, &out);
        printf("unreachable\\n");
        return 0;
    }
    else if (!strcmp(c, "touch_zero_identical")) r = pruv_abi_touch(buf, 0, buf, 0);
    else if (!strcmp(c, "touch_zero_inside")) r = pruv_abi_touch(buf, 4, buf + 2, 0);
    else if (!strcmp(c, "touch_zero_inout")) r = pruv_abi_touch(buf + 2, 0, buf, 4);
    else if (!strcmp(c, "touch_negative")) r = pruv_abi_touch(buf, -1, buf + 8, 4);
    else if (!strcmp(c, "touch_negative_b")) r = pruv_abi_touch(buf, 4, buf + 8, -3);
    else if (!strcmp(c, "size_ok")) r = pruv_abi_lone(buf, 4);
    else if (!strcmp(c, "size_negative")) r = pruv_abi_lone(buf, -1);
    else if (!strcmp(c, "size_mul_overflow")) r = pruv_abi_lone(buf, (int64_t)1 << 62);
    else if (!strcmp(c, "size_add_overflow")) r = pruv_abi_lone((int64_t *)(UINTPTR_MAX - 15), 4);
    else if (!strcmp(c, "size_empty")) r = pruv_abi_lone(buf, 0);
    else if (!strcmp(c, "bytes_adjacent")) r = pruv_abi_bytes(bb, 4, bb + 4, 4);
    else if (!strcmp(c, "bytes_overlap_1")) r = pruv_abi_bytes(bb, 4, bb + 3, 4);
    else if (!strcmp(c, "tag_adjacent_str")) r = pruv_abi_tag(bb, 4, bb + 4, 2);
    else if (!strcmp(c, "tag_overlap_str")) r = pruv_abi_tag(bb, 8, bb + 4, 2);
    else if (!strcmp(c, "flags_typed_disjoint")) r = pruv_abi_flags__typed(bb, 4, bb + 4, 4);
    else if (!strcmp(c, "flags_typed_overlap")) r = pruv_abi_flags__typed(bb, 4, bb + 3, 4);
    else if (!strcmp(c, "flags_raw_overlap_invalid")) r = pruv_abi_flags(bb, 4, bb + 3, 4);
    else if (!strcmp(c, "tri_disjoint")) r = pruv_abi_tri(buf, 2, buf + 4, 2, buf + 8, 4);
    else if (!strcmp(c, "tri_last_pair_overlap")) r = pruv_abi_tri(buf, 2, buf + 4, 4, buf + 7, 4);
    else return 64;
    printf("ret=%lld\\n", (long long)r);
    return 0;
}
"""

UNTOUCHED = "buf0=100 buf8=108"
UNTOUCHED_BB = "bb0=50 bb4=54"
# (name, expected exit, expected stdout (stripped), reject?)
CASES = [
    ("bump_disjoint", 0, "ret=4\nbuf0=3 buf8=108", False),
    ("bump_adjacent", 0, "ret=4\nbuf0=4 buf8=108", False),
    ("bump_overlap", 101, UNTOUCHED, True),
    ("bump_identical", 101, UNTOUCHED, True),
    ("bump_before", 101, UNTOUCHED, True),
    ("pair_disjoint", 0, "ret=3\nbuf0=1 buf8=2", False),
    ("pair_overlap", 101, UNTOUCHED, True),
    ("fresh_disjoint", 0, "out=108 n=4\nbuf0=5 buf8=108", False),
    ("fresh_out_in_inout", 101, UNTOUCHED, True),
    ("fresh_overlap_ab", 101, UNTOUCHED, True),
    ("touch_zero_identical", 0, "ret=0\n" + UNTOUCHED, False),
    ("touch_zero_inside", 0, "ret=4\n" + UNTOUCHED, False),
    ("touch_zero_inout", 0, "ret=0\n" + UNTOUCHED, False),
    ("touch_negative", 101, UNTOUCHED, True),
    ("touch_negative_b", 101, UNTOUCHED, True),
    # lone inout region (empty-sentinel check)
    ("size_ok", 0, "ret=4\n" + UNTOUCHED, False),
    ("size_negative", 101, UNTOUCHED, True),
    ("size_mul_overflow", 101, UNTOUCHED, True),
    ("size_add_overflow", 101, UNTOUCHED, True),
    ("size_empty", 0, "ret=0\n" + UNTOUCHED, False),
    # [u8] / str byte granularity; bb[k] = 50 + k, so bb0=50 bb4=54 untouched
    ("bytes_adjacent", 0, "ret=4\n" + UNTOUCHED + "\nbb0=54 bb4=54", False),  # a[0] = b[0] = bb[4]
    ("bytes_overlap_1", 101, UNTOUCHED + "\n" + UNTOUCHED_BB, True),
    ("tag_adjacent_str", 0, "ret=2\n" + UNTOUCHED + "\n" + UNTOUCHED_BB, False),
    ("tag_overlap_str", 101, UNTOUCHED + "\n" + UNTOUCHED_BB, True),
    # [bool]: bb all 0 except bb[4] = 1 (flags_raw_overlap_invalid also has bb[5] = 7)
    ("flags_typed_disjoint", 0, "ret=4\n" + UNTOUCHED + "\nbb0=1 bb4=1", False),  # f[0] = g[0] = bb[4]
    ("flags_typed_overlap", 101, UNTOUCHED + "\nbb0=0 bb4=1", True),
    ("flags_raw_overlap_invalid", 101, UNTOUCHED + "\nbb0=0 bb4=1", True),  # overlap message, not the bool repr panic
    # three pointers
    ("tri_disjoint", 0, "ret=2\nbuf0=100 buf8=100", False),  # z[0] = x[0] = 100
    ("tri_last_pair_overlap", 101, UNTOUCHED, True),
]


def run(argv, timeout=600):
    try:
        p = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
    except (OSError, subprocess.SubprocessError) as e:
        return 127, "", str(e)
    return p.returncode, p.stdout, p.stderr


def py_python():
    for cand in (REPO / ".venv" / "bin" / "python", REPO / ".venv" / "Scripts" / "python.exe"):
        if cand.exists():
            return str(cand)
    return sys.executable


class Ctx:
    def __init__(self, a, work):
        self.a = a
        self.work = work
        self.src = work / "overlap.pruv"
        self.src.write_text(SOURCE)
        self.objs = {}  # (compiler, trust) -> (exe, error)
        self.rt_obj = None
        self.caller_obj = None

    def common(self):
        """(runtime object, C caller object, error)"""
        if self.rt_obj is None:
            rt = self.work / "rt_plain.o"
            rc, _, err = run([self.a.clang, "-O2", CPU_FLAG, "-std=c11", "-c", str(RUNTIME), "-o", str(rt)])
            if rc != 0:
                return None, None, "clang(runtime.c) failed: " + err.strip()[-300:]
            cs = self.work / "caller.c"
            cs.write_text(CALLER)
            co = self.work / "caller.o"
            rc, _, err = run([self.a.clang, "-O2", "-c", str(cs), "-o", str(co)])
            if rc != 0:
                return None, None, "clang(caller.c) failed: " + err.strip()[-300:]
            self.rt_obj, self.caller_obj = rt, co
        return self.rt_obj, self.caller_obj, None

    def exe(self, comp, trust):
        k = (comp, trust)
        if k in self.objs:
            return self.objs[k]
        tag = "%s_%s" % (comp, "trust" if trust else "plain")
        ll = self.work / (tag + ".ll")
        flags = ["--backend", "llvm", "-o", str(ll)] + (["--trust-callers"] if trust else [])
        if comp == "rs":
            argv = [self.a.pruvc, str(self.src), "--emit", "ll"] + flags
        else:  # pruvc.py has no --emit; `--backend llvm -o X.ll` writes the IR
            argv = [py_python(), self.a.py_pruvc, str(self.src)] + flags
        rc, out, err = run(argv)
        if rc != 0 or not ll.exists():
            self.objs[k] = (None, "pruvc(%s) failed (%d): %s" % (comp, rc, (out + err).strip()[-300:]))
            return self.objs[k]
        rt, co, e = self.common()
        if e:
            self.objs[k] = (None, e)
            return self.objs[k]
        gen = self.work / (tag + "_gen.o")
        rc, _, err = run([self.a.clang, "-O2", CPU_FLAG, "-Wno-override-module", "-c", str(ll), "-o", str(gen)])
        if rc != 0:
            self.objs[k] = (None, "clang(.ll) failed: " + err.strip()[-300:])
            return self.objs[k]
        exe = self.work / (tag + (".exe" if os.name == "nt" else ""))
        rc, _, err = run([self.a.clang, str(co), str(gen), str(rt), "-o", str(exe)])
        if rc != 0:
            self.objs[k] = (None, "link failed: " + err.strip()[-300:])
            return self.objs[k]
        self.objs[k] = (exe, None)
        return self.objs[k]


def run_case(ctx, comp, trust, case):
    name, want_exit, want_out, reject = case
    exe, err = ctx.exe(comp, trust)
    if err:
        return err
    rc, out, e = run([str(exe), name], timeout=60)
    if rc != want_exit:
        return "exit code %d (expected %d) stderr=%r" % (rc, want_exit, e.strip()[-200:])
    if out.strip() != want_out:
        return "stdout %r (expected %r)" % (out.strip(), want_out)
    if reject:
        for msg in (REQ_MSG, OVERLAP_MSG):
            if msg not in e:
                return "stderr %r does not contain %r" % (e.strip()[-200:], msg)
    elif e.strip():
        return "unexpected stderr %r" % e.strip()[-200:]
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pruvc", default=str(REPO / "pruvc-rs" / "target" / "release" / ("pruvc.exe" if os.name == "nt" else "pruvc")))
    ap.add_argument("--py-pruvc", default=str(REPO / "pruvc" / "pruvc.py"))
    ap.add_argument("--keep", default=None)
    a = ap.parse_args()
    a.clang = shutil.which("clang") or "/usr/bin/clang"

    missing = []
    if not Path(a.pruvc).exists():
        missing.append("pruvc(%s)" % a.pruvc)
    if not Path(a.py_pruvc).exists():
        missing.append("pruvc.py(%s)" % a.py_pruvc)
    if not (os.path.exists(a.clang) or shutil.which(a.clang)):
        missing.append("clang")
    if not RUNTIME.exists():
        missing.append(str(RUNTIME))
    if missing:
        print("required tools/inputs not found: " + ", ".join(missing))
        return 2

    if a.keep:
        work = Path(a.keep)
        work.mkdir(parents=True, exist_ok=True)
        cleanup = False
    else:
        work = Path(tempfile.mkdtemp(prefix="pruv_inout_overlap_"))
        cleanup = True
    try:
        ctx = Ctx(a, work)
        total = ok = 0
        for comp in ("py", "rs"):
            for trust in (False, True):
                for case in CASES:
                    total += 1
                    label = "%s/%s%s" % (comp, case[0], "_trust" if trust else "")
                    try:
                        why = run_case(ctx, comp, trust, case)
                    except Exception as e:  # so one case's exception does not hide the rest
                        why = "exception: %r" % (e,)
                    if why is None:
                        ok += 1
                        print("ok %s" % label)
                    else:
                        print("FAIL %s: %s" % (label, why))
        print("%d/%d ok" % (ok, total))
        if a.keep:
            print("artifacts: " + str(work))
        return 0 if ok == total else 1
    finally:
        if cleanup:
            shutil.rmtree(str(work), ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
