// 앱이 MCP 서버에 붙는 클라이언트(docs/notes/mcp-registry-2026-10-04.md R2·R5). 의존성 없이 stdio(줄 단위 JSON-RPC)와 Streamable HTTP 를 다룬다.
// 비밀(환경·헤더)과 요청 본문은 기록하지 않는다. 실패 글에는 명령 이름·주소의 호스트만 넣는다.
import { spawn, type ChildProcess } from 'node:child_process';
import { createHash } from 'node:crypto';
import type { McpToolInfo, McpTransport } from '../shared/contracts';
import { wire, wire as wireText, WireError, wireLocal, wireOf, type WireText } from '../shared/i18n/wire';
import { systemTaskkillPath } from './windows-system';

export interface McpLaunch { transport: McpTransport; env: Record<string, string>; headers: Record<string, string>; cwd: string; baseEnv: NodeJS.ProcessEnv;
  /** 로그인 연결이면 둔다. 모든 요청이 리다이렉트를 따르지 않고, 원래 요청이 HTTP 401 을 받으면 논리 요청당 한 번 refresh(그 POST 가 실제로 쓴 Authorization 값) 뒤 다시 보낸다(launch.headers 는 refresh 가 고친다). */
  oauth?: { refresh: (used: string | undefined) => Promise<boolean> } }
export type McpStage = 'start' | 'connect' | 'list';

export class McpFailure extends Error {
  readonly stage: McpStage;
  /** message 의 전송 글(다국어, B-294). 시험 결과 messageI18n 으로 간다. */
  readonly i18n?: WireText;
  constructor(stage: McpStage, message: string, i18n?: WireText) {
    super(message);
    this.name = 'McpFailure';
    this.stage = stage;
    if (i18n) this.i18n = i18n;
  }
}

/** 로그인 연결이 401(재시도 뒤에도)·403 을 받았다. */
export class McpAuthFailure extends McpFailure {
  readonly status: 401 | 403;
  readonly scope?: string;
  readonly authError?: string;
  constructor(status: 401 | 403, message: string, details: { scope?: string; authError?: string } = {}, i18n?: WireText) {
    super('connect', message, i18n);
    this.name = 'McpAuthFailure';
    this.status = status;
    if (details.scope) this.scope = details.scope;
    if (details.authError) this.authError = details.authError;
  }
}

/** WWW-Authenticate 머리글에서 매개변수 하나(따옴표 있든 없든)를 읽는다. */
function challengeParam(header: string, name: string): string | undefined {
  const match = new RegExp(`(?:^|[\\s,])${name}\\s*=\\s*(?:"([^"]*)"|([^\\s,"]+))`, 'i').exec(header);
  const value = match?.[1] ?? match?.[2];
  return value ? oneLine(value, 200) : undefined;
}

export interface McpSession {
  /** 서버 도구 정의 원본 객체들(커서를 따라 최대 5쪽·500개) */
  tools(signal: AbortSignal): Promise<Record<string, unknown>[]>;
  /** tools/call. 결과 객체(CallToolResult) 그대로. JSON-RPC 오류면 { content:[{type:'text',text:오류}], isError:true } 로 바꿔 돌려준다. */
  call(name: string, args: Record<string, unknown>, signal: AbortSignal): Promise<Record<string, unknown>>;
  /** 프로세스 묶음·연결 정리. 끝남을 확인할 때까지 기다린다. 여러 번 불러도 된다. */
  close(): Promise<void>;
  readonly closed: boolean;
}

type RecordValue = Record<string, unknown>;

const MAX_LINE = 4 * 1024 * 1024;
const STDERR_KEEP = 1024;
const MAX_PAGES = 5;
const MAX_TOOLS = 500;
const INIT_TIMEOUT = 30_000;
const TERM_WAIT = 2000;
const KILL_WAIT = 3000;
const NEW_PROTOCOL = '2025-11-25';
const OLD_PROTOCOL = '2025-06-18';
const UNSUPPORTED = { code: -32601, message: 'Lair 앱은 이 요청을 지원하지 않아.' };

