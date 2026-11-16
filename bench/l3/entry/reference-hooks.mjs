// Module loader hooks of the pinned entry reference (DESIGN section 6). Run as
//   node --experimental-strip-types --import ./reference-hooks.mjs reference.mjs < cases.jsonl
// Redirections, keyed by (importing pinned file, specifier). Nothing pinned is copied or edited; everything not listed loads real.
//  * engine-import.ts : node:os, ./accounts, ./codex-home, ./engine-import/{claude,codex,grok,opencode,qwen} (fakes), ./engine-import/tail
//  * codex-home.ts    : node:fs (realpathSync/statSync) and node:os -> primitives; ./accounts; ./engine-path, ./provider-accounts,
//                       ./wire-reason -> fakes/stubs.ts; ../shared/i18n/wire -> fakes/wire.ts. (codex-owned-runner and
//                       engine-contract are `import type`, erased by strip-types.)
//  * accounts.ts      : ../shared/contracts -> the ENGINE_IDS stub (equal to shared-contracts.ts:17; the real file re-exports
//                       ./engine-model, which is not pinned, so it cannot load; ORACLE_CONTRACTS=real forces it);
//                       ../shared/i18n/wire -> fake
//  * tail.ts          : node:child_process, ../engine-children, ../windows-system -> fakes/stubs.ts
//  * grok opencode.ts : node:os -> primitives; ../engine-children, ./plan -> stubs; ./tail -> the entry tail (one EngineImportError)
//  * entry qwen.ts    : ./opencode -> the pinned grok opencode.ts; ./tail -> entry tail; ./plan -> stubs
// The flat pinned filenames (backend-*.ts) need the explicit extensionless-to-flat mapping below.
import { registerHooks } from 'node:module';

const here = file => new URL(file, import.meta.url).href;
const ref = name => here(`./reference/${name}`);
const primitives = here('./reference-primitives.mjs');
const stubs = here('./fakes/stubs.ts');
const wire = here('./fakes/wire.ts');
const fake = name => here(`./fakes/engine-${name}.ts`);

const E = ref('backend-engine-import.ts');
const C = ref('backend-codex-home.ts');
const A = ref('backend-accounts.ts');
const T = ref('backend-engine-import-tail.ts');
const Q = ref('backend-engine-import-qwen.ts');
const O = here('../grok/reference/opencode.ts');
const contracts = process.env.ORACLE_CONTRACTS === 'real' ? ref('shared-contracts.ts') : stubs;

const rules = new Map([
  [E, {
    'node:os': primitives,
    './accounts': A,
    './codex-home': C,
    './engine-import/claude': fake('claude'),
    './engine-import/codex': fake('codex'),
    './engine-import/grok': fake('grok'),
    './engine-import/opencode': fake('opencode'),
    './engine-import/qwen': fake('qwen'),
    './engine-import/tail': T,
  }],
  [C, {
    'node:fs': primitives,
    'node:os': primitives,
    './accounts': A,
    './engine-path': stubs,
    './provider-accounts': stubs,
    './wire-reason': stubs,
    '../shared/i18n/wire': wire,
  }],
  [A, {
    '../shared/contracts': contracts,
    '../shared/i18n/wire': wire,
  }],
  [T, {
    'node:child_process': stubs,
    '../engine-children': stubs,
    '../windows-system': stubs,
  }],
  [O, {
    'node:os': primitives,
    '../engine-children': stubs,
    './tail': T,
    './plan': stubs,
  }],
  [Q, {
    './opencode': O,
    './tail': T,
    './plan': stubs,
  }],
]);

registerHooks({
  resolve(specifier, context, next) {
    const table = context.parentURL === undefined ? undefined : rules.get(context.parentURL);
    const target = table?.[specifier];
    if (target !== undefined) return next(target, context);
    return next(specifier, context);
  },
});
