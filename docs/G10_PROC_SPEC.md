# G10: the `proc` effect and the owned `Process` handle

Status: **A0–A3 implemented, 2026-11-16 (frontend, verification, Rust runtime, LLVM shim, Windows run). Q is next.**
Batch status is recorded per batch below and in `RESULTS.md` with Q.

This document defines the process slice that `docs/G10_SPEC.md` §9 deferred. A Pruv program can start a
child from a program and an argument list, write and close its stdin, read its stdout and stderr, and end
ownership with `proc_wait` or `proc_kill`.

Python `pruvc` gets each feature first and `pruvc-rs` ports it in the same batch. The Rust source backend
uses `std::process`. The LLVM backend reaches the same Rust runtime through the G6 Rust shim, exactly as
`File` does. There is no C process runtime in this slice.

The rules and gates in §3–§8 are acceptance conditions, not measurements.

## 1. Goals

- Run a program with an ordered argument list, never through a shell.
- Choose the working directory.
- Write strings or bytes to stdin and close it explicitly.
- Read stdout and stderr in chunks or read everything that is left.
- Get the exit status, or kill the child.
- A `Process` can move, but on every normal control-flow path its last owner must consume it with
  `proc_wait` or `proc_kill`.
- Spawn and pipe I/O failures are values.
- Both compilers and both backends agree on meaning, diagnostics and generated output.

The existing ownership checker is reused. Only `Process` gets the new must-consume rule. The `File`
auto-close policy does not change.

Repository facts this design relies on (HEAD 076d6f7):

- The effect list is `io`, `alloc`, `fs` (`pruvc/vparser.py`, `C/parser.rs`).
- `[str]`, `Vec<str>` and `Vec<String>` are not supported (`pruvc/check.py` array/collection payload rules),
  so an argument list needs its own type.
- An owned catalog type cannot sit inside `Result` (`pruvc/check.py`), so `Result<Process>` is not allowed.
- `file_open_read` returns a `File`, not a `Result`; failure is `file_ok == false`.
- The ownership checker tracks live/dead/maybe, moves, branch joins and loop restoration, but today it does
  not require a live owner to be consumed at scope end or return.
- The `par` forbidden-effect collection lists `io` and `fs` explicitly (`pruvc/check.py` `par_info`,
  `C/check.rs`).
- LLVM keeps `File` as an opaque pointer; consuming adapters unbox, updates return a new box; scope cleanup
  calls the drop adapter on non-null owned slots. Native `--emit exe|obj` refuses shim programs with
  `codegen/foreign_target`.

Path abbreviations: `C/` = `pruvc-rs/crates/pruv-core/src/`, `P/` = `pruvc-rs/crates/pruv-proof/src/`,
`D/` = `pruvc-rs/crates/pruvc/src/`.

## 2. User surface

### 2.1 The `proc` effect

`proc` is the fourth effect name.

```pruv
fn run(program: str) -> Result<i64> ! proc, alloc
```

`proc` does not include `io` or `fs`. Reading a child's stdout is `proc`; Pruv's own `print` stays `io`.

Spawn and state-changing operations need `proc, alloc` (output collection and buffers allocate). The
argument builder and `proc_bytes` need `alloc` only. Observers have no effect.

### 2.2 Types

**`Process`** — owned catalog type, internal key `process`, generated Rust name `pruv_Process`.

- Allowed as a local, a `take` parameter, a read-only borrowed parameter, and the result of an internal
  function.
- No copies, no aliases, no storage inside owned containers (struct, enum, array, Option, Result,
  collections).
- Cannot be a parameter or result of a host entry.
- Moves use `move p`.
- Auto-drop cannot discharge it. A handle from a failed spawn must also be consumed by `wait` or `kill`.
- A `Process` owns the child, its stdin, both output collectors and the public read buffers.

**`ProcessArgs`** — small owned catalog type for argument lists, internal key `process_args`, generated
Rust name `pruv_ProcessArgs { args: Vec<String> }`. It exists only because `[str]` is not supported; this
slice does not generalize collections.

- Ordered list of UTF-8 strings.
- Affine with auto-drop, like `String`. Not must-consume.
- Same placement limits as `Process`.
- `proc_spawn` borrows it and copies what it needs.
- An empty string is kept as one argument. No argument is split on spaces or parsed for quotes.

### 2.3 Catalog operations

Notation (descriptive only, not syntax): `P` = `Process`, `A` = `ProcessArgs`, stream `s`: `1` = stdout,
`2` = stderr, `M` = the existing array length bound `2^40`, `ok(p)` = `proc_ok(p)`, `err(p)` =
`proc_error(p)`, `open(p)` = `proc_stdin_open(p)`, `size(p,s)` = `proc_size(p,s)`, `eof(p,s)` =
`proc_eof(p,s)`.

Parameter observers in contracts mean the value **at call entry**.

#### Argument list

| Operation | Effect | Contract / meaning |
|---|---|---|
| `proc_args_new() -> ProcessArgs` | `alloc` | ensures `proc_args_len(result) == 0` |
| `proc_args_push(take a: ProcessArgs, arg: str) -> ProcessArgs` | `alloc` | requires `proc_args_len(a) < M`; ensures `proc_args_len(result) == proc_args_len(a) + 1`; appends `arg` unchanged |
| `proc_args_len(a: ProcessArgs) -> i64` | none | ensures `0 <= result && result <= M`; allowed in code and contracts |

