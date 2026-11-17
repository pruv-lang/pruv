// 요청 전체 예산과 차단의 공유 타입과 기본값 표(docs/notes/request-budget-2026-10-24.md). 타입·데이터만 둔다. 판단과 문구는 src/backend/request-budget.ts 가 한다.
import { BUDGET_SIZES } from './schedule-lists';
import type { BUDGET_COUNTERS, BUDGET_PART_KINDS, BUDGET_STATES, WORK_CARD_STATES, WORK_CARD_VERIFY_STATUSES } from './schedule-lists';
// 크기 목록·상태·세는 항목은 폰 계약이 정본이고 생성물(schedule-lists.ts)에서 갈라 나온다(구조 규칙 2).
export { BUDGET_SIZES };

/** 세는 항목. time 은 실행 시간(분 상한), 나머지는 횟수. */
export type BudgetCounter = (typeof BUDGET_COUNTERS)[number];

/** 예산 막대 조각의 종류(폰 BudgetView.parts[].kind 와 같은 목록, 정본은 폰 계약 BUDGET_PART_KINDS). */
export type BudgetPartKind = (typeof BUDGET_PART_KINDS)[number];

/** 항목별 상한. */
export interface BudgetLimits {
  timeMinutes: number;
  delegations: number;
  commands: number;
  reviews: number;
  checks: number;
}

/** 횟수로 세고 상한이 있는 항목(차감과 저장 읽기 검사가 같이 쓰는 목록). */
export const BUDGET_COUNT_KEYS = ['delegations', 'commands', 'reviews', 'checks'] as const;

/** 사용자 결정 2026-10-24 기본 예산(요청 하나 전체). 증액은 다 찬 항목에 이 값을 한 번 더한다. */
export const DEFAULT_BUDGET: BudgetLimits = { timeMinutes: 120, delegations: 24, commands: 3, reviews: 2, checks: 1 };

/** 설정의 1묶음 단위. defaultUnlimited 는 설정 기본이 제한 없음(설정 칸 없음)이라는 뜻이고, 숫자는 칩·늘리기 단위로만 쓴다. */
export interface BudgetUnit extends BudgetLimits {
  /** 설정 기본이 제한 없음(설정 칸 없음). 숫자는 칩·늘리기 단위로만 쓴다. */
  defaultUnlimited?: true;
}

/** 설정 화면의 추천값(docs/notes/request-budget-default-unlimited-2026-10-25.md 6절). id 와 값만 둔다. */
export type BudgetPresetId = 'light' | 'standard' | 'long';
export const BUDGET_PRESETS: ReadonlyArray<{ id: BudgetPresetId; limits: BudgetLimits }> = [
  { id: 'light', limits: { timeMinutes: 60, delegations: 12, commands: 2, reviews: 2, checks: 1 } },
  { id: 'standard', limits: DEFAULT_BUDGET },
  { id: 'long', limits: { timeMinutes: 480, delegations: 96, commands: 12, reviews: 10, checks: 6 } },
];

/** 크기 보기: 1·2·4·8묶음과 제한 없음(docs/notes/request-budget-knobs-2026-10-24.md 1절). n묶음 상한은 설정의 1묶음 × n. */
export type BudgetSize = (typeof BUDGET_SIZES)[number];

/** 설정의 1묶음(기본 예산) 항목별 허용 범위. 시간은 분. */
export const BUDGET_UNIT_RANGE: Readonly<Record<keyof BudgetLimits, { min: number; max: number }>> = {
  timeMinutes: { min: 10, max: 1440 },
  delegations: { min: 1, max: 999 },
  commands: { min: 1, max: 99 },
  reviews: { min: 1, max: 99 },
  checks: { min: 1, max: 99 },
};

/** 쓴 양. activeMs 는 실행 시간, tools·failures·retries 는 상한 없이 표시만 한다. */
export interface BudgetUsage {
  activeMs: number;
  delegations: number;
  commands: number;
  reviews: number;
  checks: number;
  tools: number;
  failures: number;
  retries: number;
}

