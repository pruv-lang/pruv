// Oracle-only fake of node:crypto for the pinned request-budget class: only randomUUID, served from the scenario UUID stream in
// oracle-world.mjs (explicit list, else 00000000-0000-4000-8000-<n as 12 hex digits>). Loaded through reference-hooks.mjs; no run command of its own.
import { randomUUID as fake } from '../oracle-world.mjs';
export function randomUUID(): string { return (fake as () => string)(); }
