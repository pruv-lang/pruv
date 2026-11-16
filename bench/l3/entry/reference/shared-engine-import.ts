// 바깥 터미널 엔진 대화 이어 열기의 공유 타입(타입·데이터만, docs/notes/engine-import-2026-10-19.md). contracts.ts 가 Turn·Session 칸에 쓴다.
export type EngineImportEngine = 'claude' | 'codex' | 'grok' | 'opencode' | 'qwen';

/** 이어 열기·옮겨 오기 줄 엔진 목록(데이터만, 화면 순서). 판단 함수는 두지 않는다 */
export const ENGINE_IMPORT_ENGINES: readonly EngineImportEngine[] = ['claude', 'codex', 'grok', 'opencode', 'qwen'];

/** 마지막으로 기록된 계획의 상한: 항목 수와 항목당 글자 수 */
export const LAST_PLAN_LIMITS = { items: 30, text: 200 } as const;

/** 마지막으로 기록된 계획의 항목 하나. 상태를 모르면 칸이 없다. */
export interface LastPlanItem { text: string; status?: 'pending' | 'in_progress' | 'completed' }

/** 바깥 기록에서 읽은 마지막 계획(docs/notes/tool-migration-2026-11-03.md M4 설계). at 은 마지막으로 반영한 기록 시각, more 는 상한 때문에 뺀 항목 수 */
export interface LastPlan { at?: string; items: LastPlanItem[]; more?: number }

/** 대화의 바깥 출처. thread 는 바깥 원래 번호, forks 는 확정한 갈래 번호(최대 20). */
export interface ExternalThread {
  engine: EngineImportEngine;
  source: string;
  thread: string;
  mode: 'fork' | 'same';
  importedAt: string;
  forks?: string[];
  /** 바깥 기록의 앞부분을 상한 때문에 가져오지 못했다 */
  omittedBefore?: true;
  /** 가져올 때 바깥 기록에서 찾은 마지막으로 기록된 계획. 그 뒤 끝났는지는 모른다. 못 찾았으면 칸이 없다. */
  lastPlan?: LastPlan;
}

/** 터미널에서 가져온 읽기 전용 요청 표지 */
export interface ImportedMark {
  engine: EngineImportEngine;
  /** 이 요청의 도구 사용 수(0 이면 칸 없음) */
  tools?: number;
  /** 사람 글이나 답이 상한에서 잘렸다 */
  clipped?: true;
}

/** engine-import-sources 입력 */
export interface EngineImportSourcesRequest { kind: 'engine-import-sources' }

/** engine-import-list 요청 */
export interface EngineImportListRequest extends EngineImportListInput { kind: 'engine-import-list' }

/** engine-import-open 요청 */
export interface EngineImportOpenRequest extends EngineImportOpenInput { kind: 'engine-import-open' }

/** 바깥 터미널 대화 이어 열기 요청 셋 */
export type EngineImportRequest = EngineImportSourcesRequest | EngineImportListRequest | EngineImportOpenRequest;

/** engine-import-sources 결과 */
export interface EngineImportSourcesResult {
  sources: Array<{ source: string; engine: EngineImportEngine; account?: string; label: string }>;
}

/** engine-import-list 입력 */
export interface EngineImportListInput {
  source: string;
  folder: string;
}

/** engine-import-list 결과 */
export interface EngineImportListResult {
  items: Array<{
    thread: string;
    title: string;
    updatedAt?: string;
    size?: number;
    messages?: number;
    path?: string;
    boundSessionId?: string;
    maybeOpen?: true;
  }>;
}

/** engine-import-open 입력 */
export interface EngineImportOpenInput {
  source: string;
  folder: string;
  thread: string;
  mode: 'fork' | 'same';
  targetAccount?: string;
}

/** engine-import-open 결과 */
export interface EngineImportOpenResult {
  sessionId: string;
  existing?: true;
}
