# G4b: Packages, Versions, Proof Reuse, and Trust Reports

Status: design (2026-11-16). Follows G4b-0 (`docs/G4B_LIB_SPEC.md`, done, RESULTS.md section 131, commit 570a526).
Owns the work that `docs/G4_SPEC.md` section 15 defers to G4b. No implementation exists yet.

## 1. Goal and foundation

G4b-0 lets several programs share one copy of a module file, but the shared files are still mounted into the
consumer's namespace: a library module refers to its siblings as `crate::OTHER`, so every consumer must declare the
library's own dependencies (`lines` needs `json`, `lex`, `parse`), and every consumer re-proves every shared definition
on every compile (G8 section 5 rule 1 applies the same policy to std).

G4b fixes four things, in this order:

1. **Package identity and namespaces.** A package is a directory with a manifest. Inside a package, `crate` means that
   package's root. A consumer mounts a dependency at a name of its choice and never declares the dependency's own
   dependencies. One package is loaded, proved and emitted once per program even when several packages depend on it.
2. **Versions.** One version per package name per program; requirements are checked, not solved.
3. **Trust report.** Every compile can list every place where something is accepted without proof, per package.
4. **Proof reuse.** An opt-in local cache skips solver calls whose exact input was already proved.

Separate code generation is not a goal: output stays one flat Rust file per program (G4 section 6), monomorphization
stays whole-program.

### 1.1 Repository facts

Facts read from the tree for this design (Python paths under `pruvc/`; `CORE`, `PROOF` = `pruvc-rs/crates/pruv-core/src`,
`pruvc-rs/crates/pruv-proof/src`).

Trust points that exist today (there is no `assume`, `axiom` or `admit` syntax):

- `trusted rust fn NAME(...) ... = "::path";` (`vparser.py:259`, `:707`; `CORE/parser.rs:248`, `:1028`). Its `ensures`
  is assumed at call sites and only domain-checked, never proved (`verify.py:2855`; `PROOF/symbolic.rs:4038`).
- Built-in catalog operations: collections (`Vec`, `Map`, `String`), `File`, `Process`, `Net`, `Loop`. Their contracts
  are models written in the compiler and trusted (`foreign.py:56`-`129`; `CORE/foreign.rs:239`-`544`).
- Builtins (`len`, `alloc`, `copy`, `print`, `substr`, ...) are modeled directly by the verifier; they are part of the
  compiler, like the catalog, but have no per-use record.
- Flags: `--no-verify` (no proof at all), `--trust-callers` (host entries call bodies without checking `requires`;
  recorded nowhere today), `--foreign-checks requires|all|off` (runtime checks around trusted functions; recorded as
  `check_mode`), `--rust-extern` and `--rust-search` (outside Rust code linked into the build).

Existing provenance output: the foreign manifest `<out>.foreign.json`, schema `pruv-foreign-v1`, written only for Rust
source output (`foreign_codegen.py:43`; `CORE/foreign_codegen.rs:82`). Fields: `backend`, `check_mode`,
`rustc_checked`, `trusted[]` (`name`, `path`, `line`, `col`, `effects`, `requires`, `ensures`, `reachable`),
`catalog[]` (`op`, `type_args`), `externs[]` (`name`, `path`). It has no package, version, content hash, solver or
flag fields other than `check_mode`. The proof trace `pruv-trace-v2` records the SMT2 text of every attempt
(`verify.py:120`; `PROOF/obligation.rs:422`). There is no general JSON report.

Verification unit and solver call:

- Every definition is verified on every compile. Callers see callee contracts, never bodies. Inputs to one obligation
  are the path condition, guards, optionally the aggregate lemmas, and the negated obligation; it is first tried
  without lemmas and retried with them (`verify.py:1128`; `PROOF/obligation.rs:303`, `:392`). Type declarations,
  refinement expansion, `--infer` candidates, G9 measures and generic substitution all reach the solver only through
  that query.
