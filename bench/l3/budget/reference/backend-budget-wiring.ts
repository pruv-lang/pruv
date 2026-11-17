// 요청 전체 예산의 서비스 연결(docs/notes/request-budget-2026-10-24.md 설계 + 논쟁 D1~D9). 서비스 객체를 받지 않고 BudgetHost 의 좁은 함수만 받는다.
// 상태기계는 request-budget.ts(RequestBudgets)가 맡고, 여기는 deps 구현·시작 문 판정·위임 차감·작업 카드 도구·요청 처리 표만 둔다.
import { randomUUID } from 'node:crypto';
import type { AttentionEvent, QueuedRequest, SentFrom, Session, Snapshot, Turn } from '../shared/contracts';
import type { Delegation } from '../shared/delegation-types';
import { engineText, type EngineTextEntry, type InstructionLanguage } from '../shared/engine-text';
import { WireError, wire, wireKo, type WireText } from '../shared/i18n/wire';
import { BUDGET_SIZES } from '../shared/request-budget';
import { WORK_CARD_STATES, WORK_CARD_VERIFY_STATUSES } from '../shared/schedule-lists';
import type { BudgetCounter, BudgetRef, BudgetSize, BudgetUnit, RequestBudget } from '../shared/request-budget';
import { delegationBudgetRef } from './budget-ref';
import type { AppTool } from './engine-contract';
import { RequestBudgets, cardMissing, continueText, limitText, refusal, refusalWire, type ChargeWhat } from './request-budget';
import { RUNNING } from './requests/common';
// 서비스가 설정 칸 → 1묶음 단위를 스냅숏에도 쓰므로 연결 모듈에서 함께 내보낸다.
export { budgetUnitOf } from './request-budget';
import type { AppRequestHandlers } from './requests/types';

/** 연결이 서비스에서 빌리는 것. 서비스 객체 전체를 넘기지 않고 필요한 함수만 담는다. */
export interface BudgetHost {
  session(id: string): Session | undefined;
  sessions(): Iterable<Session>;
  /** 대화 저장(실패해도 던지지 않는다) */
  save(session: Session): void;
  /** 대화 머리(session.budget 포함)만 저장한다. 전체 대화(모든 요청 직렬화)를 쓰지 않고 닫힘을 남긴다. 던질 수 있다. */
  saveHead(session: Session): void;
  changed(): void;
  /** 메인 턴 중지(stopReason 'budget') */
  stopMain(sessionId: string): void;
  /** 위임 중지(그 위임이 든 대화 번호로) */
  stopDelegation(sessionId: string, delegationId: string): void;
  /** 대기 중 pending 항목을 모두 held 로 둔다(알림 없이) */
  holdQueue(session: Session, reason: string, reasonI18n: WireText): void;
  drain(session: Session): void;
  /** 예약된 복구 취소 */
  cancelRecovery(session: Session): void;
  /** 실행 중 턴·실행 준비(sendPreparing 포함)·새로 시작·복구 판정 중 하나라도 있다 */
  busy(sessionId: string): boolean;
  /** 이 대화에서 실행 준비 중인 요청이 붙을 예산 참조. 준비 중이 아니면 undefined */
  preparingBudget(sessionId: string): BudgetRef | undefined;
  announce(session: Session, event: Omit<AttentionEvent, 'sessionId' | 'title'>): void;
  languageOf(session: Session): InstructionLanguage;
  now(): number;
}

/** 멈춤으로 보류한 대기의 까닭. 정본은 src/shared/i18n/budget.ts 의 budget.attention.note 다(문장 칸은 그 한국어 글). */
const HOLD_I18N: WireText = wire('budget.attention.note');
const HOLD_REASON = wireKo(HOLD_I18N);
/** 끝낸·교체된 예산의 늦은 결과를 보류한 까닭(budget.held.ended). 한도 멈춤과 글을 나눈다(N02). */
const ENDED_I18N: WireText = wire('budget.held.ended');
const ENDED_REASON = wireKo(ENDED_I18N);
/** 도구 실행으로 세는 메인 활동 종류(Activity.kind). */
const TOOL_KINDS: ReadonlySet<string> = new Set(['command', 'file-change', 'tool', 'web-search', 'agent']);

