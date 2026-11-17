// 요청 전체 예산 상태기계(docs/notes/request-budget-2026-10-24.md 설계 + 논쟁 D1~D9, R10~R13).
// 서비스 객체를 받지 않고 좁은 deps 만 받는다. 중지·보류 같은 실제 동작은 연결 쪽이 deps 로 넣는다.
import { randomUUID } from 'node:crypto';
import { BUDGET_COUNT_KEYS, DEFAULT_BUDGET } from '../shared/request-budget';
import type { BudgetCounter, BudgetLimits, BudgetOrigin, BudgetRef, BudgetSize, BudgetUnit, RequestBudget, WorkCard } from '../shared/request-budget';
import { engineText, type EngineTextEntry, type InstructionLanguage } from '../shared/engine-text';
import type { WireText } from '../shared/i18n/wire';
import { WORK_CARD_STATES, WORK_CARD_VERIFY_STATUSES } from '../shared/schedule-lists';

/** 예산 칸만 아는 최소 대화 모양(contracts 의 Session 이 budget 칸을 더하면 그대로 들어맞는다). */
export type BudgetSession = { id: string; budget?: RequestBudget };

export interface RequestBudgetDeps {
  /** 지금 시각(ms) */
  now(): number;
  /** 설정의 1묶음(기본 예산). 새 예산을 여는 상한이고 크기·늘리기의 산술 단위다 */
  unit(): BudgetUnit;
  findSession(id: string): BudgetSession | undefined;
  /** 대화 저장 */
  save(session: BudgetSession): void;
  /** 화면 스냅숏 변화 알림 */
  changed(): void;
  /** 메인 턴 중지(stopReason 'budget' 은 연결 쪽이 처리) */
  stopMain(sessionId: string): void;
  stopDelegation(sessionId: string, delegationId: string): void;
  /** 이 예산에 붙어 지금 도는 위임 번호 */
  runningDelegationIds(ref: BudgetRef): string[];
  /** 다른 대화에 붙은 이 예산의 턴 중지 */
  stopAttachedTurns(ref: BudgetRef): void;
  /** 대기 항목을 모두 held 로 둔다 */
  holdQueued(session: BudgetSession, budgetId: string): void;
  /** 이 예산의 held 항목을 푼다. 풀 것이 있었으면 true */
  releaseHeld(session: BudgetSession, budgetId: string): boolean;
  /** 주인이 아닌 대화의 대기 항목 중 옛 예산을 가리키는 것(held·pending)을 새 참조로 바꾸고 held 는 pending 으로 풀어 깨운다 */
  carryHeld(owner: BudgetSession, oldBudgetId: string, newRef: BudgetRef): void;
  /** 늘린 항목(reason)과 쓴양/상한을 담은 이어 가기 글을 대기에 넣고, wake 가 참이면 깨운다. reason 이 null 이면 글은 넣지 않고 깨우기만 한다 */
  queueContinue(session: BudgetSession, budgetId: string, reason: BudgetCounter | null, wake: boolean): void;
  /** 이 예산의 held 항목을 지운다 */
  dropHeld(session: BudgetSession, budgetId: string): void;
  /** 예약된 복구(session.recovery) 취소 */
  cancelRecovery(session: BudgetSession): void;
  /** 서비스 쪽 남은 일 없음: 실행 중 턴·준비·pending 대기·복구 예약·새로 시작 대기가 모두 없다 */
  isQuiet(session: BudgetSession): boolean;
  /** 확인할 일에 "예산 한도 도달"을 올린다 */
  announce(session: BudgetSession): void;
}

export type ChargeWhat = { delegations?: number; commands?: number; reviews?: number; checks?: number; retries?: number };
export type ChargeResult = { ok: true } | { ok: false; reason: BudgetCounter };
export type DecideResult = { ok: true } | { ok: false; code: 'not-found' | 'not-halted' };
export type SetCardResult = { ok: true; budget: RequestBudget } | { ok: false; error: string };

/** 예산마다 메모리에만 두는 실행 목록과 시간 소비 상태(저장하지 않는다, D8). */
interface BudgetClock {
  sessionId: string;
  runs: Map<string, { waiting: boolean }>;
  /** 소비가 시작된 시각. 소비 중이 아니면 없다 */
  since?: number;
  timer?: ReturnType<typeof setTimeout>;
}

