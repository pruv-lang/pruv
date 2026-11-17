// Oracle-only engine reader fake (SCENARIO.md section 7).
import { readerCall } from '../world.mjs';
export async function listGrok(source: unknown, folder: unknown, options: unknown): Promise<unknown> { return readerCall('grok', 'list', source, folder, undefined, false, options); }
export async function readGrok(source: unknown, folder: unknown, thread: unknown, options: unknown): Promise<unknown> { return readerCall('grok', 'read', source, folder, thread, true, options); }
export function copyGrokSession(..._args: unknown[]): never { throw new Error('oracle fake: entry copyGrokSession is not used'); }
export function grokLoadCheck(..._args: unknown[]): never { throw new Error('oracle fake: entry grokLoadCheck is not used'); }
