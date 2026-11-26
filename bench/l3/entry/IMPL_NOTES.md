# L3-8 implementation notes

> Path note (2026-11-17): the modules named below moved to `bench/l3/lib/` as `ent_<name>.pruv` (`crate::ent_<name>::`); the body of `root.pruv` is `bench/l3/lib/entry_api.pruv` and `root.pruv` only forwards. Compile with `--lib bench/l3/lib`. Older text below keeps the old file names.

Worker notes from the implementation passes, merged in order. Each section keeps the worker's text; the lead's
resolutions of the open points they raised come first.

## Lead resolutions

- Engine trace refs: the host renders a ref argument as `{"ref":name}`; the oracle (`reference-primitives.mjs` engineCall),
  `gen_fixtures.py` D08 checks and `integrate.py` were changed to the same object form (W4core item 3).
- Probe expectations (`gen_fixtures.probe_expect`): a probe counts awaits, and untraced `account_at` requests are awaits.
  The served trace is now derived by inserting one untraced slot per Claude account row after `accounts claude`; a probe
  inside the Codex account phase is refused (its body interleaves account_at with traced requests). This fixed
  P06/unit-step and P08/duplicate-3 (W4imp).
- Ref identity of thrown values: a ref names one object. The oracle caches the first errno error built for a ref; the host
  now does the same in `err_obs` (first thrown value wins). This explained Q0087/Q1826 (two realpath entries sharing `rpe`
  with codes `""` and `ENOENT`) (W5).
- `{"u16":[...]}` object keys: the oracle now decodes them like the host (W4core item 1, X-th-slot-sur now agrees).
- Mutant sites M02 (env_or_home slot 6 instead of the ambient request) and M05 (an invalid Claude account fails the step
  instead of being skipped) were filled in `q_mutants.py`. M04 was first a reversed-order mutant; the audit (R10) required
  a real dedupe mutant, see "Audit fixes" below. All 16 are killed.

## W1: Host and harness

Status: only the foundation exists. Not implemented: packet, value, path, accounts, state, canon, sources, dispatch, bound.
root.pruv is a stub that does NOT implement the entry contract (it returns zero arrays); do not treat it as the port.

Verify command (from bench/l3/entry, PATH with ~/.cargo/bin):
  /Users/spica4345/Projects/pruv/pruvc-rs/target/release/pruvc root.pruv -o /tmp/probe/out.rs --manifest pruv.json
Result: {"ok": true}  (u16.pruv + stub root only)

Proof workarounds learned:
- Total accessors rd/wr in u16.pruv (out of range read gives 0, write dropped) keep indexing proofs cheap.
- alloc needs a provable 0 <= n <= 2^40: clamp both sides, or guard inputs (cat guards each input at 2^30-1).
- A loop start variable must be clamped into 0..n before the invariant (index_of clamps from to n).
- pub fn with an inout parameter is rejected in root.pruv; submodules are fine.
- No if-expressions, no consts (use nullary fns with ensures result == K).

Not done: normalizeString port and node probes, bound-index complexity bound, excluded-domain InvalidPacket list, mutants M02-M16 (only M01 in u16.pruv is_trim_unit).

## W2: Oracle

Files: host.rs (crate body, included by host/src/lib.rs as `entry_host`), host/Cargo.toml, host/Cargo.lock (pruned,
serde_json only), harness.rs (`#[path="@SOL@"] mod sol;` + `Sol` impl + `entry_host::run`), str_adapter.rs (symmetry only).

### Commands and results
    cd bench/l3/entry/host
    cargo build --offline --locked   # ok
    cargo test --offline --locked    # 7 unit tests pass (JV codec, rendering, key order, packet round-trip,
                                     #   session encoding, corrupt/pad, mock-core driver smoke + case errors)
Not run: harness.rs against a real generated sol (none exists yet at W2), no end-to-end fixture runs.

### Uncertainties (interpretations made)
1. Bare numerics in record specs (status, count, rowKind, thrown, engine word, route, opKind, recipe action, request
   code, done first word) are single raw words; every {..} slot is a 3-word VS.
2. Tables are added in order of first reference; input/completion packets have no state section (sections: pool,
   record, tables...). Await/done have state at 2 and rows at 3 (host checks >= 4 sections).
3. Trace shapes: ["accounts","claude"], ["ambient","CODEX_HOME"], ["homedir"], ["cwd"], ["realpath",p], ["stat",p],
   ["isTemporary",id], ["engine","<route>","list"|"read",{source,folder,thread?,options}] with refs as {"ref":name}.
   Route names assumed order claude,codex,opencode,qwen,grok. Check against DESIGN 5.2.
4. Numbers render as a JSON integer when a safe non-(-0) integer, else {"num":"16hex"}.
5. InternalInvariant results carry an extra "note" key (host diagnostics) and driver null.
6. Probes: at=0 targets begin's input, at=k the k-th completion. duplicate: driver="InvalidPacket", the host replays the
   previous completion against the new await and expects a boundary done; reentry: step on the done packet.
   If the core does not answer with a boundary, a "note" is added instead of failing the case.
7. Done packet seq accepted if equal to the last await seq or 0; op echo is checked loosely.
8. errno message is `CODE: description, syscall 'path'`; unnamed errno refs get `$errnoN`. Primitive thrown values
   have undefined name/message/code in E.
9. Engine row/record refs and thrown refs form the "seen" set used to validate core-returned refs.
10. host/Cargo.lock was derived from opencode's and pruned by `cargo build --offline`.

## W3: Fixtures (first pass)

Run: `node --experimental-strip-types --no-warnings --import ./reference-hooks.mjs reference.mjs < cases.jsonl`
(`oracle.py` wraps this: `run_oracle(cases)`, or `.venv/bin/python oracle.py cases.jsonl [out.jsonl]`).
Pinned sources are not edited. Redirection is by `registerHooks` keyed by (importing file, specifier) in `reference-hooks.mjs`.

### Stubbed or redirected imports, per pinned file

| pinned file | specifier | goes to |
|---|---|---|
| backend-engine-import.ts | node:os | reference-primitives.mjs (homedir from script) |
| | ./accounts, ./codex-home, ./engine-import/tail | real pinned files |
| | ./engine-import/{claude,codex,grok,opencode,qwen} | fakes/engine-*.ts (record trace, serve `script.engine`) |
| backend-codex-home.ts | node:fs, node:os | primitives (realpathSync, statSync, homedir from script) |
| | ./accounts | real |
| | ./engine-path, ./provider-accounts, ./wire-reason | fakes/stubs.ts (throw if reached) |
| | ../shared/i18n/wire | fakes/wire.ts (WireError with `key`) |
| | codex-owned-runner, engine-contract | `import type`, erased |
| backend-accounts.ts | ../shared/contracts | fakes/stubs.ts ENGINE_IDS (equal to shared-contracts.ts:17) |
| | ../shared/i18n/wire | fakes/wire.ts |
| backend-engine-import-tail.ts | node:child_process, ../engine-children, ../windows-system | stubs |
| backend-engine-import-bound.ts | only `import type` | nothing |
| grok/reference/opencode.ts | node:os | primitives |
| | ../engine-children, ./plan | stubs |
| | ./tail | entry tail |
| backend-engine-import-qwen.ts | ./opencode | grok opencode.ts; ./tail -> entry tail; ./plan -> stubs |

