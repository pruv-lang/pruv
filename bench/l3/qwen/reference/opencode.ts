// Non-win32 behavior from fa192269.
export const OPEN_WINDOW_MS = 2 * 60 * 1000;
export function folderKey(folder: string): string { return folder.replace(/\/+$/, '') || folder; }