An argument containing NUL can be stored; `proc_spawn` reports it as input error 1. The builder has no
failure state.

#### Spawn and stdin

| Operation | Effect | Contract / meaning |
|---|---|---|
| `proc_spawn(program: str, args: ProcessArgs, cwd: str) -> Process` | `proc, alloc` | no requires. Success: `ok(result) && open(result)`. Failure: `!ok(result) && !open(result)` with error 1 or 2. Both public buffers empty, both EOFs false |
| `proc_spawn_env(program: str, args: ProcessArgs, cwd: str, env_names: ProcessArgs, env_values: ProcessArgs) -> Process` | `proc, alloc` | same contract as `proc_spawn` (no requires; success `ok && open`, failure `!ok && !open` with error 1 or 2, buffers empty, EOFs false). All parameters borrowed. The child inherits the parent environment, then `(names[i], values[i])` are set in order; see §8b |
| `proc_write_str(take p: Process, text: str) -> Process` | `proc, alloc` | requires `ok(p) && open(p)`; writes all UTF-8 bytes; on success the observed state is preserved |
| `proc_write_bytes(take p: Process, data: [u8]) -> Process` | `proc, alloc` | same requires; writes all bytes; on success the observed state is preserved |
| `proc_close_stdin(take p: Process) -> Process` | `proc, alloc` | requires `ok(p) && open(p)`; ensures `!open(result)`; on success buffers and EOFs are preserved |

`cwd == ""` inherits the parent's working directory; otherwise it is the child's working directory. The
parent's own working directory never changes. A bare program name uses the platform search rules; an
explicit path is used as is. Relative program paths combined with a separate cwd differ across platforms,
so programs that need consistency pass an absolute program path.

#### Output and termination

| Operation | Effect | Contract / meaning |
|---|---|---|
| `proc_read(take p: Process, stream: i64, max: i64) -> Process` | `proc, alloc` | requires `ok(p)`, `stream == 1 \|\| stream == 2`, `0 < max && max <= M`; puts the next chunk of that stream in its public buffer |
| `proc_read_all(take p: Process) -> Process` | `proc, alloc` | requires `ok(p) && !open(p)`; reads the rest of both streams to EOF |
| `proc_bytes(p: Process, stream: i64) -> [u8]` | `alloc` | requires a valid stream; independent copy of the public buffer. ensures `len(result) == size(p,stream)` and every element equals the matching `proc_byte` |
| `proc_wait(take p: Process) -> Result<i64>` | `proc, alloc` | no requires. Closes stdin, keeps draining output, waits for the child, consumes the handle |
| `proc_kill(take p: Process) -> Result<i64>` | `proc, alloc` | no requires. Requests termination of the direct child if still running, reaps it, consumes the handle |

`proc_read` success contract:

1. `ok(result)`.
2. `0 <= size(result,stream) && size(result,stream) <= max`.
3. `eof(result,stream) == (size(result,stream) == 0)`.
4. The other stream's buffer and EOF, and the stdin-open state, are preserved.
5. Reading a stream already at EOF gives an empty buffer and EOF true.
6. The chunk with the last bytes may have EOF false; the next, empty read gives EOF true.
7. It blocks until bytes, EOF or an error arrive. "No bytes yet" is never reported as EOF.

`proc_read_all` success contract:

1. `ok(result) && !open(result)`.
2. Both EOFs are true.
3. Each public buffer holds the bytes of that stream **not yet delivered by earlier reads**. Earlier
   buffers are not prepended.
4. No global order between the two streams is defined.

`proc_bytes` returns raw bytes. Child output and partial UTF-8 chunks are never presented as `String`.
UTF-8 validation and line assembly belong to user code.

#### Observers

| Operation | Effect | Contract / meaning |
|---|---|---|
| `proc_ok(p: Process) -> bool` | none | `proc_error(p) == 0` |
| `proc_error(p: Process) -> i64` | none | `0 <= result && result <= 6`; the first published failure code |
| `proc_stdin_open(p: Process) -> bool` | none | stdin writes are allowed; true implies `proc_ok(p)` |
| `proc_eof(p: Process, stream: i64) -> bool` | none | requires a valid stream; EOF seen by the last explicit read |
| `proc_size(p: Process, stream: i64) -> i64` | none | requires a valid stream; ensures `0 <= result && result <= M` |
| `proc_byte(p: Process, stream: i64, i: i64) -> i64` | none | requires a valid stream and `0 <= i && i < proc_size(p,stream)`; ensures `0 <= result && result <= 255` |

Observers are allowed in code and contracts. **Observers never query the OS or the internal queues.** The
same Pruv value always gives the same observer results, even while the child runs. Internal output becomes
public only at the next explicit effect operation.

### 2.4 Failures and exit codes

These codes live only in `proc_error` and in `Err` of the terminal `Result<i64>`. They are not compiler
diagnostic codes.

| Value | Meaning |
|---|---|
| `0` | no published failure |
| `1` | bad input: empty program, NUL in program/argument/cwd, unsupported Windows batch file |
| `2` | spawn, or initial pipe / collector setup, failed |
| `3` | stdout or stderr read failed |
| `4` | stdin write or close failed |
| `5` | reaping the child's exit status failed |
| `6` | the kill request failed |