const CARD_TEXT_MAX = 300;
/** 상한을 가진 다섯 항목(크기 적용·늘리기가 같이 쓴다) */
const LIMIT_KEYS: readonly (keyof BudgetLimits)[] = ['timeMinutes', ...BUDGET_COUNT_KEYS];

// 모델에게 돌려주는 짧은 글. 언어 처리는 delegation-texts.ts 와 같다(engineText 로 지시문 언어 값을 꺼내고 값이 없으면 ko).
// 사용자에게 가는 한도 거절(직접 위임·다시 맡기기)은 아래 refusalWire 의 wire 키로 간다.
const COUNTER_NAME: Record<BudgetCounter, EngineTextEntry<string>> = {
  time: { ko: /* i18n-keep: 모델에게만 가는 거절 글의 항목 이름 */ '시간', en: 'time' },
  delegations: { ko: /* i18n-keep: 모델에게만 가는 거절 글의 항목 이름 */ '위임', en: 'delegations' },
  commands: { ko: /* i18n-keep: 모델에게만 가는 거절 글의 항목 이름 */ '빌드·명령', en: 'commands' },
  reviews: { ko: /* i18n-keep: 모델에게만 가는 거절 글의 항목 이름 */ '감사', en: 'reviews' },
  checks: { ko: /* i18n-keep: 모델에게만 가는 거절 글의 항목 이름 */ '실제 확인', en: 'checks' },
};
const BUDGET_TEXT = {
  refusal: {
    ko: (counter: string, amount: string) => /* i18n-keep: 모델에게만 가는 한도 거절 글 */ `예산 한도(${counter} ${amount})에 닿아 사용자가 늘리거나 끝내기를 고를 때까지 멈춘다. 새 위임·명령을 시도하지 않고 지금까지 결과와 확인하지 못한 것만 짧게 보고한다.`,
    en: (counter: string, amount: string) => `The request budget limit (${counter} ${amount}) was reached, so work is paused until the user extends it or ends it. Do not try new delegations or commands; briefly report the results so far and what you could not verify.`,
  } as EngineTextEntry<(counter: string, amount: string) => string>,
  continueText: {
    ko: (counter: string, amount: string) => /* i18n-keep: 모델에게만 가는 이어 가기 글 */ `사용자가 예산 한도(${counter} ${amount})를 늘렸다. 앞서 끊긴 도구 호출은 사용자 중단이 아니라 이 예산 한도에 닿아 앱이 멈춘 것이다. 멈춘 지점에서 이어 간다.`,
    en: (counter: string, amount: string) => `The user raised the request budget limit (${counter} ${amount}). The tool call that was cut off earlier was stopped by the app at this budget limit, not by the user. Continue from where you stopped.`,
  } as EngineTextEntry<(counter: string, amount: string) => string>,
  cardMissing: {
    ko: /* i18n-keep: 모델에게만 가는 작업 카드 요구 글 */ '위임 전에 work_card 도구로 산출물과 완료 조건을 먼저 적는다.',
    en: 'Before delegating, write the deliverable and the done conditions with the work_card tool.',
  } as EngineTextEntry<string>,
  card: {
    ko: {
      missing: /* i18n-keep: 모델에게만 가는 work_card 도구 오류 */ '지금 예산을 찾을 수 없어 작업 카드를 저장하지 못했다.',
      deliverable: /* i18n-keep: 모델에게만 가는 work_card 도구 오류 */ 'deliverable 은 1~300자여야 한다.',
      done: /* i18n-keep: 모델에게만 가는 work_card 도구 오류 */ 'done 은 각 300자 이하 1~5줄이어야 한다.',
      targets: /* i18n-keep: 모델에게만 가는 work_card 도구 오류 */ 'targets 의 수는 0 이상 정수여야 하고 note 는 300자 이하여야 한다.',
      verify: /* i18n-keep: 모델에게만 가는 work_card 도구 오류 */ 'verify 는 label 1~300자와 status(planned·done·skipped)를 가진 0~5개 항목이어야 한다.',
      conflicts: /* i18n-keep: 모델에게만 가는 work_card 도구 오류 */ 'conflicts 는 각 300자 이하 0~5줄이어야 한다.',
      state: /* i18n-keep: 모델에게만 가는 work_card 도구 오류 */ 'state 는 working·done·waiting-user·blocked 중 하나여야 한다.',
    },
    en: {
      missing: 'The current budget was not found, so the work card was not saved.',
      deliverable: 'deliverable must be 1–300 characters.',
      done: 'done must be 1–5 lines of at most 300 characters each.',
      targets: 'targets counts must be non-negative integers and note at most 300 characters.',
      verify: 'verify must be 0–5 items with a label of 1–300 characters and a status (planned, done or skipped).',
      conflicts: 'conflicts must be 0–5 lines of at most 300 characters each.',
      state: 'state must be one of working, done, waiting-user or blocked.',
    },
  } as EngineTextEntry<Record<'missing' | 'deliverable' | 'done' | 'targets' | 'verify' | 'conflicts' | 'state', string>>,
};

