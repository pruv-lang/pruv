// Oracle-only fake of the pinned engine-import/grok module (DESIGN section 6): records the route and the forwarded arguments and
// serves the case's engine script (or forward entry). A sync_throw entry is thrown before the first await, a reject after it.
import { engineCall } from '../reference-primitives.mjs';

export async function listGrok(source: unknown, folder: unknown, options: unknown): Promise<unknown> {
  return engineCall('grok', 'list', source, folder, undefined, false, options);
}
export async function readGrok(source: unknown, folder: unknown, thread: unknown, options: unknown): Promise<unknown> {
  return engineCall('grok', 'read', source, folder, thread, true, options);
}
export function copyGrokSession(..._args: unknown[]): never { throw new Error('oracle fake: copyGrokSession'); }
export function grokLoadCheck(..._args: unknown[]): never { throw new Error('oracle fake: grokLoadCheck'); }
