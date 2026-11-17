"""Shared run metadata and lane helpers for the L3 budget runners (spec §6.1 / P02).

Every runner that reports a Pruv number records `run_meta(...)` next to it: compiler path and version, backend,
rustc version and flags, seed, profile and lanes. Counters that a run could not read are UNAVAILABLE, never 0.

  from run_meta import run_meta, UNAVAILABLE, PROFILES, SEED, impl_argv
"""
import hashlib
import os
import platform
import shlex
import subprocess
import sys

SEED = 20261117
UNAVAILABLE = "unavailable"
# §6.1: S = 100, M = 500 (sessions / calls per size axis)
PROFILES = {"S": (100,), "M": (500,), "SM": (100, 500)}
RUSTC_FLAGS = ["-C", "opt-level=3", "--edition", "2021"]


def _out(cmd):
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=30)
    except (OSError, subprocess.SubprocessError):
        return UNAVAILABLE
    if r.returncode != 0:
        return UNAVAILABLE
    return (r.stdout or r.stderr).strip().splitlines()[0] if (r.stdout or r.stderr).strip() else UNAVAILABLE


def _sha(path):
    try:
        with open(path, "rb") as f:
            return hashlib.sha256(f.read()).hexdigest()[:16]
    except OSError:
        return UNAVAILABLE


def run_meta(pruvc=None, backend="rust", rustc=None, flags=None, seed=SEED, profile=None, lanes=None, extra=None):
    """Metadata dict for one run. Missing facts are UNAVAILABLE (a string), never 0 or an empty string."""
    m = {
        "pruvc": pruvc or UNAVAILABLE,
        "pruvc_sha256_16": _sha(pruvc) if pruvc else UNAVAILABLE,
        "backend": backend or UNAVAILABLE,
        "rustc": rustc or UNAVAILABLE,
        "rustc_version": _out([rustc, "--version"]) if rustc else UNAVAILABLE,
        "flags": list(flags) if flags is not None else list(RUSTC_FLAGS),
        "seed": seed,
        "profile": profile or UNAVAILABLE,
        "lanes": list(lanes) if lanes else UNAVAILABLE,
        "python": sys.version.split()[0],
        "machine": platform.machine() or UNAVAILABLE,
    }
    if extra:
        m.update(extra)
    return m


def num(x):
    """A counter value for a report: the number, or UNAVAILABLE when it was not read (None)."""
    return UNAVAILABLE if x is None else x


def impl_argv(value):
    """--impl value -> argv list. A value that names an existing file is one argv element (spaced paths are safe
    without inner quoting); anything else is split with POSIX shell rules (quote inner spaced paths)."""
    if isinstance(value, (list, tuple)):
        return list(value)
    if os.path.exists(value):
        return [value]
    return shlex.split(value)