const NO_LIMIT_TEXT: EngineTextEntry<string> = { ko: /* i18n-keep: 모델에게만 가는 사용량 글의 상한 자리 */ '제한 없음', en: 'no limit' };

/** 모델에게 가는 상한 글자. 제한 없음 예산이면 숫자 대신 '제한 없음'('no limit'). */
export function limitText(budget: RequestBudget, limit: number, language: InstructionLanguage = 'ko'): string {
  return budget.unlimited ? engineText(NO_LIMIT_TEXT, language) : String(limit);
}

/** 쓴 양/상한 글자. 시간은 분 단위. */
function amountText(counter: BudgetCounter, budget: RequestBudget, language: InstructionLanguage): string {
  if (counter === 'time') return `${Math.floor(budget.used.activeMs / 60000)}/${limitText(budget, budget.limits.timeMinutes, language)}`;
  return `${budget.used[counter]}/${limitText(budget, budget.limits[counter], language)}`;
}

/** 사용자에게 가는 한도 거절(직접 위임·다시 맡기기): wire 키 budget.refusal.<항목> 과 인자 {used, limit}(시간은 분). */
export function refusalWire(reason: BudgetCounter, budget: RequestBudget): WireText {
  const used = reason === 'time' ? Math.floor(budget.used.activeMs / 60000) : budget.used[reason];
  const limit = reason === 'time' ? budget.limits.timeMinutes : budget.limits[reason];
  return { key: `budget.refusal.${reason}`, args: { used, limit } };
}

/** 거절 글: 어떤 한도에 닿았고 무엇을 하지 말아야 하는지. */
export function refusal(reason: BudgetCounter, budget: RequestBudget, language: InstructionLanguage = 'ko'): string {
  return engineText(BUDGET_TEXT.refusal, language)(engineText(COUNTER_NAME[reason], language), amountText(reason, budget, language));
}

/** 이어 가기 글: 사용자가 늘린 항목과 늘린 뒤 쓴양/상한, 끊긴 도구 호출이 예산 한도 때문이라는 것. */
export function continueText(reason: BudgetCounter, budget: RequestBudget, language: InstructionLanguage = 'ko'): string {
  return engineText(BUDGET_TEXT.continueText, language)(engineText(COUNTER_NAME[reason], language), amountText(reason, budget, language));
}

/** 카드 요구 거절 글: 첫 위임 전에 작업 카드가 없을 때. */
export function cardMissing(language: InstructionLanguage = 'ko'): string {
  return engineText(BUDGET_TEXT.cardMissing, language);
}

function validText(value: unknown): value is string {
  return typeof value === 'string' && value.trim().length >= 1 && value.trim().length <= CARD_TEXT_MAX;
}

function validLines(value: unknown, min: number, max: number): value is string[] {
  return Array.isArray(value) && value.length >= min && value.length <= max && value.every(validText);
}

const VERIFY_STATUS: ReadonlySet<string> = new Set<string>(WORK_CARD_VERIFY_STATUSES);
const CARD_STATE: ReadonlySet<string> = new Set<string>(WORK_CARD_STATES);

/** 설정 칸 → 1묶음 단위. 칸이 없으면 기본이 제한 없음(숫자는 DEFAULT_BUDGET)이라 표시한다. */
export function budgetUnitOf(setting: BudgetLimits | undefined): BudgetUnit {
  if (!setting) return { ...DEFAULT_BUDGET, defaultUnlimited: true };
  return { timeMinutes: setting.timeMinutes, delegations: setting.delegations, commands: setting.commands, reviews: setting.reviews, checks: setting.checks };
}

export class RequestBudgets {
  private readonly clocks = new Map<string, BudgetClock>();
  /** 턴마다 이미 센 도구 활동 번호(턴이 끝나면 forgetTurn 으로 해제) */
  private readonly toolSeen = new Map<string, Set<string>>();

