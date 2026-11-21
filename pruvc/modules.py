"""G4 A0: program files, name resolution and visibility (docs/G4_SPEC.md sections 2, 3, 5 (names), 8, 10).

load_program(path)   file-taking entry point.  A root with neither `mod` nor a top-level `pub` is returned exactly as
                     vparser.parse(src) produced it (legacy mode: nothing in this module touches it again).
                     Otherwise the explicit visibility mode is active for the whole program: the declared child files
                     are loaded (root first, depth first, declaration order), every node is tagged with its file, and
                     Program.items / functions / types hold the items of all modules in that order.  Names are still
                     the names as written (Function.name, TypeDecl.name; qualified references stay `crate::a::b::x`).
resolve(prog)        called by check.Checker.__init__ (explicit mode only): validates declaration names, replaces every
                     lookup key by the item's resolved identity "crate::a::b::name" (root items "crate::name"),
                     checks access (private_item), public interfaces (public_interface) and the root main, and sets
                     the host-entry classification.  After it the flat program has unique names, so the existing
                     checker, G3 owners and G9 graphs run unchanged on it.

Identity key = the canonical absolute path of the item.  Function.name / TypeDecl.name hold it after resolution; the
source spelling stays in `src_name`, the defining module in `mod_path`.  Diagnostics print identities (they are
source-qualified).  Type-parameter owners become "fn:<identity>" / "type:<identity>".
"""
import hashlib
import os

from vast import *
from vparser import parse, lex
from typeinfo import BUILTIN_TYPE_NAMES, dependencies, decl_owner, fn_owner
from refine import _kids


def disp(path, pkg=None):
    """display form of a module path tuple: crate, crate::a, crate::a::b (a dependency package: @json, @json::a)"""
    return "::".join((("crate",) if pkg is None else ("@" + pkg,)) + tuple(path))


def encode_name(components):
    """G4 A2 (docs/G4_SPEC.md 6): the count of the components, then for each one `_<ASCII byte length>_<component>`.
    Length-delimited, so underscores inside a component cannot make two sequences collide."""
    comps = tuple(components)
    return str(len(comps)) + "".join("_%d_%s" % (len(c.encode("ascii")), c) for c in comps)


def encode_stem(kind, components):
    """Rust stem of an item in explicit mode: kind "f" (function) or "t" (nominal type); components = the module path
    followed by the item's source name.  pruv_m_f_1_6_helper, pruv_m_f_3_1_a_3_geo_4_area, pruv_m_t_2_3_geo_5_Point."""
    return "pruv_m_%s_%s" % (kind, encode_name(components))


def identity_components(identity):
    """"crate::a::b::x" -> ("a", "b", "x")"""
    return tuple(identity.split("::")[1:])


def safe_type(ty):
    """a Type whose nominal names (identities) are replaced by their encode_name, so that the existing G3 stem and
    argument encodings (typeinfo.encode, spec_*_stem) only ever see [0-9A-Za-z_] names; injective like the identity."""
    if ty.owner is not None:
        return ty
    args = tuple(safe_type(a) for a in ty.args)
    if ty.name in BUILTIN_TYPE_NAMES or not (ty.name.startswith("crate::") or ty.name.startswith("@")):
        if args == ty.args:
            return ty
        return Type(ty.name, args)
    if ty.name.startswith("@"):
        return Type("p" + encode_name(ty.name[1:].split("::")), args)
    return Type(encode_name(identity_components(ty.name)), args)


class Module:
    def __init__(s, path, file, parent):
        s.path = path            # tuple of module names; () is the root
        s.file = file            # path relative to the root directory, `/` separators
        s.parent = parent
        s.pkg = None             # G4b-1: package name of a dependency module, None = the program package
        s.pkginfo = None         # manifest mode: the manifest.Pkg this module belongs to
        s.abspath = None         # G4b-2: the path the file was opened from (the trust report hashes its raw bytes)
        s.mods = []              # ModDecl list in source order (duplicates included)
        s.children = {}          # name -> Module (loaded children; a duplicate declaration is not loaded again)
        s.items = []             # Function / StructDecl / EnumDecl in source order
        s.funcs = {}             # source name -> Function   (filled by resolve)
        s.types = {}             # source name -> TypeDecl   (filled by resolve)
        s.digest = None          # PR1: sha256 hex of the parsed source text (module proof record key)
        s.rdigest = None         # PR1: sha256 hex of the module's items after resolution (module_edges)
        s.deps = set()           # PR1: indices (into Program.modules) of modules this one references (filled by resolve)


