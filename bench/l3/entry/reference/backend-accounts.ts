// 추가 계정의 번호·이름과 계정 폴더(docs/notes/accounts-2026-09-26.md 3·4·7번). 계정 폴더는 저장하지 않고 엔진과 번호로 계산한다.
import { randomBytes } from 'node:crypto';
import { chmod, lstat, mkdir, realpath, rm } from 'node:fs/promises';
import { join, sep } from 'node:path';
import { ENGINE_IDS, type EngineAccount, type EngineId, type EngineLoginSetting } from '../shared/contracts';
import { wire, WireError, type WireText } from '../shared/i18n/wire';

/** 엔진마다 둘 수 있는 추가 계정 수(3번) */
export const MAX_ACCOUNTS = 8;
const ID = /^acc_[a-f\d]{16}$/;

export function newAccountId(): string { return `acc_${randomBytes(8).toString('hex')}`; }
export function validAccountId(value: unknown): value is string { return typeof value === 'string' && ID.test(value); }

/** 계정 이름을 다듬는다. 앞뒤 공백을 빼고 1~40자, 줄바꿈·제어 문자가 없어야 한다. 못 쓰면 null이다. */
export function accountName(value: unknown): string | null {
  if (typeof value !== 'string') return null;
  const name = value.trim();
  if (!name || [...name].length > 40 || /[\u0000-\u001f\u007f-\u009f\u2028\u2029]/.test(name)) return null;
  return name;
}

/** 엔진 로그인 설정을 읽는다. 모양이 틀린 칸은 버리고 이름은 계정 이름과 같은 규칙으로 다듬는다. 계정과의 이름 충돌은 읽을 때 버리지 않는다(docs/notes/settings-accounts-2026-10-08.md 3번). */
export function readEngineLogins(value: unknown): Partial<Record<EngineId, EngineLoginSetting>> {
  const out: Partial<Record<EngineId, EngineLoginSetting>> = {};
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return out;
  for (const engine of ENGINE_IDS) {
    const item = (value as Record<string, unknown>)[engine];
    if (typeof item !== 'object' || item === null || Array.isArray(item)) continue;
    const name = accountName((item as Record<string, unknown>).name);
    const removed = (item as Record<string, unknown>).removed === true;
    const creditUse = (item as Record<string, unknown>).creditUse === true;
    if (name || removed || creditUse) out[engine] = { ...(name ? { name } : {}), ...(removed ? { removed: true as const } : {}), ...(creditUse ? { creditUse: true as const } : {}) };
  }
  return out;
}

/**
 * 계정·엔진 로그인 별칭 이름 규칙 하나(앞뒤 공백 제거, 1~40자, 제어 문자 금지, 같은 엔진 안에서 추가 계정 이름과 엔진 로그인 별칭을
 * 대소문자 무시로 중복 금지, 제거한 엔진 로그인의 별칭도 센다). 충돌은 쓰기 때 막는다(docs/notes/settings-accounts-2026-10-08.md 3번).
 * except 는 이름을 바꾸는 그 대상 자신이다.
 */
export function checkAccountName(engine: EngineId, raw: unknown, accounts: readonly EngineAccount[], engineLogins: Partial<Record<EngineId, EngineLoginSetting>> | undefined,
  except: { accountId?: string; engineLogin?: true } = {}): { name: string } | { error: string; errorI18n: WireText } {
  const name = accountName(raw);
  if (!name) return { error: '계정 이름은 앞뒤 공백을 빼고 1~40자여야 하고 줄바꿈·제어 문자는 쓸 수 없습니다.', errorI18n: wire('wireEngine.account.nameShape') };
  const key = name.toLowerCase();
  if (accounts.some(account => account.engine === engine && account.id !== except.accountId && account.name.toLowerCase() === key)
    || (!except.engineLogin && engineLogins?.[engine]?.name?.toLowerCase() === key)) return { error: '같은 엔진에 같은 이름의 계정이 있습니다.', errorI18n: wire('wireEngine.account.nameDuplicate') };
  return { name };
}

