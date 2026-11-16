// 바깥 기록에서 '마지막으로 기록된 계획'을 뽑는다(docs/notes/tool-migration-2026-11-03.md M4 설계 '계획 추출 규칙').
// 이 파일은 바깥 기능 코드를 import 하지 않는다. 타입과 상수만 쓴다.
import { LAST_PLAN_LIMITS, type LastPlan, type LastPlanItem } from '../../shared/engine-import';

type RecordValue = Record<string, unknown>;
type PlanStatus = NonNullable<LastPlanItem['status']>;

function plain(value: unknown): RecordValue | undefined {
  return value !== null && typeof value === 'object' && !Array.isArray(value) ? value as RecordValue : undefined;
}

function statusOf(value: unknown): PlanStatus | undefined {
  return value === 'pending' || value === 'in_progress' || value === 'completed' ? value : undefined;
}

/** 항목 목록에 상한을 걸어 LastPlan 으로. 글은 trim, 빈 글 항목은 뺀다. 항목이 없으면 undefined */
function fit(entries: Array<{ text: string; status?: PlanStatus }>, at: string | undefined): LastPlan | undefined {
  const items: LastPlanItem[] = [];
  for (const entry of entries) {
    const text = entry.text.trim();
    if (!text) continue;
    items.push({ text: text.length > LAST_PLAN_LIMITS.text ? text.slice(0, LAST_PLAN_LIMITS.text) : text, ...(entry.status ? { status: entry.status } : {}) });
  }
  if (items.length === 0) return undefined;
  const more = items.length - LAST_PLAN_LIMITS.items;
  return { ...(at ? { at } : {}), items: more > 0 ? items.slice(0, LAST_PLAN_LIMITS.items) : items, ...(more > 0 ? { more } : {}) };
}

interface Waiting { name: string; input: RecordValue }
interface Task { id?: string; text: string; status?: PlanStatus }

/**
 * Claude JSONL 한 줄마다 line 을 부르고 끝에 finish 를 부른다. 결과 줄이 있고 오류가 아닌 호출만 계획에 반영한다.
 * 시각은 마지막으로 반영한 결과 줄의 시각이다. isoOf 는 시각 판정(claude.ts 의 isoOf)을 받는다.
 */
export class ClaudePlanTracker {
  private readonly waiting = new Map<string, Waiting>();
  private tasks: Task[] = [];
  private broken = false;
  private at: string | undefined;

  constructor(private readonly isoOf: (value: unknown) => string | undefined) {}

  line(entry: RecordValue): void {
    if (entry.isSidechain === true) return;
    const content = plain(entry.message)?.content;
    if (!Array.isArray(content)) return;
    if (entry.type === 'assistant') {
      for (const block of content) {
        const item = plain(block);
        if (item?.type !== 'tool_use' || typeof item.id !== 'string') continue;
        if (item.name !== 'TodoWrite' && item.name !== 'TaskCreate' && item.name !== 'TaskUpdate') continue;
        this.waiting.set(item.id, { name: item.name, input: plain(item.input) ?? {} });
      }
    } else if (entry.type === 'user') {
      for (const block of content) {
        const item = plain(block);
        if (item?.type !== 'tool_result' || typeof item.tool_use_id !== 'string') continue;
        const call = this.waiting.get(item.tool_use_id);
        if (!call) continue;
        this.waiting.delete(item.tool_use_id);
        if (item.is_error === true) continue;
        this.apply(call, entry);
        const at = this.isoOf(entry.timestamp);
        if (at) this.at = at;
      }
    }
  }

