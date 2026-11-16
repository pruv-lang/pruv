// Codex 추가 계정 폴더와 실행 환경(docs/notes/codex-accounts-2026-09-29.md 2-3·2-4·A02a).
// 추가 계정은 engine-homes/codex/<번호>를 CODEX_HOME으로 쓰고 HOME은 그대로 둔다.
// 기본 계정(~/.codex)에서는 로그인을 시작하지 않는다. 저장 방식·로그인 방법·공급자 고정도 기본 계정에는 붙이지 않는다.
import { realpathSync, statSync } from 'node:fs';
import { chmod, lstat, mkdir, readFile, rm } from 'node:fs/promises';
import { homedir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { accountHome } from './accounts';
import type { CodexOwnedCloseResult, CodexOwnedRun, CodexTreeGone } from './codex-owned-runner';
import type { AccountRef } from './engine-contract';
import { withEngineFolders } from './engine-path';
import { wire, WireError, wireOf, type WireText } from '../shared/i18n/wire';
import { codexProviderPrepare } from './provider-accounts';
import { wireReason } from './wire-reason';

/** 추가 계정 실행 환경에서 빼는 변수. 호스트 CODEX_HOME도 빼 계정 폴더만 남긴다(2-3). */
const STRIPPED = ['CODEX_API_KEY', 'CODEX_ACCESS_TOKEN', 'OPENAI_FEDERATION_RULE_ID', 'OPENAI_IDENTITY_TOKEN_FILE',
  'OPENAI_WORKLOAD_IDENTITY_CONTEXT', 'OPENAI_API_KEY', 'CODEX_HOME'];
const STRIPPED_KEYS = new Set(STRIPPED.map(key => key.toLowerCase()));

/**
 * 추가 계정으로 띄우는 모든 Codex 프로세스에 같은 순서로 붙인다(2-3·2-4).
 * 파일 저장, ChatGPT 로그인, OpenAI 공급자. 기본 계정에는 이 인자를 붙이지 않는다.
 */
export const CODEX_ACCOUNT_ARGS = [
  '-c', 'cli_auth_credentials_store="file"',
  '-c', 'forced_login_method="chatgpt"',
  '-c', 'model_provider="openai"',
];

const AUTH_NAME = 'auth.json';
/** 자격 파일로 인정하는 크기. 비어 있거나 1MB를 넘으면 증거가 아니다(2-4). */
const AUTH_MIN = 1;
const AUTH_MAX = 1024 * 1024;

export const CODEX_CLEANUP_UNKNOWN_MESSAGE = 'Codex 계정 프로세스 정리를 확인하지 못했어. 앱을 다시 시작한 뒤 다시 해 줘.';

export function codexAuthPath(home: string): string { return join(home, AUTH_NAME); }

/** 이 컴퓨터의 Codex 홈. 호스트 CODEX_HOME이 있으면 그 경로, 없으면 ~/.codex. 여기서는 로그인을 시작하지 않는다. */
export function hostCodexHome(): string {
  return process.env.CODEX_HOME?.trim() || join(homedir(), '.codex');
}

/**
 * 있는 경로는 realpath, 없으면 존재하는 조상의 realpath에 나머지를 붙인다.
 * realpath가 거절하면(권한·루프) 그 지점의 장치·아이노드로 구분한다.
 */
function canonicalPath(path: string): string {
  const missing: string[] = [];
  let current = resolve(path);
  for (;;) {
    try {
      const real = realpathSync(current);
      return missing.length === 0 ? real : join(real, ...missing);
    } catch (error) {
      const code = (error as NodeJS.ErrnoException).code;
      if (code !== 'ENOENT') {
        try {
          const info = statSync(current);
          const id = `dev:${String(info.dev)}:ino:${String(info.ino)}`;
          return missing.length === 0 ? id : `${id}/${missing.join('/')}`;
        } catch { /* 부모로 올라간다. */ }
      }
      const parent = dirname(current);
      if (parent === current) return resolve(path);
      missing.unshift(basename(current));
      current = parent;
    }
  }
}

/** 둘 다 있을 때만 장치·아이노드가 같은지 본다. 링크는 따라간다. */
function sameFile(left: string, right: string): boolean {
  try {
    const a = statSync(left);
    const b = statSync(right);
    return a.dev === b.dev && a.ino === b.ino;
  } catch { return false; }
}

/** 기본 계정 홈과 같으면 추가 계정 실행·로그인·삭제를 거절한다(설계 1절 운영 함정). 문자열 비교가 아니라 실제 경로로 본다. */
export function isHostCodexHome(home: string): boolean {
  const target = canonicalPath(home);
  const host = canonicalPath(hostCodexHome());
  const dot = canonicalPath(join(homedir(), '.codex'));
  if (target === host || target === dot) return true;
  return sameFile(home, hostCodexHome()) || sameFile(home, join(homedir(), '.codex'));
}

/** 번호로 다시 계산한 경로와 같을 때만 쓴다(2-3). 기본 계정 홈이면 거절한다. */
export function codexAccountFolder(dataDir: string, account: AccountRef): string {
  const expected = accountHome(dataDir, 'codex', account.id);
  if (account.home !== expected) throw new Error('Codex 계정 폴더를 확인하지 못해 쓰지 않았어.');
  if (isHostCodexHome(expected)) throw new WireError(wire('wireEngine.account.folder.codexHostExecution'), '기본 Codex 계정 폴더에서는 추가 계정으로 실행하지 않아.');
  return expected;
}

async function ensureFolder(path: string): Promise<void> {
  try { await mkdir(path, { mode: 0o700 }); }
  catch (error) { if ((error as NodeJS.ErrnoException).code !== 'EEXIST') throw error; }
  const info = await lstat(path);
  if (info.isSymbolicLink() || !info.isDirectory()) throw new WireError(wire('wireEngine.account.folder.codexNotFolder', { path }), `Codex 계정 폴더 자리에 폴더가 아닌 것이 있어: ${path}`);
  if (typeof process.getuid === 'function' && info.uid !== process.getuid()) throw new WireError(wire('wireEngine.account.folder.codexWrongOwner', { path }), `Codex 계정 폴더의 주인이 이 사용자가 아니야: ${path}`);
}

/**
 * 추가 계정 실행 환경. 빼는 변수 여섯과 호스트 CODEX_HOME을 빼고 계정 폴더를 넣는다. HOME은 복사한 값을 그대로 둔다.
 * 이 앱을 띄운 세션 표식만 추가로 뺀다(기본 계정 childEnvironment와 같은 범위).
 */
export function codexAccountEnvironment(home: string, base?: NodeJS.ProcessEnv): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = { ...(base ?? process.env) };
  for (const key of Object.keys(env)) {
    if (/^LAIR_(SESSION_ID|PARENT_SESSION_ID|DELEG_.*)$/.test(key) || /^CODEX_(THREAD_ID|SESSION_ID)$/.test(key)) delete env[key];
    if (STRIPPED_KEYS.has(key.toLowerCase())) delete env[key];
  }
  env.CODEX_HOME = home;
  return withEngineFolders(env);
}

