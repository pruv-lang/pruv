// Grok 바깥 대화 목록·기록 읽기·앱 폴더로 복사·열기 확인(docs/notes/engine-import-2026-10-19.md "엔진별 길").
// <home>/sessions/<encodeURIComponent(폴더)>/<번호>/ 를 읽는다. 인증 파일은 읽지도 복사하지도 않는다.
import { randomBytes } from 'node:crypto';
import { createWriteStream } from 'node:fs';
import { lstat, mkdir, readdir, realpath, rename, rm } from 'node:fs/promises';
import { isAbsolute, join, relative } from 'node:path';
import { pipeline } from 'node:stream/promises';
import {
  EngineImportError, IMPORT_LIMITS, TurnCollector, isoOf, mapLimit, oneLine, openLineWindow, openRead, parseLine, plainDir, plainFileIn, record, str, validThread, withRpcChild,
  type ExternalRecord, type ImportListItem, type ImportReadOptions, type ImportSource,
} from './tail';
import { folderKey } from './opencode';

type RecordValue = Record<string, unknown>;

const SUMMARY_MAX = 64 * 1024;

/**
 * 복사해도 되는 파일 이름. auth 가 들어간 이름과 *.lock 은 넣지 않는다.
 * 이름은 Grok CLI 1.0.62 의 세션 폴더 실측(docs/notes/engine-import-2026-10-19.md)에서 옮겼다.
 */
const COPY_ALLOW: ReadonlySet<string> = new Set([
  'summary.json', 'chat_history.jsonl', 'events.jsonl', 'updates.jsonl', 'prompt_context.json', 'resources_state.json',
  'rewind_points.jsonl', 'signals.json', 'usage.json', 'system_prompt.txt', 'tool_definitions.json', 'announcement_state.json',
  'title_refresh_idx',
]);

/** 띄우는 인자와 시작 요청 모양은 src/backend/grok-acp.ts 175(agent --no-leader stdio)·686(initialize)·700(session/load)에서 옮겼다 */
const ACP_ARGS = ['agent', '--no-leader', 'stdio'] as const;

function encodedFolder(folder: string): string {
  const name = encodeURIComponent(folder);
  if (!name || name === '.' || name === '..') throw new EngineImportError('not-found');
  return name;
}

/** 출처 home 아래 이 폴더의 세션 폴더 */
function sessionsOf(home: string, folder: string): string {
  return join(home, 'sessions', encodedFolder(folder));
}

