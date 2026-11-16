// Oracle-only fake of the pinned engine-import/opencode module (DESIGN section 6): records the route and the forwarded arguments and
// serves the case's engine script (or forward entry). A sync_throw entry is thrown before the first await, a reject after it.
import { engineCall } from '../reference-primitives.mjs';

export async function listOpencode(source: unknown, folder: unknown, options: unknown): Promise<unknown> {
  return engineCall('opencode', 'list', source, folder, undefined, false, options);
}
export async function readOpencode(source: unknown, folder: unknown, thread: unknown, options: unknown): Promise<unknown> {
  return engineCall('opencode', 'read', source, folder, thread, true, options);
}
// the pinned helper, never reimplemented
export { opencodeDataDir } from '../../grok/reference/opencode.ts';
