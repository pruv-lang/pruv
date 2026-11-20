"""P18d: by-value fixed-prefix twin for `inout o: [i64]` (--proven-entries).

`plan_fp(cands, fmap)` decides which internal functions whose LAST parameter is `inout o: [i64]` only ever touch `o`
through a statically fixed prefix of K words: literal indices < K, `len(o)` compared with a literal (the comparison has the
same truth value for every runtime length >= K), and `inout o` passed as the last argument to another planned function.
The rules are mirrored exactly by pruvc-rs/crates/pruv-core/src/fixed_prefix.rs.

cands: the functions the code generator already filtered (concrete specializations only, non-foreign, not host entries, not body_named, not
in the small plan, no cost, last parameter emitted as `&mut [i64]`), and only in a verified program that cannot
unwind inside bodies (the code generator gates G10 runtime and par programs; `may_unwind` names the functions that
reach a foreign function, a G6 catalog operation or print); fmap: function name -> declaration.
Returns {function name: (K, o)}."""
from vast import *
from refine import _kids
from small import _cyclic
from foreign import CATALOG

MAX_K = 16
FLIP = {"<": ">", "<=": ">=", ">": "<", ">=": "<=", "==": "==", "!=": "!="}


def may_unwind(prog, fmap):
    """Names of the functions that can unwind through a call: a foreign function, a call to print (println! panics
    when stdout fails), a G6 catalog operation (its boundary helper panics on a failed requires/ensures check, a bad
    representation or an exhausted check budget), or a call to a function in this set. A twin's adapter copies the prefix back only when the core
    returns, so these never get one. (Verified bodies have no other panic; the code generator turns the plan off for
    G10 runtime and par programs.)"""
    def names(node, acc):
        if isinstance(node, (list, tuple)):
            for x in node:
                names(x, acc)
            return
        if isinstance(node, Call):
            acc.add(node.name)
        if hasattr(node, "__dataclass_fields__") and not isinstance(node, Type):
            for n in node.__dataclass_fields__:
                names(getattr(node, n), acc)
    calls = {}
    bad = set()
    for f in prog.functions:
        if f.name in calls:
            continue
        acc = set()
        names(f.body, acc)
        calls[f.name] = acc
        g = fmap.get(f.name)
        if (f.foreign is not None or (g is not None and g.foreign is not None) or "print" in acc
                or any(c in CATALOG and c not in fmap for c in acc)):
            bad.add(f.name)
    bad |= {n for n, g in fmap.items() if g.foreign is not None}
    changed = True
    while changed:
        changed = False
        for n, cs in calls.items():
            if n not in bad and cs & bad:
                bad.add(n)
                changed = True
    return bad


def _is_len(e, o):
    return (isinstance(e, Call) and e.name == "len" and len(e.args) == 1
            and isinstance(e.args[0], Var) and e.args[0].name == o)


def _len_need(op, c):
    """K needed so that `len(o) OP c` has the same truth value for every length >= K"""
    return c if op in ("<", ">=") else c + 1


def _expr(e, o, fmap, acc):
    """every occurrence of o in e is a U1-U4 use; acc = [need, callee names]"""
    if isinstance(e, (Var, Move)):
        return e.name != o
    if isinstance(e, Binary) and e.op in FLIP:
        for a, b, op in ((e.left, e.right, e.op), (e.right, e.left, FLIP[e.op])):
            if _is_len(a, o) and isinstance(b, IntLit) and b.value >= 0:
                acc[0] = max(acc[0], _len_need(op, b.value))
                return True
    if isinstance(e, Index) and isinstance(e.array, Var) and e.array.name == o:
        if isinstance(e.index, IntLit) and e.index.value >= 0:
            acc[0] = max(acc[0], e.index.value + 1)
            return True
        return False
    if isinstance(e, Quant) and e.var == o:
        return False
    if isinstance(e, Call):
        n = len(e.args)
        for i, a in enumerate(e.args):
            if isinstance(a, InoutArg) and isinstance(a.expr, Var) and a.expr.name == o:
                g = fmap.get(e.name)
                if (i != n - 1 or g is None or g.foreign is not None or len(g.params) != n
                        or g.params[-1].mode != "inout"):
                    return False
                acc[1].append(e.name)
            elif not _expr(a, o, fmap, acc):
                return False
        return True
    return all(_expr(k, o, fmap, acc) for k in _kids(e))


def _mentions(e, o):
    if isinstance(e, (Var, Move)):
        return e.name == o
    return any(_mentions(k, o) for k in _kids(e))


def _stmts(stmts, o, fmap, acc):
    for st in stmts:
        if isinstance(st, Let):
            ok = st.name != o and _expr(st.value, o, fmap, acc)
        elif isinstance(st, (Assign, FieldAssign)):
            ok = st.name != o and _expr(st.value, o, fmap, acc)
        elif isinstance(st, IndexAssign):
            if st.name == o:
                ok = isinstance(st.index, IntLit) and st.index.value >= 0
                if ok:
                    acc[0] = max(acc[0], st.index.value + 1)
            else:
                ok = _expr(st.index, o, fmap, acc)
            ok = ok and _expr(st.value, o, fmap, acc)
        elif isinstance(st, If):
            ok = (_expr(st.cond, o, fmap, acc) and _stmts(st.then, o, fmap, acc) and _stmts(st.els, o, fmap, acc))
        elif isinstance(st, ForMap):
            ok = _expr(st.place, o, fmap, acc) and _stmts(st.body, o, fmap, acc)
        elif isinstance(st, While):
            ok = _expr(st.cond, o, fmap, acc) and _stmts(st.body, o, fmap, acc)
        elif isinstance(st, Return):
            ok = st.value is None or _expr(st.value, o, fmap, acc)
        elif isinstance(st, Assert):
            ok = _expr(st.cond, o, fmap, acc)
        elif isinstance(st, ExprStmt):
            ok = _expr(st.expr, o, fmap, acc)
        elif isinstance(st, Par):
            ok = not any(_mentions(c, o) for c in st.calls)
        elif isinstance(st, Match):
            ok = _expr(st.scrutinee, o, fmap, acc)
            for a in st.arms:
                ok = ok and o not in a.binders and _stmts(a.body, o, fmap, acc)
        else:
            ok = False
        if not ok:
            return False
    return True


def plan_fp(cands, fmap, unwind=()):
    """unwind: may_unwind's names, which get no twin"""
    own = {}      # name -> (need, callee names) of every candidate that passed the shape rules
    for f in cands:
        if f.name in own or fmap.get(f.name) is not f or f.name in unwind:
            continue
        o = f.params[-1].name
        acc = [1, []]
        if _stmts(f.body, o, fmap, acc):
            own[f.name] = (acc[0], acc[1])
    calls = {n: {c for c in cs if c in own} for n, (_, cs) in own.items()}
    rej = set(_cyclic(calls))
    ks = {}
    changed = True
    while changed:
        changed = False
        for n, (need, cs) in own.items():
            if n in rej or n in ks:
                continue
            if any(c not in own or c in rej for c in cs):
                rej.add(n)
                changed = True
            elif all(c in ks for c in cs):
                k = max([need] + [ks[c] for c in cs])
                if k > MAX_K:
                    rej.add(n)
                else:
                    ks[n] = k
                changed = True
    return {n: (k, fmap[n].params[-1].name) for n, k in ks.items()}