def _digest(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


# ---------------------------------------------------------------------------
# loading
# ---------------------------------------------------------------------------
def _tag(roots, file):
    """set `.file` on every node below roots (iterative: deep expressions must not hit the recursion limit)"""
    stack = list(roots)
    while stack:
        n = stack.pop()
        if isinstance(n, (list, tuple)):
            stack.extend(n)
            continue
        if n is None or isinstance(n, (Type, str, int, bool, bytes, dict)):
            continue
        if hasattr(n, "__dataclass_fields__") or isinstance(n, (TypeParam, ModDecl)):
            n.file = file
            stack.extend(vars(n).values())


def _sniff_explicit(src):
    """a root that failed to parse: does it use module syntax (a `mod` / `pub` token)?  -> its error carries `file`"""
    # one lexing pass: the tokens read before a lex error count, so a `pub` ahead of a bad character still counts
    toks = []
    try:
        lex(src, toks)
    except PruvError:
        pass
    return any(t.kind == "kw" and t.val in ("mod", "pub") for t in toks)


def _file_id(p):
    """the identity of a file (device, inode), computed once per loaded file; None when it cannot be inspected"""
    try:
        st = os.stat(p)
    except OSError:
        return None
    return (st.st_dev, st.st_ino)


def load_program(path, libs=(), manifest=None):
    """manifest: None, a pruv.json path, or a manifest.Graph (G4b-1: packages mounted by `mod KEY;` in package roots)"""
    graph = manifest
    if isinstance(graph, str):
        from manifest import load_graph
        graph = load_graph(graph)
    # the root is read exactly as before (same exceptions); parse(src) stays the file-local API
    src = open(path, encoding="utf-8").read()
    root_file = os.path.basename(path)
    try:
        prog = parse(src)
    except PruvError as e:
        if _sniff_explicit(src):
            e.file = root_file
        raise
    if graph is not None:
        prog.raw_entries = graph.program.raw_entries   # P06: applied by codegen in P09
        prog.pkg_name = graph.program.name   # P10H: prefix of every global C symbol
    if not (prog.mods or prog.has_pub):
        if graph is not None and graph.program.deps:
            raise PruvError("pkg", 1, 1, "unused_dependency",
                            "dependency %s is declared but not mounted by `mod %s;` in the package root"
                            % (graph.program.deps[0][0], graph.program.deps[0][0]),
                            "add `mod %s;` to the package root or remove the dependency" % graph.program.deps[0][0],
                            file=graph.program.display)
        return prog                      # legacy mode: untouched
    root_dir = os.path.dirname(os.path.abspath(path))
    prog.explicit = True
    root = Module((), root_file, None)
    root.base, root.relpre = root_dir, ""
    root.abspath = os.path.abspath(path)
    root.digest = _digest(src)
    if graph is not None:
        root.pkginfo = graph.program
    pkg_mods = {}                       # canonical dependency dir -> its root Module (one per package)
    root.mods = list(prog.mods)
    root.items = list(prog.items)
    _tag(root.items + root.mods, root_file)
    modules = [root]
    loaded = {}                         # file identity (device, inode) -> display path of every loaded file
    rid = _file_id(path)
    if rid is not None:
        loaded[rid] = root_file

    def visit(m):
        seen = set()
        for md in m.mods:
            if md.name in seen:
                continue                 # reported by resolve as type/duplicate_module; the file is loaded once
            seen.add(md.name)
            cpath = m.path + (md.name,)
            rel = m.relpre + "/".join(cpath) + ".pruv"
            fpath = os.path.join(m.base, *cpath) + ".pruv"
            base, relpre = m.base, m.relpre
            if graph is not None and not m.path:
                keys = {k: d for k, d, _ in m.pkginfo.deps}
                if md.name == "std" and m.pkg is not None:
                    raise PruvError("pkg", md.line, md.col, "std_in_dependency",
                                    "`mod std;` is only available in the program root, not in package %s" % m.pkg,
                                    "remove `mod std;`", file=m.file)
                if md.name in keys:
                    dpkg = graph.by_dir[keys[md.name]]
                    if os.path.lexists(fpath):
                        raise PruvError("pkg", md.line, md.col, "mount_conflict",
                                        "module %s is found in more than one place: %s, @pkg/%s"
                                        % (md.name, rel, dpkg.name),
                                        "remove the local file or rename the dependency key", file=m.file)
                    mount(m, md, dpkg)
                    continue
            if not m.path and md.name != "std" and graph is None:
                # G4b-0 (docs/G4B_LIB_SPEC.md 2): local file plus one candidate per --lib DIR
                cands = []
                if os.path.lexists(fpath):
                    cands.append((fpath, rel, base, relpre))
                for d in libs:
                    lp = os.path.join(d, md.name) + ".pruv"
                    if os.path.lexists(lp):
                        cands.append((lp, "@lib/" + md.name + ".pruv", d, "@lib/"))
                if len(cands) > 1:
                    raise PruvError("parse", md.line, md.col, "lib_module_conflict",
                                    "module %s is found in more than one place: %s"
                                    % (md.name, ", ".join(c[1] for c in cands)),
                                    "remove the duplicate file or drop one --lib", file=m.file)
                if cands:
                    fpath, rel, base, relpre = cands[0]
            if md.name == "std":
                if m.path:
                    raise PruvError("parse", md.line, md.col, "std_import_location",
                                    "std may only be imported at the program root",
                                    "move `mod std;` to the root and call `crate::std::name`", file=m.file)
                if os.path.lexists(fpath):
                    raise PruvError("parse", md.line, md.col, "std_module_conflict",
                                    "local std.pruv conflicts with the bundled std module",
                                    "rename the user module/file and its paths", file=m.file)
                fpath = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "stdlib", "std.pruv")
                rel = "@std/std.pruv"
                try:
                    with open(fpath, encoding="utf-8") as source:
                        text = source.read()
                except (OSError, UnicodeDecodeError):
                    raise PruvError("parse", md.line, md.col, "std_source",
                                    "cannot read the bundled std module as UTF-8",
                                    "restore/reinstall the matching compiler resource", file=m.file)
            try:
                if md.name != "std":
                    text = open(fpath, encoding="utf-8").read()
            except UnicodeDecodeError:
                raise PruvError("parse", md.line, md.col, "module_file",
                                "the file of module %s is not valid UTF-8" % disp(cpath),
                                "save %s as UTF-8" % rel, file=m.file)
            except OSError:
                if graph is not None and not m.path:
                    hint = "create %s next to the root file, add %s to dependencies, or remove `mod %s;`" \
                           % (rel, md.name, md.name)
                else:
                    hint = (("create %s next to the root file, pass --lib DIR, or remove `mod %s;`"
                             if not m.path else "create %s next to the root file, or remove `mod %s;`")
                            % (rel, md.name))
                raise PruvError("parse", md.line, md.col, "module_file",
                                "cannot read the file of module %s (expected %s)" % (disp(cpath, m.pkg), rel),
                                hint, file=m.file)
            # The bundle has a logical resource identity, not a physical one (same as pruvc-rs).
            fid = None if md.name == "std" else _file_id(fpath)
            if fid is not None and fid in loaded:
                raise PruvError("parse", md.line, md.col, "module_alias",
                                "the file %s of module %s was already loaded as %s" % (rel, disp(cpath), loaded[fid]),
                                "every module needs its own file; remove the alias", file=m.file)
            if fid is not None:
                loaded[fid] = rel
            try:
                cprog = parse(text)
            except PruvError as e:
                e.file = rel
                raise
            child = Module(cpath, rel, m)
            child.digest = _digest(text)
            child.base, child.relpre = base, relpre
            # the bundled std belongs to the compiler (its version), not to a package hash (G4B_SPEC 6 rule 11)
            child.abspath = None if md.name == "std" else os.path.abspath(fpath)
            child.pkg, child.pkginfo = m.pkg, m.pkginfo
            child.mods = list(cprog.mods)
            child.items = list(cprog.items)
            _tag(child.items + child.mods, rel)
            m.children[md.name] = child
            modules.append(child)
            visit(child)
        if graph is not None and not m.path:
            # every dependency key must be mounted by a `mod` of the package root
            for key, _, _ in m.pkginfo.deps:
                if key not in seen:
                    raise PruvError("pkg", 1, 1, "unused_dependency",
                                    "dependency %s is declared but not mounted by `mod %s;` in the package root"
                                    % (key, key),
                                    "add `mod %s;` to the package root or remove the dependency" % key,
                                    file=m.pkginfo.display)

    def mount(m, md, dpkg):
        """mount dependency package dpkg as child md.name of the package root m (its root Module is built once)"""
        done = pkg_mods.get(dpkg.dir)
        if done is not None:
            m.children[md.name] = done
            return
        cpath = (md.name,)
        sub = os.path.dirname(dpkg.root)
        rel = "@pkg/%s/%s" % (dpkg.name, dpkg.root)
        fpath = os.path.join(dpkg.dir, *dpkg.root.split("/"))
        try:
            text = open(fpath, encoding="utf-8").read()
        except UnicodeDecodeError:
            raise PruvError("parse", md.line, md.col, "module_file",
                            "the file of module %s is not valid UTF-8" % disp(cpath),
                            "save %s as UTF-8" % rel, file=m.file)
        except OSError:
            raise PruvError("parse", md.line, md.col, "module_file",
                            "cannot read the file of module %s (expected %s)" % (disp(cpath), rel),
                            "restore the root file of package %s" % dpkg.name, file=m.file)
        fid = _file_id(fpath)
        if fid is not None and fid in loaded:
            raise PruvError("parse", md.line, md.col, "module_alias",
                            "the file %s of module %s was already loaded as %s" % (rel, disp(cpath), loaded[fid]),
                            "every module needs its own file; remove the alias", file=m.file)
        if fid is not None:
            loaded[fid] = rel
        try:
            cprog = parse(text)
        except PruvError as e:
            e.file = rel
            raise
        child = Module((), rel, m)
        child.digest = _digest(text)
        child.pkg, child.pkginfo = dpkg.name, dpkg
        child.abspath = os.path.abspath(fpath)
        child.base = os.path.join(dpkg.dir, *sub.split("/")) if sub else dpkg.dir
        child.relpre = "@pkg/%s/%s" % (dpkg.name, sub + "/" if sub else "")
        child.mods = list(cprog.mods)
        child.items = list(cprog.items)
        _tag(child.items + child.mods, rel)
        pkg_mods[dpkg.dir] = child
        m.children[md.name] = child
        modules.append(child)
        visit(child)

    visit(root)
    prog.items = []
    for m in modules:
        for it in m.items:
            it.pkg = m.pkg
            it.mod_path = m.path
            it.src_name = it.name
            prog.items.append(it)
    prog.functions = [i for i in prog.items if isinstance(i, Function)]
    prog.types = [i for i in prog.items if isinstance(i, TypeDecl)]
    prog.modules = modules
    return prog


