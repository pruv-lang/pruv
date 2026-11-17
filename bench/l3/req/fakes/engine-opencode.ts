// Oracle-only engine reader fake (SCENARIO.md section 7).
import { readerCall } from '../world.mjs';
export async function listOpencode(source: unknown, folder: unknown, options: unknown): Promise<unknown> { return readerCall('opencode', 'list', source, folder, undefined, false, options); }
export async function readOpencode(source: unknown, folder: unknown, thread: unknown, options: unknown): Promise<unknown> { return readerCall('opencode', 'read', source, folder, thread, true, options); }
// the pinned helper, never reimplemented
export { opencodeDataDir } from '../../grok/reference/opencode.ts';