// 모델에게 돌려주는 글. 언어 처리는 request-budget.ts 와 같다(engineText 로 지시문 언어 값을 꺼내고 값이 없으면 ko).
const CARD_TOOL_TEXT = {
  description: {
    ko: /* i18n-keep: 모델에게만 가는 work_card 도구 설명 */ '여러 단계·위임이 필요한 요청을 시작할 때 산출물과 완료 조건을 먼저 적는다. 범위 밖이라 하지 않을 일은 conflicts 에 적고 사용자에게 넘긴다. 예산은 앱이 정하고 모델은 늘릴 수 없다. 다시 불러 갱신한다.',
    en: 'When starting a request that needs several steps or delegation, first write the deliverable and the done conditions. Put work you will not do because it is out of scope in conflicts and hand it to the user. The app sets the budget and the model cannot raise it. Call again to update.',
  } as EngineTextEntry<string>,
  deliverable: { ko: /* i18n-keep: 모델에게만 가는 work_card 도구 인자 설명 */ '산출물 한 줄(1~300자)', en: 'The deliverable in one line (1–300 characters)' } as EngineTextEntry<string>,
  done: { ko: /* i18n-keep: 모델에게만 가는 work_card 도구 인자 설명 */ '완료 조건 1~5줄. 탐색형이면 답할 질문과 조사 경계', en: '1–5 done conditions. For exploration, the question to answer and the research boundary' } as EngineTextEntry<string>,
  targets: { ko: /* i18n-keep: 모델에게만 가는 work_card 도구 인자 설명 */ '대상 수(확정·후보·미확인, 0 이상 정수)와 메모', en: 'Target counts (confirmed, candidate, unknown; non-negative integers) and a note' } as EngineTextEntry<string>,
  verify: { ko: /* i18n-keep: 모델에게만 가는 work_card 도구 인자 설명 */ '검증 계획 0~5개(planned·done·skipped)', en: '0–5 verification plan items (planned, done, skipped)' } as EngineTextEntry<string>,
  conflicts: { ko: /* i18n-keep: 모델에게만 가는 work_card 도구 인자 설명 */ '범위 밖이라 하지 않고 사용자에게 넘기는 일 0~5줄', en: '0–5 lines of work left out of scope and handed to the user' } as EngineTextEntry<string>,
  /** Codex 요청에만 붙는 카드 상태 인자 설명과, 상태 없이 불렀을 때의 오류. */
  state: {
    ko: /* i18n-keep: 모델에게만 가는 work_card 도구 인자 설명(Codex 전용) */ '일이 남았으면 working, 다 끝났으면 done, 사용자 답·결정이 필요하면 waiting-user, 외부 조건으로 막혔으면 blocked.',
    en: 'working if work remains, done if everything is finished, waiting-user if a user answer or decision is needed, blocked if an external condition blocks the work.',
  } as EngineTextEntry<string>,
  stateMissing: {
    ko: /* i18n-keep: 모델에게만 가는 work_card 도구 오류(Codex 전용) */ 'state 가 필요하다. working·done·waiting-user·blocked 중 하나를 넣어 다시 부른다.',
    en: 'state is required. Call again with one of working, done, waiting-user or blocked.',
  } as EngineTextEntry<string>,
  saved: {
    ko: (usage: string) => /* i18n-keep: 모델에게만 가는 work_card 도구 결과 */ `작업 카드를 저장했다. 지금 사용량: ${usage}. 예산은 앱이 정하고 늘릴 수 없다.`,
    en: (usage: string) => `The work card was saved. Usage so far: ${usage}. The app sets the budget and it cannot be raised.`,
  } as EngineTextEntry<(usage: string) => string>,
  usage: {
    ko: (v: UsageNumbers) => /* i18n-keep: 모델에게만 가는 work_card 도구 결과 */ `시간 ${v.minutes}/${v.limitMinutes}, 위임 ${v.delegations}/${v.limitDelegations}, 빌드·명령 ${v.commands}/${v.limitCommands}, 감사 ${v.reviews}/${v.limitReviews}, 실제 확인 ${v.checks}/${v.limitChecks}`,
    en: (v: UsageNumbers) => `time ${v.minutes}/${v.limitMinutes}, delegations ${v.delegations}/${v.limitDelegations}, builds·commands ${v.commands}/${v.limitCommands}, reviews ${v.reviews}/${v.limitReviews}, real checks ${v.checks}/${v.limitChecks}`,
  } as EngineTextEntry<(v: UsageNumbers) => string>,
  /** 시간 상한(분 단위를 붙인다). 제한 없음이면 limitText 를 쓴다. */
  minutes: { ko: (n: number) => /* i18n-keep: 모델에게만 가는 work_card 도구 결과 */ `${n}분`, en: (n: number) => `${n} min` } as EngineTextEntry<(n: number) => string>,
};

/** 상한 칸은 글자다: 제한 없음 예산이면 숫자 대신 '제한 없음'('no limit'). 시간 상한은 단위까지 붙은 글자다. */
interface UsageNumbers { minutes: number; limitMinutes: string; delegations: number; limitDelegations: string; commands: number; limitCommands: string; reviews: number; limitReviews: string; checks: number; limitChecks: string }

/** 차감 결과. 거절이면 refuse(모델용 글)와 refuseI18n(사용자 오류용 wire 글). */
export type ChargeOutcome = { ref?: BudgetRef; refuse?: string; refuseI18n?: WireText };

/** 크기 중 가장 큰 것(1<2<4<8<'unlimited'). 값이 하나도 없으면 undefined. */
function largestSize(sizes: Array<BudgetSize | undefined>): BudgetSize | undefined {
  let best: BudgetSize | undefined;
  for (const size of sizes) {
    if (size === undefined) continue;
    if (best === undefined || BUDGET_SIZES.indexOf(size) > BUDGET_SIZES.indexOf(best)) best = size;
  }
  return best;
}

/** 시작 문(startTurn)이 예산을 정하는 데 보는 요청 내용. */
export interface BudgetStartSpec { text: string; fromQueue?: QueuedRequest[]; budget?: BudgetRef; recovery?: unknown; sentFrom?: SentFrom; budgetSize?: BudgetSize }

export class BudgetWiring {
  readonly budgets: RequestBudgets;

