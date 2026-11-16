# Qwen conversation import

Status: implementation landed; Q pending

## Goal

Port the pure Qwen conversation importer from Lair commit fa192269 into Pruv, without language changes. Sources live only in bench/l3/qwen; the Claude modules are unchanged. The Rust host supplies file bytes and renders JSON.

## Decisions

Use local copies of the L3-1 parser, accessors, text, calendar, framing, boundary and collector. Parse object roots only; own-property lookup is last-wins. Do not retain JSON documents across lines.

Qwen requires currentChain before turn collection. chain.pruv first finds the last nonempty UUID on user/assistant/tool_result, falling back to the last UUID on any entry. It then follows parentUuid through last-wins UUID lookup, marks every entry with a selected UUID, stops on cycles, and sets cut on a missing parent. The initial parent comes from the actual leaf, even when a later metadata entry duplicates its UUID. Repeated scans and a byte-position bitmap keep the implementation simple; their cost is not claimed to match the TS Map implementation.

Project naming supports the requested non-win32 platform: each non-ASCII-alphanumeric UTF-16 unit becomes '-'. Supplementary characters yield two hyphens. Names input is valid UTF-8; valid_thread rejects everything outside the ASCII filename grammar.

## Scope

| Reference | Implementation |
|---|---|
| qwenProjectName, validThread | names.pruv, root wrappers |
| humanText | entry.pruv: user, provenance missing or real_user, non-thought string parts joined with two LF bytes |
| droppedUser | text.pruv: both raw unanchored type:user and provenance:real_user patterns |
| readQwenHead | import.pruv::read_head: first string cwd, first untrimmed nonblank human prompt |
| tailInfo | import.pruv::tail_info: last accepted timestamp, last nonblank system/custom_title payload |
| currentChain | chain.pruv |
| readQwen | import.pruv::read_window and host.rs record assembly |
| TurnCollector | turns.pruv: bounded ring, saturated reply budget, suffix selection |

Thought means literal true only. Non-string parts are ignored. Assistant text takes precedence over functionCall when both exist; only object functionCall counts as a tool. Model is the last nonempty string at entry.model, including assistants without a message or a current draft. Human prompts begin drafts before assistant processing.

Titles are collected from every admitted object, independently of chain membership. The host combines window title with one_line(head.firstPrompt,120), calling the Pruv title_bytes wrapper. Tail title is used for list results, not as a window-title override.

Dropped markers are slots only when the raw prefix matches both patterns. A marker calls skip_user only from immediately before the first chain member onward. Invalid/nonobject lines and nonhuman oversized lines are not slots. The implementation uses pending-marker and first-member state to reproduce that rule without exposing slots to the host.

### Host responsibilities

host.rs implements synchronous non-win32 directory discovery under home/projects/projectName/chats, *.jsonl filtering, validThread calls, lstat regular-file and directory checks, realpath containment, opened-file device/inode checks on Unix, mtime descending order, folderKey trailing-slash comparison (preserving all-slash roots), limit 50 after filtering, maybeOpen for mtime younger than 120000 ms, and mtime ISO fallback through the Pruv timestamp formatter.

It reads head/tail/window bytes from one opened descriptor and supplies truncation flags. Its --list mode renders list JSON; positional fixture paths render head, tail and record JSON. --names exercises the path-name functions. It is a benchmark host, not a deployed Lair adapter. Cancellation, asynchronous parallelism, translated error mapping and production read-race recovery are not implemented; filesystem failures fail the harness invocation. It does not authorize arbitrary paths based on Pruv output. Direct positional paths are test inputs, whereas --list performs home containment checks.

## Boundary representation

Root public functions return plain arrays. head_packet(data,cut_after), tail_packet(data,cut_before), window_packet(data,cut_before) return:

- success: [0,14,turn_slots,byte_len,...meta,...turns,...text_bytes]
- failure: [code,position], with 1 InputTooLarge or 2 InternalInvariant

No strings cross as pointers; every reference is a (start,length) byte range. Absent is (-1,0), distinct from present empty.

| Meta slots | Meaning |
|---|---|
| 0 | Schema version 1 |
| 1 | Mode 0 head, 1 tail, 2 window |
| 2 | omittedBefore, 0/1 |
| 3 | Turn count |
| 4–5 | Head firstPrompt (untrimmed); tail/window title |
| 6–7 | Head cwd |
| 8–9 | Reserved absent; no Qwen entrypoint |
| 10–11 | Tail lastAt |
| 12–13 | Window model |

Head human is exactly the presence of firstPrompt. Each turn has ten slots: prompt ref, reply ref, createdAt ref, completedAt ref, tools, clipped. Optional timestamps are absent or 24-byte ISO strings. Hosts validate the packet's exact size, version, byte range and reference ranges.

Limits: head and tail 65536 bytes, window 8388608, line 1048576, prompt/reply 32768 each, turns 200, retained prompt+reply total 2097152. JSON depth 128 and tape records 1048576 are inherited. Oversized input arrays fail rather than silently truncate.