Rules:

1. A failed spawn returns an empty failed `Process` (the `file_ok` pattern). `Result<Process>` stays
   disallowed.
2. The first published error is kept. Later cleanup errors do not overwrite it.
3. When an update fails: stdin is closed, both public buffers are emptied, both EOFs become false. The
   child is still owned.
4. So after a failure, ordinary read/write cannot meet their requires. `wait` and `kill` are always
   allowed.
5. A write can fail after a partial write. `Err` does not mean "nothing was written". No automatic retry.
6. If a read fails midway, that call's partial data is not published. Earlier `[u8]` copies are unaffected.
7. `wait` and `kill` clean up even when an entry error exists, and return that error as `Err`.
8. `Ok(code)` from `wait`/`kill` is the exit code or platform status. **`Ok(7)` is exit code 7, not a
   failure.**
9. On Unix a signal exit is the negative signal number. On Windows the exit code is the unsigned 32-bit
   value widened to i64. The exact number after a successful kill is not part of the portable contract.
10. Killing a child that already exited succeeds if its status is reaped. The exit/kill race is not error 6.
11. If the kill request fails, the handle is not abandoned; reaping is still attempted and may block.
12. An error result does not guarantee that the OS terminated or reaped the child. Must-consume is the
    program's resource duty, not a proof that failing OS calls succeed.
13. OOM, the Pruv process being killed, and contract-violation aborts are distinct from I/O errors and keep
    the existing allocation and panic policy.

### 2.5 Deadlock and blocking

A program that reads only stdout must not hang because the child's stderr pipe filled up. The child may
also write output while the parent is writing stdin. Therefore:

- Independent internal draining of stdout and stderr is set up before spawn returns.
- Both pipes are drained whichever stream the caller reads.
- Internal collectors never touch the public observer fields.
- Internal queues are memory-backed and unbounded in this slice. No memory bound is promised for unbounded
  output.
- `wait` and `kill` keep draining, discarding output that will no longer be delivered.
- Protocol deadlocks (the child waiting for input) and children that never exit are not solved.
- A grandchild holding an output pipe can delay EOF and collector join. Process trees and timeouts are
  non-goals.

The internal reader threads are an implementation means. They do not add user-facing async or `par`
process sharing.

### 2.6 Examples

These compile, verify and run once A1–A3 land.

```pruv
pub fn version(program: str, cwd: str) -> Result<i64> ! proc, alloc
{
  let mut args: ProcessArgs = proc_args_new();
  args = proc_args_push(move args, "--version");

  let mut p: Process = proc_spawn(program, args, cwd);
  if !proc_ok(p) {
    return proc_wait(move p);
  }

  p = proc_close_stdin(move p);
  if !proc_ok(p) {
    return proc_kill(move p);
  }

  p = proc_read_all(move p);
  if !proc_ok(p) {
    return proc_kill(move p);
  }

  let stdout: [u8] = proc_bytes(p, 1);
  let stderr: [u8] = proc_bytes(p, 2);
  return proc_wait(move p);
}
```

Line streaming reads `proc_read(move p, 1, 4096)` chunks in order and completes a line at LF (`10`),
keeping leftover bytes across chunks and flushing the last unterminated line at EOF. CRLF handling is the
caller's choice. Because Pruv loops need `decreases`, verified streaming takes a finite budget of reads or
bytes:

```pruv
fn read_window(take input: Process, rounds: i64) -> Process ! proc, alloc
  requires proc_ok(input)
  requires 0 <= rounds && rounds <= 1000000
{
  let mut p: Process = move input;
  let mut remaining: i64 = rounds;

  while remaining > 0 && proc_ok(p) && !proc_eof(p, 1)
    invariant 0 <= remaining && remaining <= rounds
    decreases remaining
  {
    p = proc_read(move p, 1, 4096);
    if proc_ok(p) {
      let chunk: [u8] = proc_bytes(p, 1);
      // hand chunk to a line assembler
    }
    remaining = remaining - 1;
  }

  return move p;
}
```

The helper hands the ownership duty back to the caller, who kills on budget exhaustion or waits after
EOF. `decreases` does not prove that external I/O returns, and a loop budget is not a wall-clock timeout.

## 3. Static rules

### 3.1 Effect and names

1. Add `proc` to both parsers and the AST effect description.
2. Catalog effect sets feed the existing `used_effects` check.
3. The unknown-effect hint becomes `io, alloc, fs, proc`.
4. `Process` and `ProcessArgs` normalize to their internal keys like `File`. A user declaration with the
   same name keeps priority, as today.
5. Generated Rust uses `pruv_Process` and `pruv_ProcessArgs`, never a user type name.

### 3.2 Reused ownership rules

- Use after move, double consume, owned alias: `type/foreign_move`.
- Consuming or returning a borrowed handle: `type/foreign_borrow`.
- A loop consumes an outer owner and does not restore it: `type/foreign_loop_move`.
- Storage inside struct, enum, array, Option, Result or a collection: `type/foreign_owned_type`.
- A handle in a host entry signature: `type/entry_signature` for a `pub fn`, `type/foreign_export` for `fn main`
  (the existing checks for other owned handles).
- A trusted Rust declaration that creates or consumes a Process around the catalog: `type/foreign_signature`.

