// 바깥 터미널 엔진 대화 이어 열기의 백엔드 읽기·복사 입구(docs/notes/engine-import-2026-10-19.md).
// 출처 번호만 받아 등록된 출처 폴더 안에서만 읽는다(경로 입력 없음). 처리 함수는 필요한 값만 받는다.
import { homedir } from 'node:os';
import { join } from 'node:path';
import { accountHome } from './accounts';
import { codexAccountFolder, hostCodexHome } from './codex-home';
import { listClaude, readClaude } from './engine-import/claude';
import { listCodex, makeCodexRunDeps, readCodex } from './engine-import/codex';
import { copyGrokSession, grokLoadCheck, listGrok, readGrok } from './engine-import/grok';
import { listOpencode, opencodeDataDir, readOpencode, type OpencodeRun } from './engine-import/opencode';
import { listQwen, qwenRuntimeDir, readQwen } from './engine-import/qwen';
import {
  EngineImportError, IMPORT_LIMITS,
  type CodexRunDeps, type EngineImportErrorCode, type ExternalRecord, type ExternalTurn, type ImportListItem, type ImportReadOptions as BaseReadOptions, type ImportSource,
} from './engine-import/tail';

/** 목록·읽기 공통 옵션. opencode 는 OpenCode 기록을 내보낼 때 쓸 실행 파일이다(찾기에는 필요 없다). */
export type ImportReadOptions = BaseReadOptions & { opencode?: OpencodeRun };

export { EngineImportError, IMPORT_LIMITS, copyGrokSession, grokLoadCheck, makeCodexRunDeps };
export type { CodexRunDeps, EngineImportErrorCode, ExternalRecord, ExternalTurn, ImportListItem, ImportSource, OpencodeRun };

export interface ImportSourceDeps {
  dataDir: string;
  env: NodeJS.ProcessEnv;
  /** 그 엔진의 추가 계정 목록 */
  accounts(engine: 'claude' | 'codex'): Array<{ id: string; name: string }>;
}

/**
 * 고를 수 있는 출처. 번호는 claude:home·claude:<계정>·codex:home·codex:<계정>·grok:home·opencode:home·qwen:home 이다(OpenCode·Qwen 은 계정 없음).
 * Lair 앱 Grok 폴더는 출처가 아니다. 계정 폴더를 못 만드는 계정(번호 모양 틀림·기본 Codex 폴더와 같음)은 뺀다.
 */
export function importSources(deps: ImportSourceDeps): ImportSource[] {
  const out: ImportSource[] = [
    { source: 'claude:home', engine: 'claude', kind: 'home', home: deps.env.CLAUDE_CONFIG_DIR?.trim() || join(homedir(), '.claude') },
  ];
  for (const account of deps.accounts('claude')) {
    try {
      out.push({ source: `claude:${account.id}`, engine: 'claude', kind: 'account', account: account.id, accountName: account.name, home: accountHome(deps.dataDir, 'claude', account.id) });
    } catch { /* 번호 모양이 틀린 계정은 출처로 쓰지 않는다 */ }
  }
  out.push({ source: 'codex:home', engine: 'codex', kind: 'home', home: hostCodexHome() });
  for (const account of deps.accounts('codex')) {
    try {
      const home = codexAccountFolder(deps.dataDir, { id: account.id, home: accountHome(deps.dataDir, 'codex', account.id) });
      out.push({ source: `codex:${account.id}`, engine: 'codex', kind: 'account', account: account.id, accountName: account.name, home });
    } catch { /* 기본 계정 폴더와 같거나 번호 모양이 틀린 계정은 출처로 쓰지 않는다 */ }
  }
  out.push({ source: 'grok:home', engine: 'grok', kind: 'home', home: deps.env.GROK_HOME?.trim() || join(homedir(), '.grok') });
  out.push({ source: 'opencode:home', engine: 'opencode', kind: 'home', home: opencodeDataDir(deps.env) });
  out.push({ source: 'qwen:home', engine: 'qwen', kind: 'home', home: qwenRuntimeDir(deps.env, homedir()) });
  return out;
}

/** 출처 번호로 등록된 출처를 찾는다. 없으면 undefined */
export function findSource(sources: readonly ImportSource[], source: string): ImportSource | undefined {
  return sources.find(entry => entry.source === source);
}

/** 목록 줄 제목 글자 수 */
const TITLE_LIMIT = 200;

function rethrow(error: unknown): never {
  if (error instanceof EngineImportError || (error as Error | undefined)?.name === 'AbortError') throw error;
  throw new EngineImportError('read-failed', error instanceof Error ? error.message : undefined, { cause: error });
}

/**
 * 그 폴더의 바깥 대화 최근 50개. folder 는 이미 검증된 실제 절대 경로여야 한다.
 * Codex 는 opts.codex 로 띄울 app-server 를 받는다(makeCodexRunDeps).
 */
export async function listExternal(source: ImportSource, folder: string, opts: ImportReadOptions): Promise<ImportListItem[]> {
  try {
    const items = source.engine === 'claude' ? await listClaude(source, folder, opts)
      : source.engine === 'codex' ? await listCodex(source, folder, opts)
        : source.engine === 'opencode' ? await listOpencode(source, folder, opts)
          : source.engine === 'qwen' ? await listQwen(source, folder, opts)
            : await listGrok(source, folder, opts);
    // 제목은 한 줄 미리보기라 앞 200자만 싣는다(Codex preview 는 첫 사람 글 전체라 수십 KB 일 수 있다, 10-19 실측).
    return items.map(item => item.title.length > TITLE_LIMIT ? { ...item, title: item.title.slice(0, TITLE_LIMIT) } : item);
  } catch (error) { return rethrow(error); }
}

/** 고른 대화의 최근 기록(요청 200개·전체 2MB 안, 앞을 버렸으면 omittedBefore) */
export async function readExternal(source: ImportSource, folder: string, thread: string, opts: ImportReadOptions): Promise<ExternalRecord> {
  try {
    if (source.engine === 'claude') return await readClaude(source, folder, thread, opts);
    if (source.engine === 'codex') return await readCodex(source, folder, thread, opts);
    if (source.engine === 'opencode') return await readOpencode(source, folder, thread, opts);
    if (source.engine === 'qwen') return await readQwen(source, folder, thread, opts);
    return await readGrok(source, folder, thread, opts);
  } catch (error) { return rethrow(error); }
}