  constructor(private readonly deps: RequestBudgetDeps) {}

  /** ref 가 가리키는 지금 예산. 대화에 없거나 번호가 다르면(교체된 옛 예산) undefined. */
  current(ref: BudgetRef): RequestBudget | undefined {
    const budget = this.deps.findSession(ref.sessionId)?.budget;
    return budget && budget.id === ref.budgetId ? budget : undefined;
  }

  /** 멈춤 판정 하나(D4). 시작 문들이 이것을 본다. */
  isHalted(ref: BudgetRef): boolean {
    return this.current(ref)?.state === 'halted';
  }

  /** 열린 예산이 있으면 그것, 멈춘 예산이면 그대로(시작 문이 isHalted 로 막는다), 없거나 닫혔으면 새로 연다. */
  attach(session: BudgetSession, origin: BudgetOrigin, size?: BudgetSize): BudgetRef {
    const budget = session.budget;
    if (budget && budget.state !== 'closed') return { sessionId: session.id, budgetId: budget.id };
    return this.open(session, origin, size);
  }

  /**
   * 사용자의 새 글(D6): 멈춘 예산은 닫고 새로 연다. 열린 예산이면 그대로. 사람이 만든 예약의 회차도 origin 'schedule' 로 같은 규칙으로 닫고 새로 열지만,
   * 이어 받는 표시(follows)와 다른 대화의 보류 항목 옮기기(carryHeld)는 사용자 새 글일 때만 한다(예약 회차는 사용자가 이어 간 요청이 아니다).
   */
  replaceForUserMessage(session: BudgetSession, size?: BudgetSize, origin: BudgetOrigin = 'user'): BudgetRef {
    const budget = session.budget;
    if (budget?.state === 'halted') {
      this.close(budget, session.id, 'user-message');
      const ref = this.open(session, origin, size);
      if (origin === 'schedule') return ref;
      if (session.budget) session.budget.follows = budget.id;
      // 다른 대화가 옛 예산에 붙여 둔 대기는 사용자가 이어 간 요청이라 새 예산으로 옮겨 푼다.
      this.deps.carryHeld(session, budget.id, ref);
      return ref;
    }
    return this.attach(session, origin, size);
  }

  /** 새 예산. 크기가 없으면 설정 기본(칸 없음이면 제한 없음, 있으면 1묶음). 제한 없음이어도 limits 에는 단위 숫자를 남긴다(표시용). */
  private open(session: BudgetSession, origin: BudgetOrigin, size?: BudgetSize): BudgetRef {
    const stale = session.budget;
    if (stale) this.dropClock(stale.id);
    const unit = this.deps.unit();
    const chosen = size ?? (unit.defaultUnlimited ? 'unlimited' : 1);
    const times = chosen === 'unlimited' ? 1 : chosen;
    const budget: RequestBudget = {
      id: randomUUID(),
      openedAt: new Date(this.deps.now()).toISOString(),
      origin,
      state: 'open',
      limits: { timeMinutes: unit.timeMinutes * times, delegations: unit.delegations * times, commands: unit.commands * times, reviews: unit.reviews * times, checks: unit.checks * times },
      ...(chosen === 'unlimited' ? { unlimited: true as const } : {}),
      used: { activeMs: 0, delegations: 0, commands: 0, reviews: 0, checks: 0, tools: 0, failures: 0, retries: 0 },
    };
    session.budget = budget;
    // 저장은 하지 않는다: 연 쪽(턴 접수·위임 시작)의 기존 저장이 이 변경을 싣도록 부르는 쪽이 저장 앞에서 연다.
    this.deps.changed();
    return { sessionId: session.id, budgetId: budget.id };
  }

  /** 조용히 닫힌 같은 예산을 다시 연다(예약이 가리키는 예산, D3). 사용자가 끝냈거나 교체돼 찾을 수 없으면 열지 않고 false. 저장은 부르는 쪽이 한다. */
  reopen(ref: BudgetRef): boolean {
    const budget = this.current(ref);
    if (!budget || budget.state !== 'closed' || budget.ended?.how !== 'quiet') return false;
    budget.state = 'open';
    delete budget.ended;
    this.deps.changed();
    return true;
  }

