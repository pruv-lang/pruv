// 바깥 터미널 대화 읽기의 공통 바탕: 상한 상수·오류·타입·꼬리 읽기·요청 모으기·자식 프로세스 대화(docs/notes/engine-import-2026-10-19.md).
// 엔진별 모듈(claude·codex·grok)이 이 파일만 import 한다. 서비스·설정 객체는 받지 않는다.
import { spawn, type ChildProcessByStdio } from 'node:child_process';
import { constants, type Stats } from 'node:fs';
import { lstat, open, realpath, type FileHandle } from 'node:fs/promises';
import { sep } from 'node:path';
import type { Readable, Writable } from 'node:stream';
import type { EngineId } from '../../shared/contracts';
import type { EngineImportEngine, LastPlan } from '../../shared/engine-import';
import { spawnEngine } from '../engine-children';
import { systemTaskkillPath } from '../windows-system';

/** 읽기 상한. 이 기능의 상한은 여기 한 곳에서만 정한다. */
export const IMPORT_LIMITS = {
  /** 파일 끝에서 읽는 창 */
  window: 8 * 1024 * 1024,
  /** 줄 하나. 넘으면 해석하지 않고 버린다 */
  line: 1024 * 1024,
  /** 요청 개수 */
  turns: 200,
  /** 답 하나 */
  reply: 32 * 1024,
  /** 가져온 글 전체(사람 글 포함) */
  total: 2 * 1024 * 1024,
  /** 목록 개수 */
  list: 50,
  /** 목록 한 파일의 머리·꼬리 */
  edge: 64 * 1024,
  /** 동시에 여는 파일·요청 */
  parallel: 4,
  /** 자식 프로세스 대화 제한 시간 */
  rpcMs: 20_000,
} as const;

export type EngineImportErrorCode =
  | 'not-found' | 'codex-unsupported' | 'grok-exists' | 'grok-load-failed' | 'timeout' | 'read-failed' | 'engine-missing';

/** 이 기능의 오류 하나. 사람용 문구는 없다(code 만 쓴다). detail 은 진단용이다. */
export class EngineImportError extends Error {
  readonly code: EngineImportErrorCode;
  constructor(code: EngineImportErrorCode, detail?: string, options?: { cause?: unknown }) {
    super(detail ? `${code}: ${detail}` : code, options);
    this.name = 'EngineImportError';
    this.code = code;
  }
}

/** 등록된 출처 한 곳. home 은 백엔드 전용 절대 경로라 화면에 보내지 않는다. */
export interface ImportSource {
  source: string;
  engine: EngineImportEngine;
  /** 추가 계정 번호(kind 가 account 일 때) */
  account?: string;
  home: string;
  kind: 'home' | 'account';
  accountName?: string;
}

/** Codex 단발 app-server 를 띄우는 데 필요한 값 */
export interface CodexRunDeps {
  binary: string;
  env: NodeJS.ProcessEnv;
  args: string[];
}

export interface ImportListItem {
  thread: string;
  title: string;
  updatedAt?: string;
  size?: number;
  messages?: number;
  path?: string;
  /** Claude 줄의 entrypoint 값 그대로(Lair 여부는 호출한 쪽이 결속으로 판정한다) */
  entrypoint?: string;
  /** Codex 의 originator 로 본 표시 */
  origin?: 'lair' | 'other';
  maybeOpen?: true;
}

export interface ExternalTurn {
  prompt: string;
  reply: string;
  createdAt?: string;
  completedAt?: string;
  tools: number;
  clipped?: true;
}

export interface ExternalRecord {
  turns: ExternalTurn[];
  omittedBefore: boolean;
  model?: string;
  title?: string;
  lastPlan?: LastPlan;
}

/** 목록·읽기 공통 옵션. env 는 Claude 폴더 이름 예외 변수를 읽을 때만 쓴다(없으면 process.env). */
export interface ImportReadOptions {
  signal: AbortSignal;
  codex?: CodexRunDeps;
  env?: NodeJS.ProcessEnv;
}