# ---------------------------------------------------------------------------
# resolution
# ---------------------------------------------------------------------------
def _err(node, code, msg, fix="", stage="type"):
    raise PruvError(stage, node.line, node.col, code, msg, fix, file=getattr(node, "file", None))


def resolve(prog):
    if not prog.explicit or getattr(prog, "_g4_resolved", False):
        return
    prog._g4_resolved = True
    _Resolver(prog).run()


def decl_fields(d):
    if isinstance(d, StructDecl):
        return list(d.fields)
    return [fd for v in d.variants for fd in v.fields]


class _Resolver:
    def __init__(s, prog):
        s.prog = prog
        s.by_path = {(m.pkg, m.path): m for m in prog.modules}
        s.root = s.by_path[(None, ())]
        s.m = s.root
        s.old_owner = s.new_owner = None
        s.type_by_id = {}

    @staticmethod
    def ident(path, name, pkg=None):
        if pkg is not None:
            return "::".join(("@" + pkg,) + tuple(path) + (name,))
        return "::".join(("crate",) + tuple(path) + (name,))

    def run(s):
        from check import BUILTIN_FNS, CONTRACT_ONLY, RESERVED_TYPE_NAMES, TYPE_NAME_RE
        from foreign import CATALOG_NAMES
        prog = s.prog
        # 1. declaration names per module (functions, then types, like the legacy checker), module declarations
        for m in prog.modules:
            for f in [i for i in m.items if isinstance(i, Function)]:
                if f.name in m.funcs:
                    _err(f, "dup_fn", "function %s defined twice" % s.ident(m.path, f.name, m.pkg))
                if f.name in BUILTIN_FNS or f.name in CONTRACT_ONLY or f.name in CATALOG_NAMES:
                    _err(f, "reserved", "%s is a builtin name" % f.name, "rename the function")
                m.funcs[f.name] = f
            for d in [i for i in m.items if isinstance(i, TypeDecl)]:
                if not TYPE_NAME_RE.match(d.name):
                    _err(d, "type_name", "type name %s must start with an upper-case letter" % d.name,
                         "rename it, e.g. %s" % (d.name[:1].upper() + d.name[1:]))
                if d.name in RESERVED_TYPE_NAMES:
                    _err(d, "reserved", "%s is a builtin name" % d.name, "rename the type")
                if d.name in m.types:
                    _err(d, "dup_type", "type %s defined twice" % s.ident(m.path, d.name, m.pkg))
                if d.name in m.funcs:
                    _err(d, "name_collision", "%s is both a type and a function" % s.ident(m.path, d.name, m.pkg),
                         "rename the type or the function")
                m.types[d.name] = d
            seen = set()
            for md in m.mods:
                if md.name in seen:
                    _err(md, "duplicate_module", "module %s is declared twice in %s" % (md.name, disp(m.path, m.pkg)),
                         "remove the second `mod %s;`" % md.name)
                seen.add(md.name)
                if md.name in m.funcs:
                    _err(md, "name_collision", "%s is both a module and a function in %s" % (md.name, disp(m.path, m.pkg)),
                         "rename the module or the function")
        # 1b. a type parameter may not carry the name of a type of its module (builtin names: the checker)
        for it in prog.items:
            m = s.by_path[(it.pkg, it.mod_path)]
            for tp in it.tparams:
                if tp.name in m.types:
                    _err(tp, "type_parameter", "type parameter %s shadows a declared or built-in type" % tp.name,
                         "choose a name that is not a type, e.g. T")
        for m in prog.modules:
            for name, d in m.types.items():
                s.type_by_id[s.ident(m.path, name, m.pkg)] = d
        # 2. resolve every reference (items in program order); then rename the items to their identities
        for it in prog.items:
            s.m = s.by_path[(it.pkg, it.mod_path)]
            if isinstance(it, Function):
                s.fn(it)
            else:
                s.tdecl(it)
        # 3. public interfaces, 4. root main and host entries.  One visited set for the whole program: the first leak
        # aborts the check, so every public declaration visited by an earlier item has a closure without private
        # declarations and need not be walked again.
        s.iface_seen = set()
        s.fn_by_id = {it.name: it for it in prog.items if isinstance(it, Function)}   # P06: contract calls
        for it in prog.items:
            if it.pub:
                s.interface(it)
        s.entries()
        s.module_edges()

    def module_edges(s):
        """PR1: Module.deps = the other modules whose items an item of the module mentions, read right after
        resolution (before check/refine expansion erases call names).  Every string in the resolved AST that is the
        identity of an item (or a type-parameter owner of one) counts, so calls, reveals, types, constructors,
        variants, struct literals, type arguments and refinement bases are all edges.  Over-approximation is safe:
        an extra edge only makes a module proof record depend on more files."""
        prog = s.prog
        mod_ix = {id(m): i for i, m in enumerate(prog.modules)}
        owner = {it.name: mod_ix[id(s.by_path[(it.pkg, it.mod_path)])] for it in prog.items}
        # The same walk also hashes the resolved items (Module.rdigest): a deterministic serialization of every
        # node in walk order (class name, strings, numbers, container shape).  The file bytes alone do not fix what
        # each path resolves to (mount aliases and package maps come from other files), the resolved identities at
        # each reference point do.
        for m in prog.modules:
            mi = mod_ix[id(m)]
            h = hashlib.sha256()
            seen, stack = {}, list(reversed(m.items))
            while stack:
                n = stack.pop()
                if isinstance(n, str):
                    h.update(b"s%d:" % len(n.encode("utf-8")) + n.encode("utf-8"))
                    for key in (n, n.split(":", 1)[1] if n.startswith(("fn:", "type:")) else None):
                        j = owner.get(key)
                        if j is not None and j != mi:
                            m.deps.add(j)
                    continue
                if n is None or isinstance(n, (int, float, bool, bytes)):
                    h.update(("v%s:%r;" % (type(n).__name__, n)).encode("utf-8"))
                    continue
                if isinstance(n, Module):
                    h.update(("M%s:%s;" % (n.pkg, "::".join(n.path))).encode("utf-8"))
                    continue
                if id(n) in seen:
                    h.update(b"r%d;" % seen[id(n)])
                    continue
                seen[id(n)] = len(seen)
                if isinstance(n, (set, frozenset)):
                    kids = sorted(n, key=repr)
                elif isinstance(n, (list, tuple)):
                    kids = list(n)
                elif isinstance(n, dict):
                    kids = [x for kv in n.items() for x in kv]
                elif isinstance(n, Type):
                    kids = [n.name, n.args, n.owner]
                elif hasattr(n, "__dict__"):
                    kids = [x for kv in vars(n).items() for x in kv]
                else:
                    kids = [repr(n)]
                h.update(("(%s %d;" % (type(n).__name__, len(kids))).encode("utf-8"))
                stack.extend(reversed(kids))
            m.rdigest = h.hexdigest()

    # ---- paths ----
    def walk(s, path):
        """crate::a::b::x -> (innermost module reached by module segments, remaining segments)"""
        segs = path.split("::")[1:]
        P = s.m.pkg
        m = s.by_path[(P, ())]           # `crate` = the root of the package of the module being resolved
        i = 0
        while i < len(segs) - 1 and segs[i] in m.children:
            ch = m.children[segs[i]]
            if ch.pkg != m.pkg and m.pkg != P:
                break                    # a package never sees its dependencies' dependencies
            m = ch
            i += 1
        return m, segs[i:]

    def access(s, d, m, node, path):
        if not d.pub and m is not s.m:
            _err(node, "private_item", "%s is private to module %s" % (path, disp(m.path, m.pkg)),
                 "mark it `pub` in %s, or call it through a public item of that module" % m.file)

    def unknown_module(s, m, rem, node, path):
        _err(node, "unknown_module", "unknown module %s in %s (path %s)" % (rem[0], disp(m.path, m.pkg), path),
             "declare it with `mod %s;` in %s, or correct the path" % (rem[0], m.file))

    def type_item(s, path, node):
        m, rem = s.walk(path)
        if len(rem) > 1:
            s.unknown_module(m, rem, node, path)
        d = m.types.get(rem[0])
        if d is None:
            if rem[0] in m.children:
                _err(node, "unknown_type", "%s is a module, not a type" % path, "name a struct or enum of the module")
            _err(node, "unknown_type", "unknown type %s" % path,
                 "declare it with `struct`/`enum` in %s" % m.file)
        s.access(d, m, node, path)
        return s.ident(m.path, rem[0], m.pkg)

    def local_or_path_type(s, name, node):
        """a nominal name as written -> identity (qualified: resolved; unqualified: the current module only;
        otherwise unchanged: builtin/catalog spelling or an unknown name for the checker)"""
        if name.startswith("crate::"):
            return s.type_item(name, node)
        if name in s.m.types:
            return s.ident(s.m.path, name, s.m.pkg)
        return name

    def fn_item(s, m, rem, node, path):
        if len(rem) != 1:
            s.unknown_module(m, rem, node, path)
        f = m.funcs.get(rem[0])
        if f is None:
            if rem[0] in m.children:
                _err(node, "unknown_fn", "%s is a module, not a function" % path, "name a function of the module")
            _err(node, "unknown_fn", "unknown function %s" % path, "declare it in %s" % m.file)
        s.access(f, m, node, path)
        return s.ident(m.path, rem[0], m.pkg)

    # ---- types ----
    def rtype(s, ty, node):
        if ty.owner is not None:
            if ty.owner == s.old_owner:
                return Type(ty.name, ty.args, s.new_owner, ty.index)
            return ty
        if not ty.args and ty.name in BUILTIN_TYPE_NAMES:
            return ty
        args = tuple(s.rtype(a, node) for a in ty.args)
        name = ty.name if ty.name in BUILTIN_TYPE_NAMES else s.local_or_path_type(ty.name, node)
        if name == ty.name and args == ty.args:
            return ty
        return Type(name, args)

    def rrefine(s, r):
        if r is not None and r.base not in BUILTIN_TYPE_NAMES:
            r.base = s.local_or_path_type(r.base, r)

    # ---- items ----
    def own(s, it, kind_owner):
        s.old_owner = kind_owner(it.src_name)
        s.new_owner = kind_owner(s.ident(it.mod_path, it.src_name, it.pkg))

    def tdecl(s, d):
        s.own(d, decl_owner)
        for fd in decl_fields(d):
            fd.type = s.rtype(fd.type, fd)
        d.name = s.ident(d.mod_path, d.src_name, d.pkg)

    def fn(s, f):
        s.own(f, fn_owner)
        for p in f.params:
            p.type = s.rtype(p.type, f)
            s.rrefine(p.refine)
        f.ret = s.rtype(f.ret, f)
        s.rrefine(f.ret_refine)
        f.requires = [s.rexpr(e) for e in f.requires]
        f.ensures = [s.rexpr(e) for e in f.ensures]
        if f.decreases is not None:
            f.decreases = s.rexpr(f.decreases)
        if f.cost is not None:
            f.cost = s.rexpr(f.cost)
        s.rstmts(f.body)
        f.name = s.ident(f.mod_path, f.src_name, f.pkg)

    # ---- statements ----
    def rstmts(s, stmts):
        for st in stmts:
            if isinstance(st, Let):
                st.type = s.rtype(st.type, st)
                s.rrefine(st.refine)
                st.value = s.rexpr(st.value)
            elif isinstance(st, (Assign, FieldAssign)):
                st.value = s.rexpr(st.value)
            elif isinstance(st, IndexAssign):
                st.index = s.rexpr(st.index)
                st.value = s.rexpr(st.value)
            elif isinstance(st, If):
                st.cond = s.rexpr(st.cond)
                s.rstmts(st.then)
                s.rstmts(st.els)
            elif isinstance(st, While):
                st.cond = s.rexpr(st.cond)
                st.invariants = [s.rexpr(e) for e in st.invariants]
                st.decreases = s.rexpr(st.decreases)
                s.rstmts(st.body)
            elif isinstance(st, ForMap):
                st.place = s.rexpr(st.place)
                st.invariants = [s.rexpr(e) for e in st.invariants]
                s.rstmts(st.body)
            elif isinstance(st, Return):
                if st.value is not None:
                    st.value = s.rexpr(st.value)
            elif isinstance(st, Assert):
                st.cond = s.rexpr(st.cond)
                if st.reveal is not None:      # P13o: the revealed name resolves like a call
                    st.reveal = s.rexpr(st.reveal)
            elif isinstance(st, ExprStmt):
                st.expr = s.rexpr(st.expr)
            elif isinstance(st, Par):      # G7 A0: every branch call resolves like an ordinary call
                st.calls = [s.rexpr(c) for c in st.calls]
            elif isinstance(st, Match):
                st.scrutinee = s.rexpr(st.scrutinee)
                for a in st.arms:
                    if a.enum_name is not None:
                        a.enum_name = s.local_or_path_type(a.enum_name, a)
                    s.rstmts(a.body)

    # ---- expressions ----
    def targs(s, e):
        if e.targs is not None:
            e.targs = [s.rtype(t, e) for t in e.targs]

    def rexpr(s, e):
        if isinstance(e, Call):
            if e.name.startswith("crate::"):
                m, rem = s.walk(e.name)
                if len(rem) == 2 and e.targs is None and rem[0] in m.types:
                    # crate::m::Enum::Variant(args): a variant constructor, not a call
                    if not e.args:
                        raise PruvError("parse", e.line, e.col, "syntax", "empty parentheses after a variant",
                                        "write `%s::%s` for a variant without fields, or pass its fields as arguments"
                                        % (e.name.rsplit("::", 1)[0], rem[1]), file=getattr(e, "file", None))
                    d = m.types[rem[0]]
                    s.access(d, m, e, "::".join([disp(m.path, m.pkg), rem[0]]))
                    new = EnumCtor(e.line, e.col, s.ident(m.path, rem[0], m.pkg), rem[1], e.args)
                    new.file = e.file
                    new.args = [s.rexpr(a) for a in new.args]
                    return new
                e.name = s.fn_item(m, rem, e, e.name)
            elif e.name in s.m.funcs:
                e.name = s.ident(s.m.path, e.name, s.m.pkg)
            s.targs(e)
            e.args = [s.rexpr(a) for a in e.args]
            return e
        if isinstance(e, EnumCtor):
            e.enum_name = s.local_or_path_type(e.enum_name, e)
            s.targs(e)
            e.args = [s.rexpr(a) for a in e.args]
            return e
        if isinstance(e, StructLit):
            e.name = s.local_or_path_type(e.name, e)
            s.targs(e)
            for fi in e.fields:
                fi.value = s.rexpr(fi.value)
            return e
        if isinstance(e, (IsVariant, VariantField)):
            e.enum_name = s.local_or_path_type(e.enum_name, e)
            e.value = s.rexpr(e.value)
            return e
        if isinstance(e, Ctor):
            s.targs(e)
            if e.arg is not None:
                e.arg = s.rexpr(e.arg)
            return e
        if isinstance(e, Unary):
            e.operand = s.rexpr(e.operand)
        elif isinstance(e, Binary):
            e.left = s.rexpr(e.left)
            e.right = s.rexpr(e.right)
        elif isinstance(e, Index):
            e.array = s.rexpr(e.array)
            e.index = s.rexpr(e.index)
        elif isinstance(e, Quant):
            e.lo = s.rexpr(e.lo)
            e.hi = s.rexpr(e.hi)
            e.body = s.rexpr(e.body)
            e.triggers = [s.rexpr(t) for t in e.triggers]
        elif isinstance(e, FieldAccess):
            e.base = s.rexpr(e.base)
        elif isinstance(e, InoutArg):     # G5 A0
            e.expr = s.rexpr(e.expr)
        return e

    # ---- public interfaces ----
    def private_in(s, names, seen):
        """G4 A1 (spec 5): the first private declaration reachable from the nominal names (identities): the names
        themselves, then, transitively, the field / payload types of every public declaration reached.  seen is the
        visited set shared by the whole program (cycles terminate).  Returns (private identity, via) or None, where via
        is the dependency chain "P.field" leading to it ("" for a direct mention)."""
        for n in names:
            d = s.type_by_id.get(n)
            if d is None:
                continue
            if not d.pub:
                return n, ""
            if n in seen:
                continue
            seen.add(n)
            for fd in decl_fields(d):
                r = s.private_in(dependencies(fd.type), seen)
                if r is not None:
                    return r[0], "%s.%s%s" % (n, fd.name, " -> " + r[1] if r[1] else "")
        return None

    def leak(s, node, item, r, what):
        n, via = r
        _err(node, "public_interface", "public %s %s exposes the private type %s in its %s%s" % (
            "function" if isinstance(item, Function) else "type", item.name, n, what,
            " (through %s)" % via if via else ""),
             "make %s `pub`, or keep %s private" % (n, item.name))

    def iface_type(s, ty, node, item, what, seen):
        r = s.private_in(dependencies(ty), seen)
        if r is not None:
            s.leak(node, item, r, what)

    def iface_expr(s, e, item, seen):
        """full syntax of a contract expression, unreachable short-circuit operands included"""
        names = []
        if isinstance(e, StructLit):
            names.append(e.name)
        elif isinstance(e, (EnumCtor, IsVariant, VariantField)):
            names.append(e.enum_name)
        r = s.private_in(names, seen)
        if r is not None:
            s.leak(e, item, r, "contract")
        if isinstance(e, Call):     # P06: a public contract expands only public functions (their bodies are its meaning)
            f = s.fn_by_id.get(e.name)
            if f is not None and not f.pub:
                _err(e, "public_interface", "public function %s exposes the private function %s in its contract" % (
                    item.name, f.name), "make %s `pub`, or keep %s private" % (f.name, item.name))
        for t in getattr(e, "targs", None) or []:
            s.iface_type(t, e, item, "contract", seen)
        for k in _kids(e):
            s.iface_expr(k, item, seen)

    def interface(s, it):
        seen = s.iface_seen
        if isinstance(it, Function):
            for p in it.params:
                s.iface_type(p.type, it, it, "signature", seen)
            s.iface_type(it.ret, it, it, "signature", seen)
            for e in it.requires + it.ensures + ([it.decreases] if it.decreases is not None else []) + ([it.cost] if it.cost is not None else []):
                s.iface_expr(e, it, seen)
        else:
            for fd in decl_fields(it):
                s.iface_type(fd.type, fd, it, "field %s" % fd.name, seen)

    # ---- entries ----
    def entries(s):
        prog = s.prog
        out = []
        for f in prog.functions:
            if f.mod_path != () or f.pkg is not None:
                continue
            if f.src_name == "main":
                if not f.pub:
                    _err(f, "entry_main", "the root main must be `pub fn main() -> ()`", "write `pub fn main() -> () { .. }`")
                if f.tparams:
                    _err(f, "entry_main", "the root main cannot declare type parameters",
                         "make main a plain function; call generic functions from it with explicit type arguments")
                if f.params:
                    _err(f, "entry_main", "the root main takes no parameters", "remove the parameters of main")
                if f.ret != UNIT:
                    _err(f, "entry_main", "the root main must return ()", "write `pub fn main() -> ()`")
                prog.main = f.name
            if f.pub and not f.tparams and f.foreign is None and not f.pred:   # R03L: a pred is never a host entry
                f.host_entry = True
                out.append(f.name)
        prog.entries = out