  private close(budget: RequestBudget, sessionId: string, how: 'quiet' | 'user-end' | 'user-message'): void {
    this.settle(budget, sessionId);
    budget.state = 'closed';
    budget.ended = { how, at: new Date(this.deps.now()).toISOString() };
    this.dropClock(budget.id);
  }

  /**
   * 이번 차감으로 상한을 넘으면 아무것도 차감하지 않고 멈춤으로 가며 false(D1). 상한까지는 허용한다.
   * 없거나 닫힌 예산은 막지 않는다. 제한 없음 예산은 차감만 하고 상한 초과로 멈추지 않는다. 저장은 부르는 쪽의 기존 대화 저장에 얹는다.
   */
  charge(ref: BudgetRef, what: ChargeWhat): ChargeResult {
    const budget = this.current(ref);
    if (!budget || budget.state === 'closed') return { ok: true };
    if (budget.state === 'halted') return { ok: false, reason: budget.halt?.reason ?? 'delegations' };
    for (const key of BUDGET_COUNT_KEYS) {
      const amount = what[key] ?? 0;
      if (!budget.unlimited && amount > 0 && budget.used[key] + amount > budget.limits[key]) {
        this.halt(ref, key);
        return { ok: false, reason: key };
      }
    }
    for (const key of BUDGET_COUNT_KEYS) budget.used[key] += what[key] ?? 0;
    budget.used.retries += what.retries ?? 0;
    return { ok: true };
  }

  noteFailure(ref: BudgetRef): void {
    const budget = this.current(ref);
    if (budget) budget.used.failures += 1;
  }

  /** 도구 활동 수. 턴마다 활동 번호를 한 번만 센다(Set 조회 한 번과 더하기뿐). */
  noteTool(ref: BudgetRef, turnId: string, activityId: string): void {
    let seen = this.toolSeen.get(turnId);
    if (!seen) this.toolSeen.set(turnId, seen = new Set());
    if (seen.has(activityId)) return;
    seen.add(activityId);
    const budget = this.current(ref);
    if (budget) budget.used.tools += 1;
  }

  /** 턴이 끝났다: 센 활동 번호 집합을 해제한다. */
  forgetTurn(turnId: string): void {
    this.toolSeen.delete(turnId);
  }

  // 시간(D7): 붙은 실행이 하나 이상 있고 모두가 사용자 답만 기다리는 것은 아닌 구간만 센다.

  runStarted(ref: BudgetRef, runKey: string): void {
    if (!this.current(ref)) return;
    this.clockOf(ref).runs.set(runKey, { waiting: false });
    this.refresh(ref);
  }

  runWaiting(ref: BudgetRef, runKey: string, waiting: boolean): void {
    const run = this.clocks.get(ref.budgetId)?.runs.get(runKey);
    if (!run || run.waiting === waiting) return;
    run.waiting = waiting;
    this.refresh(ref);
  }

  runEnded(ref: BudgetRef, runKey: string): void {
    const clock = this.clocks.get(ref.budgetId);
    if (!clock || !clock.runs.delete(runKey)) return;
    this.refresh(ref);
  }

  private clockOf(ref: BudgetRef): BudgetClock {
    let clock = this.clocks.get(ref.budgetId);
    if (!clock) this.clocks.set(ref.budgetId, clock = { sessionId: ref.sessionId, runs: new Map() });
    return clock;
  }

  /** 소비 구간이 바뀔 때만 계산한다: 끝나는 구간은 더하고, 시작하는 구간은 남은 시간 뒤 타이머를 건다. */
  private refresh(ref: BudgetRef): void {
    const budget = this.current(ref);
    const clock = this.clocks.get(ref.budgetId);
    if (!budget || !clock) return;
    const consuming = budget.state === 'open' && clock.runs.size > 0 && [...clock.runs.values()].some(run => !run.waiting);
    if (consuming === (clock.since !== undefined)) return;
    // 정산은 저장하지 않는다. 실행이 끝나는 쪽(턴 끝·위임 끝)이 이 변경을 자기 기존 저장보다 앞에서 일으킨다.
    if (!consuming) {
      this.settle(budget, ref.sessionId);
      return;
    }
    this.begin(budget, clock);
    this.arm(ref, budget, clock);
  }

  /** 소비 구간을 연다. 시작 시각을 예산에도 적어 화면이 진행 중 분을 계산하게 한다(다음 기존 저장에 실린다). */
  private begin(budget: RequestBudget, clock: BudgetClock): void {
    clock.since = this.deps.now();
    budget.activeSince = new Date(clock.since).toISOString();
  }