type RecordValue = Record<string, unknown>;

export function record(value: unknown): RecordValue | undefined {
  return value !== null && typeof value === 'object' && !Array.isArray(value) ? value as RecordValue : undefined;
}

export function str(value: unknown): string | undefined {
  return typeof value === 'string' ? value : undefined;
}

/** 한 줄 JSON 을 객체로 푼다. 아니면 undefined */
export function parseLine(line: string): RecordValue | undefined {
  try { return record(JSON.parse(line)); } catch { return undefined; }
}

/** 초·밀리초 숫자나 날짜 글을 ISO 로. 못 읽으면 undefined */
export function isoOf(value: unknown): string | undefined {
  let ms: number | undefined;
  if (typeof value === 'number' && Number.isFinite(value)) ms = value < 1e11 ? value * 1000 : value;
  else if (typeof value === 'string' && value.trim()) {
    ms = /^\d+(\.\d+)?$/.test(value.trim()) ? Number(value) * (Number(value) < 1e11 ? 1000 : 1) : Date.parse(value);
  }
  if (ms === undefined || !Number.isFinite(ms)) return undefined;
  const date = new Date(ms);
  return Number.isNaN(date.getTime()) ? undefined : date.toISOString();
}

/** 줄바꿈을 공백으로 접고 앞뒤를 다듬어 max 글자로 자른다 */
export function oneLine(text: string, max: number): string {
  return text.replace(/\s+/g, ' ').trim().slice(0, max);
}

/** 대화 번호를 파일 이름으로 쓰기 전 거르는 모양. 경로 글자·점으로 시작하는 이름을 막는다 */
export function validThread(thread: string): boolean {
  return /^[A-Za-z0-9][A-Za-z0-9_-]{0,127}$/.test(thread);
}

/** 글을 바이트 상한으로 자른다. 잘린 끝의 깨진 글자는 뗀다 */
export function clipBytes(text: string, max: number): string {
  if (Buffer.byteLength(text) <= max) return text;
  return Buffer.from(text, 'utf8').subarray(0, max).toString('utf8').replace(/�+$/, '');
}

/** 동시 limit 개까지 비동기 일을 돌리고 결과를 입력 순서로 돌려준다 */
export async function mapLimit<T, R>(items: readonly T[], limit: number, work: (item: T, index: number) => Promise<R>): Promise<R[]> {
  const out = new Array<R>(items.length);
  let next = 0;
  const runner = async (): Promise<void> => {
    while (next < items.length) {
      const index = next++;
      out[index] = await work(items[index], index);
    }
  };
  await Promise.all(Array.from({ length: Math.min(limit, items.length) }, runner));
  return out;
}

/** 읽기 전용 열기 플래그. O_NOFOLLOW 가 있는 운영체제에서는 마지막 이름이 링크면 열지 않는다 */
const READ_FLAGS = constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0);

/** 검사 때 본 파일의 정체(장치·inode). 연 파일과 맞춰 본다 */
export interface FileIdentity { dev: number; ino: number }

/**
 * 읽기 전용으로 연다(마지막 이름이 링크면 가능한 운영체제에서 거절). expect 를 주면 연 파일이 검사(plainFileIn) 때 본 그 파일인지
 * 장치·inode 로 맞춰 본다 — 검사와 열기 사이에 상위 폴더가 바깥 링크로 바뀌면 다른 파일이 열리므로 거절한다(감사 고친 뒤 확인 5번).
 */
export async function openRead(path: string, expect?: FileIdentity): Promise<FileHandle> {
  const handle = await open(path, READ_FLAGS);
  if (!expect) return handle;
  const info = await handle.stat().catch(() => undefined);
  if (info && info.dev === expect.dev && info.ino === expect.ino) return handle;
  await handle.close().catch(() => undefined);
  throw new EngineImportError('read-failed');
}