No new public host ABI. Internal functions may return `Process`.

### 3.3 Must-consume for `Process` only

Add Process binding identification and the checks below to Python `_Ownership` and Rust `Ownership`. Do
not write a second ownership analyzer.

1. **Block end:** a `Process` leaving scope while `live` or `maybe` is `type/proc_unconsumed`.
2. **return:** consume the return expression first, then every owned `Process` leaving the function must be
   dead. `return move p` transfers the duty.
3. **Fallthrough:** live `take Process` parameters and locals are checked the same way.
4. **Overwrite:** after evaluating and consuming the RHS, a destination still holding a live/maybe Process
   is rejected. `p = proc_read(move p, 1, 4096)` is allowed; overwriting a live `p` with a new spawn is not.
5. **Temporaries:** an unnamed Process result also carries the duty. `proc_wait(proc_spawn(...))` is
   allowed (taken at once). `proc_ok(proc_spawn(...))` (borrowed then dropped) is rejected, and so is an
   update result that is never used.
6. **Branches:** consumed in one branch only gives maybe at the join, which is rejected at scope end and
   overwrite, not just on use. Consumed in both branches is allowed.
7. **Loops:** the existing outer-owner restoration rule stays. A Process created inside a loop body must be
   consumed or moved out before that iteration's scope ends.
8. **Helpers:** a `take Process` helper must terminate it, pass it to another take helper, or return it. A
   helper that takes and forgets is rejected.
9. **Failed handles:** a failed spawn is not exempt, so the checker never needs to know whether the OS
   call succeeded.
10. **Modes:** `--no-verify`, `--trust-callers` and `--foreign-checks off` cannot turn this rule off.

The rule guarantees exactly-once termination or transfer on normal control flow. It does not prove eventual
cleanup across infinite blocking or forced aborts. The language has no user `break`/`continue`, and this
slice does not add them.

### 3.4 `par` and contracts

- Add `proc` to the `par_info` forbidden effects. Direct and indirect `proc` calls are rejected with
  `type/parallel_effect`.
- Existing owned-argument and catalog-leaf limits are not relaxed.
- Define a separate `PROC_OBSERVERS` list. Calling spawn/read/write/wait/kill or the allocating
  `proc_bytes` in a contract is `type/foreign_contract`.
- Observer stream/index requires apply to the domain checks of both code and contracts.

### 3.5 Drop

`ProcessArgs` auto-drops at normal scope end. `Process` never ends normally through auto-drop. The
generated runtime has a defensive `Drop` (best-effort kill and reap) for unwinding and internal bugs, but it
does not satisfy the must-consume rule. `File`, `String`, `Vec` and `HashMap` drop policies are unchanged.

## 4. Verification encoding

1. **Process value.** A symbolic record with the public state only:

   ```text
   error: Int
   stdin_open: Bool
   stdout_eof: Bool
   stderr_eof: Bool
   stdout_buf: (length: Int, bytes: Array Int Int)
   stderr_buf: (length: Int, bytes: Array Int Int)
   ```

   `proc_ok` is `error == 0`. The child, PID, descriptors, queues and reader timing are not modeled.

2. **Well-formedness.**
   - `0 <= error <= 6`.
   - Each buffer length is in `0..M`; bytes at valid indexes are in `0..255`.
   - `stdin_open ==> error == 0`.
   - `error != 0` implies stdin closed, both buffers empty, both EOFs false.

   EOF true and an empty buffer are not made globally equivalent: `read_all` can return EOF true with a
   non-empty final buffer.

3. **ProcessArgs value.** Only `argc: Int` with `0 <= argc <= M`. new and push change it exactly. Argument
   contents and their mapping to the OS argv are trusted runtime meaning, checked by runtime gates, not
   proved.

4. **Effect calls.** Same order as `File`: catalog requires → fresh result → well-formedness → ensures. Two
   identical spawns, successive read contents and exit codes are independent external results; they are
   never memoized or treated as equal because their inputs are equal.

5. **Preservation.** A successful write or close preserves both output snapshots. A successful single-stream
   read preserves the other stream's snapshot. Encode this as equal length and equal byte map. Write the
   success/failure cases exactly as the §2 transitions; do not drop preservation facts because the result
   is fresh.

6. **Observers and copies.** Observers are field reads or array selects. `proc_bytes` is a new array with
   the snapshot's length and byte map; at runtime its memory is independent.

7. **Terminal results.** Use the existing `Result<i64>` encoding.
   - With an entry error, `result == Err(proc_error(p))`.
   - On `is_ok(result)` the payload is the i64 exit status.
   - On Err, `1 <= err_val(result) && err_val(result) <= 6`.
   - `Ok(0)` is never assumed. A successful kill is not tied to any exit number.

8. **Entry, return, havoc.** Function parameters, user function returns, loop havoc, counterexample output
   and type recovery handle Process and ProcessArgs. After havoc only well-formedness and proven invariants
   remain; earlier `ok`, sizes and EOFs are not kept.

9. **Domains.** Register `proc_byte` for contract-domain checks like the File byte observer. A postcondition
   cannot become true through `proc_size`/`proc_eof` with an unchecked stream or `proc_byte` with an
   unchecked index.