/** 추가 계정 폴더. 번호 모양이 틀리면 경로를 만들지 않는다. */
export function accountHome(dataDir: string, engine: EngineId, id: string): string {
  if (!validAccountId(id)) throw new WireError(wire('wireEngine.account.invalidId'), '계정 번호 모양이 틀려.');
  return join(dataDir, 'engine-homes', engine, id);
}

/** 계정 폴더를 만들고(0700) 기호 링크나 폴더가 아닌 것이면 쓰지 않는다. */
export async function ensureAccountHome(dataDir: string, engine: EngineId, id: string): Promise<string> {
  const home = accountHome(dataDir, engine, id);
  for (const folder of [join(dataDir, 'engine-homes'), join(dataDir, 'engine-homes', engine)]) {
    await mkdir(folder, { recursive: true });
    const info = await lstat(folder);
    if (info.isSymbolicLink() || !info.isDirectory()) throw new WireError(wire('wireEngine.account.folder.engineNotFolder'), '엔진 계정 폴더 자리가 폴더가 아니야. 앱 데이터 폴더를 확인해 줘.');
  }
  await mkdir(home, { recursive: true, mode: 0o700 });
  const info = await lstat(home);
  if (info.isSymbolicLink() || !info.isDirectory()) throw new WireError(wire('wireEngine.account.folder.notFolder'), '계정 폴더 자리가 폴더가 아니야. 앱 데이터 폴더를 확인해 줘.');
  if (process.platform !== 'win32') await chmod(home, 0o700);
  return home;
}

function underRoot(root: string, path: string): boolean {
  const prefix = root.endsWith(sep) ? root : root + sep;
  return path === root || path.startsWith(prefix);
}

/** 계정 폴더 하나만 지운다. 안의 링크는 따라가지 않고 링크 자체만 지운다(7번). 없으면 그대로 끝난다. */
export async function removeAccountHome(dataDir: string, engine: EngineId, id: string, beforeRemove?: () => void): Promise<void> {
  const home = accountHome(dataDir, engine, id);
  let info;
  try { info = await lstat(home); }
  catch (error) { if ((error as NodeJS.ErrnoException).code === 'ENOENT') return; throw error; }
  if (info.isSymbolicLink() || !info.isDirectory()) throw new WireError(wire('wireEngine.account.folder.removeNotFolder'), '계정 폴더 자리가 폴더가 아니라 지우지 않았어.');
  let dataDirInfo;
  try { dataDirInfo = await lstat(dataDir); }
  catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') throw new WireError(wire('wireEngine.account.folder.dataDirMissing'), '앱 데이터 폴더를 찾지 못해 지우지 않았어.');
    throw error;
  }
  if (dataDirInfo.isSymbolicLink() || !dataDirInfo.isDirectory()) throw new WireError(wire('wireEngine.account.folder.dataDirNotFolder'), '앱 데이터 폴더 자리가 폴더가 아니라 지우지 않았어.');
  const dataRoot = await realpath(dataDir);
  for (const parent of [join(dataDir, 'engine-homes'), join(dataDir, 'engine-homes', engine)]) {
    try {
      const parentInfo = await lstat(parent);
      if (parentInfo.isSymbolicLink() || !parentInfo.isDirectory()) throw new WireError(wire('wireEngine.account.folder.removeEngineNotFolder'), '엔진 계정 폴더 자리가 폴더가 아니라 지우지 않았어.');
      if (!underRoot(dataRoot, await realpath(parent))) throw new WireError(wire('wireEngine.account.folder.engineOutside'), '엔진 계정 폴더가 앱 데이터 폴더 밖에 있어 지우지 않았어.');
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') continue;
      throw error;
    }
  }
  const resolvedHome = await realpath(home);
  if (!underRoot(dataRoot, resolvedHome)) throw new WireError(wire('wireEngine.account.folder.outside'), '계정 폴더가 앱 데이터 폴더 밖에 있어 지우지 않았어.');
  beforeRemove?.();
  await rm(home, { recursive: true, force: true });
}
