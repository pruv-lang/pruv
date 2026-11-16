# L3-6 Grok import (bench/l3/grok)

Pruv port of Lair's Grok conversation import (`reference/grok.ts`, pinned at Lair commit `744d14af`). Spec: `docs/GROK_IMPORT_SPEC.md`;
implementation design: `DESIGN.md`; ABI/cross-module contract of the Pruv sources: `IMPL_NOTES.md`. Pruv makes every decision; the Rust host only
runs the 13 primitives and applies bank patches.

| Path | What |
|---|---|
| `root.pruv packet.pruv util.pruv path.pruv summary.pruv list.pruv read.pruv entry.pruv framing.pruv copy.pruv rpc.pruv pruv.json` | the importer (G4b manifest) |
| `host.rs harness.rs str_adapter.rs host/` | primitive host (crate `grok_host`), harness linking the generated `sol` module, string adapter |
| `reference/` | pinned TypeScript, never modified; `sort/` the V8 sort port and its Node probe |
| `reference.mjs reference-hooks.mjs reference-primitives.mjs` | runs the pinned exports on Node 26.4.0 with the same primitive schedule/trace as the host |
| `common.py oracle.py q_server.py` | fixture trees, runner/reference execution, trace normalisation; the narrow policy-aware comparison; the fake Grok ACP |
| `test_impl.py` | builds the four pairs (py/rs compiler x rust/llvm backend, verified, G4b manifest) and runs `fixtures/` on TS and each pair; also the fixture library of the other two scripts |
| `fixtures/` | `_defaults.json` + 14 JSON files, about 250 fixed cases |
| `q.py` | seeded random scenarios (`--cases 2000 --seed 20261116`) plus the fixed cases; summary JSON |
| `q_mutants.py` | M01-M16 source mutants on copies of the `.pruv` files, rs/rust |
| `Q_README.md` | how to run Q, what is checked, result tables (placeholders until run) |
| `work/` | generated: builds, case trees (`cases/<side>/<name>`), reports, mutants. Disposable. |

## Quick start

```
export PATH="$HOME/.cargo/bin:$PATH" GROK_TS_TRANSPILER=/path/to/typescript     # typescript package, used only to transpile plan.ts
python3 bench/l3/grok/test_impl.py --list                       # the fixtures and the mutants they map to
python3 bench/l3/grok/test_impl.py --pairs rs/rust --only paths # one pair, some fixtures
python3 bench/l3/grok/test_impl.py                              # all four pairs, all fixtures (nonzero exit on any failure)
python3 bench/l3/grok/q.py --cases 2000 --seed 20261116
python3 bench/l3/grok/q_mutants.py --check && python3 bench/l3/grok/q_mutants.py
```

`test_impl.py` options: `--verify-only`, `--reuse-verified`, `--only SUBSTRING` (repeatable), `--category NAME` (a fixture file stem), `--pairs`,
`--no-ts` (only `expected` blocks), `--jobs`, `--slow` (the real 20 s deadline, the 16 MiB growth race), `--keep`, `--list`.
Nothing here uses `--no-verify`.

## Fixture files (`fixtures/*.json`)

A JSON list of cases in the DESIGN section 6 format (see `common.py` for the full key list: `mode` 0-5 as list/read/copy/load/small-json/caller-cleanup,
`tree`, `schedule` with `readdir`/`entropyHex`/`rules`, `acp`, `acp_options`, `rpcMs`, `binary`, `watchdog`, `concurrency`, `expected`), plus conveniences
expanded by `test_impl.py` and invisible to the host, the reference and `oracle.py`:

* `_defaults.json` per-mode defaults (folder `/work` for list/read, `/` for copy/load because the ACP child runs with cwd=folder).
* `gen`: `{"session": {...}}` one session directory (summary overrides, `summary_size`, `history` lines, `files`, `link`, `readdir`) and
  `{"sessions": {...}}` many (ids or prefix+n, `dates` as tokens/`asc`/seeded spec, `summary_by_id`, `also`, `order`) with the readdir schedule pinned, so
  both sides see the same enumeration order. History elements: an object (one line), `{"$raw"}`, `{"$hex"}`, `{"$line": {size, type, pre, lead, ...}}`
  (a line of an exact byte size), `{"$repeat": ELEMENT|[ELEMENTS], "count"}`. `{"$raw": "1e400"}` keeps raw number text; `{"$del": 1}` deletes a key.
