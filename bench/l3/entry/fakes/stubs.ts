// Oracle-only inert stand-ins for the imports of the pinned files that the common entry never reaches (DESIGN section 6).
// Every function throws if it is ever called, so a stub that is reached cannot silently change an observation.
const outside = (name: string): never => { throw new Error(`oracle stub reached: ${name}`); };

// codex-home.ts
export function withEngineFolders(..._args: unknown[]): never { return outside('withEngineFolders'); }
export function codexProviderPrepare(..._args: unknown[]): never { return outside('codexProviderPrepare'); }
export function wireReason(..._args: unknown[]): never { return outside('wireReason'); }
// tail.ts, opencode.ts
export function spawn(..._args: unknown[]): never { return outside('child_process.spawn'); }
export function spawnEngine(..._args: unknown[]): never { return outside('spawnEngine'); }
export function systemTaskkillPath(..._args: unknown[]): never { return outside('systemTaskkillPath'); }
// opencode.ts and qwen.ts import their plan builders from ./plan (plan.ts needs a transpiler; it is never called here)
export function opencodePlan(..._args: unknown[]): never { return outside('opencodePlan'); }
export function qwenPlan(..._args: unknown[]): never { return outside('qwenPlan'); }
// shared/contracts fallback if the real file cannot load
export const ENGINE_IDS = ['codex', 'claude', 'agy', 'grok', 'opencode', 'kiro', 'qwen'] as const;
