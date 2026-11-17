"""KD-M6-3: small array results on the stack (docs/SMALL_RESULT_SPEC.md).

`declared_bound(f)` reads the contract bound K of `ensures len(result) <= K`; `plan_small` decides which functions get a
`__pruv_small` variant and which call sites are rewritten to use a caller-owned stack buffer. The rules are mirrored
exactly by pruvc-rs/crates/pruv-core/src/small.rs."""
from vast import *
from refine import _kids

MAX_BOUND = 64
STACK_CAP = 4096       # bytes of rewritten buffers per function


def _conjuncts(e):
    if isinstance(e, Binary) and e.op == "&&":
        return _conjuncts(e.left) + _conjuncts(e.right)
    return [e]


def declared_bound(f):
    """K of a top-level `len(result) <= K` conjunct (literal, 1..=64) of a non-generic, non-foreign `[i64]` function"""
    if f.tparams or f.foreign is not None or f.ret != ARRAY:
        return None
    best = None
    for e in f.ensures:
        for c in _conjuncts(e):
            if (isinstance(c, Binary) and c.op == "<=" and isinstance(c.left, Call) and c.left.name == "len"
                    and len(c.left.args) == 1 and isinstance(c.left.args[0], Var) and c.left.args[0].name == "result"
                    and isinstance(c.right, IntLit) and 1 <= c.right.value <= MAX_BOUND):
                best = c.right.value if best is None else min(best, c.right.value)
    return best


def _expr_ok(e, name, write_ok, fmap):
    """every occurrence of `name` in e is len(name), name[i], or a direct argument of a user function"""
    if isinstance(e, (Var, Move)):
        return e.name != name
    if isinstance(e, Call):
        if e.name == "len" and len(e.args) == 1 and isinstance(e.args[0], Var) and e.args[0].name == name:
            return True
        callee = fmap.get(e.name)
        user = callee is not None and callee.foreign is None
        for a in e.args:
            if isinstance(a, InoutArg) and isinstance(a.expr, Var) and a.expr.name == name:
                if not (write_ok and user):
                    return False
            elif isinstance(a, Var) and a.name == name:
                if not user:
                    return False
            elif not _expr_ok(a, name, write_ok, fmap):
                return False
        return True
    if isinstance(e, Index) and isinstance(e.array, Var) and e.array.name == name:
        return _expr_ok(e.index, name, write_ok, fmap)
    return all(_expr_ok(k, name, write_ok, fmap) for k in _kids(e))


def _stmts_ok(stmts, name, write_ok, fmap):
    for st in stmts:
        if isinstance(st, Let):
            if st.name == name or not _expr_ok(st.value, name, write_ok, fmap):
                return False
        elif isinstance(st, (Assign, FieldAssign)):
            if st.name == name or not _expr_ok(st.value, name, write_ok, fmap):
                return False
        elif isinstance(st, IndexAssign):
            if (st.name == name and not write_ok) or not _expr_ok(st.index, name, write_ok, fmap) \
                    or not _expr_ok(st.value, name, write_ok, fmap):
                return False
        elif isinstance(st, If):
            if not (_expr_ok(st.cond, name, write_ok, fmap) and _stmts_ok(st.then, name, write_ok, fmap)
                    and _stmts_ok(st.els, name, write_ok, fmap)):
                return False
        elif isinstance(st, While):
            if not (_expr_ok(st.cond, name, write_ok, fmap) and _stmts_ok(st.body, name, write_ok, fmap)):
                return False
        elif isinstance(st, Return):
            if st.value is not None and not _expr_ok(st.value, name, write_ok, fmap):
                return False
        elif isinstance(st, ExprStmt):
            if not _expr_ok(st.expr, name, write_ok, fmap):
                return False
        elif isinstance(st, Par):
            if any(_mentions(c, name) for c in st.calls):
                return False
        elif isinstance(st, Match):
            if not _expr_ok(st.scrutinee, name, write_ok, fmap):
                return False
            for a in st.arms:
                if name in a.binders or not _stmts_ok(a.body, name, write_ok, fmap):
                    return False
    return True


def _mentions(e, name):
    if isinstance(e, (Var, Move)):
        return e.name == name
    return any(_mentions(k, name) for k in _kids(e))


def _code_calls(stmts, out):
    """names called in executable positions (contracts are erased and do not count)"""
    def ex(e):
        if isinstance(e, Call):
            out.add(e.name)
        for k in _kids(e):
            ex(k)
    for st in stmts:
        if isinstance(st, (Let, Assign, FieldAssign)):
            ex(st.value)
        elif isinstance(st, IndexAssign):
            ex(st.index); ex(st.value)
        elif isinstance(st, If):
            ex(st.cond); _code_calls(st.then, out); _code_calls(st.els, out)
        elif isinstance(st, While):
            ex(st.cond); _code_calls(st.body, out)
        elif isinstance(st, Return):
            if st.value is not None:
                ex(st.value)
        elif isinstance(st, ExprStmt):
            ex(st.expr)
        elif isinstance(st, Par):
            for c in st.calls:
                ex(c)
        elif isinstance(st, Match):
            ex(st.scrutinee)
            for a in st.arms:
                _code_calls(a.body, out)


