# Pinned Qwen oracle

`qwen.ts` and `tail.ts` are verbatim `git show fa192269:src/backend/engine-import/<name>` copies from Lair. `tail-info.ts` extracts the private `tailInfo` function verbatim, adding only imports and `export`. Original comments remain in their original language.

SHA-256: qwen.ts `3e2b034f5b0ad6dc46136ace3faa793c4f1025e7db2111bd1373fb408f9dd280`; tail.ts `750399578ca53bcf6ccf0b2b03863de08d0f16a3580c60f40d9ef8c216c3c14e`.

Node 26.4.0 (`/opt/homebrew/bin/node --experimental-strip-types`) executes them. `hooks.mjs` resolves extensionless imports and redirects unused subprocess helpers to throwing stubs. `plan.ts` returns undefined because lastPlan is deferred. `opencode.ts` contains the pinned non-win32 folder key and open-window constant. Filesystem calls use Node's real local filesystem rather than a mock: the driver builds isolated Qwen homes under `../work/q/`. Nothing invokes the Lair application, its engines, or network services.

`../reference.mjs` compares actual `readQwen`, head, extracted tail, and `listQwen` output. An explicit `not-found` record is retained for the empty-cwd fixture: the positional Pruv fixture host intentionally does not authorize a folder, whereas `readQwen` rejects the mismatch (P5). Listing rejects it on both sides.

`../oracle.py` is a narrow policy oracle, not a second importer. For P1 it removes invalid UTF-8 lines; for P4 it removes the two specifically injected unsupported dates. The driver requires complete equality to TS rerun on that normalized input. The fixed invalid-calendar list difference changes only updatedAt, to that normalized tail timestamp. There is no broad field waiver.

Run from the repository root, with `PYTHONDONTWRITEBYTECODE=1`:

```
/Users/spica4345/Projects/pruv/.venv/bin/python bench/l3/qwen/test_impl.py
/Users/spica4345/Projects/pruv/.venv/bin/python bench/l3/qwen/q.py
/Users/spica4345/Projects/pruv/.venv/bin/python bench/l3/qwen/q_mutants.py
/Users/spica4345/Projects/pruv/.venv/bin/python bench/l3/qwen/q_measure.py
```

The first command builds verified four-pair hosts. Q scripts reuse them; `q_root.pruv` mirrors the original root for isolated mutant builds. JSON reports, corpora, manifests, mutated sources and binaries stay under `work/`. The measurement is one size sweep, one cold call per size, not a statistically controlled benchmark: Pruv times window_packet only; TS times readQwen including head/file I/O. Startup is excluded on both sides. Each entry is exactly 8192 bytes, including inert padding; 256/512/1024 linked entries fill 2/4/8 MiB. Denser chains may be much slower.