Also patched at run time (per case, restored after): `process.cwd` (serves script, traces `["cwd"]`) and `process.env`
(Proxy: only `CODEX_HOME` readable; any other string key read is recorded as a violation and makes the case `native_throw`).

### Probe runs (work/w3/cases.jsonl, cases2.jsonl, c3.jsonl; one case per op)

All ran under Node v26.4.0 and gave results:
- P1 import_sources: 6 ordered rows, trace homedir, accounts claude, ambient CODEX_HOME, accounts codex, homedir x3. ok.
- P2 find_source: 2.
- P3/Q1 list_external (short titles): `["keep",{"ref":"r1"}]...`; Q4 (250-char title): `copy_title` with the cut title.
- P4 read_external: `{"ref":"rec"}`, trace has `thread` only for read.
- P5/P5b account_home: path / `throw_account_wire` key `wireEngine.account.invalidId`.
- P6 codex_account_folder: `throw_plain_error` with the folder mismatch text (no scripted realpath/stat for the default case).
- P7 session_threads `[["claude","t"],["claude","f1"],["codex","z"]]`; P8 excluded_from_bound true; P9 bound_session_index
  `[["claude:t","s1"]]` with `callerTitles` `[["s1","b"]]` (last wins).
- P10 reject with a scripted EngineImportError: `throw_existing {"ref":"e1"}`. Q2 homedir throw: `throw_existing`. Q3 sync_throw
  primitive 5 from qwen list: `throw_import` code read-failed, cause 5, hasCause true.
- common.py: `host_path('py_rust')` and one `compare` call checked by hand. oracle.py checked on cases2.jsonl and c3.jsonl.

No repository test suite was run. No fixtures exist yet (fixtures/ is absent), so `load_cases` is untested.

### Uncertainties

- shared-contracts.ts cannot load (its `./engine-model` is not pinned), so accounts.ts gets the ENGINE_IDS stub; `ORACLE_CONTRACTS=real` forces the real file (fails).
- Trace route is the engine name string and op `list`/`read`; confirm against host.rs.
- Harness binary name is `harness` per the task wording (opencode piece uses `runner`).
- stat serves BigInt dev/ino; only compared via sameFile equality.
- A thrown primitive is not identity-comparable: a scripted primitive thrown through a wrapped EngineImportError cause is emitted as the value (`cause: 5`), not `{"ref"}`; a primitive thrown directly would be `native_throw` unless registered (not exercised).
- P5 in `compare` is approximated: native-message masking applies only when both sides threw plain/import and the trace used realpath/stat/homedir/cwd.
- Probe P6 relied on defaults; the `__proto__` slot assignment, fixtures `forward` key and u16 lone surrogates were not probed.
- Not exercised: realpath/stat scripts with errno, EACCES/ELOOP paths, `plain` ERR class, rename.

## W4core: Packet layer, replay state, ops 2/3/4/7/8/9

Files: packet.pruv, value.pruv, state.pruv, dispatch.pruv, bound.pruv, root.pruv, stubs sources.pruv / canon.pruv / accounts.pruv.
u16.pruv: only helpers added (`clamp`, `put_range`, `put_all`); `is_trim_unit` (M01) unchanged. All sources verify; `sh dev_build.sh` builds work/dev/harness.

### State layout (await packet section 2)
`[magic 1162761298, op, outstanding_request_code, aux, 8 checkpoint words, L, G, blob (repacked input, L words), log (G words)]`.
Log entry: `[n, rcode, aux, completion packet (kind 2, pool truncated)]`. entry_step validates prev, state (`state_ok`) and the completion
(`comp_check` against code/aux), appends the completion to the log and re-runs the item in direct style (per-item replay).
Observation helpers take the next log entry, otherwise stall the Ctx with the request to emit. Malformed state: InvalidPacket.
`pair_ok`: op 3 -> code 9 aux 0; op 4 -> code 9 aux 1; ops 8 and 9 -> code 8 aux 1..16777216.
Packet limit 16,777,216 words; larger input/log -> InputTooLarge. Session record: 30 words (id s+0, temporary s+3, engine s+6, externalThread s+9 tag 11, terminal s+21, engineThreads s+27).

### Ctx / observation API (state.pruv)
`ctx_new(prev-derived blob, log, ck)`, `obs(cx, code, aux, pool, rec, tab, ...)` (next log entry or stall), accessors
`accounts, account_at, ambient, homedir, cwd, realpath, stat` (inout cx, Result<(), Halt>), `ent_r/ent_p/ent_s` (completion record/pool/state),
`put_ck/get_ck` (8 checkpoint words), `put_done`, `stop_too_big`, `stop_fail`, by-value `fail` for entry_api (InvalidPacket), `finish(cx, await_seq, done_seq)`.
Done packet seq: begin 0 (done at seq 0 is accepted by the host), step = prev seq; await seq = last+1.

### Stubs (ops 1, 5, 6; all return boundary InternalInvariant, op echoed)
- sources.pruv: `pub fn import_begin(input: [i64]) -> [i64] ! alloc`, `pub fn import_step(prev: [i64], completion: [i64]) -> [i64] ! alloc`
- canon.pruv: `pub fn codex_folder_begin(input: [i64]) -> [i64] ! alloc`, `pub fn codex_folder_step(prev: [i64], completion: [i64]) -> [i64] ! alloc`
- accounts.pruv: `pub fn account_home_begin(input: [i64]) -> [i64] ! alloc`
root.pruv routes ops 1/6 begin+step and op 5 begin to them; step for ops 2, 5, 7 -> InvalidPacket (no awaits).

### Cases (work/core/cases.jsonl, generator work/core/gen.py, compare work/core/cmp.py)
304 cases: 187 fixture variants of ops 2,3,4,7,8,9 (groups B,D,F,P,R with `expect`) + 117 own (titles 199/200/201 and surrogate split at 200,
all rethrow branches incl. primitive/sync/AbortError object+Error, routes incl. unknown/empty/undefined/absent, find absent/NaN/+-0/undefined,
threads forks/terminal/slots, excluded all predicate kinds + throw, bound collisions/excluded writers/predicate skip+throw/key kinds, probes).
Results (harness vs expect, own cases vs oracle, own probes vs the DESIGN 5.4 rule):
- fixtures vs expect: 187 agree, 0 disagree
- own non-probe vs oracle: 82 agree, 1 disagree (below)
- own probes (version, length, tag, unit, op_id, seq, reentry, oversize -> boundary InvalidPacket / InputTooLarge; exact_limit equals oracle): 34 agree, 0 disagree
Oracle compare ignores `driver`/`callerTitles`.