/** 링크가 아닌 보통 폴더인가. 출처 안의 폴더 사슬을 한 칸씩 이 검사로 지난다 */
export async function plainDir(path: string): Promise<boolean> {
  const info = await lstat(path).catch(() => undefined);
  return info !== undefined && !info.isSymbolicLink() && info.isDirectory();
}

/**
 * 링크가 아닌 보통 파일이고 실제 경로가 출처 home 의 실제 경로 아래일 때만 그 정보(lstat)를 돌려준다.
 * homeReal 은 호출 한 번에 한 번만 realpath(home) 으로 구해 넘긴다.
 */
export async function plainFileIn(homeReal: string, path: string): Promise<Stats | undefined> {
  const info = await lstat(path).catch(() => undefined);
  if (!info || info.isSymbolicLink() || !info.isFile()) return undefined;
  const real = await realpath(path).catch(() => undefined);
  return real !== undefined && real.startsWith(homeReal.endsWith(sep) ? homeReal : homeReal + sep) ? info : undefined;
}

/**
 * 파일 머리 또는 꼬리 edge 바이트의 온전한 줄들. 머리는 끝의 잘린 줄을, 꼬리는 첫 잘린 줄을 버린다.
 * size 는 stat 으로 이미 안 파일 크기다.
 */
export async function readEdgeLines(path: string, edge: 'head' | 'tail', size: number, expect?: FileIdentity): Promise<string[]> {
  if (size <= 0) return [];
  const handle = await openRead(path, expect);
  try {
    const length = Math.min(size, IMPORT_LIMITS.edge);
    const start = edge === 'head' ? 0 : size - length;
    const buffer = Buffer.alloc(length);
    const { bytesRead } = await handle.read(buffer, 0, length, start);
    const lines = buffer.subarray(0, bytesRead).toString('utf8').split('\n');
    if (edge === 'head' && size > length) lines.pop();
    if (edge === 'tail' && start > 0) lines.shift();
    return lines.filter(line => line.length > 0);
  } finally {
    await handle.close();
  }
}

export interface LineWindow {
  /** 창이 파일 처음이 아니라서 앞이 잘렸다 */
  omittedBefore: boolean;
  lines: AsyncGenerator<string>;
}

export interface LineDropWatch {
  /** 버린 1MB 넘는 줄의 앞 8KB(글)가 사람 줄인지. 엔진마다 도구 결과·메타 줄을 가려 판정한다 */
  isUser(head: string): boolean;
  /** isUser 가 참이면 그 줄 자리에서(뒤 줄을 내기 전에) 부른다 */
  onUserDropped(): void;
}

const DROP_HEAD = 8 * 1024;

/** 줄 조각들의 앞 DROP_HEAD 바이트(글로 본다) */
function headText(parts: Buffer[], rest: Buffer): string {
  const picked: Buffer[] = [];
  let size = 0;
  for (const part of [...parts, rest]) {
    if (size >= DROP_HEAD) break;
    picked.push(part.subarray(0, DROP_HEAD - size));
    size += picked[picked.length - 1].length;
  }
  return Buffer.concat(picked).toString('utf8');
}

/**
 * 파일 끝에서 최대 8MB 창을 비동기 스트림으로 줄 단위로 읽는다.
 * 창이 파일 처음이 아니면 첫 줄(잘림)을 버리고, 끝의 줄바꿈 없는 줄과 1MB 넘는 줄은 해석 전에 버린다.
 * 버린 줄이 사람 줄이면 watch 로 알린다. 소비하는 쪽이 중간에 그만두면 스트림이 닫힌다.
 */
