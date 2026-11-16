// 바깥 터미널 엔진 대화 이어 열기 요청 셋(docs/notes/engine-import-2026-10-19.md): 출처 보기·대화 목록·Lair 대화로 열기.
// 서비스·설정 전체를 import 하지 않고 좁은 의존(EngineImportDeps)만 받는다. 출처는 번호로만 받아 등록된 출처 안에서만 읽는다(경로 입력 없음).
// 이 컴퓨터 화면과 전체 권한 원격만 받는 판정은 관문(src/host/request-gate.ts)이 한다.
import { rm } from 'node:fs/promises';
import { basename } from 'node:path';
import { randomUUID } from 'node:crypto';
import { threadSlot } from '../../logic/records';
import type { SentFrom, Session, Turn } from '../../shared/contracts';
import type { EngineImportEngine, EngineImportListResult, EngineImportOpenRequest, EngineImportOpenResult, EngineImportSourcesResult, ExternalThread } from '../../shared/engine-import';
import type { EngineThread } from '../../shared/engine-thread';
import { WireError, wire } from '../../shared/i18n/wire';
import {
  EngineImportError, IMPORT_LIMITS, copyGrokSession, findSource, grokLoadCheck, importSources, listExternal, makeCodexRunDeps, readExternal,
  type ImportReadOptions, type ImportSource, type ImportSourceDeps,
} from '../engine-import';
import { excludedFromBound, sessionThreads } from '../engine-import/bound';
import { locateAcp } from '../engine-locate';
import { DEFAULT_ACCOUNT } from '../role-policy-resolve';
import type { AppRequestHandlers } from './types';

export type EngineImportKind = 'engine-import-sources' | 'engine-import-list' | 'engine-import-open';

/** 만들 대화의 씨앗. fill 은 첫 저장 전에 대화에 가져온 요청·결속·출처를 채운다. */
export interface EngineImportSeed {
  engine: EngineImportEngine;
  account?: string;
  model: string;
  folder: string;
  fill(session: Session): void;
}

/** 서비스가 화살표로 감싸 넘기는 의존. 쓰는 시점마다 부른다. */
export interface EngineImportDeps extends ImportSourceDeps {
  /** Grok 추가 계정 목록 */
  grokAccounts(): Array<{ id: string; name: string }>;
  /** 모든 대화(서비스 sessions.values(), 복사하지 않는다). */
  sessionList(): Iterable<Session>;
  /** 임시 대화인지(서비스 store.isTemporary). */
  isTemporary(id: string): boolean;
  /** 작업 폴더 검사(서비스 checkedProject와 같다). 실제 경로를 돌려주고 범위 밖이면 던진다. */
  checkFolder(path: string): string;
  /** 그 엔진·계정 모델 목록에 wanted 가 있으면 그것, 없으면 기본 모델 */
  modelFor(engine: EngineImportEngine, account: string | undefined, wanted?: string): string;
  /** 씨앗으로 새 대화를 만들어 저장하고 번호를 돌려준다. */
  createImported(seed: EngineImportSeed): string;
  /** 지금 요청이 AI 앱 동작 범위 안에서 도는가 */
  fromAppAction(): boolean;
  /** 이번 기동 임시 보관소 뿌리를 만들어 돌려준다. OpenCode 내보내기 임시 폴더를 여기 둔다. */
  tempRoot(): Promise<string>;
  /** 대상 Grok 계정(없으면 기본)의 GROK_HOME 과 사본 확인에 쓸 실행 파일·환경. 실행 파일이 없으면 던진다. */
  grokHomeFor(account?: string): Promise<{ home: string; binary: string; env: NodeJS.ProcessEnv }>;
}

