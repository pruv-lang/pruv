// 옮겨 오기 실행: 고른 폴더를 프로젝트로 등록하고 대화를 갈라서 이어 연다(docs/notes/tool-migration-2026-11-03.md 'M2 설계').
// 서비스 객체를 받지 않고 좁은 의존(WorkRunDeps)만 받는다. 사람용 문구는 두지 않고 결과는 코드 값으로만 남긴다.
// 실행은 하나씩, 폴더는 차례대로, 취소는 폴더 사이와 대화 사이에서만 본다. 진행은 원장(migrate_runs·migrate_items)에 쓰고 읽기는 원장에서 읽는다.
import { randomUUID } from 'node:crypto';
import { homedir } from 'node:os';
import type { Session } from '../../shared/contracts';
import { ENGINE_IMPORT_ENGINES, type EngineImportEngine } from '../../shared/engine-import';
import { MCP_SOURCES, type McpSourceKind } from '../../shared/mcp-types';
import { wire, wireLocal, wireOf } from '../../shared/i18n/wire';
import type {
  MigrateHistoryView, MigrateItemCode, MigrateItemView, MigrateOutcome, MigrateRunState, MigrateRunSummary, MigrateRunView,
  MigrateThreadRow, MigrateThreadsView, MigrateUndoCode, MigrateUndoPreview, MigrateUndoResult, MigrateFirstWorkRequest, MigrateFirstWorkResult, MigrateWorkRunRequest, MigrateWorkRunResult, WorkScanOther, WorkScanRow,
  MigrateRulesApplyRequest, MigrateRulesApplyResult, RuleToolId, RuleWriteAction,
  MemoryPasteApp, MemoryToolId, MigrateMemoryApplyRequest, MigrateMemoryApplyResult, MigrateMemoryOrigin, MigrateMemoryPasteView, MigrateMemoryScanView, MigrateSuggestAddRow,
} from '../../shared/requests/migrate';
import { MEMORY_PASTE_APPS } from '../../shared/requests/migrate';
import type { CommonRulesResult } from '../../shared/rules-types';
import {
  EngineImportError, IMPORT_LIMITS, findSource, importSources, listExternal, makeCodexRunDeps,
  type ImportReadOptions, type ImportSource, type ImportSourceDeps,
} from '../engine-import';
import { boundSessionIndex } from '../engine-import/bound';
import { canonicalPath } from '../broad-scope';
import { comparePaths } from '../path-compare';
import { sensitivePath } from '../sensitive-paths';
import type { MigrateItemCompletion, MigrateItemRow, MigrateLedger, MigrateRunRow } from '../store-migrate';
import { createArchive } from './archive';
import { setArchiveFactsReader } from './archive-facts';
import { createWorkCheckup } from './work-checkup';
import { createWorkMcp, type WorkMcpPort } from './work-mcp';
import { createWorkMemory } from './work-memory';
import { createWorkRules, type WorkRulesPort } from './work-rules';
import { createWorkUndo, threadBaseline, type WorkUndoDeps, type WorkUndoSettings } from './work-undo';

/** 실행 항목을 한 번에 읽는 수 */
const ITEMS_PAGE = 200;
/** 지난 기록을 한 번에 읽는 수 */
const HISTORY_PAGE = 50;
/** 펼쳐 보는 대화를 한 번에 읽는 수 */
const THREADS_PAGE = 200;

/** 의존: 출처 목록에 쓰는 값, 대화 목록, 원장, 훑기 줄 찾기, 프로젝트 등록, 대화 열기 */
export interface WorkRunDeps extends ImportSourceDeps {
  sessionList(): Iterable<Session>;
  isTemporary(id: string): boolean;
  /** 번호로 대화 찾기 */
  session(id: string): Session | undefined;
  /** 원장(저장소). 저장소가 닫힌 뒤에는 던질 수 있다 */
  ledger(): MigrateLedger;
  /** 훑기 번호와 줄 번호로 줄을 다시 찾는다(없으면 undefined) */
  lineOf(scanId: string, rowId: string): WorkScanRow | undefined;
  /** 앱 작업 폴더가 이미 있는가(처음 설정은 없을 때만) */
  hasWorkspace(): boolean;
  /** 처음 설정의 '이 폴더 믿기'(first-run trust 와 같은 몸통). 거절은 던진다 */
  trustFirstFolder(path: string): Promise<unknown>;
  /** 폴더를 프로젝트로 등록하거나 이미 있는 프로젝트를 돌려준다(requests/projects.ts ensureProject) */
  ensureProject(path: string, trust: boolean): { project: { id: string; path: string }; created: boolean };
  /** 이 경로의 등록 프로젝트 번호(복구용) */
  projectIdOfPath(path: string): string | undefined;
  projectExists(projectId: string): boolean;
  /** 그 엔진 실행 파일이 이 컴퓨터에 있는가 */
  engineInstalled(engine: EngineImportEngine): Promise<boolean>;
  /** 바깥 대화를 갈라서 새 Lair 대화로 연다(engine-import-open 처리 함수, mode 는 늘 fork) */
  openThread(request: { source: string; folder: string; thread: string }, signal?: AbortSignal): Promise<{ sessionId: string; existing?: true }>;
  /** 되돌리기(work-undo.ts)에 넘기는 의존. 원장·대화 읽기는 위 칸을 같이 쓴다 */
  undo: Omit<WorkUndoDeps, 'ledger' | 'session' | 'sessionList' | 'mcp'> & {
    /** MCP 요청 묶음 가운데 옮겨 오기가 쓰는 다섯 함수(되돌리기는 그중 셋만 쓴다) */
    mcp: WorkMcpPort;
    /** 규칙 파일 읽기·바꿔 쓰기·지우기와 공통 규칙 요청(규칙 옮기기와 그 되돌림이 쓴다) */
    rules: WorkRulesPort;
  };
}

