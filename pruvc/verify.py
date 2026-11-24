"""Pruv verifier: symbolic execution + z3. Proves bounds, overflow, contracts, loops."""
import time
import z3
import proof_cache
from vast import *
from typeinfo import (TypeTable, is_array, array_elem, array_of, bind_args, substitute, decl_owner, format_type,
                      is_collection, field_place)
from foreign import CATALOG, CATALOG_NAMES, FILE, FILE_OBSERVERS, PROCESS, PROCESS_ARGS, SOCKET, LISTENER, LOOP, MUTABLE_COLLECTION_MATCH
from refine import _stmt_inout
from check import array_resize_params

I64_MIN, I64_MAX = -2 ** 63, 2 ** 63 - 1
MAX_LEN = 2 ** 40
TIMEOUT_MS = 8000

_o = z3.Datatype("Opt"); _o.declare("some", ("sval", z3.IntSort())); _o.declare("none")
_r = z3.Datatype("Res"); _r.declare("ok", ("oval", z3.IntSort())); _r.declare("err", ("eval", z3.IntSort()))
Opt, Res = z3.CreateDatatypes(_o, _r)


class _OptOps:
    """the z3 operations of one Option datatype (legacy Opt, or a G3 instance Option<X>)"""
    __slots__ = ("some", "none", "is_some", "is_none", "sval")

    def __init__(s, some, none, is_some, is_none, sval):
        s.some, s.none, s.is_some, s.is_none, s.sval = some, none, is_some, is_none, sval


class _ResOps:
    """the z3 operations of one Result datatype (legacy Res, or a G3 instance Result<X>)"""
    __slots__ = ("ok", "err", "is_ok", "is_err", "oval", "eval")

    def __init__(s, ok, err, is_ok, is_err, oval, eval_):
        s.ok, s.err, s.is_ok, s.is_err, s.oval, s.eval = ok, err, is_ok, is_err, oval, eval_


# the legacy datatypes go through the very same z3 functions as before, so G3-free SMT text is unchanged
_OPT = _OptOps(Opt.some, Opt.none, Opt.is_some, Opt.is_none, Opt.sval)
_RES = _ResOps(Res.ok, Res.err, Res.is_ok, Res.is_err, Res.oval, Res.eval)


def arr_type(elem):
    """array Type holding elem (legacy constants for i64/u8/bool, a structured [T] for an abstract element)"""
    t = array_of(elem)
    return t if t is not None else mk_array(elem)

_MODEL_SEQ = [0]


def query_status(result, reason):
    """map a z3 check result (+ reason_unknown) to the trace status vocabulary"""
    if result == z3.unsat:
        return "proved"
    if result == z3.sat:
        return "not_proved"
    if reason in ("timeout", "canceled"):
        return "timeout"
    return "resource_unknown"


class QueryConfig:
    """how one solver query is configured; rlimit/seed None = option not set (no text line either)"""

    def __init__(s, phase="verification", round=None, timeout_ms=TIMEOUT_MS, rlimit=None, seed=None):
        s.phase, s.round, s.timeout_ms, s.rlimit, s.seed = phase, round, timeout_ms, rlimit, seed


class BudgetExhausted(Exception):
    """STAGE1 B2: the per-function exploration query budget is used up (infer.py catches it)"""


class InferenceTimeout(Exception):
    """STAGE1 B2: wall-clock safety limit (function start + 5 s) during exploration, or an exploration query hitting
    z3's own 500 ms limit (reason exactly "timeout"); rlimit exhaustion ("canceled" etc.) only drops the candidate
    (infer.py catches it)"""


class Probe:
    """STAGE1 B2 exploration state of one function (shared by all its rounds).
    queries: attempts made so far (budget counter); failed: this round's failed candidates
    {cand id: (obligation code, attempt status)} in the order they failed."""

    def __init__(s, max_queries, deadline):
        s.max_queries, s.deadline = max_queries, deadline
        s.queries = 0
        s.failed = {}

    def begin_round(s):
        s.failed = {}

    def tick(s):
        """called before every attempt: budget first, then the wall clock, then count the attempt"""
        if s.queries >= s.max_queries:
            raise BudgetExhausted()
        if time.monotonic() > s.deadline:
            raise InferenceTimeout()
        s.queries += 1

    def fail(s, cand, code, status):
        s.failed[cand] = (code, status)


def walk_loops(stmts):
    """(While, block, index in block) for every loop, preorder: statements in order, a While before its body,
    If then -> els, Match arms in order (STAGE1 B2 loop numbering = position in this sequence)"""
    for i, st in enumerate(stmts):
        if isinstance(st, While):
            yield st, stmts, i
            for x in walk_loops(st.body):
                yield x
        elif isinstance(st, ForMap):
            yield from walk_loops(st.body)
        elif isinstance(st, If):
            for x in walk_loops(st.then):
                yield x
            for x in walk_loops(st.els):
                yield x
        elif isinstance(st, Match):
            for a in st.arms:
                for x in walk_loops(a.body):
                    yield x


class TraceRecorder:
    """official query-recording boundary: one entry per obligation, one attempt per solver check"""

    def __init__(s):
        s.obligations = []

    def begin(s, fn, node, code, skipped, phase, round_, cand, file=None):
        entry = {"seq": len(s.obligations), "fn": fn, "line": node.line, "col": node.col, "code": code,
                 "attempts": [], "phase": phase, "round": round_, "cand": cand}
        if file is not None:        # G4 A1: explicit mode only; legacy entries keep their exact key set
            entry["file"] = file
        if skipped:
            entry["skipped"] = True
        s.obligations.append(entry)
        return len(s.obligations) - 1

    def attempt(s, idx, lemmas, smt2, result, status, timeout_ms, rlimit, seed, reason_unknown, cached=False):
        a = {
            "lemmas": lemmas, "smt2": smt2, "result": result, "status": status, "timeout_ms": timeout_ms,
            "rlimit": rlimit, "seed": seed, "reason_unknown": reason_unknown}
        if cached:      # G4b-3: only a proof-cache hit carries the key
            a["cached"] = True
        s.obligations[idx]["attempts"].append(a)


VEC_RESIZERS = ("vec_push", "vec_pop", "vec_clear", "vec_extend_from_slice")


def _vec_resize_params(prog):
    """Inout vector parameters whose length can change through vec_clear or forwarding."""
    from foreign_shim import _walk

    resizes = set()
    forwarded = {}
    for fn in prog.functions:
        params = {p.name: i for i, p in enumerate(fn.params) if p.mode == "inout" and p.type.name == "vec"}
        if not params:
            continue

        def visit(node):
            if not isinstance(node, Call):
                return
            for i, arg in enumerate(node.args):
                if isinstance(arg, InoutArg) and isinstance(arg.expr, Var) and arg.expr.name in params:
                    source = (fn.name, params[arg.expr.name])
                    if node.name in VEC_RESIZERS:
                        resizes.add(source)
                    else:
                        forwarded.setdefault((node.name, i), set()).add(source)

        _walk(fn.body, visit)
    pending = list(resizes)
    while pending:
        for source in forwarded.get(pending.pop(), ()):
            if source not in resizes:
                resizes.add(source)
                pending.append(source)
    return resizes


def _struct_array_paths(tt, ty, path=()):
    for fd in tt.fields(ty):
        field = path + (fd.name,)
        if is_array(fd.type):
            yield field
        elif tt.is_struct(fd.type):
            yield from _struct_array_paths(tt, fd.type, field)


def _struct_len_frames(prog, tt, resizes):
    """Per inout struct parameter, find the array leaves whose lengths survive a call."""
    from foreign_shim import _walk

    paths = {(f.name, i): set(_struct_array_paths(tt, p.type))
             for f in prog.functions for i, p in enumerate(f.params)
             if p.mode == "inout" and tt.is_struct(p.type)}
    funcs = {f.name: f for f in prog.functions}
    changed = {key: set() for key in paths}
    forwarded = []

    def mark(key, prefix):
        changed[key].update(path for path in paths[key] if path[:len(prefix)] == prefix)

    for f in prog.functions:
        params = {p.name: (f.name, i) for i, p in enumerate(f.params) if paths.get((f.name, i))}
        if not params:
            continue

        def visit(node):
            if isinstance(node, Assign) and node.name in params:
                mark(params[node.name], ())
            elif isinstance(node, (FieldAssign, Move)) and node.name in params:
                mark(params[node.name], tuple(node.fields))
            elif isinstance(node, Call):
                callee = funcs.get(node.name)
                for i, arg in enumerate(node.args):
                    if not isinstance(arg, InoutArg):
                        continue
                    root, path = field_place(arg.expr)
                    if not isinstance(root, Var) or root.name not in params:
                        continue
                    source, target = params[root.name], (node.name, i)
                    if callee is not None and callee.foreign is None:
                        if target in paths:
                            forwarded.append((source, path, target))
                        elif target in resizes:
                            mark(source, path)
                    elif node.name not in ("copy_range", "copy_within", "store_be", "store_le", "arr_reserve"):
                        mark(source, path)

        _walk(f.body, visit)
    again = True
    while again:
        again = False
        for source, prefix, target in forwarded:
            additional = ({prefix + path for path in changed[target]} & paths[source]) - changed[source]
            if additional:
                changed[source].update(additional)
                again = True
    return {key: tuple(sorted(paths[key] - changes)) for key, changes in changed.items()}


class Model:
    """Per-program z3 model of the user structs/enums (G1). Built once in verify_program.
    struct[name] = (constructor, [accessor per field])
    enum[name][variant] = (constructor, recognizer, [accessor per field])
    Every variant has its own constructor, so a value has no inactive payload."""

    def __init__(s, prog):
        s.tt = TypeTable(prog)
        s.vec_resize_params = _vec_resize_params(prog) | array_resize_params(prog, length_only=True)
        s.struct_len_frames = _struct_len_frames(prog, s.tt, s.vec_resize_params)
        s.sorts = {}
        s.struct = {}
        s.enum = {}
        s.by_sort = {Opt.name(): OPTION, Res.name(): RESULT}
        _MODEL_SEQ[0] += 1
        pre = "m%d" % _MODEL_SEQ[0]
        # G3 A2: generic declarations are instantiated lazily, one datatype per complete structural type
        s.pre = pre
        s.fn_index = {f.name: i for i, f in enumerate(prog.functions)}
        s.gcount = 0
        s.ginst = {}        # Type (Option<X>, Result<X>, D<..>) -> z3 sort
        s.gopt, s.gres = {}, {}
        s.gstruct, s.genum = {}, {}
        s.abstract = {}     # (owner, index) -> uninterpreted sort of a function type parameter
        s.abs_name = {}     # sort name -> type-parameter spelling (counterexample rendering)
        s.short = {}        # constructor name of a generic Option/Result instance -> source name
        # G9 A1: recursive nominal type SCCs (one simultaneous datatype each), buffer-field datatypes, and the lazily
        # built recursive wf/size definitions. All of it is empty/unused for a program without a G9 feature.
        meta = getattr(prog, "g9", None)
        s.g9 = meta is not None
        s.scc_of = {}       # recursive type name -> member names of its SCC (declaration order)
        s.recrel = set()    # names of non-generic types that are recursive or (transitively) contain a recursive type
        s.bufs = {}         # "int" | "bool" -> (sort, constructor, length accessor, array accessor)
        s.buf_names = set()
        s.vec_handles = {}
        s.vec_sums = {}
        s.maps = {}
        s.strkey = None     # declared on first String-key use so other programs keep their solver input
        s.size_f, s.wf_f, s.fixed_memo = {}, {}, {}
        if meta is not None:
            pindex = {}
            for i, d in enumerate(prog.types):
                pindex.setdefault(d.name, i)
            for names in meta["type_sccs"]:
                mem = sorted(names, key=lambda n: pindex[n])
                for n in mem:
                    s.scc_of[n] = mem
            s.recrel = set(s.scc_of)
            changed = bool(s.recrel)
            while changed:
                changed = False
                for d in s.tt.decls.values():
                    if d.name not in s.recrel and not d.tparams and any(n in s.recrel for n in s.tt._deps(d)):
                        s.recrel.add(d.name)
                        changed = True
        s.idx_of = {d.name: i for i, d in enumerate(s.tt.order)}
        s.done = set()
        for d in s.tt.order:
            s._build(d.name)

    def _build(s, name):
        """create the datatype of `name` (its whole recursive SCC at once), dependencies first. For a program
        without recursive types this visits the declarations in tt.order exactly like the old loop."""
        if name in s.done:
            return
        d = s.tt.decls[name]
        group = s.scc_of.get(name) or [name]
        for n in group:
            s.done.add(n)
        if d.tparams:
            return      # generic declarations are instantiated lazily
        members = [s.tt.decls[n] for n in group]
        for mdecl in members:
            for dep in s.tt._deps(mdecl):
                if dep not in group:
                    s._build(dep)
        s._make_group(members)

    def _make_group(s, members):
        gdt = {}
        dts = []
        for d in members:
            tag = "%st%d!" % (s.pre, s.idx_of[d.name])
            dt = z3.Datatype(tag + d.name)
            gdt[d.name] = dt
            dts.append((d, tag, dt))

        def fsort(ft):
            # a field of a member type refers to the datatype under construction (z3 simultaneous declaration)
            if ft.owner is None and not ft.args and ft.name in gdt:
                return gdt[ft.name]
            if ft.owner is None and ft.name == "vec" and ft.args[0].name in gdt:
                elem = ft.args[0]
                if elem not in s.vec_handles:
                    s.vec_handles[elem] = z3.DeclareSort("%sVecH_%s" % (s.pre, elem.name))
                return s.vec_handles[elem]
            return s.sort_of(ft)
        for d, tag, dt in dts:
            if isinstance(d, StructDecl):
                dt.declare(tag + d.name, *[(tag + d.name + "." + fd.name, fsort(fd.type)) for fd in d.fields])
            else:
                for v in d.variants:
                    cn = tag + d.name + "::" + v.name
                    dt.declare(cn, *[(cn + "." + fd.name, fsort(fd.type)) for fd in v.fields])
        srts = z3.CreateDatatypes(*[dt for _, _, dt in dts])
        for (d, tag, dt), srt in zip(dts, srts):
            if isinstance(d, StructDecl):
                s.struct[d.name] = (srt.constructor(0), [srt.accessor(0, j) for j in range(len(d.fields))])
            else:
                s.enum[d.name] = {}
                for i, v in enumerate(d.variants):
                    s.enum[d.name][v.name] = (srt.constructor(i), srt.recognizer(i),
                                              [srt.accessor(i, j) for j in range(len(v.fields))])
            s.sorts[d.name] = srt
            s.by_sort[srt.name()] = Type(d.name)
        for d, tag, _ in dts:
            elem = Type(d.name)
            if elem not in s.vec_handles:
                continue
            handle = s.vec_handles[elem]
            integer = z3.IntSort()
            array = z3.ArraySort(integer, s.sort_of(elem))
            s.bufs[elem] = (handle, z3.Function(tag + "mk_vec", integer, array, handle),
                            z3.Function(tag + "len_vec", handle, integer),
                            z3.Function(tag + "arr_vec", handle, array))
            s.buf_names.add(handle.name())
            s.vec_sums[elem] = z3.Function(tag + "vsum", array, integer, integer)

    # ---------- G9 A1: buffer fields ----------
    @staticmethod
    def is_buf(ty):
        """A field stored as a length and an array of scalar elements."""
        return ty in (ARRAY, ARRAY_U8, ARRAY_BOOL, STR) or ty.name in ("vec", "string") and ty.owner is None

    @staticmethod
    def buf_elem(ty):
        if ty.name == "vec":
            return ty.args[0]
        return BOOL if ty == ARRAY_BOOL else U8 if ty in (ARRAY_U8, STR, Type("string")) else I64

    @staticmethod
    def buf_key(ty):
        elem = Model.buf_elem(ty)
        return "bool" if elem == BOOL else "int" if elem in (I64, U8) else elem

    def bufsort(s, ty):
        """datatype Buf(len: Int, arr: Array(Int, elem sort)); one per element sort (Int for i64/u8/str, Bool)"""
        key = s.buf_key(ty)
        ent = s.bufs.get(key)
        if ent is None:
            tag = "%sbuf!%s" % (s.pre, key)
            dt = z3.Datatype(tag)
            esort = s.sort_of(key) if isinstance(key, Type) else z3.BoolSort() if key == "bool" else z3.IntSort()
            dt.declare(tag + ".mk", (tag + ".len", z3.IntSort()), (tag + ".arr", z3.ArraySort(z3.IntSort(), esort)))
            srt = dt.create()
            ent = (srt, srt.constructor(0), srt.accessor(0, 0), srt.accessor(0, 1))
            s.bufs[key] = ent
            s.buf_names.add(srt.name())
        return ent[0]

    def map_key_sort(s, ty):
        return z3.IntSort() if ty == _STRING else s.sort_of(ty)

    def map_key(s, ty, key):
        if ty != _STRING:
            return key
        if s.strkey is None:
            s.strkey = z3.Function("strkey", z3.IntSort(),
                                   z3.ArraySort(z3.IntSort(), z3.IntSort()), z3.IntSort())
        return s.strkey(key.n, key.a)

    def utf8_valid(s, b):
        # LG26 (R-6b-3, G-LG25-1): the exact Rust `str::from_utf8` validity of the first b.n bytes of b
        return utf8_window(lambda t: z3.Select(b.a, t), b.n)

    def map_fields(s, ty):
        if ty not in s.maps:
            key, value = s.map_key_sort(ty.args[0]), s.sort_of(ty.args[1])
            tag = "%smap!%d" % (s.pre, len(s.maps))
            dt = z3.Datatype(tag)
            dt.declare(tag + ".mk", (tag + ".d", z3.ArraySort(key, z3.BoolSort())),
                       (tag + ".a", z3.ArraySort(key, value)), (tag + ".n", z3.IntSort()))
            sort = dt.create()
            s.maps[ty] = (sort, sort.constructor(0), *(sort.accessor(0, i) for i in range(3)))
        return s.maps[ty]

    def field_get(s, fty, term, name="field"):
        """adapt a stored field value to the verifier's own model: a buffer field becomes an Arr"""
        if fty.name == "hashmap" and fty.owner is None:
            _, _, d, a, n = s.map_fields(fty)
            return Map(d(term), a(term), n(term), name, *fty.args)
        if not s.is_buf(fty):
            return term
        s.bufsort(fty)
        ent = s.bufs[s.buf_key(fty)]
        return mk_arr(ent[3](term), ent[2](term), name, s.buf_elem(fty))

    def field_put(s, fty, val):
        """inverse of field_get: an Arr stored into a buffer field becomes a Buf value"""
        if isinstance(val, Map):
            return s.map_fields(fty)[1](val.d, val.a, val.n)
        if not (s.is_buf(fty) and isinstance(val, Arr)):
            return val
        s.bufsort(fty)
        return s.bufs[s.buf_key(fty)][1](val.n, val.a)

    # ---------- G9 A1: structural size and well-formedness of recursive values ----------
    def rel(s, ty):
        return ty.owner is None and not ty.args and ty.name in s.recrel

    def size_fn(s, name):
        """size_D as an uninterpreted z3 function (one-step equations come from facts(); all members of the SCC of D are declared first, then defined together)"""
        f = s.size_f.get(name)
        if f is not None:
            return f
        group = s.scc_of.get(name) or [name]
        for n in group:
            s.size_f[n] = z3.Function("%ssize!%s" % (s.pre, n), s.sorts[n], z3.IntSort())
        return s.size_f[name]

    def weight(s, fty, t):
        """size weight of a field value t of type fty (None = always 0): nominal values count their own size,
        Some/Ok/Err count their payload, and a recursive Vec element type determines the sum regardless of the owner."""
        if isinstance(t, Arr) and t.elem in s.vec_sums:
            return s.vec_sums[t.elem](t.a, t.n)
        if fty.owner is not None or fty in (I64, BOOL, OPTION, RESULT) or s.is_buf(fty) or is_collection(fty):
            return None
        if fty.name == "option":
            o = s.opt(fty)
            w = s.weight(fty.args[0], o.sval(t))
            return None if w is None else z3.If(o.is_some(t), w, 0)
        if fty.name == "result":
            o = s.res(fty)
            w = s.weight(fty.args[0], o.oval(t))
            if len(fty.args) == 2:
                err = s.weight(result_error(fty), o.eval(t))
                if err is not None:
                    return z3.If(o.is_ok(t), 0 if w is None else w, err)
            return None if w is None else z3.If(o.is_ok(t), w, 0)
        if s.tt.is_struct(fty) or s.tt.is_enum(fty):
            if not fty.args:
                return s.size_fn(fty.name)(t)
            k = s.fixed_size(fty)
            if k is not None:
                return z3.IntVal(k)          # value-independent size: a constant (no function over a huge finite sort)
            return s.size_inst(fty)(t)       # admitted acyclic G3 application: its own uninterpreted size
        return None

    def fixed_size(s, ty):
        """the size of every value of the user type ty when it does not depend on the value (a struct whose fields are
        all scalars, buffers, collections or such structs), else None. Memoized per structural type, so a nested
        generic application costs one computation per level. z3 is very slow on an uninterpreted function over a deep
        nested finite datatype sort, so such sizes are never left to a function."""
        memo = s.fixed_memo
        if ty in memo:
            return memo[ty]
        memo[ty] = None      # an in-progress (cyclic) type is never fixed
        k = None
        if s.tt.is_struct(ty):
            k = 1
            for _, fty in s.fields_of(ty):
                if fty.owner is not None or fty in (I64, BOOL) or s.is_buf(fty) or is_collection(fty):
                    continue
                if fty in (OPTION, RESULT) or fty.name in ("option", "result") or not s.tt.is_struct(fty):
                    k = None
                    break
                c = s.fixed_size(fty)
                if c is None:
                    k = None
                    break
                k += c
        memo[ty] = k
        return k

    def size_inst(s, ty):
        """size of an admitted acyclic generic application D<X..> as an uninterpreted z3 function per instance
        sort; its one-step equation comes from facts() on observed terms only (never a full value expansion)"""
        srt = s.instance(ty)
        f = s.size_f.get(srt.name())
        if f is None:
            f = s.size_f[srt.name()] = z3.Function("%ssize!%s" % (s.pre, srt.name()), srt, z3.IntSort())
        return f

    def size_sum(s, ty, variant, accs, t):
        total = z3.IntVal(1)
        for (fd, fty), acc in zip(s.fields_of(ty, variant), accs):
            w = s.weight(fty, s.field_get(fty, acc(t), fd.name))
            if w is not None:
                total = total + w
        return total

    def size_value(s, ty, t):
        """one step of size: 1 + the weights of the active constructor's fields"""
        if s.tt.is_struct(ty):
            return s.size_sum(ty, None, s.struct_info(ty)[1], t)
        infos = s.enum_info(ty)
        body = None
        for v in reversed(s.tt.decl(ty).variants):
            _, rec, accs = infos[v.name]
            val = s.size_sum(ty, v.name, accs, t)
            body = val if body is None else z3.If(rec(t), val, body)
        return body

    def wf_fn(s, name):
        """wf_D (recursive types only) as an uninterpreted z3 function over the SCC of D"""
        f = s.wf_f.get(name)
        if f is not None:
            return f
        group = s.scc_of[name]
        for n in group:
            s.wf_f[n] = z3.Function("%swf!%s" % (s.pre, n), s.sorts[n], z3.BoolSort())
        return s.wf_f[name]

    def facts(s, ty, t):
        """valid facts about one observed term t of the non-generic nominal type ty (see tests/g9/model_notes.md):
        size positivity, the one-step size equation, for a recursive type the one-step wf equation, and positivity
        of the size of every direct nominal child (so child-smaller follows from the equation)."""
        n = ty.name
        sf = s.size_fn(n)
        out = [sf(t) >= 1, sf(t) == s.size_value(ty, t)]
        if n in s.scc_of:
            body = _wf_step(s, ty, t)
            out.append(s.wf_fn(n)(t) == (z3.BoolVal(True) if body is None else body))
        d = s.tt.decls[n]
        if isinstance(d, StructDecl):
            groups = [(None, s.struct_info(ty)[1])]
        else:
            groups = [(v.name, s.enum_info(ty)[v.name][2]) for v in d.variants]
        for vname, accs in groups:
            for (fd, fty), acc in zip(s.fields_of(ty, vname), accs):
                if fty.owner is None and fty.name == "vec" and fty.args[0] in s.vec_sums:
                    out.append(s.weight(fty, s.field_get(fty, acc(t), fd.name)) >= 0)
                elif s.rel_nominal(fty):
                    out.append(s.size_fn(fty.name)(acc(t)) >= 1)
                elif fty.owner is None and fty.args and s.is_user(fty) and s.fixed_size(fty) is None:
                    out.append(s.size_inst(fty)(acc(t)) >= 1)
                elif fty.owner is None and fty.name == "result" and len(fty.args) == 2:
                    # E1: the Err payload of a Result<T, E> field counts in the weight, so its size is positive too
                    ety = result_error(fty)
                    ev = s.res(fty).eval(acc(t))
                    if s.rel_nominal(ety):
                        out.append(s.size_fn(ety.name)(ev) >= 1)
                    elif ety.owner is None and ety.args and s.is_user(ety) and s.fixed_size(ety) is None:
                        out.append(s.size_inst(ety)(ev) >= 1)
        return out

    def rel_nominal(s, ty):
        """a plain non-generic user struct/enum type"""
        return ty.owner is None and not ty.args and ty.name in s.tt.decls and not s.tt.decls[ty.name].tparams

    def is_user(s, ty):
        return ty.owner is None and ty.name in s.tt.decls

    def sort_of(s, ty):
        if ty == I64:
            return z3.IntSort()
        if ty == BOOL:
            return z3.BoolSort()
        if s.is_buf(ty):        # G9 A1: a buffer field of a struct/enum
            return s.bufsort(ty)
        if ty.name == "hashmap" and ty.owner is None:
            return s.map_fields(ty)[0]
        if ty == OPTION:
            return Opt
        if ty == RESULT:
            return Res
        if ty.owner is not None:
            return s.abstract_sort(ty)
        if not ty.args:
            return s.sorts[ty.name]
        return s.instance(ty)

    # ---------- G3 A2: abstract sorts and generic datatype instances ----------
    def abstract_sort(s, ty):
        """fresh uninterpreted sort g3!<function-declaration-index>!T<i> of a function type parameter"""
        key = (ty.owner, ty.index)
        srt = s.abstract.get(key)
        if srt is None:
            fname = ty.owner[3:] if ty.owner.startswith("fn:") else ty.owner
            name = "g3!%s!T%d" % (s.fn_index.get(fname, fname), ty.index)
            srt = z3.DeclareSort(name)
            s.abstract[key] = srt
            s.abs_name[name] = ty.name
        return srt

    def wf_trivial(s, ty):
        """G3: True when well_formed(ty, v) is None for every v (no i64 leaf and no Result error code inside).
        Cached per type, so the check is linear in the size of the type."""
        cache = s.__dict__.setdefault("_wf_triv", {})
        r = cache.get(ty)
        if r is not None:
            return r
        if ty == I64 or ty == OPTION or ty == RESULT or ty == _STRING:
            r = False
        elif ty.owner is not None:
            r = True
        elif ty.name == "option":
            r = s.wf_trivial(ty.args[0])
        elif ty.name == "result":
            r = len(ty.args) == 2 and all(s.wf_trivial(a) for a in ty.args)
        elif ty.owner is None and not ty.args and ty.name in s.scc_of:
            r = False       # G9 A1: a recursive type has the wf_D definition (never trivially true here)
        elif s.tt.is_struct(ty):
            r = all(s.wf_trivial(ft) for fd, ft in s.fields_of(ty))
        elif s.tt.is_enum(ty):
            r = all(s.wf_trivial(ft) for vd in s.tt.decl(ty).variants for fd, ft in s.fields_of(ty, vd.name))
        else:
            r = True
        cache[ty] = r
        return r

    def fields_of(s, ty, variant=None):
        """[(FieldDecl, field type)] with the type arguments of ty substituted into the declared field types"""
        d = s.tt.decl(ty)
        flds = s.tt.fields(ty, variant)
        if not d.tparams:
            return [(fd, fd.type) for fd in flds]
        m = bind_args(decl_owner(ty.name), len(d.tparams), ty.args)
        return [(fd, substitute(fd.type, m)) for fd in flds]

    def instance(s, ty):
        """datatype of Option<X> / Result<X> / D<X..>, built once per complete structural type"""
        srt = s.ginst.get(ty)
        if srt is not None:
            return srt
        s.gcount += 1
        tag = "%sg%d!" % (s.pre, s.gcount)
        dt = z3.Datatype(tag + format_type(ty))
        if ty.name == "option":
            ps = s.sort_of(ty.args[0])
            dt.declare(tag + "Some", (tag + "Some.val", ps))
            dt.declare(tag + "None")
            srt = dt.create()
            s.gopt[ty] = _OptOps(srt.constructor(0), srt.constructor(1)(), srt.recognizer(0), srt.recognizer(1),
                                 srt.accessor(0, 0))
            s.short[tag + "Some"], s.short[tag + "None"] = "Some", "None"
        elif ty.name == "result":
            ps = s.sort_of(ty.args[0])
            dt.declare(tag + "Ok", (tag + "Ok.val", ps))
            dt.declare(tag + "Err", (tag + "Err.val", s.sort_of(result_error(ty))))
            srt = dt.create()
            s.gres[ty] = _ResOps(srt.constructor(0), srt.constructor(1), srt.recognizer(0), srt.recognizer(1),
                                 srt.accessor(0, 0), srt.accessor(1, 0))
            s.short[tag + "Ok"], s.short[tag + "Err"] = "Ok", "Err"
        else:
            d = s.tt.decls[ty.name]
            if isinstance(d, StructDecl):
                fl = [(tag + d.name + "." + fd.name, s.sort_of(ft)) for fd, ft in s.fields_of(ty)]
                dt.declare(tag + d.name, *fl)
                srt = dt.create()
                s.gstruct[ty] = (srt.constructor(0), [srt.accessor(0, j) for j in range(len(fl))])
            else:
                for v in d.variants:
                    cn = tag + d.name + "::" + v.name
                    dt.declare(cn, *[(cn + "." + fd.name, s.sort_of(ft)) for fd, ft in s.fields_of(ty, v.name)])
                srt = dt.create()
                s.genum[ty] = {}
                for i, v in enumerate(d.variants):
                    s.genum[ty][v.name] = (srt.constructor(i), srt.recognizer(i),
                                           [srt.accessor(i, j) for j in range(len(v.fields))])
        s.ginst[ty] = srt
        s.by_sort[srt.name()] = ty
        return srt

    def opt(s, ty):
        """operations of the Option datatype of ty (None / Option<i64> = the legacy one)"""
        if ty is None or ty == OPTION:
            return _OPT
        s.instance(ty)
        return s.gopt[ty]

    def res(s, ty):
        if ty is None or ty == RESULT:
            return _RES
        s.instance(ty)
        return s.gres[ty]

    def struct_info(s, ty):
        """(constructor, [accessor per field]) of a struct type"""
        if not ty.args:
            return s.struct[ty.name]
        s.instance(ty)
        return s.gstruct[ty]

    def enum_info(s, ty):
        """{variant: (constructor, recognizer, [accessor per field])} of an enum type"""
        if not ty.args:
            return s.enum[ty.name]
        s.instance(ty)
        return s.genum[ty]

    def type_of(s, v):
        """Pruv Type of a z3 datatype value (Opt/Res/user), or None"""
        if isinstance(v, File):
            return FILE
        if isinstance(v, Process):
            return PROCESS
        if isinstance(v, PArgs):
            return PROCESS_ARGS
        if isinstance(v, Socket):
            return SOCKET
        if isinstance(v, Loop):
            return LOOP
        if isinstance(v, Listener):
            return LISTENER
        if isinstance(v, Map):      # G6 A1
            return Type("hashmap", (v.kty, v.vty))
        try:
            return s.by_sort.get(v.sort().name())
        except Exception:
            return None


