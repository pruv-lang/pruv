// Oracle-only fake of node:crypto for the handler: only randomUUID (scenario UUID list).
import { randomUUID as fake } from '../world.mjs';
export function randomUUID(): string { return (fake as () => string)(); }