/** 계정 폴더를 0700으로 준비하고 실행 환경과 공통 인자를 돌려준다. 기본 계정 홈이면 거절한다. */
export async function prepareCodexAccount(dataDir: string, account: AccountRef, baseEnv?: NodeJS.ProcessEnv): Promise<{ home: string; env: NodeJS.ProcessEnv; args: string[]; apiKey?: string }> {
  assertCodexAccountSpawnAllowed(account.id);
  const base = join(dataDir, 'engine-homes');
  await ensureFolder(base);
  await ensureFolder(join(base, 'codex'));
  const home = codexAccountFolder(dataDir, account);
  await ensureFolder(home);
  if (process.platform !== 'win32') await chmod(home, 0o700);
  // 연결 계정(docs/notes/provider-accounts-2026-10-03.md D3): 공통 인자 대신 연결 인자·환경을 쓴다. apiKey 는 앱 서버를 띄운 직후 메모리 로그인에만 쓴다.
  if (account.provider) {
    const linked = codexProviderPrepare(account.provider);
    return { home, env: { ...codexAccountEnvironment(home, baseEnv), ...linked.env }, args: linked.args, ...(linked.apiKey ? { apiKey: linked.apiKey } : {}) };
  }
  return { home, env: codexAccountEnvironment(home, baseEnv), args: [...CODEX_ACCOUNT_ARGS] };
}