/** 1MB 넘어 버린 줄의 머리에서 사람 줄을 알아보는 표지(해석하지 않는다) */
/** 해석하지 못한 큰 줄의 머리로 본 사람 줄 판정. Grok 줄은 type 이 첫 칸이고, 앱이 넣은 글(synthetic_reason)은 사람 글이 아니다 */
function droppedUser(head: string): boolean {
  return /^\s*\{\s*"type"\s*:\s*"user"/.test(head) && !/"synthetic_reason"\s*:\s*"/.test(head);
}

/** 출처 home 의 실제 경로. 없으면 undefined */
async function realHome(home: string): Promise<string | undefined> {
  return realpath(home).catch(() => undefined);
}

/** home/sessions/<인코딩>/<번호> 사슬이 모두 링크 아닌 폴더인가(번호를 주지 않으면 인코딩 폴더까지) */
async function plainChain(home: string, folder: string, thread?: string): Promise<boolean> {
  const base = sessionsOf(home, folder);
  return await plainDir(join(home, 'sessions')) && await plainDir(base) && (thread === undefined || await plainDir(join(base, thread)));
}

export async function readSmallJson(homeReal: string, path: string): Promise<RecordValue | undefined> {
  try {
    const info = await plainFileIn(homeReal, path);
    if (!info || info.size > SUMMARY_MAX) return undefined;
    const handle = await openRead(path, info);
    try { return parseLine(await handle.readFile('utf8')); } finally { await handle.close(); }
  } catch { return undefined; }
}

/** summary.json 이 이 폴더·이 번호의 것일 때만 돌려준다 */
async function readSummary(homeReal: string, dir: string, folder: string, thread: string): Promise<RecordValue | undefined> {
  const summary = await readSmallJson(homeReal, join(dir, 'summary.json'));
  const info = record(summary?.info);
  const cwd = str(info?.cwd);
  if (!summary || str(info?.id) !== thread || cwd === undefined || folderKey(cwd) !== folderKey(folder)) return undefined;
  return summary;
}

async function activeIds(home: string, homeReal: string): Promise<Set<string>> {
  const ids = new Set<string>();
  try {
    const path = join(home, 'active_sessions.json');
    const info = await plainFileIn(homeReal, path);
    if (!info || info.size > SUMMARY_MAX) return ids;
    const handle = await openRead(path, info);
    let raw: string;
    try { raw = await handle.readFile('utf8'); } finally { await handle.close(); }
    const list: unknown = JSON.parse(raw);
    if (!Array.isArray(list)) return ids;
    for (const entry of list) {
      const id = typeof entry === 'string' ? entry : str(record(entry)?.id) ?? str(record(entry)?.sessionId) ?? str(record(entry)?.session_id);
      if (id) ids.add(id);
    }
  } catch { /* 안내용이라 읽기 실패는 모름으로 둔다 */ }
  return ids;
}

/** 이 폴더의 Grok 대화 최근 50개 */
export async function listGrok(source: ImportSource, folder: string, options: ImportReadOptions): Promise<ImportListItem[]> {
  const homeReal = await realHome(source.home);
  if (!homeReal || !await plainChain(source.home, folder)) return [];
  const base = sessionsOf(source.home, folder);
  const entries = await readdir(base, { withFileTypes: true }).catch(() => []);
  const names = entries.filter(entry => entry.isDirectory() && validThread(entry.name)).map(entry => entry.name);
  const found = await mapLimit(names, IMPORT_LIMITS.parallel, async (name): Promise<ImportListItem | undefined> => {
    options.signal.throwIfAborted();
    const dir = join(base, name);
    if (!await plainDir(dir)) return undefined;
    const summary = await readSummary(homeReal, dir, folder, name);
    if (!summary) return undefined;
    const title = str(summary.generated_title)?.trim() || str(summary.session_summary)?.trim() || '';
    const updatedAt = isoOf(summary.updated_at);
    const messages = summary.num_chat_messages;
    return {
      thread: name, title: oneLine(title, 120), path: dir, ...(updatedAt ? { updatedAt } : {}),
      ...(typeof messages === 'number' && Number.isFinite(messages) && messages >= 0 ? { messages } : {}),
    };
  });
  const items = found.filter((item): item is ImportListItem => item !== undefined);
  items.sort((a, b) => Date.parse(b.updatedAt ?? '') - Date.parse(a.updatedAt ?? '') || 0);
  const active = await activeIds(source.home, homeReal);
  return items.slice(0, IMPORT_LIMITS.list).map(item => (active.has(item.thread) ? { ...item, maybeOpen: true as const } : item));
}

/** content 가 글자열이거나 text 조각 배열일 때의 글 */
function contentText(content: unknown): string {
  if (typeof content === 'string') return content;
  if (!Array.isArray(content)) return '';
  return content.map(item => str(record(item)?.text) ?? '').filter(Boolean).join('\n\n');
}

/** 고른 Grok 대화의 최근 기록. chat_history.jsonl 끝 8MB 창만 읽는다 */
export async function readGrok(source: ImportSource, folder: string, thread: string, options: ImportReadOptions): Promise<ExternalRecord> {
  if (!validThread(thread)) throw new EngineImportError('not-found');
  const homeReal = await realHome(source.home);
  if (!homeReal || !await plainChain(source.home, folder, thread)) throw new EngineImportError('not-found');
  const dir = join(sessionsOf(source.home, folder), thread);
  const summary = await readSummary(homeReal, dir, folder, thread);
  if (!summary) throw new EngineImportError('not-found');
  const history = join(dir, 'chat_history.jsonl');
  const historyInfo = await plainFileIn(homeReal, history);
  if (!historyInfo) throw new EngineImportError('not-found');
  const model = str(summary.current_model_id)?.trim();
  const title = (str(summary.generated_title)?.trim() || str(summary.session_summary)?.trim()) ?? '';
  const collector = new TurnCollector();
  try {
    const window = await openLineWindow(history, options.signal, { isUser: droppedUser, onUserDropped: () => collector.skipUser() }, historyInfo);
    for await (const line of window.lines) {
      const entry = parseLine(line);
      if (!entry) continue;
      const at = isoOf(entry.timestamp ?? entry.created_at);
      if (entry.type === 'user') {
        if (entry.synthetic_reason !== undefined && entry.synthetic_reason !== null) continue;
        let text = contentText(entry.content);
        const trimmed = text.trim();
        if (trimmed.startsWith('<user_query>') && trimmed.endsWith('</user_query>') && trimmed.length >= '<user_query></user_query>'.length) {
          // 사람 요청은 <user_query> 안 원문만 쓴다
          text = trimmed.slice('<user_query>'.length, -'</user_query>'.length).replace(/^\r?\n/, '').replace(/\r?\n$/, '');
        } else if ((entry.prompt_index === undefined || entry.prompt_index === null) && trimmed.startsWith('<user_info>')) {
          continue; // prompt_index 없는 <user_info> 시작 줄은 Grok 머리 글
        }
        if (text.trim()) collector.begin(text, at);
      } else if (entry.type === 'assistant') {
        collector.answer(contentText(entry.content), at);
        if (Array.isArray(entry.tool_calls)) collector.tool(entry.tool_calls.length);
      }
    }
    const done = collector.finish();
    return { turns: done.turns, omittedBefore: window.omittedBefore || done.trimmed, ...(model ? { model } : {}), ...(title ? { title: oneLine(title, 120) } : {}) };
  } catch (error) {
    if (error instanceof EngineImportError || (error as Error).name === 'AbortError') throw error;
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') throw new EngineImportError('not-found', undefined, { cause: error });
    throw new EngineImportError('read-failed', undefined, { cause: error });
  }
}

export interface GrokCopyResult {
  /** 확정한 사본 폴더 */
  path: string;
  /** 허용 목록 밖이거나 보통 파일이 아니어서 뺀 이름 수 */
  skipped: number;
}

/**
 * 출처 세션 폴더를 대상 홈의 같은 자리로 복사한다. 대상이 이미 있으면 덮지 않고 grok-exists.
 * 같은 부모 아래 임시 폴더에 복사 → rename 으로 확정 → verify(targetHome) 순서이고, verify 가 던지면 확정한 폴더를 지우고 grok-load-failed.
 * signal 이 abort 되면 복사를 멈추고 임시 폴더를 지운 뒤 AbortError 를 그대로 던진다.
 */
export async function copyGrokSession(
  source: ImportSource, folder: string, thread: string, targetHome: string, verify: (home: string) => Promise<void>, signal?: AbortSignal,
): Promise<GrokCopyResult> {
  if (source.engine !== 'grok' || !validThread(thread) || !targetHome) throw new EngineImportError('not-found');
  const from = join(sessionsOf(source.home, folder), thread);
  const parent = sessionsOf(targetHome, folder);
  const target = join(parent, thread);
  const homeReal = await realHome(source.home);
  if (!homeReal || !await plainChain(source.home, folder, thread)) throw new EngineImportError('not-found');
  if (await lstat(target).then(() => true, (error: NodeJS.ErrnoException) => { if (error.code === 'ENOENT') return false; throw error; })) {
    throw new EngineImportError('grok-exists');
  }
  const temp = join(parent, `${thread}.lair-import-${randomBytes(6).toString('hex')}`);
  let skipped = 0;
  try {
    await mkdir(parent, { recursive: true, mode: 0o700 });
    await mkdir(temp, { mode: 0o700 });
    for (const name of await readdir(from)) {
      signal?.throwIfAborted();
      const info = COPY_ALLOW.has(name) ? await plainFileIn(homeReal, join(from, name)) : undefined;
      if (!info) { skipped++; continue; }
      // 원본은 링크를 따라가지 않고 열어 흘려 쓴다(대상은 이미 있으면 실패)
      const input = await openRead(join(from, name), info);
      await pipeline(input.createReadStream(), createWriteStream(join(temp, name), { flags: 'wx', mode: 0o600 }), signal ? { signal } : {});
    }
    // 확정 직전에 한 번 더 본다. 그 사이 생겼으면 덮지 않는다
    const raced = await lstat(target).then(() => true, () => false);
    if (raced) throw new EngineImportError('grok-exists');
    await rename(temp, target);
  } catch (error) {
    await rm(temp, { recursive: true, force: true }).catch(() => undefined);
    if (error instanceof EngineImportError) throw error;
    if ((error as Error).name === 'AbortError') throw error;
    throw new EngineImportError('read-failed', undefined, { cause: error });
  }
  try {
    await verify(targetHome);
  } catch (error) {
    await rm(target, { recursive: true, force: true }).catch(() => undefined);
    if ((error as Error).name === 'AbortError') throw error;
    throw new EngineImportError('grok-load-failed', undefined, { cause: error });
  }
  return { path: target, skipped };
}

/** 경로가 root 와 같거나 그 안인가(둘 다 실제 경로) */
function sameOrInside(root: string, path: string): boolean {
  const rel = relative(root, path);
  return rel === '' || (!rel.startsWith('..') && !isAbsolute(rel));
}

/**
 * 되돌리기가 이어 열기로 만든 Grok 사본(`<대상 홈>/sessions/<폴더>/<번호>`)을 지운다. 지웠거나 원래 없으면 true, 안전하지 않아 두었거나 못 지우면 false.
 * 원본 홈과 같거나 그 안이면(링크로 가리켜도) 지우지 않는다. 보통 폴더(링크 아님)일 때만 지운다. 원본 터미널 기록은 건드리지 않는다.
 */
export async function removeGrokCopy(targetHome: string, sourceHome: string, folder: string, thread: string): Promise<boolean> {
  if (!validThread(thread) || !targetHome || !sourceHome) return false;
  try {
    const target = join(sessionsOf(targetHome, folder), thread);
    const info = await lstat(target).catch((error: NodeJS.ErrnoException) => { if (error.code === 'ENOENT') return null; throw error; });
    if (info === null) return true;
    if (!info.isDirectory() || info.isSymbolicLink()) return false;
    const targetReal = await realpath(targetHome);
    const parentReal = await realpath(join(sessionsOf(targetHome, folder)));
    const sourceReal = await realHome(sourceHome);
    // 원본 홈이 없으면 그 안일 수도 없다. 있으면 대상 홈과 부모 둘 다 그 밖이어야 한다.
    if (sourceReal && (sameOrInside(sourceReal, targetReal) || sameOrInside(sourceReal, parentReal))) return false;
    await rm(target, { recursive: true, force: true });
    return true;
  } catch { return false; }
}

/**
 * 사본이 열리는지 확인한다. grok ACP 를 GROK_HOME=home 으로 띄워 initialize 뒤 session/load 가 성공하면 통과.
 * loadSession 을 알리지 않거나 시간(20초) 안에 안 열리면 던진다. 끝나면 프로세스를 정리한다.
 */
export async function grokLoadCheck(
  deps: { binary: string; env: NodeJS.ProcessEnv }, home: string, folder: string, thread: string, signal: AbortSignal,
): Promise<void> {
  await withRpcChild({
    engine: 'grok', binary: deps.binary, args: ACP_ARGS, env: { ...deps.env, GROK_HOME: home }, cwd: folder,
    jsonrpc: true, signal, timeoutMs: IMPORT_LIMITS.rpcMs, detached: true,
  }, async rpc => {
    const init = record(await rpc.request('initialize', {
      protocolVersion: 1,
      clientCapabilities: { fs: { readTextFile: false, writeTextFile: false }, terminal: false },
      clientInfo: { name: 'lair', title: 'Lair', version: '0.1.0' },
    }));
    if (record(init?.agentCapabilities)?.loadSession !== true) throw new EngineImportError('grok-load-failed', 'no loadSession');
    await rpc.request('session/load', { sessionId: thread, cwd: folder, mcpServers: [] });
  });
}