- Each attempt builds a fresh solver and calls check exactly once: Python `Model.prove` inner `run`
  (`sol.check()`, `verify.py:1164`), Rust `run_attempt` (`sol.check()`, `PROOF/obligation.rs:419`, wrapping `Z3_solver_check` at
  `PROOF/sys.rs:862`). Inference uses the same path through `_probe_attempts`.
- Python shares one z3 context across the program; Rust builds a fresh Z3 context per function (`PROOF/lib.rs:78`,
  `:119`).
- The SMT2 text is rendered only when tracing (`sol.sexpr()` / `Solver::to_smt2()`, prefixed by the
  `set-option` lines for timeout, rlimit and seed).
- Options per attempt: timeout 8000 ms (`verify.py:11`, `:61`); inference uses rlimit 100000, timeout 500 ms, seed 0,
  at most 64 candidates, 512 attempts and 5 s wall time. No user flag sets any of them.
- Solver: Python `z3-solver==5.3.3.0` (pinned in CI); Rust the vendored libz3 (`z3 =0.22.0`, `z3-sys =0.14.1`,
  `z3-src =503.3.0`). Both expose the full version (`z3.get_full_version()`; `Z3_get_full_version`, `PROOF/sys.rs:149`).
- No proof cache, fingerprint or reuse exists.

Names: the G4 symbol allocator emits `pruv_m_f_` / `pruv_m_t_` plus a count and length-prefixed path components
(`modules.py:32`, `:39`; `CORE/modules.rs:48`, `:57`). G4b-0 library files display as `@lib/NAME.pruv` and mount at
`crate::NAME`; the physical directory is not part of identity (`modules.py:144`; `CORE/modules.rs:222`).

Cost: importing std adds about 170 ms (Python) and 118 ms (Rust) per compile, for parse, check, prove and emission of
its 12 bodies (RESULTS.md line 1262, macOS arm64 median). A whole L3 root costs seconds (frozen Claude A4, Rust: 16.5 s
for the full compile, `bench/l3/claude/work/qmeasure/measure.json`). The proof cost of each shared `bench/l3/lib`
module has never been measured separately.

Transitive declarations today: every L3 root lists the shared modules it needs only through others, e.g. `json`
`a5_root.pruv:2` declares `json, lex, parse, lines` because `lines.pruv` calls `crate::lex::validate_utf8` and
`crate::parse::scan`; the MCP and Qwen roots declare all six or seven shared modules. The loader follows explicit `mod`
lists only (`modules.py:138`; `CORE/modules.rs:193`).

Further facts:

- `bench/l3/lib/` holds seven modules. Their `crate::` references form this graph: `json`, `lex`, `tail` use no other
  module; `jstr` uses `json`; `parse` uses `json`, `lex`; `lines` uses `json`, `lex`, `parse`; `access` uses `json`,
  `jstr`. `jstr.pruv:2` declares `trusted rust fn utf8_to_string`, so a trust point already lives in shared code.
- No lib module uses `crate::std`.
- The Python compiler runs on Python 3.9 (no `tomllib`). The Rust workspace already depends on `serde_json` and has no
  TOML or SHA-256 crate.

### 1.2 Non-goals

No registry, no network access from the compiler, no Git or archive sources, no publishing, no workspace feature, no
features/conditional compilation, no re-exports or `use`, no several versions of one package in a program, no
pre-verified artifacts from other machines, no proof hints, no contract-aware SemVer checker, no build of separate
object files per package.

## 2. Decisions on the ten options

The options are the table at the end of `docs/notes/g4b_package_research.md`.

