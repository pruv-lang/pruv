// 대화 하나에서 엔진 하나의 엔진 대화 타입(docs/notes/engine-import-2026-10-19.md). contracts.ts 에서 다시 내보낸다.
import type { Usage } from './delegation-types';
import type { EngineRewind } from './send-cancel-types';

/** 대화 하나에서 엔진 하나의 엔진 대화(같은 문서 2번). */
export interface EngineThread {
  /** 엔진의 대화 번호(Codex 스레드 또는 Claude 세션) */
  threadId?: string;
  /** 이 Codex 대화를 브라우저 도구(dynamicTools)를 넣어 시작했다. Codex 는 대화를 시작할 때만 앱 도구를 받는다(docs/notes/preview-browser-2026-10-02.md 조각 3). */
  browserTools?: true;
  /** 브라우저 후속-1 도구 묶음(스크립트·끌기·받은 파일, docs/notes/browser-follow1-2026-10-04.md 논쟁 11)을 넣어 시작한 Codex 대화면 2. 없으면 그 전 묶음이다. */
  browserToolsVersion?: 2;
  /** 이 엔진 대화가 받은 마지막 요청. 인계 글의 차이분은 이 요청 뒤부터 고른다. */
  lastTurnId?: string;
  /** 엔진이 마지막으로 알린 이 엔진 대화의 금액 누계. 다음 요청 몫의 기준이다(docs/notes/usage-2026-09-24.md 2번). */
  costTotalUsd?: number;
  /** 엔진이 마지막으로 알린 이 엔진 대화의 사용량 누계(하위 에이전트 포함). 다음 요청 사용량 몫의 기준이다(docs/notes/delegation-bundle2-2026-10-21.md E11). */
  usageTotal?: Usage;
  /** 이 엔진 대화에 마지막으로 보낸 생각 강도. 보내지 않았으면 null, 이 기능 전 기록은 칸이 없다(docs/notes/effort-approval-2026-10-01.md 1.3). */
  lastEffort?: string | null;
  /** 바깥 대화를 갈래로 이어 갈 첫 요청 대기. 엔진이 새 번호를 알리면 지운다(docs/notes/engine-import-2026-10-19.md). */
  fork?: true;
  /** 다음 실행 전에 엔진 대화를 되감을 지점. 엔진이 되감았다고 알리면 지운다(docs/notes/settings-topics-send-cancel-2026-11-02.md B3). */
  rewind?: EngineRewind;
  /** 이 엔진 대화에 정상 전달된 앱 블록 글의 지문(B-655). 같으면 다음 요청에 다시 싣지 않는다. */
  sentContext?: { memory?: string; rules?: string; output?: string };
}
