# MCP stdio check port

Status: implementation landed; Q pending

## Goal

L3-3 ports the stdio `checkMcp` flow from Lair `src/backend/mcp-client.ts`, pinned to
`fa192269`. Only stdioWire, connectMcp, makeSession.tools, checkMcp and toolInfo are
in scope. Production integration and language changes are not part of this batch.

## Decisions

`root.pruv` exports `mcp_check(command: str, argv: str, cwd: str, timeout_ms: i64)
-> [i64] ! async, net, proc, alloc`. Each argv element ends in NUL; an empty blob means no
arguments. The Rust harness defaults the total limit to 15000 ms. Pruv owns the
protocol state machine, initialization messages, one old-protocol retry on an RPC
error, initialized notification, server request replies, page traversal, object
counting, name validation, field selection, description cutting and packet assembly.

Transport now lives in Pruv: args.pruv constructs ProcessArgs, root.pruv spawns
the server and registers one process and two timers in a Loop, and transport.pruv
pumps stdout/stderr, splits lines, enforces the byte cap, sends requests, observes
EOF and exit, and removes/closes the owned process. JSON parsing and numeric ID
matching are also Pruv. The copied tail decoder actively retains 1024 UTF-16 units.

The current SPEC/runtime has proc_ok/proc_error, not a proc_state intrinsic.
proc_error reports every OS spawn failure as 2 and does not expose ENOENT.
After error 2, Pruv asks the host command_missing predicate and maps a missing
command to reason 1 (ENOENT); otherwise it keeps reason 2. Bad argv input also
maps to 2. G10 and both compilers remain unchanged.

## Scope

New sources, Rust harness, host adapter and actual child-process fixtures live
under `bench/l3/mcp/`. The only other addition is this specification. HTTP, OAuth,
tools/call, outer cancellation, environment overlays and process groups are out
of scope. The child inherits the harness environment. No shell is involved.

## Boundary representation

The packet is a flat i64 array. Its 10-word header is:

| Offset | Meaning |
|---|---|
| 0 | ok, 0 or 1 |
| 1 | stage: 0 start, 1 connect, 2 list |
| 2 | reason: 0 success, 1 ENOENT, 2 other spawn, 3 total timeout, 4 init timeout, 5 long line, 6 exit, 7 I/O, 8 RPC error, 9 event limit |
| 3 | exit code, or negative Unix signal number |
| 4,5 | stderr last nonempty line reference (UTF-8 bytes widened to i64) |
| 6,7 | RPC message reference (UTF-16 units) |
| 8 | number of published tools |
| 9 | reserved |

The next 500 ten-word slots are tool records: name ref, title ref, description
ref, readOnly, destructive, raw-object ref. Each ref occupies two words
`(absolute packet offset, length)`; absent is `(-1,0)`. Flags are 0/1 and
`destructive=-1` means absent. All referenced payloads begin at offset 5010.
Decoded tool strings use UTF-16 code units. Raw JSON spans use UTF-8 bytes and
preserve the original tool object, including uncut description and unknown fields.
Failure publishes zero tools even if earlier pages accumulated records.

The host renders Korean messages, converts UTF-16 to JSON strings and applies
message whitespace normalization and length cuts. Pruv extracts the stderr last
nonempty line after Unicode trimming. Fingerprinting is
host-side from the raw object: the Rust harness invokes `fingerprint.cjs` once
per result, using Node's JSON.parse, sortedJson, JSON.stringify and SHA-256. This
preserves ECMAScript number formatting, integer-property enumeration, UTF-16 key
sorting and the reference's `__proto__` behavior. It returns the first 16 hex
digits. Node is an explicit harness dependency, not verified Pruv.

### Host responsibilities

host/src/lib.rs contains packet rendering and command_missing(command, cwd).
This trusted filesystem pre-check is consulted only after G10 reports error 2.
For commands without '/', it checks for an executable regular file in PATH,
resolving relative PATH entries against cwd (unset PATH uses /usr/bin:/bin).
For commands with '/', it checks existence of the path resolved against cwd.
This approximates Node's missing-command ENOENT: it is subject to a filesystem
race, does not model PATHEXT, and does not diagnose a missing cwd, interpreter
or loader for an existing executable. Executability uses Unix mode bits, not
ACL/effective-identity access checks. The predicate performs no process execution.
The harness's fingerprint.rs invokes Node for fingerprint.cjs on raw spans. serde_json remains for renderer/harness
JSON and fingerprint IPC, never for MCP transport or response matching.
There are no trusted application transport functions or host transport threads.
Process/Loop OS behavior remains trusted through the standard G10 runtime.

## Policies