| # | Option | Decision | Reason |
|---|---|---|---|
| 1 | Separate package identity from import name | Adopt | The identity is the `name` in the package's own manifest; the mount name is the dependency key in the consumer's manifest. Without the split, two consumers that mount the same package under different names would get two copies, and a rename would change symbols. Diagnostics show the identity, so the extra complexity is one rule (section 4.3). |
| 2 | Major version in the path (Go `/v2`) | Reject | It exists to let two majors coexist (option 4), which G4b rejects. With one version per name the major is just a field. |
| 3 | Minimal version selection | Adopt the rule, implement only the check | With path sources each package has exactly one available version, so selection degenerates to "the found version satisfies every requirement". The MVS rule (pick the highest minimum, never the newest) is fixed now so that a later source kind with several versions cannot change the result of an existing graph (section 5). |
| 4 | Incompatible versions coexist (Cargo) | Reject | Two versions of one package give two nominal types with the same source name and two proofs of the same code. Proof facts about one version's `Value` would not apply to the other's, and the research notes show users already struggle with this in Cargo. The cost of rejecting is a resolution error, which is visible and fixable. |
| 5 | Pin transitive revisions in a lock file | Hold | Path sources have no revision to pin; a lock would only repeat what the manifests already determine. The lock format is designed together with the first source kind that has revisions (section 9). Provenance for proofs is covered by content hashes in the trust report (section 6). |
| 6 | Pre-verified bundle matched on tool and options (Dafny `.doo`) | Reject | It moves trust to whoever built the bundle. Pruv's rule since G4 section 5 and G8 section 5 is that no compile succeeds on a definition that was not proved. A local cache (option 7) gets most of the saving without importing foreign claims. |
| 7 | Per-module dependency hash and proof cache | Adopt, as a content-addressed solver cache | Key on the exact bytes sent to the solver plus solver identity and limits, not on a dependency graph of hashes. Exact invalidation then needs no analysis: a changed callee contract changes the query text. Only proved (`unsat`) answers are stored, so a stale or corrupt entry can at worst repeat an earlier proof of the identical query. Opt-in and reported as a trust point (section 7). |
| 8 | Proof hint replay (F* `.hints`) | Hold | No measured unstable proof in the repository motivates it, hints need upkeep, and the solver still runs. Revisit when a Q bundle records a proof that flips between runs. |
| 9 | Explicit report of trust assumptions | Adopt | Packages make it normal to compile code one did not write; the user must be able to see `trusted rust fn`, foreign declarations, unchecked flags and cache use without reading every file (section 6). |
| 10 | Library set aligned to tool releases (Isabelle AFP) | Hold, std only | std is already bundled with the compiler (G8). There is no second curated library to align. |

## 3. Manifest

A package is a directory containing `pruv.json`. JSON is chosen because both compilers already parse it
(Python `json`, Rust `serde_json`); TOML would need a new parser on Python 3.9 and a new crate.

```json
{
  "name": "lines",
  "version": "0.1.0",
  "root": "lines.pruv",
  "dependencies": {
    "json": { "path": "../json", "version": "0.1" },
    "lex": { "path": "../lex", "version": "0.1" },
    "parse": { "path": "../parse", "version": "0.1" }
  }
}
```

Rules (both compilers, same diagnostics):

1. UTF-8 JSON object. Keys `name`, `version`, `root` are required; `dependencies` is optional (missing = `{}`). Any other
   key, a duplicate key, or a wrong type is `pkg/manifest_invalid` naming the key.
2. `name`: `[a-z][a-z0-9_]*`, no `__`, not starting with `pruv_`, not `std`, not `crate`, not a reserved name.
3. `version`: `MAJOR.MINOR.PATCH`, each a decimal without leading zeros, each at most 2^31 - 1. No pre-release or
   build suffix.
4. `root`: a relative path with `/` separators, no `..` segment, ending in `.pruv`, naming an existing file inside the
   package directory. Child modules of the root follow G4 section 2 relative to the root's directory.
5. `dependencies`: object from mount name to entry. Mount names follow module-name rules (G4 section 2) and are not
   `std`. An entry has exactly `path` (relative, `/` separators, `..` allowed, names a directory containing
   `pruv.json`) and `version` (a requirement, section 5).
6. The manifest file is never written by the compiler.

CLI: `--manifest PATH` names the program's own manifest. The positional input must be the same file as that manifest's
`root` (canonical file identity), otherwise a usage error `--manifest root does not match the input file`. `--manifest`
and `--lib` together are a usage error. Without `--manifest` nothing in this document applies and every current output
is unchanged.