def _conj(xs):
    xs = [x for x in xs if x is not None]
    if not xs:
        return None
    return xs[0] if len(xs) == 1 else z3.And(*xs)


def _imp(c, d):
    return None if d is None else z3.Implies(c, d)


_TAG_FNS = ("some_val", "ok_val", "err_val")
# P2: bit builtin name -> z3 bit-vector operator (same names as check.BIT_BUILTINS); later batches add entries
BIT_Z3 = {"band": lambda a, b: a & b, "bor": lambda a, b: a | b, "bxor": lambda a, b: a ^ b,
          "wrapping_shl": lambda a, b: a << b, "ushr": z3.LShR,
          "wrapping_add": lambda a, b: a + b, "wrapping_mul": lambda a, b: a * b}
BIT_OP = {"&": "band", "|": "bor", "^": "bxor"}


def signed_bv(word):
    u = z3.BV2Int(word)
    return z3.If(u < 2 ** 63, u, u - 2 ** 64)


def shift_value(e, a, k):
    """The signed shift result and, for left shift, its exact overflow condition."""
    left = e.op == "<<"
    c = e.right.value if isinstance(e.right, IntLit) else None
    if c is not None and 0 <= c < 64:
        if not left:
            return a / 2 ** c, None
        r = a * 2 ** c
        return r, in_range(r)
    A, K = z3.Int2BV(a, 64), z3.Int2BV(k, 64)
    if not left:
        return signed_bv(A >> K), None
    w = A << K
    return signed_bv(w), (w >> K) == A


def _partial_here(e, kind):
    """is e itself a partial operation of the requested kind (None=any, "variant", "other")"""
    if isinstance(e, VariantField):
        return kind in (None, "variant")
    if kind == "variant":
        return False
    if isinstance(e, Index):
        return True
    if isinstance(e, Binary):
        return e.op in ("/", "%", "<<", ">>") or e.op in BIT_OP
    if isinstance(e, Call):
        return (e.name in _TAG_FNS or e.name == "file_byte" or e.name in BIT_Z3 or e.name in ("load_be", "load_le")
                or e.name in _PROC_DOM    # G10 proc A1: the stream / index domains of the Process observers
                or e.name in _NET_DOM or e.name in _ASYNC_DOM)    # G10 net A1: the index domain of net_byte
    if isinstance(e, Quant):
        return e.kind in ("max", "min")
    return False


def _kids(e):
    if isinstance(e, Unary):
        return [e.operand]
    if isinstance(e, Binary):
        return [e.left, e.right]
    if isinstance(e, Index):
        return [e.array, e.index]
    if isinstance(e, Ctor):
        return [] if e.arg is None else [e.arg]
    if isinstance(e, Quant):
        return [e.lo, e.hi, e.body]
    if isinstance(e, Call):
        return list(e.args)
    if isinstance(e, StructLit):
        return [fi.value for fi in e.fields]
    if isinstance(e, EnumCtor):
        return list(e.args)
    if isinstance(e, FieldAccess):
        return [e.base]
    if isinstance(e, (IsVariant, VariantField)):
        return [e.value]
    return []


def _has_vf(e, kind=None):
    """does the contract expression contain a partial operation (variant_field lookup, or for kind None/"other":
    index, / %, some_val/ok_val/err_val, max/min), i.e. a source of a nontrivial domain"""
    if _partial_here(e, kind):
        return True
    return any(_has_vf(c, kind) for c in _kids(e))


def utf8_window(B, n):
    """LG26 (R-6b-3, G-LG25-1): W(B, n), the exact UTF-8 validity (Rust str::from_utf8) of a window of n bytes, B(i)
    being the Int term of byte i. One ForAll over the bound Int utf8!i; the disjunct order is shared with the Rust
    compiler. ASCII is implied by the first disjunct."""
    i = z3.Int("utf8!i")
    c = B(i)

    def cont(x):
        return z3.And(128 <= x, x <= 191)
    p = z3.Or(
        c < 128,
        z3.And(194 <= c, c <= 223, i + 1 < n, cont(B(i + 1))),
        z3.And(c == z3.IntVal(224), i + 2 < n, 160 <= B(i + 1), B(i + 1) <= 191, cont(B(i + 2))),
        z3.And(z3.Or(z3.And(225 <= c, c <= 236), c == z3.IntVal(238), c == z3.IntVal(239)), i + 2 < n, cont(B(i + 1)), cont(B(i + 2))),
        z3.And(c == z3.IntVal(237), i + 2 < n, 128 <= B(i + 1), B(i + 1) <= 159, cont(B(i + 2))),
        z3.And(c == z3.IntVal(240), i + 3 < n, 144 <= B(i + 1), B(i + 1) <= 191, cont(B(i + 2)), cont(B(i + 3))),
        z3.And(241 <= c, c <= 243, i + 3 < n, cont(B(i + 1)), cont(B(i + 2)), cont(B(i + 3))),
        z3.And(c == z3.IntVal(244), i + 3 < n, 128 <= B(i + 1), B(i + 1) <= 143, cont(B(i + 2)), cont(B(i + 3))),
        z3.And(cont(c), z3.Or(
            z3.And(i >= 1, 194 <= B(i - 1), B(i - 1) <= 244),
            z3.And(i >= 2, 224 <= B(i - 2), B(i - 2) <= 244, cont(B(i - 1))),
            z3.And(i >= 3, 240 <= B(i - 3), B(i - 3) <= 244, cont(B(i - 2)), cont(B(i - 1))))))
    return z3.ForAll([i], z3.Implies(z3.And(0 <= i, i < n), p))


def _kids_all(e):
    """LG26: the sub-expressions of e, like _kids but also through an inout argument marker"""
    if isinstance(e, InoutArg):
        return [e.expr]
    if isinstance(e, Quant):
        return [e.lo, e.hi, e.body] + list(e.triggers or [])
    return _kids(e)


def _stmt_exprs(st):
    """LG26: the expressions directly held by statement st (conditions, values, invariants, asserts)"""
    if isinstance(st, Let):
        return [st.value]
    if isinstance(st, Assign):
        return [st.value]
    if isinstance(st, IndexAssign):
        return [st.index, st.value]
    if isinstance(st, If):
        return [st.cond]
    if isinstance(st, While):
        return [st.cond] + list(st.invariants) + ([st.decreases] if st.decreases is not None else [])
    if isinstance(st, ForMap):
        return [st.place] + list(st.invariants)
    if isinstance(st, Return):
        return [] if st.value is None else [st.value]
    if isinstance(st, Assert):
        return [st.cond]
    if isinstance(st, ExprStmt):
        return [st.expr]
    if isinstance(st, Par):
        return list(st.calls)
    if isinstance(st, FieldAssign):
        return [st.value]
    if isinstance(st, Match):
        return [st.scrutinee]
    return []


def _stmt_blocks(st):
    if isinstance(st, (While, ForMap)):
        return [st.body]
    if isinstance(st, If):
        return [st.then, st.els]
    if isinstance(st, Match):
        return [a.body for a in st.arms]
    return []


def _expr_has(e, pred):
    return pred(e) or any(_expr_has(k, pred) for k in _kids_all(e))


def _block_has(stmts, pred):
    for st in stmts:
        if any(_expr_has(e, pred) for e in _stmt_exprs(st)):
            return True
        if any(_block_has(b, pred) for b in _stmt_blocks(st)):
            return True
    return False


def _utf8_call(e):
    """LG26 (R-6b-3, G-LG25-1): a call that mentions UTF-8 validity, or a [u8] subview passed to a str parameter"""
    if not isinstance(e, Call):
        return False
    if e.name in ("utf8_valid", "utf8_valid_span", "str_from_utf8"):
        return True
    return (e.name == "subview" and getattr(e, "ty", None) == STR and bool(e.args)
            and getattr(e.args[0], "ty", None) != STR)


def _utf8_pred_call(e):
    return isinstance(e, Call) and e.name in ("utf8_valid", "utf8_valid_span")


def utf8_aware(f, funcs):
    """LG26 (R-6b-3, G-LG25-1): is f a function whose str values may be assumed valid UTF-8? True when its own
    requires/ensures/body mention UTF-8 validity (see _utf8_call), or when it calls a user function whose requires or
    ensures contain utf8_valid / utf8_valid_span. Header decreases and cost count as own contracts; synthesized struct
    invariant contracts do not (the Rust compiler builds them at verification time). A purely syntactic walk; other functions get no new assumption."""
    own = list(f.requires) + list(f.ensures) + [e for e in (f.decreases, f.cost) if e is not None]
    if any(_expr_has(e, _utf8_call) for e in own) or _block_has(f.body, _utf8_call):
        return True

    def calls_aware(e):
        if not isinstance(e, Call) or e.name not in funcs:
            return False
        g = funcs[e.name]
        return any(_expr_has(r, _utf8_pred_call) for r in list(g.requires) + list(g.ensures))
    return any(_expr_has(e, calls_aware) for e in own) or _block_has(f.body, calls_aware)


class Arr:
    __slots__ = ("a", "n", "name", "elem")

    def __init__(s, a, n, name, elem):
        s.a, s.n, s.name, s.elem = a, n, name, elem


def mk_arr(a, n, name, elem):
    """The only place an Arr is built; elem is the element Type (I64 / U8 / BOOL). Pure: touches no counter."""
    return Arr(a, n, name, elem)


class Map:
    """G6 A1 (G6_SPEC 11.2): a HashMap<K, V> value: membership d: Array(Sort(K), Bool), payload a: Array(Sort(K),
    Sort(V)), size n. kty/vty are the Pruv key/value types."""
    __slots__ = ("d", "a", "n", "name", "kty", "vty")

    def __init__(s, d, a, n, name, kty, vty):
        s.d, s.a, s.n, s.name, s.kty, s.vty = d, a, n, name, kty, vty


class File:
    """G10: three independent Bool fields and a String-shaped byte buffer (size, map)."""
    __slots__ = ("ok", "readable", "writable", "buf")

    def __init__(s, ok, readable, writable, buf):
        s.ok, s.readable, s.writable, s.buf = ok, readable, writable, buf


class Process:
    """G10 proc A1 (G10_PROC_SPEC 4.1): the public state of a Process only. error: Int, stdin_open / stdout_eof /
    stderr_eof: Bool, out / err: the public stdout / stderr buffers as U8 Arr (size, byte map). The child, PID,
    descriptors, queues and reader timing are not modeled."""
    __slots__ = ("error", "stdin_open", "stdout_eof", "stderr_eof", "out", "err")

    def __init__(s, error, stdin_open, stdout_eof, stderr_eof, out, err):
        s.error, s.stdin_open, s.stdout_eof, s.stderr_eof, s.out, s.err = error, stdin_open, stdout_eof, stderr_eof, out, err


class PArgs:
    """G10 proc A1 (G10_PROC_SPEC 4.3): a ProcessArgs value is its argument count only."""
    __slots__ = ("argc",)

    def __init__(s, argc):
        s.argc = argc


class Socket:
    """G10 net A1 (G10_NET_SPEC 4.1): the public state of a Socket only. error: Int, write_open / eof: Bool, buf: the
    public buffer as a U8 Arr (length, byte map). The descriptor, peer, address and kernel buffers are not modeled."""
    __slots__ = ("error", "write_open", "eof", "buf")

    def __init__(s, error, write_open, eof, buf):
        s.error, s.write_open, s.eof, s.buf = error, write_open, eof, buf


def _wf_loop(p):
    i = z3.Int("ev.wf.slot")
    return z3.And(p.error >= 0,
                  p.error <= 2,
                  p.cap >= 1,
                  p.cap <= 4096,
                  p.count >= 0,
                  p.count <= p.cap,
                  p.last >= -1,
                  p.last < p.cap,
                  p.ev_slot >= -1,
                  p.ev_slot < p.cap,
                  p.ev_kind_ >= 0,
                  p.ev_kind_ <= 5,
                  (p.ev_slot == -1) == (p.ev_kind_ == 0),
                  p.buf.n >= 0,
                  p.buf.n <= MAX_LEN,
                  z3.Implies(p.last != -1, z3.Select(p.kind, p.last) != 0),
                  z3.Implies(p.error != 0, z3.And(p.last == -1, p.ev_slot == -1, p.buf.n == 0)), u8_dom(p.buf),
                  z3.ForAll([i], z3.Implies(z3.And(i >= 0, i < p.cap), z3.And(z3.Select(p.kind, i) >= 0, z3.Select(p.kind, i) <= 4, z3.Implies(z3.Or(z3.Select(p.eof, i), z3.Select(p.write_open, i)), z3.Or(z3.Select(p.kind, i) == 1, z3.Select(p.kind, i) == 3))))))


class Loop:
    """G10 async public snapshot. Slot arrays are not a count sum."""
    def __init__(self, **fields):
        self.__dict__.update(fields)


class Listener:
    """G10 net A1 (G10_NET_SPEC 4.1): {error: Int, port: Int}."""
    __slots__ = ("error", "port")

    def __init__(s, error, port):
        s.error, s.port = error, port


def _wf_socket(p):
    """G10 net A1 (G10_NET_SPEC 4.2): 0 <= error <= 6; buffer length in 0..M with bytes in 0..255; an open write side
    implies no error; an error means write side closed, empty buffer, EOF false. EOF and an empty buffer are not
    made equivalent."""
    return z3.And(p.error >= 0, p.error <= 6, p.buf.n >= 0, p.buf.n <= MAX_LEN, u8_dom(p.buf),
                  z3.Implies(p.write_open, p.error == 0),
                  z3.Implies(p.error != 0, z3.And(z3.Not(p.write_open), p.buf.n == 0, z3.Not(p.eof))))


def _wf_listener(l):
    """G10 net A1 (G10_NET_SPEC 4.2): 0 <= error <= 2; ok implies 1 <= port <= 65535; failed implies port == 0."""
    return z3.And(l.error >= 0, l.error <= 2,
                  z3.Implies(l.error == 0, z3.And(l.port >= 1, l.port <= 65535)),
                  z3.Implies(l.error != 0, l.port == 0))


def _net_keep_buf(r, p):
    """the public buffer snapshot (length, byte map) and EOF of r equal those of p"""
    return z3.And(r.buf.n == p.buf.n, r.buf.a == p.buf.a, r.eof == p.eof)


_COLL = ("vec", "hashmap", "string", "file")
_STRING = Type("string")
# G10 net A1: the observer with an index domain (net_byte), checked like the File byte observer
_ASYNC_DOM = ("ev_kind", "ev_eof", "ev_write_open", "ev_byte")
_NET_DOM = ("net_byte",)
# G10 proc A1: observers with a stream (and index) domain, checked like the File byte observer
_PROC_DOM = ("proc_eof", "proc_size", "proc_byte")


def _proc_stream_ok(sm):
    return z3.Or(sm == 1, sm == 2)


def _proc_size(p, sm):
    """proc_size(p, sm): stream 1 is stdout, any other (domain-checked) stream is stderr"""
    return z3.If(sm == 1, p.out.n, p.err.n)


def _proc_eof(p, sm):
    return z3.If(sm == 1, p.stdout_eof, p.stderr_eof)


def _proc_byte(p, sm, i):
    return z3.If(sm == 1, z3.Select(p.out.a, i), z3.Select(p.err.a, i))


def _proc_keep_out(r, p):
    """the stdout snapshot (size, byte map, EOF) of r equals that of p"""
    return z3.And(r.out.n == p.out.n, r.out.a == p.out.a, r.stdout_eof == p.stdout_eof)


def _proc_keep_err(r, p):
    return z3.And(r.err.n == p.err.n, r.err.a == p.err.a, r.stderr_eof == p.stderr_eof)


def _wf_proc(p):
    """G10 proc A1 (G10_PROC_SPEC 4.2): 0 <= error <= 6; buffer sizes in 0..M with bytes in 0..255; an open stdin implies
    no error; an error means stdin closed, both buffers empty, both EOFs false. EOF and an empty buffer are not
    made equivalent."""
    return z3.And(p.error >= 0, p.error <= 6,
                  p.out.n >= 0, p.out.n <= MAX_LEN, p.err.n >= 0, p.err.n <= MAX_LEN,
                  u8_dom(p.out), u8_dom(p.err),
                  z3.Implies(p.stdin_open, p.error == 0),
                  z3.Implies(p.error != 0, z3.And(z3.Not(p.stdin_open), p.out.n == 0, p.err.n == 0,
                                                  z3.Not(p.stdout_eof), z3.Not(p.stderr_eof))))


class State:
    def __init__(s, env, pc):
        s.env, s.pc = env, pc
        s.obs = frozenset()     # G9 A1: ids of the recursive-value facts already in pc (immutable, shared by forks)
        s.rev = frozenset()     # P13o: names of the opaque functions revealed on this path
        s.apps = ()             # P13o: (function, application term, argument values, guard, enclosing quantifier constants) met on this path, in order
        s.app_ids = frozenset()     # LG30 (G-LG29-1): (application id, guard ids) of every occurrence in apps

    def fork(s, extra=None):
        n = State(dict(s.env), s.pc + ([extra] if extra is not None else []))
        n.obs = s.obs
        n.rev, n.apps, n.app_ids = s.rev, s.apps, s.app_ids
        return n


