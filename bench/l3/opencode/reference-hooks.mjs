// Module loader hooks for the pinned OpenCode reference (DESIGN section 6). Run as
//   GROK_TS_TRANSPILER=/path/to/typescript node --experimental-strip-types --import ./reference-hooks.mjs reference.mjs
// Resolves imports into ../grok/reference/ (the pin, never copied or edited):
//  * opencode.ts: node:fs/promises, node:os, node:sqlite  -> reference-primitives.mjs (FS / homedir / DatabaseSync facades)
//  * tail.ts:     node:fs/promises, node:fs               -> reference-primitives.mjs (plainFileIn's lstat/realpath)
//  * stubs.ts:    node:child_process                      -> reference-primitives.mjs (spawn trace + event hooks)
//  * '../engine-children' etc. (unrelated engine imports) -> the pinned stubs.ts; '../../shared/engine-import' -> the pinned shim
// Nothing else is redirected: this shim's own builtin imports, plan.ts's imports and the pinned stubs stay real.
// plan.ts has a constructor parameter property that strip-only mode rejects, so ONLY that file is transpiled, with the
// TypeScript package named by the explicit environment variable GROK_TS_TRANSPILER (a path to the `typescript` package
// directory or its typescript.js). There is no built-in default path.
import { registerHooks, createRequire } from 'node:module';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { resolve as resolvePath } from 'node:path';

const reference = name => new URL(`../grok/reference/${name}`, import.meta.url).href;
const primitives = new URL('./reference-primitives.mjs', import.meta.url).href;
const stub = reference('stubs.ts');
const plan = reference('plan.ts');
const opencode = reference('opencode.ts');
const tail = reference('tail.ts');
// parent module -> the builtin specifiers it may have redirected
const shimmed = new Map([
  [opencode, new Set(['node:fs/promises', 'node:os', 'node:sqlite'])],
  [tail, new Set(['node:fs/promises', 'node:fs'])],
]);
const stubbed = new Set(['../engine-children', '../windows-system', '../accounts', '../codex-engine', '../codex-home']);

let ts;
function transpiler() {
  if (ts) return ts;
  const spec = process.env.GROK_TS_TRANSPILER;
  if (!spec) throw new Error('GROK_TS_TRANSPILER is not set: point it at the typescript package (needed only to transpile plan.ts)');
  ts = createRequire(import.meta.url)(spec.startsWith('.') ? resolvePath(spec) : spec);
  return ts;
}

registerHooks({
  resolve(specifier, context, next) {
    const parent = context.parentURL;
    if (shimmed.get(parent)?.has(specifier)) return next(primitives, context);
    if (specifier === 'node:child_process' && parent === stub) return next(primitives, context);
    if (stubbed.has(specifier)) return next(stub, context);
    if (specifier === '../../shared/engine-import') return next(reference('shared-engine-import.ts'), context);
    if (specifier.startsWith('.') && !/\.[cm]?[jt]s$/.test(specifier)) return next(specifier + '.ts', context);
    return next(specifier, context);
  },
  load(url, context, next) {
    if (url !== plan) return next(url, context);
    const ts = transpiler();
    const out = ts.transpileModule(readFileSync(fileURLToPath(url), 'utf8'), {
      compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022, useDefineForClassFields: true },
    });
    return { format: 'module', source: out.outputText, shortCircuit: true };
  },
});
