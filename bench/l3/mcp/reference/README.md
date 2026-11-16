# Pinned MCP reference

`backend/mcp-client.ts` is a byte-for-byte copy of
`git -C ~/Projects/lair show fa192269:src/backend/mcp-client.ts`.
The entire source is retained; the driver only calls the stdio `checkMcp` path.
`hooks.mjs` resolves extensionless TS imports. `shared/i18n/wire.ts` supplies
metadata/error wrappers and preserves original Korean fallback messages;
`backend/windows-system.ts` throws if the out-of-scope Windows path is used.
Type-only contracts imports are erased by Node's type stripping.

`../reference.mjs` accepts the Rust harness's JSON-lines requests. It removes
only `messageI18n`, which the Pruv packet interface does not represent. All other
result JSON, including fingerprints and optional-field presence, is compared.
No source instrumentation, clock substitution or transport mocking is used.

Run on `/opt/homebrew/bin/node` (tested v26.4.0). No dependencies or network.
