// 바깥 엔진 대화 번호가 Lair 대화에 묶였는지 판정하는 한 곳(docs/notes/engine-import-2026-10-19.md, tool-migration-2026-11-03.md 8절).
// 요청 처리(requests/engine-import.ts)와 쓰던 작업 훑기(migrate/work-scan.ts)가 같은 판정을 쓴다. 판정 사본을 만들지 않는다.
import type { Session } from '../../shared/contracts';

/** 대화 하나가 묶고 있는 바깥 대화 한 개 */
export interface BoundThread {
  engine: string;
  thread: string;
}

/**
 * 대화 하나가 묶고 있는 {engine, thread} 를 낸다. 길은 셋이다.
 * externalThread 의 원래 번호·확정한 갈래 번호, 엔진 터미널 대화의 cliSession(엔진이 없으면 codex), engineThreads 슬롯(`engine` 또는 `engine:...`, 엔진은 ':' 앞).
 */
export function* sessionThreads(session: Session): Generator<BoundThread> {
  const origin = session.externalThread;
  if (origin) {
    yield { engine: origin.engine, thread: origin.thread };
    for (const fork of origin.forks ?? []) yield { engine: origin.engine, thread: fork };
  }
  // 엔진 터미널 대화(Lair 안 터미널)도 같은 번호를 쓴다. Claude·Grok 은 앱이 정한 번호, Codex 는 훅이 알린 번호가 cliSession 에 있다.
  const cli = session.terminal?.cliSession;
  if (cli) yield { engine: session.engine ?? 'codex', thread: cli };
  for (const [slot, bound] of Object.entries(session.engineThreads ?? {})) {
    if (bound?.threadId) yield { engine: slot.split(':', 1)[0], thread: bound.threadId };
  }
}

/** 묶음 판정에서 뺀 대화인가(임시 대화) */
export function excludedFromBound(session: Session, isTemporary: (id: string) => boolean): boolean {
  return session.temporary === true || isTemporary(session.id);
}

/** 바깥 대화(`${engine}:${thread}`)를 묶고 있는 Lair 대화 번호 표. 임시 대화는 뺀다. 옮기기 대화 목록·복구가 한 번 만들어 쓴다 */
export function boundSessionIndex(sessions: Iterable<Session>, isTemporary: (id: string) => boolean): Map<string, string> {
  const out = new Map<string, string>();
  for (const session of sessions) {
    if (excludedFromBound(session, isTemporary)) continue;
    for (const bound of sessionThreads(session)) {
      const key = `${bound.engine}:${bound.thread}`;
      if (!out.has(key)) out.set(key, session.id);
    }
  }
  return out;
}

/** 모든 대화가 묶고 있는 `${engine}:${thread}` 집합. 임시 대화는 뺀다. 메모리의 대화 머리만 훑는다. */
export function boundThreadIndex(sessions: Iterable<Session>, isTemporary: (id: string) => boolean): Set<string> {
  const index = new Set<string>();
  for (const session of sessions) {
    if (excludedFromBound(session, isTemporary)) continue;
    for (const { engine, thread } of sessionThreads(session)) index.add(`${engine}:${thread}`);
  }
  return index;
}