/** EngineImportError 코드와 시간 초과를 사람용 문구 키가 있는 오류로 바꾼다. 그 밖의 오류는 그대로 던진다. */
function importFailure(error: unknown): never {
  if ((error as Error | undefined)?.name === 'AbortError') throw new WireError(wire('wireWork.engineImport.timeout'), '터미널 대화를 읽는 데 시간이 너무 오래 걸렸어.');
  if (error instanceof EngineImportError) {
    switch (error.code) {
      case 'not-found': throw new WireError(wire('wireWork.engineImport.notFound'), '터미널 대화를 찾을 수 없어.');
      case 'codex-unsupported': throw new WireError(wire('wireWork.engineImport.codexUnsupported'), '이 Codex 판은 바깥 대화 읽기를 지원하지 않아.');
      case 'grok-exists': throw new WireError(wire('wireWork.engineImport.grokExists'), 'Lair 의 Grok 폴더에 같은 번호의 대화가 이미 있어.');
      case 'grok-load-failed': throw new WireError(wire('wireWork.engineImport.grokLoadFailed'), '복사한 Grok 대화를 열지 못했어.');
      case 'timeout': throw new WireError(wire('wireWork.engineImport.timeout'), '터미널 대화를 읽는 데 시간이 너무 오래 걸렸어.');
      case 'engine-missing': throw new WireError(wire('wireWork.engineImport.engineMissing'), '이 엔진을 찾을 수 없어.');
      case 'read-failed': throw new WireError(wire('wireWork.engineImport.readFailed'), '터미널 대화 기록을 읽지 못했어.');
    }
  }
  throw error;
}

/** 읽기·확인 하나에 제한 시간(20초)을 건다. 오류 변환은 부른 쪽이 importFailure 로 한다. */
async function timed<T>(work: (signal: AbortSignal) => Promise<T>, cancel?: AbortSignal): Promise<T> {
  const controller = new AbortController();
  const onCancel = (): void => controller.abort();
  if (cancel?.aborted) controller.abort(); else cancel?.addEventListener('abort', onCancel, { once: true });
  const timer = setTimeout(() => controller.abort(), IMPORT_LIMITS.rpcMs);
  try { return await work(controller.signal); } finally { clearTimeout(timer); cancel?.removeEventListener('abort', onCancel); }
}

function sourceOf(deps: EngineImportDeps, source: string): ImportSource {
  const found = findSource(importSources(deps), source);
  if (!found) throw new WireError(wire('wireWork.engineImport.sourceUnknown'), '고를 수 없는 출처야.');
  return found;
}

function folderOf(deps: EngineImportDeps, folder: string): string {
  try { return deps.checkFolder(folder); } catch (error) {
    throw new WireError(wire('wireWork.engineImport.folder'), error instanceof Error ? error.message : '이 폴더는 쓸 수 없어.');
  }
}

/**
 * 이미 어느 대화에 묶인 바깥 번호인가. 메모리의 대화 머리만 훑고 요청 기록은 열지 않는다. 임시 대화는 뺀다.
 * 같은 엔진 슬롯의 engineThreads 번호이거나 externalThread 의 원래 번호·확정한 갈래 번호이면 그 대화 번호를 돌려준다. 목록과 만들기가 같은 함수를 쓴다.
 */
function boundSessionId(deps: EngineImportDeps, engine: EngineImportEngine, thread: string): string | undefined {
  for (const session of deps.sessionList()) {
    if (excludedFromBound(session, deps.isTemporary)) continue;
    for (const bound of sessionThreads(session)) if (bound.engine === engine && bound.thread === thread) return session.id;
  }
  return undefined;
}

/** 열기 대화 씨앗에 쓸 계정. Claude·Codex 는 출처 주인, Grok 은 고른 대상 계정(없거나 기본이면 기본 계정) */
function accountOf(deps: EngineImportDeps, source: ImportSource, target: string | undefined): string | undefined {
  if (source.engine !== 'grok') return source.account;
  if (!target || target === DEFAULT_ACCOUNT) return undefined;
  if (!deps.grokAccounts().some(account => account.id === target)) throw new WireError(wire('wireWork.engineImport.accountUnknown'), '고른 Grok 계정을 찾을 수 없어.');
  return target;
}

