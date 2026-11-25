"""Saved expected outputs (golden files) for the rs-only test tools.

Since bundle PF the Python compiler (pruvc/) is frozen and no test compares Python output with Rust output any more.
The tools that used to do that (tests/diff_*.py, ...) now compare the Rust compiler's output with a golden file that
was produced by the Rust compiler and checked once against the frozen Python compiler when it was captured.

Golden file format (UTF-8 text, LF, sorted, one line per BUCKET of cases so the file stays small):

    # golden <name>: <free text describing the inputs>
    <bucket>\t<tally>\t<member>=<digest>\t<member>=<digest>...

  A case key is "<group>|<input>|<mode>" (paths relative to the repo, "/"; the tool may append a "#dupN"/"~N" suffix).
  bucket   "<group>|<input>" with a generated-variant index removed from <input> ("x.pruv#17" -> "x.pruv",
           "fuzz:41:300" -> "fuzz:41"); all modes and variants of one input share one line.
  member   the rest of the key: "<mode>" or "<index>/<mode>".
  digest   first 12 hex digits of sha256 of the normalized output text (see normalize()).
  tally    small human-readable sample of the cases in the bucket, e.g. "4 cases: ok 3, error 1"; it is not compared,
           it only makes diffs readable. The per-case summaries are printed by the tools on a mismatch.

Usage in a tool:

    import golden
    golden.add_args(ap)                                   # --golden PATH, --update-golden, --prune-golden
    g = golden.Golden.from_args(args, default_path, "diff_rust: Rust source per (input, mode)")
    g.check(key, text, summary)                           # once per case
    ok = g.finish()                                       # prints the report; writes the file with --update-golden

Rules
  - Check mode (default): a case whose digest differs from the file, or a case missing from the file, is a failure.
    Keys in the file that this run did not produce are fine (a --quick run produces a subset).
  - --update-golden: rewrite the file with this run's cases (keys not produced by this run are kept, unless
    --prune-golden is also given) and print every CHANGED / NEW / REMOVED key. Exit status is still 0 for these.
  - Normalization removes what depends on the machine: the repo root and temporary directories become <ROOT> and
    <TMP>, in plain, JSON-escaped and backslash forms, and CRLF becomes LF.
"""
import hashlib
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
GOLDEN_DIR = os.path.join(HERE, "golden")
DIGEST_LEN = 12
VARIANT_RE = re.compile(r"^(.*\.pruv)#(\d+)$|^(fuzz:\d+):(\d+)$")   # mutation / fuzz variant index
SUMMARY_MAX = 80
MAX_SHOWN = 20


def _forms(path):
    """The spellings of one directory path that can appear in compiler output."""
    out = set()
    if not path:
        return out
    path = os.fspath(path)
    for p in (path, os.path.realpath(path)):
        p = p.rstrip("/\\")
        if not p:
            continue
        for q in (p, p.replace("\\", "/"), p.replace("/", "\\")):
            out.add(q)
            out.add(json.dumps(q, ensure_ascii=False)[1:-1])          # JSON-escaped form (\\ for \)
            out.add(json.dumps(json.dumps(q, ensure_ascii=False)[1:-1], ensure_ascii=False)[1:-1])
    return out


def normalize(text, tmp_dirs=()):
    """Machine-independent form of a compiler output text (str or bytes)."""
    if isinstance(text, bytes):
        text = text.decode("utf-8", errors="replace")
    text = text.replace("\r\n", "\n")
    subs = []
    for d in tmp_dirs:
        subs += [(f, "<TMP>") for f in _forms(d)]
    subs += [(f, "<ROOT>") for f in _forms(ROOT)]
    import tempfile
    subs += [(f, "<TMP>") for f in _forms(tempfile.gettempdir())]
    # longest first so a temp dir inside the repo (or /private/var vs /var) is replaced as a whole
    for f, rep in sorted(subs, key=lambda s: -len(s[0])):
        if f and f in text:
            text = text.replace(f, rep)
    # after the prefix, the rest of a replaced path may still use backslashes on Windows
    text = re.sub(r"(<(?:ROOT|TMP)>)((?:\\\\|\\)[^\s\"'<>|:*?]*)",
                  lambda m: m.group(1) + re.sub(r"\\\\|\\", "/", m.group(2)), text)
    return text