  /** unit: 설정의 1묶음(설정이 없으면 DEFAULT_BUDGET 숫자에 defaultUnlimited). 새 예산을 여는 크기의 단위이자 늘리기의 단위다. */
  constructor(private readonly host: BudgetHost, deps: { unit: () => BudgetUnit }) {
    this.budgets = new RequestBudgets({
      now: () => host.now(),
      unit: () => deps.unit(),
      findSession: id => host.session(id),
      save: session => host.save(host.session(session.id) ?? (session as Session)),
      changed: () => host.changed(),
      stopMain: sessionId => host.stopMain(sessionId),
      stopDelegation: (_sessionId, delegationId) => this.stopDelegation(delegationId),
      runningDelegationIds: ref => this.runningDelegations(ref).map(item => item.id),
      stopAttachedTurns: ref => this.stopAttachedTurns(ref),
      holdQueued: (session, budgetId) => this.holdQueued(session as Session, { sessionId: session.id, budgetId }),
      releaseHeld: (session, budgetId) => this.releaseHeld(session as Session, budgetId),
      carryHeld: (owner, oldBudgetId, newRef) => this.carryHeld(owner as Session, oldBudgetId, newRef),
      queueContinue: (session, budgetId, reason, wake) => this.queueContinue(session as Session, budgetId, reason, wake),
      dropHeld: (session, budgetId) => this.dropHeld(session as Session, budgetId),
      cancelRecovery: session => host.cancelRecovery(session as Session),
      isQuiet: session => this.isQuiet(session as Session),
      announce: session => host.announce(session as Session, { kind: 'budget', body: wireKo(wire('budget.attention.halted')), bodyI18n: wire('budget.attention.halted') }),
    });
  }

  // ---- 멈춤 deps 구현 ----

  /** 이 예산에 붙은 지금 도는 위임(여러 대화에 걸칠 수 있다). */
  private runningDelegations(ref: BudgetRef): Array<{ id: string; sessionId: string }> {
    const found: Array<{ id: string; sessionId: string }> = [];
    for (const session of this.host.sessions()) {
      for (const item of session.delegations ?? []) {
        if (item.status === 'running' && item.budgetId === ref.budgetId && (session.id === ref.sessionId || delegationBudgetRef(session, item)?.sessionId === ref.sessionId)) found.push({ id: item.id, sessionId: session.id });
      }
    }
    return found;
  }

  private stopDelegation(delegationId: string): void {
    for (const session of this.host.sessions()) {
      if (!session.delegations?.some(item => item.id === delegationId)) continue;
      // 이미 끝난 위임을 멈추라는 거절이 멈춤 절차(다른 위임·턴 멈추기, 대기 보류)를 끊지 않게 삼킨다.
      try { this.host.stopDelegation(session.id, delegationId); } catch { /* 이미 끝났거나 찾을 수 없다 */ }
      return;
    }
  }

  /** 다른 대화의 턴 중 이 예산에 붙어 도는 것을 멈춘다. */
  private stopAttachedTurns(ref: BudgetRef): void {
    for (const session of this.host.sessions()) {
      if (session.id === ref.sessionId) continue;
      // 실행 준비 중인 요청도 시작하지 않게 취소 표지를 세운다(실제 접수 직전의 멈춤 재검사가 한 번 더 막는다).
      const preparing = this.host.preparingBudget(session.id);
      if (session.turns.some(turn => RUNNING.has(turn.status) && turn.budget?.budgetId === ref.budgetId && turn.budget.sessionId === ref.sessionId)
        || (preparing?.budgetId === ref.budgetId && preparing.sessionId === ref.sessionId)) this.host.stopMain(session.id);
    }
  }

  /** 대기 항목을 모두 held 로 둔다. 어느 예산 때문인지 푸는 쪽이 알도록 항목에 예산 참조(예산 주인 대화 기준)를 남긴다. */
  private holdQueued(session: Session, ref: BudgetRef): void {
    for (const item of session.queue ?? []) if (item.state === 'pending' && !item.budget) item.budget = { ...ref };
    this.host.holdQueue(session, HOLD_REASON, HOLD_I18N);
  }

  /** 이 예산 때문에 held 가 된 항목을 같은 예산 참조를 가진 모든 대화에서 pending 으로 풀고 깨운다. 풀 것이 있었으면 true. */
  private releaseHeld(_owner: Session, budgetId: string): boolean {
    let released = false;
    for (const session of this.host.sessions()) {
      const held = (session.queue ?? []).filter(item => item.state === 'held' && item.budget?.budgetId === budgetId);
      if (!held.length) continue;
      for (const item of held) { item.state = 'pending'; delete item.heldReason; delete item.heldReasonI18n; delete item.heldFor; }
      this.host.save(session);
      this.host.changed();
      this.host.drain(session);
      released = true;
    }
    return released;
  }

  /** 멈춘 예산이 사용자 새 글로 교체될 때: 주인이 아닌 대화의 옛 예산 대기(held·pending)를 새 참조로 옮기고 held 는 풀어 깨운다. 주인 자신의 항목은 건드리지 않는다(F05). */
  private carryHeld(owner: Session, oldBudgetId: string, newRef: BudgetRef): void {
    for (const session of this.host.sessions()) {
      if (session.id === owner.id) continue;
      const items = (session.queue ?? []).filter(item => (item.state === 'held' || item.state === 'pending') && item.budget?.budgetId === oldBudgetId);
      // 늦게 끝날 하위 위임의 결과가 새 예산에 들도록 그 대화의 턴·위임 기록도 새 참조로 옮긴다.
      const turns = session.turns.filter(turn => turn.budget?.budgetId === oldBudgetId && turn.budget.sessionId === owner.id);
      const delegations = (session.delegations ?? []).filter(item => item.budgetId === oldBudgetId && delegationBudgetRef(session, item)?.sessionId === owner.id);
      if (!items.length && !turns.length && !delegations.length) continue;
      for (const item of items) {
        item.budget = { ...newRef };
        if (item.state === 'held') { item.state = 'pending'; delete item.heldReason; delete item.heldReasonI18n; delete item.heldFor; }
      }
      for (const turn of turns) turn.budget = { ...newRef };
      for (const item of delegations) item.budgetId = newRef.budgetId;
      this.host.save(session);
      this.host.changed();
      this.host.drain(session);
    }
  }

