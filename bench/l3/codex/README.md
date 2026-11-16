# L3-5 Codex conversation import

Pruv port of Lair's Codex import (`listCodex` / `readCodex`, commit 744d14af), compared with the pinned TypeScript in
`reference/`. Design, protocol, packet layout, failure codes, policies P1-P8 and test plan: `docs/CODEX_IMPORT_SPEC.md`.

Run from the repository root:

```
PATH="$HOME/.cargo/bin:$PATH" python3 bench/l3/codex/test_impl.py
```

The driver builds the offline Rust renderer crate (`host/`) and the string adapter, verifies and compiles `root.pruv`
with both compilers and both backends (py/rust, py/llvm, rs/rust, rs/llvm), runs the pinned TypeScript on the same
fixtures first (it must agree with the checked-in expected JSON), then runs each pair and compares result JSON, expected
JSON and the requests the fake server recorded. It prints `N/N passed` per pair and exits nonzero on any mismatch.
Build products stay in `work/` (git-ignored). Options: `--verify-only`, `--reuse-verified` (host-only edits; reuses the
previously verified emission), `--only SUBSTRING` (fixtures whose name contains it).

Dependencies: the repository's compiler/solver installations, Rust (cached `serde_json`, offline), clang, Python 3,
Node (`CODEX_NODE`, default `/opt/homebrew/bin/node`, run with `--experimental-strip-types`).

## Layout

| File | Role |
|---|---|
| `root.pruv` | `codex_list` / `codex_read` (`! async, net, proc, alloc`), `mod` lines of the packages and local modules |
| `rpc.pruv` | spawn (`start`), initialize + initialized (`open`), response pump (`wait`/`call`), request builders, error classes |
| `list.pruv`, `read.pruv`, `item.pruv`, `plan.pruv`, `folder.pruv` | list rows, page absorption / held turns / collector replay / result, turn items, `codexPlan`, `folderKey` |
| `packet.pruv`, `util.pruv` | packet assembly and checks, JSON/byte helpers (`elements`, `id_is`, `escaped`, ...) |
| `pruv.json` | manifest: relative dependencies on `../pkgs/<name>` (version 0.1) |
| `host/` | Rust crate `codex_host`: decodes the packet and renders the TS JSON (key order as TS builds it) |
| `harness.rs`, `str_adapter.rs` | stdin JSON request -> `sol::codex_list/read` -> one JSON line (the harness only parses requests) |
| `q_server.py` | fake `codex app-server --stdio` driven by a scenario JSON; validates the handshake and every request shape, records requests |
| `reference.mjs` | runs the pinned TS `listCodex`/`readCodex` per request against the fake, one JSON line each |
| `common.py` | fake-binary wrapper, scenario compilation, runners, per-case checks |
| `fixtures/cases.json` | fixed cases: scenario (or list/meta/pages shorthand) + expected JSON + expected requests |
| `oracle.py`, `q.py`, `q_mutants.py` | Q differential (see `Q_README.md`) |
| `IMPL_NOTES.md` | spots to check first |

## Fixtures

`fixtures/cases.json` is `{"cases": [...]}`. A case has `name`, `mode` (`list`/`read`), optional `folder`, `thread`,
`timeout`, `binary`, and either a full `scenario` or the shorthands `list` / `list_actions` (thread/list), `meta` /
`read_actions` (thread/read), `pages` / `pages_actions` (thread/turns/list), `init` (initialize). Checks: `expected`
(result JSON), `expect_calls` (the non-handshake requests, params subset-matched), `expect_count`, `expect_recv`
(client lines such as the -32601 replies), `expect_no_spawn`. Macros (shared by the fake and the expected JSON):
`{"$str":S,"n":N}`, `{"$list":ITEM,"n":N,"base":B,"desc":bool}` (`"$i"` -> integer, `{i}` in strings),
`{"$concat":[LISTS]}`, `{"$cat":[STRINGS]}`, `{"$join":LIST,"sep":S}`.

The fake is started through a wrapper script `work/fakecodex` (`#!/bin/sh exec python q_server.py "$@"`), because the
extra argv follows `app-server --stdio`: it receives `[scenario, log]`. Both sides get separate logs; the logs are
compared as parsed JSON (the cursor is re-sent as the raw token on the Pruv side, re-serialised on the TS side).

## Boundary

`codex_list(binary, argv, cwd, folder, timeout_ms) -> [i64]` and
`codex_read(binary, argv, cwd, folder, thread, timeout_ms) -> [i64]`; `argv` is the NUL-terminated blob of the extra
arguments (Pruv prepends `app-server`, `--stdio`). Packet: `[0,16,row_words,byte_len,...meta16,...rows,...text]` or
`[code,stage,detail_len,...detail]`. No `trusted` function is declared: spawn and the event loop are G10 builtins.

Results (2026-11-16): `root.pruv` with its packages verifies on both compilers. `test_impl.py`: the pinned TypeScript
136/136 against the expected JSON, and 136/136 on each of py/rust, py/llvm, rs/rust, rs/llvm (same results and same
recorded requests as TypeScript). `q.py` (2023 cases): 0 unexplained on every pair; see `Q_README.md`. Mutants: 16/16
killed; see `Q_README.md`.