/**
 * 상태 확인·이미 로그인됨에 쓰는 자격 파일 조건(2-4). 일반 파일이고 0600이며 1바이트~1MB일 때만 맞다.
 * 기호 링크·다른 권한·빈 파일은 맞지 않다. 로그인 성공의 "이번 시도 뒤에 쓰임"은 따로 본다.
 */
export async function codexAuthFileReady(home: string): Promise<boolean> {
  try {
    const info = await lstat(codexAuthPath(home));
    if (info.isSymbolicLink() || !info.isFile()) return false;
    if (info.size < AUTH_MIN || info.size > AUTH_MAX) return false;
    if (process.platform !== 'win32' && (info.mode & 0o777) !== 0o600) return false;
    return true;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return false;
    throw error;
  }
}

/**
 * 로그인 성공 증거. 준비 조건에 더해 이번 시도 시작 시각 이후에 쓰인 파일만 인정한다(2-4).
 * 시각이 초 단위인 파일 시스템에서도 방금 쓴 파일을 살리려고 1.5초만 허용한다.
 */
function isJwtThreeParts(value: unknown): boolean {
  if (typeof value !== 'string') return false;
  const parts = value.split('.');
  return parts.length === 3 && parts.every(part => part.length > 0);
}

function authJsonRoot(value: unknown): Record<string, unknown> | undefined {
  return value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : undefined;
}

/** auth.json 구조만 본다. 없거나 읽기 실패·구조 멀쩡하면 손상 아님(docs/notes/codex-accounts-2026-09-29.md 3절 10번 1·3). */
export async function codexAuthFileDamaged(home: string): Promise<boolean> {
  let info;
  try { info = await lstat(codexAuthPath(home)); }
  catch { return false; }
  if (info.isSymbolicLink() || !info.isFile()) return false;
  let raw: string;
  try { raw = await readFile(codexAuthPath(home), 'utf8'); }
  catch { return false; }
  let parsed: unknown;
  try { parsed = JSON.parse(raw); }
  catch { return true; }
  const root = authJsonRoot(parsed);
  if (!root) return true;
  const tokens = authJsonRoot(root.tokens);
  if (!tokens) return true;
  if (typeof tokens.refresh_token !== 'string') return true;
  if (!isJwtThreeParts(tokens.id_token)) return true;
  return false;
}

export async function codexAuthFileFresh(home: string, startedAtMs: number): Promise<boolean> {
  if (!await codexAuthFileReady(home)) return false;
  try {
    const info = await lstat(codexAuthPath(home));
    return info.mtimeMs + 1500 >= startedAtMs;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return false;
    throw error;
  }
}

/** 관리 정책이 자격 저장을 파일 밖으로 고정했을 때(2-4). */
export const CODEX_MANAGED_STORE_MESSAGE = '이 컴퓨터는 관리 정책으로 Codex 자격 저장 방식이 정해져 있어 추가 계정을 쓸 수 없어.';
/** 관리 요구가 ChatGPT 로그인을 막을 때(2-3). */
export const CODEX_UNSUPPORTED_LOGIN_MESSAGE = '이 컴퓨터는 관리 정책으로 ChatGPT 로그인을 막아 추가 계정을 지원하지 않아.';
/** 로그인은 됐는데 계정 폴더에 자격 파일이 없을 때(2-4). */
export const CODEX_UNSUPPORTED_FILE_MESSAGE = '지원하지 않는 저장 방식이야. 계정 폴더에 자격 파일이 없어.';

/** 유효 설정의 저장 방식. 관리 요구가 없을 때의 참고다. 막는지의 판정은 관리 요구(codexManagedStoreBlocked)로 한다. */
export function codexStoreBlocked(store: string | null | undefined): boolean {
  return store === 'keyring' || store === 'ephemeral';
}