  /** 참조가 가리키는 예산이 사라졌고 주인 대화의 지금 열린 예산이 그것을 이어받았으면(follows) 주인의 지금 참조. 아니면 undefined. */
  private followedRef(ref: BudgetRef): BudgetRef | undefined {
    if (this.budgets.current(ref)) return undefined;
    const budget = this.host.session(ref.sessionId)?.budget;
    // 이어받기 식별만 한다(open·halted). 허용 여부는 시작 문의 멈춤 판정이 맡는다.
    return budget && budget.state !== 'closed' && budget.follows === ref.budgetId ? { sessionId: ref.sessionId, budgetId: budget.id } : undefined;
  }

  private queueContinue(session: Session, budgetId: string, reason: BudgetCounter | null, wake: boolean): void {
    if (reason && session.budget) {
      session.queue = [...(session.queue ?? []), { id: randomUUID(), text: continueText(reason, session.budget, this.host.languageOf(session)), createdAt: new Date(this.host.now()).toISOString(), state: 'pending', budget: { sessionId: session.id, budgetId } }];
      this.host.save(session);
      this.host.changed();
    }
    if (wake) this.host.drain(session);
  }

  /** 같은 예산 참조를 가진 모든 대화의 held 항목을 지운다. */
  private dropHeld(_owner: Session, budgetId: string): void {
    for (const session of this.host.sessions()) {
      if (!(session.queue ?? []).some(item => item.state === 'held' && item.budget?.budgetId === budgetId)) continue;
      session.queue = (session.queue ?? []).filter(item => !(item.state === 'held' && item.budget?.budgetId === budgetId));
      if (!session.queue.length) delete session.queue;
      this.host.save(session);
    }
  }

  /**
   * 서비스 쪽 남은 일이 없다: 실행 중 턴·준비·pending·waiting 대기·복구 예약이 모두 없고, 그 예산 참조를 가진 다른 대화의 대기·실행 준비도 없다.
   * 닫힘 판정은 턴 끝·위임 끝 같은 드문 때만 돌아 대화 목록을 훑어도 된다.
   */
  private isQuiet(session: Session): boolean {
    if (this.host.busy(session.id) || session.recovery || (session.queue ?? []).some(item => item.state === 'pending' || item.state === 'waiting')) return false;
    const budgetId = session.budget?.id;
    if (!budgetId) return true;
    for (const other of this.host.sessions()) {
      if (other.id === session.id) continue;
      if (this.host.preparingBudget(other.id)?.budgetId === budgetId) return false;
      if ((other.queue ?? []).some(item => (item.state === 'pending' || item.state === 'waiting') && item.budget?.budgetId === budgetId)) return false;
    }
    return true;
  }

  // ---- 멈춤 판정 문(D4): 판정은 RequestBudgets.isHalted 하나만 쓴다 ----

  /** 이 대화 자신의 지금 예산이 멈춤인가(새로 시작 요약·압축 시작이 본다). */
  sessionHalted(session: Session): boolean {
    return session.budget !== undefined && this.budgets.isHalted({ sessionId: session.id, budgetId: session.budget.id });
  }

  /** 도는 요청이 붙은 예산이 멈춤인가(실행 중 추가 지시가 본다). */
  turnHalted(turn: Turn): boolean {
    return turn.budget !== undefined && this.budgets.isHalted(turn.budget);
  }

  /**
   * 그 위임 결과를 held 로 넣을 이유(결과 묶음 넣기가 본다). 위임 레코드의 budgetId(와 주인 대화)로 판정한다: 사람이 직접 맡긴 위임은 Turn 이 없어도 맞다.
   * 한도로 멈췄으면 budget.attention.note, 사용자가 끝냈거나 교체돼 찾을 수 없으면(끝난 요청의 늦은 결과) budget.held.ended 로 held 에만 둔다. 예산이 없던 위임·그 밖이면 undefined.
   */
  deliveryHold(session: Session, item: Delegation): { text: string; i18n: WireText } | undefined {
    const ref = delegationBudgetRef(session, item);
    if (!ref) return undefined;
    const budget = this.budgets.current(ref);
    if (budget?.state === 'halted') return { text: HOLD_REASON, i18n: HOLD_I18N };
    // 다른 대화 결과가 교체로 이어진 예산에 붙을 수 있으면 보류하지 않는다. 같은 대화의 늦은 결과는 그대로 held(F05).
    if (!budget && ref.sessionId !== session.id && this.followedRef(ref)) return undefined;
    return !budget || budget.ended?.how === 'user-end' ? { text: ENDED_REASON, i18n: ENDED_I18N } : undefined;
  }

  // ---- 턴 시작·끝 ----

  /**
   * startTurn 의 최종 접수 문이 이번 턴의 예산을 정한다. 대기 항목이나 요청에 예산 참조가 있으면 그것(닫혔으면 받는 대화의 예산에 다시 붙는다),
   * 사용자 글이면 멈춘 예산을 닫고 새로 열기(replaceForUserMessage), 예약 실행이면 'schedule', 위임 결과만 있는 전달이면 'release', 그 밖은 열린 예산에 붙는다.
   */
  startRef(session: Session, spec: BudgetStartSpec): BudgetRef {
    const ref = this.pickRef(session, spec);
    return ref;
  }

