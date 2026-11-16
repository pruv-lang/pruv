// Runs the pinned TypeScript listCodex / readCodex against the fake app-server, one JSON request per stdin line,
// one JSON result per stdout line. Run as:
//   /opt/homebrew/bin/node --experimental-strip-types --import ./reference/hooks.mjs reference.mjs < requests.jsonl
// Request: {"mode":"list"|"read","binary","args":[...],"cwd","folder","thread","timeout"}.
// Result: the ImportListItem array / ExternalRecord as JSON, or {"error":"<EngineImportError code>"}.
import { createInterface } from 'node:readline';
const { listCodex, readCodex } = await import('./reference/codex.ts');
const { EngineImportError } = await import('./reference/tail.ts');
for await (const line of createInterface({ input: process.stdin })) {
  if (!line.trim()) continue;
  const v = JSON.parse(line);
  const options = {
    signal: new AbortController().signal,
    codex: { binary: v.binary, env: { ...process.env }, args: v.args ?? [] },
    timeoutMs: v.timeout ?? 20000,
  };
  let out;
  try {
    const source = { source: 'q', engine: 'codex', home: '', kind: 'home' };
    out = v.mode === 'list' ? await listCodex(source, v.folder ?? '', options) : await readCodex(source, v.folder ?? '', v.thread ?? '', options);
  } catch (error) {
    out = { error: error instanceof EngineImportError ? error.code : `internal: ${error instanceof Error ? error.message : String(error)}` };
  }
  console.log(JSON.stringify(out));
}
// exit only after stdout drains: a bare exit cuts a large last result short on a pipe
process.stdout.write('', () => process.exit(0));
