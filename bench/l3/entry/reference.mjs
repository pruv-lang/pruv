// Pinned-function oracle of the L3-8 engine-import common entry (DESIGN section 6). It imports and runs the real pinned
// exports; nothing is reimplemented. Run as
//   node --experimental-strip-types --import ./reference-hooks.mjs reference.mjs < cases.jsonl > results.jsonl
// One case JSON line per stdin line (DESIGN 5.1), one result line per case: {id, terminal, value, trace, driver:null}.
// The fixture keys name, expect, probe and integration are ignored. B10 results also carry "callerTitles".
import { createInterface } from 'node:readline';
import { importSources, findSource, listExternal, readExternal } from './reference/backend-engine-import.ts';
import { accountHome } from './reference/backend-accounts.ts';
import { codexAccountFolder } from './reference/backend-codex-home.ts';
import { sessionThreads, excludedFromBound, boundSessionIndex } from './reference/backend-engine-import-bound.ts';
import { EngineImportError } from './reference/backend-engine-import-tail.ts';
import { WireError } from './fakes/wire.ts';
import {
  ABSENT, OracleError, accountsCallback, begin, currentCase, dec, end, enc, isTemporaryCallback, nameOf, put, refOf,
} from './reference-primitives.mjs';

if (process.version !== 'v26.4.0') {
  process.stderr.write(`reference.mjs needs Node v26.4.0, found ${process.version}\n`);
  process.exit(2);
}

const OPS = {
  1: 'import_sources', 2: 'find_source', 3: 'list_external', 4: 'read_external', 5: 'account_home', 6: 'codex_account_folder',
  7: 'session_threads', 8: 'excluded_from_bound', 9: 'bound_session_index',
  import_sources: 'import_sources', find_source: 'find_source', find: 'find_source', list_external: 'list_external', list: 'list_external',
  read_external: 'read_external', read: 'read_external', account_home: 'account_home', codex_account_folder: 'codex_account_folder',
  session_threads: 'session_threads', threads: 'session_threads', excluded_from_bound: 'excluded_from_bound', excluded: 'excluded_from_bound',
  bound_session_index: 'bound_session_index', bound: 'bound_session_index',
};

/** An oracle-side failure (reported as unexplained by the comparison). */
class Mismatch extends Error {}

function record(entries) {
  const out = {};
  for (const [key, jv] of Object.entries(entries ?? {})) { const value = dec(jv); if (value !== ABSENT) put(out, key, value); }
  return out;
}

/** The source object of ops 3 and 4: the sentinel of args.source.ref with engine and the extra fields assigned in listed order. */
function sourceOf(spec) {
  const { ref, ...fields } = spec;
  const source = refOf(ref);
  for (const [key, jv] of Object.entries(fields)) { const value = dec(jv); if (value !== ABSENT) put(source, key, value); }
  return source;
}

function verifyList(out, items) {
  if (!Array.isArray(out) || !Array.isArray(items) || out.length !== items.length) throw new Mismatch('list result is not the scripted rows');
  return out.map((got, i) => {
    const row = items[i];
    const name = nameOf(row);
    if (name === undefined) throw new Mismatch(`row ${i} has no ref name`);
    if (got === row) return ['keep', { ref: name }];
    if (got === null || typeof got !== 'object' || Object.getPrototypeOf(got) !== Object.prototype) throw new Mismatch(`row ${i} copy is not a plain object`);
    const left = Object.keys(got);
    const right = Object.keys(row);
    if (left.length !== right.length || left.some((key, k) => key !== right[k])) throw new Mismatch(`row ${i} copy has different keys or order`);
    for (const key of left) if (key !== 'title' && !Object.is(got[key], row[key])) throw new Mismatch(`row ${i} copy differs at ${key}`);
    return ['copy_title', { ref: name }, enc(got.title)];
  });
}

