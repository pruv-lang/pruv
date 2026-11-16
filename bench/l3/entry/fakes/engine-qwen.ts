// Oracle-only fake of the pinned engine-import/qwen module (DESIGN section 6): records the route and the forwarded arguments and
// serves the case's engine script (or forward entry). A sync_throw entry is thrown before the first await, a reject after it.
import { engineCall } from '../reference-primitives.mjs';

export async function listQwen(source: unknown, folder: unknown, options: unknown): Promise<unknown> {
  return engineCall('qwen', 'list', source, folder, undefined, false, options);
}
export async function readQwen(source: unknown, folder: unknown, thread: unknown, options: unknown): Promise<unknown> {
  return engineCall('qwen', 'read', source, folder, thread, true, options);
}
// the pinned helper, never reimplemented
export { qwenRuntimeDir } from '../reference/backend-engine-import-qwen.ts';
