// Oracle-only engine reader fake (SCENARIO.md section 7): records the call and serves the scenario's reader queue (deferred Promises).
import { readerCall } from '../world.mjs';
export async function listClaude(source: unknown, folder: unknown, options: unknown): Promise<unknown> { return readerCall('claude', 'list', source, folder, undefined, false, options); }
export async function readClaude(source: unknown, folder: unknown, thread: unknown, options: unknown): Promise<unknown> { return readerCall('claude', 'read', source, folder, thread, true, options); }