def _wf_of(m, ty, v):
    """well_formed(ty, v): every active leaf is an i64. None when trivially true. m: the Model.
    G9 A1: a recursive nominal type is the application of its wf_D definition; a buffer field (v is an Arr
    adapted by Model.field_get) has 0 <= len <= 2^40 and, for u8/str, the byte domain."""
    if ty == I64:
        return in_range(v)
    if isinstance(v, Arr) and m.is_buf(ty):
        ln = z3.And(v.n >= 0, v.n <= MAX_LEN)
        return z3.And(ln, u8_dom(v)) if v.elem == U8 else ln
    if ty == FILE:
        return z3.And(v.buf.n >= 0, u8_dom(v.buf))
    if ty.owner is None and ty.name == "process":       # G10 proc A1 (4.2)
        return _wf_proc(v)
    if ty.owner is None and ty.name == "socket":        # G10 net A1 (4.2)
        return _wf_socket(v)
    if ty.owner is None and ty.name == "loop":
        return _wf_loop(v)
    if ty.owner is None and ty.name == "listener":      # G10 net A1 (4.2)
        return _wf_listener(v)
    if ty.owner is None and ty.name == "process_args":  # G10 proc A1 (4.3): 0 <= argc <= M
        return z3.And(v.argc >= 0, v.argc <= MAX_LEN)
    if ty.owner is None and ty.name in _COLL:     # G6 A1: 0 <= n <= MAX_LEN (and the u8 domain of a String)
        ln = z3.And(v.n >= 0, v.n <= MAX_LEN)
        return z3.And(ln, u8_dom(v)) if ty.name == "string" else ln
    if ty == OPTION:
        return z3.Implies(Opt.is_some(v), in_range(Opt.sval(v)))
    if ty == RESULT:
        return z3.And(z3.Implies(Res.is_ok(v), in_range(Res.oval(v))),
                      z3.Implies(Res.is_err(v), in_range(Res.eval(v))))
    if ty.owner is not None:        # G3: an abstract value has no well-formedness predicate
        return None
    if ty.args and m.wf_trivial(ty):
        # G3: no constrained leaf anywhere inside (e.g. Dup<Dup<T>>); create the datatype as the full walk would,
        # but skip the field recursion, which is exponential in the nesting depth
        if ty.name == "option":
            m.opt(ty)
        elif ty.name == "result":
            m.res(ty)
        elif m.tt.is_struct(ty):
            m.struct_info(ty)
        elif m.tt.is_enum(ty):
            for vd in m.tt.decl(ty).variants:
                m.enum_info(ty)[vd.name]
        return None
    if ty.name == "option":         # G3: Option<X>, X other than i64
        o = m.opt(ty)
        w = _wf_of(m, ty.args[0], o.sval(v))
        return None if w is None else z3.Implies(o.is_some(v), w)
    if ty.name == "result":
        o = m.res(ty)
        w = _wf_of(m, ty.args[0], o.oval(v))
        parts = [] if w is None else [z3.Implies(o.is_ok(v), w)]
        error = result_error(ty)
        err = _wf_of(m, error, m.field_get(error, o.eval(v)) if error == _STRING else o.eval(v))
        if err is not None:
            parts.append(z3.Implies(o.is_err(v), err))
        return _conj(parts)
    if not ty.args and ty.name in m.scc_of:     # G9 A1: recursive type: the definition, never a Python expansion
        return m.wf_fn(ty.name)(v)
    return _wf_step(m, ty, v)


def _wf_step(m, ty, v):
    """one structural step of well_formed for a struct/enum type (the fields are checked with _wf_of)"""
    if m.tt.is_struct(ty):
        accs = m.struct_info(ty)[1]
        return _conj([_wf_of(m, ft, m.field_get(ft, acc(v), fd.name))
                      for (fd, ft), acc in zip(m.fields_of(ty), accs)])
    if m.tt.is_enum(ty):
        parts = []
        for vd in m.tt.decl(ty).variants:
            _, rec, accs = m.enum_info(ty)[vd.name]
            w = _conj([_wf_of(m, ft, m.field_get(ft, acc(v), fd.name))
                       for (fd, ft), acc in zip(m.fields_of(ty, vd.name), accs)])
            if w is not None:
                parts.append(z3.Implies(rec(v), w))
        return _conj(parts)
    return None


def _expr_text(e):
    """source-like text of a contract expression (used to name measures in messages)"""
    if isinstance(e, IntLit):
        return str(e.value)
    if isinstance(e, BoolLit):
        return "true" if e.value else "false"
    if isinstance(e, Var):
        return e.name
    if isinstance(e, Unary):
        return e.op + _expr_text(e.operand)
    if isinstance(e, Binary):
        def side(x):
            return "(%s)" % _expr_text(x) if isinstance(x, Binary) else _expr_text(x)
        return "%s %s %s" % (side(e.left), e.op, side(e.right))
    if isinstance(e, Call):
        return "%s(%s)" % (e.name, ", ".join(_expr_text(a) for a in e.args))
    if isinstance(e, Index):
        return "%s[%s]" % (_expr_text(e.array), _expr_text(e.index))
    if isinstance(e, FieldAccess):
        return "%s.%s" % (_expr_text(e.base), e.field)
    if isinstance(e, Ctor):
        return e.name if e.arg is None else "%s(%s)" % (e.name, _expr_text(e.arg))
    if isinstance(e, Quant):
        return "%s(%s in %s..%s: %s)" % (e.kind, e.var, _expr_text(e.lo), _expr_text(e.hi), _expr_text(e.body))
    if isinstance(e, IsVariant):
        return "is_variant(%s, %s::%s)" % (_expr_text(e.value), e.enum_name, e.variant)
    if isinstance(e, VariantField):
        return "variant_field(%s, %s::%s, %s)" % (_expr_text(e.value), e.enum_name, e.variant, e.field)
    return "..."


def _opq_parts(args):
    """P13o: the argument encodings of an opaque application: an array is its contents and its length"""
    out = []
    for a in args:
        if isinstance(a, Arr):
            out += [a.a, a.n]
        elif isinstance(a, Map):
            out += [a.d, a.a, a.n]
        else:
            out.append(a)
    return out


def _arr_shape(t):
    """LG4: the shape of an aggregate body with every array-sorted subterm replaced by `A` (frame lemma pairing).
    A quantifier or other non-application keeps its full text, so it pairs only with an identical body."""
    if z3.is_array(t):
        return "A"
    if z3.is_app(t) and t.num_args() > 0:
        return "(" + t.decl().name() + " " + " ".join(_arr_shape(c) for c in t.children()) + ")"
    return t.sexpr()


def _occurs(t, c):
    """does z3 constant c appear in term t"""
    seen, todo = set(), [t]
    while todo:
        n = todo.pop()
        i = n.get_id()
        if i in seen:
            continue
        seen.add(i)
        if z3.is_const(n) and n.eq(c):
            return True
        if z3.is_quantifier(n):
            todo.append(n.body())
            todo.extend(_pattern_terms(n))    # R-6-3: a trigger may use an outer constant the body does not
        else:
            todo.extend(n.children())
    return False


def _pattern_terms(q):
    """every term of every pattern of quantifier q"""
    out = []
    for i in range(q.num_patterns()):
        p = q.pattern(i)
        out.extend(p.arg(j) for j in range(p.num_args()))
    return out


def in_range(x):
    return z3.And(x >= I64_MIN, x <= I64_MAX)


def u8_dom(arr):
    """G2 A2: every element of a U8 symbolic array is in 0..=255 (bound variable `elem!k`, fixed name, no counter)."""
    k = z3.Int("elem!k")
    A = arr.a
    return z3.ForAll([k], z3.Implies(z3.And(k >= 0, k < arr.n),
                                     z3.And(z3.Select(A, k) >= 0, z3.Select(A, k) <= 255)))


def i64_dom(arr):
    """G6b: every element of a Vec<i64> returned by a trusted rust fn is an i64 (bound variable `elem!k`)."""
    k = z3.Int("elem!k")
    return z3.ForAll([k], z3.Implies(z3.And(k >= 0, k < arr.n), in_range(z3.Select(arr.a, k))))


def str_boundary(arr, i):
    """G2 B1 (G2_SPEC 4.4): i is a UTF-8 character boundary of byte array arr (total, no domain)."""
    b = z3.Select(arr.a, i)
    return z3.Or(i == 0, i == arr.n, z3.And(i > 0, i < arr.n, z3.Or(b < 128, b >= 192)))


FIX = {
    "index_oob": "add a requires/invariant/if that bounds the index, e.g. `invariant 0 <= i && i <= len(a)`",
    "overflow": "bound the operands with requires (element ranges, len(a) <= N) and loop invariants on accumulators",
    "div": "prove divisor > 0 and dividend >= 0 (add an if or requires)",
    "shift_range": "prove 0 <= shift count < 64 (add an if or requires)",
    "alloc_negative": "prove 0 <= length <= 2^40",
    "call_requires": "make the callee's requires hold before the call",
    "ensures": "fix the code, or strengthen loop invariants so the postcondition follows",
    "assert": "strengthen invariants or fix the code",
    "refinement_init": "make the initial value satisfy the declared range/length, or widen the refinement; it is a proof obligation, not a run-time check",
    "refinement_assign": "make the stored value satisfy the declared range, or widen the refinement; a loop that updates it needs invariants that imply the range",
    "invariant_entry": "the invariant must hold before the first iteration; check initial values",
    "invariant_preserved": "the invariant must hold again after one body iteration; add facts it depends on as more invariants",
    "decreases_nonneg": "the decreases expression must be >= 0 whenever the loop condition holds; add an invariant",
    "decreases_decrease": "the decreases expression must strictly decrease each iteration",
    "recursive_nonneg": "the decreases measure must be >= 0 at function entry and at every recursive call; add a requires, or guard the call with an if/match",
    "recursive_decrease": "the callee's measure must be strictly smaller than the caller's entry measure: recurse on a smaller argument (a child of a match binder, an index one closer to the end), or fix the measure",
    "variant_domain": "guard the lookup so it is only reached for that variant, e.g. `is_variant(e, E::V) && variant_field(e, E::V, f) > 0` or `!is_variant(e, E::V) || ...`",
    "opaque_total": "an opaque body must be defined for every argument: guard each partial operation inside the body (for example i < len(a) && a[i] > 0)",
    "contract_domain": "guard the partial operation earlier in the same contract or in an earlier clause: index range, divisor != 0, is_some/is_ok before some_val/ok_val/err_val, lo < hi before max/min",
    "elem_range": "store or fill only values in 0..=255 into a [u8] array: bound the value with requires, an if, or a loop invariant",
    "utf8_invalid": "guard str_from_utf8(b) with utf8_valid(b) in a requires clause or an if",
    "string_slice_range": "keep 0 <= lo <= hi <= len(s) for substr(s, lo, hi): bound lo and hi with requires, an if, or a loop invariant (a counterexample str may not be valid UTF-8)",
    "utf8_boundary": "cut a str only at character boundaries: require or check is_char_boundary(s, lo) and is_char_boundary(s, hi) (a counterexample str may not be valid UTF-8)",
}


