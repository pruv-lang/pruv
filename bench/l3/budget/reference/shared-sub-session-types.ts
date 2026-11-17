// 하위 대화 타입과 상한 값(docs/notes/sub-sessions-2026-10-19.md 4.2). 타입·정책 데이터만 둔다(판단 함수 없음).
import type { ApprovalMode, SentFromRoot } from './core-types';

/**
 * 하위 대화 상한(정의는 여기 한 곳, 백엔드 판정·화면 메뉴·앱 지시문이 읽는다). 깊이 상한은 없다(순환만 막는다).
 * running: 같은 부모의 하위가 동시에 도는 수의 기본값(설정 subSessionRunning 이 없을 때), 넘으면 요청이 기다린다.
 * runningMin·runningMax: 설정으로 고를 수 있는 동시 수의 범위(docs/notes/sub-sessions-2-2026-10-19.md 4.2).
 */
export const SUB_SESSION_LIMITS = { running: 3, runningMin: 1, runningMax: 8 } as const;

/** 하위 대화 쪽 칸. reportedTurnId 는 마지막으로 부모에 보고한 하위 요청 번호다(중복 막기·기동 보충 판정). */
export type SessionParent = { sessionId: string; createdAt: string; by: 'ai' | 'user'; reportedTurnId?: string; reportedAt?: string };

export type ChildResultKind = 'done' | 'failed' | 'stopped' | 'unknown';

/** 부모 쪽에 고정한 하위 결과. id 는 배달 배열에 위임 번호와 함께 들어간다. text 는 24,000자까지. */
export type ChildResult = {
  id: string;
  childSessionId: string;
  childTurnId: string;
  childTitle: string;
  kind: ChildResultKind;
  text: string;
  createdAt: string;
  approval: ApprovalMode;
  chain: { chainId: string; chainCount: number; root: SentFromRoot };
  deliveredTurnId?: string;
  deliveredAt?: string;
  /** 사용자가 대기에서 취소하거나 비워 보내지 않기로 한 시각. 있으면 미배달로 보지 않고 기동 복구도 다시 넣지 않는다. */
  droppedAt?: string;
};

/** 'session-children' 의 항목. state 는 도는 요청이 있으면 running, 대기 waiting 이 있으면 waiting, 보류 held 가 있으면 held, 마지막 요청이 실패면 failed, 아니면 idle. */
export type SubSessionChild = {
  sessionId: string;
  title: string;
  state: 'running' | 'waiting' | 'held' | 'idle' | 'failed';
  lastReportedAt?: string;
  /** 작업트리 하위면 그 작업트리 번호(AI 가 합칠 때 쓴다). */
  worktreeId?: string;
};

/** 화면용: 본문 대신 글자 수만 싣는다. */
export type ChildResultView = Omit<ChildResult, 'text'> & { chars: number };