  private arm(ref: BudgetRef, budget: RequestBudget, clock: BudgetClock): void {
    if (budget.unlimited) return;
    const left = budget.limits.timeMinutes * 60000 - budget.used.activeMs;
    if (left <= 0) {
      this.fire(ref);
      return;
    }
    clock.timer = setTimeout(() => this.fire(ref), left);
    clock.timer.unref?.();
  }

  private fire(ref: BudgetRef): void {
    const budget = this.current(ref);
    const clock = this.clocks.get(ref.budgetId);
    if (clock) clock.timer = undefined;
    // 제한 없음으로 바뀐 뒤의 늦은 발화는 무시한다(K4).
    if (!budget || budget.state !== 'open' || budget.unlimited) return;
    this.settle(budget, ref.sessionId);
    // 타이머가 조금 일찍 터졌으면 남은 만큼 다시 건다.
    if (clock && budget.used.activeMs < budget.limits.timeMinutes * 60000 && clock.runs.size > 0) {
      this.begin(budget, clock);
      this.arm(ref, budget, clock);
      return;
    }
    this.halt(ref, 'time');
  }

  /** 진행 중이던 소비 구간을 activeMs 에 더하고 타이머를 푼다. 더한 ms 를 돌려준다. */
  private settle(budget: RequestBudget, sessionId: string): number {
    const clock = this.clocks.get(budget.id);
    if (!clock || clock.sessionId !== sessionId) return 0;
    if (clock.timer) clearTimeout(clock.timer);
    clock.timer = undefined;
    if (clock.since === undefined) return 0;
    const added = Math.max(0, this.deps.now() - clock.since);
    clock.since = undefined;
    delete budget.activeSince;
    budget.used.activeMs += added;
    return added;
  }

  private dropClock(budgetId: string): void {
    const clock = this.clocks.get(budgetId);
    if (clock?.timer) clearTimeout(clock.timer);
    this.clocks.delete(budgetId);
  }

  /** 멈춤 절차. 열린 예산만 멈추고 같은 예산에 두 번 들어오면 무시한다. 요약을 위한 모델 실행은 하지 않는다. */
  halt(ref: BudgetRef, reason: BudgetCounter): void {
    const session = this.deps.findSession(ref.sessionId);
    const budget = this.current(ref);
    if (!session || !budget || budget.state !== 'open') return;
    this.settle(budget, ref.sessionId);
    const stopped = this.deps.runningDelegationIds(ref);
    budget.state = 'halted';
    budget.halt = { reason, at: new Date(this.deps.now()).toISOString(), stoppedDelegationIds: [...stopped] };
    this.deps.cancelRecovery(session);
    this.deps.stopMain(ref.sessionId);
    for (const delegationId of stopped) this.deps.stopDelegation(ref.sessionId, delegationId);
    this.deps.stopAttachedTurns(ref);
    this.deps.holdQueued(session, budget.id);
    this.deps.save(session);
    this.deps.changed();
    this.deps.announce(session);
  }

  /** 대화가 조용해지면 닫고 닫았으면 true. 열린 예산만 닫고 멈춘 예산은 자동으로 닫지 않는다(D5). */
  maybeClose(session: BudgetSession): boolean {
    const budget = session.budget;
    if (!budget || budget.state !== 'open') return false;
    if ((this.clocks.get(budget.id)?.runs.size ?? 0) > 0 || !this.deps.isQuiet(session)) return false;
    // 여기서는 저장하지 않는다: 부르는 쪽(budget-wiring.ts closeAndKeep)이 대화 머리만 저장해 짧은 요청마다 대화 전체를 쓰지 않는다.
    this.close(budget, session.id, 'quiet');
    this.deps.changed();
    return true;
  }