### Remaining disagreements and deviations
1. X-th-slot-sur: engineThreads key with a lone surrogate given as `{"u16":..}` object key; oracle prints "[object Object]" for the key (its case decoding of u16 keys),
   Pruv/host give the units before the first colon ([55357]). Case-decoding bug in oracle.py, not in the Pruv side.
2. Known oracle bug (primitive thrown value wrapped as a cause prints raw) did not show in my cases; the primitive-throw cases agree.
3. host.rs vs fixtures: the engine trace renders ref arguments as `{"ref":"s1"}`, fixtures/oracle expect the bare name `"s1"`. My cmp.py
   normalises `{"ref":x}` to `x` in engine trace args; host.rs was not edited (it is read-only for me). Lead should decide which side to fix.
4. Probe `duplicate` is unreachable for my ops: the host only replays a completion when the stepped result is another await, and every op of mine makes
   at most one request. The three duplicate probes I wrote ended `ok` with driver InvalidPacket and were removed from the cases. P08 covers import_sources only.
5. Invalid inputs the host rejects as case errors (non-string row title) were dropped. Bound keys: refs and non-safe-integer Numbers as engine/thread give InvalidPacket
   (unsupported domain); more than 2,796,202 pairs or 16,000,000 key units give InputTooLarge.
6. Fixed during testing: boundary packet length word (was 10, now 8); bound index excluded-session handling was inverted.

### Not tested
Multi-entry log replay (log with more than one entry) is untested: each of my ops makes exactly one request, so every emitted await has an empty log.

### Mutant sites (one `// MUTANT Mxx:` comment directly above the site)
- M08 dispatch.pruv find_item: `if found < 0 { found = i; }` becomes `found = i` (last match wins).
- M09 dispatch.pruv route: unknown engine `return 4` becomes `return 1` (codex).
- M10 dispatch.pruv list_recipe: title cut backs off a trailing high surrogate (199 units when unit 199 is a high surrogate).
- M11 dispatch.pruv rethrow: AbortError passes through on `name === 'AbortError'` only; the mutant also requires the thrown value to be an Error.
- M12 dispatch.pruv rethrow: `has_detail` is true, so a non-Error detail is the stringified value "undefined".
- M13 bound.pruv ses_pairs: engine fallback uses `||` (falsy engines give codex) instead of `??`.
- M14 bound.pruv colon_at: prefix ends before the LAST colon instead of the first.
- M15 bound.pruv temp_true: truthiness instead of `=== true`.
- M16 bound.pruv index_item dedupe: a later write overwrites the stored session (last write wins).

## W4path: Path module

Module: `bench/l3/entry/path.pruv` (verifies with pruvc, no --no-verify). Tests: `work/path/` (`gen.mjs` makes `cases.txt`/`expect.txt` from Node v26.4.0, `main.rs` runner, `root.pruv` wrappers; rebuild: copy u16.pruv/path.pruv there, `pruvc root.pruv -o out.rs --manifest pruv.json`, `rustc -O main.rs -o runner`, `./runner < cases.txt | cmp - expect.txt`).

### Node v26.4.0 sources (dumped with `.toString()`)
```js
resolve(...args) {
    if (args.length === 0 || (args.length === 1 && (args[0] === '' || args[0] === '.'))) {
      const cwd = posixCwd();
      if (StringPrototypeCharCodeAt(cwd, 0) === CHAR_FORWARD_SLASH) {
        return cwd;
      }
    }
    let resolvedPath = '';
    let resolvedAbsolute = false;

    for (let i = args.length - 1; i >= 0 && !resolvedAbsolute; i--) {
      const path = args[i];
      validateString(path, `paths[${i}]`);

      // Skip empty entries
      if (path.length === 0) {
        continue;
      }

      resolvedPath = `${path}/${resolvedPath}`;
      resolvedAbsolute =
        StringPrototypeCharCodeAt(path, 0) === CHAR_FORWARD_SLASH;
    }

    if (!resolvedAbsolute) {
      const cwd = posixCwd();
      resolvedPath = `${cwd}/${resolvedPath}`;
      resolvedAbsolute =
        StringPrototypeCharCodeAt(cwd, 0) === CHAR_FORWARD_SLASH;
    }

    // At this point the path should be resolved to a full absolute path, but
    // handle relative paths to be safe (might happen when process.cwd() fails)

    // Normalize the path
    resolvedPath = normalizeString(resolvedPath, !resolvedAbsolute, '/',
                                   isPosixPathSeparator);

    if (resolvedAbsolute) {
      return `/${resolvedPath}`;
    }
    return resolvedPath.length > 0 ? resolvedPath : '.';
  }
normalize(path) {
    validateString(path, 'path');

    if (path.length === 0)
      return '.';

    const isAbsolute =
      StringPrototypeCharCodeAt(path, 0) === CHAR_FORWARD_SLASH;
    const trailingSeparator =
      StringPrototypeCharCodeAt(path, path.length - 1) === CHAR_FORWARD_SLASH;

    // Normalize the path
    path = normalizeString(path, !isAbsolute, '/', isPosixPathSeparator);

    if (path.length === 0) {
      if (isAbsolute)
        return '/';
      return trailingSeparator ? './' : '.';
    }
    if (trailingSeparator)
      path += '/';

    return isAbsolute ? `/${path}` : path;
  }
join(...args) {
    if (args.length === 0)
      return '.';

    const path = [];
    for (let i = 0; i < args.length; ++i) {
      const arg = args[i];
      validateString(arg, 'path');
      if (arg.length > 0) {
        path.push(arg);
      }
    }

    if (path.length === 0)
      return '.';

    return posix.normalize(ArrayPrototypeJoin(path, '/'));
  }
dirname(path) {
    validateString(path, 'path');
    if (path.length === 0)
      return '.';
    const hasRoot = StringPrototypeCharCodeAt(path, 0) === CHAR_FORWARD_SLASH;
    let end = -1;
    let matchedSlash = true;
    for (let i = path.length - 1; i >= 1; --i) {
      if (StringPrototypeCharCodeAt(path, i) === CHAR_FORWARD_SLASH) {
        if (!matchedSlash) {
          end = i;
          break;
        }
      } else {
        // We saw the first non-path separator
        matchedSlash = false;
      }
    }

    if (end === -1)
      return hasRoot ? '/' : '.';
    if (hasRoot && end === 1)
      return '//';
    return StringPrototypeSlice(path, 0, end);
  }
basename(path, suffix) {
    if (suffix !== undefined)
      validateString(suffix, 'suffix');
    validateString(path, 'path');

    let start = 0;
    let end = -1;
    let matchedSlash = true;

    if (suffix !== undefined && suffix.length > 0 && suffix.length <= path.length) {
      if (suffix === path)
        return '';
      let extIdx = suffix.length - 1;
      let firstNonSlashEnd = -1;
      for (let i = path.length - 1; i >= 0; --i) {
        const code = StringPrototypeCharCodeAt(path, i);
        if (code === CHAR_FORWARD_SLASH) {
          // If we reached a path separator that was not part of a set of path
          // separators at the end of the string, stop now
          if (!matchedSlash) {
            start = i + 1;
            break;
          }
        } else {
          if (firstNonSlashEnd === -1) {
            // We saw the first non-path separator, remember this index in case
            // we need it if the extension ends up not matching
            matchedSlash = false;
            firstNonSlashEnd = i + 1;
          }
          if (extIdx >= 0) {
            // Try to match the explicit extension
            if (code === StringPrototypeCharCodeAt(suffix, extIdx)) {
              if (--extIdx === -1) {
                // We matched the extension, so mark this as the end of our path
                // component
                end = i;
              }
            } else {
              // Extension does not match, so our result is the entire path
              // component
              extIdx = -1;
              end = firstNonSlashEnd;
            }
          }
        }
      }

      if (start === end)
        end = firstNonSlashEnd;
      else if (end === -1)
        end = path.length;
      return StringPrototypeSlice(path, start, end);
    }
    for (let i = path.length - 1; i >= 0; --i) {
      if (StringPrototypeCharCodeAt(path, i) === CHAR_FORWARD_SLASH) {
        // If we reached a path separator that was not part of a set of path
        // separators at the end of the string, stop now
        if (!matchedSlash) {
          start = i + 1;
          break;
        }
      } else if (end === -1) {
        // We saw the first non-path separator, mark this as the end of our
        // path component
        matchedSlash = false;
        end = i + 1;
      }
    }

    if (end === -1)
      return '';
    return StringPrototypeSlice(path, start, end);
  }
```