  /** 보낼 때 고른 크기(K1·K2)의 가장 큰 것. AI 출처 지시는 크기를 싣지 못한다(5절). 적용은 startTurn 이 턴을 접수하는 지점에서 raise 로 한다. */
  startSize(spec: BudgetStartSpec): BudgetSize | undefined {
    const aiSent = spec.sentFrom?.via === 'ai' || Boolean(spec.fromQueue?.some(item => item.sentFrom?.via === 'ai'));
    return aiSent ? undefined : largestSize([spec.budgetSize, ...(spec.fromQueue ?? []).map(item => item.budgetSize)]);
  }

  /**
   * 크기를 열린·멈춘 예산에 적용한다(올리기만). 바뀌었으면 그 예산의 주인 대화(ref.sessionId)의 머리만 저장하고 화면에 알린다(K2).
   * 실행 중 추가 지시·대기 글처럼 startTurn 을 지나지 않는 경로도 이것을 직접 부른다. skipSave 는 startTurn 이 곧 대화를 저장하는 경로다.
   */
  raise(ref: BudgetRef | undefined, size: BudgetSize | undefined, skipSave = false): void {
    if (!ref || size === undefined) return;
    if (!this.budgets.raise(ref, size)) return;
    const owner = skipSave ? undefined : this.host.session(ref.sessionId);
    if (owner) { try { this.host.saveHead(owner); } catch { /* 다음 저장에 함께 남는다 */ } }
    this.host.changed();
  }

  /**
   * 자동 이어가기 기록을 예산에 쓰고 그 예산의 주인 대화(ref.sessionId) 머리를 저장한다. 예산 주인이 받은 대화가 아닐 수 있어
   * 받은 대화의 접수 저장에 실린다고 가정하지 않는다. 저장이 던져도 다음 저장에 함께 남는다.
   */
  recordAutoContinue(ref: BudgetRef, record: RequestBudget['autoContinue'] | undefined): void {
    if (!this.budgets.recordAutoContinue(ref, record)) return;
    const owner = this.host.session(ref.sessionId);
    if (owner) { try { this.host.saveHead(owner); } catch { /* 다음 저장에 함께 남는다 */ } }
    this.host.changed();
  }

  private pickRef(session: Session, spec: BudgetStartSpec): BudgetRef {
    // AI 가 앱 동작으로 보낸 지시(D2)는 보낸 요청(sentFrom 의 대화·요청)의 예산에 붙는다. 보내는 쪽 app-actions.ts 가 그 예산에 이미 위임 1건을 셌다.
    const explicit = spec.budget ?? this.senderRef(spec.sentFrom) ?? spec.fromQueue?.map(item => this.senderRef(item.sentFrom)).find(Boolean) ?? spec.fromQueue?.find(item => item.budget)?.budget;
    if (explicit) {
      const live = this.budgets.current(explicit);
      if (live && live.state !== 'closed') return { ...explicit };
      return (!live && explicit.sessionId !== session.id ? this.followedRef(explicit) : undefined) ?? this.budgets.attach(session, 'release', this.startSize(spec));
    }
    if (spec.recovery || spec.fromQueue?.some(item => item.continuation)) return this.budgets.attach(session, 'user');
    if (spec.fromQueue && !spec.fromQueue.some(item => !item.delivery)) return this.budgets.attach(session, 'release');
    // 예약 회차(첫 요청이든 이어 가는 대화의 뒤 요청이든)는 launchSchedule 이 scheduleFresh·scheduleStart 로 정한 참조를 spec.budget 으로 싣고 와 위 explicit 에서 끝난다(F4).
    return this.budgets.replaceForUserMessage(session, this.startSize(spec));
  }

  /** AI 가 보낸 지시의 보낸 요청이 붙은 예산 참조. 보낸 사람이 AI 가 아니거나 보낸 요청에 예산이 없으면 undefined. */
  private senderRef(from: SentFrom | undefined): BudgetRef | undefined {
    if (from?.via !== 'ai') return undefined;
    const ref = this.host.session(from.sessionId)?.turns.find(turn => turn.id === from.turnId)?.budget;
    return ref ? { ...ref } : undefined;
  }

  /** 멈춤이라 시작하지 않을 때 대기를 held 로 둔다. */
  holdForHalt(session: Session, ref: BudgetRef): void {
    this.holdQueued(session, ref);
  }

  /** 턴이 실제로 돌기 시작했다(active 등록 직후). */
  turnStarted(turn: Turn): void {
    if (turn.budget) this.budgets.runStarted(turn.budget, `turn:${turn.id}`);
  }

  /** 승인·질문 대기가 열리거나 닫혔다: 턴 상태로 대기 여부를 맞춘다. */
  syncWaiting(turn: Turn): void {
    if (turn.budget) this.budgets.runWaiting(turn.budget, `turn:${turn.id}`, turn.status === 'waiting-approval');
  }

  /** 메인 도구 활동 하나(활동 번호로 중복 제거). 말·생각·계획·그림·알림 줄은 도구 실행이 아니라 세지 않는다. */
  noteTool(turn: Turn, activity: { id: string; kind: string }): void {
    if (turn.budget && TOOL_KINDS.has(activity.kind)) this.budgets.noteTool(turn.budget, turn.id, activity.id);
  }

  /** 턴이 끝났다(완료·실패·중지): 실행 구간을 닫는다. 끝 결과 저장보다 앞에서 불러 쓴 시간이 그 저장에 실리게 한다. */
  turnRunEnded(turn: Turn): void {
    this.budgets.forgetTurn(turn.id);
    if (turn.budget) this.budgets.runEnded(turn.budget, `turn:${turn.id}`);
  }

