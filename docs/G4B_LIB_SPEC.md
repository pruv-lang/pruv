# G4b-0: Library directories (`--lib DIR`)

Status: done (2026-11-16, RESULTS.md section 131). First step of G4b (G4_SPEC.md section 15). Packages, package identities,
versions, manifests and separate compilation stay deferred.

## 1. Why

The L3 pieces carry byte-identical copies of the same verified modules: `json`, `jstr`, `lex`, `lines`, `parse`,
`access` in `bench/l3/{claude,json,mcp,qwen}/` and `tail` in `bench/l3/{mcp,supervisor}/`. A fix to one copy
silently diverges from the others. The compiler only finds module files next to the root, so there is no way to
share one file. Two or more pieces need this, so it is a language step, not a host workaround.

## 2. Rule

`--lib DIR` (repeatable, order kept) names a library directory. It only changes where the file of a module declared
in the root file is looked up. Nothing else (names, visibility, proof, code generation) changes.

1. **Root declarations only.** For `mod NAME;` written in the root file, with `NAME != std`:
   - the local file is `ROOT_DIR/NAME.pruv`; a library candidate is `DIR/NAME.pruv` for each `--lib DIR`.
   - existence uses the same test as the std conflict check (lexists: a directory or dangling link also counts).
   - exactly one candidate in total (local or one library): use it.
   - none: the existing `parse/module_file` error; the hint also says `or pass --lib DIR`.
   - more than one (local plus a library, or two libraries): `parse/lib_module_conflict`,
     message `module NAME is found in more than one place: <places>`, hint
     `remove the duplicate file or drop one --lib`. `<places>` lists the display paths of section 3 in lookup order
     (local first, then libraries in flag order), joined by `, `.
2. **Children of a library module** resolve inside that library: `mod x;` in library module `json` reads
   `DIR/json/x.pruv`. Libraries are never consulted for modules declared below the root.
3. **Display path** (the `file` field of diagnostics and every place the loaded relative path is used):
   `@lib/NAME.pruv`, `@lib/NAME/x.pruv` for its children. It never contains `DIR`, so output is identical on every
   machine and between the two compilers. Local modules keep their relative path.
4. **Mounting.** A library module is mounted at `crate::NAME` exactly like a local one. Library modules refer to
   each other with `crate::OTHER` paths, so a program that uses `lines` must also declare `json`, `lex` and `parse`
   (each from a library or locally). There is no package namespace; that is G4b proper.
5. **std** is never looked up in libraries; `mod std;` keeps its G8 rules (a `std.pruv` inside a library directory is
   ignored and is not a conflict).
6. **Aliases.** The existing file-identity alias check still applies to library files.
7. **Legacy roots** (no `mod` and no top-level `pub`) ignore `--lib`.
8. **CLI.** Both compilers accept `--lib DIR` before or after other flags. An empty value or a path that is not an
   existing directory is a usage error with the same exit code and style as the other path flags
   (`--lib needs an existing directory`). The value is not written to any output or manifest.

## 3. Tests

`tests/g4/check_lib.py` builds small temporary projects and runs one compiler (`--rs PATH` selects the Rust one, the
same convention as `tests/g8/check_codegen.py`). Cases: library-only module, library child module, two libraries,
local+library conflict, two-library conflict, missing module hint, std inside a library ignored, library module that
fails to verify reports `file` = `@lib/NAME.pruv`, legacy root ignores `--lib`, bad `--lib` value, generated Rust
identical to the same project with the files copied next to the root. `run_all.py` runs it as row
`g4_frontend/lib`.

## 4. Use

`bench/l3/lib/` holds the single copy of `json`, `jstr`, `lex`, `lines`, `parse`, `access` and `tail`.
`bench/l3/{json,mcp,qwen,supervisor}/` drop their copies and compile with `--lib bench/l3/lib`.
`bench/l3/claude/*.pruv` stays frozen as the L3-1 record (byte-identical to the library today).
`time.pruv` (claude, qwen) depends on the per-piece `text` module and stays local.
