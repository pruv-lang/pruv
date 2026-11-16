// Non-win32 spawnEngine at Lair 744d14af reduces to spawn(command, args, options) plus a process
// count that only feeds Lair's UI; the count is dropped here. Everything else must never run.
import { spawn, type SpawnOptions } from 'node:child_process';
export function spawnEngine(_engine: string, command: string, args: readonly string[], options: SpawnOptions) {
  return spawn(command, [...args], options);
}
export function systemTaskkillPath(): never { throw new Error('Windows outside Q'); }
export function accountHome(): never { throw new Error('accounts outside Q'); }
export function childEnvironment(): never { throw new Error('codex install outside Q'); }
export function findCodex(): never { throw new Error('codex install outside Q'); }
export function prepareCodexAccount(): never { throw new Error('codex accounts outside Q'); }