normalizeString (internal, reconstructed from lib/path.js; the port in path.pruv follows it line by line and is confirmed by the
normalize/join/resolve corpus, which exercises it for allowAboveRoot true and false):
```js
function normalizeString(path, allowAboveRoot, separator, isPathSeparator) {
  let res = ''; let lastSegmentLength = 0; let lastSlash = -1; let dots = 0; let code = 0;
  for (let i = 0; i <= path.length; ++i) {
    if (i < path.length) code = path.charCodeAt(i);
    else if (isPathSeparator(code)) break;
    else code = CHAR_FORWARD_SLASH;
    if (isPathSeparator(code)) {
      if (lastSlash === i - 1 || dots === 1) {
        // NOOP
      } else if (dots === 2) {
        if (res.length < 2 || lastSegmentLength !== 2 || res.charCodeAt(res.length - 1) !== CHAR_DOT ||
            res.charCodeAt(res.length - 2) !== CHAR_DOT) {
          if (res.length > 2) {
            const lastSlashIndex = res.lastIndexOf(separator);
            if (lastSlashIndex === -1) { res = ''; lastSegmentLength = 0; }
            else { res = res.slice(0, lastSlashIndex); lastSegmentLength = res.length - 1 - res.lastIndexOf(separator); }
            lastSlash = i; dots = 0; continue;
          } else if (res.length !== 0) {
            res = ''; lastSegmentLength = 0; lastSlash = i; dots = 0; continue;
          }
        }
        if (allowAboveRoot) { res += res.length > 0 ? `${separator}..` : '..'; lastSegmentLength = 2; }
      } else {
        if (res.length > 0) res += `${separator}${path.slice(lastSlash + 1, i)}`;
        else res = path.slice(lastSlash + 1, i);
        lastSegmentLength = i - lastSlash - 1;
      }
      lastSlash = i; dots = 0;
    } else if (code === CHAR_DOT && dots !== -1) { ++dots; } else { dots = -1; }
  }
  return res;
}
```
Port detail: `dots` saturates at 3 (only 1 and 2 are special, so behaviour is identical; needed for the overflow proof).

### cwd-call rules (probed with a patched process.cwd that counts calls and returns scripted values)
`posixCwd()` is `process.cwd()` called at call time. For `resolve(p)` with one string argument:
- p absolute (first unit '/'): 0 calls. Result `"/" + normalizeString(p + "/", false)`.
- p == "" or p == "." (fast path): call 1 (cwd1). If cwd1 starts with '/' (charCodeAt(0) === 47; "" does not) the result is cwd1 verbatim
  (not normalized, 1 call). Otherwise the general path calls cwd again (cwd2, 2 calls total); resolved = cwd2 + "/" + ("" for p "", "./" for p ".").
- any other relative p: exactly 1 call (cwd1); resolved = cwd1 + "/" + p + "/".
- General finish: used cwd (cwd2 on the fast path with 2 calls, else cwd1) starting with '/' -> result `"/" + normalizeString(resolved, false)`;
  else `normalizeString(resolved, true)` or "." when that is empty (relative/empty cwd, "can happen when process.cwd() fails").
- Never more than 2 calls. Corpus: 1512 cases with 0 calls, 1657 with 1, 111 with 2 (the call counts are compared too).

Differences from DESIGN §4.1: none in substance. DESIGN's "otherwise cwd once more" is refined: the extra (second) call happens only
on the fast path (p "" or ".") when cwd1 is not absolute; a non-fast relative p makes exactly one call, which is the "first" call.
The relative-cwd result of the fast path `resolve('.')` with cwd1="abc" is `normalizeString("<cwd2>/./", ...)`, not cwd1.

### API (all pure)
- `is_abs(p) -> bool`; `normalize_string(p, allow_above_root)`; `normalize(p)`; `join2/join3/join4`; `join_segs(base, segs)`; `dirname(p)`; `basename(p)` (no suffix).
- `resolve_needs_first_cwd(p) -> bool` == `!is_abs(p)`.
- `resolve_need(p, ncwd, cwd1, cwd2) -> bool`: true iff another cwd call is required after `ncwd` (0,1,2) calls
  (ncwd 0: !is_abs(p); ncwd 1: p is ""/"." and cwd1 not absolute; ncwd >= 2: false). Pass empty buffers for results not yet obtained.
- `resolve_finish(p, ncwd, cwd1, cwd2) -> [i64]`: result once `resolve_need` is false. Calling it with ncwd 0 on a relative p treats cwd as empty (do not do that).
- Driver loop: `n = 0; while resolve_need(p, n, c1, c2) { call cwd -> n==0 ? c1 : c2; n += 1 }; r = resolve_finish(p, n, c1, c2)`.

### Corpus result (work/path, Node v26.4.0 vs pruvc-compiled Rust, exact code-unit equality, 0 mismatches)
Random paths over segments {'', '.', '..', '...', 'a', 'bc', 'é', 0xD800, 0xDC00, surrogate pair, ' ', '.a', 'a.', '..a', 'abcde'} with 0-3 leading
slashes, 1-3 internal slashes, 0-2 trailing slashes, plus edge strings ('', '/', '.', '//', '/.', '..', '/..', './', '///'):
is_abs 3000, normalize 3000, join2 3000, join3 3000, join4 3000, dirname 3000, basename 3000, join_segs 3000,
resolve 3280 (3000 random with scripted cwd sequences + 280 exhaustive specials; results and cwd-call counts). Total 27280 lines, 0 mismatches.