## 4. Packages in the program

### 4.1 Loading

1. Load the program manifest, then resolve dependencies depth-first in key order. Each dependency directory is
   canonicalized; its manifest is read once per canonical directory.
2. The package graph must be acyclic: `pkg/dependency_cycle`, listing the names on the cycle in order.
3. In any package root file (the program root is the root of the program's own package), `mod NAME;` resolves as:
   - `NAME` is a key of that package's `dependencies` and no local `NAME.pruv` exists: mount that package.
   - local file exists and not a dependency key: the ordinary G4 local module.
   - both: `pkg/mount_conflict` (same message shape as `parse/lib_module_conflict`).
   - neither: the existing `parse/module_file` error; the hint also says `or add NAME to dependencies`.
   `mod NAME;` in a non-root module never resolves to a dependency.
4. Every dependency key must be mounted by exactly one `mod` in that package's root: an unmounted key is
   `pkg/unused_dependency`. This keeps "declared" equal to "loaded and proved".
5. `mod std;` keeps its G8 meaning only in the program root. In a dependency's root it is `pkg/std_in_dependency`
   (deferred, section 9).

### 4.2 Names

1. Inside a package, `crate` denotes that package's root. A dependency mounted as `mod json;` is reached as
   `crate::json::item` from any module of the mounting package, exactly as a local child module would be.
2. A package sees only its own modules and the packages it mounts. It never sees its consumer, its siblings or its
   dependencies' dependencies. There is no re-export; a consumer that calls `json` directly mounts `json` itself.
3. G4 visibility applies at package boundaries unchanged: only `pub` items of a mounted package's modules are usable.
4. Host entries (`--rust-bindings`, native entry, checked host wrappers) are exactly the program root's, as in G4.
   A dependency root's `pub fn` is an ordinary public Pruv function.

### 4.3 Identity

1. Package identity is the manifest `name`. A program contains at most one package per name. Two canonical directories
   with the same name, or one name reached with two different `version`s, is `pkg/identity_conflict` listing both
   display paths and versions.
2. A package mounted by several packages (a diamond, e.g. the program and `lines` both mount `json`) is loaded,
   checked, proved and emitted once. Its nominal types are one type everywhere: a `crate::json::Value` produced by
   `lines` is the program's `crate::json::Value`. Mount names may differ (`mod js;` with key `js` pointing at package
   `json`); identity does not.
3. The program's own package keeps today's identities, display paths and generated names byte for byte, so adding a
   manifest with no dependencies changes no output except the trust report (section 6).
4. Declaration identity of a dependency item is `(package name, module path inside the package, item name)`.
   Diagnostics, traces and contract panic text spell it `@NAME::module::item` (`@json::parse_value`). The `@` keeps it
   distinct from consumer paths and is never valid source syntax.
5. Display path of a dependency file is `@pkg/NAME/<path relative to the package directory>`
   (`@pkg/json/json.pruv`). No machine path appears in any deterministic output.
6. Symbol allocation (G4 section 6) gains one kind pair for dependency items: `pruv_p_f_` and `pruv_p_t_`, followed by
   the same length-delimited component encoding with the package name as the first component. Program-package items
   keep `pruv_m_f_` / `pruv_m_t_` and root source spellings. G3 specialization keys include the package name as part of
   the declaration identity.

### 4.4 Proof and emission

Nothing about proof changes: every definition of every loaded package is verified on every compile (unless the proof
cache of section 7 is enabled), calls across packages use the G4 call rule, a failing or unknown proof in any package
fails the compile with the ordinary stage and code and the dependency's display path. `--no-verify`, `--trust-callers`
and `--foreign-checks` apply program-wide as today. Output is one Rust file; foreign manifests keep every used trusted
declaration including those in dependencies.

## 5. Versions

1. A requirement is `MAJOR`, `MAJOR.MINOR` or `MAJOR.MINOR.PATCH` and means "at least this, same compatibility line".
   The compatibility line is the major when it is at least 1, otherwise `0.MINOR` (so `0.1` does not accept `0.2.0`).
   Missing parts are 0.