10. **Ownership and termination.** Must-consume is static and is not re-encoded in SMT. No axioms about
    child termination, EOF arrival, wall-clock time or total memory.

11. **Inference.** `--infer` does not guess OS-state invariants or termination measures for Process. Add
    only the branches needed so the new types are not treated as scalars or collection elements.

## 5. Rust source backend

### 5.1 Representation

```rust
struct pruv_ProcessArgs {
    args: Vec<String>,
}

struct pruv_Process {
    child: Option<std::process::Child>,
    stdin: Option<std::process::ChildStdin>,
    // stdout/stderr collector state and join handles
    // public state, changed only by explicit operations
    error: i64,
    stdin_open: bool,
    stdout_eof: bool,
    stderr_eof: bool,
    stdout_buf: Vec<u8>,
    stderr_buf: Vec<u8>,
}
```

The queue struct is an implementation detail, but public state and worker state must be separate.

### 5.2 Spawn

1. `Command::new(program).args(...)`. No shell, no command strings, no splitting on spaces.
2. `current_dir` only when cwd is non-empty.
3. stdin, stdout and stderr are all piped.
4. Check NUL in program/arguments/cwd and an empty program before spawning.
5. On Windows, `.bat`/`.cmd` are refused as targets, including when a bare name would resolve to one.
   Refusal is error 1.
6. Windows quoting uses `Command`'s standard argument passing. No `raw_arg`, no hand-made shell quoting.
   argv identity for non-standard command-line parsers is not promised.
7. If collector setup fails after spawn, clean up the child and return a failed handle with error 2.

### 5.3 Draining both outputs

One internal reader thread each for stdout and stderr, sending chunks, EOF and read errors to an internal
queue.

- The queues drain ahead of the consumer so one full pipe never blocks the other stream.
- A public read takes at most `max` bytes from the chosen queue and keeps the rest for the next read.
- `read_all` collects the rest of both queues until EOF.
- Order within a stream is kept; no order across streams is synthesized.
- A read error before EOF is error 3. A reader thread panic is a read failure, never success.
- EOF and errors become public at explicit effect calls, not at observer calls.

Internal errors are published in a fixed order: the already recorded error → the current operation's error
→ the stdout collector's error → the stderr collector's error. Arrival time is not part of the contract.

### 5.4 Writing and cleanup

- Writes use `write_all`: success means everything was delivered; failure may mean a partial write.
- A broken pipe must not abort the Pruv process; it is error 4.
- close really releases the stdin endpoint. A second close on the same handle is a requires violation.
- wait closes stdin, keeps draining both outputs, waits, and joins the collectors.
- kill targets the direct child only, then waits to reap it, handling the already-exited race.
- Neither terminal operation returns the public buffers; copy them first with `proc_bytes`.
- Update operations consume the old value and return the new one. The defensive `Drop` (best-effort kill
  and reap) is never a reason to skip must-consume.

### 5.5 Runtime contract checks

Follows the existing `--foreign-checks requires|all|off`.

- An enabled requires violation uses the existing `pruv: foreign requires violated:` prefix and exit 101.
- Under `--no-verify`, the default foreign requires checks still catch writes to closed stdin, bad streams
  and bad max/index.
- With checks off, a bad index or handle still cannot cause UB; the existing representation defenses stay.
- OS failures never use this panic path.

## 6. LLVM backend

### 6.1 Same Rust shim as File

1. LLVM holds Process and ProcessArgs as opaque pointers.
2. The generated Rust bindings' G6 shim contains **the same runtime/helper code** as §5.
3. Catalog adapters evaluate arguments once, in order.
4. A take adapter unboxes its owned box exactly once.
5. Process updates return a new owned box. wait and kill do not create a new box.
6. A moved LLVM slot becomes null so scope cleanup does not consume it again.
7. ProcessArgs uses the normal drop adapter.
8. The Process drop adapter is defensive only; a live Process drop in a normal program is rejected by §3.
9. Observers that appear only in contracts are still collected for the shim.

### 6.2 Adapter ABI (shim-internal, not host ABI)

| Kind | Shape |
|---|---|
| spawn | program `(ptr,len)`, args handle, cwd `(ptr,len)` → Process pointer |
| update | Process pointer plus scalar/slice arguments → Process pointer |
| observer | borrowed pointer plus scalars → `i64`; bool is 0/1 |
| wait, kill | Process pointer, `tag_out: ptr`, `value_out: ptr` → void |
| bytes snapshot | borrowed Process pointer, stream → borrowed `(data,len)` view through out parameters |

Terminal shim tags are fixed: `1 = Ok`, `0 = Err`. LLVM assembles the existing internal Result from them.
Host Result returns keep the existing target ABI lowering.

`proc_bytes` copies the borrowed view at once with the existing `pruv_copy_v2(..., size=1, align=1)`; the
handle is not updated or consumed in between. Rust-allocated buffers are never passed to C `free`. Copying a
whole buffer never calls the shim once per byte.

### 6.3 `runtime.c`

**No new functions in `runtime.c`.** Only the existing v2 byte copy, allocation, free and representation
checks are used. No `posix_spawn`, `CreateProcess` or C pipe/wait code. Platform I/O for both backends lives
in the shared Rust runtime. The C runtime ABI and allocator markers do not change. A standalone process
runtime without the Rust shim is later work and needs its own C runtime design.