  /** 턴 끝 처리가 모두 끝났다: 예산 주인 대화가 조용해졌으면 닫는다. */
  turnEnded(turn: Turn): void {
    if (turn.budget) this.closeIfQuiet(turn.budget);
  }

  /** 압축·요약 대상 턴(마지막 턴)이 붙은 예산 참조. 다른 대화의 예산으로 돈 턴이면 그 예산이고, 턴에 참조가 없으면 대화의 지금 예산이다. */
  private compactionRef(session: Session): BudgetRef | undefined {
    return session.turns.at(-1)?.budget ?? (session.budget ? { sessionId: session.id, budgetId: session.budget.id } : undefined);
  }

  /** 압축·요약 실행(대화 안의 한 실행 자리)도 대상 턴이 붙은 예산의 시간에 든다. 열린 예산이 아니면 센 것이 없다. */
  compactionStarted(session: Session, runId: string): void {
    const ref = this.compactionRef(session);
    if (ref && this.budgets.current(ref)?.state === 'open') this.budgets.runStarted(ref, `cmp:${runId}`);
  }

  compactionEnded(session: Session, runId: string): void {
    const ref = this.compactionRef(session);
    if (ref) this.budgets.runEnded(ref, `cmp:${runId}`);
  }

  /** 위임 작업자의 사용자 승인 대기가 열리거나 닫혔다: 그동안은 시간으로 세지 않는다(D7). */
  delegationWaiting(sessionId: string, delegationId: string, waiting: boolean): void {
    const ref = this.refOfDelegation(sessionId, delegationId);
    if (ref) this.budgets.runWaiting(ref, `del:${delegationId}`, waiting);
  }

  /** AI 가 만들거나 고친 예약이 가리키는 예산 참조: 부른 요청(sentFrom 의 대화·요청)의 예산. AI 가 아니거나 예산이 없으면 undefined. */
  callerBudget(caller: SentFrom | undefined): BudgetRef | undefined {
    return this.senderRef(caller);
  }

  /**
   * 예약 실행의 예산 판정(D3). 가리키는 예산이 멈춤이면 막고(halted), 사용자가 끝냈으면 막으며(ended), 대화의 예산이 다른 것으로 바뀌어 찾을 수 없으면
   * 새 예산으로 대체하지 않고 막는다(replaced). 조용히 닫혔으면 같은 예산을 다시 열어, 열려 있으면 거기에 붙여 위임 1건을 센다(ref). 예산 주인은 이 예약이 새로 만드는 대화가 아니라 다른 대화라
   * 여기서 센 것과 다시 연 상태는 그 대화의 기존 저장이 없어 직접 저장한다.
   */
  scheduleStart(ref: BudgetRef): { ref?: BudgetRef; blocked?: 'halted' | 'ended' | 'replaced' } {
    const budget = this.budgets.current(ref);
    if (!budget) return { blocked: 'replaced' };
    if (budget.ended?.how === 'user-end') return { blocked: 'ended' };
    if (budget.state === 'halted') return { blocked: 'halted' };
    if (budget.state === 'closed' && !this.budgets.reopen(ref)) return { blocked: 'replaced' };
    if (!this.budgets.charge(ref, { delegations: 1 }).ok) return { blocked: 'halted' };
    const owner = this.host.session(ref.sessionId);
    if (owner) this.host.save(owner);
    return { ref: { ...ref } };
  }

  /**
   * 사람이 만든 예약의 회차 예산(F4). 이 회차를 받을 대화에 열린 예산이 있어도 재사용하지 않는다: 조용하면 닫은 뒤 새 'schedule' 예산을 연다.
   * 멈춘 앞 작업의 예산은 사용자가 새 글을 보낼 때와 같이 닫고 바꾼다(replaceForUserMessage). 아직 조용하지 않은 열린 예산은 새로 열 수 없어 막는다(halted, 보통은 scheduleFreshBusy 가 접수 전에 먼저 거른다).
   * 여는 변경의 저장은 이어지는 startTurn 접수 저장이 싣는다.
   */
  scheduleFresh(session: Session): { ref?: BudgetRef; blocked?: 'halted' } {
    if (this.scheduleFreshBusy(session)) return { blocked: 'halted' };
    return { ref: this.budgets.replaceForUserMessage(session, undefined, 'schedule') };
  }

  /** 새 회차 예산을 지금 열 수 없는지: 조용하면 닫은 뒤에도 열린 채 남은 예산(일이 붙어 있음). 멈춤·닫힘·없음은 바꾸거나 새로 열 수 있어 바쁘지 않다. */
  scheduleFreshBusy(session: Session): boolean {
    this.closeAndKeep(session);
    return session.budget?.state === 'open';
  }

  /** 예산 주인 대화가 조용하면 닫는다. */
  closeIfQuiet(ref: BudgetRef): void {
    const owner = this.host.session(ref.sessionId);
    if (owner) this.closeAndKeep(owner);
  }

  /** 닫았으면 머리만 저장해 닫힘을 남긴다(F15). 결과 저장 뒤에 닫혀도 DB 에 open 으로 남지 않는다. 저장이 실패하면 다음 저장에 실리고, 그 전에 꺼지면 기동 복구가 다시 판정한다. */
  private closeAndKeep(session: Session): void {
    if (!this.budgets.maybeClose(session)) return;
    try { this.host.saveHead(session); } catch { /* 다음 저장에 함께 남는다 */ }
  }