/** tools/call 을 보낸 뒤 응답을 받지 못했다(연결 끊김·SSE 도중 끊김). 서버가 실행했는지 알 수 없다. */
export class McpUnknownOutcome extends Error {
  constructor(message: string) { super(message); this.name = 'McpUnknownOutcome'; }
}

class RpcError extends Error {
  readonly code: number;
  /** 서버가 글을 주지 않아 앱이 채운 문장일 때만 있다. */
  readonly i18n?: WireText;
  constructor(code: number, message: string, i18n?: WireText) {
    super(message);
    this.name = 'RpcError';
    this.code = code;
    if (i18n) this.i18n = i18n;
  }
}

/** 서버가 준 오류 객체로 RpcError 를 만든다. 글이 없으면 앱 문장 '서버 오류'. */
function rpcErrorOf(error: RecordValue): RpcError {
  const code = typeof error.code === 'number' ? error.code : -32603;
  return typeof error.message === 'string' ? new RpcError(code, error.message) : new RpcError(code, '서버 오류', wire('wireMcp.client.serverError'));
}

function closedError(): Error {
  return new WireError(wire('wireMcp.client.closed'), '연결이 닫혔어.');
}

/** 요청 하나를 보내고 결과를 받는 길. 오류 응답은 RpcError 로 던진다. */
interface Wire {
  request(method: string, params: RecordValue | undefined, signal: AbortSignal): Promise<unknown>;
  notify(method: string, params?: RecordValue): void;
  /** 초기화가 끝난 뒤 알려 준다(HTTP 머리·종료 글이 달라진다). */
  ready(protocolVersion: string): void;
  close(): Promise<void>;
  readonly closed: boolean;
}

const object = (value: unknown): RecordValue | undefined =>
  value !== null && typeof value === 'object' && !Array.isArray(value) ? value as RecordValue : undefined;

function abortError(): Error {
  const error = Object.assign(new Error('중단됐어.'), { i18n: wire('wireMcp.client.aborted') });
  error.name = 'AbortError';
  return error;
}

function oneLine(text: string, max = 300): string {
  const line = text.replace(/\s+/g, ' ').trim();
  return line.length > max ? line.slice(0, max) : line;
}