- Requests start at 1 and advance through at most 7 (two initialization requests
  and five pages). Only matching numeric replies resolve the pending request.
  A server request takes precedence over a same-id reply. Ping receives an empty
  result; other methods receive -32601 and the reference Korean message.
- Initialization uses 2025-11-25 then, only on an RPC error, 2025-06-18. Its limit
  is 30000 ms combined with the total deadline. Both attempts share the init limit.
- Up to five pages and 500 object items are collected. Invalid names still count
  toward the object cap. Empty/nonstring cursors stop paging; wrong result/tools
  types behave as empty pages.
- The line limit is 4 MiB of incoming UTF-8 **bytes**, including CR before LF,
  rather than TS UTF-16 units. Oversized partial lines fail without waiting for LF.
  Unterminated EOF fragments are ignored. Invalid UTF-8 stdout lines are rejected by the Pruv JSON parser.
  Stderr uses the copied streaming WHATWG decoder and keeps 1024 UTF-16 units;
  a leading low surrogate cut from its pair is dropped by tail_bytes.
- Description is at most 65536 UTF-16 units and never splits a surrogate pair.
  Name is 1..200 units and excludes U+0000..001F, U+007F..009F, U+2028 and U+2029.
  Lone surrogate strings in JSON are preserved in the packet but the Rust JSON
  renderer replaces lone surrogates with U+FFFD. Fingerprints use original JSON.
- Close kills and waits for the directly spawned child immediately, equivalent
  to the supervisor's ev_remove policy; no 2-second TERM/3-second KILL grace.
  Descendant process-group termination is not supplied by this harness.
- JSON parser limits are inherited: 16 MiB source, depth 128 and 1048576 tape
  records. Resource-rejected lines are ignored like malformed lines. Numeric ID
  equality is exact decimal equality in Pruv for IDs 1..7, including fractional
  and exponent spellings. Unlike JS f64, near-integer decimals that round to an
  ID are not accepted.
- There is a one-million-pump-iteration limit, reported as a failure, never a
  truncated success. Host timeout inputs are clamped to 0..86400000 ms.

## Proof targets

Both compilers verify the root module graph. Local contracts prove array safety,
loop progress, page/tool bounds, request-id bounds, the accepted name predicate
and description cut lengths. The root promises a minimum packet size. Packet
references and all externally reconstructed fields are checked by the Rust
harness and fixtures; full packet semantic invariants are not claimed formally
proved. Monotonic IDs and surrogate-safe cutting are exercised behaviorally in
addition to local bounds. The G10 runtime, host renderer, Node, compiler,
allocator and solver remain trusted. No whole-program TS equivalence, timing,
allocation success, OS scheduling or process-tree cleanup proof is claimed.

## Batches

One implementation batch: Pruv root and packet helpers; copied JSON/UTF-8 modules;
G10 transport and Rust renderer; fake stdio servers; four-pair fixture driver. No
language changes or edits to other L3 ports. Q is separate and pending.

## Q reference and oracle

Pinned reference: Lair fa192269, src/backend/mcp-client.ts. Fixed fixtures assert
handshake wire messages inside real child processes and compare the host output
to independent expected results. Both Python/Rust compilers and Rust/LLVM
backends run the same fixtures. This is implementation testing, not Q. Future Q
should run the unchanged pinned TS against the same servers, add randomized
protocol traces and targeted mutants, and classify the explicit Unicode, resource,
close policies without masking unrelated mismatches.

## Deferred

Full Q differential/mutation/performance campaign; process-group integration;
HTTP/authentication; outer abort; packaging into Lair. The native 30-second initialization
limit requires a slow fixture and is not shortened by a testing-only switch.

## Risks

G10 ev_proc_write writes synchronously and may block; timers cannot interrupt a
blocked write. Parsing/packing can also delay deadline observation. The pump polls
timers during noise and again before publication, but scheduling and a queued
event can delay observation. G10 worker queues are not memory-bounded against an
unlimited producer. No process-group termination or malicious-descendant cleanup
guarantee is claimed.

## Implementation evidence

The fixture driver preserves all 22 original expected outputs and reports each
pair even when a fixture assertion fails. Both compilers/backends verified and built successfully: py/rust 22/22,
py/llvm 22/22, rs/rust 22/22, rs/llvm 22/22. The missing-command fixture now
passes with its original ENOENT wording. Only this command is used:

```
PATH="$HOME/.cargo/bin:$PATH" python3 bench/l3/mcp/test_impl.py
```

No full repository suite, Q campaign, native 30-second init-timeout wait or
malicious-descendant cleanup experiment is included. Unix signals still render
numeric identifiers rather than Node's symbolic names.
