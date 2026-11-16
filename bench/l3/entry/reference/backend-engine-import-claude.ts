// Claude 바깥 대화 목록·기록 읽기(docs/notes/engine-import-2026-10-19.md "실제 기록 형식"·"엔진별 길").
// <home>/projects/<변환 이름>/<번호>.jsonl 을 읽기만 한다. 큰 파일은 머리·꼬리·끝 창만 읽는다.
import { readdir, realpath } from 'node:fs/promises';
import { join } from 'node:path';
import {
  EngineImportError, IMPORT_LIMITS, TurnCollector, isoOf, mapLimit, oneLine, openLineWindow, openRead, parseLine, plainDir, plainFileIn, readEdgeLines, record, str, validThread,
  type ExternalRecord, type ImportListItem, type ImportReadOptions, type ImportSource,
} from './tail';
import { folderKey } from './opencode';
import { ClaudePlanTracker } from './plan';

type RecordValue = Record<string, unknown>;

/** Claude 가 폴더 이름을 200자로 자르고 해시를 붙이므로 앞 200자로 후보를 찾는다 */
export const NAME_PREFIX = 200;
const SESSIONS_MAX = 200;
const SESSION_FILE_MAX = 16 * 1024;
/** 모델이 쓰지 않고 엔진이 채운 줄에 붙는 model 값 */
const SYNTHETIC_MODEL = '<synthetic>';
/** Claude 엔진이 중단 때 적는 고정 문구 둘의 전체 일치만(앞뒤 공백을 뺀 글 전체) */
const ENGINE_INTERRUPT_LINE = /^\[Request interrupted by user( for tool use)?\]$/;
/** 엔진이 넣은 작업 알림 줄: origin kind 와 글 전체 감쌈이 둘 다 맞을 때만 뺀다(사람 글에 태그가 일부 든 경우와 origin 없는 글은 남긴다) */
const TASK_NOTIFICATION_LINE = /^<task-notification>[\s\S]*<\/task-notification>$/;
/** 1MB 넘어 버린 줄의 머리에서 사람 줄을 알아보는 표지(해석하지 않는다) */
/**
 * 해석하지 못한 큰 줄의 머리로 본 사람 줄 판정(감사 c89db8de 필수 9·고친 뒤 확인 새 필수 1). user 줄이라도 첫 내용 블록이 도구 결과이거나
 * 메타·곁가지 줄이면 사람 글이 아니다(큰 도구 결과 줄은 흔하다).
 */
