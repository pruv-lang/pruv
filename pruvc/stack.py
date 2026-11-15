"""P3-S1: deterministic post-mono analysis choosing local arrays that live on the stack.

API: plan_stack(program_after_mono, storage_plan, verified) -> StackPlan.
  StackPlan.contains(let_node) -> bool       (node identity, like the P3-N1 storage plan)
  StackPlan.length(let_node)   -> constant element count
  StackPlan.entries: ordered tuple of (function, statement ordinal, name, length, element bytes)

Rules (docs/P3_STACK_DESIGN.md decisions 1 and 2), applied per function in program order:
  function  : plan_stack       - has a body, not in a recursive call SCC (self-recursion included), and no `cost`
                                  clause when unverified (runtime metering; verified builds erase cost, so the
                                  plan must not depend on it there).
  declaration: eligible_decl   - `let [mut] x: [E] = alloc(C, v)`, C an IntLit >= 0, E in i64 (refined or not), u8, bool
                                  (type ARRAY / ARRAY_U8 / ARRAY_BOOL), C * elem_bytes <= 16384, running total of
                                  accepted arrays of the function (source order) <= 65536.
  uses      : Uses.expr/stmt   - every use of the binding must be x[i] read, x[i] = e store, len(x), a direct argument
                                  `x` / `inout x` of a user (non foreign) function whose parameter at that position is
                                  an array, not `take`, and has the matching mode, or the same forms inside contracts,
                                  invariants, decreases and assertions. Everything else disqualifies.
Statement ordinals are a per-function preorder counter over all statements (nested blocks included).
"""
from dataclasses import fields, is_dataclass
from vast import *

MAX_ARRAY_BYTES = 16384
MAX_TOTAL_BYTES = 65536
KNOWN = (Var, Move, InoutArg, Call, Index, Quant, Unary, Binary, IntLit, BoolLit, StrLit, Ctor, StructLit,
         EnumCtor, FieldAccess, IsVariant, VariantField, Spent)


class StackPlan:
    def __init__(self):
        self.entries = ()
        self.ids = {}
        self.lits = set()

    def contains(self, node):
        return id(node) in self.ids

    def length(self, node):
        return self.ids[id(node)]

    def lit(self, node):
        return id(node) in self.lits


def _collect_calls(x, out):
    if isinstance(x, Call):
        out.add(x.name)
    if isinstance(x, (list, tuple)):
        for v in x:
            _collect_calls(v, out)
    elif is_dataclass(x) and not isinstance(x, type):
        for fd in fields(x):
            _collect_calls(getattr(x, fd.name), out)


def recursive_functions(prog):
    """names of functions on a call cycle (self-recursion included): f is recursive iff f is reachable from a callee"""
    fmap = {f.name: f for f in prog.functions}
    edges = {}
    for f in prog.functions:
        names = set()
        _collect_calls([f.body, f.requires, f.ensures], names)
        for extra in (getattr(f, "cost", None), getattr(f, "decreases", None)):
            if extra is not None:
                _collect_calls(extra, names)
        edges[f.name] = sorted(n for n in names if n in fmap)
    rec = set()
    for f in prog.functions:
        seen, todo = set(), list(edges[f.name])
        while todo:
            n = todo.pop()
            if n in seen:
                continue
            seen.add(n)
            todo.extend(edges[n])
        if f.name in seen:
            rec.add(f.name)
    return rec


def elem_bytes(let, storage):
    """element bytes of the declaration, or None when its element type does not qualify"""
    t = let.type
    if t == ARRAY_U8 or t == ARRAY_BOOL:
        return 1
    if t == ARRAY:
        return int(storage.layout_of(let)[1:]) // 8
    return None


def eligible_decl(let, storage):
    """(length, elem bytes) when the declaration shape qualifies, else None (size caps are checked by the caller)"""
    v = let.value
    if not (isinstance(v, Call) and v.name == "alloc" and len(v.args) == 2):
        return None
    n = v.args[0]
    if not (isinstance(n, IntLit) and n.value >= 0):
        return None
    eb = elem_bytes(let, storage)
    if eb is None or n.value * eb > MAX_ARRAY_BYTES:
        return None
    return n.value, eb


def eligible_lit(let):
    """true for `let x: str = "literal"`; such a binding may alias the read-only global when only read"""
    return let.type == STR and isinstance(let.value, StrLit)