  /**
   * 열린·멈춘 예산의 상한을 크기만큼 올리기만 한다(보낼 때 고른 크기). 항목마다 max(지금, n × 1묶음), 'unlimited' 는 unlimited 표시(흡수).
   * 1묶음이거나 바뀐 것이 없으면 false. 저장은 부르는 쪽(budget-wiring.ts raise)이 한다.
   */
  raise(ref: BudgetRef, size: BudgetSize): boolean {
    const budget = this.current(ref);
    if (!budget || budget.state === 'closed') return false;
    const clock = this.clocks.get(ref.budgetId);
    if (size === 'unlimited') {
      if (budget.unlimited) return false;
      budget.unlimited = true;
      // 걸린 시간 타이머를 취소한다. 소비 구간은 그대로 이어져 시간은 계속 센다(K4).
      if (clock?.timer) clearTimeout(clock.timer);
      if (clock) clock.timer = undefined;
      this.deps.changed();
      return true;
    }
    if (size === 1) return false;
    const unit = this.deps.unit();
    let changed = false;
    let timeRaised = false;
    for (const key of LIMIT_KEYS) {
      const next = Math.max(budget.limits[key], size * unit[key]);
      if (next === budget.limits[key]) continue;
      budget.limits[key] = next;
      changed = true;
      if (key === 'timeMinutes') timeRaised = true;
    }
    if (!changed) return false;
    // 시간 상한이 늘었으면 소비 중인 구간을 정산하고 남은 시간으로 타이머를 다시 건다.
    if (timeRaised && clock?.since !== undefined && !budget.unlimited) {
      this.settle(budget, ref.sessionId);
      this.refresh(ref);
    }
    this.deps.changed();
    return true;
  }

  /** 사람의 결정. extend 는 모든 항목에 size × 1묶음을 더하고(size 없으면 1) 쓴 양은 그대로 둔다. 'unlimited' 면 제한 없음 표시. */
  decide(sessionId: string, budgetId: string, choice: 'extend' | 'end', size?: BudgetSize): DecideResult {
    const session = this.deps.findSession(sessionId);
    const budget = session?.budget;
    if (!session || !budget || budget.id !== budgetId) return { ok: false, code: 'not-found' };
    if (budget.state !== 'halted') return { ok: false, code: 'not-halted' };
    if (choice === 'extend') {
      const reason = budget.halt?.reason;
      if (size === 'unlimited') budget.unlimited = true;
      else {
        const unit = this.deps.unit();
        for (const key of LIMIT_KEYS) budget.limits[key] += (size ?? 1) * unit[key];
      }
      budget.state = 'open';
      delete budget.halt;
      this.deps.save(session);
      this.deps.changed();
      // 멈춘 실행이 아직 목록에 남아 있으면 시간 소비를 다시 시작한다.
      this.refresh({ sessionId, budgetId });
      // 이어 가기 글을 먼저 대기에 넣고(깨우지 않음) 보류 결과를 풀며 깨운다. 같은 예산 참조라 한 턴으로 합쳐진다. 풀 것이 없었으면 그때 한 번 깨운다.
      if (reason) this.deps.queueContinue(session, budget.id, reason, false);
      if (!this.deps.releaseHeld(session, budget.id)) this.deps.queueContinue(session, budget.id, null, true);
      return { ok: true };
    }
    this.close(budget, sessionId, 'user-end');
    this.deps.dropHeld(session, budget.id);
    this.deps.save(session);
    this.deps.changed();
    return { ok: true };
  }

  /** 작업 카드를 검사해 저장한다. 잘못된 입력은 던지지 않고 error 로 돌려준다. */
  setCard(ref: BudgetRef, card: Omit<WorkCard, 'updatedAt' | 'by'>, language: InstructionLanguage = 'ko', by?: WorkCard['by']): SetCardResult {
    const text = engineText(BUDGET_TEXT.card, language);
    const session = this.deps.findSession(ref.sessionId);
    const budget = this.current(ref);
    if (!session || !budget || budget.state === 'closed') return { ok: false, error: text.missing };
    const input = card as Record<string, unknown>;
    if (!validText(input.deliverable)) return { ok: false, error: text.deliverable };
    if (!validLines(input.done, 1, 5)) return { ok: false, error: text.done };
    const next: WorkCard = { deliverable: input.deliverable.trim(), done: input.done.map(line => line.trim()), updatedAt: new Date(this.deps.now()).toISOString() };
    if (input.targets !== undefined) {
      const targets = input.targets as Record<string, unknown> | null;
      const counts = [targets?.confirmed, targets?.candidate, targets?.unknown];
      const noteOk = targets?.note === undefined || (typeof targets.note === 'string' && targets.note.length <= CARD_TEXT_MAX);
      if (!targets || !counts.every(count => Number.isInteger(count) && (count as number) >= 0) || !noteOk) return { ok: false, error: text.targets };
      next.targets = { confirmed: counts[0] as number, candidate: counts[1] as number, unknown: counts[2] as number, ...(typeof targets.note === 'string' && targets.note ? { note: targets.note } : {}) };
    }
    if (input.verify !== undefined) {
      const verify = input.verify;
      if (!Array.isArray(verify) || verify.length > 5 || !verify.every(item => item && validText(item.label) && VERIFY_STATUS.has(item.status))) return { ok: false, error: text.verify };
      next.verify = verify.map(item => ({ label: item.label.trim(), status: item.status }));
    }
    if (input.conflicts !== undefined) {
      if (!validLines(input.conflicts, 0, 5)) return { ok: false, error: text.conflicts };
      next.conflicts = input.conflicts.map(line => line.trim());
    }
    if (input.state !== undefined) {
      if (typeof input.state !== 'string' || !CARD_STATE.has(input.state)) return { ok: false, error: text.state };
      next.state = input.state as NonNullable<WorkCard['state']>;
    }
    if (by) next.by = { sessionId: by.sessionId, turnId: by.turnId };
    budget.card = next;
    this.deps.save(session);
    this.deps.changed();
    return { ok: true, budget };
  }