### 6.4 Build limits

- Run with `--backend llvm --rust-bindings PATH` and a Rust host build.
- Native `--emit exe|obj` for programs that need the shim keeps `codegen/foreign_target`.
- A program that only declares `! proc` without using Process or the catalog does not get the shim limit.
- The `io` host stdout flush policy does not change.

## 7. Diagnostics and compatibility

### 7.1 New diagnostic

| Stage / code | Condition | Hint |
|---|---|---|
| `type/proc_unconsumed` | a Process duty is left at scope end, return, overwrite or a discarded temporary | consume it with `proc_wait(move p)` or `proc_kill(move p)`, or return / transfer ownership |

Location is where the duty is lost: the return for return, the assignment for overwrite, the expression for
a temporary, and the binding's declaration for normal scope end. The message names the binding or says it is
a temporary Process.

### 7.2 Reused diagnostics

| Condition | Diagnostic |
|---|---|
| missing `proc` or `alloc` | `effect/missing_effect` |
| unknown effect | existing parse diagnostic, longer hint list |
| double wait/kill, use after move | `type/foreign_move` |
| consuming a borrowed Process | `type/foreign_borrow` |
| loop does not restore an outer owner | `type/foreign_loop_move` |
| bad placement of an owned type | `type/foreign_owned_type` |
| handle in a host entry | `type/entry_signature` (`pub fn`), `type/foreign_export` (`fn main`) |
| trusted declaration bypassing signatures | `type/foreign_signature` |
| effect operation in a contract | `type/foreign_contract` |
| `proc` on a `par` path | `type/parallel_effect` |
| unproved catalog requires | `verify/call_requires` |
| contract observer domain not met | existing `verify/contract_domain` |
| staging guard for unimplemented batches | `codegen/unsupported_type` |
| native output without the shim | `codegen/foreign_target` |

`proc_unconsumed` had no matches in the repository when this was designed; A0 re-runs the search once.
Runtime error values 1–6 are read only inside `proc_error` and proc terminal Results, so they do not clash
with compiler diagnostics or OS exit codes.

### 7.3 Compatibility

- Programs that do not use Process keep their ownership and drop meaning.
- Programs that do not use the new features generate identical Rust, LLVM and bindings.
- The unknown-effect hint change is an intended diagnostic change.
- An existing user struct named `Process` keeps its meaning.
- `File` is not redesigned.

## 8. Batches and gates

Each batch finishes the Python implementation and the Rust port together. New frontend, proof and runtime
rows must agree between the compilers. Generated text must be byte-identical for the same target and
options. Asynchronous chunk boundaries and cross-stream order are not compared; each stream's reassembled
bytes are. During a batch only that batch's tests run; the full suite and row comparison follow the
repository commit procedure.

### A0. Effect, types, must-consume

Files: `pruvc/vparser.py`, `pruvc/vast.py`, `pruvc/typeinfo.py`, `pruvc/check.py`, `pruvc/foreign.py`,
`pruvc/pruvc.py`, `C/parser.rs`, `C/typeinfo.rs`, `C/gtype.rs`, `C/check.rs`, `C/foreign.rs`,
`D/driver.rs`, `tests/run_all.py`, `tests/run_all_rows.txt`, new `tests/g10_proc/frontend/*.pruv`, new
`tests/must_reject/g10p_*.pruv` with `.json`, this document.

Work: effect, types, catalog signatures and effect registration; Process must-consume and the `par` effect;
a staging guard for unimplemented parts; the diagnostic name clash search.

Gates:

1. Moves, borrows, helper returns and the argument builder type-check.
2. Each of these is rejected with its diagnostic: double termination, use after move, consuming a borrow;
   consumed in one branch only, missing on an early return, a forgotten take parameter; overwriting a live
   owner, discarding a borrowed temporary, discarding an update result; a loop local left unconsumed, an
   outer owner not restored; placement in Result/struct/collection, host entry exposure; a missing `proc`
   effect, an indirect `par` effect.
3. Consumed in both branches, `p = update(move p)` and `return move p` are allowed.
4. Existing File auto-drop cases are still allowed.
5. Programs using Process stop at the staging guard in every mode. A function with only `! proc` is not
   guarded.
6. Frontend diagnostics (stage/code/location) and output agree between the compilers.

The A0 catalog contract placeholders can never run past the staging guard or be reported as proved.

### A1. Verification

Files: `pruvc/foreign.py`, `pruvc/verify.py`, `pruvc/pruvc.py`, `C/foreign.rs`, `P/symbolic.rs`,
`P/obligation.rs`, `D/driver.rs`, `tests/run_all.py`, `tests/run_all_rows.txt`, new
`tests/g10_proc/proof/*.pruv`, new `tests/must_reject/g10p_v_*.pruv` with `.json`.

Work: the §4 encoding and the full catalog contracts; observer domains, call-entry snapshots, havoc and
counterexamples; move the guard to the codegen entry, after verification.

Gates:

1. stdin write, close, read-all and termination after a confirmed spawn verify.
2. Paths that wait or kill a failed handle verify.
3. write/read without a confirmed spawn, write after close, bad stream/max/index are rejected.
4. Preservation of the other buffer on a single-stream read, and the exact copy relation of `proc_bytes`,
   verify.
