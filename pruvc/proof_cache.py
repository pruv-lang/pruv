"""G4b-3: local content-addressed proof cache (docs/G4B_SPEC.md section 7).

Only `unsat` answers are stored. An entry is the file DIR/<key[:2]>/<key> holding exactly ENTRY.
"""
import hashlib
import os
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