2. With path sources the version is whatever the dependency manifest says. Every requirement on a name must be satisfied
   by it, otherwise `pkg/version_unsatisfied` listing each requiring package, its requirement, and the found version.
3. Rule fixed for later source kinds with several available versions: per name, select the highest minimum among all
   requirements reachable in the selected graph (Go MVS), never a newer version than some requirement asks for;
   requirements from different compatibility lines are `pkg/version_unsatisfied`, never two copies.
4. Compatibility is a promise by the author, not checked. The design records which contract changes break consumers
   (research notes, "Contract changes"): strengthening requires, weakening ensures, removing or renaming a public item.
   A checker is deferred (section 9).

## 6. Trust report

`--trust-report PATH` writes a JSON file. Apart from the `compiler` object it is the same bytes from both compilers for
the same input and flags. It is
written for every compile that reaches the end of checking, including failing proofs, so a user can see what a failed
compile would have trusted. It is allowed with and without `--manifest` (the program then has one package, named `""`
when there is no manifest).

```json
{
  "format": 1,
  "compiler": { "impl": "rs", "version": "<compiler version>", "solver": "<full Z3 version>" },
  "flags": { "verify": true, "trust_callers": false, "foreign_checks": "requires", "infer": false,
             "rust_extern": [], "rust_search": [], "proof_cache": false },
  "packages": [
    {
      "name": "jstr", "version": "0.1.0", "display": "@pkg/jstr",
      "sha256": "<hex over the files loaded from this package>",
      "trusted": [
        { "kind": "trusted_rust_fn", "item": "@jstr::utf8_to_string",
          "file": "@pkg/jstr/jstr.pruv", "line": 2, "reachable": true }
      ],
      "catalog": [ { "op": "map_updated", "type_args": ["..."] } ]
    }
  ],
  "proof_cache": { "hits": 0, "stores": 0 }
}
```

1. `packages` is sorted by name; the program package is first. A package with nothing trusted still appears, with
   `"trusted": []`, so the report doubles as the list of exact sources the result depends on.
2. `sha256` is over the files loaded from that package in load order, each as `<display path>\0<byte length>\0<bytes>`.
   Rust gets SHA-256 from the `sha2` crate (one new dependency); Python from `hashlib`.
3. Trust points, from the survey in section 1.1, are a closed list:
   - per package, `trusted`: one entry per `trusted rust fn`, kind `trusted_rust_fn`, with item, file, line, and the
     `reachable` value the foreign manifest already computes; sorted by file then line.
   - per package, `catalog`: the built-in catalog operations (collections, `File`, `Process`, `Net`, `Loop`) used by
     that package's code, as `{op, type_args}` exactly as the foreign manifest's `catalog[]`, sorted.
   - program-wide, `flags`: `verify` (false under `--no-verify`), `trust_callers`, `foreign_checks`, `infer`,
     `rust_extern` and `rust_search` (the values exactly as given on the command line, in flag order), `proof_cache`.
   - program-wide, `compiler`: implementation, compiler version and full solver version. Builtins and catalog models
     are part of the compiler, so this object names that trusted base; builtin uses are not listed one by one.
   Adding an unproved construct to the language later must add a kind; G4b-2 adds a fixture that fails when a trust
   construct in either parser or catalog has no report case.
4. Relation to the foreign manifest: `pruv-foreign-v1` is unchanged and still written only for Rust source output.
   The trust report reuses the same collection code (so the two never disagree), adds package, version, hash, flag and
   compiler fields, and is available for every backend.
5. Only items that are loaded count; there is no new transitive "uses" analysis beyond the existing `reachable`. A
   loaded trusted function is listed even if nothing calls it, matching the rule that every loaded definition is part
   of the program.
6. `proof_cache` counts are 0 when the cache is off.

## 7. Proof cache

`--proof-cache DIR` enables a local, content-addressed cache of solver answers. Off by default, so the G8 policy
"every compile re-proves" stays the default behavior.

