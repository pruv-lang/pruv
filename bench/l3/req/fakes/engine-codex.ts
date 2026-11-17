// Oracle-only engine reader fake (SCENARIO.md section 7).
import { readerCall } from '../world.mjs';
export async function listCodex(source: unknown, folder: unknown, options: unknown): Promise<unknown> { return readerCall('codex', 'list', source, folder, undefined, false, options); }
export async function readCodex(source: unknown, folder: unknown, thread: unknown, options: unknown): Promise<unknown> { return readerCall('codex', 'read', source, folder, thread, true, options); }
export function makeCodexRunDeps(..._args: unknown[]): never { throw new Error('oracle fake: entry makeCodexRunDeps is not used; the handler gets fakes/engine-import.ts'); }
