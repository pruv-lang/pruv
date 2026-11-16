// OpenCode 바깥 대화 목록·기록 읽기(docs/notes/tool-migration-2026-11-03.md M11 설계 '찾기'·'기록 가져오기').
// 찾기는 데이터 폴더의 opencode.db 를 읽기 전용으로 한 번 열어 읽고 바로 닫는다(폴더 코드를 돌리지 않는다).
// 기록은 공식 `opencode --pure export <번호>` 를 데이터 폴더 cwd 로 돌려 받는다. tail.ts 와 엔진 실행 입구만 import 하고 서비스·설정 객체는 받지 않는다.
import type { ChildProcess } from 'node:child_process';
import { mkdtemp, open, readFile, realpath, rm, stat } from 'node:fs/promises';
import { homedir } from 'node:os';
import { dirname, isAbsolute, join } from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { spawnEngine } from '../engine-children';
import {
  EngineImportError, IMPORT_LIMITS, TurnCollector, isoOf, oneLine, plainFileIn, record, str, validThread,
  type ExternalRecord, type ImportListItem, type ImportReadOptions, type ImportSource,
} from './tail';
import { opencodePlan } from './plan';

type RecordValue = Record<string, unknown>;
type DbRow = Record<string, unknown>;

/** OpenCode 실행 파일(앱이 ACP 로 쓰는 것과 같은 것). 기록 내보내기에만 필요하고 찾기에는 필요 없다 */
export interface OpencodeRun {
  binary: string;
  /** 내보내기 임시 폴더를 만들 앱 임시 보관소 뿌리. 앱 종료 때 지워지고, 남으면 다음 기동이 치운다. */
  tempRoot: string;
}

/** 목록·읽기 옵션 + 내보내기에 쓸 실행 파일 */
export type OpencodeReadOptions = ImportReadOptions & { opencode?: OpencodeRun };

/** 내보내기 파일 크기 상한 */
const EXPORT_MAX = 32 * 1024 * 1024;
/** 이 시간 안에 갱신된 대화는 지금 열려 있을 수 있다고 알린다(Claude·Grok 은 살아 있는 프로세스로 보지만 OpenCode 는 그런 표지가 없다) */
export const OPEN_WINDOW_MS = 2 * 60 * 1000;
const SESSION_REQUIRED = ['id', 'parent_id', 'directory', 'title', 'time_updated'] as const;

/** 읽은 대화 한 줄 */
export interface OpencodeRow { id: string; directory: string; title: string; updatedMs: number }

/** OpenCode 데이터 폴더: XDG_DATA_HOME 이 절대 경로면 그 아래 opencode, 아니면 <집>/.local/share/opencode(Windows 도 같다) */
export function opencodeDataDir(env: NodeJS.ProcessEnv): string {
  const xdg = env.XDG_DATA_HOME?.trim();
  return xdg && isAbsolute(xdg) ? join(xdg, 'opencode') : join(homedir(), '.local', 'share', 'opencode');
}

/** DB 경로: OPENCODE_DB 가 :memory: 면 없음, 절대 경로면 그것, 상대 경로면 데이터 폴더 기준, 없으면 opencode.db */
function dbPathOf(home: string, env: NodeJS.ProcessEnv): string | undefined {
  const given = env.OPENCODE_DB?.trim();
  if (given === ':memory:') return undefined;
  if (!given) return join(home, 'opencode.db');
  return isAbsolute(given) ? given : join(home, given);
}

/** DB 파일이 링크 아닌 보통 파일이면 그 경로, 없으면 undefined */
async function dbFile(home: string, env: NodeJS.ProcessEnv): Promise<string | undefined> {
  const path = dbPathOf(home, env);
  if (!path) return undefined;
  const base = await realpath(dirname(path)).catch(() => undefined);
  return base !== undefined && await plainFileIn(base, path) ? path : undefined;
}