/**
 * 관리 요구가 저장 방식을 명시했을 때. 요구가 없거나 file이면 파일 조건으로 판정한다.
 * keyring·ephemeral·auto와, file이 아닌 다른 명시 값은 파일 저장임을 증명하지 못한다.
 */
export function codexManagedStoreBlocked(store: 'file' | 'keyring' | 'auto' | 'ephemeral' | null | undefined): boolean {
  return store !== undefined && store !== 'file';
}

interface AccountWork {
  token: number;
  accountId: string;
  purpose: string;
  run: CodexOwnedRun;
  childDone: boolean;
  treeGone: CodexTreeGone | false;
  cleanupError?: string;
  cleanup: Promise<CodexOwnedCloseResult>;
}

/** run.done과 같은 기준. 정착과 정리 성공은 다르다(A02b1). */
export function codexOwnedCloseSuccess(result: CodexOwnedCloseResult): boolean {
  return result.childDone === true && result.treeGone === true && !result.cleanupError;
}

/** 프로세스 없이 끝난 로그인(already)용 합성 정리 성공(A02b2). */
export const codexNoProcessCleanup: Promise<CodexOwnedCloseResult> = Promise.resolve({ childDone: true, treeGone: true });

/** spawn·파일 변경 직전. shutdown·정리 불명이면 거절한다(A02b1). */
export function assertCodexAccountSpawnAllowed(accountId: string): void {
  if (codexAccountProcessesStopping()) throw new WireError(wire('wireEngine.account.processesStopped'), 'Codex 계정 작업을 멈췄어.');
  const blocked = codexAccountCleanupBlocked(accountId);
  if (blocked) throw new WireError(codexAccountCleanupBlockedWire(accountId) ?? wire('wireEngine.account.cleanupUnknown'), blocked);
}

/** 정리 결과의 전송 글. 앱 고정 사유는 생산 자리가 실은 글, 외부 원문은 그대로 사유 자리에, 사유가 없으면 기본 문장 글. */
export function codexCleanupResultWire(result: CodexOwnedCloseResult): WireText {
  if (result.notStarted) {
    if (result.cleanupI18n) return result.cleanupI18n;
    const reason = (result.cleanupError ?? '').slice(0, 500);
    const args: Record<string, string> = { reason, reasonEn: reason };
    return wire('wireEngine.account.startFailed', args as { reason: string });
  }
  if (result.cleanupI18n) return cleanupFailedWire([wireReason({ i18n: result.cleanupI18n })]);
  return result.cleanupError ? cleanupFailedWire([{ reason: result.cleanupError, reasonEn: result.cleanupError }]) : wire('wireEngine.account.cleanupUnknown');
}

let nextWorkToken = 1;
const accountWorks = new Map<number, AccountWork>();
/** 작업 토큰별 정리 실패. 등록부에서 지워도 유지한다. */
const tokenCleanupFailures = new Map<number, { accountId: string; message: string; reason?: string; i18n?: WireText }>();
let accountProcessesStopping = false;
let shutdownCleanup: Promise<void> | undefined;

/** 앱 종료가 상한을 넘겨 자식을 강제 종료한 뒤. 새 로그인·확인 프로세스를 띄우지 않는다. 그 종료 시도가 보류되면 풀린다. */
export function codexAccountProcessesStopping(): boolean { return accountProcessesStopping; }

/** closing=true 직후 취소 콜백보다 먼저 동기적으로 켠다(A02b2). */
export function beginCodexAccountProcessesStopping(): void { accountProcessesStopping = true; }

/** 종료 시도가 보류돼 closing을 되돌릴 때 차단을 푼다. 차단의 수명은 그 시도까지다. */
export function releaseCodexAccountProcesses(): void {
  accountProcessesStopping = false;
}

/** 정리 불명으로 막힌 계정이면 남은 실패를 모두 담은 오류 문구를 돌려준다. */
export function codexAccountCleanupBlocked(accountId: string): string | undefined {
  const messages = [...tokenCleanupFailures.values()]
    .filter(entry => entry.accountId === accountId)
    .map(entry => entry.message);
  if (messages.length === 0) return undefined;
  return messages.join(' ');
}