export async function openLineWindow(path: string, signal: AbortSignal, watch?: LineDropWatch, expect?: FileIdentity): Promise<LineWindow> {
  const handle = await openRead(path, expect);
  let info: Stats;
  try { info = await handle.stat(); } catch (error) { await handle.close().catch(() => undefined); throw error; }
  const start = Math.max(0, info.size - IMPORT_LIMITS.window);
  async function* lines(): AsyncGenerator<string> {
    if (info.size === 0) { await handle.close(); return; }
    const stream = handle.createReadStream({ start, end: info.size - 1, highWaterMark: 256 * 1024 });
    let parts: Buffer[] = [];
    let length = 0;
    // 창이 중간에서 시작하면 첫 줄은 잘렸다. 첫 줄바꿈까지 건너뛴다
    let skipping = start > 0;
    const dropped = (rest: Buffer): void => {
      if (watch?.isUser(headText(parts, rest))) watch.onUserDropped();
    };
    try {
      for await (const chunk of stream) {
        signal.throwIfAborted();
        const data = chunk as Buffer;
        let from = 0;
        for (;;) {
          const at = data.indexOf(0x0a, from);
          if (at < 0) {
            if (!skipping) {
              length += data.length - from;
              if (length > IMPORT_LIMITS.line) { dropped(data.subarray(from)); skipping = true; parts = []; length = 0; }
              else parts.push(data.subarray(from));
            }
            break;
          }
          if (skipping) skipping = false;
          else if (length + (at - from) <= IMPORT_LIMITS.line) {
            parts.push(data.subarray(from, at));
            const line = Buffer.concat(parts).toString('utf8');
            parts = [];
            length = 0;
            yield line;
          } else dropped(data.subarray(from, at));
          parts = [];
          length = 0;
          from = at + 1;
        }
      }
    } finally {
      stream.destroy();
      await handle.close().catch(() => undefined);
    }
  }
  return { omittedBefore: start > 0, lines: lines() };
}

interface Draft {
  prompt: string;
  reply: string;
  replyBytes: number;
  tools: number;
  createdAt?: string;
  completedAt?: string;
  clipped?: true;
}

/** 요청(사람 글 하나 + 그 뒤 답)을 모으고 개수·크기 상한으로 앞을 버린다(최근 것을 남긴다) */
export class TurnCollector {
  private readonly drafts: Draft[] = [];
  private current: Draft | undefined;
  /** 상한으로 버린 사람 줄이 바로 앞에 있었다. 다음 요청을 clipped 로 둔다 */
  private lostBefore = false;

  begin(prompt: string, createdAt?: string): void {
    // 사람 글도 답 하나와 같은 상한으로 자른다(Lair 가 만든 대화의 첫 글은 앱 문맥이 붙어 수십 KB 다, 10-19 실측).
    const long = Buffer.byteLength(prompt) > IMPORT_LIMITS.reply || this.lostBefore;
    this.lostBefore = false;
    this.current = { prompt: clipBytes(prompt, IMPORT_LIMITS.reply), reply: '', replyBytes: 0, tools: 0, ...(createdAt ? { createdAt } : {}), ...(long ? { clipped: true as const } : {}) };
    this.drafts.push(this.current);
  }

  /** 1MB 넘어 버린 사람 줄이 있었다: 지금 요청을 닫아 뒤 답이 앞 요청에 붙지 않게 하고(그 사이 답은 버린다), 다음 요청을 잘림으로 둔다 */
  skipUser(): void {
    this.current = undefined;
    this.lostBefore = true;
  }

  /** 지금 요청에 답 글을 이어 붙인다. 시작한 요청이 없으면(사람 글 없이 시작한 앞쪽 답) 버린다 */
  answer(text: string, at?: string): void {
    const turn = this.current;
    if (!turn) return;
    if (at) turn.completedAt = at;
    // 사람 글이 잘려 clipped 가 섰어도 답은 이어 붙인다. 답이 상한에 닿았는지만 본다.
    if (!text || turn.replyBytes >= IMPORT_LIMITS.reply) return;
    const piece = turn.reply ? `\n\n${text}` : text;
    const size = Buffer.byteLength(piece);
    const room = IMPORT_LIMITS.reply - turn.replyBytes;
    if (size <= room) {
      turn.reply += piece;
      turn.replyBytes += size;
      return;
    }
    turn.reply += clipBytes(piece, Math.max(0, room));
    turn.replyBytes = IMPORT_LIMITS.reply;
    turn.clipped = true;
  }