1. Key: SHA-256 over `<compiler id>\0<full solver version>\0<query text>`. The compiler id is the implementation
   (`py` or `rs`) and its version string; the solver version is `z3.get_full_version()` / `Z3_get_full_version`. The
   query text is exactly the SMT2 the trace already records for the attempt: the `set-option` lines for timeout, rlimit
   and seed followed by `sol.sexpr()` / `Solver::to_smt2()`. The options are therefore part of the text, so an
   inference attempt (500 ms, rlimit 100000, seed 0) and a verification attempt (8000 ms) of the same formula have
   different keys. Nothing else (file paths, package names, definition names) is in the key. With the cache on, the
   text is rendered for every attempt, not only when tracing.
2. On a cache hit the compiler uses the stored answer instead of calling the solver. Only `unsat` answers (the
   obligation is proved) are stored. `sat` (counterexample) and unknown are never stored, so failure output, models
   and inference search on failures always come from a live solver.
3. Entry format: file `DIR/<first 2 hex>/<key hex>` containing `pruv-proof-cache 1\nunsat\n`. Written to a temporary
   file in the same directory and renamed, so concurrent compiles never see partial entries. A missing, unreadable or
   malformed entry is a miss; it never fails the compile.
4. Applies to every solver call, including invariant inference and G9 measure obligations, because it sits at the
   one check call per attempt (section 1.1: `verify.py:1164`, `PROOF/obligation.rs:419`). A hit records the attempt in
   the trace with its usual fields and `"cached": true`. Unknown, timeout, `canceled` and errors are never success and
   never stored.
5. Soundness argument: a hit returns `unsat` only for byte-identical input to the same solver and options that once
   answered `unsat`. The trust that remains is in the cache directory's integrity (anyone who can write `DIR` can
   insert answers). That is why use is reported (section 6) and why entries carry no data the solver did not produce.
6. No eviction, size limit or cleanup in G4b; deleting `DIR` is always safe.
7. Query bytes must be deterministic for the same input for hits to happen. Rust uses a fresh Z3 context per function,
   which helps; Python shares one context across the program, so auxiliary names in the text may depend on what was
   verified before. A nondeterministic query is a missed saving, not an unsound result; G4b-3 measures it on the L3
   pieces and the cold/warm test of G4b-3 must pass on each compiler before the batch is done.
8. Rendering the text costs time on every attempt. If G4b-3 measures that the cache-on cold compile is slower than
   cache-off by more than 10 percent on an L3 piece, the batch records it in RESULTS.md and the decision on option 7 is
   revisited before G4b-Q.

## 8. Batches

Each batch ships the Python implementation and its Rust port in one commit (AGENTS.md rule 6). Tests run per group
while developing; the full suites run once before each commit as AGENTS.md specifies; every commit that changes the
Rust compiler runs `cargo test --release` and `tests/diff_rust.py --rs-path pruvc-rs/target/release/pruv-rust-dump
--fuzz-count 20` before committing. Audits: G4b-1, G4b-2 and G4b-3 introduce new semantics and get one audit each;
a confirmation audit only when the audit has required findings, limited to them.

### G4b-1: manifests, package namespaces, identity, version check

Scope: sections 3, 4, 5 in both compilers; `--manifest`; the new `pkg/*` diagnostics; symbol kinds `pruv_p_*`.

Migration: `bench/l3/lib/NAME.pruv` become seven packages `bench/l3/pkgs/NAME/{pruv.json,NAME.pruv}` whose roots add
`mod` lines for their dependencies (graph in section 1.1). Item bodies are unchanged. Pieces in
`bench/l3/{json,mcp,qwen,supervisor}` get a `pruv.json` and drop the `mod` lines for modules they only needed
transitively. `bench/l3/lib/` and `--lib` stay (the `g4_frontend/lib` row keeps covering `--lib`) until a later batch
decides to retire them; the pieces stop using it.