def digest(text, tmp_dirs=()):
    return hashlib.sha256(normalize(text, tmp_dirs).encode("utf-8")).hexdigest()[:DIGEST_LEN]


def _clean(s, limit):
    s = re.sub(r"[\t\r\n]+", " ", str(s)).strip()
    return s[:limit]


def add_args(ap):
    ap.add_argument("--golden", default=None, metavar="PATH",
                    help="golden file (default: the tool's file under tests/golden/)")
    ap.add_argument("--update-golden", action="store_true",
                    help="rewrite the golden file from this run's Rust output and print the differences")
    ap.add_argument("--prune-golden", action="store_true",
                    help="with --update-golden: also drop keys this run did not produce")
    return ap


def split_key(key):
    """case key -> (bucket, member); see the module docstring."""
    parts = key.split("|", 2)
    if len(parts) < 3:
        return key, ""
    group, inp, mode = parts
    m = VARIANT_RE.match(inp)
    if m:
        base, idx = (m.group(1), m.group(2)) if m.group(1) else (m.group(3), m.group(4))
        return "%s|%s" % (group, base), "%s/%s" % (idx, mode)
    return "%s|%s" % (group, inp), mode


def join_key(bucket, member):
    if not member:
        return bucket
    group, inp = bucket.split("|", 1)
    idx, sep, mode = member.partition("/")
    if sep and idx.isdigit():
        inp = "%s:%s" % (inp, idx) if inp.startswith("fuzz:") else "%s#%s" % (inp, idx)
        member = mode
    return "%s|%s|%s" % (group, inp, member)


def _member_order(m):
    idx, sep, mode = m.partition("/")
    return (0, int(idx), mode) if sep and idx.isdigit() else (1, 0, m)


def tally(summaries, limit=4):
    """'N cases: ok 3, error parse/syntax 1' (the first word, or the first two for errors, of each summary)."""
    c = {}
    for s in summaries:
        w = s.split()
        t = " ".join(w[:2]) if w[:1] == ["error"] else (w[0] if w else "?")
        c[t] = c.get(t, 0) + 1
    top = sorted(c.items(), key=lambda kv: (-kv[1], kv[0]))
    txt = ", ".join("%s %d" % kv for kv in top[:limit]) + (", ..." if len(top) > limit else "")
    n = len(summaries)
    return _clean("%d case%s: %s" % (n, "" if n == 1 else "s", txt), SUMMARY_MAX)


def read(path):
    """-> (header line or None, {key: (digest, summary)}, {bucket: tally})"""
    header, rows, tallies = None, {}, {}
    if not os.path.isfile(path):
        return header, rows, tallies
    with open(path, encoding="utf-8") as fh:
        for ln in fh:
            ln = ln.rstrip("\n")
            if not ln:
                continue
            if ln.startswith("#"):
                header = header or ln
                continue
            parts = ln.split("\t")
            if len(parts) < 3:
                raise ValueError("%s: bad golden line %r" % (path, ln[:100]))
            bucket = parts[0]
            tallies[bucket] = parts[1]
            for tok in parts[2:]:
                member, eq, dg = tok.rpartition("=")
                if not eq:
                    raise ValueError("%s: bad golden entry %r" % (path, tok[:100]))
                rows[join_key(bucket, member)] = (dg, "")
    return header, rows, tallies


