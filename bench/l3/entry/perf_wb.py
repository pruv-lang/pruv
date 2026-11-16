"""Scaling check for R2/R4: large bound_session_index inputs on work/dev/harness; if work/harness_old exists (a harness
built from the pre-fix sources) it is run too and the outputs are compared.
Every session has a forks table and an engineThreads table (2 sections each, so 2*S+2 sections) and every key appears
twice (first write wins). Outputs must be byte identical; prints wall time per size."""
import json, subprocess, sys, time
from pathlib import Path
HERE = Path(__file__).resolve().parent

def case(s, tables=True):
    ses = []
    for i in range(s):
        et = {"engine": "claude", "thread": "t%d" % (i % (s // 2 or 1))}
        o = {"id": "s%d" % i, "externalThread": et}
        if tables:
            et["forks"] = {"arr": ["f"]}
            o["engineThreads"] = {"obj": [["codex:x%d" % (i % (s // 2 or 1)), {"threadId": "z%d" % i}]]}
        ses.append(o)
    return json.dumps({"id": "P", "op": "bound_session_index", "args": {"sessions": ses}, "script": {"isTemporary": [{"ok": False}]}})

def run(h, line):
    t = time.time()
    p = subprocess.run([str(h)], input=line + "\n", capture_output=True, text=True, timeout=3000)
    return time.time() - t, p.stdout

tables = True
args = sys.argv[1:]
if args and args[0] == 'keys':   # keys only: no forks/engineThreads tables (isolates R2)
    tables = False
    args = args[1:]
for s in [int(a) for a in args] or [500, 1000, 2000]:
    line = case(s, tables)
    tn, on = run(HERE / 'work/dev/harness', line)
    old = HERE / 'work/harness_old'
    tag = 'keys-only ' if not tables else 'tables '
    if old.exists():
        to, oo = run(old, line)
        print(tag + 'sessions=%d old=%.2fs new=%.2fs identical=%s len=%d' % (s, to, tn, oo == on, len(on)))
    else:
        print(tag + 'sessions=%d new=%.2fs len=%d' % (s, tn, len(on)))
    if '"terminal":"ok"' not in on: print(on[:300])