  private apply(call: Waiting, entry: RecordValue): void {
    if (call.name === 'TodoWrite') {
      const todos = Array.isArray(call.input.todos) ? call.input.todos : [];
      this.tasks = [];
      this.broken = false;
      for (const raw of todos) {
        const todo = plain(raw);
        if (typeof todo?.content !== 'string') continue;
        const status = statusOf(todo.status);
        this.tasks.push({ text: todo.content, ...(status ? { status } : {}) });
      }
    } else if (call.name === 'TaskCreate') {
      const id = plain(plain(entry.toolUseResult)?.task)?.id;
      if (typeof id !== 'string' || !id) { this.broken = true; return; }
      const subject = typeof call.input.subject === 'string' && call.input.subject.trim() ? call.input.subject : call.input.description;
      this.tasks.push({ id, text: typeof subject === 'string' ? subject : '', status: 'pending' });
    } else {
      const id = call.input.taskId;
      const at = typeof id === 'string' ? this.tasks.findIndex(task => task.id === id) : -1;
      if (at < 0) { this.broken = true; return; }
      const status = call.input.status;
      if (status === 'deleted') { this.tasks.splice(at, 1); return; }
      const task = this.tasks[at];
      if (status !== undefined) {
        const known = statusOf(status);
        if (known) task.status = known;
        else delete task.status;
      }
      if (typeof call.input.subject === 'string') task.text = call.input.subject;
    }
  }

  finish(): LastPlan | undefined {
    if (this.broken || this.tasks.length === 0) return undefined;
    return fit(this.tasks, this.at);
  }
}

/**
 * OpenCode 내보내기의 메시지(옛것부터)에서 마지막 todowrite 도구 파트의 todos 를 계획으로 뽑는다.
 * 뒤 메시지부터, 한 메시지 안에서도 뒤 파트부터 보아 처음 맞는 것만 쓴다(비어 있으면 계획 없음). 오류로 끝난 호출은 건너뛴다.
 * at 은 호출한 쪽이 메시지 시각에서 구해 넘긴다.
 */
export function opencodePlan(messages: ReadonlyArray<{ parts: readonly unknown[]; at?: string }>): LastPlan | undefined {
  for (let index = messages.length - 1; index >= 0; index--) {
    const { parts, at } = messages[index];
    for (let partIndex = parts.length - 1; partIndex >= 0; partIndex--) {
      const part = plain(parts[partIndex]);
      const state = plain(part?.state);
      if (part?.type !== 'tool' || part.tool !== 'todowrite' || state?.status === 'error') continue;
      const todos = plain(state?.input)?.todos;
      if (!Array.isArray(todos)) continue;
      const entries: Array<{ text: string; status?: PlanStatus }> = [];
      for (const raw of todos) {
        const todo = plain(raw);
        if (typeof todo?.content !== 'string') continue;
        const status = statusOf(todo.status);
        entries.push({ text: todo.content, ...(status ? { status } : {}) });
      }
      return fit(entries, at);
    }
  }
  return undefined;
}

/**
 * Qwen 기록의 assistant 줄(옛것부터)에서 마지막 todo_write 호출(functionCall.name)의 args.todos 를 계획으로 뽑는다.
 * 뒤 줄부터, 한 줄 안에서도 뒤 파트부터 보아 처음 맞는 것만 쓴다. 실제 파트 모양은 못 봤으니(기본 도구 목록에 없음) 안 맞으면 계획 없음.
 */
export function qwenPlan(messages: ReadonlyArray<{ parts: readonly unknown[]; at?: string }>): LastPlan | undefined {
  for (let index = messages.length - 1; index >= 0; index--) {
    const { parts, at } = messages[index];
    for (let partIndex = parts.length - 1; partIndex >= 0; partIndex--) {
      const call = plain(plain(parts[partIndex])?.functionCall);
      if (call?.name !== 'todo_write') continue;
      const todos = plain(call.args)?.todos;
      if (!Array.isArray(todos)) continue;
      const entries: Array<{ text: string; status?: PlanStatus }> = [];
      for (const raw of todos) {
        const todo = plain(raw);
        if (typeof todo?.content !== 'string') continue;
        const status = statusOf(todo.status);
        entries.push({ text: todo.content, ...(status ? { status } : {}) });
      }
      return fit(entries, at);
    }
  }
  return undefined;
}

/** Codex 계획 글을 줄 단위 항목으로(상태 없음). 줄 앞뒤 공백과 맨 앞 목록 표지(- * 숫자.)를 뗀다. 항목이 없으면 undefined */
export function codexPlan(text: string, at: string | undefined): LastPlan | undefined {
  const entries = text.split(/\r?\n/).map(line => ({ text: line.trim().replace(/^(?:[-*]|\d+\.)\s+/, '').trim() }));
  return fit(entries, at);
}