## W4fix: Fixtures, runners, integration

Status: the Pruv core (`root.pruv`) is still a stub, so every fixture run ends `boundary_error InternalInvariant` ("core packet
version is not 1"). Only the plumbing is verified; no pair has passed a fixture yet.

### Files

- `gen_fixtures.py`: writes `fixtures/{S,E,A,C,H,F,D,T,R,U,V,X,B,P,I}.json`; `--check` derives again and compares without writing.
  Result: 150 fixtures, 283 runnable variants (273 outside the I group), 17 mutant-killers (each has a `mutant` tag).
- `fixtures/_defaults.json`: homedir `/home/test`, cwd `/work`, dataDir `/data`, ids `acc_0123456789abcdef` / `acc_fedcba9876543210`.
- `test_impl.py`: `--pairs`, `--only GROUP`, `--case ID`, `--reuse-verified`, `--list`, `--show`.
- `integrate.py`: I01-I10 (below). `oracle.py`, `common.py`: fixes below.

### Fixture derivation

Every non-I variant is run through the pinned-TS oracle (`oracle.py`); `expect` = `{terminal, value, trace, [callerTitles]}` from that
run. A per-case check (in `gen_fixtures.py`, run during derivation) then asserts the named behaviour, so a fixture that does not
show what it says fails generation (four such failures were fixed: C10/equal, H06, R06/unicode, P03/slot-key). The 17 mutant
killers (E09 S08 E02 A07 A08 C04 H03 F05 D06 T07 R04 R09 U09 V03 X06 X01 B03 -> M01..M16) each carry a check that the
behaviour differs under the named mutation (e.g. D06 unknown engine goes to grok, not codex; X01/X06 `??` vs `||`).
P01-P10 are packet-boundary probes: expect is the host-defined boundary terminal (`probe_expect`; corrupt probes at k keep
`trace[:k]`, `duplicate` has driver `InvalidPacket`, `reentry` the full trace); only `exact_limit` is compared with the oracle.
Subs expand to variants with ids like `D01/list`.

Interpreted spec cases: (1) probes are not oracle-comparable (the oracle has no packets) except exact_limit; (2) lone-surrogate
strings use `{"u16":[..]}`; (3) the title-cut tests treat the cut as a plain 200-unit cut with no surrogate back-off (M10);
(4) B10 also records `callerTitles` (verbatim copy of the caller loop in the oracle).

### Oracle fix

- Primitive thrown causes are reported as `{"ref":name}` (earlier fix).
- `oracle.py` splits results on `\n` only (`splitlines` also splits at U+2028/U+2029/U+0085, which broke a result line
  "Unterminated string").
- stdin JSON escapes U+2028, U+2029, U+0085: node `readline` splits on them, which produced 275 results for 273 cases. The
  same escaping is in `test_impl.run_runner`.

### integrate.py feasibility per route

Driver: the piece's own built `work/rs_rust/runner` (nothing rebuilt: all existing). No network, accounts or tokens (fake app-server,
fake ACP child, fake exporter, throwaway homes). One capture per case, converted to the entry's `forward` spec (rows -> refs
`r0..` with the row's own keys, record -> ref `rec`, error -> `reject` EngineImportError ref `e1`), then one entry run on rs/rust.

| route | cases | driver run | note |
|---|---|---|---|
| claude | I01 I02 | not runnable: integration limit | `claude` is the pure packet API (head/tail/window packet words); no list/read driver returning rows or a record. Its runner exists only for a0..a4, none for an entry-shaped result. Not stubbed, not built. |
| qwen | I03 I04 | `runner --list home /project` on `fixtures/normal.jsonl` in a temp home (mtime 1700000000); `runner <path>` -> `record` | The runner has no error result, so no error-forward run for qwen (limit for that sub-check). |
| codex | I05 I06 | codex common `request_for` + `run_lines`: list_basic, read_basic; errors list_error, read_meta_error | fake app-server (`q_server.py`) |
| grok | I07 I08 | `test_impl.run_pair`: listing-mixed-missing-3, budgets-200-turns-no-omit; errors paths-empty-folder, window-history-is-directory | fake ACP child |
| opencode | I09 I10 | `test_impl.run_pair`: F04-limit-49, F06-extra-slashes; errors F03-count-prepare-empty-error, F06-invalid-thread | fake exporter |

Per list case a second run extends the first captured row's title by 205 characters (synthetic, labelled `list+long-title`)
to exercise the 200-unit cut. Assertions: one trace entry `["engine",route,op,{source:s1,folder,[thread],options:o1}]`; list rows
`keep`/`copy_title` in order with the title cut to 200 units; read returns ref `rec`; error returns `throw_existing {ref:e1}`.
Exit code 0 = all asserted, 1 = failed assertion, 2 = only limits / entry core not ready. The `check` function was exercised on
synthetic results (pass and fail); the entry side cannot be asserted until the core exists.

### Commands run and results

- `python3 gen_fixtures.py` -> 150 fixtures, 283 variants, 17 mutant-killers; `--check` ok.
- `python3 test_impl.py --list` -> 273 variants.
- `python3 test_impl.py --pairs rs/rust --only S` (earlier also P): builds, links, runs; 12/12 InternalInvariant (stub core),
  reported as 12 unexplained differences, exit 1, as designed.
- `python3 integrate.py`: I01/I02 integration limits; I03-I10 captured (qwen, codex, grok, opencode drivers ran), entry assertions
  not run (stub core: InternalInvariant); exit 2 ("entry core not ready").
- Not run: py/rust, py/llvm, rs/llvm pairs, other groups (stub core gives nothing to compare), no full suites.

## W4imp: Ops 1/5/6 (sources, accounts, canon)

Exact ports of `backend-engine-import.ts`, `backend-accounts.ts`, `backend-codex-home.ts`. All three verify
(`sh dev_build.sh` gives `{"ok": true}`); the harness links.

### Design

Ops 1 and 6 use the per-item replay of state.pruv: an await state is `[magic, op, code, aux, 8 ck words, L, G, blob, log]`, the log
holds `[n, rcode, aux, completion]` entries, and `obs` consumes them in order or stalls with the next request. Items are re-run from
scratch each step.

* Op 1 (sources.pruv). The checkpoint is ck0 = phase (0 Claude home, 1 accounts(claude), 2 Claude account loop, 3 Codex home,
  4 accounts(codex), 5 Codex account loop, 6 Grok, 7 OpenCode, 8 Qwen, 9 done), ck1 = account index, ck2 = account count. One *item* is
  one phase step (one row, or one account). When the item completes, its row is appended to a step-local accumulator and a fresh item
  (empty log, new ck) continues in the same step (`drive`, at most 64 chained items, then the step returns an await/done). Rows are
  emitted once with the await/done packet and are never carried in the state.
  The Codex account body (`is_host_codex_home`: canonical x3, host home, two sameFile) is one item; its several observations live in the
  item log (up to ~20 entries) and are replayed per step. A done==1 throw in the body, or a host-home match, skips the account.
