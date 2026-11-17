/**
 * 화면 문구 사전의 핵심(다국어-1, docs/notes/i18n-base-2026-10-04.md I1). DOM·Node 모듈을 import 하지 않아 화면·주 프로세스·웹·서버 어느 묶음에서도 쓴다.
 * 엔진에 보내는 글은 여기가 아니라 src/shared/engine-text.ts 다.
 *
 * 쓰는 법:
 *   const ko = defineKo({ 'common.cancel': '취소', 'common.count': (p: { n: number }) => `${p.n}개`, 'shell.hello': '{name} 님' });
 *   const en = defineEn(ko, { 'common.cancel': 'Cancel', 'common.count': (p: { n: number }) => `${p.n} ${p.n === 1 ? 'item' : 'items'}`, 'shell.hello': 'Hi {name}' });
 *   export const t = makeText(ko, en);   t('common.cancel'), t('common.count', { n: 3 }), t('shell.hello', { name: 'A' })
 * 영어 사전은 한국어 사전과 키 집합·함수 인자·문자열 {자리} 이름이 같아야 한다(어기면 tsc 오류). 번역 메모는 항목 바로 위 주석으로 단다.
 */

export type UiLanguage = 'ko' | 'en';
/** 설정 값. 'system' 은 운영체제(웹은 브라우저) 언어 목록에서 정한다. */
export type UiLanguageSetting = 'system' | UiLanguage;
export const UI_LANGUAGES: readonly UiLanguage[] = ['ko', 'en'];
export const UI_LANGUAGE_SETTINGS: readonly UiLanguageSetting[] = ['system', 'ko', 'en'];
/** 선택지 이름은 그 언어 자신으로 쓴다(번역하지 않음). */
export const UI_LANGUAGE_NATIVE_NAMES: Readonly<Record<UiLanguage, string>> = { ko: '한국어', en: 'English' };

export function isUiLanguageSetting(value: unknown): value is UiLanguageSetting {
  return typeof value === 'string' && (UI_LANGUAGE_SETTINGS as readonly string[]).includes(value);
}

/** 선호 순서 언어 목록(app.getPreferredSystemLanguages, navigator.languages)을 앞에서부터 보고 처음 맞는 ko·en. 없으면 en. */
export function languageFromPreferred(preferred: readonly string[] | undefined): UiLanguage {
  for (const raw of preferred ?? []) {
    const lower = String(raw).trim().toLowerCase();
    for (const code of UI_LANGUAGES) if (lower === code || lower.startsWith(`${code}-`) || lower.startsWith(`${code}_`)) return code;
  }
  return 'en';
}

export function resolveUiLanguage(setting: UiLanguageSetting | undefined, preferred: readonly string[] | undefined): UiLanguage {
  return setting === 'ko' || setting === 'en' ? setting : languageFromPreferred(preferred);
}

// 주 묶음(main.cjs)과 연결 묶음(remote-guest.cjs)이 같은 프로세스에서 이 모듈을 따로 싣는다(secure-storage.ts B-444 와 같은 방식).
// 언어 상태를 globalThis 한 칸에 두어 같은 프로세스의 모든 묶음이 함께 본다. 렌더러·웹은 프로세스가 따로라 서로 영향이 없다.
const LANGUAGE_KEY = Symbol.for('lair.uiLanguage');
const languageSlot = globalThis as unknown as Record<symbol, UiLanguage | undefined>;

/** 진입점이 화면 초기화 전에 한 번 부른다. 상태는 같은 프로세스의 모든 묶음이 공유하므로 어느 묶음에서 불러도 된다. */
export function setUiLanguage(language: UiLanguage): void { languageSlot[LANGUAGE_KEY] = language; }

/**
 * 지금 화면 언어. 진입점이 정하지 않았으면 데스크톱 preload·웹 다리가 창 전역에 둔 값(window.lair.uiLanguage)을 읽고, 그것도 없으면 'ko'
 * (아직 옮기지 않은 서버·도구 글과 같은 언어). 모듈 초기화 때 번역 글을 상수로 만들지 말고 쓸 때 부른다.
 */
export function uiLanguage(): UiLanguage {
  const current = languageSlot[LANGUAGE_KEY];
  if (current) return current;
  const bridged = (globalThis as { lair?: { uiLanguage?: unknown } }).lair?.uiLanguage;
  return bridged === 'en' || bridged === 'ko' ? bridged : 'ko';
}

/** Intl·toLocale* 에 넘길 로케일. */
export function uiLocale(language: UiLanguage = uiLanguage()): 'ko-KR' | 'en-US' {
  return language === 'en' ? 'en-US' : 'ko-KR';
}

// ---- 사전 타입 ----

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type MessageFunction = (params: any) => string;
export type Message = string | MessageFunction;
export type Messages = Record<string, Message>;

/** 문자열 값 안의 {이름} 자리들. */
export type Holes<S extends string> = S extends `${string}{${infer Name}}${infer Rest}` ? Name | Holes<Rest> : never;

type Same<A, B> = [A] extends [B] ? ([B] extends [A] ? true : false) : false;

