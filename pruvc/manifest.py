"""G4b-1 (docs/G4B_SPEC.md sections 3, 4.1, 5): package manifests (pruv.json) and the package graph.

load_graph(manifest_path)  validates the program manifest and, depth first in key order, every dependency manifest;
                           checks the graph is acyclic, one directory per package name and every version requirement
                           satisfied.  Returns a Graph; load_program mounts the packages it describes.
All errors are PruvError stage "pkg"; no machine path appears in any message.
"""
import json
import os
import re

from vast import PruvError
from vparser import KEYWORDS, RUST_KEYWORDS, _MOD_NAME_RE

_NAME_RE = re.compile(r"[a-z][a-z0-9_]*\Z")
_NUM = r"(?:0|[1-9][0-9]*)"
_VERSION_RE = re.compile(r"%s\.%s\.%s\Z" % (_NUM, _NUM, _NUM))
_REQ_RE = re.compile(r"%s(?:\.%s){0,2}\Z" % (_NUM, _NUM))
_MAX = 2 ** 31 - 1
_FIX = "see docs/G4B_SPEC.md section 3"


class Pkg:
    def __init__(s, name, version, dir, root, deps, display, raw_entries=None):
        s.name = name
        s.version = version
        s.dir = dir                # canonical package directory
        s.root = root              # root file, relative to dir, `/` separators
        s.deps = deps              # [(mount key, canonical dir, requirement text)] in JSON key order
        s.display = display        # display of this package's manifest
        s.raw_entries = raw_entries  # P06: None (host entries stay `pub fn`), "crate" or "unsafe" (program only)
        s.reqs = []                # [(requiring Pkg, mount key, requirement text)] in discovery order
        s.first = None             # (requiring Pkg, mount key) that first reached this directory; None = the program


class Graph:
    def __init__(s):
        s.program = None
        s.pkgs = []                # discovery order, the program package first
        s.by_dir = {}              # canonical dir -> Pkg


class _Obj(dict):
    """a JSON object that remembers its duplicate keys"""
    dups = ()


def _pairs(pairs):
    d = _Obj()
    dups = []
    for k, v in pairs:
        if k in d:
            dups.append(k)
        d[k] = v
    d.dups = dups
    return d


def _bad(display, keypath, reason):
    raise PruvError("pkg", 1, 1, "manifest_invalid", "invalid manifest key %s: %s" % (keypath, reason), _FIX,
                    file=display)


def _parts(text):
    return tuple(int(x) for x in text.split("."))


def _plain_rel(p, allow_dotdot):
    """a relative path with `/` separators and no empty segment"""
    if not p or p.startswith("/") or "\\" in p or "\0" in p or (len(p) > 1 and p[1] == ":" and p[0].isalpha()):
        return False
    segs = p.split("/")
    if any(x == "" for x in segs):
        return False
    return allow_dotdot or ".." not in segs


def _check_obj(obj, path, allowed, display):
    """duplicate and unknown keys of one object (path = key path prefix, "" at the top)"""
    pre = path + "." if path else ""
    if obj.dups:
        _bad(display, pre + obj.dups[0], "duplicate key")
    for k in obj:
        if k not in allowed:
            _bad(display, pre + k, "unknown key")