* Op 6 (canon.pruv): a single item (`folder_item`) whose log is replayed; no chaining.
* Op 5 (accounts.pruv): never awaits; validates and returns `[0, home]` or the wire error `[3, 0]`.
* Own input check `io_check` (ops 1, 5, 6) and own state check `state_ok_ie` (op 1 codes 1..7, op 6 codes 3..7, aux 0) replace
  `input_check`/`state_ok`, which cover only ops 2,3,4,7,8,9. root.pruv already routes ops 1 and 6.
* Never cached: homedir, cwd, ambient, realpath and stat are asked every time the TS calls them (`host_home`, `home_sub`,
  `same_file`, `is_host_codex_home` have no memo).
* A boundary InvalidPacket is returned for bad pairs, malformed state, `seq` out of range, or a chain whose item did not consume
  exactly its log (`cx.pos != len(lg)`).

### Per-step cost

A step decodes carried input (dataDir, the four env strings, ck words), the current item's log (bounded by the one account body or one
canonical climb, O(path length x segments) entries), and the request. It never touches earlier rows: those were emitted by earlier
steps and are not in the state. Chaining adds O(rows produced this step) for the accumulator, with a 64-item cap. So the per-step
cost is O(carried input + current item log + request), and total cost is linear in the number of items times that bound, not O(A^2)
in emitted rows.

### Mutant sites (`// MUTANT Mxx` directly above the site)

* M02 sources.pruv `ph_codex_home`: reads input env slot CODEX_HOME instead of the ambient request.
* M03 sources.pruv `ph_opencode`: the starts-with '/' check is dropped.
* M04 sources.pruv, both emit sites (Claude and Codex account loops): skip a row whose id equals an earlier emitted id.
* M05 sources.pruv, both invalid-account conditions: an invalid or nullish account throws instead of being skipped.
* M06 canon.pruv `canonical`: ENOENT also issues stat.
* M07 canon.pruv `is_host_codex_home`: the `dot` comparison and the second sameFile are omitted (two markers).

### Changes outside the three files

* `root.pruv`: added `mod path;` after `mod accounts;` (path.pruv is imported by my modules; root had no `mod path`). Nothing else.
  No edits to packet, state, dispatch, bound, path, host.rs, oracle, fixtures, DESIGN.md, reference/.
* `canon.pruv::with_pool` is `pub` (used by sources.pruv).

### Cases run (work/imp/)

`gen.py <seed> <n>`, `gen2.py <seed> <n>` write cases; `cmp.py <cases.jsonl>` runs `work/dev/harness` and `oracle.py` and compares
terminal, value and trace with `common.compare` (refs by name). The generators cover: 0-12 accounts per engine (valid, invalid ids,
duplicates, null/primitive rows, id/name of other types, lone-surrogate name), env slots absent/undefined/empty/whitespace (U+FEFF,
NBSP, U+0085, U+200B, tab, newline, U+2028, U+3000), relative and absolute XDG, ambient CODEX_HOME (undefined, empty, relative, host
home, account home, repeated different values) versus the input env slot, canonical paths with 0-5 missing components, realpath
ENOENT/EACCES/ELOOP/ENOTDIR/EIO, stat ok (equal and unequal dev/ino)/fail, homedir and cwd throws and repeated different values
inside and outside the Codex body, relative/odd dataDir (`rel/data`, `/`, empty, `//`), account_home with bad ids/engines,
codex_account_folder with wrong homes and invalid ids. `gen2.py` stresses the multi-entry log replay (up to 12 Codex accounts, dataDir
depth 0-5, ambient depth 0-5, 3-entry queues).

Results: seed 7 x 400 = 400 exact; seeds 11..16 x 1000 = 6000 exact; dense (gen2) 600 exact. Total 7000 cases, 0 disagreements.
Oracle terminals seen: ok, throw_existing, throw_account_wire, throw_plain_error.
Oracle limitation found while generating: `reference-primitives.mjs` has no `throw` entries for realpath/stat (only ok/errno), so the
generator uses errno entries there (`throw` stays in homedir/cwd/accounts).

Fixture groups (`test_impl.py --pairs rs/rust --reuse-verified --only G`): S 12/12, E 17/17, A 23/23, C 13/13, H 10/10, P 35/37.

### Remaining disagreements (2, explained, not fixable inside the three files)

`P06/unit-step` (probe at=3) and `P08/duplicate-3` (probe at=3) expect trace `[homedir, accounts claude, ambient CODEX_HOME]`, we give
`[homedir, accounts claude]`. Cause: `gen_fixtures.probe_expect` takes `trace[:k]` of the oracle trace, but host `calls` counts every
await including `account_at`, which is untraced (DESIGN 4.4 and "account_at is not traced"). With the fixture's one Claude account
the 3rd await is account_at, so the ambient await is produced but never served (the probe replaces it). Passing would need the ambient
request to precede account_at, contradicting DESIGN 4.4 phase order. I did not reorder. P06/unit-step and P08/duplicate-3 otherwise
match (terminal boundary_error InvalidPacket, driver). P08/duplicate-1 passes.

## W5: Q generator and mutant runner

Design: see Q_README.md and the q_mutants.py docstring. Mutants: sites found by exact old text (each must occur once, markers
`// MUTANT Mxx:` sit at the sites), built in work/mut/Mxx with rs/rust, killers from MUTANT_KILLERS via the `mutant` key
of the loaded cases. Killed = differs from expect and not a crash or InternalInvariant.

### Commands run (rs/rust only)
- `q.py --pairs rs/rust --shrink-max 3` (seed 20261118, 2000 cases): 1994 exact, 4 excluded-domain (G7 excluded_ok), 2 unexplained.
  G1 400, G3 200, G4 300, G5 300, G6 200, G8 100 all exact; G2 298 exact + 2 unexplained.
- `q_mutants.py --jobs 3`: M01, M03, M06-M16 killed; M02, M04, M05 TODO entries (site-missing). 0 verification-rejected.
  M03, M06, M07 edits were added by me although the brief listed M02-M07 as TODO; M02/M04/M05 remain for the lead.

### Interpretations
- Probe positions are clamped as in Q_README.md; two early G8 failures (Q1042, Q1736) were generator mistakes, fixed.
- Mutants killed by variant fixtures: a mutant counts as killed when any killing variant differs.

### Unexplained failures (not fixed, .pruv is not mine)
Both are G2 codex_account_folder (op 6), trace differs, terminal and value equal. Both shrink to an `errno: ""` realpath step:
- Q0087 (trace[11]): oracle `realpath /home/test/.codex, stat /home/test/.codex, realpath /home/test`; Pruv skips the stat
  (19 vs 20 steps). Shrunk: realpath("/home/test/.codex") = [{errno:"", ref:rpe}, {errno:"ENOENT", ref:rpe}].
- Q1826 (trace[9]): oracle `realpath /, homedir, realpath ~/.codex`; Pruv issues an extra `stat /` before homedir (20 vs 18).
  Shrunk: realpath("/") = [{errno:"", ref:rpe}].
