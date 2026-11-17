// 바깥 터미널 엔진 대화 이어 열기 요청 셋의 입력 검사(docs/notes/engine-import-2026-10-19.md). 스키마는 모양 판정(fits)에만 쓰고 반환 객체는 읽기 함수가 만든다.
// 경로는 출처 번호로 다시 찾으므로 입력에 출처 폴더 경로는 없다. folder 는 작업 폴더 후보라 모양만 보고, 실제 검사는 처리 함수가 한다.
import * as v from 'valibot';
import { absolutePathSchema, accountSchema, fits, type InputReaders } from './common';

type EngineImportInputKind = 'engine-import-sources' | 'engine-import-list' | 'engine-import-open';

const sourceSchema = v.pipe(v.string(), v.minLength(1), v.maxLength(80));
const threadSchema = v.pipe(v.string(), v.minLength(1), v.maxLength(200), v.regex(/^[\w.-]+$/));
const listSchema = v.object({ source: sourceSchema, folder: absolutePathSchema });
const openSchema = v.object({
  source: sourceSchema, folder: absolutePathSchema, thread: threadSchema, mode: v.picklist(['fork', 'same']),
  targetAccount: v.optional(accountSchema),
});

export const ENGINE_IMPORT_INPUTS = {
  'engine-import-sources': () => ({ kind: 'engine-import-sources' }),
  'engine-import-list': r => {
    if (!fits(listSchema, { source: r.source, folder: r.folder })) return undefined;
    return { kind: 'engine-import-list', source: r.source as string, folder: r.folder as string };
  },
  'engine-import-open': r => {
    if (!fits(openSchema, { source: r.source, folder: r.folder, thread: r.thread, mode: r.mode, targetAccount: r.targetAccount })) return undefined;
    return { kind: 'engine-import-open', source: r.source as string, folder: r.folder as string, thread: r.thread as string, mode: r.mode as 'fork' | 'same',
      ...(r.targetAccount !== undefined ? { targetAccount: r.targetAccount as string } : {}) };
  },
} satisfies InputReaders<EngineImportInputKind>;
