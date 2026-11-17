// 답 끝의 후속 제안·작업 결과 보고 블록의 구분자와 머리말 데이터(docs/notes/result-card-2026-09-24.md 3번).
// 엔진 지시는 src/logic/app-instructions.ts·src/shared/engine-text.ts, 떼어 내는 해석은 src/logic/reply-blocks.ts 가 이 정의를 함께 쓴다(B-225).
import type { AgentReport } from './contracts';

export const NEXT_DELIM = '<<<LAIR_NEXT>>>';
export const REPORT_DELIM = '<<<LAIR_REPORT>>>';

/** 기억 후보 블록(docs/notes/common-memory-2026-10-03.md M3). 후보 줄 읽기는 백엔드 memory-context.ts. */
export const MEMORY_DELIM = '<<<LAIR_MEMORY>>>';

/** 작업 결과 보고 머리말(지시문 언어별). 지시(engine-text.ts)·인계 글(handoff.ts)·하위 대화 결과 글이 이 표를 쓰고, 해석은 모든 언어의 머리말을 받는다(옛 답의 한국어 머리말도 계속 읽힘, B-224). */
export const REPORT_HEADINGS: Readonly<Record<'ko' | 'en', Readonly<Record<keyof AgentReport, string>>>> = {
  ko: { verified: '확인함', unverified: '확인 못 함', remaining: '남은 일' },
  en: { verified: 'Verified', unverified: 'Not verified', remaining: 'Remaining' },
};