Likely cause: an empty-string errno is treated as ENOENT-like or not (canon.pruv, the `"ENOENT"` comparison near M06); not confirmed.
Files: work/q/fail/87.json, work/q/fail/1826.json.

## Audit fixes (R1-R10)

The audit of the first complete version raised ten REQUIRED findings. They were fixed in three disjoint file sets
(WA: replay and climb, WB: two quadratic scans, WC: host, oracle and comparison policy); the lead fixed R10 and the
remaining `__proto__` sites in `reference.mjs`. The fixers' notes follow.

- R10 (lead): M04 is now a real dedupe mutant. It removes the `ck6 == 0` check from `state.pruv` `canon_ck_ok` and makes
  the Codex account phase keep a hash of the last emitted Codex id in ck6 and skip a row whose id hash equals it. A07
  (adjacent Codex duplicates `x`, `y` with the same id) kills it (`value[6].source`).
- R6 (lead, the rest of WC's fix): `reference.mjs` `record()` and `sourceOf()` now use `put()` (own data properties) too.
- M14's anchor in `q_mutants.py` moved to the comment that now follows the function (WB added `ses_pairs`'s offset table).

### WA
#### R1 replay validation
- state.pruv `obs`: when the last log entry is consumed, the regenerated request (code, args record, pool, tables) is
  compared with the request recorded in prev (`with_prev`, `req_same`). Mismatch is a protocol error (R1a).
- `log_cont`: every entry's op equals the item op; entry seqs strictly increase, start above 0 and stay below the
  current seq; for ops other than 1 the entry count must equal seq-1-(dropped) exactly (R1b). Op 1 drops log at item
  boundaries, so there it is a range check.
- `finish`: the log must be fully consumed when replay ends (R1c).
- `ck_ok` / `st_replay_ok` (called from root, canon, sources steps): phase/idx/count consistency per op (R1d).
- `with_prev` is added to the ctx in all three step entry points; public signatures are unchanged.

#### R3 canonical climb cost
`canonical()` stored no climb progress, so each resumed step restarted the climb (O(N^2)). Each canonical call now
records its climb depth in checkpoint words and, on resume, rebuilds the climb state directly at that depth, and drops
the log entries of the finished levels (`drop_level`).

Checkpoint word allocation (ck words in the await state):
- ck0 phase, ck1 idx, ck2 cnt (existing)
- ck3, ck4, ck5 climb depth of the 0th, 1st, 2nd `canonical` call within the item
- ck6 reserved, must be 0
- ck7 total dropped log entries (consistency check with the log; cleared by `adv` in sources.pruv)

#### New tests
- forge.rs, forge_build.sh: harness that tampers with prev of one entry_step (host-level, the oracle cannot express it).
- forge_test.py: 3 honest controls, 18 forgeries (R1a record/code, R1b reorder/drop/dup/foreign op/unissued seq/
  appended entries, R1c extra entry, R1d bad phase/idx/ck6/climb word/drop count), all must end in boundary_error
  InvalidPacket. Deep-path rows for 10..160 missing segments.
- Deep path result (requests = n + 16, constant 8 entries per step, state grows only by the path itself):
  n=10: 26, n=20: 36, n=40: 56, n=80: 96, n=160: 176.
- Note: R1c cannot be isolated for op 6 because the exact count check of R1b already rejects the same forgeries;
  it is a second line of defence. Op 9 fixtures finish in one step in the harness, so forgeries use op 6.

#### Final numbers
- `python3 test_impl.py --pairs rs/rust`: 273 variants x 1 pairs, 0 unexplained.
- `python3 q.py --pairs rs/rust --count 300 --reuse-verified`: 300 cases, 0 failures.
- `python3 forge_test.py`: 21 scenarios ok, 0 failures.
- Mutant marker comments M02..M07: no marker text changed.

### R1/R3 follow-up (second audit)
- R1 (op 1 gap forgery): op 1 packs the item start seq s0 into ck0 (`ck0 = phase + 16 * s0`, s0 < 2^28; ck6 stays reserved).
  `sources.pruv` `drive` tracks s0 (begin: 1; an item that completes in a step makes the next item start at the seq the step emits)
  and packs it when it encodes the await; `import_step` unpacks it (`st_s0`, `set_ck_arr0`). `state.pruv` `log_cont` now checks
  for every op the exact continuity `s0 + dropped (ck7) + kept entries == seq` with kept seqs strictly increasing, >= s0, < seq
  (non-op-1 items have s0 = 1). `ck_ok` reads the phase as ck0 % 16. forge_test.py: 4 new op 1 scenarios (2 controls, entry seq
  below the item start = the reported forgery, seq above the current request, bogus drop count, changed s0).
- R3 (relative paths): `canon.pruv` `canonical` also checkpoints a normalized relative resolved path (a relative cwd). Levels
  1..k are cut at the k-th slash from the end, level k+1 is "." (its parent is itself, then the plain fallback). Relative climb at
  depth 10/20/40 now: max log entries 10/10/10 (was 24/34/54), max state words 700/820/1060 (was 2301/3971/8511); forge_test.py
  asserts entries <= 12 and state within 100 words of the absolute run.

### R8 follow-up (second audit, lead)

`common.forbidden_reached(case)` replays excludedFromBound over the excluded_ok case (temporary === true, else the next scripted isTemporary answer, default false; a throw or a non-boolean answer ends with False). q.py passes it as `excluded`, so excluded-domain needs the forbidden session to be really stringified by the oracle. `judge_test.py` holds 8 rows (5 unexplained, 3 excluded-domain). Q seed 20261118 x 2000: still 4 excluded-domain, 0 unexplained.

### WB
Only `bound.pruv` and `packet.pruv` changed (plus `perf_wb.py`, this file). All pub signatures are unchanged
(`new_meta`, `session_check`, `sec_start`, ... keep their types). `dev_build.sh` verifies everything with pruvc (`{"ok": true}`).

#### R2: duplicate-key check in `index_item` (bound.pruv)

Before: for each of the n entries, a scan over all kept keys (`g < m`) with a length compare and `pool_eq`: O(n^2) key compares.
After: open-addressing hash index `tbl` (size 2n+1, slot = position in `keep`, -1 = empty), built while entries are visited in order.
The hash is a polynomial over the key units (units clamped to 0..2^32, modulus 2147483629, so no overflow); a hash hit is
confirmed by the same length compare plus `pool_eq`, so equality is still exact. The first occurrence is inserted, a later
equal key hits it and is dropped, `keep`/`val` are written in the same order: first-write-wins and output order are unchanged.
Expected cost O(sum of key lengths + n). The probe loop is bounded by `tsz` steps; the table is never full (m < tsz), so the
`!done` branch is unreachable and only keeps the loop total for the verifier. The `MUTANT M16` marker line is untouched.

#### R4: `sec_start` rescan (packet.pruv, plus the same pattern in bound.pruv)