function lastLine(text: string): string {
  const lines = text.split(/\r?\n/).map(line => line.trim()).filter(Boolean);
  return oneLine(lines[lines.length - 1] ?? '', 200);
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function serverRequestReply(message: RecordValue): RecordValue {
  const id = message.id as string | number;
  return message.method === 'ping' ? { jsonrpc: '2.0', id, result: {} } : { jsonrpc: '2.0', id, error: UNSUPPORTED };
}

function cancelParams(id: number): RecordValue {
  return { requestId: id, reason: 'cancelled' };
}

// ───────────────────────── stdio ─────────────────────────

function stdioWire(launch: McpLaunch, command: string, args: string[]): Wire {
  let child: ChildProcess;
  let nextId = 1;
  let fatal: Error | undefined;
  let exited = false;
  let isReady = false;
  let isClosed = false;
  let stderrTail = '';
  let buffer = '';
  const pending = new Map<number, { resolve(value: unknown): void; reject(error: Error): void }>();
  const exitWaiters: Array<() => void> = [];

  const failAll = (error: Error) => {
    fatal ??= error;
    for (const entry of [...pending.values()]) entry.reject(error);
    pending.clear();
  };
  const write = (message: RecordValue) => {
    if (exited || !child.stdin || child.stdin.destroyed) return;
    try { child.stdin.write(`${JSON.stringify(message)}\n`); } catch { /* 끊긴 파이프는 종료 처리가 맡는다 */ }
  };

  const onMessage = (message: RecordValue) => {
    if (message.id !== undefined && typeof message.method === 'string') { write(serverRequestReply(message)); return; }
    if (typeof message.id !== 'number') return;
    const entry = pending.get(message.id);
    if (!entry) return;
    pending.delete(message.id);
    const error = object(message.error);
    if (error) entry.reject(rpcErrorOf(error));
    else entry.resolve(message.result);
  };
  const onLine = (line: string) => {
    const text = line.trim();
    if (!text.startsWith('{')) return;
    let parsed: unknown;
    try { parsed = JSON.parse(text); } catch { return; }
    const message = object(parsed);
    if (message) onMessage(message);
  };

  try {
    child = spawn(command, args, {
      cwd: launch.cwd,
      env: { ...launch.baseEnv, ...launch.env },
      stdio: ['pipe', 'pipe', 'pipe'],
      detached: process.platform !== 'win32',
      shell: false,
      windowsHide: true,
    });
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code;
    fatal = code === 'ENOENT' ? new McpFailure('start', `명령을 찾지 못했어: ${command}`, wire('wireMcp.client.commandNotFound', { command }))
      : new McpFailure('start', `명령을 시작하지 못했어: ${command}`, wire('wireMcp.client.commandStartFailed', { command }));
    exited = true;
    child = undefined as unknown as ChildProcess;
  }

  if (child!) {
    child.stdout?.setEncoding('utf8');
    child.stderr?.setEncoding('utf8');
    child.stdout?.on('data', (chunk: string) => {
      buffer += chunk;
      for (let at = buffer.indexOf('\n'); at >= 0; at = buffer.indexOf('\n')) {
        const line = buffer.slice(0, at);
        buffer = buffer.slice(at + 1);
        if (line.length > MAX_LINE) { failAll(new McpFailure('connect', '서버가 너무 긴 줄을 보냈어.', wire('wireMcp.client.lineTooLong'))); return; }
        onLine(line);
      }
      if (buffer.length > MAX_LINE) { buffer = ''; failAll(new McpFailure('connect', '서버가 너무 긴 줄을 보냈어.', wire('wireMcp.client.lineTooLong'))); }
    });
    child.stderr?.on('data', (chunk: string) => { stderrTail = (stderrTail + chunk).slice(-STDERR_KEEP); });
    child.stdin?.on('error', () => { /* EPIPE: 종료 처리가 맡는다 */ });
    child.on('error', error => {
      const code = (error as NodeJS.ErrnoException).code;
      exited = true;
      failAll(code === 'ENOENT' ? new McpFailure('start', `명령을 찾지 못했어: ${command}`, wire('wireMcp.client.commandNotFound', { command }))
        : new McpFailure('start', `명령을 시작하지 못했어: ${command}${code ? ` (${code})` : ''}`,
          code ? wire('wireMcp.client.commandStartFailedCode', { command, code }) : wire('wireMcp.client.commandStartFailed', { command })));
      for (const waiter of exitWaiters.splice(0)) waiter();
    });
    child.on('exit', (code, signal) => {
      exited = true;
      const tail = lastLine(stderrTail);
      const how = code !== null ? `종료 코드 ${code}` : `신호 ${signal}`;
      const howEn = code !== null ? `exit code ${code}` : `signal ${signal}`;
      const reason = isReady
        ? `서버가 끝났어(${how})`
        : `서버가 시작 직후 끝났어(${how})`;
      // 영어 자리 이름(howEn)은 사전 모양에 없어 변수로 넘긴다(wire-work 의 같은 방식).
      const howArgs = { how, howEn };
      const tailArgs = { how, howEn, tail };
      const i18n = isReady
        ? (tail ? wire('wireMcp.client.exitedTail', tailArgs) : wire('wireMcp.client.exited', howArgs))
        : (tail ? wire('wireMcp.client.exitEarlyTail', tailArgs) : wire('wireMcp.client.exitEarly', howArgs));
      failAll(isReady ? new WireError(i18n, tail ? `${reason}: ${tail}` : reason) : new McpFailure('connect', tail ? `${reason}: ${tail}` : reason, i18n));
      for (const waiter of exitWaiters.splice(0)) waiter();
    });
  }

  const waitExit = (ms: number) => new Promise<void>(resolve => {
    if (exited) { resolve(); return; }
    const timer = setTimeout(done, ms);
    function done() { clearTimeout(timer); const at = exitWaiters.indexOf(done); if (at >= 0) exitWaiters.splice(at, 1); resolve(); }
    exitWaiters.push(done);
  });

  const killGroup = (signal: NodeJS.Signals) => {
    const pid = child.pid;
    if (pid === undefined) return;
    try { process.kill(-pid, signal); } catch { try { process.kill(pid, signal); } catch { /* 이미 없다 */ } }
  };

  return {
    get closed() { return isClosed; },
    ready() { isReady = true; },
    request(method, params, signal) {
      if (fatal) return Promise.reject(fatal);
      if (isClosed) return Promise.reject(closedError());
      if (signal.aborted) return Promise.reject(abortError());
      const id = nextId++;
      return new Promise<unknown>((resolve, reject) => {
        const onAbort = () => {
          if (!pending.delete(id)) return;
          write({ jsonrpc: '2.0', method: 'notifications/cancelled', params: cancelParams(id) });
          reject(abortError());
        };
        pending.set(id, {
          resolve: value => { signal.removeEventListener('abort', onAbort); resolve(value); },
          reject: error => { signal.removeEventListener('abort', onAbort); reject(error); },
        });
        signal.addEventListener('abort', onAbort, { once: true });
        write({ jsonrpc: '2.0', id, method, ...(params ? { params } : {}) });
      });
    },
    notify(method, params) { write({ jsonrpc: '2.0', method, ...(params ? { params } : {}) }); },
    async close() {
      if (isClosed) return;
      isClosed = true;
      failAll(closedError());
      if (!child) return;
      try { child.stdin?.end(); } catch { /* 이미 닫혔다 */ }
      if (process.platform === 'win32') {
        if (child.pid !== undefined && !exited) {
          await new Promise<void>(resolve => {
            try {
              const killer = spawn(systemTaskkillPath(), ['/pid', String(child.pid), '/T', '/F'], { stdio: 'ignore', windowsHide: true });
              killer.once('error', () => resolve());
              killer.once('exit', () => resolve());
            } catch { resolve(); }
          });
        }
        await waitExit(KILL_WAIT);
        return;
      }
      killGroup('SIGTERM');
      await waitExit(TERM_WAIT);
      if (!exited) {
        killGroup('SIGKILL');
        await waitExit(KILL_WAIT);
      } else {
        // 대장이 먼저 끝나도 묶음에 남은 자식이 있을 수 있다.
        killGroup('SIGKILL');
      }
      child.stdout?.destroy();
      child.stderr?.destroy();
    },
  };
}

// ───────────────────────── http ─────────────────────────

function httpWire(launch: McpLaunch, url: string): Wire {
  let nextId = 1;
  let sessionId: string | undefined;
  let protocol: string | undefined;
  let reached = false;
  let isClosed = false;
  const controller = new AbortController();

  let host = '서버';
  try { host = new URL(url).host; } catch { /* 주소가 이상하면 아래 fetch 가 실패한다 */ }

  const headers = (): Record<string, string> => ({
    ...launch.headers,
    'Content-Type': 'application/json',
    Accept: 'application/json, text/event-stream',
    ...(protocol ? { 'MCP-Protocol-Version': protocol } : {}),
    ...(sessionId ? { 'Mcp-Session-Id': sessionId } : {}),
  });

  const post = async (body: RecordValue, signal: AbortSignal, usedOut?: { value?: string }): Promise<Response> => {
    let response: Response;
    const sent = headers();
    if (usedOut) usedOut.value = sent.Authorization;
    try {
      response = await fetch(url, { method: 'POST', headers: sent, body: JSON.stringify(body), signal, ...(launch.oauth ? { redirect: 'error' as const } : {}) });
    } catch (error) {
      if (signal.aborted) throw abortError();
      if (!reached) throw new McpFailure('start', `주소에 연결하지 못했어: ${host}`, wire('wireMcp.client.startUnreachable', { host }));
      throw new WireError(wire('wireMcp.client.connectFailed', { host }), `서버에 연결하지 못했어: ${host}`);
    }
    reached = true;
    return response;
  };

  const statusFailure = (status: number, res?: Response): Error => {
    if (launch.oauth && res && (status === 401 || status === 403)) {
      const challenge = res.headers.get('www-authenticate') ?? '';
      const scope = challengeParam(challenge, 'scope');
      const authError = challengeParam(challenge, 'error');
      const message = status === 401
        ? '로그인이 풀렸어. 설정 MCP 연결에서 다시 로그인해 줘'
        : authError === 'insufficient_scope'
          ? `권한이 더 필요해${scope ? `(${scope})` : ''}. 다시 로그인해 줘`
          : '이 계정에는 권한이 없어(HTTP 403)';
      const i18n = status === 401 ? wire('wireWork.mcp.auth.loginLost')
        : authError === 'insufficient_scope' ? (scope ? wire('wireMcp.client.scopeNeeded', { scope }) : wire('wireMcp.client.scopeNeededPlain'))
          : wire('wireMcp.client.forbidden');
      return new McpAuthFailure(status, message, { scope, authError }, i18n);
    }
    if (status === 401 || status === 403) return new McpFailure('connect', `로그인이 필요한 서버야(HTTP ${status})`, wire('wireMcp.client.loginNeededStatus', { status }));
    return new McpFailure('connect', `HTTP ${status}`);
  };

  const answerServerRequest = (message: RecordValue, signal: AbortSignal) => {
    post(serverRequestReply(message), signal).then(res => res.body?.cancel().catch(() => undefined), () => undefined);
  };

  /** 이 id 의 응답 메시지를 찾는다. 서버가 보낸 요청에는 답한다. */
  const pick = (message: RecordValue, id: number, signal: AbortSignal): RecordValue | undefined => {
    if (message.id !== undefined && typeof message.method === 'string') { answerServerRequest(message, signal); return undefined; }
    return message.id === id ? message : undefined;
  };

  const readSse = async (res: Response, id: number, signal: AbortSignal): Promise<RecordValue | undefined> => {
    const reader = res.body?.getReader();
    if (!reader) return undefined;
    const decoder = new TextDecoder();
    let text = '';
    let data: string[] = [];
    let found: RecordValue | undefined;
    const dispatch = () => {
      if (!data.length) return;
      const joined = data.join('\n');
      data = [];
      let parsed: unknown;
      try { parsed = JSON.parse(joined); } catch { return; }
      for (const item of Array.isArray(parsed) ? parsed : [parsed]) {
        const message = object(item);
        const hit = message ? pick(message, id, signal) : undefined;
        if (hit) found = hit;
      }
    };
    const eat = (line: string) => {
      if (line === '') { dispatch(); return; }
      if (line.startsWith('data:')) data.push(line.slice(5).replace(/^ /, ''));
    };
    try {
      while (!found) {
        const { done, value } = await reader.read();
        if (done) break;
        text += decoder.decode(value, { stream: true });
        if (text.length > MAX_LINE * 2) throw new McpFailure('connect', '서버 응답이 너무 커.', wire('wireMcp.client.tooBig'));
        for (let at = text.search(/\r\n|\n|\r/); at >= 0; at = text.search(/\r\n|\n|\r/)) {
          const sep = text.startsWith('\r\n', at) ? 2 : 1;
          eat(text.slice(0, at));
          text = text.slice(at + sep);
          if (found) break;
        }
      }
      if (!found) { eat(text); dispatch(); }
    } catch (error) {
      if (signal.aborted) throw abortError();
      throw error;
    } finally {
      reader.cancel().catch(() => undefined);
    }
    return found;
  };

  return {
    get closed() { return isClosed; },
    ready(version) { protocol = version; },
    async request(method, params, signal) {
      if (isClosed) throw closedError();
      if (signal.aborted) throw abortError();
      const id = nextId++;
      const both = AbortSignal.any([signal, controller.signal]);
      const onAbort = () => {
        post({ jsonrpc: '2.0', method: 'notifications/cancelled', params: cancelParams(id) }, controller.signal).then(res => res.body?.cancel().catch(() => undefined), () => undefined);
      };
      signal.addEventListener('abort', onAbort, { once: true });
      try {
        const body = { jsonrpc: '2.0', id, method, ...(params ? { params } : {}) };
        const used: { value?: string } = {};
        let res: Response;
        try { res = await post(body, both, used); } catch (error) {
          if (method === 'tools/call' && !(error instanceof McpFailure) && !both.aborted && reached) throw new McpUnknownOutcome(errorText(error));
          throw error;
        }
        if (res.status === 401 && launch.oauth) {
          const first = res;
          first.body?.cancel().catch(() => undefined);
          let renewed = false;
          try { renewed = await launch.oauth.refresh(used.value?.replace(/^Bearer /, '')); } catch { renewed = false; }
          if (both.aborted) throw abortError();
          if (renewed) {
            try { res = await post(body, both); } catch (error) {
              if (method === 'tools/call' && !(error instanceof McpFailure) && !both.aborted) throw new McpUnknownOutcome(errorText(error));
              throw error;
            }
          }
        }
        const session = res.headers.get('mcp-session-id');
        if (session && method === 'initialize') sessionId = session;
        if (!res.ok) { res.body?.cancel().catch(() => undefined); throw statusFailure(res.status, res); }
        const type = (res.headers.get('content-type') ?? '').split(';')[0]!.trim().toLowerCase();
        let message: RecordValue | undefined;
        // tools/call 은 보낸 뒤 응답을 못 받으면 서버가 실행했는지 모른다(A12).
        const lost = (text: string, i18n: WireText): Error => method === 'tools/call' ? new McpUnknownOutcome(text) : new McpFailure('connect', text, i18n);
        try {
          if (type === 'text/event-stream') {
            message = await readSse(res, id, both);
          } else {
            const raw = await res.text();
            if (raw.length > MAX_LINE) throw lost('서버 응답이 너무 커.', wire('wireMcp.client.tooBig'));
            let parsed: unknown;
            try { parsed = JSON.parse(raw); } catch { throw lost('서버 응답이 JSON 이 아니야.', wire('wireMcp.client.notJson')); }
            for (const item of Array.isArray(parsed) ? parsed : [parsed]) {
              const one = object(item);
              const hit = one ? pick(one, id, both) : undefined;
              if (hit) message = hit;
            }
          }
        } catch (error) {
          if (both.aborted) throw abortError();
          if (method === 'tools/call' && !(error instanceof McpUnknownOutcome)) throw new McpUnknownOutcome(errorText(error));
          throw error;
        }
        if (both.aborted) throw abortError();
        if (!message) throw lost('서버가 답을 주지 않았어.', wire('wireMcp.client.noAnswer'));
        const error = object(message.error);
        if (error) throw rpcErrorOf(error);
        return message.result;
      } finally {
        signal.removeEventListener('abort', onAbort);
      }
    },
    notify(method, params) {
      if (isClosed) return;
      post({ jsonrpc: '2.0', method, ...(params ? { params } : {}) }, controller.signal)
        .then(res => res.body?.cancel().catch(() => undefined), () => undefined);
    },
    async close() {
      if (isClosed) return;
      isClosed = true;
      controller.abort();
      if (!sessionId || !reached) return;
      try {
        await fetch(url, { method: 'DELETE', headers: { ...launch.headers, 'Mcp-Session-Id': sessionId, ...(protocol ? { 'MCP-Protocol-Version': protocol } : {}) }, signal: AbortSignal.timeout(2000), ...(launch.oauth ? { redirect: 'error' as const } : {}) });
      } catch { /* 실패는 무시한다 */ }
    },
  };
}

// ───────────────────────── 세션 ─────────────────────────

function makeSession(wire: Wire): McpSession {
  return {
    get closed() { return wire.closed; },
    async tools(signal) {
      const out: RecordValue[] = [];
      let cursor: string | undefined;
      for (let page = 0; page < MAX_PAGES; page++) {
        let result: unknown;
        try {
          result = await wire.request('tools/list', cursor ? { cursor } : undefined, signal);
        } catch (error) {
          if (error instanceof McpFailure || (error as Error).name === 'AbortError') throw error;
          throw new McpFailure('list', oneLine(errorText(error)), wireOf(error));
        }
        const body = object(result);
        const list = Array.isArray(body?.tools) ? body.tools : [];
        for (const item of list) {
          const tool = object(item);
          if (tool) out.push(tool);
          if (out.length >= MAX_TOOLS) return out;
        }
        cursor = typeof body?.nextCursor === 'string' && body.nextCursor ? body.nextCursor : undefined;
        if (!cursor) break;
      }
      return out;
    },
    async call(name, args, signal) {
      try {
        return object(await wire.request('tools/call', { name, arguments: args }, signal)) ?? { content: [], isError: false };
      } catch (error) {
        if (error instanceof RpcError) return { content: [{ type: 'text', text: error.message }], isError: true };
        // 보낸 뒤 연결이 끊기거나 서버가 끝났다면(plain Error) 결과를 모른다. 서버가 명시한 HTTP 오류·로그인 실패(McpFailure)는 실패다.
        if (error instanceof McpFailure || error instanceof McpUnknownOutcome || (error as Error).name === 'AbortError') throw error;
        throw new McpUnknownOutcome(errorText(error));
      }
    },
    close: () => wire.close(),
  };
}

export async function connectMcp(launch: McpLaunch, signal: AbortSignal): Promise<McpSession> {
  if (signal.aborted) throw abortError();
  const transport = launch.transport;
  const wire = transport.kind === 'stdio' ? stdioWire(launch, transport.command, transport.args) : httpWire(launch, transport.url);
  const limit = AbortSignal.any([signal, AbortSignal.timeout(INIT_TIMEOUT)]);
  try {
    let result: unknown;
    let sent = NEW_PROTOCOL;
    const initialize = (version: string) => wire.request('initialize', {
      protocolVersion: version, capabilities: {}, clientInfo: { name: 'lair', version: '1' },
    }, limit);
    try {
      result = await initialize(sent);
    } catch (error) {
      if (!(error instanceof RpcError)) throw error;
      sent = OLD_PROTOCOL;
      result = await initialize(sent);
    }
    const negotiated = object(result)?.protocolVersion;
    wire.ready(typeof negotiated === 'string' && negotiated ? negotiated : sent);
    wire.notify('notifications/initialized');
    return makeSession(wire);
  } catch (error) {
    await wire.close();
    if (signal.aborted) throw abortError();
    if (error instanceof McpFailure) throw error;
    if (limit.aborted) throw new McpFailure('connect', `초기화 응답이 ${INIT_TIMEOUT / 1000}초 안에 오지 않았어.`, wireText('wireMcp.client.initTimeout', { seconds: INIT_TIMEOUT / 1000 }));
    if (error instanceof RpcError) {
      const args = { reason: oneLine(wireLocal(error.i18n, error.message, 'ko')), reasonEn: oneLine(wireLocal(error.i18n, error.message, 'en')) };
      throw new McpFailure('connect', oneLine(`초기화를 거절했어: ${error.message}`), wireText('wireMcp.client.initRejected', args));
    }
    throw new McpFailure('connect', oneLine(errorText(error)), wireOf(error));
  }
}

export async function checkMcp(launch: McpLaunch, timeoutMs = 15_000, outer?: AbortSignal): Promise<{ ok: boolean; stage: McpStage; message?: string; messageI18n?: WireText; tools: McpToolInfo[] }> {
  const controller = new AbortController();
  let timedOut = false;
  const timer = setTimeout(() => { timedOut = true; controller.abort(); }, timeoutMs);
  const onOuter = () => controller.abort();
  if (outer?.aborted) controller.abort(); else outer?.addEventListener('abort', onOuter, { once: true });
  let stage: McpStage = 'connect';
  let session: McpSession | undefined;
  try {
    session = await connectMcp(launch, controller.signal);
    stage = 'list';
    const raw = await session.tools(controller.signal);
    const tools = raw.map(toolInfo).filter((tool): tool is McpToolInfo => tool !== undefined);
    return { ok: true, stage: 'list', tools };
  } catch (error) {
    if (timedOut) return { ok: false, stage, message: `시간 초과: ${Math.round(timeoutMs / 1000)}초 안에 끝나지 않았어.`, messageI18n: wire('wireMcp.client.checkTimeout', { seconds: Math.round(timeoutMs / 1000) }), tools: [] };
    const i18n = wireOf(error);
    if (error instanceof McpFailure) return { ok: false, stage: error.stage, message: oneLine(error.message), ...(i18n ? { messageI18n: i18n } : {}), tools: [] };
    if ((error as Error).name === 'AbortError') return { ok: false, stage, message: '시험이 중단됐어.', messageI18n: wire('wireMcp.client.checkAborted'), tools: [] };
    return { ok: false, stage, message: oneLine(errorText(error)), ...(i18n ? { messageI18n: i18n } : {}), tools: [] };
  } finally {
    clearTimeout(timer);
    outer?.removeEventListener('abort', onOuter);
    if (session) await session.close().catch(() => undefined);
  }
}

// ───────────────────────── 도구 정보 ─────────────────────────

function sortedJson(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(sortedJson);
  const record = object(value);
  if (!record) return value;
  const out: RecordValue = {};
  for (const key of Object.keys(record).sort()) out[key] = sortedJson(record[key]);
  return out;
}

export function toolFingerprint(raw: Record<string, unknown>): string {
  const text = JSON.stringify(sortedJson({
    name: raw.name, description: raw.description, inputSchema: raw.inputSchema, annotations: raw.annotations,
  })) ?? '';
  return createHash('sha256').update(text).digest('hex').slice(0, 16);
}

/** 중계가 다루는 도구 이름: 1~200자, 제어 문자 없음. */
export function usableToolName(name: unknown): name is string {
  return typeof name === 'string' && name.length >= 1 && name.length <= 200 && !/[\u0000-\u001f\u007f-\u009f\u2028\u2029]/.test(name);
}

export function toolInfo(raw: Record<string, unknown>): McpToolInfo | undefined {
  if (!usableToolName(raw.name)) return undefined;
  const annotations = object(raw.annotations);
  return {
    name: raw.name,
    ...(typeof raw.title === 'string' && raw.title ? { title: raw.title } : {}),
    // 설명은 여기서 300자로 자르지 않는다. 저장하는 쪽이 비밀을 가린 뒤 자른다(자른 경계에 비밀 일부가 남지 않게, 고침 확인 1). 큰 값만 막는다.
    ...(typeof raw.description === 'string' ? { description: raw.description.slice(0, 65536) } : {}),
    readOnly: annotations?.readOnlyHint === true,
    ...(typeof annotations?.destructiveHint === 'boolean' ? { destructive: annotations.destructiveHint } : {}),
    fingerprint: toolFingerprint(raw),
  };
}
