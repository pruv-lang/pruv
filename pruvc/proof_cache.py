"""G4b-3: local content-addressed proof cache (docs/G4B_SPEC.md section 7).

Only `unsat` answers are stored. An entry is the file DIR/<key[:2]>/<key> holding exactly ENTRY.
"""
import hashlib
import os
import stat
import re

from trust_report import VERSION

ENTRY = b"pruv-proof-cache 1\nunsat\n"

# SMT-LIB lexemes as Z3_benchmark_to_smtlib_string prints them: a quoted symbol `|...|`, a string literal `"..."`
# (`""` reads as two adjacent literals), a `;` comment (an unclosed quote or literal runs to the end), or an atom
_LEX = re.compile(r'\|[^|]*(?:\||\Z)|"[^"]*(?:"|\Z)|;[^\n]*|[^ \t\n\r\x0b\x0c()|";]+')
_LET = re.compile(r"[?$]x[0-9]+")
_scratch = None     # key_text: the z3 context the assertions are copied into for printing


_TAG = re.compile(r"([a-z]*)([0-9]+)")


def _fresh(sym):
    """`<prefix>!<tag>` (the prefix and the tag's lowercase letters) when the symbol (without quotes) is
    `<prefix>!<lowercase letters><digits>` with a nonempty prefix, split at the last `!`; else None."""
    pre, bang, num = sym.rpartition("!")
    m = _TAG.fullmatch(num) if bang and pre else None
    return pre + "!" + m.group(1) if m else None


def canonical(text):
    """docs/G4B_SPEC.md 7 rule 9: (canonical text, fresh names). Renumbers by first appearance, one counter each:
    the let names `?x<id>` / `$x<id>` (sigil kept), and the fresh names `<prefix>!<letters><n>` (atoms or quoted
    symbols; prefix, letters and quoting kept). The fresh names and let-form names that first appear before the
    `assert` command (the declarations: free constants, functions and sorts, never a bound variable alone or a let
    name) are returned in local order, as symbols without quotes, so the key can record which assertions share them."""
    lets = {}
    fresh = {}
    order = []
    body = []       # nonempty once the atom `assert` has been seen

    def sub(m):
        tok = m.group(0)
        c = tok[0]
        if c in '";':
            return tok
        if tok == "assert" and text[max(m.start() - 2, 0):m.start()] in ("(", "\n("):
            # the command: each command is printed at the start of a line, a declared symbol `assert` never is
            body.append(True)
            return tok
        quoted = c == "|"
        sym = tok[1:-1] if quoted and len(tok) >= 2 and tok[-1] == "|" else (None if quoted else tok)
        if sym is None:
            return tok
        if not quoted and _LET.fullmatch(sym):
            if sym not in lets:
                lets[sym] = "%sx%d" % (sym[0], len(lets) + 1)
                if not body:
                    order.append(sym)   # a declared symbol of the let form: shared across the query like a fresh name
            return lets[sym]
        pre = _fresh(sym)
        if pre is None:
            return tok
        if sym not in fresh:
            fresh[sym] = len(fresh) + 1
            if not body:
                order.append(sym)
        new = "%s%d" % (pre, fresh[sym])
        return "|%s|" % new if quoted else new
    return _LEX.sub(sub, text), order


def piece(text):
    """docs/G4B_SPEC.md 7 rule 9: (lowercase hex SHA-256 of the canonical text, fresh names in local order)."""
    c, names = canonical(text)
    return hashlib.sha256(c.encode("utf-8")).hexdigest(), names


def key_text(head, assertions, memo):
    """docs/G4B_SPEC.md 7 rule 9: the set-option head, then one line per solver assertion in order: the digest of its
    canonical benchmark text (Z3_benchmark_to_smtlib_string with no assumptions), then for each of its fresh names in
    local order a space and that name's number in the query (first appearance over the whole query).
    `assertions` are the objects passed to `Solver.add`, one each, in order; `memo` (id(object) -> (object, piece),
    one per function verifier; holding the object keeps its id unique) renders each distinct object once."""
    import z3
    global _scratch
    if _scratch is None:
        _scratch = z3.Context()
    out = [head]
    num = {}
    for p in assertions:
        hit = memo.get(id(p))
        if hit is None:
            e = p if isinstance(p, z3.BoolRef) else z3.BoolSort().cast(p)     # what Solver.add asserts
            # The printer names a subterm with `let` when its reference count is above one, and in the shared
            # context other live terms add references. A copy in a context holding nothing else is shared only
            # within itself, so the text depends on the formula alone. The copy is dropped right after printing.
            c = e.translate(_scratch)
            t = z3.Z3_benchmark_to_smtlib_string(_scratch.ref(), "benchmark generated from python API", "", "unknown",
                                                 "", 0, (z3.Ast * 0)(), c.as_ast())
            del c
            hit = memo[id(p)] = (p, piece(t))
        digest, names = hit[1]
        out.append(digest)
        for n in names:
            if n not in num:
                num[n] = len(num) + 1
            out.append(" %d" % num[n])
        out.append("\n")
    return "".join(out)