5. `read_all` with EOF true and a non-empty buffer is consistent.
6. Claims that use an earlier `ok`, size or EOF after loop havoc without an invariant are rejected.
7. Claims that two identical spawns give equal results, claims about arbitrary output content, and
   unconditional exit code 0 are rejected.
8. Wrong observer domains in contracts are rejected.
9. A finite read-budget helper verifies. A loop claiming to decrease by EOF alone does not pass.
10. New obligations, conclusions and diagnostics agree between py and rs.

### A2. Rust runtime and source generation

Files: `pruvc/codegen.py`, `pruvc/foreign_codegen.py`, `pruvc/foreign.py`, `C/codegen.rs`,
`C/foreign_codegen.rs`, `C/foreign.rs`, `pruvc/pruvc.py`, `D/driver.rs`, `tests/run_all.py`,
`tests/run_all_rows.txt`, new `tests/g10_proc/child.rs`, new `tests/runtime/g10p_r*.pruv`, `.rs`, `.json`.

Work: the §5 runtime and helpers, snapshot copy; remove the Rust backend guard (the LLVM guard stays). The
fixture child executable is prepared in the test work directory; no real engine, account or Git install is
needed.

Gates:

1. Argument order, empty arguments, spaces, quotes, backslashes and Unicode arrive exactly.
2. Inherited and explicit cwd work, and the parent's cwd is unchanged.
3. A missing program, a bad cwd, NUL and initial setup failure are error values.
4. stdin echo with explicit EOF, empty writes, and partial write / broken pipe errors.
5. stdout and stderr reassembled across chunk boundaries equal the fixture's bytes.
6. A run finishes when stderr exceeds the pipe capacity and the parent reads stdout first.
7. A child that writes a lot to both outputs before reading stdin does not hang on pipe backpressure.
8. `read_all` does not repeat chunks already delivered.
9. Exit 0, a non-zero exit, killing a running child and killing an exited child are told apart.
10. Termination after a failed handle or an I/O failure cleans up endpoints, workers and the direct child.
11. Runtime requires violations exit 101 and are distinct from OS failures.
12. Generated Rust is identical between py and rs.

read, wait and kill errors and post-spawn setup failures that the OS cannot produce reliably are tested
through a fixture-only fault path. No fault option is exposed in the user API.

### A3. LLVM shim, Windows, user spec

Files: `pruvc/llvmgen.py`, `pruvc/foreign_shim.py`, `C/llvmgen.rs`, `C/foreign_shim.rs`, `D/driver.rs`,
`tests/run_all.py`, `tests/run_all_rows.txt`, `tests/runtime/g10p_r*.pruv`, `.rs`, `.json`,
`tests/g10_proc/child.rs`, new `tests/g10_proc/windows.py`, `SPEC.md`, this document. `pruvc/runtime.c`
does not change.

Work: Process and ProcessArgs adapters, the Result out-parameter adapter, borrowed bytes view plus v2 copy;
remove the LLVM guard; at most one page of proc user rules in `SPEC.md`.

Gates:

1. The A2 runtime cases pass on LLVM too.
2. `Result` Ok/Err and payloads are never swapped.
3. G6 live boxes are 0 after updates, branches, helper returns, wait and kill.
4. A `proc_bytes` copy stays valid after the handle is updated or consumed.
5. Observers that appear only in contracts are linked.
6. Generated IR and bindings are identical between py and rs.
7. The native output limit is `foreign_target` and does not conflict with the existing File limit.
8. On Windows, the real fixture checks argument quoting, Unicode, cwd, stdin EOF, both outputs, exit codes
   and kill.
9. Windows batch files, direct or found by search, are refused and never run through a shell.
10. The negative mapping of Unix signal exits is checked for real.

Building a Windows artifact does not count as passing the Windows run gate. Without a run environment, that
part stays pending and is recorded as such.

Record (2026-11-16): gate 1, all g10p runtime cases on LLVM with both compilers; gates 2 and 10, `g10p_r10`
(Unix signal exits are negative); gate 3, the G6 drop balance check; gate 4, `g10p_r21_copy_contract`; gate 5,
the same case, whose `proc_stdin_open` and `proc_eof` appear only in a contract and are defined in the
bindings; gate 6, `diff_compilers.py --backend llvm`; gate 7, `foreign_target`. Gates 8 and 9 ran for real on
Windows 11 (MSVC rustc 1.93.0) with `tests/g10_proc/windows.py`, 11/11 with either compiler, Rust backend; the
LLVM shim links the same runtime text. The run found that `x.bat.` and `x.bat ` (Windows drops trailing dots and
spaces) escaped the batch check and were then not found; the check now trims them first.

### Q. Quality

Files: new `bench/g10_proc/PROBLEMS.md`, `bench/g10_proc/solutions/q01.pruv`, `q02.pruv`,
`bench/g10_proc/run.py`, `mutate.py`, `perf.py`, `bench/g10_proc/faults/`, `bench/g10_proc/results/`,
`RESULTS.md`, `BLUEPRINT.md`, this document.

Problems:

1. A `--version`-style run: exact argv and cwd, stdin close, both outputs kept, the exit Result handled.
2. Line streaming: lines longer than a chunk, empty lines, a last line without LF, concurrent stderr, a
   finite budget and a kill on budget exhaustion. Byte-based line assembly; no UTF-8 conversion assumed.