* string macros `{"$rep": "x", "n": 129}` and `{"$cat": [...]}`; `rawhex_of` (ACP action text -> `rawhex`).
* `kills: ["M01"]` maps a fixture to the mutants it must kill; `slow`, `disabled`.

Categories (file) and what they cover:

| File | Cases | Covers |
|---|---|---|
| `paths` | 19 | percent-encoding (slash/space/Unicode/reserved/`%`), empty/dot/dotdot folder, invalid and 128/129-char threads, trailing-slash and all-slash cwd, no case folding/dot resolution, lone-surrogate cwd (M01) |
| `identity` | 18 | linked home accepted; linked sessions/folder/thread rejected; symlink/FIFO/directory summary and history; hard link; sibling-prefix containment; replacement between lstat and open, after open, and an ancestor swapped before realpath (M02, M03) |
| `summaries` | 26 | 65536/65537 sizes, growth after the check (also past 16 MiB, slow), malformed/array/null/string roots, duplicate keys, wrong id/cwd, field types, missing history, small-json numbers/surrogates/overflow, invalid UTF-8 and depth 129, open/read errors swallowed (M04) |
| `listing` | 25 | 51 and 60 sessions with newer rejected ones, mixed missing dates at 3/5/7/8/10/64/65/130 entries, ties, date variety (P4), titles (fallback, whitespace, 120-unit cut, lone surrogate), message counts (P9), non-directory entries, four-slot concurrency (A-E and four outstanding), readdir error, missing home (M05, M06) |
| `active` | 13 | strings/objects/precedence, empty id suppressing the fallback, non-string ids, wrong roots, 65536/65537 bytes, unreadable, symlink, only the top 50 marked, invalid UTF-8 (M07) |
| `content` | 16 | string/array parts, empty answers, tool_calls with junk, plan-like calls, junk lines, duplicate keys, CRLF, timestamp fallback and P4 variants, P1/P2/P3 history cases (M11) |
| `tags` | 7 | `<user_query>` unwrapping (CRLF/LF/lone CR/boundaries/repeats), `<user_info>` with prompt_index variants, synthetic_reason variants (M08, M09) |
| `dropped` | 14 | oversized user prefixes (plain, synthetic early/late/numeric, type not first, escaped key, leading blanks), oversized assistant and final fragment, the 1 MiB cap with LF/CR at +0/+1 (M10) |
| `budgets` | 12 | 200/201 turns, 2 MiB suffix, saturation and empty answers, scalar-boundary clips, 8 MiB window on/off/mid line, empty and directory history (M12) |
| `read` | 17 | title/model priority and absence, long model, no history, open/read errors, ENOENT race, appended and truncated history, summary growth, hard-linked history |
| `copy` | 30 | all 13 allowed names, skipped counts, non-regular allowed names, empty source, no summary, chunk boundaries, target exists (file/dir/dangling), race before the second lstat, first/second lstat errors, temp collision, umask, faults at mkdir/readdir/read/create/write/rename and a failing cleanup, source replaced before open, wrong engine, invalid thread, empty target (M13, M14) |
| `verify` | 9 | copy followed by initialize/session/load errors, no loadSession, timeout, engine missing, early exit, failing target cleanup (M15, M16) |
| `acp` | 47 | load mode: capability variants, every `error.code` type, notifications and server requests (incl. null id), malformed lines, spawn/exit/timeout, P1, P6, P10, P11, P12, P7 lifecycle (M16) |
| `rollback` | 4 | caller cleanup of the returned copy |

## Differences between `host.rs` and `reference.mjs` input/output

* Error JSON has the same shape on both sides (class names `RawOsError`/`OtherError`, `errnoClass` `"none"` when absent, the `raw` key omitted when
  there is no raw code). The host prints no `{"meta":...}` line.
* Schedule `bytes`/`blob` accept only `utf8`/`hex` in `host.rs`, `repeat`/`concat` in the reference; `test_impl.prepare` converts recipes to hex.
* A hold's `releaseWhen` needs `"event": "issue"` in the reference and is ignored by the host; fixtures carry it.