/** withBinary: OpenCode 기록을 내보내려고 실행 파일이 필요한 읽기(열기)일 때만 참. 못 찾으면 칸을 비워 읽기 쪽이 engine-missing 으로 끝낸다 */
async function readOptions(deps: EngineImportDeps, source: ImportSource, signal: AbortSignal, withBinary = false): Promise<ImportReadOptions> {
  const opencode = source.engine === 'opencode' && withBinary ? (await locateAcp('opencode')).binary : undefined;
  return { signal, env: deps.env, ...(source.engine === 'codex' ? { codex: await makeCodexRunDeps(source, deps.dataDir) } : {}), ...(opencode ? { opencode: { binary: opencode, tempRoot: await deps.tempRoot() } } : {}) };
}

/** kind → 처리 함수 표. 모양(글 길이·절대 경로·모드 값)은 입력 검사(inputs/engine-import.ts)가 본다. */
export function engineImportHandlers(deps: EngineImportDeps): AppRequestHandlers<EngineImportKind> & { readonly 'engine-import-open': (request: EngineImportOpenRequest, remote: boolean, tempVisible: boolean, caller?: SentFrom, cancel?: AbortSignal) => Promise<EngineImportOpenResult> } {
  /** 지금 열고 있는 바깥 대화(엔진:번호). 같은 번호를 동시에 열지 못하게 한다. */
  const opening = new Set<string>();
  return {
    'engine-import-sources': (): EngineImportSourcesResult => ({
      // home 경로는 내보내지 않는다. label 은 계정 이름이고 기본 출처는 빈 글이다(화면이 자기 문구를 쓴다).
      sources: importSources(deps).map(entry => ({ source: entry.source, engine: entry.engine, ...(entry.account ? { account: entry.account } : {}),
        label: entry.kind === 'account' ? entry.accountName ?? '' : '' })),
    }),
    'engine-import-list': async (request): Promise<EngineImportListResult> => {
      const source = sourceOf(deps, request.source);
      const folder = folderOf(deps, request.folder);
      try {
        const items = await timed(async signal => listExternal(source, folder, await readOptions(deps, source, signal)));
        const titles = new Map<string, string>();
        for (const session of deps.sessionList()) titles.set(session.id, session.title);
        return { items: items.map(item => {
          const bound = boundSessionId(deps, source.engine, item.thread);
          // 이미 Lair 대화에 묶인 줄은 Lair 대화 제목을 쓴다. 엔진 기록의 첫 글은 앱 문맥 글로 시작해 제목으로 알아보기 어렵다(10-19 실제 확인 캡처).
          const title = bound ? titles.get(bound) || item.title : item.title;
          return { thread: item.thread, title, ...(item.updatedAt !== undefined ? { updatedAt: item.updatedAt } : {}),
            ...(item.size !== undefined ? { size: item.size } : {}), ...(item.messages !== undefined ? { messages: item.messages } : {}),
            ...(item.path !== undefined ? { path: item.path } : {}), ...(bound ? { boundSessionId: bound } : {}), ...(item.maybeOpen ? { maybeOpen: true as const } : {}) };
        }) };
      } catch (error) { return importFailure(error); }
    },
    'engine-import-open': async (request, _remote, _tempVisible, _caller, cancel?: AbortSignal): Promise<EngineImportOpenResult> => {
      const source = sourceOf(deps, request.source);
      const folder = folderOf(deps, request.folder);
      const engine = source.engine;
      // 같은 번호 이어 쓰기는 기본 Claude·Codex·OpenCode·Qwen 출처에서만, 사람 화면 확인을 지난 요청만. AI 는 사람 확인을 대신하지 못한다.
      if (request.mode === 'same') {
        if (engine === 'grok' || source.kind !== 'home') throw new WireError(wire('wireWork.engineImport.sameRefused'), '원래 대화에 그대로 이어 쓰기는 기본 Claude·Codex·OpenCode·Qwen 에서만 돼.');
        if (deps.fromAppAction()) throw new WireError(wire('wireWork.engineImport.sameByAi'), '그대로 이어 쓰기는 사람이 확인해야 해서 AI 는 쓸 수 없어.');
      }
      const account = accountOf(deps, source, request.targetAccount);
      const known = boundSessionId(deps, engine, request.thread);
      if (known) return { sessionId: known, existing: true };
      const key = `${engine}:${request.thread}`;
      if (opening.has(key)) throw new WireError(wire('wireWork.engineImport.busy'), '같은 터미널 대화를 이미 여는 중이야.');
      opening.add(key);
      let copied: string | undefined;
      try {
        const external = await timed(async signal => readExternal(source, folder, request.thread, await readOptions(deps, source, signal, true)), cancel);
        if (engine === 'grok') {
          const target = await deps.grokHomeFor(account).catch((error: unknown) => { throw new EngineImportError('engine-missing', error instanceof Error ? error.message : undefined, { cause: error }); });
          const check = { binary: target.binary, env: target.env };
          copied = (await copyGrokSession(source, folder, request.thread, target.home, home => timed(signal => grokLoadCheck(check, home, folder, request.thread, signal), cancel), cancel)).path;
        }
        // 비동기가 끝난 뒤 저장 직전에 같은 함수로 다시 본다. 그 사이 묶였으면 이번에 만든 사본만 지우고 그 대화로 보낸다.
        if (cancel?.aborted) throw cancel.reason;
        const raced = boundSessionId(deps, engine, request.thread);
        if (raced) {
          if (copied) await rm(copied, { recursive: true, force: true }).catch(() => undefined);
          return { sessionId: raced, existing: true };
        }
        const now = new Date().toISOString();
        const turns: Turn[] = external.turns.map(item => ({
          id: randomUUID(), prompt: item.prompt, reply: item.reply, progress: '', status: 'completed' as const, usage: null,
          createdAt: item.createdAt ?? now, ...(item.completedAt !== undefined ? { completedAt: item.completedAt } : {}),
          imported: { engine, ...(item.tools > 0 ? { tools: item.tools } : {}), ...(item.clipped ? { clipped: true as const } : {}) },
        }));
        const lastTurnId = turns.at(-1)?.id;
        const bound: EngineThread = { threadId: request.thread, ...(lastTurnId ? { lastTurnId } : {}), ...(engine !== 'grok' && request.mode === 'fork' ? { fork: true as const } : {}) };
        const origin: ExternalThread = { engine, source: source.source, thread: request.thread, mode: request.mode, importedAt: now,
          ...(external.omittedBefore ? { omittedBefore: true as const } : {}),
          ...(external.lastPlan ? { lastPlan: external.lastPlan } : {}) };
        // 엔진이 붙인 제목이 없으면 첫 사람 글을 쓴다. 한 폴더에서 연 대화들이 폴더 이름으로 겹치지 않게 한다.
        const title = (external.title || external.turns[0]?.prompt || '').replace(/\s+/g, ' ').trim().slice(0, 80) || basename(folder) || folder;
        const sessionId = deps.createImported({ engine, ...(account ? { account } : {}), model: deps.modelFor(engine, account, external.model), folder,
          fill: session => {
            session.title = title;
            session.turns = turns;
            session.externalThread = origin;
            session.engineThreads = { ...session.engineThreads, [threadSlot(engine, account)]: bound };
          } });
        copied = undefined;
        return { sessionId };
      } catch (error) {
        if (copied) await rm(copied, { recursive: true, force: true }).catch(() => undefined);
        if (cancel?.aborted) throw cancel.reason;
        return importFailure(error);
      } finally { opening.delete(key); }
    },
  };
}