/** 오류의 기계 값(WireError.code·i18n 키)을 결과 까닭으로 바꾼다. 문구는 비교하지 않는다 */
const CODE_BY_ERROR_CODE: Record<string, MigrateItemCode> = {
  'no-workspace': 'no-workspace', 'broad-scope-consent': 'folder-broad', 'broad-scope-ai': 'folder-broad', 'folder-missing': 'folder-missing',
};
const CODE_BY_KEY: Record<string, MigrateItemCode> = {
  'wireWork.folder.trustRequired': 'folder-outside',
  'wireWork.folder.broadConsent': 'folder-broad',
  'wireWork.folder.broadAi': 'folder-broad',
  'wireWork.workspace.noneChosen': 'no-workspace',
  'wireRequests.service.projectLimit': 'project-limit',
  'wireWork.engineImport.notFound': 'not-found',
  'wireWork.engineImport.sourceUnknown': 'not-found',
  'wireWork.engineImport.codexUnsupported': 'codex-unsupported',
  'wireWork.engineImport.grokExists': 'grok-exists',
  'wireWork.engineImport.grokLoadFailed': 'grok-load-failed',
  'wireWork.engineImport.timeout': 'timeout',
  'wireWork.engineImport.engineMissing': 'engine-missing',
  'wireWork.engineImport.readFailed': 'read-failed',
  'wireWork.engineImport.busy': 'busy',
  'wireWork.engineImport.folder': 'folder-refused',
};
const ENGINE_IMPORT_CODES: Record<string, MigrateItemCode> = {
  'not-found': 'not-found', 'codex-unsupported': 'codex-unsupported', 'grok-exists': 'grok-exists', 'grok-load-failed': 'grok-load-failed',
  'timeout': 'timeout', 'engine-missing': 'engine-missing', 'read-failed': 'read-failed',
};
/** 사람의 손이 있으면 이어 갈 수 있는 까닭 */
const NEEDS_ACTION: ReadonlySet<MigrateItemCode> = new Set(['engine-missing', 'codex-unsupported', 'folder-outside', 'folder-broad', 'project-limit', 'no-workspace']);

function codeOf(error: unknown, fallback: MigrateItemCode): MigrateItemCode {
  if ((error as Error | undefined)?.name === 'AbortError') return 'timeout';
  if (error instanceof EngineImportError) return ENGINE_IMPORT_CODES[error.code] ?? fallback;
  const code = (error as { code?: unknown } | null)?.code;
  if (typeof code === 'string' && CODE_BY_ERROR_CODE[code]) return CODE_BY_ERROR_CODE[code];
  const key = wireOf(error)?.key;
  if (key && CODE_BY_KEY[key]) return CODE_BY_KEY[key];
  // 폴더 검사 오류는 문제 종류 칸(problem)을 싣는다
  const problem = (error as { problem?: unknown } | null)?.problem;
  if (problem === 'missing') return 'folder-missing';
  return fallback;
}

const outcomeOf = (code: MigrateItemCode): MigrateOutcome => (NEEDS_ACTION.has(code) ? 'needs_action' : 'failed');

/** 원장 항목 종류 -> 화면 종류(모르는 종류는 프로젝트로 본다) */
const itemKindOf = (kind: string): MigrateItemView['kind'] => (kind === 'thread' || kind === 'mcp' || kind === 'rule-file' || kind === 'common-rule' || kind === 'memory' ? kind : 'project');

const MEMORY_TOOLS: ReadonlySet<string> = new Set<MemoryToolId>(['claude', 'codex', 'qwen', 'gemini', 'grok', 'windsurf']);
const MEMORY_APPS: ReadonlySet<string> = new Set<string>(MEMORY_PASTE_APPS);

/** 원장 detail 의 origin 칸을 화면 모양으로(모르는 값은 버린다. 글은 원장에 없다) */
function memoryOriginOf(value: unknown): MigrateMemoryOrigin | undefined {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return undefined;
  const source = value as Record<string, unknown>;
  const text = (entry: unknown): string | undefined => (typeof entry === 'string' && entry ? entry : undefined);
  const tool = text(source.tool);
  const app = text(source.app);
  const display = text(source.display);
  const section = text(source.section);
  const origin: MigrateMemoryOrigin = {
    ...(tool && MEMORY_TOOLS.has(tool) ? { tool: tool as MemoryToolId } : {}), ...(app && MEMORY_APPS.has(app) ? { app: app as MemoryPasteApp } : {}),
    ...(display ? { display } : {}), ...(section ? { section } : {}),
  };
  return Object.keys(origin).length ? origin : undefined;
}

const RULE_ACTIONS: ReadonlySet<string> = new Set<RuleWriteAction>(['create', 'append', 'link-create', 'link-append', 'replace-link', 'strip']);
const RULE_TOOLS: ReadonlySet<string> = new Set<RuleToolId>(['cursor', 'windsurf', 'devin', 'gemini', 'copilot', 'aider', 'cline', 'roo']);

