// Oracle-only inert wire support for the pinned accounts.ts and codex-home.ts (DESIGN section 6): WireError stores key,
// wire(key) returns {key}. Nothing here formats text; the pinned callers pass their own fallback message as the Error message.
export interface WireText { key: string }
export class WireError extends Error {
  key: string;
  constructor(text: WireText, message?: string) {
    super(message);
    this.name = 'WireError';
    this.key = text.key;
  }
}
export function wire(key: string, _args?: unknown): WireText { return { key }; }
export function wireOf(_error: unknown): WireText | undefined { return undefined; }