Done when:
- `tests/g4b/check_pkg.py` passes on both compilers (`--rs PATH` as in `tests/g4/check_lib.py`).
- Generated Rust for every existing test row is byte-identical (no manifest in any existing row).
- A program with a manifest and no dependencies produces the same Rust as without the manifest.
- All L3 piece tests named in AGENTS.md pass on the package layout with unchanged runtime results.
- `diff_compilers.py` / `diff_rust.py` compare manifest programs (saved cases copy the package directories).

Tests (`tests/g4b/check_pkg.py`, run_all row `g4b_frontend/pkg`, plus proof rows where a case needs the solver):
manifest key/type errors; bad name, version, root, path; `--manifest` with `--lib`; root mismatch; dependency mount;
mount alias (key differs from name); unused dependency; mount conflict with a local file; missing module hint;
dependency cycle of two and three packages; identity conflict (same name, two directories; same name, two versions);
version unsatisfied, including `0.1` vs `0.2.0`; diamond (program and `lines` both mount `json`) loaded once with one
`Value` type proved to flow between them; a dependency's private item not reachable; a dependency not reachable from a
package that did not mount it; failing proof in a dependency reports `@pkg/NAME/...`; `mod std;` in a dependency root;
`pruv_p_*` names stable under renaming the mount key.

### G4b-2: trust report

Scope: section 6. First step fixes the closed list of trust kinds from the survey in section 1.1.

Done when:
- `--trust-report` output, without the `compiler` object, is byte-identical between the compilers on every case of the
  test file.
- Every trust construct in section 6 rule 3 has at least one case; the completeness fixture fails if a construct is
  removed from the report code.
- The L3 `mcp` piece's report has `@jstr::utf8_to_string` as its only `trusted` entry and lists the `Process`, `Loop`
  and collection operations it uses under `catalog`, matching its foreign manifest.

Tests (`tests/g4b/check_trust.py`, row `g4b_frontend/trust`): one case per trust kind; each flag in `flags`; report
written for a failing proof; no-manifest program; package sorting and `sha256` (changing one byte of one dependency
changes only that package's hash); determinism across two runs and across the compilers.

### G4b-3: proof cache

Scope: section 7.

Done when:
- A second compile of each L3 piece with a warm cache makes no solver call for obligations proved in the first, and the
  generated Rust, diagnostics and trace are byte-identical to a cold compile.
- Changing one callee `ensures` in a dependency causes a live solver call for exactly the queries that mention it, and
  a now-false obligation fails as before.
- A corrupted, truncated or unreadable entry is a miss.
- The trust report counts hits and stores.

Tests (`tests/g4b/check_cache.py`, row `g4b_proof/cache`): cold/warm identity; contract change invalidation; failing
and unknown obligations never stored; corrupt entry; read-only `DIR` (compile succeeds, no stores); two concurrent
compiles into one `DIR`; keys differ between the two compilers and between solver option changes.

### G4b-Q: stress, mutants, measurement

Q-only work (AGENTS.md rule 7): fuzz manifests and package graphs (random DAGs, diamonds, aliases, cycles) against both
compilers; mutants of the resolver (drop the cycle check, the identity check, the version-line check, the unused check)
must each be caught by a test; mutants of the cache (store `sat`, drop the solver options from the key) must be caught;
measure cold vs warm compile time and hit rate for every L3 piece and record it in RESULTS.md.

## 9. Deferred

- Git, archive and registry sources, the lock file that pins them (option 5), and MVS selection over several
  available versions (section 5 rule 3 fixes its semantics).
- `mod std;` in dependency roots: needs std to become a shared bundled package without changing the generated names of
  existing std programs.
- A contract-compatibility checker between two versions of a package.
- Proof hints (option 8), pre-verified bundles (option 6), curated library sets (option 10).
- Re-exports, `use`, opaque types (unchanged from G4 section 15).
- Retiring `--lib`.

Open evidence: whether query bytes are deterministic across runs in each compiler (decides the cache hit rate), and
the real saving on the L3 pieces. Neither is measured by this design.