/** open 진행 중, halted 상한에 닿아 사람의 결정을 기다림, closed 끝남. */
export type BudgetState = (typeof BUDGET_STATES)[number];
/** 닫힌 까닭: 조용해짐, 사용자가 끝내기를 고름, 멈춘 뒤 사용자가 새 글을 보냄. */
export type BudgetEnd = 'quiet' | 'user-end' | 'user-message';
/** 예산을 연 쪽: 사용자 글, 예약 실행, 사용자가 직접 맡긴 위임, 보류를 푼 결과 전달. */
export type BudgetOrigin = 'user' | 'schedule' | 'direct' | 'release';

export interface WorkCardVerify {
  label: string;
  status: (typeof WORK_CARD_VERIFY_STATUSES)[number];
}

/** 작업 카드: 산출물과 완료 조건은 필수, 나머지는 선택(R10). */
export interface WorkCard {
  deliverable: string;
  done: string[];
  targets?: { confirmed: number; candidate: number; unknown: number; note?: string };
  verify?: WorkCardVerify[];
  conflicts?: string[];
  /** 카드 상태(Codex 요청의 work_card 만 채운다): 일이 남음·끝남·사용자 답 필요·외부 조건으로 막힘. */
  state?: (typeof WORK_CARD_STATES)[number];
  /** 카드를 마지막으로 쓴 대화·요청. 예산은 대화 사이에서 공유될 수 있어 자동 이어가기가 자기 카드인지 가리는 데 쓴다. */
  by?: { sessionId: string; turnId: string };
  updatedAt: string;
}

/** 사용자 요청 하나(같은 사람 글에서 시작한 사슬)당 자동 이어가기 상한. */
export const AUTO_CONTINUE_MAX = 2;

export interface RequestBudget {
  id: string;
  openedAt: string;
  origin: BudgetOrigin;
  state: BudgetState;
  limits: BudgetLimits;
  /** 제한 없음: 세기·표시는 그대로 하고 횟수 멈춤 판정과 시간 타이머만 건너뛴다. limits 숫자는 남겨 둔다(표시용). */
  unlimited?: true;
  used: BudgetUsage;
  /** 지금 실행 시간을 세는 중이면 그 구간의 시작 시각(ISO). 쓴 시간에는 아직 안 더해졌고 정산하면 지운다. 화면이 진행 중 분을 계산한다. */
  activeSince?: string;
  halt?: { reason: BudgetCounter; at: string; stoppedDelegationIds: string[] };
  ended?: { how: BudgetEnd; at: string };
  card?: WorkCard;
  /** 자동 이어가기 기록: 사슬을 연 사람 요청(userTurnId), 이어 간 횟수, 마지막으로 이어 간 끝난 요청, 사용자가 중지했으면 stopped. 사람이 쓴 새 요청이 지운다. */
  autoContinue?: { sessionId: string; userTurnId: string; count: number; lastFrom?: string; stopped?: true };
  /** 이 예산이 사용자 새 글로 교체한 멈춘 예산의 번호. 교체 뒤에는 옛 예산이 사라져 '사용자가 이어 간 요청'과 '끝낸 요청'을 구별할 수 없어 남긴다. */
  follows?: string;
}

/** 턴·위임·대기 항목이 가리키는 예산(다른 대화에 붙은 턴도 보낸 쪽 예산을 가리킨다). */
export interface BudgetRef {
  sessionId: string;
  budgetId: string;
}

/** 멈춘 예산에 사람이 늘리기·끝내기를 고르는 요청. */
export interface BudgetDecideRequest {
  kind: 'budget-decide';
  sessionId: string;
  budgetId: string;
  choice: 'extend' | 'end';
  /** 늘리기 폭(묶음 수 또는 제한 없음). 없으면 1묶음(옛 화면 호환). */
  size?: BudgetSize;
}

/** 설정의 기본 예산(1묶음) 바꾸기. 사람 전용이고 앱 동작 통로는 닫는다. limits null 은 제한 없음(설정 칸을 지운다). */
export interface SetRequestBudgetRequest {
  kind: 'set-request-budget';
  limits: BudgetLimits | null;
}