def _read(pkgdir, mfile, display):
    """read and validate one manifest; returns a Pkg (deps as canonical directories)"""
    try:
        with open(mfile, "rb") as fh:
            data = json.loads(fh.read().decode("utf-8"), object_pairs_hook=_pairs)
    except (OSError, ValueError, RecursionError):
        data = None
    if not isinstance(data, _Obj):
        raise PruvError("pkg", 1, 1, "manifest_invalid", "invalid manifest: not a UTF-8 JSON object", _FIX,
                        file=display)
    _check_obj(data, "", ("name", "version", "root", "dependencies", "raw_entries"), display)
    for k in ("name", "version", "root"):
        if k not in data:
            _bad(display, k, "missing")
        if not isinstance(data[k], str):
            _bad(display, k, "expected a string")
    name, version, root = data["name"], data["version"], data["root"]
    if not _NAME_RE.match(name) or "__" in name or name.startswith("pruv_") or name in ("std", "crate") \
            or name in RUST_KEYWORDS or name in KEYWORDS:
        _bad(display, "name", "bad package name")
    if display.startswith("@pkg/"):
        # a dependency manifest is reported under the package's real name, not its mount key
        display = "@pkg/%s/pruv.json" % name
    if not _VERSION_RE.match(version) or any(x > _MAX for x in _parts(version)):
        _bad(display, "version", "bad version")
    if not _plain_rel(root, False) or not root.endswith(".pruv"):
        _bad(display, "root", "bad root path")
    rpath = os.path.join(pkgdir, *root.split("/"))
    if not os.path.isfile(rpath):
        _bad(display, "root", "root file not found")
    # the root file (after resolving symlinks) must stay inside the canonical package directory, by path components
    try:
        inside = os.path.commonpath([pkgdir, os.path.realpath(rpath)]) == pkgdir
    except ValueError:
        inside = False
    if not inside:
        _bad(display, "root", "root file is outside the package directory")
    raw = data.get("raw_entries")
    if raw is not None or "raw_entries" in data:
        # P06 (docs/L3_RESTRUCTURE_SPEC.md appendix P06): visibility of the raw host entries.  A raw entry takes
        # arbitrary buffers, so it is never a public safe Rust fn: pub(crate) behind a host adapter, or a
        # documented `pub unsafe fn`.  Only the program package has host entries, so only it may choose.
        if display.startswith("@pkg/"):
            _bad(display, "raw_entries", "only the program package chooses raw entry visibility")
        if not isinstance(raw, str):
            _bad(display, "raw_entries", "expected a string")
        if raw in ("pub", "public", "safe"):
            _bad(display, "raw_entries", "a raw entry is never a public safe function (use \"crate\" or \"unsafe\")")
        if raw not in ("crate", "unsafe"):
            _bad(display, "raw_entries", "expected \"crate\" or \"unsafe\"")
    deps = []
    dd = data.get("dependencies", _Obj())
    if not isinstance(dd, _Obj):
        _bad(display, "dependencies", "expected an object")
    if dd.dups:
        _bad(display, "dependencies." + dd.dups[0], "duplicate key")
    for key, ent in dd.items():
        kp = "dependencies." + key
        # the key must be a name that `mod KEY;` can declare (same predicate as the package name)
        if not _MOD_NAME_RE.match(key) or key == "std" or "__" in key or key.startswith("pruv_") \
                or key in ("std", "crate") or key in RUST_KEYWORDS or key in KEYWORDS:
            _bad(display, kp, "bad mount name")
        if not isinstance(ent, _Obj):
            _bad(display, kp, "expected an object")
        _check_obj(ent, kp, ("path", "version"), display)
        for k in ("path", "version"):
            if k not in ent:
                _bad(display, kp + "." + k, "missing")
            if not isinstance(ent[k], str):
                _bad(display, kp + "." + k, "expected a string")
        p, req = ent["path"], ent["version"]
        if not _plain_rel(p, True):
            _bad(display, kp + ".path", "bad dependency path")
        ddir = os.path.realpath(os.path.join(pkgdir, *p.split("/")))
        if not os.path.isfile(os.path.join(ddir, "pruv.json")):
            _bad(display, kp + ".path", "no pruv.json in the dependency directory")
        if not _REQ_RE.match(req) or any(x > _MAX for x in _parts(req)):
            _bad(display, kp + ".version", "bad version requirement")
        deps.append((key, ddir, req))
    return Pkg(name, version, pkgdir, root, deps, display, raw)


def _line(v):
    return (v[0],) if v[0] >= 1 else (0, v[1])


def load_graph(manifest_path):
    g = Graph()
    mpath = os.path.abspath(manifest_path)
    prog = _read(os.path.realpath(os.path.dirname(mpath)), mpath, os.path.basename(manifest_path))
    g.program = prog
    g.pkgs.append(prog)
    g.by_dir[prog.dir] = prog
    stack = [prog]

    def visit(pkg):
        for key, ddir, req in pkg.deps:
            for i, s in enumerate(stack):
                if s.dir == ddir:
                    names = [x.name for x in stack[i:]] + [s.name]
                    raise PruvError("pkg", 1, 1, "dependency_cycle", "dependency cycle: " + " -> ".join(names),
                                    "remove one of the dependencies on the cycle", file=pkg.display)
            d = g.by_dir.get(ddir)
            if d is None:
                d = _read(ddir, os.path.join(ddir, "pruv.json"), "@pkg/%s/pruv.json" % key)
                for o in g.pkgs:
                    if o.name == d.name:
                        if o.first is None:
                            a = "the program package %s (version %s)" % (o.name, o.version)
                        else:
                            a = "required by %s as %s (version %s)" % (o.first[0].name, o.first[1], o.version)
                        raise PruvError("pkg", 1, 1, "identity_conflict",
                                        "package %s is loaded from two places: %s and by %s as %s (version %s)"
                                        % (d.name, a, pkg.name, key, d.version),
                                        "point every dependency on %s at one directory" % d.name, file=d.display)
                d.first = (pkg, key)
                g.pkgs.append(d)
                g.by_dir[ddir] = d
                d.reqs.append((pkg, key, req))
                stack.append(d)
                visit(d)
                stack.pop()
            else:
                d.reqs.append((pkg, key, req))

    visit(prog)
    for p in g.pkgs:
        found = _parts(p.version)
        bad = [r for r in p.reqs
               if not (_line(found) == _line(_pad(r[2])) and found >= _pad(r[2]))]
        if bad:
            raise PruvError("pkg", 1, 1, "version_unsatisfied",
                            "package %s version %s does not satisfy: %s"
                            % (p.name, p.version, ", ".join("%s requires %s" % (r[0].name, r[2]) for r in p.reqs)),
                            "change the requirement or the dependency's version", file=bad[0][0].display)
    return g


def _pad(req):
    v = _parts(req)
    return v + (0,) * (3 - len(v))
