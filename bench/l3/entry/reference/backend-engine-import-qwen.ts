// Qwen Code 바깥 대화 목록·기록 읽기(docs/notes/tool-migration-2026-11-03.md M12 설계 'Qwen 찾기·읽기').
// <런타임>/projects/<폴더 이름>/chats/<번호>.jsonl 을 읽기만 한다(설치 CLI 를 돌리지 않는다). 재개는 공식 ACP 가 한다.
// 폴더 비교와 열림 시각 기준은 OpenCode 와 같은 것을 쓰려고 opencode.ts 에서 가져온다(복사하지 않는다).
import { readdir, realpath } from 'node:fs/promises';
import { join } from 'node:path';
import {
  EngineImportError, IMPORT_LIMITS, TurnCollector, isoOf, mapLimit, oneLine, openLineWindow, parseLine, plainDir, plainFileIn, readEdgeLines, record, str, validThread,
  type ExternalRecord, type ImportListItem, type ImportReadOptions, type ImportSource,
} from './tail';
import { OPEN_WINDOW_MS, folderKey } from './opencode';
import { qwenPlan } from './plan';

type RecordValue = Record<string, unknown>;

/** Qwen 전역 폴더: QWEN_HOME, 없으면 <집>/.qwen */
export function qwenGlobalDir(env: NodeJS.ProcessEnv, home: string): string {
  return env.QWEN_HOME?.trim() || join(home, '.qwen');
}

/** Qwen 기록(런타임) 폴더: QWEN_RUNTIME_DIR, 없으면 전역 폴더 */
export function qwenRuntimeDir(env: NodeJS.ProcessEnv, home: string): string {
  return env.QWEN_RUNTIME_DIR?.trim() || qwenGlobalDir(env, home);
}

/** Qwen 이 폴더를 기록 폴더 이름으로 바꾸는 규칙: win32 는 소문자로 먼저, 영숫자 아닌 글자를 모두 '-' 로 */
export function qwenProjectName(folder: string, platform: NodeJS.Platform): string {
  return (platform === 'win32' ? folder.toLowerCase() : folder).replace(/[^a-zA-Z0-9]/g, '-');
}

/** 사람 글 표지: provenance 가 real_user 이거나(칸이 없는 옛 줄 포함) 아닌 user 줄은 사람 글이 아니다 */
function humanText(entry: RecordValue): string | undefined {
  if (entry.type !== 'user') return undefined;
  if (entry.provenance !== undefined && entry.provenance !== 'real_user') return undefined;
  const parts = record(entry.message)?.parts;
  if (!Array.isArray(parts)) return undefined;
  const pieces: string[] = [];
  for (const raw of parts) {
    const part = record(raw);
    if (part && part.thought !== true && typeof part.text === 'string') pieces.push(part.text);
  }
  const joined = pieces.join('\n\n');
  return joined.trim() ? joined : undefined;
}

/** 1MB 넘어 해석하지 못한 줄의 머리로 본 사람 줄 판정 */
function droppedUser(head: string): boolean {
  return /"type"\s*:\s*"user"/.test(head) && /"provenance"\s*:\s*"real_user"/.test(head);
}

export interface QwenHead { cwd?: string; human: boolean; firstPrompt?: string }

/** 머리 줄들에서 cwd·사람 글이 있는가·첫 사람 글 */
export function readQwenHead(lines: string[]): QwenHead {
  let cwd: string | undefined;
  let firstPrompt: string | undefined;
  for (const line of lines) {
    const entry = parseLine(line);
    if (!entry) continue;
    cwd ??= str(entry.cwd);
    if (firstPrompt === undefined) {
      const text = humanText(entry);
      if (text) firstPrompt = text;
    }
    if (cwd !== undefined && firstPrompt !== undefined) break;
  }
  return { ...(cwd !== undefined ? { cwd } : {}), human: firstPrompt !== undefined, ...(firstPrompt !== undefined ? { firstPrompt } : {}) };
}

