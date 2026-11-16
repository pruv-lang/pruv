// Unused subprocess paths must never run in this filesystem-only oracle.
export function spawnEngine(): never { throw new Error('subprocess outside Q'); }
export function systemTaskkillPath(): never { throw new Error('Windows outside Q'); }
