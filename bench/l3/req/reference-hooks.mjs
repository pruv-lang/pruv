// Module loader hooks of the L3-9 request-handler oracle (SCENARIO.md section 7). Run as
//   node --experimental-strip-types --no-warnings --import ./reference-hooks.mjs reference.mjs < scenarios.jsonl
// The pinned handler (reference/backend-requests-engine-import.ts) and the pinned L3-8 entry/bound/tail/accounts/codex-home files
// load for real; nothing pinned is copied or edited. Redirections (keyed by importing file and specifier):
//  * handler R : node:fs/promises (rm) and node:crypto (randomUUID) -> req fakes; ../../logic/records -> pinned req logic-records.ts;
//                ../../shared/i18n/wire -> entry fakes/wire.ts (one WireError class); ../engine-import -> fakes/engine-import.ts
//                (real importSources/findSource/listExternal/readExternal/EngineImportError/IMPORT_LIMITS of the entry file + fake
//                copyGrokSession/grokLoadCheck/makeCodexRunDeps); ../engine-import/bound -> pinned entry bound.ts;
//                ../engine-locate -> fakes/engine-locate.ts; ../role-policy-resolve -> fakes/role-policy.ts (DEFAULT_ACCOUNT)
//  * the entry files (E, C, A, T, O, Q): exactly the L3-8 rules, except that E's five engine readers are the req fakes
//    (deferred Promises) instead of the entry fakes.
import { registerHooks } from 'node:module';

const here = file => new URL(file, import.meta.url).href;
const ref = name => here(`../entry/reference/${name}`);
const entryFakes = name => here(`../entry/fakes/${name}`);
const primitives = here('../entry/reference-primitives.mjs');
const stubs = entryFakes('stubs.ts');
const wire = entryFakes('wire.ts');
const fake = name => here(`./fakes/${name}`);

const R = here('./reference/backend-requests-engine-import.ts');
const E = ref('backend-engine-import.ts');
const C = ref('backend-codex-home.ts');
const A = ref('backend-accounts.ts');
const T = ref('backend-engine-import-tail.ts');
const Q = ref('backend-engine-import-qwen.ts');
const O = here('../grok/reference/opencode.ts');
const contracts = process.env.ORACLE_CONTRACTS === 'real' ? ref('shared-contracts.ts') : stubs;

const rules = new Map([
  [R, {
    'node:fs/promises': fake('node-fs-promises.ts'),
    'node:crypto': fake('node-crypto.ts'),
    '../../logic/records': here('./reference/logic-records.ts'),
    '../../shared/i18n/wire': wire,
    '../engine-import': fake('engine-import.ts'),
    '../engine-import/bound': ref('backend-engine-import-bound.ts'),
    '../engine-locate': fake('engine-locate.ts'),
    '../role-policy-resolve': fake('role-policy.ts'),
  }],
  [E, {
    'node:os': primitives,
    './accounts': A,
    './codex-home': C,
    './engine-import/claude': fake('engine-claude.ts'),
    './engine-import/codex': fake('engine-codex.ts'),
    './engine-import/grok': fake('engine-grok.ts'),
    './engine-import/opencode': fake('engine-opencode.ts'),
    './engine-import/qwen': fake('engine-qwen.ts'),
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
  [A, { '../shared/contracts': contracts, '../shared/i18n/wire': wire }],
  [T, { 'node:child_process': stubs, '../engine-children': stubs, '../windows-system': stubs }],
  [O, { 'node:os': primitives, '../engine-children': stubs, './tail': T, './plan': stubs }],
  [Q, { './opencode': O, './tail': T, './plan': stubs }],
]);

registerHooks({
  resolve(specifier, context, next) {
    const table = context.parentURL === undefined ? undefined : rules.get(context.parentURL);
    const target = table?.[specifier];
    if (target !== undefined) return next(target, context);
    return next(specifier, context);
  },
});