/** 꼬리 줄들에서 마지막 custom_title 제목과 마지막 시각 */
function tailInfo(lines: string[]): { title?: string; lastAt?: string } {
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

/** 이 폴더의 기록 폴더(<home>/projects/<이름>/chats). 링크이거나 없으면 undefined */
async function chatsDir(home: string, folder: string): Promise<string | undefined> {
  const name = qwenProjectName(folder, process.platform);
  if (!name) return undefined;
  const root = join(home, 'projects');
  const project = join(root, name);
  const chats = join(project, 'chats');
  return await plainDir(root) && await plainDir(project) && await plainDir(chats) ? chats : undefined;
}

/** 이 폴더의 Qwen 대화 최근 50개 */
export async function listQwen(source: ImportSource, folder: string, options: ImportReadOptions): Promise<ImportListItem[]> {
  const homeReal = await realpath(source.home).catch(() => undefined);
  if (!homeReal) return [];
  const dir = await chatsDir(source.home, folder);
  if (!dir) return [];
  const names = (await readdir(dir).catch(() => [] as string[])).filter(name => name.endsWith('.jsonl') && validThread(name.slice(0, -6)));
  const stats = await mapLimit(names, IMPORT_LIMITS.parallel, async name => {
    options.signal.throwIfAborted();
    const path = join(dir, name);
    const info = await plainFileIn(homeReal, path);
    return info ? { thread: name.slice(0, -6), path, size: info.size, mtimeMs: info.mtimeMs, id: { dev: info.dev, ino: info.ino } } : undefined;
  });
  const files = stats.filter((entry): entry is NonNullable<typeof entry> => entry !== undefined);
  files.sort((a, b) => b.mtimeMs - a.mtimeMs);
  const now = Date.now();
  const readItem = async (file: (typeof files)[number]): Promise<ImportListItem | undefined> => {
    options.signal.throwIfAborted();
    try {
      const head = readQwenHead(await readEdgeLines(file.path, 'head', file.size, file.id));
      // 폴더 이름이 같은 다른 폴더의 대화는 줄의 cwd 로 걸러낸다. 파일 전체를 머리로 읽었는데 사람 글이 없을 때만 뺀다(머리 밖이나 큰 줄에 있을 수 있다)
      if (head.cwd !== undefined && folderKey(head.cwd) !== folderKey(folder)) return undefined;
      if (!head.human && file.size <= IMPORT_LIMITS.edge) return undefined;
      const tail = tailInfo(await readEdgeLines(file.path, 'tail', file.size, file.id));
      const title = tail.title ?? (head.firstPrompt !== undefined ? oneLine(head.firstPrompt, 120) : '');
      return {
        thread: file.thread, title, updatedAt: tail.lastAt ?? new Date(file.mtimeMs).toISOString(),
        size: file.size, path: file.path,
        ...(now - file.mtimeMs < OPEN_WINDOW_MS ? { maybeOpen: true as const } : {}),
      };
    } catch (error) {
      if ((error as Error | undefined)?.name === 'AbortError') throw error;
      return undefined;
    }
  };
  // 걸러진 파일만큼 다음 후보를 더 읽어 상한을 채운다(최신 50개가 다 빈 대화여도 그 뒤 대화가 나오게)
  const items: ImportListItem[] = [];
  for (let at = 0; at < files.length && items.length < IMPORT_LIMITS.list; at += IMPORT_LIMITS.list) {
    const batch = await mapLimit(files.slice(at, at + IMPORT_LIMITS.list), IMPORT_LIMITS.parallel, readItem);
    for (const item of batch) if (item) items.push(item);
  }
  return items.slice(0, IMPORT_LIMITS.list);
}

/** 창에서 읽은 줄: 해석한 줄, 또는 너무 커서 버린 사람 줄 자리 */
type Slot = { entry: RecordValue } | { dropped: true };

/** 끝 줄에서 parentUuid 를 따라간 uuid 집합. 끝 줄은 기록 줄 중 마지막 uuid 줄(없으면 마지막 uuid 줄). 부모가 창 밖이거나 버린 큰 줄이면 거기까지이고 cut 이 참이다 */
function currentChain(slots: readonly Slot[]): { uuids: Set<string>; cut: boolean } | undefined {
  const byUuid = new Map<string, RecordValue>();
  let leaf: RecordValue | undefined;
  let leafAny: RecordValue | undefined;
  for (const slot of slots) {
    if (!('entry' in slot)) continue;
    const uuid = str(slot.entry.uuid);
    if (!uuid) continue;
    byUuid.set(uuid, slot.entry);
    leafAny = slot.entry;
    if (slot.entry.type === 'user' || slot.entry.type === 'assistant' || slot.entry.type === 'tool_result') leaf = slot.entry;
  }
  const start = leaf ?? leafAny;
  if (!start) return undefined;
  const uuids = new Set<string>();
  let cut = false;
  let at: RecordValue | undefined = start;
  while (at) {
    const uuid = str(at.uuid);
    if (!uuid || uuids.has(uuid)) break;
    uuids.add(uuid);
    const parent = str(at.parentUuid);
    at = parent ? byUuid.get(parent) : undefined;
    if (parent && !at) cut = true;
  }
  return { uuids, cut };
}

/** 고른 Qwen 대화의 최근 기록. 파일 끝 8MB 창만 읽고 현재 사슬의 줄만 요청으로 만든다 */
export async function readQwen(source: ImportSource, folder: string, thread: string, options: ImportReadOptions): Promise<ExternalRecord> {
  if (!validThread(thread)) throw new EngineImportError('not-found');
  const homeReal = await realpath(source.home).catch(() => undefined);
  if (!homeReal) throw new EngineImportError('not-found');
  const dir = await chatsDir(source.home, folder);
  if (!dir) throw new EngineImportError('not-found');
  const path = join(dir, `${thread}.jsonl`);
  const found = await plainFileIn(homeReal, path);
  if (!found) throw new EngineImportError('not-found');
  const head = readQwenHead(await readEdgeLines(path, 'head', found.size, found));
  if (head.cwd !== undefined && folderKey(head.cwd) !== folderKey(folder)) throw new EngineImportError('not-found');
  const slots: Slot[] = [];
  let title: string | undefined;
  try {
    const window = await openLineWindow(path, options.signal, { isUser: droppedUser, onUserDropped: () => slots.push({ dropped: true }) }, found);
    for await (const line of window.lines) {
      const entry = parseLine(line);
      if (!entry) continue;
      slots.push({ entry });
      if (entry.type === 'system' && entry.subtype === 'custom_title') {
        const text = str(record(entry.systemPayload)?.customTitle);
        if (text && text.trim()) title = oneLine(text, 120);
      }
    }
    const chain = currentChain(slots);
    const inChain = (slot: Slot): boolean => 'entry' in slot && (chain === undefined || (typeof slot.entry.uuid === 'string' && chain.uuids.has(slot.entry.uuid)));
    const first = slots.findIndex(inChain);
    const collector = new TurnCollector();
    const planInput: Array<{ parts: readonly unknown[]; at?: string }> = [];
    let model: string | undefined;
    slots.forEach((slot, index) => {
      // 버린 큰 사람 줄은 사슬의 첫 줄 바로 앞부터만 센다(그 앞은 창 밖으로 끊긴 기록이다)
      if ('dropped' in slot) { if (first >= 0 && index >= first - 1) collector.skipUser(); return; }
      if (!inChain(slot)) return;
      const entry = slot.entry;
      const at = isoOf(entry.timestamp);
      const prompt = humanText(entry);
      if (prompt !== undefined) { collector.begin(prompt, at); return; }
      if (entry.type !== 'assistant') return;
      const modelId = str(entry.model);
      if (modelId) model = modelId;
      const parts = record(entry.message)?.parts;
      if (!Array.isArray(parts)) return;
      planInput.push({ parts, ...(at ? { at } : {}) });
      for (const raw of parts) {
        const part = record(raw);
        if (!part || part.thought === true) continue;
        if (typeof part.text === 'string') collector.answer(part.text, at);
        else if (record(part.functionCall)) collector.tool();
      }
    });
    const done = collector.finish();
    const plan = qwenPlan(planInput);
    const shownTitle = title ?? (head.firstPrompt !== undefined ? oneLine(head.firstPrompt, 120) : undefined);
    return {
      turns: done.turns, omittedBefore: window.omittedBefore || done.trimmed || chain?.cut === true,
      ...(model ? { model } : {}), ...(shownTitle ? { title: shownTitle } : {}), ...(plan ? { lastPlan: plan } : {}),
    };
  } catch (error) {
    if (error instanceof EngineImportError || (error as Error).name === 'AbortError') throw error;
    throw new EngineImportError('read-failed', undefined, { cause: error });
  }
}
