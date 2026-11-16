# Pinned Grok oracle

`grok.ts`, `tail.ts`, `plan.ts` and `opencode.ts` are verbatim `git show 744d14af:src/backend/engine-import/<name>`
copies from Lair. `shared-engine-import.ts` is a verbatim `git show 744d14af:src/shared/engine-import.ts`. Lair's
`src/backend/engine-import/` and `src/shared/engine-import.ts` are unchanged from 744d14af to bd602412 (2026-11-16),
so L3-6 pins the same commit as L3-5. Original comments remain in their original language.

`hooks.mjs` and `stubs.ts` are the L3-5 (`bench/l3/codex/reference/`) copies, unchanged.

SHA-256:

| file | sha256 |
|---|---|
| grok.ts | `65e5bd61d32d7226736ae3049e76215a0d2d37015199834e2183afaa9b8642e2` |
| tail.ts | `750399578ca53bcf6ccf0b2b03863de08d0f16a3580c60f40d9ef8c216c3c14e` (same as the qwen and codex copies) |
| plan.ts | `e432a9988c50b8f6500cb01db5c31816ad86cf30f64a9ee2c7d56a67afd4bda5` |
| opencode.ts | `985a4a9d5e79a97278e0ab7050b673977dde1230a968b9a448fe0f6d624804a0` |
| shared-engine-import.ts | `89cd7c0acb6cee4fb2534f3a9a7fffa2032de5e3eba7a9231bfa9e8851a1d583` |
| stubs.ts | `903507e9d24035160faea464502076f1471353f1600ec2a295a13df7906d948d` |

Node 26.4.0 runs them with `/opt/homebrew/bin/node --experimental-strip-types --import ./hooks.mjs` (see the codex
reference README for what the hooks do). A load check imports `grok.ts` and lists its exports:
`copyGrokSession, grokLoadCheck, listGrok, readGrok, readSmallJson, removeGrokCopy`.

Callers at 744d14af: `requests/engine-import.ts` (`engine-import-open`: `readExternal`, then for Grok
`copyGrokSession(..., home => grokLoadCheck(...))`, and `rm` of the copy on a race or failure),
`migrate/work-undo.ts` (`removeGrokCopy`) and `migrate/work-scan-engines.ts` (`readSmallJson`).