  tool(count = 1): void {
    if (this.current) this.current.tools += count;
  }

  /** 최근 것부터 요청 200개·전체 2MB 안에서 남긴다. trimmed 는 앞을 버렸는지 */
  finish(): { turns: ExternalTurn[]; trimmed: boolean } {
    let kept = 0;
    let total = 0;
    for (let index = this.drafts.length - 1; index >= 0 && kept < IMPORT_LIMITS.turns; index--) {
      const draft = this.drafts[index];
      const size = Buffer.byteLength(draft.prompt) + Buffer.byteLength(draft.reply);
      if (total + size > IMPORT_LIMITS.total) break;
      total += size;
      kept++;
    }
    const turns = this.drafts.slice(this.drafts.length - kept).map((draft): ExternalTurn => ({
      prompt: draft.prompt, reply: draft.reply, tools: draft.tools,
      ...(draft.createdAt ? { createdAt: draft.createdAt } : {}),
      ...(draft.completedAt ? { completedAt: draft.completedAt } : {}),
      ...(draft.clipped ? { clipped: true as const } : {}),
    }));
    return { turns, trimmed: kept < this.drafts.length };
  }
}

/** 상대가 JSON-RPC 오류로 답했다 */
export class RpcResponseError extends Error {
  readonly code: unknown;
  constructor(message: string, code?: unknown) {
    super(message);
    this.name = 'RpcResponseError';
    this.code = code;
  }
}

export interface Rpc {
  request(method: string, params: unknown): Promise<unknown>;
  notify(method: string, params: unknown): void;
  /** 시간 초과·중단·프로세스 종료로 대화가 이미 실패했으면 그 오류 */
  readonly failure: Error | undefined;
}

export interface RpcChildOptions {
  engine: EngineId;
  binary: string;
  args: readonly string[];
  env: NodeJS.ProcessEnv;
  cwd: string;
  /** 참이면 줄마다 jsonrpc 2.0 칸을 붙인다(ACP). 거짓이면 Codex app-server 모양 */
  jsonrpc: boolean;
  signal: AbortSignal;
  timeoutMs: number;
  /** 참이면 자식을 새 프로세스 묶음으로 띄우고 묶음째 끝낸다(맥·리눅스) */
  detached?: boolean;
}

function abortError(): Error {
  return Object.assign(new Error('aborted'), { name: 'AbortError' });
}

/**
 * 엔진 실행 파일을 이 요청 동안만 띄워 줄 단위 JSON 으로 대화한다. 성공·실패·중단 모두 같은 finally 에서
 * 타이머·구독을 풀고 자식을 끝낸 뒤(SIGTERM, 1.5초 뒤 SIGKILL, 최대 5초 기다림) 돌려준다.
 */