  /** 대기 취소·비우기 뒤에도 같은 닫힘 판정을 부른다(D5). */
  maybeClose(session: Session): void {
    this.closeAndKeep(session);
    // 다른 대화의 예산으로 돈 마지막 턴(압축이 그 예산 시간을 붙들었을 수 있다)의 주인도 같은 판정을 받는다(F07).
    const ref = session.turns.at(-1)?.budget;
    if (ref && ref.sessionId !== session.id) this.closeIfQuiet(ref);
  }

  // ---- 위임 ----

  /** 위임 입구: AI 가 낸 위임(턴 있음)은 그 턴의 예산에서 작업 카드가 없으면 거절한다. 사람이 직접 맡긴 위임·다시 맡기기는 카드를 요구하지 않는다(D9). */
  cardGate(session: Session, turnId: string, human: boolean): string | undefined {
    if (human) return undefined;
    const ref = session.turns.find(turn => turn.id === turnId)?.budget;
    if (!ref || !this.budgets.current(ref) || this.budgets.isHalted(ref) || this.budgets.hasCard(ref)) return undefined;
    return cardMissing(this.host.languageOf(session));
  }

  /**
   * 작업자 실행이 실제로 시작되기 직전의 차감(D1). 사람이 맡긴 위임은 열린 예산에 붙고('direct'), AI 위임은 그 턴의 예산을 쓴다.
   * 예산이 없으면(옛 턴) 막지 않는다. 거절이면 refuse 에 거절 글이 있다(charge 가 이미 멈춤을 불렀다).
   */
  charge(session: Session, turnId: string, human: boolean, what: ChargeWhat): ChargeOutcome {
    const ref = human ? this.budgets.attach(session, 'direct') : session.turns.find(turn => turn.id === turnId)?.budget;
    if (!ref) return {};
    return this.chargeRef(session, ref, what);
  }

  /**
   * AI 앱 동작(app-actions.ts budgetGate)의 차감. 다른 대화로 보내는 지시는 위임 1건을 세고 멈춤을 본다. 작업 카드는 요구하지 않는다:
   * 받는 대화의 턴이 같은 예산에 붙어 첫 작업자 위임을 낼 때 cardGate 가 걸린다(knobs 5-1절). 자기 대화로 보내거나 예약 실행처럼 세지 않는 일은 멈춤만 본다.
   */
  chargeAction(session: Session, turn: Turn, what: { delegations?: number }): { refuse?: string; refuseI18n?: WireText } {
    const { refuse, refuseI18n } = this.charge(session, turn.id, false, what);
    return { refuse, refuseI18n };
  }

  /** 대상 넘김 재실행처럼 이미 있는 위임 기록의 예산에 차감한다. */
  chargeItem(session: Session, item: Delegation, what: ChargeWhat): string | undefined {
    const ref = delegationBudgetRef(session, item);
    return ref ? this.chargeRef(session, ref, what).refuse : undefined;
  }

  private chargeRef(session: Session, ref: BudgetRef, what: ChargeWhat): ChargeOutcome {
    const result = this.budgets.charge(ref, what);
    if (result.ok) return { ref };
    const budget = this.budgets.current(ref);
    // refuse 는 모델에게 가는 글이고, refuseI18n 은 사람이 직접 맡긴 위임·다시 맡기기의 오류로 쓴다.
    return budget ? { refuse: refusal(result.reason, budget, this.host.languageOf(session)), refuseI18n: refusalWire(result.reason, budget) } : { refuse: cardMissing(this.host.languageOf(session)) };
  }

  /**
   * 쓰기 위임용 연결 콜백(다음 작업자가 write-run.ts 의 후보 시작 직전에 부른다). 후보마다 {delegations:1, ...}을 차감하고 거절 글을 돌려준다.
   * 쓰기 위임은 startDelegation 입구에서 세지 않는다.
   */
  readonly chargeForWrite = (sessionId: string, turnId: string, human: boolean, what: ChargeWhat): ChargeOutcome => {
    const session = this.host.session(sessionId);
    return session ? this.charge(session, turnId, human, what) : {};
  };

  delegationStarted(sessionId: string, delegationId: string): void {
    const ref = this.refOfDelegation(sessionId, delegationId);
    if (ref) this.budgets.runStarted(ref, `del:${delegationId}`);
  }

  /** 위임 실행 기록을 지울 때(끝·중지): 실행 구간만 닫는다. 닫힘 판정은 결과를 대기에 넣고 깨운 뒤 delegationDone 이 한다. */
  delegationRunEnded(sessionId: string, delegationId: string): void {
    const ref = this.refOfDelegation(sessionId, delegationId);
    if (ref) this.budgets.runEnded(ref, `del:${delegationId}`);
  }

  /** 위임이 끝났고 결과를 대기에 넣어 깨운 뒤: 실패·시간 초과면 세고, 예산 주인 대화가 조용하면 닫는다. */
  delegationDone(sessionId: string, delegationId: string): void {
    const session = this.host.session(sessionId);
    const item = session?.delegations?.find(entry => entry.id === delegationId);
    if (!session || !item) return;
    const ref = delegationBudgetRef(session, item);
    if (!ref) return;
    if (item.status === 'failed' || item.status === 'timeout') this.budgets.noteFailure(ref);
    this.closeIfQuiet(ref);
  }

  private refOfDelegation(sessionId: string, delegationId: string): BudgetRef | undefined {
    const session = this.host.session(sessionId);
    const item = session?.delegations?.find(entry => entry.id === delegationId);
    return session && item ? delegationBudgetRef(session, item) : undefined;
  }

