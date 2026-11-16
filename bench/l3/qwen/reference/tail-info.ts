import { isoOf, parseLine, str, record, oneLine } from "./tail";
export function tailInfo(lines: string[]): { title?: string; lastAt?: string } {
  let title: string | undefined;
  let lastAt: string | undefined;
  for (const line of lines) {
    const entry = parseLine(line);
    if (!entry) continue;
    const at = isoOf(entry.timestamp);
    if (at) lastAt = at;
    if (entry.type === 'system' && entry.subtype === 'custom_title') {
      const text = str(record(entry.systemPayload)?.customTitle);
      if (text && text.trim()) title = oneLine(text, 120);
    }
  }
  return { ...(title ? { title } : {}), ...(lastAt ? { lastAt } : {}) };
}

