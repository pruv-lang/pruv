// Oracle-only engine reader fake (SCENARIO.md section 7).
import { readerCall } from '../world.mjs';
export async function listQwen(source: unknown, folder: unknown, options: unknown): Promise<unknown> { return readerCall('qwen', 'list', source, folder, undefined, false, options); }
export async function readQwen(source: unknown, folder: unknown, thread: unknown, options: unknown): Promise<unknown> { return readerCall('qwen', 'read', source, folder, thread, true, options); }
// the pinned helper, never reimplemented
export { qwenRuntimeDir } from '../../entry/reference/backend-engine-import-qwen.ts';