def _count(stmts, pred):
    n = 0
    for st in stmts:
        if pred(st):
            n += 1
        for body in _children(st):
            n += _count(body, pred)
    return n


def _children(st):
    if isinstance(st, If):
        return [st.then, st.els]
    if isinstance(st, While):
        return [st.body]
    if isinstance(st, Match):
        return [a.body for a in st.arms]
    return []


def _cyclic(calls):
    """names on a call cycle (iterative Tarjan, linear in the call graph)"""
    index, low, on, stack, out = {}, {}, set(), [], set()
    for root in calls:
        if root in index:
            continue
        index[root] = low[root] = len(index)
        stack.append(root)
        on.add(root)
        work = [(root, iter(calls[root]))]
        while work:
            v, it = work[-1]
            w = next(it, None)
            if w is not None:
                if w not in index:
                    index[w] = low[w] = len(index)
                    stack.append(w)
                    on.add(w)
                    work.append((w, iter(calls[w])))
                elif w in on:
                    low[v] = min(low[v], index[w])
                continue
            work.pop()
            if work:
                u = work[-1][0]
                low[u] = min(low[u], low[v])
            if low[v] == index[v]:
                comp = []
                while True:
                    w = stack.pop()
                    on.discard(w)
                    comp.append(w)
                    if w == v:
                        break
                if len(comp) > 1 or v in calls[v]:
                    out.update(comp)
    return out


def plan_small(prog, fmap, layout, stack):
    """({function name: (K, o)}, {id(Let node): (callee, K)}); layout(node) is the storage layout of a declaration or
    function result, stack the P3-S1 plan (or None)"""
    calls = {}
    for f in fmap.values():
        out = set()
        if f.foreign is None:
            _code_calls(f.body, out)
        calls[f.name] = {n for n in out if n in fmap}
    callers = {n: set() for n in calls}
    for n, cs in calls.items():
        for m in cs:
            callers[m].add(n)
    tainted = {n for n, f in fmap.items() if f.foreign is not None}
    todo = list(tainted)
    while todo:
        for c in callers.get(todo.pop(), ()):
            if c not in tainted:
                tainted.add(c)
                todo.append(c)
    cyclic = _cyclic(calls)

    def in_cycle(n):
        return n in cyclic

    fns = {}
    for f in prog.functions:
        if f.foreign is not None or f.name in fns or fmap.get(f.name) is not f or f.ret != ARRAY or f.name in tainted:
            continue
        k = f.small_bound if f.small_bound is not None else declared_bound(f)
        if k is None or not 1 <= k <= MAX_BOUND or not f.body or not isinstance(f.body[-1], Return):
            continue
        last = f.body[-1].value
        if not isinstance(last, Var):
            continue
        o = last.name
        idx = [i for i, st in enumerate(f.body) if isinstance(st, Let) and st.name == o]
        if len(idx) != 1 or _count(f.body, lambda st: isinstance(st, Let) and st.name == o) != 1:
            continue
        decl = f.body[idx[0]]
        if not (decl.mutable and decl.type == ARRAY and isinstance(decl.value, Call) and decl.value.name == "alloc"
                and len(decl.value.args) == 2):
            continue
        if _count(f.body, lambda st: isinstance(st, Return)) != 1:
            continue
        if layout(f) != "i64" or layout(decl) != "i64" or (stack is not None and stack.contains(decl)):
            continue
        if not _stmts_ok(f.body[idx[0] + 1:-1], o, True, fmap):
            continue
        fns[f.name] = (k, o)

    sites = {}
    for f in prog.functions:
        if f.foreign is not None or fmap.get(f.name) is not f or not fns or in_cycle(f.name):
            continue
        used = [0, False]    # bytes, full

        def walk(stmts):
            for i, st in enumerate(stmts):
                if (isinstance(st, Let) and not st.mutable and st.type == ARRAY and isinstance(st.value, Call)
                        and st.value.name in fns and not used[1] and layout(st) == "i64"
                        and _stmts_ok(stmts[i + 1:], st.name, False, fmap)):
                    k = fns[st.value.name][0]
                    if used[0] + 8 * k <= STACK_CAP:
                        used[0] += 8 * k
                        sites[id(st)] = (st.value.name, k)
                    else:
                        used[1] = True
                for body in _children(st):
                    walk(body)
        walk(f.body)
    return fns, sites