/** 한국어 값 하나에 맞는 영어 값의 모양: 함수면 같은 인자 튜플, 문자열이면 같은 {자리} 집합. */
type MatchEntry<K, E> =
  K extends (...args: infer KA) => string
    ? E extends (...args: infer EA) => string ? (Same<KA, EA> extends true ? E : never) : never
    : K extends string
      ? E extends string ? (Same<Holes<K>, Holes<E>> extends true ? E : never) : never
      : never;

/** defineEn 이 받는 영어 사전의 모양: 키 집합이 같고(남는 키는 never) 항목마다 MatchEntry. */
export type Translation<K extends Messages, E> =
  { [Key in keyof K]: Key extends keyof E ? MatchEntry<K[Key], E[Key]> : K[Key] }
  & { [Key in Exclude<keyof E, keyof K>]: never };

/** 한국어(원본) 사전. 문자열 값의 리터럴 타입을 보존해 {자리} 이름을 검사할 수 있게 한다. */
export function defineKo<const K extends Messages>(messages: K): K { return messages; }

/** 영어 사전. 한국어 사전과 키·인자·{자리}가 다르면 tsc 오류. */
export function defineEn<K extends Messages, const E extends Messages>(_ko: K, messages: E & Translation<K, E>): E { return messages; }

/** 호출 인자: 함수 값이면 그 인자, {자리}가 있는 문자열이면 그 이름들의 객체, 없으면 인자 없음. */
export type ArgsOf<V> =
  V extends (params: infer P) => string ? (V extends () => string ? [] : [P])
    : V extends string ? ([Holes<V>] extends [never] ? [] : [Record<Holes<V>, string | number>])
      : never;

function fill(template: string, params: Record<string, unknown> | undefined): string {
  if (!params) return template;
  return template.replace(/\{([^{}]+)\}/g, (whole, name: string) => (name in params ? String(params[name]) : whole));
}

export interface TextFunction<K extends Messages> {
  <Key extends keyof K & string>(key: Key, ...args: ArgsOf<K[Key]>): string;
  /** HTML data-i18n 처럼 키를 실행 중에 받는 곳. 인자 없는 문자열 키만 돌려주고, 없거나 인자가 필요한 키면 undefined. */
  plain(key: string): string | undefined;
  has(key: string): boolean;
  keys(): readonly string[];
  /** 지금 화면 언어와 상관없이 정한 언어로 그린다(다국어-6 전송 글의 한국어 문장 칸, docs/notes/i18n6-wire-text-2026-10-05.md E2). */
  in<Key extends keyof K & string>(language: UiLanguage, key: Key, ...args: ArgsOf<K[Key]>): string;
}

/** 영역 사전 한 쌍으로 t() 를 만든다. 지금 언어는 부를 때마다 읽는다. */
export function makeText<K extends Messages, E extends Messages>(ko: K, en: E): TextFunction<K> {
  const pick = (key: string, language: UiLanguage = uiLanguage()): Message | undefined => {
    const table = (language === 'en' ? en : ko) as Messages;
    return Object.prototype.hasOwnProperty.call(table, key) ? table[key] : (ko as Messages)[key];
  };
  const render = (language: UiLanguage | undefined, key: string, params?: Record<string, unknown>) => {
    const value = pick(key, language);
    if (value === undefined) return key;
    return typeof value === 'function' ? value(params) : fill(value, params);
  };
  const t = ((key: string, params?: Record<string, unknown>) => render(undefined, key, params)) as unknown as TextFunction<K>;
  t.in = ((language: UiLanguage, key: string, params?: Record<string, unknown>) => render(language, key, params)) as TextFunction<K>['in'];
  t.plain = key => {
    const value = pick(key);
    return typeof value === 'string' && !/\{[^{}]+\}/.test(value) ? value : undefined;
  };
  t.has = key => Object.prototype.hasOwnProperty.call(ko, key);
  t.keys = () => Object.keys(ko);
  return t;
}

/** 조사를 고를 때 보는 받침. Codex(코덱스)·Claude(클로드)·운영 Lair는 받침이 없고 Grok(그록)은 있다. */
const FINAL_CONSONANT = new Set(['Grok']);

/** 엔진 이름 뒤에 맞는 조사를 붙인다. withJosa('Grok', '이/가')는 "Grok이", withJosa('Codex', '이/가')는 "Codex가"다. */
export function withJosa(label: string, pair: '은/는' | '이/가' | '을/를' | '과/와' | '으로/로'): string {
  const [consonant, vowel] = pair.split('/');
  return label + (FINAL_CONSONANT.has(label) ? consonant : vowel);
}

/** 마지막 글자의 받침에 맞는 조사를 붙인다. 한글이 아니면 받침 없는 쪽이다. withBatchim('브랜치 작업', '이/가')는 "브랜치 작업이"다. */
export function withBatchim(text: string, pair: '은/는' | '이/가' | '을/를' | '과/와' | '으로/로'): string {
  const [consonant, vowel] = pair.split('/');
  const code = text.trim().charCodeAt(text.trim().length - 1);
  const batchim = code >= 0xac00 && code <= 0xd7a3 ? (code - 0xac00) % 28 : 0;
  // '으로/로'는 ㄹ 받침(8)도 '로'다("서울로").
  return text + (batchim !== 0 && !(pair === '으로/로' && batchim === 8) ? consonant : vowel);
}