export async function withRpcChild<T>(options: RpcChildOptions, work: (rpc: Rpc) => Promise<T>): Promise<T> {
  if (options.signal.aborted) throw abortError();
  let child: ChildProcessByStdio<Writable, Readable, Readable>;
  try {
    child = spawnEngine(options.engine, options.binary, options.args, {
      cwd: options.cwd, env: options.env, shell: false, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'],
      detached: options.detached === true && process.platform !== 'win32',
    });
  } catch (error) {
    throw new EngineImportError('engine-missing', error instanceof Error ? error.message : String(error), { cause: error });
  }
  const waiting = new Map<number, { resolve(value: unknown): void; reject(error: Error): void }>();
  let nextId = 1;
  let buffer = '';
  let failure: Error | undefined;
  let exited = false;
  let markClosed!: () => void;
  const closed = new Promise<void>(resolve => { markClosed = () => { exited = true; resolve(); }; });

  const fail = (error: Error): void => {
    if (failure) return;
    failure = error;
    for (const entry of waiting.values()) entry.reject(error);
    waiting.clear();
  };
  const write = (value: RecordValue): void => {
    if (failure || exited || !child.stdin.writable) throw failure ?? new EngineImportError('read-failed', 'closed');
    child.stdin.write(`${JSON.stringify(options.jsonrpc ? { jsonrpc: '2.0', ...value } : value)}\n`);
  };
  const terminate = (signalName: 'SIGTERM' | 'SIGKILL'): void => {
    if (exited || child.exitCode !== null || child.signalCode !== null) return;
    try {
      if (process.platform === 'win32') {
        if (child.pid) spawn(systemTaskkillPath(), ['/PID', String(child.pid), '/T', '/F'], { windowsHide: true, stdio: 'ignore' });
      } else if (options.detached && child.pid) process.kill(-child.pid, signalName);
      else child.kill(signalName);
    } catch { /* 이미 끝났다 */ }
  };

  const onAbort = (): void => fail(abortError());
  options.signal.addEventListener('abort', onAbort, { once: true });
  const timer = setTimeout(() => fail(new EngineImportError('timeout')), options.timeoutMs);

  child.stdout.setEncoding('utf8');
  child.stdout.on('data', (text: string) => {
    buffer += text;
    let newline: number;
    while ((newline = buffer.indexOf('\n')) !== -1) {
      const line = buffer.slice(0, newline).trim();
      buffer = buffer.slice(newline + 1);
      if (!line) continue;
      const message = parseLine(line);
      if (!message) continue;
      if (typeof message.method === 'string') {
        // 서버가 묻는 요청은 처리하지 않는다고 답해 기다리지 않게 한다. 알림은 무시한다
        if (message.id !== undefined) {
          try { write({ id: message.id, error: { code: -32601, message: 'not handled' } }); } catch { /* 곧 끝낸다 */ }
        }
        continue;
      }
      const entry = typeof message.id === 'number' ? waiting.get(message.id) : undefined;
      if (!entry || typeof message.id !== 'number') continue;
      waiting.delete(message.id);
      const error = record(message.error);
      if (error) entry.reject(new RpcResponseError(str(error.message)?.slice(0, 300) ?? 'rejected', error.code));
      else entry.resolve(message.result);
    }
    if (buffer.length > IMPORT_LIMITS.window) fail(new EngineImportError('read-failed', 'line too long'));
  });
  // 읽지 않으면 가득 차 자식이 멈출 수 있어 버린다
  child.stderr.on('data', () => undefined);
  child.stdin.on('error', () => undefined);
  child.stdout.on('error', error => fail(new EngineImportError('read-failed', error.message, { cause: error })));
  child.on('error', error => {
    fail(new EngineImportError('engine-missing', error.message, { cause: error }));
    markClosed();
  });
  child.on('close', () => {
    fail(new EngineImportError('read-failed', 'exited'));
    markClosed();
  });

  const rpc: Rpc = {
    request(method, params) {
      if (failure) return Promise.reject(failure);
      return new Promise<unknown>((resolve, reject) => {
        const id = nextId++;
        waiting.set(id, { resolve, reject });
        try { write({ id, method, params }); } catch (error) {
          waiting.delete(id);
          reject(error instanceof Error ? error : new Error(String(error)));
        }
      });
    },
    notify(method, params) { write({ method, params }); },
    get failure() { return failure; },
  };

  try {
    return await work(rpc);
  } finally {
    options.signal.removeEventListener('abort', onAbort);
    clearTimeout(timer);
    fail(new EngineImportError('read-failed', 'finished'));
    try { child.stdin.end(); } catch { /* 닫혔다 */ }
    terminate('SIGTERM');
    const killTimer = setTimeout(() => terminate('SIGKILL'), 1_500);
    await new Promise<void>(resolve => {
      const giveUp = setTimeout(resolve, 5_000);
      void closed.then(() => { clearTimeout(giveUp); resolve(); });
    });
    clearTimeout(killTimer);
  }
}