  hasCard(ref: BudgetRef): boolean {
    return Boolean(this.current(ref)?.card);
  }

  /** 사람이 쓴 새 요청이 이 예산에서 시작한다: 자동 이어가기 회차·중지 표시를 지운다. 저장은 부르는 쪽(턴 접수)이 한다. */
  clearAutoContinue(ref: BudgetRef): void {
    const budget = this.current(ref);
    if (budget?.autoContinue) delete budget.autoContinue;
  }

  /** 자동 이어가기 접수 기록(회차·마지막으로 이어 간 요청). undefined 면 지운다. 기록했으면 true. 저장은 부르는 쪽(budget-wiring.ts recordAutoContinue)이 예산 주인 대화에 한다. */
  recordAutoContinue(ref: BudgetRef, record: RequestBudget['autoContinue'] | undefined): boolean {
    const budget = this.current(ref);
    if (!budget) return false;
    if (record) budget.autoContinue = record; else delete budget.autoContinue;
    return true;
  }

  /**
   * 사용자가 이 대화를 중지했다: 열린 예산의 자동 이어가기를 멈춘 것으로 남긴다. 예산은 그 대화의 마지막 요청이 가리키는 예산(ref, 다른 대화 것일 수 있다)을 먼저 보고
   * 없으면 이 대화 자신의 예산을 본다. 기록이 이 대화 것이면 stopped 만 켜고, 기록이 없을 때만 카드를 이 대화가 working 으로 쓴 경우에
   * 빈 기록(회차 0)에 stopped 를 둔다(다른 대화 기록은 덮지 않는다). 바뀌었으면 예산 주인 대화를 저장한다.
   */
  stopAutoContinue(sessionId: string, ref?: BudgetRef): void {
    const pointed = ref ? this.current(ref) : undefined;
    const owner = this.deps.findSession(pointed && ref ? ref.sessionId : sessionId);
    const budget = pointed ?? owner?.budget;
    if (!owner || !budget || budget.state !== 'open') return;
    const record = budget.autoContinue;
    if (record?.sessionId === sessionId) {
      if (record.stopped) return;
      record.stopped = true;
    } else if (!record && budget.card?.state === 'working' && budget.card.by?.sessionId === sessionId) {
      budget.autoContinue = { sessionId, userTurnId: '', count: 0, stopped: true };
    } else return;
    this.deps.save(owner);
    this.deps.changed();
  }

  /** 기동 복구(D8): 저장된 사용량과 상태를 그대로 두고 타이머도 걸지 않는다. 열린 시간 구간은 버린다(덜 세는 쪽). halted 는 유지. */
  bootRestore(session: BudgetSession): void {
    if (!session.budget) return;
    this.dropClock(session.budget.id);
    delete session.budget.activeSince;
  }

  /** 대화를 지울 때 그 대화의 시간 상태와 타이머를 푼다. */
  forgetSession(sessionId: string): void {
    for (const [budgetId, clock] of this.clocks) if (clock.sessionId === sessionId) this.dropClock(budgetId);
  }

  /** 앱 종료: 타이머를 모두 푼다. */
  dispose(): void {
    for (const clock of this.clocks.values()) if (clock.timer) clearTimeout(clock.timer);
    this.clocks.clear();
    this.toolSeen.clear();
  }
}