function droppedUser(head: string): boolean {
  return /"type"\s*:\s*"user"/.test(head) && !/"content"\s*:\s*\[\s*\{[^{}]*"type"\s*:\s*"tool_result"/.test(head)
    && !/"isMeta"\s*:\s*true/.test(head) && !/"isSidechain"\s*:\s*true/.test(head);
}

/** 경로의 영숫자 아닌 글자를 모두 '-' 로 바꾼 이름. 환경 변수로 이름을 고정했으면 그 이름 */
export function claudeProjectName(folder: string, env: NodeJS.ProcessEnv): string {
  const fixed = env.CLAUDE_CODE_PROJECT_DIR_NAME?.trim();
  return fixed || folder.replace(/[^A-Za-z0-9]/g, '-');
}

function plainName(name: string): boolean {
  return name.length > 0 && name !== '.' && name !== '..' && !/[\\/\0]/.test(name);
}

/** 이 폴더의 대화가 든 projects 하위 폴더 후보들(절대 경로). 파일은 열지 않는다 */
async function projectFolders(home: string, folder: string, env: NodeJS.ProcessEnv): Promise<string[]> {
  const name = claudeProjectName(folder, env);
  if (!plainName(name)) return [];
  const root = join(home, 'projects');
  if (!await plainDir(root)) return [];
  if (name.length <= NAME_PREFIX) {
    const exact = join(root, name);
    if (await plainDir(exact)) return [exact];
  }
  const names = await readdir(root).catch(() => [] as string[]);
  const prefix = name.slice(0, NAME_PREFIX);
  const found: string[] = [];
  // 링크인 폴더는 건너뛴다
  for (const entry of names) if (plainName(entry) && folderKey(entry).startsWith(folderKey(prefix)) && await plainDir(join(root, entry))) found.push(join(root, entry));
  return found;
}

/** 출처 home 의 실제 경로. 없으면 undefined */
async function realHome(home: string): Promise<string | undefined> {
  return realpath(home).catch(() => undefined);
}

/** 엔진이 넣은 줄인가: 중단 고정 줄이거나 작업 알림 줄(text 는 이어 붙이기 전 글 전체) */
function engineLine(entry: RecordValue, text: string): boolean {
  const trimmed = text.trim();
  return ENGINE_INTERRUPT_LINE.test(trimmed) || (record(entry.origin)?.kind === 'task-notification' && TASK_NOTIFICATION_LINE.test(trimmed));
}

/** user 줄에서 사람이 쓴 글. 도구 결과·메타·부가 대화 줄이면 undefined */
function humanText(entry: RecordValue): string | undefined {
  if (entry.type !== 'user' || entry.isMeta === true || entry.isSidechain === true) return undefined;
  const content = record(entry.message)?.content;
  // Claude 엔진이 중단 때 적는 고정 줄·작업 알림 줄이고 사람이 쓴 글이 아니다
  if (typeof content === 'string') return content.trim() && !engineLine(entry, content) ? content : undefined;
  if (!Array.isArray(content)) return undefined;
  const pieces: string[] = [];
  for (const block of content) {
    const item = record(block);
    if (!item) continue;
    if (item.type === 'tool_result') return undefined;
    if (item.type === 'text' && typeof item.text === 'string') pieces.push(item.text);
  }
  const joined = pieces.join('\n\n');
  return joined.trim() && !engineLine(entry, joined) ? joined : undefined;
}

export interface Head { title?: string; cwd?: string; entrypoint?: string }

/** 머리 64KB 에서 첫 사람 글·cwd·entrypoint */
export function readHead(lines: string[]): Head {
  const out: Head = {};
  for (const line of lines) {
    const entry = parseLine(line);
    if (!entry) continue;
    out.cwd ??= str(entry.cwd);
    out.entrypoint ??= str(entry.entrypoint);
    if (out.title === undefined) {
      const text = humanText(entry);
      if (text) out.title = oneLine(text, 120);
    }
    if (out.title !== undefined && out.cwd !== undefined && out.entrypoint !== undefined) break;
  }
  return out;
}

/** 엔진이 붙인 제목 줄. 사람이 바꾼 이름 > 자동 제목 > 요약 순, 같은 종류는 뒤엣것 */
function titleOf(entry: RecordValue): { rank: number; text: string } | undefined {
  const pick = (...keys: string[]): string | undefined => keys.map(key => str(entry[key])).find(value => value && value.trim());
  if (entry.type === 'custom-title') { const text = pick('customTitle', 'title'); return text ? { rank: 3, text } : undefined; }
  if (entry.type === 'ai-title') { const text = pick('aiTitle', 'title'); return text ? { rank: 2, text } : undefined; }
  if (entry.type === 'summary') { const text = pick('summary', 'title'); return text ? { rank: 1, text } : undefined; }
  return undefined;
}

function tailInfo(lines: string[]): { title?: string; lastAt?: string } {
  let best: { rank: number; text: string } | undefined;
  let lastAt: string | undefined;
  for (const line of lines) {
    const entry = parseLine(line);
    if (!entry) continue;
    const at = isoOf(entry.timestamp);
    if (at) lastAt = at;
    const found = titleOf(entry);
    if (found && (!best || found.rank >= best.rank)) best = found;
  }
  return { ...(best ? { title: oneLine(best.text, 120) } : {}), ...(lastAt ? { lastAt } : {}) };
}

async function alivePids(home: string, homeReal: string): Promise<Set<string>> {
  const alive = new Set<string>();
  const folder = join(home, 'sessions');
  if (!await plainDir(folder)) return alive;
  const names = (await readdir(folder).catch(() => [] as string[])).filter(name => name.endsWith('.json')).slice(0, SESSIONS_MAX);
  await mapLimit(names, IMPORT_LIMITS.parallel, async name => {
    try {
      const path = join(folder, name);
      const info = await plainFileIn(homeReal, path);
      if (!info || info.size > SESSION_FILE_MAX) return;
      const handle = await openRead(path, info);
      let raw: string;
      try { raw = (await handle.readFile({ encoding: 'utf8' })); } finally { await handle.close(); }
      const entry = parseLine(raw);
      const pid = entry?.pid;
      const session = str(entry?.sessionId);
      if (!session || typeof pid !== 'number' || !Number.isInteger(pid) || pid <= 0) return;
      try { process.kill(pid, 0); alive.add(session); } catch (error) {
        // 권한이 없어도 프로세스는 있다
        if ((error as NodeJS.ErrnoException).code === 'EPERM') alive.add(session);
      }
    } catch { /* 안내용이라 읽기 실패는 모름으로 둔다 */ }
  });
  return alive;
}

/** 이 폴더의 Claude 대화 최근 50개 */
export async function listClaude(source: ImportSource, folder: string, options: ImportReadOptions): Promise<ImportListItem[]> {
  const homeReal = await realHome(source.home);
  if (!homeReal) return [];
  const folders = await projectFolders(source.home, folder, options.env ?? process.env);
  const files: Array<{ thread: string; path: string; size: number; mtimeMs: number; id: { dev: number; ino: number } }> = [];
  for (const dir of folders) {
    const names = (await readdir(dir).catch(() => [] as string[])).filter(name => name.endsWith('.jsonl') && validThread(name.slice(0, -6)));
    const stats = await mapLimit(names, IMPORT_LIMITS.parallel, async name => {
      options.signal.throwIfAborted();
      const path = join(dir, name);
      const info = await plainFileIn(homeReal, path);
      return info ? { thread: name.slice(0, -6), path, size: info.size, mtimeMs: info.mtimeMs, id: { dev: info.dev, ino: info.ino } } : undefined;
    });
    for (const entry of stats) if (entry) files.push(entry);
  }
  files.sort((a, b) => b.mtimeMs - a.mtimeMs);
  const recent = files.slice(0, IMPORT_LIMITS.list);
  const items = await mapLimit(recent, IMPORT_LIMITS.parallel, async (file): Promise<ImportListItem | undefined> => {
    options.signal.throwIfAborted();
    try {
      const head = readHead(await readEdgeLines(file.path, 'head', file.size, file.id));
      // 변환 이름이 같은 다른 폴더의 대화는 줄의 cwd 로 걸러낸다
      if (head.cwd !== undefined && folderKey(head.cwd) !== folderKey(folder)) return undefined;
      const tail = tailInfo(await readEdgeLines(file.path, 'tail', file.size, file.id));
      return {
        thread: file.thread, title: tail.title ?? head.title ?? '', updatedAt: tail.lastAt ?? new Date(file.mtimeMs).toISOString(),
        size: file.size, path: file.path, ...(head.entrypoint ? { entrypoint: head.entrypoint } : {}),
      };
    } catch { return undefined; }
  });
  const listed = items.filter((item): item is ImportListItem => item !== undefined);
  const alive = await alivePids(source.home, homeReal);
  return listed.map(item => (alive.has(item.thread) ? { ...item, maybeOpen: true as const } : item));
}

/** 고른 Claude 대화의 최근 기록. 파일 끝 8MB 창만 읽는다 */
export async function readClaude(source: ImportSource, folder: string, thread: string, options: ImportReadOptions): Promise<ExternalRecord> {
  if (!validThread(thread)) throw new EngineImportError('not-found');
  const homeReal = await realHome(source.home);
  if (!homeReal) throw new EngineImportError('not-found');
  let path: string | undefined;
  let found: { dev: number; ino: number } | undefined;
  for (const dir of await projectFolders(source.home, folder, options.env ?? process.env)) {
    const candidate = join(dir, `${thread}.jsonl`);
    const info = await plainFileIn(homeReal, candidate);
    if (!info) continue;
    const head = readHead(await readEdgeLines(candidate, 'head', info.size, info));
    if (head.cwd !== undefined && folderKey(head.cwd) !== folderKey(folder)) continue;
    path = candidate;
    found = info;
    break;
  }
  if (!path) throw new EngineImportError('not-found');
  const collector = new TurnCollector();
  const tracker = new ClaudePlanTracker(isoOf);
  let model: string | undefined;
  let titleRank = 0;
  let title: string | undefined;
  try {
    const window = await openLineWindow(path, options.signal, { isUser: droppedUser, onUserDropped: () => collector.skipUser() }, found);
    for await (const line of window.lines) {
      const entry = parseLine(line);
      if (!entry) continue;
      tracker.line(entry);
      const found = titleOf(entry);
      if (found && found.rank >= titleRank) { titleRank = found.rank; title = oneLine(found.text, 120); }
      const prompt = humanText(entry);
      if (prompt !== undefined) { collector.begin(prompt, isoOf(entry.timestamp)); continue; }
      if (entry.type !== 'assistant' || entry.isSidechain === true) continue;
      const message = record(entry.message);
      const at = isoOf(entry.timestamp);
      const modelId = str(message?.model);
      // 엔진은 재개할 때 기록 끝이 사람 줄이면 교대를 맞추려 이 줄을 넣는다. isApiErrorMessage 가 참인 줄은 실패 사유라 남긴다
      if (modelId === SYNTHETIC_MODEL && entry.isApiErrorMessage !== true) continue;
      if (modelId && modelId !== SYNTHETIC_MODEL) model = modelId;
      const content = message?.content;
      if (typeof content === 'string') { collector.answer(content, at); continue; }
      if (!Array.isArray(content)) continue;
      for (const block of content) {
        const item = record(block);
        if (item?.type === 'text' && typeof item.text === 'string') collector.answer(item.text, at);
        else if (item?.type === 'tool_use') collector.tool();
      }
    }
    const done = collector.finish();
    const plan = tracker.finish();
    return { turns: done.turns, omittedBefore: window.omittedBefore || done.trimmed, ...(model ? { model } : {}), ...(title ? { title } : {}), ...(plan ? { lastPlan: plan } : {}) };
  } catch (error) {
    if (error instanceof EngineImportError || (error as Error).name === 'AbortError') throw error;
    throw new EngineImportError('read-failed', undefined, { cause: error });
  }
}