async function runOp(op, args) {
  switch (op) {
    case 'import_sources': {
      const sources = importSources({ dataDir: dec(args.dataDir), env: record(args.env), accounts: accountsCallback });
      return { terminal: 'ok', value: sources.map(entry => Object.fromEntries(Object.keys(entry).map(key => [key, enc(entry[key])]))) };
    }
    case 'find_source': {
      const sources = args.sources.map(item => record(item));
      const hit = findSource(sources, dec(args.query));
      const index = sources.indexOf(hit);
      return { terminal: 'ok', value: hit === undefined ? { undef: 1 } : index };
    }
    case 'list_external': {
      const source = sourceOf(args.source);
      const out = await listExternal(source, dec(args.folder), refOf(args.options.ref));
      return { terminal: 'ok', value: verifyList(out, [...currentCase().rows.values()].at(-1)) };
    }
    case 'read_external': {
      const source = sourceOf(args.source);
      const out = await readExternal(source, dec(args.folder), dec(args.thread), refOf(args.options.ref));
      const name = nameOf(out);
      if (name === undefined) throw new Mismatch('read result is not the scripted record');
      return { terminal: 'ok', value: { ref: name } };
    }
    case 'account_home':
      return { terminal: 'ok', value: enc(accountHome(dec(args.dataDir), dec(args.engine), dec(args.id))) };
    case 'codex_account_folder':
      return { terminal: 'ok', value: enc(codexAccountFolder(dec(args.dataDir), record(args.account))) };
    case 'session_threads':
      return { terminal: 'ok', value: [...sessionThreads(dec(args.session))].map(bound => [enc(bound.engine), enc(bound.thread)]) };
    case 'excluded_from_bound':
      return { terminal: 'ok', value: enc(excludedFromBound(dec(args.session), isTemporaryCallback)) };
    case 'bound_session_index': {
      const sessions = args.sessions.map(dec);
      return { terminal: 'ok', value: [...boundSessionIndex(sessions, isTemporaryCallback)].map(([key, id]) => [enc(key), enc(id)]) };
    }
    default: throw new OracleError(`unknown op ${op}`);
  }
}

/** Thrown value -> terminal and value (DESIGN section 6). */
function classify(error) {
  const name = nameOf(error);
  if (name !== undefined) return { terminal: 'throw_existing', value: { ref: name } };
  if (error instanceof Mismatch || error instanceof OracleError) return { terminal: 'native_throw', value: { detail: error.message } };
  if (error instanceof EngineImportError) {
    const hasCause = 'cause' in error;
    const value = { code: error.code, message: enc(error.message) };
    if (hasCause) { const named = nameOf(error.cause); value.cause = named !== undefined ? { ref: named } : enc(error.cause); }
    value.hasCause = hasCause;
    return { terminal: 'throw_import', value };
  }
  if (error instanceof WireError) return { terminal: 'throw_account_wire', value: { key: enc(error.key), message: enc(error.message) } };
  if (error instanceof Error) return { terminal: 'throw_plain_error', value: { message: enc(error.message) } };
  return { terminal: 'native_throw', value: { detail: `non-Error value thrown: ${typeof error}` } };
}

/** Verbatim copy of the title loop at requests/engine-import.ts:133-134 (deps.sessionList() is the case's sessions). */
function callerTitles(sessions) {
  const titles = new Map();
  for (const session of sessions) titles.set(session.id, session.title);
  return [...titles].map(([id, title]) => [enc(id), enc(title)]);
}

async function runCase(c) {
  const op = OPS[c.op];
  const state = begin(c);
  let outcome;
  let titles;
  try {
    if (op === undefined) throw new OracleError(`unknown op ${JSON.stringify(c.op)}`);
    const args = c.args ?? {};
    if (Array.isArray(args.sessions)) titles = callerTitles(args.sessions.map(dec));
    outcome = await runOp(op, args);
  } catch (error) {
    outcome = classify(error);
  } finally {
    end();
  }
  if (state.violations.length > 0) outcome = { terminal: 'native_throw', value: { detail: state.violations.join('; ') } };
  const result = { id: c.id, terminal: outcome.terminal, value: outcome.value, trace: state.trace, driver: null };
  if (titles !== undefined) result.callerTitles = titles;
  return result;
}

const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
for await (const line of lines) {
  if (line.trim() === '') continue;
  let c;
  try { c = JSON.parse(line); } catch (error) {
    process.stdout.write(JSON.stringify({ id: null, terminal: 'native_throw', value: { detail: `bad case line: ${error.message}` }, trace: [], driver: null }) + '\n');
    continue;
  }
  process.stdout.write(JSON.stringify(await runCase(c)) + '\n');
}
