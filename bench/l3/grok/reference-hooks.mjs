// Module loader hooks for the pinned Grok reference (DESIGN §5). Run as
//   GROK_TS_TRANSPILER=/path/to/typescript node --experimental-strip-types --import ./reference-hooks.mjs reference.mjs
// Like reference/hooks.mjs (stubs, shared-engine-import, extensionless imports, types stripped by Node) plus:
//  * node:fs, node:fs/promises, node:crypto imported by the pinned grok.ts / tail.ts -> reference-primitives.mjs
//  * node:child_process imported by stubs.ts -> reference-primitives.mjs (spawn trace)
// Nothing else is redirected (opencode.ts and the shim's own imports use the real builtins).
// plan.ts uses a constructor parameter property, which strip-only mode rejects; only that file is transpiled,
// with the TypeScript package named by the explicit environment variable GROK_TS_TRANSPILER (a path to the
// `typescript` package directory or its typescript.js). There is no built-in default path.
import { registerHooks, createRequire } from 'node:module';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { resolve as resolvePath } from 'node:path';

const reference = name => new URL(`./reference/${name}`, import.meta.url).href;
const primitives = new URL('./reference-primitives.mjs', import.meta.url).href;
const stub = reference('stubs.ts');
const plan = reference('plan.ts');
const pinned = new Set([reference('grok.ts'), reference('tail.ts')]);
const shimmed = new Set(['node:fs', 'node:fs/promises', 'node:crypto']);
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
    if (shimmed.has(specifier) && pinned.has(parent)) return next(primitives, context);
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