class Golden:
    def __init__(self, path, name, description, update=False, prune=False, tmp_dirs=()):
        self.path = path
        self.name = name
        self.description = description
        self.update = update
        self.prune = prune
        self.tmp_dirs = list(tmp_dirs)
        _, self.saved, self.tallies = read(path)
        self.seen = {}
        self.mismatch = []   # (key, saved (digest, summary), new (digest, summary))
        self.missing = []    # (key, new)

    @classmethod
    def from_args(cls, args, default_name, description, tmp_dirs=()):
        path = args.golden or os.path.join(GOLDEN_DIR, default_name)
        return cls(path, os.path.basename(path), description, args.update_golden, args.prune_golden, tmp_dirs)

    def add_tmp(self, d):
        self.tmp_dirs.append(d)

    def check(self, key, text, summary=""):
        key = _clean(key, 1000)
        new = (digest(text, self.tmp_dirs), _clean(summary, SUMMARY_MAX))
        if key in self.seen and self.seen[key][0] != new[0]:
            # the same key twice in one run with different output: the tool's keys are not unique
            self.mismatch.append((key, self.seen[key], new))
        self.seen[key] = new
        old = self.saved.get(key)
        if old is None:
            self.missing.append((key, new))
            return False
        if old[0] != new[0]:
            self.mismatch.append((key, old, new))
            return False
        return True

    def _write(self, rows):
        os.makedirs(os.path.dirname(self.path) or ".", exist_ok=True)
        lines = ["# golden %s: %s" % (self.name, _clean(self.description, 300))]
        buckets = {}
        for k, v in rows.items():
            b, m = split_key(k)
            buckets.setdefault(b, {})[m] = v
        for b in sorted(buckets):
            mem = buckets[b]
            fresh = [s for s in (v[1] for v in mem.values()) if s]
            t = tally(fresh) if len(fresh) == len(mem) else self.tallies.get(b) or tally(fresh)
            lines.append("\t".join([b, t] + ["%s=%s" % (m, mem[m][0]) for m in sorted(mem, key=_member_order)]))
        with open(self.path, "w", encoding="utf-8", newline="\n") as fh:
            fh.write("\n".join(lines) + "\n")

    def finish(self, out=None):
        """Print the report; with --update-golden write the file. -> True when the run matches (or was updated)."""
        out = out or sys.stdout
        rel = os.path.relpath(self.path, ROOT).replace(os.sep, "/")
        if self.update:
            rows = dict(self.saved)
            removed = []
            if self.prune:
                removed = sorted(k for k in rows if k not in self.seen)
                for k in removed:
                    del rows[k]
            rows.update(self.seen)
            changed = sorted(set((k, o, n) for k, o, n in self.mismatch))
            for k, o, n in changed:
                out.write("CHANGED %s: %s (%s) -> %s (%s)\n" % (k, o[0], o[1], n[0], n[1]))
            for k, n in sorted(self.missing):
                out.write("NEW %s: %s (%s)\n" % (k, n[0], n[1]))
            for k in removed:
                out.write("REMOVED %s: %s (%s)\n" % (k, self.saved[k][0], self.saved[k][1]))
            self._write(rows)
            out.write("golden %s updated: %d cases this run, %d changed, %d new, %d removed, %d total\n" % (
                rel, len(self.seen), len(changed), len(self.missing), len(removed), len(rows)))
            return True
        for k, o, n in self.mismatch[:MAX_SHOWN]:
            out.write("GOLDEN MISMATCH %s: saved %s (%s), now %s (%s)\n" % (k, o[0], o[1], n[0], n[1]))
        for k, n in self.missing[:MAX_SHOWN]:
            out.write("GOLDEN MISSING %s: now %s (%s)\n" % (k, n[0], n[1]))
        bad = len(self.mismatch) + len(self.missing)
        out.write("golden %s: %d cases checked, %d match, %d mismatch, %d missing\n" % (
            rel, len(self.seen), len(self.seen) - bad, len(self.mismatch), len(self.missing)))
        if bad:
            out.write("  if the change is intended: rerun with --update-golden (prints every changed key) and commit "
                      "the file; to see the output use the tool's --save-dir\n")
        return bad == 0
