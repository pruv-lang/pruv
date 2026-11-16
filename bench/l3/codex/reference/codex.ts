// Codex 바깥 대화 목록·기록 읽기(docs/notes/engine-import-2026-10-19.md "엔진별 길"). 롤아웃 파일을 직접 해석하지 않고
// 단발 app-server 의 thread/list·thread/turns/list·thread/read 로 읽는다. app-server 는 이 요청 동안만 띄운다.
import { homedir } from 'node:os';
import { accountHome } from '../accounts';
import { childEnvironment, findCodex } from '../codex-engine';
import { prepareCodexAccount } from '../codex-home';
import {
  EngineImportError, IMPORT_LIMITS, RpcResponseError, TurnCollector, isoOf, oneLine, record, str, withRpcChild, validThread,
  type CodexRunDeps, type ExternalRecord, type ImportListItem, type ImportReadOptions, type ImportSource, type Rpc,
} from './tail';
import type { LastPlan } from '../../shared/engine-import';
import { folderKey } from './opencode';
import { codexPlan } from './plan';

type RecordValue = Record<string, unknown>;

const PAGE = 20;
/** 쪽이 비어도 끝없이 도는 것을 막는 상한 */
const PAGES_MAX = 40;
/** thread/list 가 기본으로 훑는 대화형 출처에 비대화형을 더한다(docs/notes/engine-import-2026-10-19.md 필수 9) */
export const SOURCE_KINDS = ['cli', 'vscode', 'exec', 'appServer'];

/** 출처에 맞는 binary·환경·인자. 추가 계정이면 그 계정 폴더 환경과 공통 인자를 쓴다. 연결(API 키) 계정의 키는 넘기지 않는다 */
export async function makeCodexRunDeps(source: ImportSource, dataDir: string): Promise<CodexRunDeps> {
  let install: Awaited<ReturnType<typeof findCodex>>;
  try { install = await findCodex(); } catch (error) {
    throw new EngineImportError('engine-missing', error instanceof Error ? error.message : String(error), { cause: error });
  }
  const base = childEnvironment(install);
  if (source.kind === 'account' && source.account) {
    const prepared = await prepareCodexAccount(dataDir, { id: source.account, home: accountHome(dataDir, 'codex', source.account) }, base);
    return { binary: install.binary, env: prepared.env, args: prepared.args };
  }
  return { binary: install.binary, env: { ...base, CODEX_HOME: source.home }, args: [] };
}

function importError(error: unknown): Error {
  if (error instanceof EngineImportError || (error as Error | undefined)?.name === 'AbortError') return error as Error;
  return new EngineImportError('read-failed', error instanceof Error ? error.message : undefined, { cause: error });
}

/** 단발 app-server 를 띄워 initialize 까지 마친 뒤 work 를 돈다. 끝나면 withRpcChild 가 프로세스를 정리한다. timeoutMs 를 주지 않으면 IMPORT_LIMITS.rpcMs */
export async function withCodex<T>(options: ImportReadOptions & { timeoutMs?: number }, work: (rpc: Rpc) => Promise<T>): Promise<T> {
  const deps = options.codex;
  if (!deps) throw new EngineImportError('engine-missing');
  try {
    return await withRpcChild({
      engine: 'codex', binary: deps.binary, args: ['app-server', '--stdio', ...deps.args], env: deps.env, cwd: homedir(),
      jsonrpc: false, signal: options.signal, timeoutMs: options.timeoutMs ?? IMPORT_LIMITS.rpcMs,
    }, async rpc => {
      await rpc.request('initialize', {
        clientInfo: { name: 'lair', title: 'Lair', version: '0.0.1' }, capabilities: { experimentalApi: true, requestAttestation: false },
      });
      rpc.notify('initialized', {});
      return work(rpc);
    });
  } catch (error) {
    throw importError(error);
  }
}

