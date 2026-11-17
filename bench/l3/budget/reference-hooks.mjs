// Module loader hooks of the L3-10 request-budget oracle (docs/REQUEST_BUDGET_SPEC.md section 7.1). Run as
//   GROK_TS_TRANSPILER=/path/to/typescript node --experimental-strip-types --no-warnings --import ./reference-hooks.mjs reference.mjs [--time [--repeat N]] < scenarios.jsonl > observations.jsonl
// What this file does:
//  * Verifies the SHA-256 of every pinned file listed in reference/README.md at startup (exit 2 on any mismatch or missing file).
//  * Transpiles the pinned .ts files with the TypeScript package named by GROK_TS_TRANSPILER (the class B uses a constructor
//    parameter property, which Node's strip-only mode cannot run). compilerOptions: module ESNext, target ES2022, useDefineForClassFields.
//  * Redirects imports keyed by importing file (see `rules`); node:crypto -> fakes/node-crypto.ts (scenario UUID stream).
//  * Never loads backend-budget-wiring.ts (it is pinned and hashed as evidence only).
import { registerHooks, createRequire } from 'node:module';
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { resolve as resolvePath } from 'node:path';

const here = file => new URL(file, import.meta.url).href;
const ref = name => here(`./reference/${name}`);
const fake = name => here(`./fakes/${name}`);

// ---- pin verification -------------------------------------------------------------------------------------------------------
function die(message) {
  process.stderr.write(`reference-hooks: ${message}\n`);
  process.exit(2);
}

const manifest = readFileSync(new URL('./reference/README.md', import.meta.url), 'utf8');
const pins = [...manifest.matchAll(/^\| `([^`]+)` \| `[^`]*` \| `([0-9a-f]{64})` \|\s*$/gm)].map(m => ({ file: m[1], sha: m[2] }));
if (pins.length !== 8) die(`expected 8 pinned files in reference/README.md, found ${pins.length}`);
for (const pin of pins) {
  let bytes;
  try { bytes = readFileSync(new URL(`./reference/${pin.file}`, import.meta.url)); } catch (error) { die(`cannot read pinned file ${pin.file}: ${error.message}`); }
  const actual = createHash('sha256').update(bytes).digest('hex');
  if (actual !== pin.sha) die(`SHA-256 mismatch for ${pin.file}: expected ${pin.sha}, got ${actual}`);
}

// ---- transpiler -------------------------------------------------------------------------------------------------------------
const spec = process.env.GROK_TS_TRANSPILER;
if (!spec) die('GROK_TS_TRANSPILER is not set: point it at the typescript package (directory or typescript.js)');
const ts = createRequire(import.meta.url)(spec.startsWith('.') ? resolvePath(spec) : spec);

const WIRING = ref('backend-budget-wiring.ts');
const B = ref('backend-request-budget.ts');
const T = ref('shared-request-budget.ts');
const L = ref('shared-schedule-lists.ts');
const E = ref('shared-engine-text.ts');
const CORE = ref('shared-i18n-core.ts');
const SUB = ref('shared-sub-session-types.ts');
const REPLY = ref('shared-reply-blocks.ts');
// Every pinned file except the wiring is transpiled (they are all loaded for real).
const transpiled = new Set([B, T, L, E, CORE, SUB, REPLY]);

// Redirects keyed by importing file and specifier. Type-only imports ('./contracts', './core-types', '../shared/i18n/wire') are erased by
// the transpile; the wire entry only exists so that a surviving import would resolve to something harmless.
const rules = new Map([
  [B, {
    'node:crypto': fake('node-crypto.ts'),
    '../shared/request-budget': T,
    '../shared/engine-text': E,
    '../shared/schedule-lists': L,
    '../shared/i18n/wire': fake('node-crypto.ts'),
  }],
  [T, { './schedule-lists': L }],
  [E, { './i18n/core.js': CORE, './sub-session-types': SUB, './reply-blocks.js': REPLY }],
]);

registerHooks({
  resolve(specifier, context, next) {
    const table = context.parentURL === undefined ? undefined : rules.get(context.parentURL);
    const target = table !== undefined && Object.hasOwn(table, specifier) ? table[specifier] : undefined;
    const result = next(target ?? specifier, context);
    if (result.url === WIRING) throw new Error('backend-budget-wiring.ts must never be loaded');
    return result;
  },
  load(url, context, next) {
    if (!transpiled.has(url)) return next(url, context);
    const path = fileURLToPath(url);
    const out = ts.transpileModule(readFileSync(path, 'utf8'), {
      fileName: path,
      compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022, useDefineForClassFields: true },
    });
    return { format: 'module', source: out.outputText, shortCircuit: true };
  },
});
