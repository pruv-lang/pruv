// Oracle-only fake of backend/role-policy-resolve: DEFAULT_ACCOUNT is an immutable host binding (spec 3.4). The scenario world sets it
// once before the handler is created (world.defaultAccount, default 'default', the real Lair literal).
export let DEFAULT_ACCOUNT = 'default';
export function bindDefaultAccount(value: string): void { DEFAULT_ACCOUNT = value; }