class Uses:
    """walks one function; `cands` maps id(Let) -> Let for declarations still believed eligible"""

    def __init__(self, fmap, bad):
        self.fmap, self.bad = fmap, bad

    def kill(self, scope, name):
        d = scope.get(name)
        if d is not None:
            self.bad.add(id(d))

    def kill_all(self, scope):
        for d in scope.values():
            if d is not None:
                self.bad.add(id(d))

    def generic(self, x, scope, deny=False):
        if isinstance(x, Expr):
            self.expr(x, scope, deny)
        elif isinstance(x, (list, tuple)):
            for v in x:
                self.generic(v, scope, deny)
        elif is_dataclass(x) and not isinstance(x, type):
            for fd in fields(x):
                self.generic(getattr(x, fd.name), scope, deny)

    def children(self, e, scope, deny, skip=()):
        for fd in fields(e):
            if fd.name in skip or fd.name == "ty":
                continue
            self.generic(getattr(e, fd.name), scope, deny)

    def expr(self, e, scope, deny=False):
        if e is None:
            return
        if not isinstance(e, KNOWN):
            self.kill_all(scope)
            self.children(e, scope, True)
            return
        if isinstance(e, Var):
            self.kill(scope, e.name)
        elif isinstance(e, Move):
            self.kill(scope, e.name)
        elif isinstance(e, InoutArg):
            self.generic(e.expr, scope, True)
        elif isinstance(e, Index):
            if isinstance(e.array, Var) and not deny:
                pass            # a read of x[i]
            else:
                self.expr(e.array, scope, deny)
            self.expr(e.index, scope, deny)
        elif isinstance(e, Quant):
            self.expr(e.lo, scope, deny)
            self.expr(e.hi, scope, deny)
            inner = dict(scope)
            inner[e.var] = None
            self.expr(e.body, inner, deny)
        elif isinstance(e, Call):
            self.call(e, scope, deny)
        else:
            self.children(e, scope, deny)

    def call(self, e, scope, deny):
        if e.name == "len" and len(e.args) == 1 and isinstance(e.args[0], Var) and not deny:
            return
        callee = self.fmap.get(e.name)
        for j, a in enumerate(e.args):
            ok = False
            if (not deny and callee is not None and callee.foreign is None and j < len(callee.params)
                    and e.name not in ("len", "alloc", "copy", "truncate", "print")):
                p = callee.params[j]
                array = p.type in (ARRAY, ARRAY_U8, ARRAY_BOOL, STR)
                if array and not p.take:
                    if isinstance(a, Var) and p.mode != "inout":
                        ok = True
                    elif isinstance(a, InoutArg) and isinstance(a.expr, Var) and p.mode == "inout":
                        ok = True
                        d = scope.get(a.expr.name)
                        if d is not None and isinstance(d.value, StrLit):
                            self.bad.add(id(d))     # a literal candidate must not be mutated through inout
            if ok:
                continue
            self.expr(a, scope, deny)

    def block(self, stmts, outer, ctx):
        scope = dict(outer)
        for st in stmts:
            self.stmt(st, scope, ctx)

    def stmt(self, st, scope, ctx):
        ctx.ordinal += 1
        number = ctx.ordinal - 1
        if isinstance(st, Let):
            self.expr(st.value, scope)
            scope[st.name] = None
            e = eligible_decl(st, ctx.storage)
            if e is not None:
                ctx.decls.append((number, st, e))
                scope[st.name] = st
            elif eligible_lit(st):
                ctx.lits.append(st)
                scope[st.name] = st
        elif isinstance(st, Assign):
            self.kill(scope, st.name)
            self.expr(st.value, scope)
        elif isinstance(st, IndexAssign):
            d = scope.get(st.name)
            if d is not None and isinstance(d.value, StrLit):
                self.bad.add(id(d))
            self.expr(st.index, scope)
            self.expr(st.value, scope)
        elif isinstance(st, FieldAssign):
            self.kill(scope, st.name)
            self.expr(st.value, scope)
        elif isinstance(st, Return):
            self.expr(st.value, scope)
        elif isinstance(st, If):
            self.expr(st.cond, scope)
            self.block(st.then, scope, ctx)
            self.block(st.els, scope, ctx)
        elif isinstance(st, While):
            self.expr(st.cond, scope)
            for inv in st.invariants:
                self.expr(inv, scope)
            self.expr(st.decreases, scope)
            self.block(st.body, scope, ctx)
        elif isinstance(st, Assert):
            self.expr(st.cond, scope)
        elif isinstance(st, ExprStmt):
            self.expr(st.expr, scope)
        elif isinstance(st, Par):
            for c in st.calls:
                self.expr(c, scope, True)
        elif isinstance(st, Match):
            self.expr(st.scrutinee, scope)
            for arm in st.arms:
                inner = dict(scope)
                for name in arm.binders:
                    inner[name] = None
                self.block(arm.body, inner, ctx)
        else:
            self.kill_all(scope)
            self.generic(st, scope, True)


class _Ctx:
    def __init__(self, storage):
        self.storage = storage
        self.ordinal = 0
        self.decls = []
        self.lits = []


def plan_stack(prog, storage_plan, verified):
    out = StackPlan()
    fmap = {f.name: f for f in prog.functions}
    rec = recursive_functions(prog)
    entries = []
    for f in prog.functions:
        if f.foreign is not None or (not verified and getattr(f, "cost", None) is not None):
            continue
        bad = set()
        ctx = _Ctx(storage_plan)
        walker = Uses(fmap, bad)
        walker.block(f.body, {p.name: None for p in f.params}, ctx)
        for st in ctx.lits:
            if id(st) not in bad:
                out.lits.add(id(st))
        if f.name in rec:
            continue            # only literals are planned in recursive functions
        total = 0
        for number, st, (length, eb) in ctx.decls:
            if id(st) in bad:
                continue
            size = length * eb
            if total + size > MAX_TOTAL_BYTES:
                continue
            total += size
            out.ids[id(st)] = length
            entries.append((f.name, number, st.name, length, eb))
    out.entries = tuple(entries)
    return out
