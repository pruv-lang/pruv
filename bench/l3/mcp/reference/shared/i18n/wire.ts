// Metadata stubs; original Korean fallback messages are retained.
export type WireText = { key: string; args?: object };
export const wire = (key: string, args?: object): WireText => ({key, args});
export const wireLocal = (_: unknown, fallback: string) => fallback;
export const wireOf = (error: any) => error?.i18n;
export class WireError extends Error {
  i18n: WireText;
  constructor(i18n: WireText, fallback: string) { super(fallback); this.i18n = i18n; }
}