Head discards an unfinished edge fragment when cut_after; tail skips its first range when cut_before and accepts a genuine final unterminated range. Window skips its first range when cut_before and accepts only LF-terminated ordinary lines. An oversized final unterminated line still emits a dropped event. CR counts toward the raw line limit. Only the first 8192 bytes of an oversized line enter the raw heuristic.

omittedBefore = cut_before || collector.trimmed || chain.cut. Malformed or oversized lines alone do not imply omission. A skipped user detaches the current draft and marks the next draft clipped. Replies update completion time even when empty or saturated. Tools do not update time. Finish retains the newest contiguous suffix under count and byte caps.

Additional roots: valid_thread returns 0/1; project_name and title_bytes return UTF-8 bytes. Host title fallback stays in Pruv via title_bytes.

## Policies (differences from TS)

- P1: strict UTF-8 JSON decoding and bounded parser depth/tape; invalid lines are skipped. Raw dropped-prefix whitespace scanning is independent of strict JSON decoding.
- P2: lone surrogate values become U+FFFD, keys retain exact UTF-16 identity. one_line uses ECMAScript whitespace and 120 UTF-16 units, omitting a scalar split by the final cut.
- P3: byte clipping keeps genuine trailing U+FFFD, unlike TS's replacement-character stripping. Reply budget still saturates at 32768 after a partial append.
- P4: iso_of accepts safe lexical integer numbers, unsigned digit strings after ECMAScript trim, and exact canonical UTC YYYY-MM-DDTHH:mm:ss.sssZ with Gregorian validation, years 0000–9999. Integers below 100000000000 are seconds (signed comparison), otherwise milliseconds. Fractional/exponent forms, offset/local dates, signed numeric strings and calendar overflow are absent. No native Date.parse fallback.
- P5: bounded pure byte API and unpolluted standard runtime assumptions; host path/list/filesystem responsibilities and plan tracking are outside verified output. Head slot 4 contains the full firstPrompt rather than Claude's shortened title. Windows project naming and folder comparison are outside this implementation's platform scope.

These are the L3-1 policy decisions, not a claim of full JS Date or Unicode-string equivalence.

## Proof targets

Both compilers verify the reachable root/module contracts, including memory access, loop termination, collector bounds and publication shape. Inherited semantic contracts cover selection helpers, UTF-8 clipping and collector transitions. Qwen chain membership and full TS equivalence are fixture-tested, not claimed as a global mechanized theorem. The bitmap scan has bounded loops but no linear-time cost claim.

## Batches

One implementation batch: copied primitives, Qwen classification, chain selection, collector integration, Rust host, JSONL fixtures and four-pair tests. No compiler, language, Claude source or global test changes.

Run only:

    PYTHONDONTWRITEBYTECODE=1 /Users/spica4345/Projects/pruv/.venv/bin/python bench/l3/qwen/test_impl.py

The test uses installed local compilers and an offline rustc-built UTF-8 representation adapter. It compiles root.pruv with verification enabled for py/rust, py/llvm, rs/rust and rs/llvm, renders fixture JSON and compares checked-in expected results. Build artifacts stay in bench/l3/qwen/work.

Implementation verification: all four pairs passed verification and 24 original checks
(20 transcript fixtures, three name checks, one listing scenario). After adding the
mtime fallback/recent-file scenario, the targeted command
`test_impl.py --host-policy` passed its additional check on all four existing runners.
Combined result: 25/25 per pair. The default driver now includes that check.
The Python/Rust host was rebuilt after adding opened-file identity checks; the other
three hosts were initially built with those checks.

## Q reference and oracle

Pinned reference: Lair fa192269, src/backend/engine-import/qwen.ts, tail.ts, opencode.ts and plan.ts (plan is deferred). Read with git show; do not use an evolving checkout as the oracle.

Current implementation tests use explicit expected JSON for normal conversations, thought/tool behavior, provenance, custom title, branch cut, duplicate UUIDs, cycles, missing UUID fallback, oversized dropped markers, invalid timestamps, turn/window trimming, clipping and malformed/final fragments. Names and host discovery/filtering are separately asserted.

Q remains pending: execute a differential oracle against the pinned TS importer; project away lastPlan; explicitly account for P1–P5 rather than treating them as blanket mismatch exemptions. Measure repeated-scan time/memory on long UUID chains and add fault injection and mutants in Q.

## Deferred

lastPlan/qwenPlan from plan.ts: no plan field is emitted. Production Node bindings, Windows host behavior, asynchronous cancellation/concurrency, and robust concurrent-file mutation recovery are deferred. No deployment is performed.

## Risks

Chain selection reparses the window for each ancestor and copies the bitmap for cycle detection; this can be substantially slower than the TS UUID Map. Deep chains need Q measurements before production integration. The pure packet cannot prove host file snapshot identity. A live file can grow or change after metadata checks. Policy differences can affect title bytes, timestamps, retained suffixes and filename collision behavior; equality outside the documented shared domain is not asserted.