/**
 * 정리 막힘 글의 전송 글(문장은 codexAccountCleanupBlocked 가 그대로 돌려준다). 문장 비교 없이 만든 자리에서 정한다:
 * 사유가 없어 기본 문장을 쓴 실패는 앱 고정 글 키, 사유가 있던 실패는 그 원문을 reason 으로 싣는다. name 이 있으면 `이름 · ` 머리가 붙는 키다.
 */
export function codexAccountCleanupBlockedWire(accountId: string, name?: string): WireText | undefined {
  const entries = [...tokenCleanupFailures.values()].filter(entry => entry.accountId === accountId);
  if (entries.length === 0) return undefined;
  if (entries.every(entry => entry.reason === undefined)) return name === undefined ? wire('wireEngine.account.cleanupUnknown') : wire('wireEngine.account.cleanupUnknownNamed', { name });
  // 실패마다 두 언어 사유를 각각 만든다: 앱 고정 사유는 그 전송 글을 ko/en 으로, 외부 원문은 두 언어에 그대로, 기본 문장 실패는 cleanupUnknown 글로.
  const parts = entries.map(entry => {
    if (entry.i18n) return wireReason({ i18n: entry.i18n });
    if (entry.reason !== undefined) return { reason: entry.reason, reasonEn: entry.reason };
    return wireReason({ i18n: wire('wireEngine.account.cleanupUnknown') });
  });
  return cleanupFailedWire(parts, name);
}

/** 두 언어 사유 목록을 공통 래퍼 키 하나에 싣는다. 이름은 이 래퍼의 name 인자로만 붙는다. 길이 제한은 언어별로 건다. */
function cleanupFailedWire(parts: { reason: string; reasonEn: string }[], name?: string): WireText {
  const reason = parts.map(part => part.reason).join(' ').slice(0, 500);
  const reasonEn = parts.map(part => part.reasonEn).join(' ').slice(0, 500);
  // en 사전 값은 ko 값 모양({reason})으로 단언돼 있어 인자 타입에 reasonEn 이 없다. 인자를 변수로 두고 한 번만 맞춘다.
  const args: Record<string, string> = { reason, reasonEn };
  return name === undefined ? wire('wireEngine.account.cleanupFailed', args as { reason: string }) : wire('wireEngine.account.cleanupFailedNamed', { ...args, name } as { reason: string; name: string });
}

/** reason 은 실패에 사유 원문이 있었을 때만 둔다(없으면 앱 고정 기본 문장). */
function noteCleanupFailure(token: number, accountId: string, error?: string, reason?: string, i18n?: WireText): void {
  tokenCleanupFailures.set(token, {
    accountId,
    message: error?.trim() ? error : CODEX_CLEANUP_UNKNOWN_MESSAGE,
    ...(reason?.trim() ? { reason } : {}),
    ...(reason?.trim() && i18n ? { i18n } : {}),
  });
}

function settleWork(entry: AccountWork, childDone: boolean, treeGone: CodexTreeGone, cleanupError?: string, cleanupI18n?: WireText): void {
  entry.childDone = childDone;
  entry.treeGone = treeGone;
  if (!childDone || cleanupError || treeGone !== true) {
    entry.cleanupError = cleanupError ?? CODEX_CLEANUP_UNKNOWN_MESSAGE;
    noteCleanupFailure(entry.token, entry.accountId, entry.cleanupError, cleanupError, cleanupI18n);
  }
}

/**
 * 추가 계정 owned runner를 등록한다. cleanup은 run.done의 CodexOwnedCloseResult로 정착한다(정착≠성공).
 * shutdown은 같은 cleanup Promise를 기다린다.
 */