/** 이 폴더의 Codex 대화 최근 50개. 메서드가 없거나 거절되면 codex-unsupported. Windows 는 cwd 를 거르지 않고 쪽을 넘기며 대소문자 무시로 모은다 */
export async function listCodex(_source: ImportSource, folder: string, options: ImportReadOptions): Promise<ImportListItem[]> {
  return withCodex(options, async rpc => {
    const win = process.platform === 'win32';
    const data: unknown[] = [];
    let cursor: string | undefined;
    for (;;) {
      let result: RecordValue | undefined;
      try {
        result = record(await rpc.request('thread/list', {
          ...(win ? {} : { cwd: [folder] }), limit: IMPORT_LIMITS.list, sortKey: 'updated_at', useStateDbOnly: true, sourceKinds: SOURCE_KINDS,
          ...(cursor ? { cursor } : {}),
        }));
      } catch (error) {
        if (error instanceof RpcResponseError) throw new EngineImportError('codex-unsupported', error.message, { cause: error });
        throw error;
      }
      const page: unknown[] = Array.isArray(result?.data) ? result.data : Array.isArray(result?.threads) ? result.threads : [];
      // Windows 는 앱 서버의 cwd 거르기가 글자 그대로라 거르지 않고 받아 대소문자 무시로 거른다(B-621)
      data.push(...(win ? page.filter(raw => { const cwd = str(record(raw)?.cwd); return cwd !== undefined && folderKey(cwd) === folderKey(folder); }) : page));
      const next = str(result?.nextCursor);
      if (!win || data.length >= IMPORT_LIMITS.list || !next || page.length === 0 || next === cursor) break;
      cursor = next;
    }
    const items: ImportListItem[] = [];
    for (const raw of data.slice(0, IMPORT_LIMITS.list)) {
      const thread = record(raw);
      const id = str(thread?.id);
      if (!thread || !id || !validThread(id)) continue;
      const title = str(thread.name)?.trim() || str(thread.preview)?.trim() || '';
      const updatedAt = isoOf(thread.updatedAt);
      const path = str(thread.path);
      const originator = str(thread.originator);
      items.push({
        thread: id, title: oneLine(title, 120), ...(updatedAt ? { updatedAt } : {}), ...(path ? { path } : {}),
        ...(originator !== undefined ? { origin: /^lair/.test(originator) ? 'lair' as const : 'other' as const } : {}),
      });
    }
    return items;
  });
}

/** 항목 글 칸. content 가 text 조각 배열이거나 text 글자열 */
function itemText(item: RecordValue): string {
  if (typeof item.text === 'string') return item.text;
  if (!Array.isArray(item.content)) return '';
  return item.content.map(part => str(record(part)?.text) ?? '').filter(Boolean).join('\n\n');
}

interface RawTurn { prompt: string; replies: string[]; tools: number; planText?: string; createdAt?: string; completedAt?: string }

function readTurn(turn: RecordValue): RawTurn {
  const items = Array.isArray(turn.items) ? turn.items.map(record).filter((item): item is RecordValue => item !== undefined) : [];
  const prompts: string[] = [];
  const replies: string[] = [];
  let tools = 0;
  let planText: string | undefined;
  for (const item of items) {
    if (item.type === 'userMessage') {
      const text = itemText(item);
      if (text.trim()) prompts.push(text);
    } else if (item.type === 'agentMessage') {
      // 중간 설명(commentary)은 답으로 보지 않는다
      const text = item.phase === 'commentary' ? '' : itemText(item);
      if (text.trim()) replies.push(text);
    } else if (item.type === 'plan') {
      // 계획 항목은 도구로 세지 않는다. 한 턴에 여럿이면 마지막 글을 쓴다(앱 서버 스키마 ThreadItem plan {id, text})
      if (typeof item.text === 'string' && item.text.trim()) planText = item.text;
    } else if (item.type !== 'reasoning') {
      // 생각 항목은 도구로 세지 않는다
      tools++;
    }
  }
  const createdAt = isoOf(turn.startedAt);
  const completedAt = isoOf(turn.completedAt);
  return { prompt: prompts.join('\n\n'), replies, tools, ...(planText !== undefined ? { planText } : {}), ...(createdAt ? { createdAt } : {}), ...(completedAt ? { completedAt } : {}) };
}