  // ---- 작업 카드 도구 ----

  cardTool(session: Session, turn: Turn): AppTool {
    const language = this.host.languageOf(session);
    const text = (entry: EngineTextEntry<string>) => engineText(entry, language);
    // 카드 상태는 그 요청을 실제로 돌리는 엔진이 Codex 일 때만 스키마에 넣고 필수로 한다. 다른 엔진의 스키마·설명은 그대로다.
    const codex = turn.execution?.engine === 'codex';
    return {
      name: 'work_card',
      description: text(CARD_TOOL_TEXT.description),
      inputSchema: { type: 'object', additionalProperties: false, required: codex ? ['deliverable', 'done', 'state'] : ['deliverable', 'done'], properties: {
        ...(codex ? { state: { type: 'string', enum: [...WORK_CARD_STATES], description: text(CARD_TOOL_TEXT.state) } } : {}),
        deliverable: { type: 'string', minLength: 1, maxLength: 300, description: text(CARD_TOOL_TEXT.deliverable) },
        done: { type: 'array', minItems: 1, maxItems: 5, items: { type: 'string', maxLength: 300 }, description: text(CARD_TOOL_TEXT.done) },
        targets: { type: 'object', additionalProperties: false, required: ['confirmed', 'candidate', 'unknown'], description: text(CARD_TOOL_TEXT.targets), properties: {
          confirmed: { type: 'integer', minimum: 0 }, candidate: { type: 'integer', minimum: 0 }, unknown: { type: 'integer', minimum: 0 }, note: { type: 'string', maxLength: 300 } } },
        verify: { type: 'array', maxItems: 5, description: text(CARD_TOOL_TEXT.verify), items: { type: 'object', additionalProperties: false, required: ['label', 'status'], properties: {
          label: { type: 'string', maxLength: 300 }, status: { type: 'string', enum: [...WORK_CARD_VERIFY_STATUSES] } } } },
        conflicts: { type: 'array', maxItems: 5, items: { type: 'string', maxLength: 300 }, description: text(CARD_TOOL_TEXT.conflicts) },
      } },
      call: async args => {
        if (codex && (args as { state?: unknown }).state === undefined) return { text: text(CARD_TOOL_TEXT.stateMissing), isError: true };
        const ref = turn.budget ?? (session.budget ? { sessionId: session.id, budgetId: session.budget.id } : { sessionId: session.id, budgetId: '' });
        const saved = this.budgets.setCard(ref, args as unknown as Parameters<RequestBudgets['setCard']>[1], language, { sessionId: session.id, turnId: turn.id });
        if (!saved.ok) return { text: saved.error, isError: true };
        const { used, limits } = saved.budget;
        const lim = (value: number) => limitText(saved.budget, value, language);
        const usage = engineText(CARD_TOOL_TEXT.usage, language)({ minutes: Math.floor(used.activeMs / 60000), limitMinutes: saved.budget.unlimited ? lim(limits.timeMinutes) : engineText(CARD_TOOL_TEXT.minutes, language)(limits.timeMinutes), delegations: used.delegations, limitDelegations: lim(limits.delegations),
          commands: used.commands, limitCommands: lim(limits.commands), reviews: used.reviews, limitReviews: lim(limits.reviews), checks: used.checks, limitChecks: lim(limits.checks) });
        return { text: engineText(CARD_TOOL_TEXT.saved, language)(usage) };
      },
    };
  }

  // ---- 요청 처리·기동·종료 ----

  handlers(snapshot: (sessionId: string) => Snapshot): AppRequestHandlers<'budget-decide'> {
    return {
      'budget-decide': request => {
        const result = this.budgets.decide(request.sessionId, request.budgetId, request.choice, request.size);
        if (!result.ok) throw new WireError(result.code === 'not-found' ? wire('budget.error.notFound') : wire('budget.error.notHalted'));
        return snapshot(request.sessionId);
      },
    };
  }

  /**
   * 기동 복구(D8): 사용량과 상태는 보존하고 열린 시간 구간만 버린다. 그 예산을 가리키는 대기 항목(pending·held)·결과 불명 위임·복구 예약이 하나도 없는
   * 열린 예산만 닫는다(다음 사용자 글이 옛 사용량에 붙지 않게). 멈춘 예산은 그대로 둔다.
   */
  bootRestore(sessions: Iterable<Session>): void {
    const all = [...sessions];
    for (const session of all) {
      this.budgets.bootRestore(session);
      if (session.budget?.state === 'open' && !this.referenced(all, session, session.budget.id)) this.budgets.maybeClose(session);
    }
  }

  /** 기동 때 이 예산을 이어 쓸 일이 남았는가: 복구 예약, 대기 항목(예산 참조가 없는 주인 대화의 항목 포함), 결과 불명 위임. */
  private referenced(sessions: Session[], owner: Session, budgetId: string): boolean {
    return sessions.some(session => {
      const mine = session.id === owner.id;
      return (session.recovery !== undefined && (mine || session.turns.at(-1)?.budget?.budgetId === budgetId))
        || (session.queue ?? []).some(item => (item.state === 'pending' || item.state === 'held') && (item.budget ? item.budget.budgetId === budgetId : mine))
        || (session.delegations ?? []).some(item => item.status === 'unknown' && item.budgetId === budgetId);
    });
  }

  forgetSession(sessionId: string): void {
    this.budgets.forgetSession(sessionId);
  }

  dispose(): void {
    this.budgets.dispose();
  }
}