/** 한 폴더의 실행 입력: 훑기 줄에서 다시 찾은 폴더와 엔진, 고른 대화 */
interface RunRowPlan {
  folder: string;
  /** 훑기가 접은 까닭(사람 화면은 체크칸을 끈다. 임시·판정 못 함 폴더는 등록하지 않는다) */
  other?: WorkScanOther;
  engines: EngineImportEngine[];
  threads?: Array<{ source: string; thread: string }>;
  /** 프로젝트 항목 번호 */
  seq: number;
}

interface ThreadPick { source: string; thread: string; title?: string }

interface ListedRow {
  threads: MigrateThreadRow[];
  engines: MigrateThreadsView['engines'];
}

const zeroCounts = { imported: 0, already: 0, needsAction: 0, failed: 0, planned: 0, undone: 0, undoable: 0 };

export function createWorkRuns(deps: WorkRunDeps) {
  let disposed = false;
  let active: { runId: string; cancelled: boolean; abort: AbortController } | undefined;
  // 되돌리기는 옮기기와 MCP 가져오기가 도는 동안 돌지 않고(lock), 되돌리기가 도는 동안 옮기기를 시작하지 않는다.
  // 둘이 서로를 부르므로 만드는 순서는 지연 함수(호출할 때 읽는다)로 잇는다.
  const undo = createWorkUndo({ ...deps.undo, ledger: deps.ledger, session: deps.session, sessionList: deps.sessionList }, { busy: () => active !== undefined || mcpWork.running() || rulesWork.running() || memoryWork.running() });
  const mcpWork = createWorkMcp({
    lineOf: deps.lineOf, ledger: deps.ledger, mcp: deps.undo.mcp, projectIdOfPath: deps.projectIdOfPath, undoRunning: () => undo.running() || rulesWork.running() || memoryWork.running(), read: (runId, offset) => read(runId, offset),
  });
  // 점검(읽기만). undo.settings 는 앱 설정 전체를 돌려주므로 MCP 스위치 두 칸만 좁혀 읽는다
  const checkup = createWorkCheckup({
    lineOf: deps.lineOf, env: deps.env, dataDir: deps.dataDir, accounts: deps.accounts,
    mcpServers: () => deps.undo.mcp.mcpServerList(),
    settings: () => {
      const x = deps.undo.settings() as WorkUndoSettings & { claudeUserMcp?: boolean; codexUserMcp?: boolean };
      return { claudeUserMcp: x.claudeUserMcp, codexUserMcp: x.codexUserMcp };
    },
    rulesMove: (line, facts) => rulesWork.summary(line, facts),
    memoryMove: line => memoryWork.summary(line),
  });
  // 규칙 옮기기(읽기 쪽). 점검이 낸 판정 입력을 받아 쓰고, 점검 줄 요약은 위 checkup 이 부른다(서로를 지연 함수로 잇는다)
  const rulesWork = createWorkRules({
    lineOf: deps.lineOf, env: deps.env, sensitive: real => sensitivePath(real, { home: homedir(), dataDir: deps.dataDir, reuse: true }),
    commonRules: value => deps.undo.rules.commonRules(value), readTarget: path => deps.undo.rules.files.readRuleTarget(path),
    facts: line => checkup.rulesFacts(line),
    replaceFile: (path, version, build, kind) => deps.undo.rules.files.replaceRuleFile(path, version, build, kind),
    ledger: deps.ledger, undoRunning: () => undo.running() || active !== undefined || mcpWork.running() || memoryWork.running(), read: (runId, offset) => read(runId, offset),
  });
  // 보관함(M13): 앱 내보내기·Markdown 파일을 별도 DB 에 넣는다. 원장을 쓰지 않아 위 잠금과 얽히지 않고, 한 번에 하나는 보관함 안에서 지킨다.
  const archive = createArchive({ dataDir: deps.dataDir, sensitive: real => sensitivePath(real, { home: homedir(), dataDir: deps.dataDir, reuse: true }) });
  const unhookFacts = setArchiveFactsReader(offset => archive.sources(offset));
  // 기억 옮기기(다른 도구 기억 파일·붙여넣은 글 -> 공통 기억). 쓰기 잠금은 옮기기·MCP·규칙·되돌리기와 양쪽으로 건다
  const memoryWork = createWorkMemory({
    lineOf: deps.lineOf, env: deps.env, sensitive: real => sensitivePath(real, { home: homedir(), dataDir: deps.dataDir, reuse: true }),
    tables: deps.undo.store.commonMemory, projects: () => deps.undo.projectList(), projectIdOfPath: deps.projectIdOfPath,
    ledger: deps.ledger, undoRunning: () => undo.running() || active !== undefined || mcpWork.running() || rulesWork.running(), read: (runId, offset) => read(runId, offset),
    memoryChanged: () => deps.undo.memoryChanged(),
  });
  /** 도는 대화 목록·읽기 호출의 중단 신호(닫을 때 한꺼번에 끊는다) */
  const controllers = new Set<AbortController>();

  /** 원장 호출. 닫은 뒤의 오류는 삼킨다(저장소가 이미 닫혔을 수 있다) */
  const led = <T>(work: (ledger: MigrateLedger) => T, fallback: T): T => {
    if (disposed) { try { return work(deps.ledger()); } catch { return fallback; } }
    return work(deps.ledger());
  };

  /** 읽기 하나에 제한 시간(20초)을 건다. 타이머는 끝나면 늘 푼다 */
  async function timed<T>(work: (signal: AbortSignal) => Promise<T>): Promise<T> {
    const controller = new AbortController();
    controllers.add(controller);
    const timer = setTimeout(() => controller.abort(), IMPORT_LIMITS.rpcMs);
    try { return await work(controller.signal); } finally { clearTimeout(timer); controllers.delete(controller); }
  }

  async function readOptions(source: ImportSource, signal: AbortSignal): Promise<ImportReadOptions> {
    return { signal, env: deps.env, ...(source.engine === 'codex' ? { codex: await makeCodexRunDeps(source, deps.dataDir) } : {}) };
  }

  /** 설치 판정을 호출 묶음 안에서 엔진당 한 번만 한다 */
  const installProbe = (): ((engine: EngineImportEngine) => Promise<boolean>) => {
    const seen = new Map<EngineImportEngine, Promise<boolean>>();
    return engine => {
      let known = seen.get(engine);
      if (!known) { known = deps.engineInstalled(engine); seen.set(engine, known); }
      return known;
    };
  };

  /** 줄의 엔진 출처를 다 읽어 최근 순 대화 목록을 만든다. 설치 안 된 엔진은 읽지 않고 missing 으로 알린다 */
  async function listRow(folder: string, engines: readonly EngineImportEngine[], installed: (engine: EngineImportEngine) => Promise<boolean>): Promise<ListedRow> {
    const wanted = new Set(engines);
    const state = new Map<EngineImportEngine, 'ok' | 'missing' | 'failed'>();
    const threads: MigrateThreadRow[] = [];
    const boundIndex = boundSessionIndex(deps.sessionList(), deps.isTemporary);
    for (const source of importSources(deps)) {
      if (!wanted.has(source.engine) || disposed) continue;
      if (!(await installed(source.engine))) { if (!state.has(source.engine)) state.set(source.engine, 'missing'); continue; }
      try {
        const items = await timed(async signal => listExternal(source, folder, await readOptions(source, signal)));
        state.set(source.engine, 'ok');
        for (const item of items) {
          const bound = boundIndex.get(`${source.engine}:${item.thread}`);
          threads.push({
            source: source.source, engine: source.engine, ...(source.accountName ? { accountName: source.accountName } : {}), thread: item.thread,
            title: (bound ? deps.session(bound)?.title : undefined) || item.title,
            ...(item.updatedAt !== undefined ? { updatedAt: item.updatedAt } : {}), ...(item.messages !== undefined ? { messages: item.messages } : {}),
            ...(bound ? { boundSessionId: bound } : {}), ...(item.maybeOpen ? { maybeOpen: true as const } : {}),
          });
        }
      } catch { if (state.get(source.engine) !== 'ok') state.set(source.engine, 'failed'); }
    }
    threads.sort((a, b) => (b.updatedAt ?? '').localeCompare(a.updatedAt ?? ''));
    return { threads, engines: [...state].map(([engine, value]) => ({ engine, state: value })) };
  }

  // ---- 읽기 ----

  function itemView(item: MigrateItemRow): MigrateItemView {
    const detail = item.detail;
    const text = (value: unknown): string | undefined => (typeof value === 'string' && value ? value : undefined);
    const view: MigrateItemView = {
      seq: item.seq, kind: itemKindOf(item.kind), folder: text(detail.folder) ?? '', state: item.state === 'done' ? 'done' : 'planned',
    };
    if (item.outcome) view.outcome = item.outcome as MigrateOutcome;
    if (item.code) view.code = item.code as MigrateItemCode;
    // 되돌리기 결과(원장 undo 칸). 되돌린 항목은 대상이 없는 게 당연해 targetGone 을 붙이지 않는다
    const undone = typeof item.undo === 'object' && item.undo !== null ? item.undo as Record<string, unknown> : {};
    if (typeof undone.undoneAt === 'string') view.undo = 'undone';
    else if (typeof undone.skipped === 'string') { view.undo = 'skipped'; view.undoCode = undone.skipped as MigrateUndoCode; }
    if (undone.copyLeft === true) view.copyLeft = true;
    const engine = text(detail.engine);
    const knownEngine = ENGINE_IMPORT_ENGINES.find(candidate => candidate === engine);
    if (knownEngine) view.engine = knownEngine;
    const title = text(detail.title);
    if (title) view.title = title;
    const projectId = view.kind === 'project' ? item.targetRef : text(detail.projectId);
    if (projectId) view.projectId = projectId;
    if (view.kind === 'thread' && item.targetRef) view.sessionId = item.targetRef;
    if (detail.omittedBefore === true) view.omittedBefore = true;
    // 규칙 항목: 보여 줄 경로·쓰기 모양·출처·공통 규칙 적용 결과. 되돌림 칸의 before(원본 바이트)는 내보내지 않는다
    if (view.kind === 'rule-file' || view.kind === 'common-rule') {
      const display = text(detail.display);
      if (display) view.display = display;
      if (typeof detail.action === 'string' && RULE_ACTIONS.has(detail.action)) view.action = detail.action as RuleWriteAction;
      if (Array.isArray(detail.sources)) {
        const sources: NonNullable<MigrateItemView['sources']> = [];
        for (const entry of detail.sources) {
          if (typeof entry !== 'object' || entry === null) continue;
          const source = entry as Record<string, unknown>;
          const sourceDisplay = text(source.display);
          if (!sourceDisplay) continue;
          const tool = text(source.tool);
          sources.push({
            display: sourceDisplay, ...(tool && RULE_TOOLS.has(tool) ? { tool: tool as RuleToolId } : {}), ...(source.conditionLost === true ? { conditionLost: true as const } : {}),
          });
        }
        if (sources.length) view.sources = sources;
      }
      if (Array.isArray(detail.applied)) view.applied = detail.applied as CommonRulesResult[];
    }
    // 기억 항목: 출처·범위·고쳤는지만(글은 원장에 없다). target_ref 는 저장한 기억 번호
    if (view.kind === 'memory') {
      const origin = memoryOriginOf(detail.origin);
      if (origin) view.origin = origin;
      if (detail.scope === 'global' || detail.scope === 'project') view.memoryScope = detail.scope;
      if (detail.edited === true) view.edited = true;
      if (item.targetRef) view.memoryId = item.targetRef;
    }
    const sourceKind = text(detail.sourceKind);
    if (view.kind === 'mcp' && sourceKind && Object.hasOwn(MCP_SOURCES, sourceKind)) view.sourceKind = sourceKind as McpSourceKind;
    if (view.undo !== 'undone' && item.state === 'done' && item.targetRef && (item.outcome === 'imported' || item.outcome === 'already')) {
      const target = item.targetRef;
      if (view.kind === 'project') { if (!deps.projectExists(target)) view.targetGone = true; }
      else if (view.kind === 'mcp') { if (!deps.undo.mcp.mcpServerList().some(server => server.id === target)) view.targetGone = true; }
      else if (view.kind === 'thread' && !deps.session(target)) view.targetGone = true;
      else if (view.kind === 'memory' && !deps.undo.store.commonMemory.get(target)) view.targetGone = true;
    }
    return view;
  }

  const summaryOf = (run: MigrateRunRow): MigrateRunSummary => ({
    id: run.id, kind: run.kind === 'mcp' ? 'mcp' : run.kind === 'rules' ? 'rules' : run.kind === 'memory' ? 'memory' : 'work', startedAt: run.startedAt, ...(run.endedAt ? { endedAt: run.endedAt } : {}),
    state: run.state as MigrateRunState, folders: run.folders, counts: { ...run.counts },
  });

  function read(runId: string, offset: number): MigrateRunView {
    const run = led(ledger => ledger.readRun(runId), undefined);
    if (!run) {
      return { id: runId, kind: 'work', startedAt: '', state: 'failed', folders: 0, counts: { ...zeroCounts }, gone: true, items: [], nextOffset: offset, more: false };
    }
    const rows = led(ledger => ledger.readItems(runId, offset, ITEMS_PAGE + 1), [] as MigrateItemRow[]);
    const page = rows.slice(0, ITEMS_PAGE);
    return { ...summaryOf(run), items: page.map(itemView), nextOffset: offset + page.length, more: rows.length > ITEMS_PAGE };
  }

  // ---- 실행 ----

  async function runFolder(runId: string, plan: RunRowPlan, state: { cancelled: boolean; abort: AbortController }, trust: boolean, next: () => number, installed: (engine: EngineImportEngine) => Promise<boolean>): Promise<void> {
    const complete = (seq: number, result: MigrateItemCompletion): void => led(ledger => ledger.completeItem(runId, seq, result), undefined);
    const insert = (seq: number, kind: 'thread', sourceRef: string, detail: Record<string, unknown>): void => led(ledger => ledger.insertItem(runId, { seq, kind, sourceRef, detail }), undefined);
    if (state.cancelled) { complete(plan.seq, { outcome: 'skipped', code: 'cancelled' }); return; }
    if (plan.other === 'temporary' || plan.other === 'unknown') { complete(plan.seq, { outcome: 'failed', code: 'folder-refused' }); return; }

    // 기록에서 온 경로는 사람이 고칠 길이 없어, Windows 에서 대소문자만 다르면 디스크의 실제 표기로 바꿔 쓴다(B-622). 사람이 입력한 경로의 거절(B-611)은 그대로다.
    let folder = plan.folder;
    if (process.platform === 'win32') {
      try { const real = canonicalPath(folder); if (comparePaths(real, folder) === 'case') folder = real; } catch { /* 없거나 못 여는 폴더는 등록 판정이 알린다 */ }
    }

    let projectId: string;
    try {
      const { project, created } = deps.ensureProject(folder, trust);
      projectId = project.id;
      complete(plan.seq, { outcome: created ? 'imported' : 'already', targetRef: projectId, detail: { folder, projectId }, ...(created ? { undo: { created: true, path: project.path } } : {}) });
    } catch (error) {
      const code = codeOf(error, 'folder-refused');
      complete(plan.seq, { outcome: outcomeOf(code), code });
      return;
    }

    // 고를 대화: 칸이 있으면 그것, 없으면 그 줄의 엔진 출처를 다 읽어 가장 최근 하나, 빈 배열이면 프로젝트만
    let picks: ThreadPick[] = [];
    if (plan.threads) picks = plan.threads;
    else {
      const listed = await listRow(folder, plan.engines, installed);
      const first = listed.threads.find(thread => !thread.boundSessionId) ?? listed.threads[0];
      if (first) picks = [{ source: first.source, thread: first.thread, title: first.title }];
      // 읽지 못한 엔진은 대화가 있어도 몰라서 그 엔진당 한 줄로 알린다. 번호가 없어 복구 대상이 아니다.
      for (const entry of listed.engines) {
        if (entry.state === 'ok') continue;
        const seq = next();
        insert(seq, 'thread', `${entry.engine}\n`, { folder, engine: entry.engine, projectId });
        complete(seq, entry.state === 'missing' ? { outcome: 'needs_action', code: 'engine-missing' } : { outcome: 'failed', code: 'read-failed' });
      }
    }

    for (const pick of picks) {
      if (disposed) return;
      const source = findSource(importSources(deps), pick.source);
      const seq = next();
      insert(seq, 'thread', `${pick.source}\n${pick.thread}`, { folder, ...(source ? { engine: source.engine } : {}), ...(pick.title ? { title: pick.title } : {}), projectId });
      if (state.cancelled) { complete(seq, { outcome: 'skipped', code: 'cancelled' }); continue; }
      if (!source) { complete(seq, { outcome: 'failed', code: 'not-found' }); continue; }
      if (!(await installed(source.engine))) { complete(seq, { outcome: 'needs_action', code: 'engine-missing' }); continue; }
      try {
        const opened = await deps.openThread({ source: pick.source, folder, thread: pick.thread }, state.abort.signal);
        const session = deps.session(opened.sessionId);
        const title = pick.title || session?.title;
        complete(seq, {
          outcome: opened.existing ? 'already' : 'imported', targetRef: opened.sessionId,
          detail: { folder, engine: source.engine, ...(title ? { title } : {}), projectId, ...(session?.externalThread?.omittedBefore ? { omittedBefore: true } : {}) },
          // 가져온 직후 대화 설정을 같이 적어 두어, 이후 사람이 바꾼 설정을 되돌리기가 알아보게 한다. 대화를 못 읽으면 undo 를 적지 않아 되돌림 대상에서 빠진다.
          ...(opened.existing || !session ? {} : { undo: { created: true, baseline: threadBaseline(session) } }),
        });
      } catch (error) {
        if (state.cancelled && (error as Error | undefined)?.name === 'AbortError') { complete(seq, { outcome: 'skipped', code: 'cancelled' }); continue; }
        const code = codeOf(error, 'open-failed');
        complete(seq, { outcome: outcomeOf(code), code });
      }
    }
  }

  /** 뒤에서 도는 본체. 어떤 예외도 밖으로 새지 않는다 */
  async function execute(runId: string, plans: RunRowPlan[], trust: boolean, state: { runId: string; cancelled: boolean; abort: AbortController }): Promise<void> {
    let seq = plans.length;
    const next = (): number => ++seq;
    let ending: MigrateRunState = 'done';
    try {
      const installed = installProbe();
      for (const plan of plans) {
        // 앱을 닫는 중이면 원장을 건드리지 않고 멈춘다. 남은 항목은 다음 기동의 복구가 맞춘다.
        if (disposed) return;
        await runFolder(runId, plan, state, trust, next, installed);
      }
      if (disposed) return;
      if (state.cancelled) ending = 'cancelled';
    } catch { ending = 'failed'; }
    finally { if (active === state) active = undefined; }
    try { led(ledger => ledger.finishRun(runId, ending, new Date().toISOString()), undefined); } catch { /* 원장을 못 쓰면 복구가 맡는다 */ }
  }

  /** 실행을 시작하고 곧바로 첫 모습을 돌려준다. 이미 도는 실행이 있으면 busy, 훑기나 줄이 없으면 scan-gone. 사람이든 AI 든 같게 믿고 등록한다 */
  function start(request: MigrateWorkRunRequest): MigrateWorkRunResult {
    if (disposed || active || undo.running() || rulesWork.running() || memoryWork.running()) return { refused: 'busy' };
    const seen = new Set<string>();
    const plans: RunRowPlan[] = [];
    for (const row of request.rows) {
      if (seen.has(row.rowId)) continue;
      seen.add(row.rowId);
      const line = deps.lineOf(request.id, row.rowId);
      if (!line) return { refused: 'scan-gone' };
      plans.push({ folder: line.folder, ...(line.other ? { other: line.other } : {}), engines: line.engines.map(entry => entry.engine), ...(row.threads ? { threads: row.threads } : {}), seq: plans.length + 1 });
    }
    const runId = randomUUID();
    led(ledger => {
      ledger.insertRun({ id: runId, kind: 'work', startedAt: new Date().toISOString(), folders: plans.length });
      for (const plan of plans) ledger.insertItem(runId, { seq: plan.seq, kind: 'project', sourceRef: plan.folder, detail: { folder: plan.folder, ...(deps.projectIdOfPath(plan.folder) ? { existed: true } : {}) } });
    }, undefined);
    const state = { runId, cancelled: false, abort: new AbortController() };
    active = state;
    void execute(runId, plans, true, state);
    return { run: read(runId, 0) };
  }

  return {
    start,
    /**
     * 처음 설정: 훑기 줄 하나의 폴더를 앱 작업 폴더로 믿은 뒤 그 줄을 옮기기로 시작한다. 요청은 경로를 받지 않고 줄에서 찾는다.
     * 믿기 전 거절이면 아무것도 바뀌지 않고, 믿기 거절은 그대로 던진다. 믿은 뒤 시작만 거절되면 trusted 를 함께 돌려준다.
     */
    async startFirst(request: MigrateFirstWorkRequest): Promise<MigrateFirstWorkResult> {
      if (disposed || active || undo.running() || rulesWork.running() || memoryWork.running()) return { refused: 'busy' };
      if (deps.hasWorkspace()) return { refused: 'has-workspace' };
      const line = deps.lineOf(request.id, request.rowId);
      if (!line || line.other || line.broad) return { refused: 'scan-gone' };
      await deps.trustFirstFolder(line.folder);
      const started = start({ kind: 'migrate-work-run', id: request.id, rows: [{ rowId: request.rowId }] });
      return 'refused' in started ? { refused: started.refused, trusted: true } : started;
    },
    /**
     * 처음 설정 추천 폴더 추가: 백엔드가 직접 찾은 추천 후보(suggestable)만 믿고 프로젝트로 등록한다. 후보가 아니면 refused, 등록 실패는 그 폴더만 failed 로 두고 나머지는 계속한다.
     * 이미 등록된 폴더는 already. 사람용 문구(reason)는 다국어 키로 만든다. 원장(실행 기록)은 남기지 않는다.
     */
    suggestAdd(folders: readonly string[], suggestable: (folder: string) => boolean): MigrateSuggestAddRow[] {
      const out: MigrateSuggestAddRow[] = [];
      const seen = new Set<string>();
      for (const path of folders) {
        if (seen.has(path)) continue;
        seen.add(path);
        if (disposed || !suggestable(path)) { const text = wire('wireRequests.suggest.notCandidate'); out.push({ path, outcome: 'refused', reason: wireLocal(text, ''), reasonI18n: text }); continue; }
        try {
          const { created } = deps.ensureProject(path, true);
          out.push({ path, outcome: created ? 'added' : 'already' });
        } catch (error) {
          const fallback = error instanceof Error ? error.message : '';
          const text = wireOf(error) ?? wire('wireRequests.suggest.addFailed');
          out.push({ path, outcome: 'failed', reason: wireLocal(text, fallback), reasonI18n: text });
        }
      }
      return out;
    },
    read,
    /** 되돌리기 미리보기: 지금 되돌리면 무엇이 지워지고 무엇을 건너뛰는지(저장하지 않는다) */
    undoPreview(runId: string, offset: number): Promise<MigrateUndoPreview> { return undo.preview(runId, offset); },
    /** 되돌리기 실행: 끝날 때까지 기다린 뒤 결과 모습을 돌려준다 */
    async undoRun(runId: string): Promise<MigrateUndoResult> {
      const result = await undo.run(runId);
      return 'refused' in result ? result : { run: read(runId, 0), ...result };
    },
    /** 승인 카드 이름표용: 되돌릴 항목(이 실행이 만들었고 아직 안 되돌린)의 폴더 이름. 겹침 없이 */
    undoFolders(runId: string): string[] {
      const folders = led(ledger => ledger.undoItems(runId), [] as MigrateItemRow[])
        .filter(item => { const record = item.undo as Record<string, unknown> | undefined; return record?.created === true && record.undoneAt === undefined; })
        .map(item => item.detail.folder);
      return [...new Set(folders.filter((folder): folder is string => typeof folder === 'string' && folder !== ''))];
    },
    /** 도는 실행을 멈추게 한다. 읽는 중인 대화는 끊고 건너뛰며(저장을 마친 대화는 그대로), 남은 폴더·대화는 건너뛴다. 지금 상태를 돌려준다 */
    cancel(runId: string): MigrateRunView {
      if (active && active.runId === runId) { active.cancelled = true; active.abort.abort(); }
      return read(runId, 0);
    },
    history(offset: number): MigrateHistoryView {
      const rows = led(ledger => ledger.listRuns(offset, HISTORY_PAGE + 1), [] as MigrateRunRow[]);
      const page = rows.slice(0, HISTORY_PAGE);
      return { runs: page.map(summaryOf), nextOffset: offset + page.length, more: rows.length > HISTORY_PAGE };
    },
    /** 훑기의 폴더 줄 하나에서 이어 갈 수 있는 대화 목록(최근 순, 200개씩) */
    async threads(scanId: string, rowId: string, offset = 0): Promise<MigrateThreadsView> {
      const line = deps.lineOf(scanId, rowId);
      if (!line) return { gone: true, rowId, folder: '', threads: [], nextOffset: offset, more: false, engines: [] };
      const listed = await listRow(line.folder, line.engines.map(entry => entry.engine), installProbe());
      const at = Math.min(Math.max(0, Math.floor(offset)), listed.threads.length);
      const page = listed.threads.slice(at, at + THREADS_PAGE);
      return { rowId, folder: line.folder, threads: page, nextOffset: at + page.length, more: at + page.length < listed.threads.length, engines: listed.engines };
    },
    /** 훑기의 폴더 줄 하나에 딸린 규칙 파일과 내 MCP 상태(읽기만, 저장하지 않는다) */
    checkup: checkup.checkup,
    /** 점검이 돌려준 규칙 파일 하나의 본문 조각 */
    checkupRule: checkup.rule,
    /** 훑기의 폴더 줄 하나에서 가져올 수 있는 MCP 서버 후보(읽기만, 명령을 실행하지 않는다) */
    mcpScan: mcpWork.scan,
    /** 고른 후보를 Lair 목록에 넣고 원장에 남긴다 */
    mcpImport: mcpWork.importInto,
    /** 훑기의 폴더 줄 하나에서 규칙 옮기기 후보(다른 도구 규칙·이 폴더 엔진이 못 읽는 규칙·내 규칙)를 찾는다(읽기만) */
    rulesScan: rulesWork.scan,
    /** 후보 하나가 쓸 파일의 전·후 글(사람이 차이를 보는 용) */
    rulesPreview: rulesWork.preview,
    /** 고른 후보를 쓴다(work-rules.ts apply) */
    rulesApply(request: MigrateRulesApplyRequest): Promise<MigrateRulesApplyResult> { return rulesWork.apply(request); },
    /** 훑기의 폴더 줄 하나에서 다른 도구의 기억 후보를 찾는다(읽기만, 큰 결과는 offset 으로 쪽을 넘긴다) */
    memoryScan(scanId: string, rowId: string, offset?: number): Promise<MigrateMemoryScanView> { return memoryWork.scan(scanId, rowId, offset); },
    /** 붙여넣은 기억 글을 항목으로 나눠 후보로 돌려준다(읽기만, 모델을 부르지 않는다) */
    memoryPaste(app: MemoryPasteApp, text: string, offset?: number): MigrateMemoryPasteView { return memoryWork.paste(app, text, offset); },
    /** 고른 기억 후보를 공통 기억에 더하고 원장에 남긴다(work-memory.ts apply) */
    memoryApply(request: MigrateMemoryApplyRequest): Promise<MigrateMemoryApplyResult> { return memoryWork.apply(request); },
    /** 보관함(archive.ts): 넣은 파일·보관 대화 읽기, 파일 넣기·이어서 넣기·지우기·멈춤 */
    archive,
    /** 기동 때: 도는 중이던 실행의 planned 항목을 프로젝트·결속으로 맞추고 실행을 failed 로 닫는다 */
    recover(): void {
      try {
        const runs = led(ledger => ledger.runningRuns(), [] as string[]);
        if (!runs.length) return;
        const boundIndex = boundSessionIndex(deps.sessionList(), deps.isTemporary);
        for (const runId of runs) {
          const startedAt = led(ledger => ledger.readRun(runId), undefined)?.startedAt ?? '';
          for (const item of led(ledger => ledger.plannedItems(runId), [] as MigrateItemRow[])) {
            const done = (result: MigrateItemCompletion): void => led(ledger => ledger.completeItem(runId, item.seq, result), undefined);
            if (item.kind === 'project') {
              const projectId = deps.projectIdOfPath(item.sourceRef);
              if (projectId) {
                // 시작 전에 있던 프로젝트(기록이 없어도 만든 시각이 실행 시작보다 앞)는 이 실행이 만든 것이 아니다
                const createdAt = deps.undo.project(projectId)?.createdAt;
                if (item.detail.existed === true || (createdAt !== undefined && createdAt < startedAt)) done({ outcome: 'already', targetRef: projectId });
                else done({ outcome: 'imported', targetRef: projectId, undo: { created: true } });
              } else done({ outcome: 'failed', code: 'interrupted' });
              continue;
            }
            // MCP 항목은 저장과 같은 턴에 끝난 모습으로 적히니 planned 로 남았다면 중단된 것이다(대화로 해석하지 않는다)
            if (item.kind === 'mcp') { done({ outcome: 'failed', code: 'interrupted' }); continue; }
            // 기억 항목도 add 와 같은 턴에 끝난 모습으로 적히니 planned 로 남았다면 중단된 것이다
            if (item.kind === 'memory') { done({ outcome: 'failed', code: 'interrupted' }); continue; }
            // 규칙 항목은 쓰기 앞에 되돌림 칸을 적었으므로 중단으로만 닫고(completeItem 이 undo 를 생략하면 칸이 그대로 남는다) 쓰다 만 파일 판정은 되돌림이 한다
            if (item.kind === 'rule-file' || item.kind === 'common-rule') { done({ outcome: 'failed', code: 'interrupted' }); continue; }
            const engine = typeof item.detail.engine === 'string' ? item.detail.engine : item.sourceRef.slice(0, item.sourceRef.indexOf(':'));
            const thread = item.sourceRef.slice(item.sourceRef.indexOf('\n') + 1);
            const bound = thread ? boundIndex.get(`${engine}:${thread}`) : undefined;
            if (bound) {
              // 결속만으로는 이 실행이 만든 대화인지 모른다: 가져온 시각이 실행 시작 이후일 때만 만든 것으로 적는다
              const boundSession = deps.session(bound);
              const importedAt = boundSession?.externalThread?.importedAt;
              if (boundSession && importedAt !== undefined && importedAt >= startedAt) done({ outcome: 'imported', targetRef: bound, undo: { created: true, baseline: threadBaseline(boundSession) } });
              else done({ outcome: 'already', targetRef: bound });
            } else done({ outcome: 'failed', code: 'interrupted' });
          }
          led(ledger => ledger.finishRun(runId, 'failed', new Date().toISOString()), undefined);
        }
      } catch { /* 복구 실패가 앱 시작을 막지 않는다. 다음 기동에서 다시 본다 */ }
    },
    /** 닫기: 대기 중인 읽기를 끊는다. 도는 실행은 다음 항목 앞에서 멈추고 다음 기동의 복구가 맞춘다. 이 뒤 원장 오류는 삼킨다 */
    dispose(): void {
      disposed = true;
      unhookFacts();
      archive.dispose();
      undo.dispose();
      for (const controller of controllers) controller.abort();
      controllers.clear();
    },
  };
}

export type WorkRuns = ReturnType<typeof createWorkRuns>;
