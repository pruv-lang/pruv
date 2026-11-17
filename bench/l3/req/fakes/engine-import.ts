// Oracle-only facade standing in for the handler's `../engine-import`: the REAL pinned entry functions plus the fake host calls.
import { copyGrokSession as copyFake, grokLoadCheck as loadFake, makeCodexRunDeps as codexFake } from '../world.mjs';
export { EngineImportError, IMPORT_LIMITS, importSources, findSource, listExternal, readExternal } from '../../entry/reference/backend-engine-import.ts';
export function copyGrokSession(...args: unknown[]): Promise<unknown> { return (copyFake as (...a: unknown[]) => Promise<unknown>)(...args); }
export function grokLoadCheck(...args: unknown[]): Promise<unknown> { return (loadFake as (...a: unknown[]) => Promise<unknown>)(...args); }
export function makeCodexRunDeps(...args: unknown[]): Promise<unknown> { return (codexFake as (...a: unknown[]) => Promise<unknown>)(...args); }
