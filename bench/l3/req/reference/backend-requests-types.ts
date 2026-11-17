// 도메인 처리 모듈 둘이 같은 모양을 쓰기 위한 타입(docs/notes/refactor-phase3-notes-boards-2026-10-15.md). 서비스·실행부를 import 하지 않는다.
import type { AppRequest, AppRequestResult, SentFrom } from '../../shared/contracts';

/** 실행부 요청 칸을 읽기만 하는 모양. runtime.ts 의 envelope.request 가 그대로 들어간다. */
export type DomainRequest = Readonly<Record<string, unknown>>;

/** kind → 처리 함수 표. 동기 결과는 그대로, 비동기 처리는 Promise 를 돌려준다. */
export type DomainHandlers<K extends string> = Record<K, (request: DomainRequest) => unknown>;

/**
 * 검증을 마친 앱 요청(handleRequestCore 의 공통 검사 뒤)용 처리 표. 실행부 직접 요청 DomainHandlers 와 입구가 다르다.
 * 세 번째 인자 tempVisible 은 관문이 싣는 임시 보기 가능 연결 권한(handleRequestCore 의 tempVisible)이다.
 * 네 번째 인자 caller 는 봉투의 보낸 곳(sentFrom)이다. 앱 동작 다리가 예약 세 요청(schedule-save·schedule-finish·schedule-action)에만 호출한 대화·요청을 via 'ai' 로 실어 보내고,
 * 요청 본문과 모델 인자의 같은 이름 칸은 관문이 지운다. 다리를 거치지 않은 화면·원격 요청에는 이 세 요청에서 via 'ai' 가 없다(docs/notes/mid-features-3-2026-10-17.md N4·N6).
 */
export type AppRequestHandlers<K extends AppRequest['kind']> = { readonly [P in K]: (request: Extract<AppRequest, { kind: P }>, remote: boolean, tempVisible: boolean, caller?: SentFrom) => AppRequestResult | Promise<AppRequestResult> };

/** 표에 있는 kind 인가. 참이면 요청을 그 kind 들로 좁혀 handleRequestCore 뒤쪽 분기에서 빠지게 한다(옮기기 전 if 연쇄가 하던 좁히기). */
export const inAppRequests = <K extends AppRequest['kind']>(table: AppRequestHandlers<K>, request: AppRequest): request is Extract<AppRequest, { kind: K }> => Object.hasOwn(table, request.kind);
