import type { EngineId } from '../shared/contracts';

/** 복사했거나 터미널에서 가져온 읽기 전용 요청인가(같은 문서 D3 [논쟁 11], 가져온 요청은 docs/notes/engine-import-2026-10-19.md) */
export function copiedTurn(turn: { copiedFrom?: unknown; imported?: unknown }): boolean { return Boolean(turn.copiedFrom || turn.imported); }

/** 대화의 엔진 대화 결속 열쇠(docs/notes/accounts-2026-09-26.md 9번). 기본 계정은 엔진 번호 그대로라 옛 기록이 그대로 읽힌다. */
export function threadSlot(engine: EngineId, account?: string): string {
  return account ? `${engine}:${account}` : engine;
}

/**
 * 엔진 대화를 갈래로 요약할 수 있는 엔진인지(docs/notes/browser-follow3-2026-10-04.md 논쟁 반영). capabilities.summary 는 요약 자체를 만드는지이고,
 * 갈래 요약은 지금 Claude 만 된다. 아니면 앱 기록으로 요약한다. 실행부의 요약 고르기와 화면 확인창 요약 줄이 같이 쓴다.
 */
export function summarizesThread(engine: EngineId): boolean {
  return engine === 'claude';
}