Gates:

1. Both reference solutions pass on all four py/rs × rust/llvm combinations.
2. An independent author solves both from `SPEC.md` and the problems only; first result and fix count are
   recorded.
3. 16 explicit wrong mutants: missing consume, overwrite, branch, temporary, unchecked I/O, write after
   close, range errors, EOF misreading, read-all duplication, line boundary errors.
4. Static and contract mutants are caught at compile time; wrong handling of external output is caught by
   the fixture oracle. The two are reported separately; no claim that all external behavior is proved.
5. Four compiler faults are caught by named tests: removing must-consume, removing a catalog requires, a
   wrong snapshot preservation, an inverted Result tag.
6. One fixed workload moves 1 MiB total over both outputs in 4 KiB chunks with zero loss and duplication,
   and reports time and memory against hand-written Rust.
7. No performance target is declared without evidence. Per-byte shim cost and bulk copy cost are recorded
   separately.
8. Results and limits go into `RESULTS.md`; the proc status in `BLUEPRINT.md` is updated. Existing File
   measurements are not replaced by proc measurements.

Record (2026-11-16): all eight gates pass, see `bench/g10_proc/README.md` and RESULTS.md §125. Gate 1: 4/4 pairs.
Gate 2: independent author 2/2 first attempt, 0 fixes. Gates 3-4: 16 mutants, 12 static rejected at compile time, 4
output mutants caught by the fixture oracle (reported as `wrong_but_proved`), 0 unexpected. Gate 5: 4/4 faults caught.
Gates 6-7: 1 MiB with exact totals on both sides; bulk copy 0.98-1.01x and per byte 1.14-1.86x hand Rust, no target.

## 8b. Environment (G10-PROC-ENV)

Signature: `proc_spawn_env(program: str, args: ProcessArgs, cwd: str, env_names: ProcessArgs, env_values: ProcessArgs) -> Process ! proc, alloc`.
All parameters are borrowed; the returned handle carries the same must-consume duty as `proc_spawn`.

- **Order:** the child inherits the parent environment first; then the pairs `(env_names[i], env_values[i])` are set in
  list order. A new name is added; a name the parent already has is overridden. The parent's own environment never changes.
- **Repeated name:** the last pair wins.
- **No removal:** a pair always sets. An empty value is a set, empty variable (`NAME=`), not a removal. Removing or
  clearing variables is later work (§9).
- **Error 1 (bad input):** the counts of names and values differ; a name is empty; a name contains `=` or NUL; a value
  contains NUL. The result is a failed Process with `proc_error == 1`, exactly like an invalid `proc_spawn` input. The
  handle is still owned and must be consumed with `proc_wait` or `proc_kill` (`proc_wait` returns `Err(1)`). Program,
  argument and cwd errors are checked as in `proc_spawn`; an OS refusal is error 2.
- **Windows:** variable names are case-insensitive there; Rust `std::process::Command` handles that, the runtime adds
  no handling of its own.
- **PATH:** if the pairs set `PATH`, a bare program name is looked up the way Rust `std::process::Command` does it (on
  Unix the new `PATH`; on Windows the platform rules). An explicit path is used as is.
- **Verification:** the same contract as `proc_spawn` (§2.3): no requires; on success `ok(result) && open(result)`; on
  failure `!ok(result) && !open(result)`, error 1 or 2, buffers empty, EOFs false. Nothing is proved about the child's
  environment.
- **Lowering:** Rust and LLVM share one runtime function `pruv_proc_spawn_in`, used by both `proc_spawn` (no pairs) and
  `proc_spawn_env`; the Rust backend emits `pruv_proc_spawn_env`, the LLVM shim `pruv_g6_proc_spawn_env`.

Tests:

| Test | What it checks |
|---|---|
| `tests/runtime/g10p_r22_env` | inherited, new, overridden, repeated, empty and Unicode / quoted values, zero pairs, PATH lookup (Unix), error 1 for each invalid case with the failed handle consumed, parent environment unchanged; fixture child mode `env` |
| `tests/g10_proc/proof/spawn_env.pruv` | a root that verifies completely: failed-handle branch and a write / close / read-all / wait session |
| `tests/must_reject/g10p_env_unconsumed` | `type` / `proc_unconsumed`: a `proc_spawn_env` handle leaked on an early return |
| `tests/must_reject/g10p_v_env_unchecked` | `verify` / `call_requires`: `proc_write_str` on a `proc_spawn_env` result without `proc_ok` |
| fault `f5_env_ignored` | the runtime ignores the pairs; caught by `g10p_r22_env` |
| fault `f6_env_validation` | the `=` rule of names is dropped; caught by `g10p_r22_env` |

## 9. Later work

- User-facing async, poll, timeouts, cancellation and long-running engine sessions.
- Expressing an external event loop beyond a finite budget.
- Explicit memory limits, backpressure or spooling for output queues.
- Process groups, killing descendant trees, lifetimes of descendants that inherit pipes.
- Environment variable changes and explicit execution environments.
- PTYs, user signals, pipes between children.
- An incremental UTF-8 decoder and verified line / JSONL stream libraries.
- General string collections and richer error types.
- A C process runtime for LLVM without the Rust shim.
- Verifying real Lair engine protocols, run lifetimes and output volumes.
