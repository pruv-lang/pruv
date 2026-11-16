# Pinned Codex oracle

`codex.ts`, `tail.ts`, `plan.ts` and `opencode.ts` are verbatim `git show 744d14af:src/backend/engine-import/<name>` copies from Lair. `shared-engine-import.ts` is a verbatim `git show 744d14af:src/shared/engine-import.ts`. Original comments remain in their original language.

SHA-256:

| file | sha256 |
|---|---|
| codex.ts | `19e07785fa085e60c78937730aef76dd00d1d0a99b80ad08008b4fb4008dfdb8` |
| tail.ts | `750399578ca53bcf6ccf0b2b03863de08d0f16a3580c60f40d9ef8c216c3c14e` (same as the qwen copy) |
| plan.ts | `e432a9988c50b8f6500cb01db5c31816ad86cf30f64a9ee2c7d56a67afd4bda5` |
| opencode.ts | `985a4a9d5e79a97278e0ab7050b673977dde1230a968b9a448fe0f6d624804a0` |
| shared-engine-import.ts | `89cd7c0acb6cee4fb2534f3a9a7fffa2032de5e3eba7a9231bfa9e8851a1d583` |

Node 26.4.0 (`/opt/homebrew/bin/node --experimental-strip-types --import ./hooks.mjs`) executes them. `hooks.mjs`:

- adds `.ts` to extensionless relative imports and maps `../../shared/engine-import` to the local copy;
- redirects `../engine-children`, `../windows-system`, `../accounts`, `../codex-engine` and `../codex-home` to `stubs.ts`;
- transpiles `plan.ts` alone with Lair's own TypeScript 5.7.3 (`transpileModule`, ESNext module, ES2022 target), because its constructor parameter property is rejected by strip-only mode. The other files are only stripped.

`stubs.ts`: on non-win32, Lair's `spawnEngine` is `spawn(command, args, options)` plus a process count that only feeds the UI, so the stub is exactly that `spawn`. Account, install and Windows helpers throw; the driver passes `options.codex = {binary, env, args}` directly instead of `makeCodexRunDeps`.

The oracle spawns the fake app-server `../q_server.py` with a scenario file, never a real Codex, and never touches the network or the Lair application. Both the oracle and the Pruv hosts speak to the same fake server, which records every request it receives so that requests are compared as well as results.
