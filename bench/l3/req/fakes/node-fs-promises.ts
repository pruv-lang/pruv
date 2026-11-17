// Oracle-only fake of node:fs/promises for the handler: only rm (the cleanup request).
import { rm as fake } from '../world.mjs';
export function rm(path: unknown, options: unknown): Promise<void> { return (fake as (p: unknown, o: unknown) => Promise<void>)(path, options); }