class FnVerifier:
    def __init__(s, prog, f, errors, model=None, rec=None, qcfg=None, extra=None, probe=None, cache=None, frame=None):
        s.rec = rec
        s.cache = cache     # G4b-3: ProofCache or None
        s._kmemo = {}       # G4b-3: id(assertion object) -> (object, key text piece), proof_cache.key_text
        s.qcfg = qcfg if qcfg is not None else QueryConfig()
        # STAGE1 B2: extra = {loop number: [(cand id, expr, text)]} (inferred invariants); probe = exploration
        # state (only candidate obligations are queried, nothing is reported). Both empty = the old path.
        s.extra = extra or {}
        s.frame = frame or {}
        s.frame_failed = {}
        s.probe = probe
        s.loop_no = {id(w): n for n, (w, _, _) in enumerate(walk_loops(f.body))} if s.extra or s.frame else {}
        s.funcs = {g.name: g for g in prog.functions}
        s.explicit = bool(prog.explicit)    # G4 A1
        s.model = model if model is not None else Model(prog)
        s.f, s.errors = f, errors
        # LG26 (R-6b-3, G-LG25-1): computed once per function; only a UTF-8 aware function assumes its str values valid
        s.utf8_aware = utf8_aware(f, s.funcs)
        s.k = 0
        s.seen = set()
        s.qstack = []
        s.opq_fns = {}      # P13o: opaque function name -> its uninterpreted symbol opq!F
        s.aggs = {}
        s.agg_shapes = []   # LG4: (kind, F, body, shape) for the aggregate frame lemma
        s.lemmas = []
        s.split_lemmas = []  # LG19: count range split, a third stage only (it slows unrelated proofs)
        s.lemma_keys = set()
        s.theta = {}        # G3 A2: callee type-parameter substitution while a callee contract is re-interpreted
        s._opq = {}
        # G9 A1: function SCCs (Program.g9) and the frozen entry value of this function's decreases measure
        meta = getattr(prog, "g9", None)
        s.scc_id = {}
        if meta is not None:
            for i, names in enumerate(meta["fn_sccs"]):
                for n in names:
                    s.scc_id[n] = i
        s.cost0 = None
        s.m0 = None
        s._cex_m = None
        # G5 A1: immutable entry snapshots {inout parameter name: entry array}; separate from the current environment.
        # Empty for a function without inout parameters.
        s.snap = {}
        s.iter_places = []

    def T(s, ty):
        """type of a checked node under the current contract-instantiation substitution"""
        if ty is not None and s.theta:
            return substitute(ty, s.theta)
        return ty

    def fresh(s, base, ty):
        s.k += 1
        n = "%s!%d" % (base, s.k)
        if ty == I64:
            return z3.Int(n)
        if ty == BOOL:
            return z3.Bool(n)
        if ty == OPTION:
            return z3.Const(n, Opt)
        if ty == RESULT:
            return z3.Const(n, Res)
        if ty == STR:   # G2 B1: str = U8 byte array
            return mk_arr(z3.Array(n, z3.IntSort(), z3.IntSort()), z3.Int(n + ".len"), base, U8)
        if is_array(ty):
            elem = array_elem(ty)
            if elem.owner is not None:      # G3: [T] = (Array Int sort(T), length)
                esort = s.model.sort_of(elem)
            else:
                esort = z3.BoolSort() if elem == BOOL else z3.IntSort()
            return mk_arr(z3.Array(n, z3.IntSort(), esort), z3.Int(n + ".len"), base, elem)
        if ty.owner is None and ty.name == "process":       # G10 proc A1 (4.1): the public state record
            out = mk_arr(z3.Array(n + ".stdout", z3.IntSort(), z3.IntSort()), z3.Int(n + ".stdout.size"), base, U8)
            err = mk_arr(z3.Array(n + ".stderr", z3.IntSort(), z3.IntSort()), z3.Int(n + ".stderr.size"), base, U8)
            return Process(z3.Int(n + ".error"), z3.Bool(n + ".stdin_open"), z3.Bool(n + ".stdout_eof"),
                           z3.Bool(n + ".stderr_eof"), out, err)
        if ty.owner is None and ty.name == "process_args":  # G10 proc A1 (4.3)
            return PArgs(z3.Int(n + ".argc"))
        if ty.owner is None and ty.name == "socket":        # G10 net A1 (4.1): the public state record
            buf = mk_arr(z3.Array(n + ".buf", z3.IntSort(), z3.IntSort()), z3.Int(n + ".buf.len"), base, U8)
            return Socket(z3.Int(n + ".error"), z3.Bool(n + ".write_open"), z3.Bool(n + ".eof"), buf)
        if ty.owner is None and ty.name == "loop":
            return Loop(buf=mk_arr(z3.Array(n + ".buf", z3.IntSort(), z3.IntSort()), z3.Int(n + ".buf.len"), base, U8),
                        error=z3.Int(n + ".error"),
                        cap=z3.Int(n + ".cap"),
                        count=z3.Int(n + ".count"),
                        last=z3.Int(n + ".last"),
                        ev_slot=z3.Int(n + ".ev_slot"),
                        ev_kind_=z3.Int(n + ".ev_kind_"),
                        exit=z3.Int(n + ".exit"),
                        kind=z3.Array(n + ".kind", z3.IntSort(), z3.IntSort()),
                        eof=z3.Array(n + ".eof", z3.IntSort(), z3.BoolSort()),
                        write_open=z3.Array(n + ".write_open", z3.IntSort(), z3.BoolSort()))
        if ty.owner is None and ty.name == "listener":      # G10 net A1 (4.1)
            return Listener(z3.Int(n + ".error"), z3.Int(n + ".port"))
        if ty.owner is None and ty.name in _COLL:   # G6 A1: Vec = [T] array, String = u8 array, HashMap = (d, a, n)
            if ty.name == "file":
                buf = mk_arr(z3.Array(n, z3.IntSort(), z3.IntSort()), z3.Int(n + ".size"), base, U8)
                return File(z3.Bool(n + ".ok"), z3.Bool(n + ".readable"), z3.Bool(n + ".writable"), buf)
            if ty.name == "string":
                return mk_arr(z3.Array(n, z3.IntSort(), z3.IntSort()), z3.Int(n + ".len"), base, U8)
            if ty.name == "vec":
                return mk_arr(z3.Array(n, z3.IntSort(), s.model.sort_of(ty.args[0])), z3.Int(n + ".len"), base,
                              ty.args[0])
            ks, vs = s.model.map_key_sort(ty.args[0]), s.model.sort_of(ty.args[1])
            return Map(z3.Array(n + ".d", ks, z3.BoolSort()), z3.Array(n + ".a", ks, vs), z3.Int(n + ".len"), base,
                       ty.args[0], ty.args[1])
        if ty.owner is not None or ty.name in ("option", "result") or s.model.is_user(ty):
            return z3.Const(n, s.model.sort_of(ty))
        raise Exception("fresh: " + ty.name)

    # ---------- value domain ----------
    def wf(s, ty, v):
        """well_formed(ty, v): every active leaf is an i64. None when trivially true (module-level _wf_of)."""
        return _wf_of(s.model, ty, v)

    def observe(s, st, t, ty, force=False, *, g=()):
        """G9 A1: add the valid one-step size/wf facts of the observed nominal term t (Model.facts) to the path
        condition of st, once per fact. Only for recursive-related types, or any non-generic nominal when forced
        (an explicit `size` call). A no-op for programs without recursive types."""
        m = s.model
        if isinstance(t, Arr) and t.elem in m.vec_sums:
            s.vec_sum(st, g, t)
        if ty is None or isinstance(t, (Arr, Map, File)):
            return
        if not (m.rel(ty) or (force and m.rel_nominal(ty))):
            return
        tid = t.get_id()            # z3 AST ids: a term (datatype sort) never shares an id with a fact (Bool)
        if tid in st.obs:
            return
        new = [tid]
        for fct in m.facts(ty, t):
            i = fct.get_id()
            if i not in st.obs:
                new.append(i)
                st.pc.append(fct)
        st.obs = st.obs | set(new)

    def vec_sum(s, st, g, v):
        total = s.model.vec_sums[v.elem](v.a, v.n)
        s.assume(st, g, z3.And(total >= 0, z3.Implies(v.n == 0, total == 0)), bound=s.qstack)
        return total

    def observe_vec_element(s, st, g, v, i, element):
        guard = [*g, i >= 0, i < v.n]
        size = s.model.size_fn(v.elem.name)(element)
        s.assume(st, guard, z3.And(size >= 1, size <= s.vec_sum(st, g, v)), bound=s.qstack)
        wf = s.wf(v.elem, element)
        if wf is not None:
            s.assume(st, guard, wf, bound=s.qstack)

    def field_put(s, st, g, ty, val):
        term = s.model.field_put(ty, val)
        if isinstance(val, Arr) and val.elem in s.model.vec_sums:
            # The handle projections recover the vector at each observed construction.
            vec = s.model.field_get(ty, term)
            s.assume(st, g, z3.And(vec.n == val.n, vec.a == val.a))
            s.vec_sum(st, g, val)
        return term

    def rebuild(s, ty, cur, path, new, st, g=()):
        """struct value `cur` of type ty with the field path replaced by `new`"""
        if not path:
            return new
        m = s.model
        i = m.tt.field_index(ty, path[0])
        ctor, accs = m.struct_info(ty)
        vals = [acc(cur) for acc in accs]
        fty = m.fields_of(ty)[i][1]
        if len(path) == 1:
            vals[i] = s.field_put(st, g, fty, new)
        else:
            vals[i] = s.rebuild(fty, vals[i], path[1:], new, st, g)
        return ctor(*vals)

    # ---------- obligations ----------
    def prove(s, st, g, ob, node, code, what, key_extra=None, cand=None):
        probing = s.probe is not None
        frame_clause = cand is not None and cand.startswith("F")
        if probing and (cand is None or frame_clause or cand in s.probe.failed):
            return      # exploration looks only at candidate obligations that have not failed this round
        key = (node.line, node.col, code) if key_extra is None else (node.line, node.col, code, key_extra)
        skipped = False if probing else key in s.seen
        idx = None
        if s.rec is not None:
            idx = s.rec.begin(s.f.name, node, code, skipped, s.qcfg.phase, s.qcfg.round, cand, file=s.file_of(node))
        if skipped:
            return
        def run(with_lemmas):
            cfg = s.qcfg
            sol = z3.Solver()
            sol.set("timeout", cfg.timeout_ms)
            if cfg.rlimit is not None:
                sol.set("rlimit", cfg.rlimit)
            if cfg.seed is not None:
                sol.set("random_seed", cfg.seed)
            # the terms are built in the same order as without the cache: z3 numbers terms by creation, and the
            # numbering steers the solver (a sat answer can become unknown)
            for p in st.pc:
                sol.add(p)
            for p in g:
                sol.add(p)
            lemmas = (s.lemmas + (s.split_lemmas if with_lemmas == 2 else [])) if with_lemmas else []
            for p in lemmas:
                sol.add(p)
            neg = z3.Not(ob)
            sol.add(neg)
            text = key_text = None
            cache = s.cache
            if s.rec is not None or cache is not None:
                head = "(set-option :timeout %d)\n" % cfg.timeout_ms
                if cfg.rlimit is not None:
                    head += "(set-option :rlimit %d)\n" % cfg.rlimit
                if cfg.seed is not None:
                    head += "(set-option :random_seed %d)\n" % cfg.seed
                # the trace text (sexpr) and the cache key text (per-assertion benchmark pieces, docs/G4B_SPEC.md 7 rule 9)
                if s.rec is not None:
                    text = head + sol.sexpr()
                if cache is not None:
                    parts = list(st.pc) + list(g) + list(lemmas) + [neg]
                    key_text = proof_cache.key_text(head, parts, s._kmemo)
            if cache is not None and cache.lookup(key_text):
                cache.hits += 1
                if s.rec is not None:
                    s.rec.attempt(idx, bool(with_lemmas), text, "unsat", "proved", cfg.timeout_ms, cfg.rlimit,
                                  cfg.seed, None, cached=True)
                return sol, z3.unsat
            if cache is not None:
                cache.misses += 1
            try:
                r = sol.check()
            except z3.Z3Exception as e:
                if s.rec is not None:
                    s.rec.attempt(idx, bool(with_lemmas), text, "error", "error", cfg.timeout_ms, cfg.rlimit,
                                  cfg.seed, str(e))
                raise
            if s.rec is not None:
                reason = sol.reason_unknown() if r == z3.unknown else None
                s.rec.attempt(idx, bool(with_lemmas), text, str(r), query_status(r, reason), cfg.timeout_ms,
                              cfg.rlimit, cfg.seed, reason)
            if cache is not None and r == z3.unsat:
                cache.store(key_text)
            return sol, r

        if probing:
            s._probe_attempts(run, code, cand)
            return
        # first without aggregate lemmas: fewer assumptions, so unsat here is still a sound proof,
        # and it keeps quantified lemmas out of the many obligations that never need them
        sol, r = run(False)
        if r == z3.unsat:
            return
        relaxed = None
        if s.lemmas:
            if r == z3.sat:
                relaxed = s.cex(sol.model(), st)
            sol, r = run(True)
            if r == z3.unsat:
                return
            if s.split_lemmas:
                # LG19: the count range split only when the other lemmas are not enough
                sol, r = run(2)
                if r == z3.unsat:
                    return
        if frame_clause:
            reason = sol.reason_unknown() if r == z3.unknown else None
            s.frame_failed.setdefault(cand, (code, query_status(r, reason) or "error"))
            return
        s.seen.add(key)
        cex = None
        if r == z3.sat or relaxed is not None:
            cex = s.cex(sol.model(), st) if r == z3.sat else dict(relaxed, _note="counterexample ignores aggregate (sum/count/max/min) definitions; retry with them: %s" % sol.reason_unknown())
            code2, msg = code, "cannot prove %s in %s" % (what, s.f.name)
            hid = s.hidden(st)
            if hid:     # P13o (P13o.3): name the opaque functions whose bodies the proof did not see
                msg += "".join(" (opaque %s is not revealed here; add reveal %s; before this point if the proof needs "
                               "its body)" % (h, h) for h in hid)
                cex = dict(cex or {}, _opaque=hid)
        else:
            code2, msg = "verify_unknown", "solver gave up proving %s in %s (%s)" % (what, s.f.name, sol.reason_unknown())
        if code == "struct_invariant":
            if code2 == "verify_unknown":
                cex = {"_note": sol.reason_unknown()}
            code2, msg = code, what
        s.errors.append(PruvError("verify", node.line, node.col, code2, msg, FIX.get(code, ""), cex,
                                  file=s.file_of(node)))

    def file_of(s, node):
        """G4 A1: the file of an obligation's node; synthesized nodes (inferred invariants) fall back to the function's
        file. None for every legacy program (nodes carry no file there), so legacy output is unchanged."""
        return getattr(node, "file", None) or s.f.file

    def _probe_attempts(s, run, code, cand):
        """STAGE1 B2 exploration of one candidate obligation: the same two stages as prove (without lemmas, then
        with them if there are any and the first was not unsat); every attempt checks the budget and the clock
        first. An attempt that is unknown with z3 reason exactly "timeout" (the query's own 500 ms wall-clock limit)
        aborts the whole function (InferenceTimeout), as does the 5 s wall clock (Probe.tick). Any other last attempt
        that is not proved (not_proved, resource_unknown, error, or unknown with another reason such as rlimit
        exhaustion "canceled") marks only this candidate failed for this round; the status string is recorded
        as query_status gives it."""
        pr = s.probe
        status = None
        for with_lemmas in (False, True, 2):
            if with_lemmas and not s.lemmas or with_lemmas == 2 and not s.split_lemmas:
                break
            pr.tick()
            try:
                sol, r = run(with_lemmas)
            except z3.Z3Exception:
                status = "error"
                break
            reason = sol.reason_unknown() if r == z3.unknown else None
            status = query_status(r, reason)
            if reason == "timeout":
                raise InferenceTimeout()
            if r == z3.unsat:
                return
        pr.fail(cand, code, status)

    def opaque(s, val):
        """G3 A2 (4.7): token of an abstract model value; ordinals are per abstract sort, in first-seen order of
        this counterexample, and equal model values share one"""
        sname = val.sort().name()
        tab = s._opq.setdefault(sname, {})
        key = val.sexpr()
        if key not in tab:
            tab[key] = len(tab)
        return {"type_parameter": s.model.abs_name[sname], "value": "opaque#%d" % tab[key]}

    def cex(s, m, st):
        out = {}
        s._opq = {}
        s._cex_m = m
        for name, v in st.env.items():
            try:
                if isinstance(v, File):
                    out[name] = {field: z3.is_true(m.eval(getattr(v, field), model_completion=True))
                                 for field in ("ok", "readable", "writable")}
                    out[name]["size"] = m.eval(v.buf.n, model_completion=True).as_long()
                elif isinstance(v, Process):     # G10 proc A1: the public state; ok is error == 0
                    err = m.eval(v.error, model_completion=True).as_long()
                    out[name] = {"ok": err == 0, "error": err,
                                 "stdin_open": z3.is_true(m.eval(v.stdin_open, model_completion=True)),
                                 "stdout_eof": z3.is_true(m.eval(v.stdout_eof, model_completion=True)),
                                 "stderr_eof": z3.is_true(m.eval(v.stderr_eof, model_completion=True)),
                                 "stdout_size": m.eval(v.out.n, model_completion=True).as_long(),
                                 "stderr_size": m.eval(v.err.n, model_completion=True).as_long()}
                elif isinstance(v, PArgs):
                    out[name] = {"argc": m.eval(v.argc, model_completion=True).as_long()}
                elif isinstance(v, Socket):      # G10 net A1: the public state; ok is error == 0
                    err = m.eval(v.error, model_completion=True).as_long()
                    out[name] = {"ok": err == 0, "error": err,
                                 "write_open": z3.is_true(m.eval(v.write_open, model_completion=True)),
                                 "eof": z3.is_true(m.eval(v.eof, model_completion=True)),
                                 "size": m.eval(v.buf.n, model_completion=True).as_long()}
                elif isinstance(v, Loop):
                    out[name] = {k: m.eval(getattr(v, k), model_completion=True).as_long()
                                 for k in ("error", "cap", "count", "last", "ev_slot", "ev_kind_", "exit")}
                    out[name]["ok"] = out[name]["error"] == 0
                    out[name]["size"] = m.eval(v.buf.n, model_completion=True).as_long()
                elif isinstance(v, Listener):
                    err = m.eval(v.error, model_completion=True).as_long()
                    out[name] = {"ok": err == 0, "error": err, "port": m.eval(v.port, model_completion=True).as_long()}
                elif isinstance(v, Arr):
                    n = m.eval(v.n, model_completion=True).as_long()
                    if v.elem == BOOL:   # G2 A5: bool elements print as true/false
                        els = [z3.is_true(m.eval(z3.Select(v.a, i), model_completion=True))
                               for i in range(min(max(n, 0), 8))]
                    elif v.elem.owner is not None:   # G3: abstract elements are opaque tokens
                        els = [s.opaque(m.eval(z3.Select(v.a, i), model_completion=True))
                               for i in range(min(max(n, 0), 8))]
                    else:
                        els = [m.eval(z3.Select(v.a, i), model_completion=True).as_long() for i in range(min(max(n, 0), 8))]
                    out[name] = {"len": n, "first": els}
                elif z3.is_int(v):
                    out[name] = m.eval(v, model_completion=True).as_long()
                elif z3.is_bool(v):
                    out[name] = z3.is_true(m.eval(v, model_completion=True))
                else:
                    out[name] = s.fmt_val(m.eval(v, model_completion=True))
            except Exception:
                pass
        if s.cost0 is not None:
            out["C"] = m.eval(s.cost0, model_completion=True).as_long()
        return out

    # G9 A1: display bounds of a recursive counterexample value (they never constrain the solver)
    CEX_MAX_NODES = 64      # nominal constructor nodes printed per value, preorder
    CEX_MAX_DEPTH = 16      # nesting depth printed (the value itself is depth 1)
    CEX_TRUNCATED = "<truncated>"

    def fmt_val(s, v, nested=False):
        """z3 model value -> Pruv syntax on one line (user struct/enum); unknown shapes keep z3's str().
        G3: an abstract value is its opaque token (a dict at the top, `T:opaque#N` inside a datatype).
        A program with a G9 feature takes the bounded printer; any other program the original one."""
        if s.model.g9:
            return s._fmt_g9(v, nested, [0], 1, None)
        return s._fmt_val_old(v, nested)

    def _fmt_buf(s, v, fty):
        """a buffer field value Buf(len, arr): [e0, e1, ...] (first 8 elements) or "bytes" for a str field"""
        m = s._cex_m
        n = v.children()[0].as_long()
        arr = v.children()[1]
        els = []
        for i in range(min(max(n, 0), 8)):
            x = m.eval(z3.Select(arr, z3.IntVal(i)), model_completion=True)
            els.append(z3.is_true(x) if fty.name == "array_bool" else x.as_long())
        more = n > 8
        if fty.name == "str":
            body = "".join(chr(b) if 32 <= b < 127 and chr(b) not in '"\\' else "\\x%02x" % (b & 255) for b in els)
            return '"%s%s"' % (body, "..." if more else "")
        txt = ", ".join(("true" if x else "false") if fty.name == "array_bool" else str(x) for x in els)
        return "[%s%s]" % (txt, (", ..." if txt else "...") if more else "")

    def _fmt_g9(s, v, nested, lim, depth, fty):
        try:
            if z3.is_int_value(v):
                return str(v.as_long())
            if z3.is_bool(v):
                return "true" if z3.is_true(v) else "false"
            sname = v.sort().name()
            if sname in s.model.buf_names:
                return s._fmt_buf(v, fty)
            if sname in s.model.abs_name:
                tok = s.opaque(v)
                return "%s:%s" % (tok["type_parameter"], tok["value"]) if nested else tok
            ty = s.model.type_of(v)
            if ty is not None and ty.name in ("option", "result") and ty.args and z3.is_app(v):
                nm = s.model.short.get(v.decl().name(), v.decl().name())
                ks = [s._fmt_g9(c, True, lim, depth, None) for c in v.children()]
                return nm if not ks else "%s(%s)" % (nm, ", ".join(ks))
            if ty is None or not s.model.is_user(ty) or not z3.is_app(v):
                return str(v)
            if lim[0] >= s.CEX_MAX_NODES or depth > s.CEX_MAX_DEPTH:
                return s.CEX_TRUNCATED
            lim[0] += 1
            dn = v.decl().name()
            decl = s.model.tt.decls.get(ty.name)
            if decl is None:
                return str(v)
            if isinstance(decl, StructDecl):
                ctor, _ = s.model.struct_info(ty)
                if ctor.name() == dn and len(decl.fields) == v.num_args():
                    if not decl.fields:
                        return ty.name
                    kids = [s._fmt_g9(c, True, lim, depth + 1, fd.type) for fd, c in zip(decl.fields, v.children())]
                    return "%s { %s }" % (ty.name, ", ".join("%s: %s" % (fd.name, k) for fd, k in zip(decl.fields, kids)))
            else:
                for vd in decl.variants:
                    ctor = s.model.enum_info(ty)[vd.name][0]
                    if ctor.name() == dn:
                        if not vd.fields:
                            return "%s::%s" % (ty.name, vd.name)
                        kids = [s._fmt_g9(c, True, lim, depth + 1, fd.type) for fd, c in zip(vd.fields, v.children())]
                        return "%s::%s(%s)" % (ty.name, vd.name, ", ".join(kids))
            return str(v)
        except Exception:
            return str(v)

    def _fmt_val_old(s, v, nested=False):
        try:
            if z3.is_int_value(v):
                return str(v.as_long())
            if z3.is_bool(v):
                return "true" if z3.is_true(v) else "false"
            if v.sort().name() in s.model.abs_name:
                tok = s.opaque(v)
                return "%s:%s" % (tok["type_parameter"], tok["value"]) if nested else tok
            ty = s.model.type_of(v)
            if ty is not None and ty.name in ("option", "result") and ty.args and z3.is_app(v):
                nm = s.model.short.get(v.decl().name(), v.decl().name())
                ks = [s.fmt_val(c, True) for c in v.children()]
                return nm if not ks else "%s(%s)" % (nm, ", ".join(ks))
            if ty is None or not s.model.is_user(ty) or not z3.is_app(v):
                return str(v)
            dn = v.decl().name()
            kids = [s.fmt_val(c, True) for c in v.children()]
            if ty.name in s.model.struct or (ty.args and s.model.tt.is_struct(ty)):
                ctor, _ = s.model.struct_info(ty)
                decl = next((d for d in s.model.tt.order if d.name == ty.name), None)
                if ctor.name() == dn and decl is not None and len(decl.fields) == len(kids):
                    if not kids:
                        return ty.name
                    return "%s { %s }" % (ty.name, ", ".join(
                        "%s: %s" % (fd.name, k) for fd, k in zip(decl.fields, kids)))
            elif ty.name in s.model.enum or (ty.args and s.model.tt.is_enum(ty)):
                for vn, (ctor, _, _) in s.model.enum_info(ty).items():
                    if ctor.name() == dn:
                        if not kids:
                            return "%s::%s" % (ty.name, vn)
                        return "%s::%s(%s)" % (ty.name, vn, ", ".join(kids))
            return str(v)
        except Exception:
            return str(v)

    def assume(s, st, g, fact, *, bound=()):
        """add a fact that only holds on the short-circuit path g"""
        fact = z3.Implies(z3.And(*g), fact) if g else fact
        st.pc.append(z3.ForAll(list(bound), fact) if bound else fact)

    def assume_str(s, st, g, v):
        """LG26 (R-6b-3, G-LG25-1): a str value is valid UTF-8 at run time; assumed only in a UTF-8 aware function
        LG27 (G-LG26-3): each fact is pushed once per path (deduped by z3 id in st.obs); the guard is kept"""
        if s.utf8_aware and isinstance(v, Arr):
            valid = s.model.utf8_valid(v)
            if valid.get_id() in st.obs:    # LG28 (G-LG27-5): the unguarded W is already on this path
                return v
            if s.qstack:    # a field or call result can depend on an enclosing quantifier's indices
                fact = z3.ForAll(list(s.qstack), z3.Implies(z3.And(list(g)), valid))
            else:
                fact = z3.Implies(z3.And(*g), valid) if g else valid
            i = fact.get_id()
            if i not in st.obs:     # LG27: obs only holds ids of facts already in pc
                st.pc.append(fact)
                st.obs = st.obs | {i}
        return v

    # ---------- expressions ----------
    def read_field(s, st, v, field, g=()):
        model = s.model
        ty = model.type_of(v)
        i = model.tt.field_index(ty, field)
        r = model.struct_info(ty)[1][i](v)
        if model.g9:
            s.observe(st, v, ty)
            fty = model.fields_of(ty)[i][1]
            r = model.field_get(fty, r, field)
            s.observe(st, r, fty, g=g)
            if fty == STR:      # LG26 (R-6b-3, G-LG25-1)
                s.assume_str(st, list(g), r)
        return r

    def field_chain_type(s, ty, path):
        """declared type of the field reached by `path` from a struct of type ty"""
        for fname in path:
            ty = s.model.fields_of(ty)[s.model.tt.field_index(ty, fname)][1]
        return ty

    def ev(s, e, st, g, code, env=None):
        """code=True: emit proof obligations and i64 semantics. env overrides st.env (contracts)."""
        env = st.env if env is None else env
        if isinstance(e, Spent):
            return st.env["spent"]
        if isinstance(e, IntLit):
            return z3.IntVal(e.value)
        if isinstance(e, BoolLit):
            return z3.BoolVal(e.value)
        if isinstance(e, StrLit):   # G2 B1: constant byte array, no counter, no assumption
            a = z3.K(z3.IntSort(), z3.IntVal(0))
            for i, b in enumerate(e.value):
                a = z3.Store(a, z3.IntVal(i), z3.IntVal(b))
            return s.assume_str(st, g, mk_arr(a, z3.IntVal(len(e.value)), "literal", U8))   # LG26
        if isinstance(e, Var):
            v = env[e.name]
            if s.utf8_aware and getattr(e, "ty", None) == STR:   # LG26: loop havoc makes a mutable str a fresh array
                return s.assume_str(st, g, v)
            if isinstance(v, Arr) and v.elem in s.model.vec_sums:
                s.vec_sum(st, g, v)
            return v
        if isinstance(e, Move):
            v = env[e.name]
            for field in e.fields:
                v = s.read_field(st, v, field)
            return v
        if isinstance(e, InoutArg):     # G5 A1: the marked place's current array (the call transition rebinds it)
            return s.ev(e.expr, st, g, code, env)
        if isinstance(e, Unary):
            v = s.ev(e.operand, st, g, code, env)
            if e.op == "!":
                return z3.Not(v)
            if e.op == "~":
                return -v - 1
            if code:
                s.prove(st, g, in_range(-v), e, "overflow", "no overflow in unary -")
            return -v
        if isinstance(e, Binary):
            op = e.op
            if op in ("&&", "||", "==>"):
                l = s.ev(e.left, st, g, code, env)
                g2 = g + [l if op != "||" else z3.Not(l)]
                r = s.ev(e.right, st, g2, code, env)
                return z3.And(l, r) if op == "&&" else z3.Or(l, r) if op == "||" else z3.Implies(l, r)
            l = s.ev(e.left, st, g, code, env)
            r = s.ev(e.right, st, g, code, env)
            if op in BIT_OP:
                res = signed_bv(BIT_Z3[BIT_OP[op]](z3.Int2BV(l, 64), z3.Int2BV(r, 64)))
                if code:
                    s.assume(st, g, in_range(res))
                    if op == "&":
                        s.assume(st, g, z3.Implies(r >= 0, z3.And(0 <= res, res <= r)))
                        s.assume(st, g, z3.Implies(l >= 0, z3.And(0 <= res, res <= l)))
                        for lit, other in ((e.left, r), (e.right, l)):
                            if isinstance(lit, IntLit) and 0 < lit.value < 2 ** 62:
                                modulus = lit.value + 1
                                if modulus & (modulus - 1) == 0:
                                    s.assume(st, g, z3.Implies(other >= 0, res == other % modulus))
                    else:
                        s.assume(st, g, z3.Implies(z3.And(l >= 0, r >= 0), z3.And(0 <= res, res <= l + r)))
                return res
            if op in ("<<", ">>"):
                if code:
                    s.prove(st, g, z3.And(r >= 0, r < 64), e, "shift_range", "shift count in 0..64 in `%s`" % op)
                res, no_overflow = shift_value(e, l, r)
                if code and no_overflow is not None:
                    s.prove(st, g, no_overflow, e, "overflow", "no i64 overflow in `%s`" % op)
                if code and not isinstance(e.right, IntLit):
                    s.assume(st, g, in_range(res))
                    if op == ">>":
                        s.assume(st, g, z3.Implies(l >= 0, z3.And(0 <= res, res <= l)))
                        s.assume(st, g, z3.Implies(l < 0, z3.And(l <= res, res < 0)))
                    s.assume(st, g, z3.Implies(r == 0, res == l))
                return res
            if op == "+": res = l + r
            elif op == "-": res = l - r
            elif op == "*": res = l * r
            elif op in ("/", "%"):
                if code:
                    s.prove(st, g, z3.And(r > 0, l >= 0), e, "div", "divisor > 0 and dividend >= 0")
                return l / r if op == "/" else l % r
            elif op == "==": return l == r
            elif op == "!=": return l != r
            elif op == "<": return l < r
            elif op == "<=": return l <= r
            elif op == ">": return l > r
            elif op == ">=": return l >= r
            if code:
                s.prove(st, g, in_range(res), e, "overflow", "no i64 overflow in `%s`" % op)
            return res
        if isinstance(e, Index):
            arr = s.ev(e.array, st, g, code, env)
            i = s.ev(e.index, st, g, code, env)
            if code:
                s.prove(st, g, z3.And(i >= 0, i < arr.n), e, "index_oob",
                        "index in bounds for %s[...]" % arr.name)
            v = z3.Select(arr.a, i)
            if arr.elem in s.model.vec_sums:
                s.observe_vec_element(st, g, arr, i, v)
            # G2 A5: a bool element is a Bool; G3: an abstract element has no numeric domain: no in_range assumption
            elif code and arr.elem != BOOL and arr.elem.owner is None:
                s.assume(st, g, in_range(v))
            return v
        if isinstance(e, Ctor):
            cty = s.T(e.ty)
            if e.name == "None":
                return s.model.opt(cty).none
            v = s.ev(e.arg, st, g, code, env)
            if e.name == "Some":
                return s.model.opt(cty).some(v)
            o = s.model.res(cty)
            return o.ok(v) if e.name == "Ok" else o.err(s.model.field_put(result_error(cty), v))
        if isinstance(e, Quant):
            lo = s.ev(e.lo, st, g, False, env)
            hi = s.ev(e.hi, st, g, False, env)
            s.k += 1
            k = z3.Int("%s!q%d" % (e.var, s.k))
            env2 = dict(env); env2[e.var] = k
            s.qstack.append(k)
            try:
                body = s.ev(e.body, st, g, False, env2)
                # R-6-3: trigger terms, evaluated like the body (a term, no domain obligation)
                trig = [s.ev(t, st, g, False, env2) for t in e.triggers]
            finally:
                s.qstack.pop()
            if e.kind in ("sum", "count", "max", "min"):
                return s.aggregate(e.kind, k, lo, hi, body)
            rng = z3.And(k >= lo, k < hi)
            pats = {"patterns": [z3.MultiPattern(*trig)]} if trig else {}
            if e.kind == "forall":
                return z3.ForAll([k], z3.Implies(rng, body), **pats)
            return z3.Exists([k], z3.And(rng, body), **pats)
        if isinstance(e, Call):
            return s.call(e, st, g, code, env)
        if isinstance(e, StructLit):
            vals = [s.ev(fi.value, st, g, code, env) for fi in e.fields]
            sty = s.T(e.ty) if e.ty is not None else Type(e.name)
            if s.model.g9:      # G9 A1: buffer fields store Buf values; recursive results expose their facts
                vals = [s.field_put(st, g, ft, x) for (fd, ft), x in zip(s.model.fields_of(sty), vals)]
                res = s.model.struct_info(sty)[0](*vals)
                s.observe(st, res, sty)
                return res
            return s.model.struct_info(sty)[0](*vals)
        if isinstance(e, EnumCtor):
            vals = [s.ev(a, st, g, code, env) for a in e.args]
            ety = s.T(e.ty) if e.ty is not None else Type(e.enum_name)
            if s.model.g9:
                vals = [s.field_put(st, g, ft, x) for (fd, ft), x in zip(s.model.fields_of(ety, e.variant), vals)]
                res = s.model.enum_info(ety)[e.variant][0](*vals)
                s.observe(st, res, ety)
                return res
            return s.model.enum_info(ety)[e.variant][0](*vals)
        if isinstance(e, FieldAccess):
            return s.read_field(st, s.ev(e.base, st, g, code, env), e.field, g)
        if isinstance(e, IsVariant):
            v = s.ev(e.value, st, g, code, env)
            return s.model.enum_info(s._enum_ty(e, v))[e.variant][1](v)
        if isinstance(e, VariantField):
            v = s.ev(e.value, st, g, code, env)
            ety = s._enum_ty(e, v)
            i = s.model.tt.field_index(ety, e.field, e.variant)
            r = s.model.enum_info(ety)[e.variant][2][i](v)
            if s.model.g9:
                s.observe(st, v, ety)
                fty = s.model.fields_of(ety, e.variant)[i][1]
                r = s.model.field_get(fty, r, e.field)
                s.observe(st, r, fty, g=g)
                if fty == STR:      # LG26 (R-6b-3, G-LG25-1): only where the variant is the active one
                    s.assume_str(st, list(g) + [s.model.enum_info(ety)[e.variant][1](v)], r)
            return r
        raise Exception("ev: %r" % e)

    def _enum_ty(s, e, v):
        """instantiated enum type of an is_variant / variant_field scrutinee: from its z3 sort (already
        substituted), else from the checker's record, else the plain enum name"""
        ty = s.model.type_of(v)
        if ty is not None and ty.name == e.enum_name:
            return ty
        return Type(e.enum_name)

    # ---------- contract domain of variant_field lookups ----------
    def dom(s, e, st, g, env, kind=None):
        """D(e): z3 condition under which every partial operation in contract e is defined (index in range,
        divisor != 0, some_val/ok_val/err_val on the right tag, variant_field on the named variant, max/min range
        non-empty), following the short-circuit order. kind: None=all, "variant"=variant_field only,
        "other"=the rest. None means trivially true."""
        if not _has_vf(e, kind):
            return None
        wo = kind in (None, "other")
        wv = kind in (None, "variant")
        if isinstance(e, Unary):
            return s.dom(e.operand, st, g, env, kind)
        if isinstance(e, Binary):
            dl = s.dom(e.left, st, g, env, kind)
            if e.op in ("&&", "||", "==>"):
                l = s.ev(e.left, st, g, False, env)
                guard = z3.Not(l) if e.op == "||" else l
                dr = s.dom(e.right, st, list(g) + [guard], env, kind)    # G-LG27-7: the short-circuit guard
                return _conj([dl, _imp(guard, dr)])
            parts = [dl, s.dom(e.right, st, g, env, kind)]
            if wo and e.op in ("/", "%"):
                parts.append(s.ev(e.right, st, g, False, env) != 0)
            if wo and (e.op in BIT_OP or e.op in ("<<", ">>")):
                l = s.ev(e.left, st, g, False, env)
                r = s.ev(e.right, st, g, False, env)
                parts.append(in_range(l))
                if e.op in BIT_OP:
                    parts.append(in_range(r))
                else:
                    parts.append(z3.And(r >= 0, r < 64))
                    if e.op == "<<":
                        parts.append(shift_value(e, l, r)[1])
            return _conj(parts)
        if isinstance(e, Index):
            parts = [s.dom(e.array, st, g, env, kind), s.dom(e.index, st, g, env, kind)]
            if wo:
                arr = s.ev(e.array, st, g, False, env)
                i = s.ev(e.index, st, g, False, env)
                parts.append(z3.And(i >= 0, i < arr.n))
            return _conj(parts)
        if isinstance(e, Ctor):
            return s.dom(e.arg, st, g, env, kind)
        if isinstance(e, Call):
            parts = [s.dom(a, st, g, env, kind) for a in e.args]
            if wo and e.name == "file_byte":
                f = s.ev(e.args[0], st, g, False, env)
                i = s.ev(e.args[1], st, g, False, env)
                parts.append(z3.And(i >= 0, i < f.buf.n))
            if wo and e.name in _PROC_DOM:      # G10 proc A1 (4.9): a valid stream, and for proc_byte a valid index
                pv = s.ev(e.args[0], st, g, False, env)
                sm = s.ev(e.args[1], st, g, False, env)
                parts.append(_proc_stream_ok(sm))
                if e.name == "proc_byte":
                    i = s.ev(e.args[2], st, g, False, env)
                    parts.append(z3.And(i >= 0, i < _proc_size(pv, sm)))
            if wo and e.name in _ASYNC_DOM:
                lv = s.ev(e.args[0], st, g, False, env)
                i = s.ev(e.args[1], st, g, False, env)
                parts.append(z3.And(i >= 0, i < (lv.buf.n if e.name == "ev_byte" else lv.cap)))
            if wo and e.name in _NET_DOM:       # G10 net A1 (4.9): net_byte needs a valid index
                sv = s.ev(e.args[0], st, g, False, env)
                i = s.ev(e.args[1], st, g, False, env)
                parts.append(z3.And(i >= 0, i < sv.buf.n))
            if wo and e.name in ("load_be", "load_le"):      # P2 batch 4: the word lies inside the array
                arr = s.ev(e.args[0], st, g, False, env)
                i = s.ev(e.args[1], st, g, False, env)
                parts.append(z3.And(i >= 0, i + e.args[2].value <= arr.n))
            if wo and e.name in BIT_Z3:     # P2: both arguments inside i64
                x = s.ev(e.args[0], st, g, False, env)
                y = s.ev(e.args[1], st, g, False, env)
                parts.append(z3.And(x >= -2 ** 63, x <= 2 ** 63 - 1, y >= -2 ** 63, y <= 2 ** 63 - 1))
                if e.name in ("wrapping_shl", "ushr"):      # shift count in 0..64
                    parts.append(z3.And(y >= 0, y < 64))
            if wo and e.name in _TAG_FNS:
                v = s.ev(e.args[0], st, g, False, env)
                vty = s.model.type_of(v)       # G3: the Option/Result instance of the argument
                if e.name == "some_val":
                    parts.append(s.model.opt(vty).is_some(v))
                else:
                    ro = s.model.res(vty)
                    parts.append(ro.is_ok(v) if e.name == "ok_val" else z3.Not(ro.is_ok(v)))
            return _conj(parts)
        if isinstance(e, StructLit):
            return _conj([s.dom(fi.value, st, g, env, kind) for fi in e.fields])
        if isinstance(e, EnumCtor):
            return _conj([s.dom(a, st, g, env, kind) for a in e.args])
        if isinstance(e, FieldAccess):
            return s.dom(e.base, st, g, env, kind)
        if isinstance(e, IsVariant):
            return s.dom(e.value, st, g, env, kind)
        if isinstance(e, VariantField):
            inner = s.dom(e.value, st, g, env, kind)
            if not wv:
                return inner
            v = s.ev(e.value, st, g, False, env)
            return _conj([inner, s.model.enum_info(s._enum_ty(e, v))[e.variant][1](v)])
        if isinstance(e, Quant):
            lo = s.ev(e.lo, st, g, False, env)
            hi = s.ev(e.hi, st, g, False, env)
            outer = [s.dom(e.lo, st, g, env, kind), s.dom(e.hi, st, g, env, kind)]
            if wo and e.kind in ("max", "min"):
                outer.append(lo < hi)
            outer = _conj(outer)
            s.k += 1
            k = z3.Int("%s!dk%d" % (e.var, s.k))
            envk = dict(env); envk[e.var] = k
            s.qstack.append(k)
            try:
                dbody = s.dom(e.body, st, g, envk, kind)
            finally:
                s.qstack.pop()
            if dbody is None:
                return outer
            rng = z3.And(k >= lo, k < hi)
            if e.kind in ("forall", "exists"):
                # the body at k is evaluated only if every earlier body was true (forall) / false (exists)
                j = z3.Int("%s!dj%d" % (e.var, s.k))
                envj = dict(env); envj[e.var] = j
                s.qstack.append(j)
                try:
                    bj = s.ev(e.body, st, g, False, envj)
                finally:
                    s.qstack.pop()
                prior = z3.ForAll([j], z3.Implies(z3.And(j >= lo, j < k), bj if e.kind == "forall" else z3.Not(bj)))
                rng = z3.And(rng, prior)
            return _conj([outer, z3.ForAll([k], z3.Implies(rng, dbody))])
        return None

    def dom_obligations(s, e, st, g, env):
        """[(D, code)]: variant_field lookups -> variant_domain, other partial operations -> contract_domain.
        Trivial domains (no partial op, or simplify to True) give no entry, hence no query."""
        out = []
        for kind, code in (("variant", "variant_domain"), ("other", "contract_domain")):
            if not _has_vf(e, kind):
                continue
            d = s.dom(e, st, g, env, kind)
            if d is None or z3.is_true(z3.simplify(d)):
                continue
            out.append((d, code))
        return out

    def check_dom(s, st, g, e, node, what, env=None, cand=None):
        """prove D(e) separately per code (variant_domain / contract_domain); returns the proven domains.
        cand: STAGE1 B2 candidate id (keys the duplicate suppression and tags the obligation)"""
        obs = s.dom_obligations(e, st, g, st.env if env is None else env)
        for d, code in obs:
            s.prove(st, g, d, node, code, what if code == "variant_domain" else what.replace(
                "variant_field lookups", "partial operations (index, /, %, some_val/ok_val/err_val, max/min)"),
                key_extra=cand, cand=cand)
        return [d for d, _ in obs]

    def agg_frame(s, kind, F, Tc, K0, Ps):
        """LG4 G-R03D-1 aggregate frame lemma. Pairs a new aggregate with every earlier one of the same kind and
        arity whose body differs only in array terms (same shape with every array-sorted subterm abstracted).
        For such a pair, on a non-empty range either the two aggregates are equal or some slot in the range has
        different bodies, named by a witness function W. This is true of any two aggregates over the same range."""
        shape = _arr_shape(Tc)
        for kind2, F2, Tc2, shape2 in s.agg_shapes:
            if kind2 != kind or F2.arity() != F.arity() or shape2 != shape:
                continue
            x, y = z3.Ints("aggf!x aggf!y")
            os_ = [z3.Int("aggf!o%d" % i) for i in range(len(Ps))]
            W = z3.Function("aggf!w%d" % len(s.agg_shapes) + "_%s" % F2.name(), *([z3.IntSort()] * (2 + len(Ps))),
                            z3.IntSort())
            w = W(x, y, *os_)
            b1 = z3.substitute(Tc2, (K0, w), *zip(Ps, os_))
            b2 = z3.substitute(Tc, (K0, w), *zip(Ps, os_))
            a1, a2 = F2(x, y, *os_), F(x, y, *os_)
            s.lemmas.append(z3.ForAll([x, y] + os_, z3.Implies(
                x < y, z3.Or(a1 == a2, z3.And(x <= w, w < y, b1 != b2))), patterns=[z3.MultiPattern(a1, a2)]))
        s.agg_shapes.append((kind, F, Tc, shape))

    def aggregate(s, kind, k, lo, hi, T):
        """sum/count/max/min over lo<=k<hi as an uninterpreted F(lo, hi, *outer) plus unfolding lemmas."""
        outer = [c for c in s.qstack if _occurs(T, c)]
        K0 = z3.Int("agg!k")
        Ps = [z3.Int("agg!p%d" % i) for i in range(len(outer))]
        Tc = z3.substitute(T, (k, K0), *[(o, p) for o, p in zip(outer, Ps)])
        key = (kind, Tc.sexpr(), len(outer))
        if key not in s.aggs:
            F = z3.Function("agg!%s%d" % (kind, len(s.aggs)), z3.IntSort(), z3.IntSort(),
                            *([z3.IntSort()] * len(outer)), z3.IntSort())
            s.aggs[key] = F
            s.agg_frame(kind, F, Tc, K0, Ps)
        F = s.aggs[key]

        def body_at(x):
            return z3.substitute(Tc, (K0, x), *[(p, o) for p, o in zip(Ps, outer)])

        def val(x):
            return z3.If(body_at(x), 1, 0) if kind == "count" else body_at(x)

        def comb(smaller, x, empty):
            # aggregate of (smaller range) extended by element x; `empty`: the smaller range is empty
            if kind in ("sum", "count"):
                return z3.If(empty, 0, smaller) + val(x)
            better = (smaller >= val(x)) if kind == "max" else (smaller <= val(x))
            return z3.If(empty, val(x), z3.If(better, smaller, val(x)))

        if (kind, F) not in s.lemma_keys:
            # empty range: sum/count are 0 (no new terms, so no matching loop)
            s.lemma_keys.add((kind, F))
            if kind in ("sum", "count"):
                x, y = z3.Ints("agg!x agg!y")
                os_ = list(z3.Ints(" ".join("agg!o%d" % i for i in range(len(outer))))) if outer else []
                fa = F(x, y, *os_)
                s.lemmas.append(z3.ForAll([x, y] + os_, z3.Implies(y <= x, fa == 0), patterns=[fa]))
            if kind == "count":
                # LG19 (R03L-G2): range split, count(lo, hi) == count(lo, m) + count(m, hi) for lo <= m <= hi. Every
                # trigger pairs two existing terms, and the instance only adds a term over their endpoints, so the
                # endpoint set never grows and there is no matching loop.
                x, m, y = z3.Ints("agg!x agg!m agg!y")
                os_ = list(z3.Ints(" ".join("agg!o%d" % i for i in range(len(outer))))) if outer else []
                fxy, fxm, fmy = F(x, y, *os_), F(x, m, *os_), F(m, y, *os_)
                s.split_lemmas.append(z3.ForAll([x, m, y] + os_, z3.Implies(z3.And(x <= m, m <= y), fxy == fxm + fmy),
                                          patterns=[z3.MultiPattern(fxm, fmy), z3.MultiPattern(fxy, fxm),
                                                    z3.MultiPattern(fxy, fmy)]))

        app = F(lo, hi, *outer)
        facts = []
        if kind == "count":
            facts.append(z3.Implies(lo < hi, z3.And(app >= 0, app <= hi - lo)))
        bound_lo = [c for c in s.qstack if _occurs(lo, c)]
        bound_hi = [c for c in s.qstack if _occurs(hi, c)]
        # unfold only by moving an unbound end, so a quantified lemma never re-matches its own output
        if not bound_hi:
            facts.append(z3.Implies(lo < hi, app == comb(F(lo, hi - 1, *outer), hi - 1, hi - 1 <= lo)))
            facts.append(z3.Implies(lo <= hi, F(lo, hi + 1, *outer) == comb(app, hi, hi <= lo)))
        if not bound_lo:
            facts.append(z3.Implies(lo < hi, app == comb(F(lo + 1, hi, *outer), lo, hi <= lo + 1)))
            facts.append(z3.Implies(lo <= hi, F(lo - 1, hi, *outer) == comb(app, lo - 1, hi <= lo)))
        if facts:
            L = z3.And(*facts)
            bs = [c for c in s.qstack if any(c.eq(o) for o in outer) or _occurs(lo, c) or _occurs(hi, c)]
            if bs:
                L = z3.ForAll(bs, L, patterns=[app])
            lk = L.sexpr()
            if lk not in s.lemma_keys:
                s.lemma_keys.add(lk)
                s.lemmas.append(L)
        return app

    def call(s, e, st, g, code, env):
        n = e.name
        if e.old is not None:
            # G5 A1: old(a) is the frozen entry array. A callee contract being instantiated at a call site supplies its
            # own pre-call arrays under ("old", name) in its environment; otherwise it is this function's snapshot.
            key = ("old", e.old["param"])
            return env[key] if key in env else s.snap[e.old["param"]]
        args = [s.ev(a, st, g, code, env) for a in e.args]
        if n == "len":
            return args[0].n
        if n == "size":     # G9 A1: contract-only structural size of a non-generic nominal value
            sty = s.model.type_of(args[0])
            s.observe(st, args[0], sty, force=True)
            return s.model.size_fn(sty.name)(args[0])
        if n == "alloc":
            s.prove(st, g, z3.And(args[0] >= 0, args[0] <= MAX_LEN), e, "alloc_negative",
                    "alloc length in 0..2^40")
            tname = getattr(e.ty, "name", None)
            # G2 A5: array_bool -> BOOL elements, no elem_range obligation
            aelem = U8 if tname == "array_u8" else BOOL if tname == "array_bool" else I64
            aty = s.T(e.ty)
            if aty is not None and aty.name == "array" and aty.args:   # G3: alloc of a [T]
                aelem = aty.args[0]
            if aelem == U8:
                s.prove(st, g, z3.And(args[1] >= 0, args[1] <= 255), e, "elem_range",
                        "u8 element range 0..=255 for the alloc fill value")
            s.spend(st, g, 1 + args[0])
            s.k += 1
            return mk_arr(z3.K(z3.IntSort(), args[1]), args[0], "alloc", aelem)
        if n == "copy":
            s.spend(st, g, 1 + args[0].n)
            res = mk_arr(args[0].a, args[0].n, "copy", args[0].elem)
            if getattr(e.args[0], "ty", None) == STR:     # LG26 (R-6b-3, G-LG25-1)
                s.assume_str(st, g, res)
            return res
        if n == "truncate":     # the first m elements of a, as a fresh array of length m
            av, m = args
            s.prove(st, g, z3.And(m >= 0, m <= av.n), e, "index_oob",
                    "truncate length 0 <= m <= len(%s)" % av.name)
            s.spend(st, g, 1 + m)
            return mk_arr(av.a, m, "truncate", av.elem)
        if n == "resize":       # P05G: a grown to length m; k < len(a) keeps a[k], len(a) <= k < m is v
            av, m, v = args
            minimum = 0 if getattr(e, "resize_scratch", False) else av.n
            bound = "0" if getattr(e, "resize_scratch", False) else "len(%s)" % av.name
            s.prove(st, g, z3.And(minimum <= m, m <= MAX_LEN), e, "index_oob",
                    "resize length %s <= n <= 2^40" % bound)
            if av.elem == U8:
                s.prove(st, g, z3.And(v >= 0, v <= 255), e, "elem_range",
                        "u8 element range 0..=255 for the resize fill value")
            s.spend(st, g, 1 + m)
            k = z3.Int("resize_k")
            return mk_arr(z3.Lambda([k], z3.If(k < av.n, av.a[k], v)), m, "resize", av.elem)
        if n == "eq_range":     # P16: five bounds obligations in order, then a fresh bool r with r == forall k in 0..n: a[i+k] == b[j+k] assumed on the call's path (the arrays as they are at the call)
            av, i, bv, j, cnt = args
            s.prove(st, g, i >= 0, e, "index_oob", "eq_range start 0 <= i for %s" % av.name)
            s.prove(st, g, j >= 0, e, "index_oob", "eq_range start 0 <= j for %s" % bv.name)
            s.prove(st, g, cnt >= 0, e, "index_oob", "eq_range length 0 <= n")
            s.prove(st, g, i + cnt <= av.n, e, "index_oob", "eq_range i + n <= len(%s)" % av.name)
            s.prove(st, g, j + cnt <= bv.n, e, "index_oob", "eq_range j + n <= len(%s)" % bv.name)
            r = s.fresh("eq_range", BOOL)
            s.k += 1
            k = z3.Int("eq_range!q%d" % s.k)
            s.assume(st, g, r == z3.ForAll([k], z3.Implies(z3.And(k >= 0, k < cnt),
                                                         z3.Select(av.a, i + k) == z3.Select(bv.a, j + k))))
            return r
        if n == "arr_reserve":
            cnt = args[1]
            s.prove(st, g, z3.And(cnt >= 0, cnt <= MAX_LEN), e, "call_requires", "precondition of arr_reserve")
            return None
        if n == "arr_push":
            av, v = args
            s.prove(st, g, av.n < MAX_LEN, e, "call_requires", "precondition of arr_push")
            if av.elem == U8:
                s.prove(st, g, z3.And(v >= 0, v <= 255), e, "elem_range", "u8 element range 0..=255 for the pushed value")
            name = e.args[0].expr.name
            st.env[name] = mk_arr(z3.Store(av.a, av.n, v), av.n + 1, name, av.elem)
            if av.elem == U8:
                s.assume(st, g, u8_dom(st.env[name]))
            s.post_refine(e, [0], st, g)
            return None
        if n == "arr_append":
            dv, sv, start, cnt = args
            need = dv.n + cnt
            s.prove(st, g, z3.And(start >= 0, cnt >= 0, start + cnt <= sv.n, need <= MAX_LEN),
                    e, "call_requires", "precondition of arr_append")
            p = z3.Int("arr_append!p")
            arr = z3.Lambda([p], z3.If(z3.And(p >= dv.n, p < need),
                                       z3.Select(sv.a, start + p - dv.n), z3.Select(dv.a, p)))
            root, path = field_place(e.args[0].expr)
            nv = mk_arr(arr, need, ".".join((root.name,) + path), dv.elem)
            st.env[root.name] = s.rebuild(root.ty, st.env[root.name], path, nv, st, g) if path else nv
            if nv.elem == U8:
                s.assume(st, g, u8_dom(nv))
            s.post_refine(e, [0], st, g)
            return None
        if n in ("copy_range", "copy_within"):
            if n == "copy_within":
                dv, i, j, cnt = args
                sv = dv
                dst, src = "dst", "src"
            else:
                dv, i, sv, j, cnt = args
                dst, src = "i", "j"
            s.prove(st, g, i >= 0, e, "index_oob", "%s start 0 <= %s for %s" % (n, dst, dv.name))
            s.prove(st, g, j >= 0, e, "index_oob", "%s start 0 <= %s for %s" % (n, src, sv.name))
            s.prove(st, g, cnt >= 0, e, "index_oob", "%s length 0 <= n" % n)
            s.prove(st, g, i + cnt <= dv.n, e, "index_oob", "%s %s + n <= len(%s)" % (n, dst, dv.name))
            s.prove(st, g, j + cnt <= sv.n, e, "index_oob", "%s %s + n <= len(%s)" % (n, src, sv.name))
            p = z3.Int(n + "!p")
            # a definitional (array-theory) update, no quantified assumption: the content fact dst'[i+k] == src[j+k] for 0 <= k < n,
            # the frame fact dst'[p] == dst[p] outside [i, i+n), and len(dst') == len(dst) hold by construction
            arr = z3.Lambda([p], z3.If(z3.And(p >= i, p < i + cnt), z3.Select(sv.a, j + p - i), z3.Select(dv.a, p)))
            root, path = field_place(e.args[0].expr)
            nv = mk_arr(arr, dv.n, ".".join((root.name,) + path), dv.elem)
            st.env[root.name] = s.rebuild(root.ty, st.env[root.name], path, nv, st, g) if path else nv
            if nv.elem == U8:       # both operands are in the byte domain, so the result is too
                s.assume(st, g, u8_dom(nv))
            s.post_refine(e, [0], st, g)
            return None
        if n in ("load_be", "load_le"):
            av, i = args[0], args[1]
            nb = e.args[2].value
            if code:
                s.prove(st, g, z3.And(i >= 0, i + nb <= av.n), e, "index_oob",
                        "word in bounds for %s[i..i+%d]" % (av.name, nb))
            bs = [z3.Int2BV(z3.Select(av.a, i + k), 8) for k in range(nb)]
            if n == "load_le":
                bs.reverse()
            u = z3.BV2Int(bs[0] if nb == 1 else z3.Concat(*bs))
            return z3.If(u < 2 ** 63, u, u - 2 ** 64) if nb == 8 else u
        if n in ("store_be", "store_le"):
            av, i, w = args[0], args[1], args[2]
            nb = e.args[3].value
            s.prove(st, g, z3.And(i >= 0, i + nb <= av.n), e, "index_oob",
                    "word in bounds for %s[i..i+%d]" % (av.name, nb))
            wb = z3.Int2BV(w, 64)
            arr = av.a
            for k in range(nb):
                sh = 8 * (k if n == "store_le" else nb - 1 - k)
                arr = z3.Store(arr, i + k, z3.BV2Int(z3.Extract(sh + 7, sh, wb)))
            root, path = field_place(e.args[0].expr)
            name = root.name
            nv = mk_arr(arr, av.n, ".".join((name,) + path), av.elem)
            st.env[name] = s.rebuild(root.ty, st.env[name], path, nv, st, g) if path else nv
            s.post_refine(e, [0], st, g)
            return None
        if n == "is_char_boundary":
            return str_boundary(args[0], args[1])
        if n in BIT_Z3:     # P2: exact 64-bit two's-complement result as a signed Int
            x, y = args
            r = signed_bv(BIT_Z3[n](z3.Int2BV(x, 64), z3.Int2BV(y, 64)))
            if code and n in ("wrapping_shl", "ushr"):    # shift count must be in 0..64
                s.prove(st, g, z3.And(y >= 0, y < 64), e, "shift_range", "shift count in 0..64")
            if code:
                s.assume(st, g, z3.And(r >= -2 ** 63, r <= 2 ** 63 - 1))
                if n == "band":
                    s.assume(st, g, z3.Implies(y >= 0, z3.And(r >= 0, r <= y)))
                    s.assume(st, g, z3.Implies(x >= 0, z3.And(r >= 0, r <= x)))
                elif n == "wrapping_shl":
                    s.assume(st, g, z3.Implies(y == 0, r == x))
                elif n == "ushr":
                    s.assume(st, g, z3.Implies(y == 0, r == x))
                    s.assume(st, g, z3.Implies(x >= 0, z3.And(0 <= r, r <= x)))
                    s.assume(st, g, z3.Implies(y > 0, r >= 0))
                    c = e.args[1].value if isinstance(e.args[1], IntLit) else None
                    if c is not None and 0 < c < 64:    # literal shift count
                        s.assume(st, g, r <= 2 ** (64 - c) - 1)
                        s.assume(st, g, r == z3.If(x < 0, x + 2 ** 64, x) / 2 ** c)
                elif n in ("bor", "bxor"):    # P2 batch 3: wrapping_add / wrapping_mul get no extra facts
                    s.assume(st, g, z3.Implies(z3.And(x >= 0, y >= 0), z3.And(r >= 0, r <= x + y)))
            return r
        if n == "utf8_valid":
            return s.model.utf8_valid(args[0])
        if n == "utf8_valid_span":      # LG26 (R-6b-3, G-LG25-1): contract only, no domain obligation
            b, lo, hi = args
            return z3.And(lo >= 0, lo <= hi, hi <= b.n, utf8_window(lambda t: z3.Select(b.a, lo + t), hi - lo))
        if n == "str_from_utf8":
            b = args[0]
            s.prove(st, g, s.model.utf8_valid(b), e, "utf8_invalid", "valid UTF-8 bytes in %s" % b.name)
            res = s.fresh("str_from_utf8", STR)
            s.assume(st, g, res.n == b.n)
            k = z3.Int("str!k")
            s.assume(st, g, z3.ForAll([k], z3.Implies(z3.And(k >= 0, k < res.n),
                                                       z3.Select(res.a, k) == z3.Select(b.a, k))))
            return s.assume_str(st, g, res)     # LG26 (R-6b-3, G-LG25-1)
        if n == "subview":
            base, lo, hi = args
            to_str = getattr(e, "ty", None) == STR   # LG25 (R-6g-10): a view passed to a str parameter
            from_str = to_str and getattr(e.args[0], "ty", None) == STR
            if from_str:
                s.prove(st, g, z3.And(lo >= 0, lo <= hi, hi <= base.n), e, "string_slice_range",
                        "subview range 0 <= lo <= hi <= len(%s)" % base.name)
                s.prove(st, g, z3.And(str_boundary(base, lo), str_boundary(base, hi)), e, "utf8_boundary",
                        "subview cuts of %s at UTF-8 character boundaries" % base.name)
            else:
                s.prove(st, g, z3.And(lo >= 0, lo <= hi, hi <= base.n), e, "index_oob",
                        "subview range 0 <= lo <= hi <= len(%s)" % base.name)
            p = z3.Int("subview!p")
            span = mk_arr(z3.Lambda([p], z3.Select(base.a, lo + p)), hi - lo, "subview", base.elem)
            if to_str and not from_str:
                cb = z3.And(str_boundary(base, lo), str_boundary(base, hi))
                s.prove(st, g, z3.Or(s.model.utf8_valid(span), z3.And(s.model.utf8_valid(base), cb)), e,
                        "utf8_invalid", "valid UTF-8 bytes in subview of %s" % base.name)
            if to_str:
                return s.assume_str(st, g, span)   # LG26: a str view is a valid str (as Rust)
            return span
        if n == "substr":   # G2 B1 (G2_SPEC 4.4): range -> boundary -> fresh result
            sv, lo, hi = args
            s.prove(st, g, z3.And(lo >= 0, lo <= hi, hi <= sv.n), e, "string_slice_range",
                    "substr range 0 <= lo <= hi <= len(%s)" % sv.name)
            s.prove(st, g, z3.And(str_boundary(sv, lo), str_boundary(sv, hi)), e, "utf8_boundary",
                    "substr cuts of %s at UTF-8 character boundaries" % sv.name)
            res = s.fresh("substr", STR)
            s.assume(st, g, res.n == hi - lo)
            k = z3.Int("str!k")
            s.assume(st, g, z3.ForAll([k], z3.Implies(z3.And(k >= 0, k < res.n),
                                                       z3.Select(res.a, k) == z3.Select(sv.a, lo + k))))
            return s.assume_str(st, g, res)     # LG26 (R-6b-3, G-LG25-1)
        if n in CATALOG_NAMES and n not in s.funcs:
            return s.catalog_call(e, args, st, g, code)
        if n == "print":
            return None
        if n in ("is_some", "some_val"):
            o = s.model.opt(s.model.type_of(args[0]))     # G3: the datatype of the argument's own sort
            return o.is_some(args[0]) if n == "is_some" else o.sval(args[0])
        if n in ("is_ok", "ok_val", "err_val"):
            ty = s.model.type_of(args[0])
            o = s.model.res(ty)
            if n == "err_val" and result_error(ty) == _STRING:
                return s.model.field_get(_STRING, o.eval(args[0]), "err_val")
            return o.is_ok(args[0]) if n == "is_ok" else o.oval(args[0]) if n == "ok_val" else o.eval(args[0])
        f = s.funcs[n]
        if f.opaque and not code:   # P13o: a contract application of an opaque function is its symbol
            return s.opq(f, args, st, g)
        cenv = {p.name: a for p, a in zip(f.params, args)}
        # G5 A1 (5.2 step 2): the pre-call arrays freeze as the callee's old bindings (scoped to this contract
        # evaluation: the caller's own snapshot map is never touched)
        ios = [i for i, p in enumerate(f.params) if p.mode is not None]
        for i in ios:
            cenv[("old", f.params[i].name)] = args[i]
        # G3 A2 (4.4): typed re-interpretation of the callee contract under the checker's recorded substitution
        theta = e.inst["subst"] if getattr(e, "inst", None) else {}
        saved, s.theta = s.theta, theta
        try:
            if code:
                # No domain proof of callee requires here: the callee's own run() already proved
                # D(r) under "param wf + earlier requires (values and variant domains)"; the call site
                # proves each earlier requires value and the args satisfy wf, so it would be redundant.
                s.prove_call_requires(e, f, st, g, cenv)
                s.termination(e, f, st, g, cenv)    # G9 A1: only a call inside the caller's own recursive SCC
                s.call_cost(e, f, st, g, cenv)
            if ios:
                # R-6g-1: code evaluates this call once in place, on the enclosing statement's path.
                s.inout_finals(e, f, ios, args, cenv, st, g)    # G5 A1 (5.2 step 5): fresh same-length contents
            if f.ret == UNIT:
                # G6 A1 (11.4): a trusted unit result still assumes its ensures; G9 A1: so does a member of a recursive
                # SCC, after the termination obligations above (also when called from outside the SCC)
                # G4 A1 (G4_SPEC 5): in explicit visibility mode every ordinary unit call assumes its ensures, after
                # the same requires/termination obligations; legacy programs keep their exact old behavior and SMT.
                # G5 A1: every inout call applies its ensures, in legacy single-file mode too
                if ios or f.foreign is not None or n in s.scc_id or s.explicit:
                    for en in s.inv_of(e, f)[1] + f.ensures:
                        s.assume(st, g, s.ev(en, st, g, False, cenv))
                if ios:
                    s.post_refine(e, ios, st, g)
                return None
            rty = substitute(f.ret, theta) if theta else f.ret
            res = s.fresh("ret_" + n, rty)
            if rty == STR:      # LG26 (R-6b-3, G-LG25-1)
                s.assume_str(st, g, res)
            if isinstance(res, Arr):
                s.assume(st, g, z3.And(res.n >= 0, res.n <= MAX_LEN))
                if res.elem == U8:
                    s.assume(st, g, u8_dom(res))
                elif f.foreign is not None and rty == Type("vec", (I64,)):   # G6b: a fresh foreign Vec<i64>
                    s.assume(st, g, i64_dom(res))
            elif isinstance(res, Map):
                s.assume(st, g, z3.And(res.n >= 0, res.n <= MAX_LEN))
            else:
                w = s.wf(rty, res)
                if w is not None:
                    s.assume(st, g, w)
            xa = getattr(f, "x_auto", None)
            # P07: the result equals the template over the arguments. For read helpers of borrowing
            # structs, the call's proved requires cover the template's domain. Evaluate before "result" is bound,
            # so a parameter named result keeps its argument value.
            if code and f.opaque:   # P13o: the result is the symbol; its body is known only where F is revealed
                xv = s.opq(f, args, st, g)
            else:
                xv = s.ev(xa, st, g, False, cenv) if code and xa is not None else None
            cenv["result"] = res
            for en in s.inv_of(e, f)[1] + f.ensures:
                s.assume(st, g, s.ev(en, st, g, False, cenv))
            if xv is not None:
                s.assume(st, g, res == xv)
            if ios:
                s.post_refine(e, ios, st, g)
            return res
        finally:
            s.theta = saved

    def inv_of(s, node, f):
        """LG24: the struct invariant (requires, ensures) at a call: the instantiated ones of a generic call, else f's"""
        return getattr(node, "inv_inst", None) or (f.inv_requires, f.inv_ensures)

    def prove_call_requires(s, node, f, st, g, env):
        prev = []   # earlier invariant clauses (domain + value) proven above may justify later lookups
        for i, r in enumerate(s.inv_of(node, f)[0]):
            name, line, param = r.struct_invariant
            what = "invariant of %s (line %d) for parameter %s of %s" % (name, line, param, node.name)
            for dom, code in s.dom_obligations(r, st, g + prev, env):
                s.prove(st, g + prev, dom, node, "struct_invariant", what, key_extra=(i, code))
                prev.append(dom)
            ob = s.ev(r, st, g, False, env)
            s.prove(st, g, ob, node, "struct_invariant", what, key_extra=i)
            prev.append(ob)
        for r in f.requires:
            ob = s.ev(r, st, g, False, env)
            s.prove(st, g, ob, node, "call_requires", "precondition of %s (line %d)" % (node.name, r.line))

    def opq(s, f, args, st, g):
        """P13o (appendix P13o.1): the application of the opaque function f to the argument values: the
        uninterpreted opq!F over the argument encodings (an array is its contents and its length). Under a quantifier
        whose bound variable reaches an argument the application is expanded in place instead (a reveal equation
        cannot mention a bound variable). A new application is recorded on the path; if F is revealed on the path
        its equation is appended to pc at once.
        LG30 (G-LG29-1): every occurrence is recorded with its path guard g (one entry per application and guard);
        the body is evaluated under that guard, so the facts it pushes (W of a str read) hold only where the call
        is reached, while the definition equation app == body is pushed once per application, unguarded."""
        if s.qstack and any(_occurs(t, c) for t in _opq_parts(args) for c in s.qstack):
            return s.opq_body(f, args, st, g)
        parts = _opq_parts(args)
        fn = s.opq_fns.get(f.name)
        if fn is None:
            fn = z3.Function("opq!" + f.name, *([t.sort() for t in parts] + [z3.BoolSort()]))
            s.opq_fns[f.name] = fn
        app = fn(*parts)
        i = app.get_id()
        key = (i, tuple(c.get_id() for c in g))
        if key not in st.app_ids:
            first = not any(a.get_id() == i for _, a, _, _, _ in st.apps)
            st.apps = st.apps + ((f, app, tuple(args), tuple(g), tuple(s.qstack)),)
            st.app_ids = st.app_ids | {key}
            if f.name in st.rev:
                s.opq_occ(f, app, args, st, g, tuple(s.qstack), first)
        return app

    def opq_occ(s, f, app, args, st, g, qs, first):
        """LG30 (G-LG29-1): the body of one recorded occurrence, evaluated under its guard g (its facts go to pc
        under g); the first occurrence of an application also pushes the unguarded equation app == body. The
        quantifier constants qs enclosing the occurrence are bound again while the body is evaluated, so a guard over
        a bound variable stays quantified on a later reveal (the arguments never mention them)."""
        outer = s.qstack
        s.qstack = list(qs)
        try:
            b = s.opq_body(f, args, st, g)
        finally:
            s.qstack = outer
        if first:
            st.pc.append(app == b)

    def opq_body(s, f, args, st, g):
        """P13o: the body of the opaque f over the argument values (its contract template; opaque calls in it stay
        symbols, so a reveal does not cascade). LG30 (G-LG29-1): evaluated under the guard g of the occurrence."""
        return s.ev(f.x_auto, st, list(g), False, {p.name: a for p, a in zip(f.params, args)})

    def reveal(s, stmt, st):
        """P13o: `reveal F;` makes F transparent on the rest of this path: the equation of every application already
        met, in the order met; later applications add theirs when met (opq)"""
        st = st.fork()
        name = stmt.reveal.name
        if name in st.rev:
            return [st]
        st.rev = st.rev | {name}
        seen = set()
        for f, app, args, g, qs in st.apps:
            if f.name == name:
                s.opq_occ(f, app, list(args), st, g, qs, app.get_id() not in seen)
                seen.add(app.get_id())
        return [st]

    def hidden(s, st):
        """P13o: the opaque functions applied on this path and not revealed, sorted"""
        return sorted({f.name for f, _, _, _, _ in st.apps if f.name not in st.rev})

    def inout_finals(s, e, f, ios, args, cenv, st, g):
        """G5 A1 (G5_SPEC 5.2 step 5): fresh contents for every inout argument, retaining unchanged array
        lengths. The final value rebinds the caller's marked place and the callee's parameter in ensures. All finals exist
        before any ensures clause is assumed, so relational ensures may connect several of them."""
        for i in ios:
            pre = args[i]
            root, path = field_place(e.args[i].expr)
            name = root.name
            ty = substitute(f.params[i].type, s.theta)
            if isinstance(pre, Arr) and (f.name, i) not in s.model.vec_resize_params:
                fv = s.fresh(name, ty if ty.name == "vec" else arr_type(pre.elem))
                nv = mk_arr(fv.a, pre.n, name, pre.elem)
                if ty == STR:
                    s.assume_str(st, g, nv)
                elif nv.elem == U8:
                    s.assume(st, g, u8_dom(nv))
            else:
                nv = s.fresh(name, ty)
                wf = s.wf(ty, nv)
                if wf is not None:
                    s.assume(st, g, wf)
                if isinstance(nv, Arr):     # LG5: a generic [T] has no wf, so its new length gets the entry bounds
                    s.assume(st, g, z3.And(nv.n >= 0, nv.n <= MAX_LEN))
                for field_path in s.model.struct_len_frames.get((f.name, i), ()):
                    before, after = pre, nv
                    for field in field_path:
                        before = s.read_field(st, before, field)
                        after = s.read_field(st, after, field)
                    s.assume(st, g, after.n == before.n)
            cenv[f.params[i].name] = nv
            st.env[name] = s.rebuild(root.ty, st.env[name], path, nv, st, g) if path else nv

    def post_refine(s, e, ios, st, g):
        """G5 A1 (G5_SPEC 5.2 step 8): the refinement of every marked refined local must hold right after the call
        (refinement_assign, proved from the callee ensures and the remaining facts; never assumed beforehand). The
        assertions were attached to the inout argument by refine.py."""
        for i in ios:
            for a in getattr(e.args[i], "post_refine", None) or ():
                st.pc.append(s.check_assert(a, st))

    def par(s, p, st):
        """G7 A1 (G7_SPEC 5): one par group. (1) every branch's actuals are evaluated against the group-entry state and
        frozen (callee old = that actual); (2) every branch requires (and applicable recursive decreases) is proven in
        source order against that same entry state, before any ensures is assumed (prove never adds to the path
        condition, so nothing of a sibling is available either); (3) all inout finals of all branches are created with
        the F3 transition (same length, element type, u8 domain) before any ensures; (4) every branch's ensures is
        conjoined into the joined state, also for legacy unit branches without inout; (5) each branch's refined locals
        are checked on a fork that assumes only that branch's own ensures. Unwritten bindings keep their values."""
        st = st.fork()
        brs = []
        for c in p.calls:
            f = s.funcs[c.name]
            args = [s.ev(a, st, [], True) for a in c.args]
            cenv = {pm.name: a for pm, a in zip(f.params, args)}
            ios = [i for i, pm in enumerate(f.params) if pm.mode is not None]
            for i in ios:
                cenv[("old", f.params[i].name)] = args[i]
            theta = c.inst["subst"] if getattr(c, "inst", None) else {}
            brs.append((c, f, args, cenv, ios, theta))
        for c, f, args, cenv, ios, theta in brs:
            saved, s.theta = s.theta, theta
            try:
                s.prove_call_requires(c, f, st, [], cenv)
                s.termination(c, f, st, [], cenv)
                s.call_cost(c, f, st, [], cenv)
            finally:
                s.theta = saved
        for c, f, args, cenv, ios, theta in brs:
            if ios:
                saved, s.theta = s.theta, theta     # LG5: a generic resizable final takes the branch's element type
                try:
                    s.inout_finals(c, f, ios, args, cenv, st, [])
                finally:
                    s.theta = saved
        base = st.fork()        # all finals exist, no ensures assumed yet
        for c, f, args, cenv, ios, theta in brs:
            saved, s.theta = s.theta, theta
            try:
                for en in s.inv_of(c, f)[1] + f.ensures:
                    s.assume(st, [], s.ev(en, st, [], False, cenv))
            finally:
                s.theta = saved
        for c, f, args, cenv, ios, theta in brs:
            if not any(getattr(c.args[i], "post_refine", None) for i in ios):
                continue
            b = base.fork()
            saved, s.theta = s.theta, theta
            try:
                for en in s.inv_of(c, f)[1] + f.ensures:
                    s.assume(b, [], s.ev(en, b, [], False, cenv))
            finally:
                s.theta = saved
            for i in ios:
                for a in getattr(c.args[i], "post_refine", None) or ():
                    st.pc.append(s.check_assert(a, b))
        return st

    def spend(s, st, g, units):
        # Ghost arithmetic is mathematical, including across loop havoc.
        if s.f.cost is not None:
            st.env["spent"] = st.env["spent"] + (z3.If(z3.And(*g), units, 0) if g else units)

    def cost_value(s, expr, st, g, node, env):
        domain = s.dom(expr, st, g, env)
        if domain is not None:
            s.prove(st, g, domain, node, "contract_domain", "cost `%s` is defined" % _expr_text(expr))
        return s.ev(expr, st, g, False, env)

    def call_cost(s, e, f, st, g, cenv):
        if s.f.cost is not None:
            bound = s.cost_value(f.cost, st, g, e, cenv)
            s.spend(st, g, 1 + bound)

    def termination(s, e, f, st, g, cenv):
        """G9 A1 (G9_SPEC 5): the obligations of a call s.f -> f inside one recursive SCC, after the callee requires:
        definedness of the instantiated callee measure (existing domain codes), M_f(actuals) >= 0
        (recursive_nonneg), M_f(actuals) < M_s.f(frozen entry value) (recursive_decrease). Only after these the
        caller introduces the fresh result and assumes the callee ensures (so the call's own ensures never justify
        its own decrease or precondition). cenv: the callee parameters bound to the actual argument values."""
        a, b = s.scc_id.get(f.name), s.scc_id.get(s.f.name)
        if a is None or a != b or f.decreases is None or s.m0 is None:
            return
        s.check_dom(st, g, f.decreases, e, "variant_field lookups in the decreases clause of %s are defined" % f.name,
                    cenv)
        mg = s.ev(f.decreases, st, g, False, cenv)
        tg, tf = _expr_text(f.decreases), _expr_text(s.f.decreases)
        s.prove(st, g, mg >= 0, e, "recursive_nonneg",
                "decreases measure of %s (`%s`) >= 0 at this call" % (f.name, tg))
        s.prove(st, g, mg < s.m0, e, "recursive_decrease",
                "decreases measure of %s (`%s`) < decreases measure of %s at its entry (`%s`)" % (f.name, tg, s.f.name, tf))

    def catalog_need(s, st, g, e, code, n):
        """need(ob) for a catalog call n (fs, proc, async, net): ob is a `call_requires` obligation, only in code."""
        def need(ob):
            if code:
                s.prove(st, g, ob, e, "call_requires", "precondition of %s (catalog)" % n)
        return need

    def catalog_call(s, e, args, st, g, code):
        """G6 A1 (G6_SPEC 11.3): the standard catalog; every requires is a `call_requires` obligation
        (only in code), then an exact term is the result. G10 fs calls return fresh values with catalog ensures."""
        n = e.name
        rty = s.T(e.ty)

        need = s.catalog_need(st, g, e, code, n)

        if n.startswith("proc_"):
            return s.proc_call(n, e, args, st, g, code, rty)
        if n.startswith("ev_"):
            return s.async_call(n, e, args, st, g, code, rty)
        if n.startswith("net_"):
            return s.net_call(n, e, args, st, g, code, rty)
        if n in FILE_OBSERVERS:
            f = args[0]
            if n == "file_size":
                return f.buf.n
            if n == "file_byte":
                i = args[1]
                need(z3.And(i >= 0, i < f.buf.n))
                r = z3.Select(f.buf.a, i)
                if code:
                    s.assume(st, g, z3.And(r >= 0, r <= 255))
                return r
            return getattr(f, n[len("file_"):])
        if n.startswith("file_"):
            f = CATALOG[n]
            # Keep the entry arguments, including a consumed handle, for file_close's ensures.
            cenv = {p.name: a for p, a in zip(f.params, args)}
            for req in f.requires:
                need(s.ev(req, st, g, False, cenv))
            res = s.fresh(n, rty)
            w = s.wf(rty, res)
            if w is not None:
                s.assume(st, g, w)
            cenv["result"] = res
            for en in f.ensures:
                s.assume(st, g, s.ev(en, st, g, False, cenv))
            return res
        if n in ("vec_new", "string_new", "map_new"):
            r = s.fresh(n, rty)
            if n == "map_new":
                kt = z3.K(s.model.map_key_sort(r.kty), z3.BoolVal(False))
                return Map(kt, r.a, z3.IntVal(0), n, r.kty, r.vty)
            empty = mk_arr(r.a, z3.IntVal(0), n, r.elem)
            if r.elem in s.model.vec_sums:
                s.vec_sum(st, g, empty)
            return empty
        if n == "vec_push":
            v, x = args
            need(v.n < MAX_LEN)
            r = mk_arr(z3.Store(v.a, v.n, s.model.field_put(v.elem, x)), v.n + 1, n, v.elem)
            if v.elem in s.model.vec_sums:
                s.observe(st, x, v.elem)
                s.assume(st, g, z3.Implies(z3.And(v.n >= 0, v.n < MAX_LEN),
                    s.vec_sum(st, g, r) == s.vec_sum(st, g, v) + s.model.size_fn(v.elem.name)(x)))
            return r
        if n == "vec_reserve":
            cnt = args[1]
            need(z3.And(cnt >= 0, cnt <= MAX_LEN))
            return None
        if n == "vec_clear":
            v = args[0]
            root, path = field_place(e.args[0].expr)
            empty = mk_arr(v.a, z3.IntVal(0), v.name, v.elem)
            if v.elem in s.model.vec_sums:
                s.vec_sum(st, g, empty)
            st.env[root.name] = s.rebuild(root.ty, st.env[root.name], path, empty, st, g)
            return None
        if n == "vec_extend_from_slice":   # D95: old prefix kept, suffix equals the slice
            v, src = args
            need(v.n + src.n <= MAX_LEN)
            root, path = field_place(e.args[0].expr)
            r = s.fresh(n, e.args[0].expr.ty)
            k = z3.Int("vec_extend!q%d" % s.k)
            s.k += 1
            s.assume(st, g, z3.ForAll([k], z3.Implies(z3.And(k >= 0, k < v.n),
                                                      z3.Select(r.a, k) == z3.Select(v.a, k))))
            s.assume(st, g, z3.ForAll([k], z3.Implies(z3.And(k >= 0, k < src.n),
                                                      z3.Select(r.a, v.n + k) == z3.Select(src.a, k))))
            out = mk_arr(r.a, v.n + src.n, v.name, v.elem)
            st.env[root.name] = s.rebuild(root.ty, st.env[root.name], path, out, st, g)
            return None
        if n == "string_eq":
            a, b = args
            r = s.fresh(n, BOOL)
            k = z3.Int("string_eq!q%d" % s.k)
            s.k += 1
            equal_bytes = z3.ForAll([k], z3.Implies(z3.And(k >= 0, k < a.n),
                                                   z3.Select(a.a, k) == z3.Select(b.a, k)))
            s.assume(st, g, r == z3.And(a.n == b.n, equal_bytes))
            return r
        if n in ("vec_get", "string_byte"):
            v, i = args
            bounds = z3.And(i >= 0, i < v.n)
            need(bounds)
            r = z3.Select(v.a, i)
            if v.elem == _STRING:
                text = s.model.field_get(_STRING, r, n)
                if code:
                    s.assume(st, g, s.wf(_STRING, text))
                return text
            if code and v.elem in (I64, U8):   # as an array read: an i64/u8 element is a machine integer
                s.assume(st, g, in_range(r))
            if v.elem in s.model.vec_sums:
                s.observe(st, r, v.elem)
                s.observe_vec_element(st, g, v, i, r)
            elif code and s.model.tt.is_struct(v.elem):
                wf = s.wf(v.elem, r)
                if wf is not None:
                    s.assume(st, g, wf)
            return r
        if n in ("map_get", "map_lookup"):
            m, k = args
            return s.map_lookup(m, s.model.map_key(m.kty, k), rty)
        if n in ("map_remove", "map_take"):
            m, k = args
            k = s.model.map_key(m.kty, k)
            has = z3.Select(m.d, k)
            s.assume(st, g, z3.Implies(has, m.n > 0))
            # A present entry witnesses a nonempty map; the abstract length is
            # otherwise independent of the presence array.
            new = Map(z3.Store(m.d, k, z3.BoolVal(False)), m.a, m.n - z3.If(has, 1, 0),
                      m.name, m.kty, m.vty)
            root, path = field_place(e.args[0].expr)
            st.env[root.name] = s.rebuild(root.ty, st.env[root.name], path, new, st, g)
            return s.map_lookup(m, k, rty) if n == "map_take" else has
        if n == "map_insert":
            m, k, x = args
            k = s.model.map_key(m.kty, k)
            has = z3.Select(m.d, k)
            need(z3.Or(has, m.n < MAX_LEN))
            return Map(z3.Store(m.d, k, z3.BoolVal(True)), z3.Store(m.a, k, x), m.n + z3.If(has, 0, 1), n, m.kty,
                       m.vty)
        if n == "map_updated":
            aft, bef, k, x = args
            j = z3.Const("map!j", s.model.sort_of(aft.kty))
            return z3.And(aft.d == z3.Store(bef.d, k, z3.BoolVal(True)),
                          aft.n == bef.n + z3.If(z3.Select(bef.d, k), 0, 1),
                          z3.ForAll([j], z3.Implies(z3.Select(aft.d, j),
                                                    z3.Select(aft.a, j) == z3.Select(z3.Store(bef.a, k, x), j))))
        if n == "string_from":
            t = args[0]
            need(t.n <= MAX_LEN)
            res = s.fresh(n, _STRING)
            s.assume(st, g, res.n == t.n)
            k = z3.Int("str!k")
            s.assume(st, g, z3.ForAll([k], z3.Implies(z3.And(k >= 0, k < t.n),
                                                       z3.Select(res.a, k) == z3.Select(t.a, k))))
            return res
        if n == "string_push_str":
            sv, t = args
            need(sv.n + t.n <= MAX_LEN)
            res = s.fresh(n, _STRING)
            s.assume(st, g, res.n == sv.n + t.n)
            k = z3.Int("str!k")
            s.assume(st, g, z3.ForAll([k], z3.Implies(z3.And(k >= 0, k < sv.n),
                                                       z3.Select(res.a, k) == z3.Select(sv.a, k))))
            s.assume(st, g, z3.ForAll([k], z3.Implies(z3.And(k >= 0, k < t.n),
                                                       z3.Select(res.a, sv.n + k) == z3.Select(t.a, k))))
            return res
        if n == "string_to_str":
            return s.assume_str(st, g, mk_arr(args[0].a, args[0].n, n, U8))   # LG27 (G-LG26-2)
        raise Exception("catalog: " + n)

    def proc_call(s, n, e, args, st, g, code, rty):
        """G10 proc A1 (G10_PROC_SPEC 2.3, 2.4, 4): the catalog contracts of the proc operations. The requires of an
        operation are call_requires obligations in order (code only); an effect operation then gets a fresh result with
        its well-formedness and the ensures facts below. The argument builder and the observers are exact terms.
        Parameter observers in the ensures denote the values at call entry (the actuals in args). Two identical calls
        are never equated: every effect result is a fresh symbol."""
        need = s.catalog_need(st, g, e, code, n)

        # ProcessArgs: argc only
        if n == "proc_args_new":
            return PArgs(z3.IntVal(0))
        if n == "proc_args_push":
            a = args[0]
            need(a.argc < MAX_LEN)
            return PArgs(a.argc + 1)
        if n == "proc_args_len":
            return args[0].argc
        # observers: field reads and array selects
        if n == "proc_ok":
            return args[0].error == 0
        if n == "proc_error":
            return args[0].error
        if n == "proc_stdin_open":
            return args[0].stdin_open
        if n in _PROC_DOM:
            p, sm = args[0], args[1]
            need(_proc_stream_ok(sm))
            if n == "proc_eof":
                return _proc_eof(p, sm)
            if n == "proc_size":
                return _proc_size(p, sm)
            i = args[2]
            need(z3.And(i >= 0, i < _proc_size(p, sm)))
            r = _proc_byte(p, sm, i)
            if code:
                s.assume(st, g, z3.And(r >= 0, r <= 255))
            return r
        if n == "proc_bytes":       # 4.6: a new array with the snapshot's size and byte map
            p, sm = args
            need(_proc_stream_ok(sm))
            return mk_arr(z3.If(sm == 1, p.out.a, p.err.a), _proc_size(p, sm), n, U8)
        # effect operations: requires -> fresh result -> well-formedness -> ensures
        p = args[0]
        if n in ("proc_spawn", "proc_spawn_env"):
            pass
        elif n in ("proc_write_str", "proc_write_bytes", "proc_close_stdin"):
            need(z3.And(p.error == 0, p.stdin_open))
        elif n == "proc_read":
            need(p.error == 0)
            need(_proc_stream_ok(args[1]))
            need(z3.And(args[2] > 0, args[2] <= MAX_LEN))
        elif n == "proc_read_all":
            need(z3.And(p.error == 0, z3.Not(p.stdin_open)))
        res = s.fresh(n, rty)
        s.assume(st, g, s.wf(rty, res))
        if n in ("proc_wait", "proc_kill"):     # 4.7: the existing Result<i64> encoding; Ok(0) is never assumed
            o = s.model.res(None)
            s.assume(st, g, z3.Implies(p.error != 0, res == o.err(p.error)))
            s.assume(st, g, z3.Implies(o.is_err(res), z3.And(o.eval(res) >= 1, o.eval(res) <= 6)))
            return res
        ok = res.error == 0
        if n in ("proc_spawn", "proc_spawn_env"):
            s.assume(st, g, z3.Implies(ok, res.stdin_open))
            s.assume(st, g, z3.Implies(z3.Not(ok), z3.And(z3.Not(res.stdin_open), z3.Or(res.error == 1, res.error == 2))))
            s.assume(st, g, z3.And(res.out.n == 0, res.err.n == 0, z3.Not(res.stdout_eof), z3.Not(res.stderr_eof)))
        elif n in ("proc_write_str", "proc_write_bytes"):
            s.assume(st, g, z3.Implies(ok, z3.And(res.stdin_open == p.stdin_open, _proc_keep_out(res, p),
                                                  _proc_keep_err(res, p))))
        elif n == "proc_close_stdin":
            s.assume(st, g, z3.Not(res.stdin_open))
            s.assume(st, g, z3.Implies(ok, z3.And(_proc_keep_out(res, p), _proc_keep_err(res, p))))
        elif n == "proc_read":
            sm, mx = args[1], args[2]
            sz = _proc_size(res, sm)
            s.assume(st, g, z3.Implies(ok, z3.And(sz >= 0, sz <= mx)))
            s.assume(st, g, z3.Implies(ok, _proc_eof(res, sm) == (sz == 0)))
            s.assume(st, g, z3.Implies(z3.And(ok, _proc_eof(p, sm)), sz == 0))
            s.assume(st, g, z3.Implies(z3.And(ok, sm == 1), _proc_keep_err(res, p)))
            s.assume(st, g, z3.Implies(z3.And(ok, sm == 2), _proc_keep_out(res, p)))
            s.assume(st, g, z3.Implies(ok, res.stdin_open == p.stdin_open))
        elif n == "proc_read_all":
            s.assume(st, g, z3.Not(res.stdin_open))
            s.assume(st, g, z3.Implies(ok, z3.And(res.stdout_eof, res.stderr_eof)))
        else:
            raise Exception("catalog: " + n)
        return res

    def async_call(s, n, e, args, st, g, code, rty):
        """G10 async section 4: requires, fresh snapshot, well-formedness, transitions."""
        need = s.catalog_need(st, g, e, code, n)
        into = n in ("ev_read_into", "ev_proc_read_into")
        if n != "ev_new": p = args[0]
        if n in ('ev_ok',):
            return p.error == 0
        if n in ('ev_error',):
            return p.error
        if n in ('ev_cap',):
            return p.cap
        if n in ('ev_count',):
            return p.count
        if n in ('ev_last',):
            return p.last
        if n in ('ev_event_slot',):
            return p.ev_slot
        if n in ('ev_event_kind',):
            return p.ev_kind_
        if n in ('ev_exit',):
            return p.exit
        if n in ('ev_size',):
            return p.buf.n
        if n in ('ev_kind', 'ev_eof', 'ev_write_open', 'ev_byte'):
            i = args[1]
            need(z3.And(i >= 0, i < (p.buf.n if n == "ev_byte" else p.cap)))
            return z3.Select({"ev_kind": p.kind, "ev_eof": p.eof, "ev_write_open": p.write_open, "ev_byte": p.buf.a}[n], i)
        if n in ('ev_bytes',):
            return mk_arr(p.buf.a, p.buf.n, n, U8)
        if n == "ev_new":
            cap = args[0]
            need(z3.And(cap >= 1, cap <= 4096))
            r = s.fresh(n, rty)
            s.assume(st, g, _wf_loop(r))
            s.assume(st, g, r.error == 0)
            s.assume(st, g, r.cap == cap)
            s.assume(st, g, r.count == 0)
            s.assume(st, g, r.last == -1)
            s.assume(st, g, r.ev_slot == -1)
            s.assume(st, g, r.ev_kind_ == 0)
            s.assume(st, g, r.exit == 0)
            s.assume(st, g, r.buf.n == 0)
            s.assume(st, g, r.kind == z3.K(z3.IntSort(), 0))
            s.assume(st, g, r.eof == z3.K(z3.IntSort(), False))
            s.assume(st, g, r.write_open == z3.K(z3.IntSort(), False))
            return r
        if n in ('ev_close',):
            res = s.fresh(n, rty)
            o = s.model.res(None)
            s.assume(st, g, z3.Implies(p.error != 0, o.is_err(res)))
            s.assume(st, g, z3.Implies(o.is_ok(res), z3.And(p.error == 0, o.oval(res) == p.count)))
            s.assume(st, g, z3.Implies(o.is_err(res), z3.And(o.eval(res) >= z3.If(p.error == 0, 3, 1), o.eval(res) <= 8)))
            return res
        need(p.error == 0)
        if n in ('ev_add_socket', 'ev_add_listener', 'ev_add_process', 'ev_add_timer', 'ev_accept'):
            need(p.count < p.cap)
        if n in ('ev_add_socket',):
            h = args[1]
            need(h.error == 0)
        if n in ('ev_add_listener',):
            h = args[1]
            need(h.error == 0)
        if n in ('ev_add_process',):
            h = args[1]
            need(h.error == 0)
        if n in ('ev_add_timer', 'ev_wait'):
            ms = args[1]
            need(z3.And(ms >= (-1 if n == "ev_wait" else 0), ms <= 86400000))
        if n in ('ev_proc_wait_for', 'ev_proc_write_for'):
            ms = args[-1]
            need(z3.And(ms >= 0, ms <= 86400000))
        if into or n in ('ev_read', 'ev_write', 'ev_shutdown_write', 'ev_accept', 'ev_proc_read', 'ev_proc_write', 'ev_proc_close_stdin', 'ev_proc_wait', 'ev_remove', 'ev_proc_wait_for', 'ev_proc_write_for'):
            i = args[1]
            need(z3.And(i >= 0, i < p.cap))
            k = z3.Select(p.kind, i)
            expected = 2 if n == "ev_accept" else 3 if n.startswith("ev_proc_") else 1
            need(k != 0 if n == "ev_remove" else k == expected)
        if n in ('ev_write', 'ev_shutdown_write', 'ev_proc_write', 'ev_proc_close_stdin', 'ev_proc_write_for'):
            i = args[1]
            need(z3.Select(p.write_open, i))
        if n in ('ev_read', 'ev_proc_read'):
            mx = args[-1]
            need(z3.And(mx > 0, mx <= MAX_LEN))
        if n in ('ev_proc_read', 'ev_proc_read_into'):
            stream = args[2]
            need(z3.Or(stream == 1, stream == 2))
        if into:
            ios = [len(args) - 2, len(args) - 1]
            finals = {}
            s.inout_finals(e, CATALOG[n], ios, args, finals, st, g)
            count = finals["n"]
            s.assume(st, g, z3.And(count >= 0, count <= finals["dst"].n))
        r = s.fresh(n, rty)
        s.assume(st, g, _wf_loop(r))
        s.assume(st, g, r.cap == p.cap)
        if n in ('ev_add_socket', 'ev_add_listener', 'ev_add_process', 'ev_add_timer', 'ev_accept'):
            a = r.last
            ok = r.error == 0
            added = z3.And(ok, a != -1)
            k = 4 if n == "ev_add_timer" else 3 if n == "ev_add_process" else 2 if n == "ev_add_listener" else 1
            s.assume(st, g, z3.Implies(added, z3.And(a >= 0, a < p.cap, z3.Select(p.kind, a) == 0, r.count == p.count + 1, r.kind == z3.Store(p.kind, a, k), r.eof == z3.Store(p.eof, a, False))))
            s.assume(st, g, z3.Implies(z3.Not(added), z3.And(r.last == -1, r.count == p.count, r.kind == p.kind, r.eof == p.eof, r.write_open == p.write_open)))
            opened = args[1].write_open if n == "ev_add_socket" else args[1].stdin_open if n == "ev_add_process" else z3.BoolVal(n == "ev_accept")
            s.assume(st, g, z3.Implies(added, r.write_open == z3.Store(p.write_open, a, opened)))
            if n != "ev_accept": s.assume(st, g, z3.Implies(ok, a != -1))
            if n == "ev_add_timer": s.assume(st, g, ok)
        else:
            s.assume(st, g, r.error == 0)
            s.assume(st, g, r.last == -1)
        if n in ('ev_wait',):
            i, k = r.ev_slot, r.ev_kind_
            ms = args[1]
            s.assume(st, g, z3.Implies(ms == -1, k != 0))
            s.assume(st, g, z3.Implies(z3.Or(k == 1, k == 3), z3.Or(z3.Select(p.kind, i) == 1, z3.Select(p.kind, i) == 3)))
            s.assume(st, g, z3.Implies(k == 2, z3.Select(p.kind, i) == 2))
            s.assume(st, g, z3.Implies(k == 4, z3.Select(p.kind, i) == 4))
            s.assume(st, g, z3.Implies(k == 5, z3.And(z3.Select(p.kind, i) >= 1, z3.Select(p.kind, i) <= 3)))
            s.assume(st, g, z3.Implies(k == 4, z3.And(r.kind == z3.Store(p.kind, i, 0), r.eof == z3.Store(p.eof, i, False), r.write_open == z3.Store(p.write_open, i, False), r.count == p.count - 1)))
            s.assume(st, g, z3.Implies(k != 4, z3.And(r.kind == p.kind, r.count == p.count)))
            s.assume(st, g, z3.Implies(k == 5, z3.And(r.eof == z3.Store(p.eof, i, False), r.write_open == z3.Store(p.write_open, i, False))))
            s.assume(st, g, z3.Implies(z3.And(k != 4, k != 5), z3.And(r.eof == p.eof, r.write_open == p.write_open)))
        if into or n in ('ev_read', 'ev_write', 'ev_shutdown_write', 'ev_proc_read', 'ev_proc_write', 'ev_proc_close_stdin', 'ev_proc_wait', 'ev_remove', 'ev_proc_wait_for', 'ev_proc_write_for'):
            i = args[1]
            freed = n in ("ev_remove", "ev_proc_wait")
            k = z3.IntVal(0) if freed else z3.Select(r.kind, i) if n == "ev_proc_wait_for" else z3.Select(p.kind, i)
            s.assume(st, g, r.kind == z3.Store(p.kind, i, k))
            s.assume(st, g, r.eof == z3.Store(p.eof, i, z3.Select(r.eof, i)))
            s.assume(st, g, r.write_open == z3.Store(p.write_open, i, z3.Select(r.write_open, i)))
            if n == "ev_proc_wait_for":
                # G10-ASYNC-DEADLINE: either the slot was freed (as ev_proc_wait) or it was kept with stdin closed
                s.assume(st, g, z3.Not(z3.Select(r.write_open, i)))
                s.assume(st, g, z3.Or(z3.And(z3.Select(r.kind, i) == 0, z3.Not(z3.Select(r.eof, i)), r.count == p.count - 1),
                                      z3.And(z3.Select(r.kind, i) == 3, z3.Select(r.eof, i) == z3.Select(p.eof, i), r.count == p.count, r.exit == p.exit)))
            else:
                s.assume(st, g, r.count == (p.count - 1 if freed else p.count))
            if n == "ev_proc_wait_for":
                pass
            elif freed:
                s.assume(st, g, z3.Not(z3.Select(r.eof, i)))
                s.assume(st, g, z3.Not(z3.Select(r.write_open, i)))
            elif into:
                s.assume(st, g, z3.Implies(z3.Select(p.eof, i), z3.Select(r.eof, i)))
                s.assume(st, g, z3.Implies(z3.Select(r.eof, i), count == 0))
            elif n in ("ev_read", "ev_proc_read"):
                mx = args[-1]
                s.assume(st, g, r.buf.n <= mx)
                s.assume(st, g, z3.Implies(z3.Select(p.eof, i), z3.Select(r.eof, i)))
                s.assume(st, g, z3.Implies(z3.Select(r.eof, i), r.buf.n == 0))
                s.assume(st, g, z3.Implies(z3.Select(r.write_open, i), z3.Select(p.write_open, i)))
            else:
                s.assume(st, g, z3.Select(r.eof, i) == z3.Select(p.eof, i))
            if n in ("ev_shutdown_write", "ev_proc_close_stdin"): s.assume(st, g, z3.Not(z3.Select(r.write_open, i)))
        if n != "ev_wait":
            s.assume(st, g, z3.Implies(r.error == 0, r.ev_slot == p.ev_slot))
            s.assume(st, g, z3.Implies(r.error == 0, r.ev_kind_ == p.ev_kind_))
        if n not in ("ev_proc_wait", "ev_proc_wait_for"):
            s.assume(st, g, r.exit == p.exit)
        if n not in ("ev_read", "ev_proc_read"):
            s.assume(st, g, z3.Implies(r.error == 0, r.buf.n == p.buf.n))
            s.assume(st, g, z3.Implies(r.error == 0, r.buf.a == p.buf.a))
        if into:
            s.post_refine(e, ios, st, g)
        return r

    def net_call(s, n, e, args, st, g, code, rty):
        """G10 net A1 (G10_NET_SPEC 2.3, 2.4, 4): the catalog contracts of the net operations. The requires of an
        operation are call_requires obligations in order (code only); an effect operation then gets a fresh result with
        its well-formedness and the ensures facts below. The observers and net_bytes are exact terms. Parameter
        observers in the ensures denote the values at call entry (the actuals in args). Two identical connects or
        accepts are never equated: every effect result is a fresh symbol. net_accept borrows its listener: the
        listener value is not touched."""
        need = s.catalog_need(st, g, e, code, n)

        # observers: field reads and array selects
        if n == "net_ok":
            return args[0].error == 0
        if n == "net_error":
            return args[0].error
        if n == "net_write_open":
            return args[0].write_open
        if n == "net_eof":
            return args[0].eof
        if n == "net_size":
            return args[0].buf.n
        if n == "net_byte":
            sv, i = args
            need(z3.And(i >= 0, i < sv.buf.n))
            r = z3.Select(sv.buf.a, i)
            if code:
                s.assume(st, g, z3.And(r >= 0, r <= 255))
            return r
        if n == "net_listener_ok":
            return args[0].error == 0
        if n == "net_listener_error":
            return args[0].error
        if n == "net_listener_port":
            return args[0].port
        if n == "net_bytes":        # 4.6: a new array with the snapshot's length and byte map
            return mk_arr(args[0].buf.a, args[0].buf.n, n, U8)
        # effect operations: requires -> fresh result -> well-formedness -> ensures
        p = args[0]
        if n == "net_listen":
            need(z3.And(args[1] >= 0, args[1] <= 65535))
        elif n == "net_accept":
            need(p.error == 0)
        elif n == "net_connect":
            need(z3.And(args[1] >= 1, args[1] <= 65535))
        elif n in ("net_write_str", "net_write_bytes", "net_shutdown_write"):
            need(z3.And(p.error == 0, p.write_open))
        elif n == "net_set_timeout":
            need(p.error == 0)
            need(z3.And(args[1] >= 0, args[1] <= 86400000))
        elif n == "net_read":
            need(p.error == 0)
            need(z3.And(args[1] > 0, args[1] <= MAX_LEN))
        elif n == "net_read_all":
            need(p.error == 0)
        res = s.fresh(n, rty)
        s.assume(st, g, s.wf(rty, res))
        if n in ("net_close", "net_listener_close"):    # 4.7: the existing Result<i64> encoding
            o = s.model.res(None)
            top = 6 if n == "net_close" else 2
            s.assume(st, g, z3.Implies(p.error != 0, res == o.err(p.error)))
            s.assume(st, g, z3.Implies(p.error == 0, res == o.ok(z3.IntVal(0))))
            s.assume(st, g, z3.Implies(o.is_err(res), z3.And(o.eval(res) >= 1, o.eval(res) <= top)))
            return res
        ok = res.error == 0
        if n == "net_listen":
            s.assume(st, g, z3.Implies(z3.Not(ok), z3.Or(res.error == 1, res.error == 2)))
        elif n == "net_accept":
            s.assume(st, g, z3.Implies(ok, z3.And(res.write_open, res.buf.n == 0, z3.Not(res.eof))))
            s.assume(st, g, z3.Implies(z3.Not(ok), z3.And(res.error == 5, res.buf.n == 0, z3.Not(res.eof))))
        elif n == "net_connect":
            s.assume(st, g, z3.Implies(ok, z3.And(res.write_open, res.buf.n == 0, z3.Not(res.eof))))
            s.assume(st, g, z3.Implies(z3.Not(ok), z3.And(z3.Or(res.error == 1, res.error == 2), res.buf.n == 0,
                                                          z3.Not(res.eof))))
        elif n in ("net_write_str", "net_write_bytes"):
            s.assume(st, g, z3.Implies(ok, z3.And(res.write_open == p.write_open, _net_keep_buf(res, p))))
            s.assume(st, g, z3.Implies(z3.Not(ok), z3.Or(res.error == 4, res.error == 6)))
        elif n == "net_shutdown_write":
            s.assume(st, g, z3.Not(res.write_open))
            s.assume(st, g, z3.Implies(ok, _net_keep_buf(res, p)))
            s.assume(st, g, z3.Implies(z3.Not(ok), res.error == 4))
        elif n == "net_set_timeout":
            s.assume(st, g, z3.Implies(ok, z3.And(res.write_open == p.write_open, _net_keep_buf(res, p))))
            s.assume(st, g, z3.Implies(z3.Not(ok), res.error == 4))     # the OS refused the timeout (A2 audit)
        elif n == "net_read":
            mx = args[1]
            sz = res.buf.n
            s.assume(st, g, z3.Implies(ok, z3.And(sz >= 0, sz <= mx)))
            s.assume(st, g, z3.Implies(ok, res.eof == (sz == 0)))
            s.assume(st, g, z3.Implies(z3.And(ok, p.eof), sz == 0))
            s.assume(st, g, z3.Implies(ok, res.write_open == p.write_open))
            s.assume(st, g, z3.Implies(z3.Not(ok), z3.Or(res.error == 3, res.error == 6)))
        elif n == "net_read_all":
            s.assume(st, g, z3.Implies(ok, z3.And(res.eof, res.write_open == p.write_open)))
            s.assume(st, g, z3.Implies(z3.Not(ok), z3.Or(res.error == 3, res.error == 6)))
        else:
            raise Exception("catalog: " + n)
        return res

    def map_lookup(s, m, k, rty):
        """If(m.d[k], Some(m.a[k]), None) as an Option<V> of the call's own result type"""
        o = s.model.opt(rty)
        return z3.If(z3.Select(m.d, k), o.some(z3.Select(m.a, k)), o.none)

    # ---------- statements ----------
    def ensures_ok(s, st, val, node):
        if s.f.cost is not None:
            s.prove(st, [], st.env["spent"] <= s.cost0, node, "cost_bound",
                    "spent <= cost `%s` (C at entry)" % _expr_text(s.f.cost))
        env = dict(st.env)
        for p in s.f.params:
            if p.mode is None:      # G5 A1: ordinary parameters denote their entry values; an inout parameter keeps its
                env[p.name] = s.params[p.name]      # current (final) value, and old(a) resolves through s.snap
        if val is not None:
            env["result"] = val
        prev = []   # earlier clauses (domain + value) were proven above and may justify later lookups
        for i, en in enumerate(s.f.inv_ensures):
            name, line, param = en.struct_invariant
            target = param if param == "result" or param.startswith(("result.", "result[")) else "inout %s at exit" % param
            what = "invariant of %s (line %d) for %s" % (name, line, target)
            for dom, code in s.dom_obligations(en, st, prev, env):
                s.prove(st, prev, dom, node, "struct_invariant", what, key_extra=(i, code))
                prev.append(dom)
            ob = s.ev(en, st, [], False, env)
            s.prove(st, [], ob, node, "struct_invariant", what, key_extra=i)
            prev.append(ob)
        for en in s.f.ensures:
            prev.extend(s.check_dom(st, prev, en, node, "variant_field lookups in postcondition (line %d) are defined" % en.line, env))
            ob = s.ev(en, st, [], False, env)
            s.prove(st, [], ob, node, "ensures", "postcondition (line %d)" % en.line)
            prev.append(ob)

    def exec_block(s, stmts, st):
        states = [st]
        for stmt in stmts:
            nxt = []
            for cur in states:
                nxt.extend(s.exec(stmt, cur))
            states = nxt
            if not states:
                break
        return states

    def assigned(s, stmts, out):
        for st in stmts:
            if isinstance(st, (Assign, IndexAssign, FieldAssign)):
                out.add(st.name)
            elif isinstance(st, If):
                s.assigned(st.then, out); s.assigned(st.els, out)
            elif isinstance(st, (While, ForMap)):
                s.assigned(st.body, out)
            elif isinstance(st, Match):
                sc = st.scrutinee
                if isinstance(sc, Call) and sc.name not in s.funcs and sc.name in MUTABLE_COLLECTION_MATCH + ("map_take",):
                    root, _ = field_place(sc.args[0].expr)
                    out.add(root.name)
                for a in st.arms:
                    s.assigned(a.body, out)
        return out

    # ---------- G5 A1: inout writes in the changed-variable analysis ----------
    def call_writes(s, stmts, out, *, select=None):
        """Names passed inout at any expression depth, including in nested blocks."""
        for st in stmts:
            _stmt_inout(st, out, select=select)
            if isinstance(st, If):
                s.call_writes(st.then, out, select=select)
                s.call_writes(st.els, out, select=select)
            elif isinstance(st, (While, ForMap)):
                s.call_writes(st.body, out, select=select)
            elif isinstance(st, Match):
                for a in st.arms:
                    s.call_writes(a.body, out, select=select)
        return out

    def whole_assigned(s, stmts, out):
        """names that a whole-variable assignment in stmts can replace (nested blocks included)"""
        for st in stmts:
            if isinstance(st, Assign):
                out.add(st.name)
            elif isinstance(st, If):
                s.whole_assigned(st.then, out); s.whole_assigned(st.els, out)
            elif isinstance(st, (While, ForMap)):
                s.whole_assigned(st.body, out)
            elif isinstance(st, Match):
                for a in st.arms:
                    s.whole_assigned(a.body, out)
        return out

    def havoc(s, st, names, keep=frozenset()):
        """keep: array names whose havoc retains the length term (G5 A1: inout parameters, and locals changed only by
        inout calls); every other array gets an arbitrary well-formed length exactly as before"""
        h = st.fork()
        for n in sorted(names):
            if n not in h.env:
                continue
            old = h.env[n]
            if isinstance(old, File):
                nv = s.fresh(n, FILE)
                h.pc.append(s.wf(FILE, nv))
                h.env[n] = nv
            elif isinstance(old, (Process, PArgs)):     # G10 proc A1 (4.8): only well-formedness survives
                pty = PROCESS if isinstance(old, Process) else PROCESS_ARGS
                nv = s.fresh(n, pty)
                h.pc.append(s.wf(pty, nv))
                h.env[n] = nv
            elif isinstance(old, (Socket, Listener, Loop)):   # G10 net A1 (4.8): only well-formedness survives
                nty = LOOP if isinstance(old, Loop) else SOCKET if isinstance(old, Socket) else LISTENER
                nv = s.fresh(n, nty)
                h.pc.append(s.wf(nty, nv))
                h.env[n] = nv
            elif isinstance(old, Arr):
                # G6 A1: an element type without a legacy array type (a Vec<X>) is re-created as a Vec
                legacy = old.elem in (I64, U8, BOOL) or old.elem.owner is not None
                nv = s.fresh(n, arr_type(old.elem) if legacy else Type("vec", (old.elem,)))
                if n in keep:
                    nv = mk_arr(nv.a, old.n, nv.name, nv.elem)
                else:
                    h.pc.append(z3.And(nv.n >= 0, nv.n <= MAX_LEN))
                if nv.elem == U8:
                    h.pc.append(u8_dom(nv))
                h.env[n] = nv
            elif isinstance(old, Map):
                nv = s.fresh(n, Type("hashmap", (old.kty, old.vty)))
                h.pc.append(z3.And(nv.n >= 0, nv.n <= MAX_LEN))
                h.env[n] = nv
            elif z3.is_int(old):
                nv = s.fresh(n, I64)
                if n != "spent" or s.f.cost is None:
                    h.pc.append(in_range(nv))
                h.env[n] = nv
            elif z3.is_bool(old):
                h.env[n] = s.fresh(n, BOOL)
            else:
                nv = z3.Const("%s!h%d" % (n, s.k), old.sort()); s.k += 1
                h.env[n] = nv
                ty = s.model.type_of(old)
                if ty is not None:
                    w = s.wf(ty, nv)
                    if w is not None:
                        h.pc.append(w)
        # A disjoint sibling write can havoc the root of a read-only iteration place.
        for place in s.iter_places:
            root, path = field_place(place)
            if root.name in names:
                saved = s.ev(place, st.fork(), [], False)
                h.env[root.name] = s.rebuild(root.ty, h.env[root.name], path, saved, h)
        return h

    def check_assert(s, stmt, st):
        """prove one assertion (user or synthesized by refine.py) on st; returns its value term"""
        s.check_dom(st, [], stmt.cond, stmt, "variant_field lookups in assertion are defined")
        ob = s.ev(stmt.cond, st, [], False)
        if stmt.origin_code is None:
            s.prove(st, [], ob, stmt, "assert", "assertion")
        else:
            # synthesized by refine.py: own code and a source description; the description also keys
            # the duplicate suppression, so different sites of one annotation are all reported
            s.prove(st, [], ob, stmt, stmt.origin_code, stmt.origin_what, stmt.origin_what)
        return ob

    def exec(s, stmt, st):
        if isinstance(stmt, Let) or isinstance(stmt, Assign):
            st = st.fork()
            v = s.ev(stmt.value, st, [], True)
            if isinstance(v, Arr):
                v = mk_arr(v.a, v.n, stmt.name, v.elem)
            st.env[stmt.name] = v
            return [st]
        if isinstance(stmt, IndexAssign):
            st = st.fork()
            fpath = tuple(stmt.fields)
            place = ".".join((stmt.name,) + fpath)
            if fpath:
                # LG4: `p.a.b[i] = e`: the field array is read with the ordinary field read (an Arr)
                root = st.env[stmt.name]
                cur = root
                for fname in fpath:
                    cur = s.read_field(st, cur, fname)
                arr = cur
                if not isinstance(arr, Arr):
                    arr = s.model.field_get(s.field_chain_type(s.model.type_of(root), fpath), arr, fpath[-1])
            else:
                arr = st.env[stmt.name]
            i = s.ev(stmt.index, st, [], True)
            v = s.ev(stmt.value, st, [], True)
            s.prove(st, [], z3.And(i >= 0, i < arr.n), stmt, "index_oob", "index in bounds for %s[...] =" % place)
            if arr.elem == U8:
                s.prove(st, [], z3.And(v >= 0, v <= 255), stmt, "elem_range",
                        "u8 element range 0..=255 for %s[...] =" % place)
            if stmt.elem_refine is not None:
                # refine.py: the stored value (already evaluated) must lie in the element range. One query, no
                # domain check, no re-evaluation, no forall; not assumed afterwards (it is implied by the path).
                lo, hi, what = stmt.elem_refine
                s.prove(st, [], z3.And(v >= lo, v <= hi), stmt, "refinement_assign", what, what)
            if fpath:
                new = mk_arr(z3.Store(arr.a, i, v), arr.n, arr.name, arr.elem)
                root = st.env[stmt.name]
                st.env[stmt.name] = s.rebuild(s.model.type_of(root), root, fpath, new, st)
                return [st]
            st.env[stmt.name] = mk_arr(z3.Store(arr.a, i, v), arr.n, stmt.name, arr.elem)
            return [st]
        if isinstance(stmt, If):
            st = st.fork()
            c = s.ev(stmt.cond, st, [], True)
            return s.exec_block(stmt.then, st.fork(c)) + s.exec_block(stmt.els, st.fork(z3.Not(c)))
        if isinstance(stmt, Match):
            st = st.fork()
            scrutinee = stmt.scrutinee
            update = (isinstance(scrutinee, Call) and scrutinee.name not in s.funcs
                      and scrutinee.name in MUTABLE_COLLECTION_MATCH)
            if update:
                place = scrutinee.args[0].expr
                root, path = field_place(place)
                collection = s.ev(place, st, [], True)
                key = s.ev(scrutinee.args[1], st, [], True)
                if isinstance(collection, Map):
                    key = s.model.map_key(collection.kty, key)
                    v = s.map_lookup(collection, key, scrutinee.ty)
                else:
                    o = s.model.opt(scrutinee.ty)
                    bounds = z3.And(key >= 0, key < collection.n)
                    element = z3.Select(collection.a, key)
                    v = z3.If(bounds, o.some(element), o.none)
            else:
                v = s.ev(scrutinee, st, [], True)
            recursive_vec = update and isinstance(collection, Arr) and collection.elem in s.model.vec_sums
            outer = set(st.env)
            out = []
            sty = s.model.type_of(v)     # G3: the scrutinee's complete (possibly generic) type
            if s.model.g9:
                s.observe(st, v, sty)    # G9 A1: one-step facts of the matched value
            payload =sty.args[0] if sty is not None and sty.args and sty.name in ("option", "result") else I64
            for a in stmt.arms:
                if a.enum_name is not None:
                    ety = sty if sty is not None and sty.name == a.enum_name else Type(a.enum_name)
                    _, rec, accs = s.model.enum_info(ety)[a.ctor]
                    cond = rec(v)
                    vals = [acc(v) for acc in accs]
                    ftys = [ft for _, ft in s.model.fields_of(ety, a.ctor)]
                else:
                    if a.ctor in ("Some", "None"):
                        o = s.model.opt(sty)
                        if a.ctor == "Some": cond, val = o.is_some(v), o.sval(v)
                        else: cond, val = o.is_none(v), None
                    else:
                        o = s.model.res(sty)
                        if a.ctor == "Ok": cond, val = o.is_ok(v), o.oval(v)
                        else: cond, val = o.is_err(v), o.eval(v)
                    vals = [] if val is None else [val]
                    ftys = [result_error(sty) if a.ctor == "Err" else payload] * len(vals)
                b = st.fork(cond)
                for name, val, fty in zip(a.binders, vals, ftys):
                    if s.model.g9:       # G9 A1: buffer payloads become Arr views; nominal payloads are observed
                        val = s.model.field_get(fty, val, name)
                        s.observe(b, val, fty)
                    elif fty == _STRING and a.ctor == "Err":
                        val = s.model.field_get(fty, val, name)
                    b.env[name] = val
                    w = s.wf(fty, val)
                    if w is not None:
                        b.pc.append(w)
                writes_back = update and a.ctor == "Some"
                if writes_back and recursive_vec:
                    size = s.model.size_fn(collection.elem.name)
                    b.pc.append(size(element) <= s.vec_sum(b, [], collection))
                for end in s.exec_block(a.body, b):
                    if writes_back:
                        final = end.env[a.binders[0]]
                        array = z3.Store(collection.a, key, final)
                        if isinstance(collection, Map):
                            new = Map(collection.d, array, collection.n, collection.name,
                                      collection.kty, collection.vty)
                        else:
                            new = mk_arr(array, collection.n, collection.name, collection.elem)
                            if recursive_vec:
                                s.observe(end, final, collection.elem)
                                old_sum = s.vec_sum(end, [], collection)
                                new_sum = s.vec_sum(end, [], new)
                                end.pc.append(new_sum == old_sum - size(element) + size(final))
                        end.env[root.name] = s.rebuild(root.ty, end.env[root.name], path, new, end)

                    # binders and arm locals end with the arm; only updates to outer variables remain
                    for k in [k for k in end.env if k not in outer]:
                        del end.env[k]
                    out.append(end)
            return out
        if isinstance(stmt, FieldAssign):
            st = st.fork()
            v = s.ev(stmt.value, st, [], True)          # rhs first, on the state before the store
            root = st.env[stmt.name]
            st.env[stmt.name] = s.rebuild(s.model.type_of(root), root, stmt.fields, v, st)
            return [st]
        if isinstance(stmt, Return):
            st = st.fork()
            v = s.ev(stmt.value, st, [], True) if stmt.value is not None else None
            s.ensures_ok(st, v, stmt)
            return []
        if isinstance(stmt, Assert) and stmt.reveal is not None:     # P13o: proof-only, no query
            return s.reveal(stmt, st)
        if isinstance(stmt, Assert):
            ob = s.check_assert(stmt, st)
            st = st.fork(ob)
            return [st]
        if isinstance(stmt, ExprStmt):
            st = st.fork()
            s.ev(stmt.expr, st, [], True)
            return [st]
        if isinstance(stmt, While):
            return s.loop(stmt, st)
        if isinstance(stmt, ForMap):
            s.iter_places.append(stmt.place)
            ends = s.loop(stmt, st)
            s.iter_places.pop()
            return ends
        if isinstance(stmt, Par):       # G7 A1
            return [s.par(stmt, st)]
        raise Exception("exec: %r" % stmt)

    def loop(s, w, st):
        # (cand id | None, expr, text): written invariants, then frame clauses, then inferred candidates.
        invs = [(None, inv, None) for inv in w.invariants]
        if s.loop_no and isinstance(w, While):
            n = s.loop_no[id(w)]
            invs.extend(s.frame.get(n, ()))
            invs.extend(s.extra.get(n, ()))
        prev = []   # invariants run in order: an earlier one (proven here) may justify a later one's lookups
        for cid, inv, text in invs:
            if cid is None:
                w_dom, w_ob = "variant_field lookups in loop invariant (entry) are defined", "loop invariant on entry"
            elif cid.startswith("F"):
                w_dom = "partial operations in loop frame clause `%s` (entry) are defined" % text
                w_ob = "loop frame clause `%s` on entry" % text
            else:
                w_dom = "partial operations in inferred loop invariant `%s` (entry) are defined" % text
                w_ob = "inferred loop invariant `%s` on entry" % text
            prev.extend(s.check_dom(st, prev, inv, inv, w_dom, cand=cid))
            ob = s.ev(inv, st, [], False)
            s.prove(st, [], ob, inv, "invariant_entry", w_ob, cid, cid)
            prev.append(ob)
        names = s.assigned(w.body, set())
        if s.f.cost is not None:
            names.add("spent")
        # G5 A1 (5.3): writes through inout places also havoc their roots. Only
        # operands of resizing calls or whole assignments may lose their length.
        calls = s.call_writes(w.body, set(), select=lambda call, i: call.name not in ("vec_reserve", "arr_reserve"))
        keep = frozenset()
        if calls or s.snap:
            whole = s.whole_assigned(w.body, set())
            resizes = s.call_writes(w.body, set(), select=lambda call, i:
                                    call.name in VEC_RESIZERS + ("arr_push", "arr_append") or (call.name, i) in s.model.vec_resize_params)
            names |= calls
            keep = frozenset((names & s.snap.keys() | calls) - whole - resizes)
        # one arbitrary iteration; the invariants held (and were defined) at its start
        b = s.havoc(st, names, keep)
        for _cid, inv, _text in invs:
            d = s.dom(inv, b, [], b.env, "variant")
            if d is not None:
                b.pc.append(d)
            b.pc.append(s.ev(inv, b, [], False))
        if isinstance(w, ForMap):
            m = s.ev(w.place, b, [], True)
            key = s.fresh(w.key, m.kty)
            index = s.model.map_key(m.kty, key)
            value = z3.Select(m.a, index)
            b.env[w.key], b.env[w.val] = key, value
            b.pc.append(z3.Select(m.d, index))
            for ty, term in ((m.kty, key), (m.vty, value)):
                s.observe(b, term, ty)
                wf = s.wf(ty, term)
                if wf is not None:
                    b.pc.append(wf)
        else:
            c = s.ev(w.cond, b, [], True)
            b.pc.append(c)
            s.check_dom(b, [], w.decreases, w.decreases, "variant_field lookups in decreases are defined")
            d0 = s.ev(w.decreases, b, [], False)
            s.prove(b, [], d0 >= 0, w.decreases, "decreases_nonneg", "decreases >= 0")
        s.spend(b, [], z3.IntVal(1))
        for end in s.exec_block(w.body, b):
            prev = []
            for cid, inv, text in invs:
                if cid is None:
                    w_dom, w_ob = ("variant_field lookups in loop invariant (preserved) are defined",
                                   "loop invariant preserved")
                elif cid.startswith("F"):
                    w_dom = "partial operations in loop frame clause `%s` (preserved) are defined" % text
                    w_ob = "loop frame clause `%s` preserved" % text
                else:
                    w_dom = "partial operations in inferred loop invariant `%s` (preserved) are defined" % text
                    w_ob = "inferred loop invariant `%s` preserved" % text
                prev.extend(s.check_dom(end, prev, inv, inv, w_dom, cand=cid))
                ob = s.ev(inv, end, [], False)
                s.prove(end, [], ob, inv, "invariant_preserved", w_ob, cid, cid)
                prev.append(ob)
            if isinstance(w, ForMap):
                continue
            if _has_vf(w.decreases):
                # decreases is read at the end of the body: the invariants just proved may justify its lookups
                extra = []
                for _cid, inv, _text in invs:
                    d = s.dom(inv, end, [], end.env)
                    if d is not None:
                        extra.append(d)
                    extra.append(s.ev(inv, end, [], False))
                s.check_dom(end, extra, w.decreases, w.decreases, "variant_field lookups in decreases are defined")
            s.prove(end, [], s.ev(w.decreases, end, [], False) < d0, w.decreases, "decreases_decrease",
                    "decreases strictly decreases")
        # after the loop
        a = s.havoc(st, names, keep)
        for _cid, inv, _text in invs:
            d = s.dom(inv, a, [], a.env, "variant")
            if d is not None:
                a.pc.append(d)
            a.pc.append(s.ev(inv, a, [], False))
        if isinstance(w, While):
            c = s.ev(w.cond, a, [], True)
            a.pc.append(z3.Not(c))
        return [a]

    def run_foreign(s, st):
        """domain-only check of the ensures of a trusted rust fn: fresh result `result!k` with its well-formedness,
        then each ensures is domain-checked under the requires, the result facts and the earlier ensures"""
        f = s.f
        env = dict(st.env)
        if f.ret != UNIT:
            res = s.fresh("result", f.ret)
            if isinstance(res, Arr):
                st.pc.append(z3.And(res.n >= 0, res.n <= MAX_LEN))
                if res.elem == U8:
                    st.pc.append(u8_dom(res))
                elif f.ret == Type("vec", (I64,)):   # G6b: a foreign Vec<i64> result has i64 elements
                    st.pc.append(i64_dom(res))
            elif isinstance(res, Map):
                st.pc.append(z3.And(res.n >= 0, res.n <= MAX_LEN))
            else:
                w = s.wf(f.ret, res)
                if w is not None:
                    st.pc.append(w)
            env["result"] = res
        prev = []
        for en in f.inv_ensures + f.ensures:
            prev.extend(s.check_dom(st, prev, en, en, "variant_field lookups in postcondition (line %d) are defined"
                                    % en.line, env))
            prev.append(s.ev(en, st, [], False, env))

    def run(s):
        f = s.f
        env, pc = {}, []
        w_ids = set()
        shape = []      # P13o: the facts every value of the parameter types meets in a contract too
        for p in f.params:
            v = s.fresh(p.name, p.type)
            if isinstance(v, Arr):
                pc.append(z3.And(v.n >= 0, v.n <= MAX_LEN))
                if v.elem == U8:
                    pc.append(u8_dom(v))
                shape.extend(pc[-2:] if v.elem == U8 else pc[-1:])
                if p.type == STR and s.utf8_aware:      # LG26 (R-6b-3, G-LG25-1)
                    pc.append(s.model.utf8_valid(v))
                    w_ids.add(pc[-1].get_id())     # LG28 (G-LG27-5): the entry W is in obs, so no read adds it again
            elif isinstance(v, Map):    # G6 A1
                pc.append(z3.And(v.n >= 0, v.n <= MAX_LEN))
                shape.append(pc[-1])
            else:
                w = s.wf(p.type, v)
                if w is not None:
                    pc.append(w)
            env[p.name] = v
        s.params = dict(env)
        # G5 A1 (5.1): the immutable entry snapshots of the inout parameters; the executable name starts as the same
        # array and is rebound by element stores, calls and loop havoc, never the snapshot
        s.snap = {p.name: env[p.name] for p in f.params if p.mode is not None and (
            is_array(p.type) or p.type.owner is None and not p.type.args and p.type.name in s.model.struct)}
        st = State(env, pc)
        st.obs = frozenset(w_ids)
        if f.opaque and f.x_auto is not None:
            # P13o (P13o.1 totality): the body is defined for every argument, proven once here, so a reveal equation
            # never stands on an undefined body. Contract arithmetic is unbounded, so a contract may apply F to a
            # scalar outside i64 (or to an array element, which has no i64 fact): the proof assumes only the array
            # and map shape facts, never the i64 range of a scalar parameter (audit P13o.9)
            st0 = State(dict(env), list(shape))
            for d, _ in s.dom_obligations(f.x_auto, st0, [], st0.env):
                s.prove(st0, [], d, f, "opaque_total", "the body of opaque %s is defined for every argument" % f.name)
        for r in f.inv_requires + f.requires:
            # D(r) is proven from the earlier requires only, then D(r) && r is assumed, in declaration order
            s.check_dom(st, [], r, r, "variant_field lookups in requires (line %d) are defined" % r.line)
            # assume only the variant_field part of D(r) (as before): the other domains are proven above but not
            # assumed, since nested quantified index ranges in every later query made p06 ~150x slower
            d = s.dom(r, st, [], st.env, "variant")
            if d is not None:
                st.pc.append(d)
            st.pc.append(s.ev(r, st, [], False))
        if f.cost is not None:
            s.cost0 = s.cost_value(f.cost, st, [], f.cost, dict(st.env))
            st.env["spent"] = z3.IntVal(0)
        if f.decreases is not None and f.foreign is None:
            # G9 A1 (G9_SPEC 5): the measure is defined and >= 0 under the requires at every entry (errors point to the
            # clause); its value is frozen here and is the caller measure of every recursive call in the body
            s.check_dom(st, [], f.decreases, f.decreases, "variant_field lookups in the decreases clause are defined")
            s.m0 = s.ev(f.decreases, st, [], False)
            s.prove(st, [], s.m0 >= 0, f.decreases, "recursive_nonneg",
                    "decreases measure `%s` >= 0 at function entry" % _expr_text(f.decreases))
        if f.foreign is not None:       # G6 A1 (11.4): a trusted declaration has no body and no ensures proof
            s.run_foreign(st)
            return
        for end in s.exec_block(f.body, st):
            if f.ret == UNIT:
                s.ensures_ok(end, None, f)