/** 고른 Codex 대화의 최근 기록. 최신부터 쪽으로 받아 요청 200개·2MB 까지 */
export async function readCodex(_source: ImportSource, folder: string, thread: string, options: ImportReadOptions): Promise<ExternalRecord> {
  if (!validThread(thread)) throw new EngineImportError('not-found');
  return withCodex(options, async rpc => {
    // 대화의 작업 폴더가 고른 폴더와 같은 폴더일 때만(win32 는 대소문자 무시) 읽는다(모델·이름도 이 응답에서 얻는다)
    let meta: RecordValue;
    try {
      meta = record(record(await rpc.request('thread/read', { threadId: thread, includeTurns: false }))?.thread) ?? {};
    } catch (error) {
      if (rpc.failure) throw rpc.failure;
      if (!(error instanceof RpcResponseError)) throw error;
      throw new EngineImportError('not-found', error.message, { cause: error });
    }
    const cwd = str(meta.cwd);
    if (cwd === undefined || folderKey(cwd) !== folderKey(folder)) throw new EngineImportError('not-found');
    let model: string | undefined;
    let title: string | undefined;
    let lastPlan: LastPlan | undefined;
    let planSeen = false;
    const newestFirst: RawTurn[] = [];
    let bytes = 0;
    let cursor: string | undefined;
    let more = false;
    for (let page = 0; page < PAGES_MAX; page++) {
      options.signal.throwIfAborted();
      let result: RecordValue | undefined;
      try {
        result = record(await rpc.request('thread/turns/list', {
          threadId: thread, limit: PAGE, sortDirection: 'desc', itemsView: 'summary', ...(cursor ? { cursor } : {}),
        }));
      } catch (error) {
        if (!(error instanceof RpcResponseError)) throw error;
        // 메서드가 없으면 판 차이, 대화가 없으면 못 찾음
        if (error.code === -32601) throw new EngineImportError('codex-unsupported', error.message, { cause: error });
        if (/not found|no rollout|unknown thread/i.test(error.message)) throw new EngineImportError('not-found', error.message, { cause: error });
        throw new EngineImportError('read-failed', error.message, { cause: error });
      }
      const data = Array.isArray(result?.data) ? result.data : [];
      let full = false;
      for (const raw of data) {
        const turn = record(raw);
        if (!turn) continue;
        const read = readTurn(turn);
        // 최신부터 읽으므로 처음 만난 계획 글이 가장 늦은 것이다
        if (!planSeen && read.planText !== undefined) {
          planSeen = true;
          lastPlan = codexPlan(read.planText, read.completedAt ?? read.createdAt);
        }
        if (!read.prompt && read.replies.length === 0) continue;
        // 답은 32KB 로 잘려 담기므로 그 상한까지만 센다
        const replyBytes = read.replies.reduce((sum, text) => sum + Buffer.byteLength(text), 0);
        const size = Buffer.byteLength(read.prompt) + Math.min(replyBytes, IMPORT_LIMITS.reply);
        if (newestFirst.length >= IMPORT_LIMITS.turns || bytes + size > IMPORT_LIMITS.total) { full = true; break; }
        bytes += size;
        newestFirst.push(read);
      }
      const next = str(result?.nextCursor);
      if (full) { more = true; break; }
      if (!next || data.length === 0) break;
      cursor = next;
      if (page === PAGES_MAX - 1) more = true;
    }
    const collector = new TurnCollector();
    for (const turn of [...newestFirst].reverse()) {
      collector.begin(turn.prompt, turn.createdAt);
      for (const text of turn.replies) collector.answer(text, turn.completedAt);
      collector.tool(turn.tools);
    }
    const done = collector.finish();
    const modelId = typeof meta.model === 'string' ? meta.model : str(record(meta.model)?.id);
    if (modelId?.trim() && modelId.length <= 200) model = modelId.trim();
    const name = str(meta.name)?.trim();
    if (name) title = oneLine(name, 120);
    return { turns: done.turns, omittedBefore: more || done.trimmed, ...(model ? { model } : {}), ...(title ? { title } : {}), ...(lastPlan ? { lastPlan } : {}) };
  });
}