export function trackCodexAccountWork(accountId: string, purpose: string, run: CodexOwnedRun): { token: number; cleanup: Promise<CodexOwnedCloseResult> } {
  const token = nextWorkToken++;
  let cleanupResolve!: (result: CodexOwnedCloseResult) => void;
  const cleanup = new Promise<CodexOwnedCloseResult>(resolve => { cleanupResolve = resolve; });
  const entry: AccountWork = { token, accountId, purpose, run, childDone: false, treeGone: false, cleanup };
  accountWorks.set(token, entry);
  void run.done.then(result => {
    settleWork(entry, result.childDone, result.treeGone, result.notStarted ? undefined : result.cleanupError, result.cleanupI18n);
    cleanupResolve(result);
    accountWorks.delete(token);
  }).catch(error => {
    const message = error instanceof Error ? error.message : String(error);
    const cleanupI18n = wireOf(error);
    const failed: CodexOwnedCloseResult = { childDone: false, treeGone: 'unknown', cleanupError: message, ...(cleanupI18n ? { cleanupI18n } : {}) };
    settleWork(entry, false, 'unknown', message, cleanupI18n);
    cleanupResolve(failed);
    accountWorks.delete(token);
  });
  return { token, cleanup };
}

/** 등록된 추가 계정 작업의 cleanup. 새로 붙은 작업이 있으면 그 cleanup까지 이어서 기다린다. */
export function codexAccountChildrenClosed(): Promise<void> {
  const pending = [...accountWorks.values()].map(work => work.cleanup);
  if (pending.length === 0) return Promise.resolve();
  return Promise.all(pending).then(() => codexAccountChildrenClosed());
}

/**
 * 상한을 넘긴 로그인·확인 자식을 강제 종료하고, treeGone·cleanup이 올 때까지 확인한다.
 * 플래그를 먼저 켜 확인·정리가 새 프로세스를 띄우지 못하게 한다.
 */
export async function stopCodexAccountChildren(): Promise<void> {
  beginCodexAccountProcessesStopping();
  const closeAll = async (): Promise<void> => {
    const pending = [...accountWorks.values()];
    await Promise.all(pending.map(work => work.run.close().catch(() => undefined)));
    await Promise.all(pending.map(work => work.cleanup));
  };
  if (!shutdownCleanup) shutdownCleanup = closeAll();
  await shutdownCleanup;
  await Promise.resolve();
  await closeAll();
}

/** 정리 성공·토큰 실패 없음·같은 계정 미완료 작업 없을 때만 true(A02b1). */
export function codexAccountWorkReleased(accountId: string, cleanup: Promise<CodexOwnedCloseResult>): Promise<boolean> {
  return cleanup.then(result => {
    if (!codexOwnedCloseSuccess(result) && !result.notStarted) return false;
    if (codexAccountCleanupBlocked(accountId)) return false;
    for (const work of accountWorks.values()) if (work.accountId === accountId) return false;
    return true;
  });
}

/**
 * 추가 계정 자식의 인자·환경 경계. 값과 비밀은 적지 않는다(3절 9). 실행부 콘솔이 앱 로그의 [backend] 줄이 된다.
 * 모양: `Codex 추가 계정 <용도> · 계정 <번호> · CODEX_HOME=계정 폴더 · 인자 -c cli_auth_credentials_store="file" -c forced_login_method="chatgpt" -c model_provider="openai" · 뺀 변수 …`
 */
export function logCodexAccountProcess(purpose: string, accountId: string): void {
  console.log(`Codex 추가 계정 ${purpose} · 계정 ${accountId} · CODEX_HOME=계정 폴더 · 인자 -c cli_auth_credentials_store="file" -c forced_login_method="chatgpt" -c model_provider="openai" · 뺀 변수 ${STRIPPED.join(' ')}`);
}

export async function removeStaleCodexAuth(home: string, beforeRemove?: () => void, accountId?: string): Promise<void> {
  if (isHostCodexHome(home)) throw new WireError(wire('wireEngine.account.folder.codexHostCredentials'), '기본 Codex 계정 폴더의 자격은 지우지 않아.');
  if (accountId) assertCodexAccountSpawnAllowed(accountId);
  const path = codexAuthPath(home);
  let info;
  try { info = await lstat(path); }
  catch (error) { if ((error as NodeJS.ErrnoException).code === 'ENOENT') return; throw error; }
  if (info.isSymbolicLink() || !info.isFile()) throw new WireError(wire('wireEngine.account.folder.codexAuthNotFile'), 'Codex 자격 파일 자리에 다른 것이 있어 지우지 않았어.');
  beforeRemove?.();
  if (accountId) assertCodexAccountSpawnAllowed(accountId);
  await rm(path, { force: false });
}