def verify_program(prog, rec=None, infer=False, report=None, cache=None, modrec=None):
    """infer=True (STAGE1 B2, --infer): per function, explore template invariant candidates (infer.py), then
    verify the whole function with the survivors as extra loop invariants. report (a list) receives one record
    per loop (docs/STAGE1_B_INFER.md section 5).
    modrec: PR1 proof_cache.ModuleRecords or None. With it (explicit-module programs only, no trace, no
    inference report) a module whose record exists is not verified again, and a module whose functions all verify
    with no error gets a record (docs/G4B_SPEC.md 7.8)."""
    from typeinfo import g2_unimplemented_error
    # G3 A2: the verifier handles generics (no guard here); the code generators and pruvc.py still reject them.
    # "array" in skip exempts only the structured [T] (the plain [i64] is never a G2 hit).
    g2err = g2_unimplemented_error(prog, "verify", skip=("array_u8", "array_bool", "str", "array"))  # G2 B1
    if g2err is not None:
        return [g2err]
    errors = []
    model = Model(prog)

    def verify_once(f, errs, records, frame=None):
        if not infer:
            v = FnVerifier(prog, f, errs, model, rec=rec, cache=cache, frame=frame)
            v.run()
            return v.frame_failed
        import infer as infer_mod      # lazy: infer.py imports this module
        res = infer_mod.infer_function(prog, f, model, rec, cache, frame=frame)
        before = len(errs)
        failed = {}
        if res.status == "timeout":
            errs.append(PruvError(
                "verify", f.line, f.col, "inference_timeout",
                "invariant inference for %s hit the time safety limit (5 s per function, 500 ms per query)" % f.name,
                "write the loop invariants explicitly, or compile without --infer", file=f.file))
        else:
            v = FnVerifier(prog, f, errs, model, rec=rec, extra=res.extra(), cache=cache, frame=frame)
            v.run()
            failed = v.frame_failed
        if records is not None:
            records.extend(res.records(len(errs) - before))
        return failed

    def one(f):
        # A round with frame clauses is kept only if it proves every clause and reports no error; otherwise its trace
        # entries are rewound, and with no failed clause (another error, an inference timeout) the old path decides.
        frame = getattr(f, "frame_cands", None)
        while frame:
            mark = len(rec.obligations) if rec is not None else 0
            errs, records = [], [] if report is not None else None
            failed = verify_once(f, errs, records, frame)
            if not failed and not errs:
                if report is not None:
                    report.extend(records)
                return
            if rec is not None:
                del rec.obligations[mark:]
            if not failed:
                break
            frame = {n: [c for c in cands if c[0] not in failed] for n, cands in frame.items()}
            frame = {n: cands for n, cands in frame.items() if cands}
        verify_once(f, errors, report)

    groups = _module_groups(prog) if modrec is not None and rec is None and report is None else None
    if groups is None:
        for f in prog.functions:
            one(f)
        return errors
    import proof_cache
    closures = proof_cache.module_closures(prog)
    keys = {}
    for f in prog.functions:
        mi = groups.get(id(f))
        if mi is None:
            one(f)
            continue
        if mi not in keys:
            k = modrec.key(proof_cache.module_key_text(prog, mi, closures[mi], infer))
            keys[mi] = [k, modrec.lookup(k), len(errors), 0]
            if keys[mi][1]:
                modrec.hits += 1
            else:
                modrec.misses += 1
        st = keys[mi]
        if st[1]:
            continue
        before = len(errors)
        one(f)
        st[3] += len(errors) - before
    for k, hit, _, nerr in keys.values():
        if not hit and nerr == 0:
            modrec.store(k)
    return errors


def _module_groups(prog):
    """id(Function) -> index of its module in prog.modules, or None when the program has no module structure"""
    mods = getattr(prog, "modules", None)
    if not getattr(prog, "explicit", False) or not mods or not getattr(prog, "_g4_resolved", False):
        return None
    ix = {(m.pkg, m.path): i for i, m in enumerate(mods)}
    out = {}
    for f in prog.functions:
        i = ix.get((getattr(f, "pkg", None), getattr(f, "mod_path", None)))
        if i is not None and any(it is f for it in mods[i].items):
            out[id(f)] = i
    return out
