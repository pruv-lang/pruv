import { registerHooks, createRequire } from 'node:module';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
// plan.ts uses a constructor parameter property, which strip-only mode rejects; only that file is
// transpiled, with the TypeScript compiler from Lair's own dependencies (5.7.3).
const ts = createRequire('/Users/spica4345/Projects/lair/package.json')('typescript');
const stub = new URL('./stubs.ts', import.meta.url).href;
const plan = new URL('./plan.ts', import.meta.url).href;
const redirect = new Set(['../engine-children', '../windows-system', '../accounts', '../codex-engine', '../codex-home']);
registerHooks({
 resolve(s,c,next) {
  if(redirect.has(s)) return next(stub,c);
  if(s==='../../shared/engine-import') return next(new URL('./shared-engine-import.ts',import.meta.url).href,c);
  if(s.startsWith('.')&&!/\.[cm]?[jt]s$/.test(s)) return next(s+'.ts',c);
  return next(s,c);
 },
 load(url,c,next) {
  if(url!==plan) return next(url,c);
  const out=ts.transpileModule(readFileSync(fileURLToPath(url),'utf8'),{compilerOptions:{module:ts.ModuleKind.ESNext,target:ts.ScriptTarget.ES2022,useDefineForClassFields:true}});
  return {format:'module',source:out.outputText,shortCircuit:true};
 },
});