/** 폴더 비교 키: win32 는 / 를 \로 맞추고 끝 \를 떼 소문자로, 그 밖은 끝 / 만 뗀다 */
export function folderKey(folder: string): string {
  if (process.platform === 'win32') {
    const unified = folder.replace(/\//g, '\\');
    return (unified.replace(/\\+$/, '') || unified).toLowerCase();
  }
  return folder.replace(/\/+$/, '') || folder;
}

/** OpenCode 기본 제목(`New session - …`·`Child session - …`)은 제목 없음으로 본다 */
function cleanTitle(title: string | undefined): string {
  if (!title || title.startsWith('New session - ') || title.startsWith('Child session - ')) return '';
  return oneLine(title, 120);
}

function columns(db: DatabaseSync, table: string): Set<string> {
  const names = new Set<string>();
  for (const row of db.prepare('SELECT name FROM pragma_table_info(?)').all(table) as DbRow[]) {
    const name = row.name;
    if (typeof name === 'string') names.add(name);
  }
  return names;
}

/**
 * 뿌리 대화 중 보관하지 않았고 메시지가 있는 줄. where.id 를 주면 그 번호 줄만, where.folder 를 주면 그 폴더 줄만 최근 갱신 순으로 목록 상한까지(SQL 로 거른다).
 * win32 는 대소문자 없이 비교한다. 칸이 모자라면 추측하지 않고 read-failed
 */
function sessionRows(db: DatabaseSync, where: { id?: string; folder?: string } = {}): OpencodeRow[] {
  const session = columns(db, 'session');
  if (!SESSION_REQUIRED.every(name => session.has(name)) || !columns(db, 'message').has('session_id')) throw new EngineImportError('read-failed', 'columns');
  const archived = session.has('time_archived') ? ' AND s.time_archived IS NULL' : '';
  const params: Array<string | number> = [];
  let filter = '';
  if (where.id !== undefined) { filter += ' AND s.id = ?'; params.push(where.id); }
  if (where.folder !== undefined) {
    // 끝 구분자가 있는 값도 맞게 둘 다 비교한다. win32 는 DB 의 / 를 \로 맞춰 대소문자 없이, 그 밖은 끝 / 만 뗀다
    if (process.platform === 'win32') {
      const unified = where.folder.replace(/\//g, '\\');
      const base = unified.replace(/\\+$/, '') || unified;
      filter += " AND REPLACE(s.directory, '/', '\\') COLLATE NOCASE IN (?, ?)";
      params.push(base, `${base}\\`);
    } else {
      const base = where.folder.replace(/\/+$/, '') || where.folder;
      filter += ' AND s.directory IN (?, ?)';
      params.push(base, `${base}/`);
    }
  }
  const tail = where.folder === undefined ? '' : ' ORDER BY s.time_updated DESC LIMIT ?';
  if (where.folder !== undefined) params.push(IMPORT_LIMITS.list);
  const statement = db.prepare(
    `SELECT s.id AS id, s.directory AS directory, s.title AS title, s.time_updated AS updated FROM session s WHERE s.parent_id IS NULL${archived} AND EXISTS (SELECT 1 FROM message m WHERE m.session_id = s.id)${filter}${tail}`,
  );
  const rows: OpencodeRow[] = [];
  for (const row of statement.all(...params) as DbRow[]) {
    if (typeof row.id !== 'string' || !validThread(row.id) || typeof row.directory !== 'string' || !row.directory) continue;
    const updated = Number(row.updated);
    rows.push({ id: row.id, directory: process.platform === 'win32' ? row.directory.replace(/\//g, '\\') : row.directory, title: cleanTitle(typeof row.title === 'string' ? row.title : undefined), updatedMs: Number.isFinite(updated) ? updated : 0 });
  }
  return rows;
}

/** DB 를 읽기 전용으로 열어 work 를 돌리고 닫는다. 파일이 없으면 undefined. 열기·질의 예외는 read-failed */
async function withDb<T>(home: string, env: NodeJS.ProcessEnv, work: (db: DatabaseSync) => T): Promise<T | undefined> {
  const path = await dbFile(home, env);
  if (!path) return undefined;
  let db: DatabaseSync | undefined;
  try {
    db = new DatabaseSync(path, { readOnly: true });
    return work(db);
  } catch (error) {
    if (error instanceof EngineImportError) throw error;
    throw new EngineImportError('read-failed', error instanceof Error ? error.message : undefined, { cause: error });
  } finally {
    try { db?.close(); } catch { /* 이미 닫혔거나 열리지 못했다 */ }
  }
}

/** 훑기용: DB 의 모든 읽을 줄(폴더 거르기 없음). DB 가 없으면 빈 배열 */
export async function readOpencodeRows(home: string, env: NodeJS.ProcessEnv): Promise<OpencodeRow[]> {
  return (await withDb(home, env, sessionRows)) ?? [];
}

/** 이 폴더의 OpenCode 대화 최근 50개. 메시지 수는 뽑은 줄만 같은 연결에서 센다 */
export async function listOpencode(source: ImportSource, folder: string, options: OpencodeReadOptions): Promise<ImportListItem[]> {
  options.signal.throwIfAborted();
  const found = await withDb(source.home, options.env ?? process.env, db => {
    const rows = sessionRows(db, { folder });
    const count = db.prepare('SELECT COUNT(*) AS n FROM message WHERE session_id = ?');
    return rows.map(row => ({ row, messages: Number((count.get(row.id) as DbRow | undefined)?.n) }));
  });
  const now = Date.now();
  return (found ?? []).map(({ row, messages }): ImportListItem => {
    const updatedAt = row.updatedMs > 0 ? isoOf(row.updatedMs) : undefined;
    return {
      thread: row.id, title: row.title, ...(updatedAt ? { updatedAt } : {}),
      ...(Number.isFinite(messages) && messages >= 0 ? { messages } : {}),
      ...(row.updatedMs > 0 && now - row.updatedMs < OPEN_WINDOW_MS ? { maybeOpen: true as const } : {}),
    };
  });
}

function abortError(): Error {
  return Object.assign(new Error('aborted'), { name: 'AbortError' });
}

interface ExportOutput { over: boolean; text: string }

/** 자식이 끝날 때까지 기다린다. 제한 시간·중단이면 끄고, 끝난 뒤(또는 SIGKILL 뒤 잠깐)에 오류를 낸다. 출력 파일을 지우기 전에 자식이 놓게 한다. 종료 코드가 0 이 아니면 read-failed */
function waitExport(child: ChildProcess, signal: AbortSignal): Promise<void> {
  return new Promise<void>((resolve, reject) => {
    let settled = false;
    let failure: unknown;
    let killTimer: ReturnType<typeof setTimeout> | undefined;
    let giveUpTimer: ReturnType<typeof setTimeout> | undefined;
    const send = (name: 'SIGTERM' | 'SIGKILL'): void => {
      if (child.exitCode !== null || child.signalCode !== null) return;
      try {
        if (process.platform !== 'win32' && child.pid) process.kill(-child.pid, name);
        else child.kill(name);
      } catch { /* 이미 끝났다 */ }
    };
    const finish = (action: () => void): void => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      clearTimeout(killTimer);
      clearTimeout(giveUpTimer);
      signal.removeEventListener('abort', onAbort);
      action();
    };
    const halt = (reason: unknown): void => {
      if (failure !== undefined) return;
      failure = reason;
      send('SIGTERM');
      killTimer = setTimeout(() => {
        send('SIGKILL');
        giveUpTimer = setTimeout(() => finish(() => reject(reason)), 1_000);
      }, 1_500);
    };
    const onAbort = (): void => halt(abortError());
    const timer = setTimeout(() => halt(new EngineImportError('timeout')), IMPORT_LIMITS.rpcMs);
    if (signal.aborted) onAbort();
    else signal.addEventListener('abort', onAbort, { once: true });
    child.on('error', error => finish(() => reject(new EngineImportError('engine-missing', error.message, { cause: error }))));
    child.on('close', code => finish(() => {
      if (failure !== undefined) reject(failure);
      else if (code === 0) resolve();
      else reject(new EngineImportError('read-failed', `exit ${code}`));
    }));
  });
}

/**
 * `opencode --pure export <번호>` 의 출력을 받는다. 파이프로 받으면 1.19.5 가 196,608 바이트에서 끊고도 종료 코드 0 을 내서(11-04 실측, 파일로는 온전함)
 * 앱 임시 보관소(0700, 앱 종료 때 지워지고 남으면 다음 기동이 치움) 아래 mkdtemp 폴더의 파일로 받아 읽고 지운다. 크기가 상한을 넘으면 읽지 않고 over.
 */
async function runExport(binary: string, tempRoot: string, thread: string, home: string, env: NodeJS.ProcessEnv, signal: AbortSignal): Promise<ExportOutput> {
  const dir = await mkdtemp(join(tempRoot, 'opencode-'));
  try {
    const path = join(dir, 'export.json');
    const out = await open(path, 'wx', 0o600);
    try {
      let child: ChildProcess;
      try {
        child = spawnEngine('opencode', binary, ['--pure', 'export', thread], {
          cwd: home, env, shell: false, windowsHide: true, stdio: ['ignore', out.fd, 'ignore'], detached: process.platform !== 'win32',
        });
      } catch (error) {
        throw new EngineImportError('engine-missing', error instanceof Error ? error.message : String(error), { cause: error });
      }
      await waitExport(child, signal);
    } finally { await out.close(); }
    if ((await stat(path)).size > EXPORT_MAX) return { over: true, text: '' };
    return { over: false, text: await readFile(path, 'utf8') };
  } finally { await rm(dir, { recursive: true, force: true }).catch(() => undefined); }
}

/** parts 의 text 파트 글을 이은 것. skipMarked 면 synthetic·ignored 파트는 뺀다 */
function textOf(parts: readonly unknown[], skipMarked: boolean): string {
  const pieces: string[] = [];
  for (const raw of parts) {
    const part = record(raw);
    if (part?.type !== 'text' || typeof part.text !== 'string' || !part.text.trim()) continue;
    if (skipMarked && (part.synthetic === true || part.ignored === true)) continue;
    pieces.push(part.text);
  }
  return pieces.join('\n\n');
}

/** 내보내기 JSON({info, messages:[{info, parts}]})을 요청 목록으로 */
function convert(root: RecordValue, fallbackTitle: string): ExternalRecord {
  const messages = root.messages;
  if (!Array.isArray(messages)) throw new EngineImportError('read-failed', 'shape');
  const collector = new TurnCollector();
  const planInput: Array<{ parts: readonly unknown[]; at?: string }> = [];
  let model: string | undefined;
  // 사람 글이 없는 user 메시지(압축 등) 뒤의 assistant 는 앞 요청에 붙이지 않고 버린다
  let open = false;
  for (const raw of messages) {
    const message = record(raw);
    const info = record(message?.info);
    if (!message || !info || !Array.isArray(message.parts)) throw new EngineImportError('read-failed', 'shape');
    const time = record(info.time);
    const created = isoOf(time?.created);
    if (info.role === 'user') {
      const prompt = textOf(message.parts, true);
      open = prompt.trim() !== '';
      if (open) collector.begin(prompt, created);
    } else if (info.role === 'assistant') {
      if (info.summary === true) continue;
      const at = isoOf(time?.completed) ?? created;
      const provider = str(info.providerID);
      const modelId = str(info.modelID);
      if (provider && modelId) model = `${provider}/${modelId}`;
      planInput.push({ parts: message.parts, ...(at ? { at } : {}) });
      if (!open) continue;
      collector.answer(textOf(message.parts, false), isoOf(time?.completed));
      const tools = message.parts.filter(part => record(part)?.type === 'tool').length;
      if (tools > 0) collector.tool(tools);
    }
  }
  const done = collector.finish();
  const title = cleanTitle(str(record(root.info)?.title)) || fallbackTitle;
  const plan = opencodePlan(planInput);
  return { turns: done.turns, omittedBefore: done.trimmed, ...(model ? { model } : {}), ...(title ? { title } : {}), ...(plan ? { lastPlan: plan } : {}) };
}

/** 고른 OpenCode 대화의 최근 기록. 공식 내보내기를 돌려 받는다(출력이 32MiB 를 넘으면 기록 없이 앞부분 없음으로 연다) */
export async function readOpencode(source: ImportSource, folder: string, thread: string, options: OpencodeReadOptions): Promise<ExternalRecord> {
  if (!validThread(thread)) throw new EngineImportError('not-found');
  const env = options.env ?? process.env;
  const row = (await withDb(source.home, env, db => sessionRows(db, { id: thread })[0]));
  if (!row || folderKey(row.directory) !== folderKey(folder)) throw new EngineImportError('not-found');
  const binary = options.opencode?.binary;
  const tempRoot = options.opencode?.tempRoot;
  if (!binary || !tempRoot) throw new EngineImportError('engine-missing');
  const output = await runExport(binary, tempRoot, thread, source.home, env, options.signal);
  if (output.over) return { turns: [], omittedBefore: true, ...(row.title ? { title: row.title } : {}) };
  let parsed: unknown;
  try { parsed = JSON.parse(output.text); } catch (error) { throw new EngineImportError('read-failed', 'json', { cause: error }); }
  const root = record(parsed);
  if (!root) throw new EngineImportError('read-failed', 'shape');
  return convert(root, row.title);
}