Before: `sec_start(w, i)` sums section lengths 0..i, O(nsec) per call; `session_check` (via `rows1_check`, `ets_check`) and the
op 9 / rcode 9 table loops called it per session/table, so O(sessions x nsec).
After: `new_meta` now also builds the offset table once (O(nsec)); `meta` layout is `[0]` pool high-water, `[1+s]` claimed,
`[nsec+1+j]` offs[j] for j in 0..nsec, `[len-1]` bad flag (length 2*nsec+3; was nsec+3, the bad flag is still last).
`sec_start_m(w, meta, i)` reads it with the same clamps as `sec_start` (index clamped to 0..nsec, result to 2^25), so values
are identical for every input, including malformed packets (the table is computed from the same unclamped sum, clamp applied
at the end like `sec_start`). `rows1_check`, `ets_check`, op 9 `session_check` loop and the rcode 9 loop use it.
Single-use `sec_start(w, 1)`/`(w, 0)` calls stay (once per packet).
The same quadratic existed in `bound.pruv` `ses_pairs` (called per session in both passes of `index_item`), so it now takes the
offset table (`om = new_meta(w)`, built once in `index_item` / `threads_item`). Behaviour is identical.

#### Measurements (`python3 perf_wb.py ...`, old = original files built to `work/harness_old`, new = `work/dev/harness`)

Input: `bound_session_index` with S sessions, every key duplicated once (first write wins), outputs compared byte for byte
(identical in every run). "tables" = each session also has a forks table and an engineThreads table (2S+2 sections).

| input | S | old | new |
|---|---|---|---|
| keys only (R2) | 5000 | 0.05s | 0.03s |
| keys only | 10000 | 0.14s | 0.03s |
| keys only | 20000 | 0.50s | 0.05s |
| tables (R2+R4) | 2000 | 0.04-0.09s | 0.03s |
| tables | 5000 | 0.18-0.22s | 0.03-0.07s |
| tables | 10000 | 0.71-0.74s | 0.06s |
| tables | 20000 | 2.6s | 0.13s |

Old time grows about x4 per doubling (quadratic); new time grows about x2 (linear). Timings are wall clock including harness
start and JSON encode/decode (the largest input is 1.2 MB of JSON); an intermediate build with only R2+packet.pruv changed still
showed quadratic growth (0.07, 0.23, 0.64s for 5k/10k/20k), which exposed the `ses_pairs` copy of R4 fixed above.

#### Tests

`python3 test_impl.py --pairs rs/rust`: 273 variants x 1 pair = 273 runs; unexplained differences: 0 (unchanged).
`q_mutants.py` was not run; its M13/M14/M15/M16 patch sites (text in bound.pruv) are unchanged.

### WC
Files changed: host.rs, reference-primitives.mjs, common.py, q.py, gen_fixtures.py, DESIGN.md, fixtures/{S,C,H,V}.json, this file.
No .pruv file was touched. reference.mjs (not in the allowed list) was not touched; see "Open point".

#### R5 stat dev/ino leading zeros
Decision: normalize in both (not reject). The spec says the host does boundary conversion to lossless renderings and never
compares them, and the oracle already compared BigInt values, so canonical decimal is the existing oracle behavior.
- host.rs `dec_text`: digit-only text (string or non-negative integer), leading zeros stripped, "0" kept; empty, signs,
  spaces, radix prefixes, floats are a case error.
- reference-primitives.mjs `decimalOf`: same domain (`/^[0-9]+$/` or safe non-negative integer), BigInt rendering; others
  throw OracleError (BigInt("") = 0n, BigInt("0x10") = 16n and BigInt(" 1 ") = 1n used to be accepted silently).
- DESIGN.md script table: sentence on canonical dev/ino.
- New fixture H11 (6 variants): dev padded, ino padded, both padded, "00"/"000" vs "0"/"0", integer vs padded string
  (all give the same-file wire rejection), and a still-different pair (ok, four stats).

#### R6 `__proto__` key
- reference-primitives.mjs: new exported `put(target, key, value)` (Object.defineProperty, own enumerable data
  property). Used by `dec` for `{"obj"}` entries and for record objects, and by `rowsOf` for fake-engine rows.
- DESIGN.md (JV table, `{"obj":...}` row): own data properties like JSON.parse, `__proto__` is an ordinary key.
- New fixture V11 (4 variants, session_threads): slot name `__proto__` (two shapes), a record with a `__proto__` key
  that must not be inherited as `threadId` (expect []), and the same beside an own `threadId` (expect the own one).
  Both oracle and host agree; with the old assignment oracle the first two lose the key and the third inherits.
- Open point: reference.mjs `record()` and `sourceOf()` (env and source extra fields) still assign with `out[key] = v`.
  Not in my file list; the lead should switch them to `put` (exported from reference-primitives.mjs) if `__proto__` keys
  can occur there. No fixture reaches them.

#### R7 u16 success values for homedir, cwd, realpath
- reference-primitives.mjs `scriptedString`: `dec()` of the scripted success value, must be a string. Used by homedir,
  process.cwd and realpathSync. DESIGN.md script table now says `{"ok":JV string}`.
- New fixtures: S11 (homedir: valid pair as units, lone high, lone low), C11 (cwd: pair, lone high; the relative home is
  resolved against the decoded cwd and the realpath request carries it), C12 (realpath: pair-equal, lone-equal, and a lone-differs
  pair that is accepted).

#### R8 strict P6 / excluded-domain (common.compare)
- P6 only if expected and actual are both `boundary_error` with code `InputTooLarge` and have equal traces.
- excluded-domain only if the oracle completed (`terminal` ok, so the bound-key stringification of the ref was reached,
  after the whole isTemporary batch), the actual side is the `InvalidPacket` boundary error, and the traces are equal.
- q.py passes the existing flags; the strictness lives in common.compare so test_impl uses it too.

#### R9 shared probe derivation
- Moved `probe_expect` (plus `_probe_slots`, `probe_derivable`, `UNTRACED`) from gen_fixtures.py to common.py; gen_fixtures
  imports it, q.py `judge` calls it (replacing its own trace[:at] expectations).
- q.py `probe_clamp` now also lowers `at` until `probe_derivable` holds (an underivable probe inside the codex account phase
  would otherwise raise); a duplicate probe that ends below 1 becomes `version`, as before.

#### Numbers
- gen_fixtures.py: 155 fixtures (150 + S11, C11, C12, H11, V11), 301 runnable variants (was 283), `--check` ok.
  Count assertion updated 150 -> 155. Regeneration diff: pure additions to S, C, H, V (no existing entry changed;
  verified by comparing every old entry to the new file).
- test_impl.py --pairs rs/rust: 291 variants x 1 pair, 0 unexplained.
- q.py seed 20261118, 2000 cases, rs/rust: exact 1996, excluded-domain 4 (G7), P5 0, P6 0, unexplained 0 (same as baseline).
- integrate.py: 10 integration cases pass. q_mutants.py: 16 killed.