class ProofCache:
    def __init__(self, dir):
        self.dir = dir
        self.hits = 0
        self.misses = 0
        self.stores = 0
        self._n = 0
        import z3
        self._prefix = ("py " + VERSION + "\0" + z3.get_full_version() + "\0")

    def key(self, text):
        return hashlib.sha256((self._prefix + text).encode("utf-8")).hexdigest()

    def lookup(self, text):
        """True only for a well-formed `unsat` entry; any OS error is a miss (not counted here)."""
        k = self.key(text)
        try:
            with open(os.path.join(self.dir, k[:2], k), "rb") as fh:
                return fh.read() == ENTRY
        except OSError:
            return False

    def store(self, text):
        """Write the entry atomically (temp file in the same directory, then rename); never raises."""
        k = self.key(text)
        sub = os.path.join(self.dir, k[:2])
        self._n += 1
        tmp = os.path.join(sub, ".tmp.%d.%d" % (os.getpid(), self._n))
        try:
            os.makedirs(sub, exist_ok=True)
            with open(tmp, "wb") as fh:
                fh.write(ENTRY)
            os.replace(tmp, os.path.join(sub, k))
        except OSError:
            try:
                os.remove(tmp)
            except OSError:
                pass
            return
        self.stores += 1


# ---------------------------------------------------------------------------
# PR1: module proof records, `--module-cache DIR` (docs/G4B_SPEC.md section 7.8)
# ---------------------------------------------------------------------------
MODULE_ENTRY = b"pruv-module-proof 1\nproved\n"
_code_id = None


def code_identity():
    """sha256 over the compiler's own source files (pruvc/*.py, pruvc/data/* and stdlib/*, name and bytes, sorted): any edit of
    the compiler, its built-in catalogs or the bundled std gives new module keys.  None when any of them cannot be listed
    or read: a partial hash could miss an input, so module records are then off."""
    global _code_id
    if _code_id is None:
        here = os.path.dirname(os.path.abspath(__file__))
        h = hashlib.sha256()
        try:
            for d, pat in ((here, ".py"), (os.path.join(here, "data"), ""), (os.path.join(here, "..", "stdlib"), "")):
                for n in sorted(os.listdir(d)):
                    p = os.path.join(d, n)
                    if not n.endswith(pat) or not stat.S_ISREG(os.stat(p).st_mode):
                        continue  # os.stat raises on error (identity off) instead of skipping the file
                    with open(p, "rb") as fh:
                        b = fh.read()
                    h.update(("%s\0%d\0" % (n, len(b))).encode("utf-8"))
                    h.update(b)
            _code_id = h.hexdigest()
        except OSError:
            _code_id = ""
    return _code_id or None


def module_key_text(prog, mi, closure, infer):
    """the key text of the module record of prog.modules[mi]; closure = sorted module indices (mi included) whose
    files its proof can read.  Global inputs: explicit mode, --infer, G9 activation and the G9 metadata entries that
    mention an item of the closure (an entry naming only outside items cannot reach the closure's encoding)."""
    import json
    mods = prog.modules
    ids = {it.name for j in closure for it in mods[j].items}
    meta = getattr(prog, "g9", None)
    if meta is None:
        g9 = "off"
    else:
        known = _all_ids(prog)

        def mentions(x):
            if not isinstance(x, str):
                return any(mentions(y) for y in x)
            head = x.split(".", 1)[0]
            for c in (head, head.rsplit("::", 1)[0]):
                if c in known:
                    return c in ids
            return True                  # not an item identity: keep it (conservative)
        g9 = json.dumps({k: [x for x in meta[k] if mentions(x)] for k in sorted(meta)}, sort_keys=True)
    out = ["pruv-module-proof 1", "explicit %d" % bool(prog.explicit), "infer %d" % bool(infer), "g9 " + g9,
           "module %s\0%s" % (mods[mi].pkg or "", "::".join(mods[mi].path))]
    for j in closure:
        m = mods[j]
        out.append("dep %s\0%s\0%s\0%s" % (m.pkg or "", "::".join(m.path), m.digest, m.rdigest))
    return "\n".join(out) + "\n"


def _all_ids(prog):
    ids = getattr(prog, "_pr1_ids", None)
    if ids is None:
        ids = prog._pr1_ids = {it.name for it in prog.items}
    return ids


def module_closures(prog):
    """sorted transitive closure (by Module.deps) of every module index"""
    out = []
    for i in range(len(prog.modules)):
        seen, stack = {i}, [i]
        while stack:
            for j in prog.modules[stack.pop()].deps:
                if j not in seen:
                    seen.add(j)
                    stack.append(j)
        out.append(sorted(seen))
    return out


class ModuleRecords:
    """Module proof records under DIR/<key[:2]>/<key>. A record means: every function of the module verified
    with no error under exactly the inputs hashed into the key."""
    def __init__(self, dir):
        self.dir = dir
        self.hits = 0
        self.misses = 0
        self.stores = 0
        self._n = 0
        import z3
        cid = code_identity()
        # None = the compiler could not be hashed: every key is None, never hit, never store
        self._prefix = None if cid is None else "py " + VERSION + "\0" + cid + "\0" + z3.get_full_version() + "\0"

    def key(self, text):
        if self._prefix is None:
            return None
        return hashlib.sha256((self._prefix + text).encode("utf-8")).hexdigest()

    def lookup(self, k):
        if k is None:
            return False
        try:
            with open(os.path.join(self.dir, k[:2], k), "rb") as fh:
                return fh.read(len(MODULE_ENTRY) + 1) == MODULE_ENTRY
        except OSError:
            return False

    def store(self, k):
        if k is None:
            return
        sub = os.path.join(self.dir, k[:2])
        self._n += 1
        tmp = os.path.join(sub, ".tmp.%d.%d" % (os.getpid(), self._n))
        fd = None
        try:
            os.makedirs(sub, exist_ok=True)
            fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o644)     # exclusive: never share a temporary file
            os.write(fd, MODULE_ENTRY)
            os.close(fd)
            fd = None
            os.replace(tmp, os.path.join(sub, k))
        except OSError:
            if fd is not None:
                os.close(fd)
                try:
                    os.remove(tmp)
                except OSError:
                    pass
            return
        self.stores += 1
