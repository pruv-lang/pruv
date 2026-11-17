// Oracle-only fake of backend/engine-locate (locateAcp).
import { locateAcp as fake } from '../world.mjs';
export function locateAcp(name: unknown): Promise<unknown> { return (fake as (n: unknown) => Promise<unknown>)(name); }
