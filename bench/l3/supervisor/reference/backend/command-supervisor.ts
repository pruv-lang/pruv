// 명령 위임 감독 프로세스(docs/notes/claude-background-2026-10-01.md 4번). 백엔드가 detached fork 로 띄우고, 이 프로세스가 셸을 새 프로세스 그룹에서 띄운다.
// 백엔드와 잇는 IPC 파이프가 끊기면(앱 비정상 종료 포함) 그룹에 SIGTERM → 5초 → SIGKILL 을 보내고 끝난다. 셸이 끝난 뒤 남은 자식도 같은 사다리로 거둔다.
// 출력은 여기서 합쳐 파일(앞 64MB)에 쓰고 마지막 글자들을 따로 들고 있다가 끝에 알린다. node: 내장 외에는 node:path 만 쓰는 가벼운 windows-system.ts 만 가져오고(bundle: true 라 번들에 함께 들어간다), 나머지 백엔드 파일은 가져오지 않는 독립 번들이다(codex-account-watcher.ts 와 같은 방식).
import { execFile, spawn, spawnSync, type ChildProcess } from 'node:child_process';
import { closeSync, openSync, readFileSync, renameSync, rmSync, writeFileSync, writeSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { StringDecoder } from 'node:string_decoder';
import type { WireText } from '../shared/i18n/wire';
import { systemTaskkillPath } from './windows-system';

const KILL_AFTER_MS = 5_000;
const REAP_WAIT_MS = 3_000;
const STREAM_END_WAIT_MS = 2_000;
const HOLD_STOP_WAIT_MS = 12_000;
const windows = process.platform === 'win32';

interface RunMessage {
  type: 'run';
  shell: string;
  args: string[];
  cwd: string;
  env: NodeJS.ProcessEnv;
  outputFile: string;
  fileLimit: number;
  tailChars: number;
  /** 있으면 끝날 때 출력 파일 옆 결과 파일을 쓴다. 셸 환경에는 넣지 않는다. */
  delegationId?: string;
  attempt?: number;
}

let child: ChildProcess | undefined;
let finished = false;
/** 사다리가 돌고 있으면 그 약속. 같은 사다리를 두 번 돌리지 않는다. */
let ladder: Promise<boolean> | undefined;
let stopReason: 'stop' | 'disconnect' | undefined;
/** 윈도우: 작업 개체를 쥔 도우미 프로세스(child 와 같다)의 결과 파일 경로. */
let holdStatusFile: string | undefined;

/**
 * 윈도우 작업 개체 도우미(PowerShell 5.1). 작업 개체(KILL_ON_JOB_CLOSE)를 만들어 자신을 넣고 그 안에서 셸을 띄운다(자손은 모두 작업 개체에 든다).
 * 셸이 끝나거나 표준 입력이 끊기면(감독이 멈추라 했거나 죽음) 작업 개체에 남은 프로세스를 하나씩 끝내 자신만 남을 때까지 확인하고 결과 파일에 적는다.
 * 도우미가 죽어도 작업 개체 핸들이 닫혀 OS 가 남은 프로세스를 끝낸다. 셸의 표준 출력·오류는 도우미가 물려받아 그대로 감독 파이프로 간다.
 */
const HOLD_CSHARP = [
  'using System; using System.Runtime.InteropServices;',
  'public static class LairHoldJob {',
  '  [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern IntPtr CreateJobObjectW(IntPtr a, string n);',
  '  [DllImport("kernel32.dll", SetLastError = true)] static extern bool SetInformationJobObject(IntPtr j, int c, IntPtr i, uint l);',
  '  [DllImport("kernel32.dll", SetLastError = true)] static extern bool AssignProcessToJobObject(IntPtr j, IntPtr p);',
  '  [DllImport("kernel32.dll", SetLastError = true)] static extern bool QueryInformationJobObject(IntPtr j, int c, IntPtr i, uint l, out uint r);',
  '  [DllImport("kernel32.dll")] static extern IntPtr GetCurrentProcess();',
  '  [DllImport("kernel32.dll", SetLastError = true)] static extern IntPtr OpenProcess(uint a, bool i, int id);',
  '  [DllImport("kernel32.dll", SetLastError = true)] static extern bool IsProcessInJob(IntPtr p, IntPtr j, out bool r);',
  '  [DllImport("kernel32.dll", SetLastError = true)] static extern bool TerminateProcess(IntPtr p, uint c);',
  '  [DllImport("kernel32.dll", SetLastError = true)] static extern bool CloseHandle(IntPtr h);',
  '  static IntPtr job = IntPtr.Zero;',
  '  public static int Assigned = 0;',
  '  public static bool Create() {',
  '    IntPtr h = CreateJobObjectW(IntPtr.Zero, null);',
  '    if (h == IntPtr.Zero) return false;',
  '    int size = IntPtr.Size == 8 ? 144 : 112;',
  '    IntPtr p = Marshal.AllocHGlobal(size);',
  '    try {',
  '      for (int i = 0; i < size; i++) Marshal.WriteByte(p, i, 0);',
  '      Marshal.WriteInt32(p, 16, 0x2000);',
  '      if (!SetInformationJobObject(h, 9, p, (uint)size)) return false;',
  '    } finally { Marshal.FreeHGlobal(p); }',
  '    if (!AssignProcessToJobObject(h, GetCurrentProcess())) return false;',
  '    job = h; return true;',
  '  }',
  // 목록의 번호는 그새 다른 프로세스에 다시 쓰일 수 있다. 연 핸들이 지금도 이 작업 개체 소속일 때만 끝낸다(PID 재사용 오종료 방지).
  '  public static void KillMember(int id) {',
  '    IntPtr h = OpenProcess(0x0001 | 0x1000, false, id);',
  '    if (h == IntPtr.Zero) return;',
  '    try { bool inJob; if (IsProcessInJob(h, job, out inJob) && inJob) TerminateProcess(h, 1); } finally { CloseHandle(h); }',
  '  }',
  '  public static int[] Pids() {',
  '    int cap = 1024; int size = 8 + IntPtr.Size * cap;',
  '    IntPtr p = Marshal.AllocHGlobal(size);',
  '    try {',
  '      for (int i = 0; i < size; i++) Marshal.WriteByte(p, i, 0);',
  '      uint r;',
  '      bool ok = QueryInformationJobObject(job, 3, p, (uint)size, out r);',
  '      if (!ok && Marshal.GetLastWin32Error() != 234) return null;',
  '      Assigned = Marshal.ReadInt32(p, 0);',
  '      int n = Marshal.ReadInt32(p, 4);',
  '      int[] ids = new int[n];',
  '      for (int i = 0; i < n; i++) ids[i] = (int)Marshal.ReadIntPtr(p, 8 + IntPtr.Size * i).ToInt64();',
  '      return ids;',
  '    } finally { Marshal.FreeHGlobal(p); }',
  '  }',
  '}',
].join('\n');

const HOLD_SCRIPT = [
  "$ErrorActionPreference = 'Stop'",
  "$ProgressPreference = 'SilentlyContinue'",
  '$statusPath = $env:LAIR_HOLD_STATUS',
  'function Report($o) { try { [System.IO.File]::WriteAllText($statusPath, ($o | ConvertTo-Json -Compress)) } catch { } }',
  'function Quote($a) { if ($a -eq "") { return \'""\' }; if ($a -notmatch \'[\\s"]\') { return $a }; $s = [regex]::Replace($a, \'(\\\\*)"\', \'$1$1\\"\'); $s = [regex]::Replace($s, \'(\\\\+)$\', \'$1$1\'); return \'"\' + $s + \'"\' }',
  'try {',
  '  $shell = $env:LAIR_HOLD_SHELL; $cwd = $env:LAIR_HOLD_CWD',
  '  $argText = [System.Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($env:LAIR_HOLD_ARGS))',
  '  $csText = [System.Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($env:LAIR_HOLD_CS))',
  "  foreach ($n in 'LAIR_HOLD_SHELL','LAIR_HOLD_CWD','LAIR_HOLD_ARGS','LAIR_HOLD_CS','LAIR_HOLD_STATUS') { Remove-Item ('env:' + $n) -ErrorAction SilentlyContinue }",
  '  Add-Type -TypeDefinition $csText',
  "  if (-not [LairHoldJob]::Create()) { throw 'job object' }",
  '  $argList = ConvertFrom-Json $argText',
  '  $psi = New-Object System.Diagnostics.ProcessStartInfo',
  '  $psi.FileName = $shell; $psi.WorkingDirectory = $cwd; $psi.UseShellExecute = $false; $psi.RedirectStandardInput = $true; $psi.CreateNoWindow = $true',
  "  $psi.Arguments = ((@($argList) | ForEach-Object { Quote $_ }) -join ' ')",
  '  $p = [System.Diagnostics.Process]::Start($psi)',
  '  $p.StandardInput.Close()',
  '  $buf = New-Object byte[] 1',
  '  $watch = [Console]::OpenStandardInput().ReadAsync($buf, 0, 1)',
  '  while (-not $p.WaitForExit(100)) { if ($watch.IsCompleted) { break } }',
  '  $code = $null; if ($p.HasExited) { $code = $p.ExitCode }',
  '  $self = [System.Diagnostics.Process]::GetCurrentProcess().Id',
  '  $clean = $false; $deadline = (Get-Date).AddSeconds(8)',
  '  while ((Get-Date) -lt $deadline) {',
  '    $ids = [LairHoldJob]::Pids()',
  '    if ($null -eq $ids) { break }',
  '    $others = @($ids | Where-Object { $_ -ne $self })',
  '    if ([LairHoldJob]::Assigned -le 1 -and $others.Count -eq 0) { $clean = $true; break }',
  '    foreach ($id in $others) { try { [LairHoldJob]::KillMember($id) } catch { } }',
  '    Start-Sleep -Milliseconds 100',
  '  }',
  '  Report @{ code = $code; clean = $clean }',
  '  exit 0',
  '} catch {',
  '  Report @{ error = $_.ToString() }',
  '  exit 1',
  '}',
].join('\n');

/** 번역 메타 인자에 싣는 외부 글(OS 오류 등)의 길이 한도. 원문은 그대로 두고 메타만 자른다. */
const META_DETAIL_CHARS = 1000;
const metaDetail = (value: string): string => value.slice(0, META_DETAIL_CHARS);
/** 부분 쓰기에서 쓰인 바이트가 없을 때 던지는 표지. 메시지 비교 없이 같은 객체인지로 가른다. */
const NO_BYTES_WRITTEN = new Error('쓰인 바이트가 없어');

function delay(ms: number): Promise<void> { return new Promise(resolve => setTimeout(resolve, ms)); }

/** 셸 그룹에 아직 살아 있는 프로세스가 있는가. 윈도우는 작업 개체로 따로 확인해 여기서는 쓰지 않는다. */
function groupAlive(pid: number): boolean {
  if (windows) return false;
  try { process.kill(-pid, 0); return true; } catch (error) { return (error as NodeJS.ErrnoException).code === 'EPERM'; }
}

/** 그룹(윈도우는 작업 개체)을 SIGTERM → 5초 → SIGKILL 로 끝내고 비었는지 돌려준다. */
function killLadder(pid: number): Promise<boolean> {
  ladder ??= (async () => {
    if (windows) {
      // 도우미의 표준 입력을 닫으면 도우미가 작업 개체에 남은 프로세스를 끝내고 결과를 적고 끝난다. 끝나지 않으면 도우미를 끝내 작업 개체 핸들을 닫는다(OS 가 남은 것을 끝낸다).
      try { child?.stdin?.end(); } catch { /* 이미 닫혔다 */ }
      const until = Date.now() + HOLD_STOP_WAIT_MS;
      while (Date.now() < until && child && child.exitCode === null && child.signalCode === null) await delay(100);
      if (child && child.exitCode === null && child.signalCode === null) {
        spawnSync(systemTaskkillPath(), ['/PID', String(pid), '/T', '/F'], { windowsHide: true, stdio: 'ignore' });
        return false;
      }
      return readHold()?.clean === true;
    }
    if (!groupAlive(pid)) return true;
    try { process.kill(-pid, 'SIGTERM'); } catch { /* 이미 없다 */ }
    const until = Date.now() + KILL_AFTER_MS;
    while (Date.now() < until && groupAlive(pid)) await delay(100);
    if (groupAlive(pid)) {
      try { process.kill(-pid, 'SIGKILL'); } catch { /* 이미 없다 */ }
      const reapUntil = Date.now() + REAP_WAIT_MS;
      while (Date.now() < reapUntil && groupAlive(pid)) await delay(50);
    }
    return !groupAlive(pid);
  })();
  return ladder;
}

/** 윈도우 도우미가 적은 결과(셸 종료 코드·남은 것 0 확인·오류). 없으면 undefined. */
function readHold(): { code?: number | null; clean?: boolean; error?: string } | undefined {
  if (!holdStatusFile) return undefined;
  try { return JSON.parse(readFileSync(holdStatusFile, 'utf8').replace(/^\uFEFF/, '')); } catch { return undefined; }
}

/**
 * 프로세스 시작 시각 표지(process-tree.ts captureStartId 와 같은 형식이라 재시작 뒤 그 값과 비교한다). 이 번들은 process-tree.ts 를 가져오지 않아 같은 방법을 여기에 둔다.
 * 맥은 ps lstart, 리눅스는 /proc/<pid>/stat 의 시작 틱, Windows 는 PowerShell StartTime 의 UTC 틱. 얻지 못하면 null.
 * Windows 는 powershell(run 메시지의 셸, command-run.ts chooseShell 이 준 시스템 PowerShell 절대 경로)로 조회한다. PATH 를 찾지 않는다(B-233).
 */
function readStartId(pid: number, powershell: string): Promise<string | null> {
  if (process.platform === 'linux') {
    try {
      const stat = readFileSync(`/proc/${pid}/stat`, 'utf8');
      const end = stat.lastIndexOf(')');
      return Promise.resolve(end < 0 ? null : stat.slice(end + 2).split(' ')[19] ?? null);
    } catch { return Promise.resolve(null); }
  }
  return new Promise(resolve => {
    try {
      const done = (error: Error | null, stdout: string) => { const value = String(stdout).trim(); resolve(error || !value || (windows && !/^\d+$/.test(value)) ? null : value); };
      if (windows) execFile(powershell, ['-NoProfile', '-NonInteractive', '-Command', `(Get-Process -Id ${Math.trunc(pid)} -ErrorAction Stop).StartTime.ToUniversalTime().Ticks`],
        { encoding: 'utf8', timeout: 5_000, windowsHide: true }, done);
      else execFile('ps', ['-p', String(pid), '-o', 'lstart='], { encoding: 'utf8', timeout: 2_000, killSignal: 'SIGKILL' }, done);
    } catch { resolve(null); }
  });
}

/**
 * 끝날 때 출력 파일 옆(<출력파일>.result.json)에 결과를 임시 파일 → rename 으로 쓴다(0o600). 위임 번호가 없으면 쓰지 않는다.
 * 쓰지 못하면 그 이유를 돌려주고 임시 파일은 치운다(백엔드에 resultFileError 로 알린다).
 */
function writeResultFile(message: RunMessage, result: { code: number | null; signal: string | null; reaped: boolean; truncated: boolean; tail: string; fileError: string | undefined; fileErrorI18n?: WireText }): string | undefined {
  if (message.delegationId === undefined) return undefined;
  const target = `${message.outputFile}.result.json`;
  const temporary = `${target}.${process.pid}.tmp`;
  try {
    const body = { delegationId: message.delegationId, attempt: message.attempt ?? null, code: result.code, signal: result.signal, reaped: result.reaped, clean: result.reaped,
      truncated: result.truncated, tail: result.tail, fileError: result.fileError ?? null, ...(result.fileError !== undefined && result.fileErrorI18n ? { fileErrorI18n: result.fileErrorI18n } : {}),
      finishedAt: new Date().toISOString(), stopReason: stopReason ?? null };
    writeFileSync(temporary, JSON.stringify(body), { mode: 0o600 });
    renameSync(temporary, target);
    return undefined;
  } catch (error) {
    try { rmSync(temporary, { force: true }); } catch { /* 임시 파일 */ }
    return `결과 파일을 쓰지 못했어: ${error instanceof Error ? error.message : String(error)}`;
  }
}

function send(message: Record<string, unknown>): Promise<void> {
  return new Promise(resolve => {
    if (!process.connected || !process.send) { resolve(); return; }
    try { process.send(message, () => resolve()); } catch { resolve(); }
  });
}

function run(message: RunMessage): void {
  if (child) return;
  let fd: number | undefined;
  let fileError: string | undefined;
  let fileErrorI18n: WireText | undefined;
  try { fd = openSync(message.outputFile, 'w', 0o600); }
  catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    fileError = `출력 파일을 만들지 못했어: ${detail}`;
    fileErrorI18n = { key: 'wireWork.command.outputCreateFailed', args: { detail: metaDetail(detail) } };
  }
  let written = 0;
  let truncated = false;
  let tail = '';
  let total = 0;
  const decoders = [new StringDecoder('utf8'), new StringDecoder('utf8')];

  const take = (chunk: Buffer, index: number) => {
    total += chunk.length;
    // 파일: 상한을 넘으면 더 쓰지 않고 표시만 남긴다. 파이프는 끝까지 읽는다.
    if (fd !== undefined && !fileError && !truncated) {
      const room = message.fileLimit - written;
      const part = chunk.length <= room ? chunk : chunk.subarray(0, Math.max(0, room));
      try {
        // 부분 쓰기는 남은 바이트를 이어 쓰고, 실제로 쓰인 바이트만 센다.
        let offset = 0;
        while (offset < part.length) { const n = writeSync(fd, part, offset, part.length - offset); if (n <= 0) throw NO_BYTES_WRITTEN; offset += n; written += n; }
        if (chunk.length > room) truncated = true;
      } catch (error) {
        const detail = error instanceof Error ? error.message : String(error);
        fileError = `출력 파일에 쓰지 못했어: ${detail}`;
        // 감독이 만든 '쓰인 바이트가 없어'만 두 언어 완성 글을 준다. OS 오류 원문은 두 언어 인자에 그대로 간다.
        const args = error === NO_BYTES_WRITTEN ? { detail: '쓰인 바이트가 없습니다.', detailEn: 'No bytes were written.' } : { detail: metaDetail(detail), detailEn: metaDetail(detail) };
        fileErrorI18n = { key: 'wireWork.command.outputWriteFailed', args };
      }
    }
    tail += decoders[index].write(chunk);
    if (tail.length > message.tailChars * 2) tail = tail.slice(-message.tailChars);
  };

  let spawnError: string | undefined;
  try {
    if (windows) {
      // 셸은 작업 개체 도우미 안에서 뜬다. 도우미가 셸 뒤 정리와 감독 죽음 대응을 맡는다.
      holdStatusFile = join(tmpdir(), `lair-cmd-${process.pid}-${Date.now()}.json`);
      const env = { ...message.env, LAIR_HOLD_SHELL: message.shell, LAIR_HOLD_CWD: message.cwd, LAIR_HOLD_STATUS: holdStatusFile,
        LAIR_HOLD_ARGS: Buffer.from(JSON.stringify(message.args), 'utf8').toString('base64'), LAIR_HOLD_CS: Buffer.from(HOLD_CSHARP, 'utf8').toString('base64') };
      child = spawn(message.shell, ['-NoProfile', '-NonInteractive', '-EncodedCommand', Buffer.from(HOLD_SCRIPT, 'utf16le').toString('base64')],
        { cwd: message.cwd, env, stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true, shell: false });
    } else {
      child = spawn(message.shell, message.args, { cwd: message.cwd, env: message.env, stdio: ['ignore', 'pipe', 'pipe'], detached: true, windowsHide: true, shell: false });
    }
  } catch (error) { spawnError = error instanceof Error ? error.message : String(error); }
  if (!child || spawnError) {
    // OS 오류 원문(spawnError)에는 번역 메타를 붙이지 않는다. 감독이 만든 고정 문장일 때만 준다.
    void report({ spawnError: spawnError ?? '셸을 띄우지 못했어.', ...(spawnError ? {} : { spawnErrorI18n: { key: 'wireWork.command.shellStartFailed' } satisfies WireText }) });
    return;
  }
  const shell = child;
  shell.stdout?.on('data', (chunk: Buffer) => take(chunk, 0));
  shell.stderr?.on('data', (chunk: Buffer) => take(chunk, 1));
  shell.once('error', error => { if (!shell.pid) void report({ spawnError: error.message }); });
  // 시작 알림: 셸 pid·시작 시각 표지·감독 pid. 시각 조회가 걸려도 끝 보고(report)가 이 알림 뒤에 가도록 약속을 들고 있는다. 조회는 짧은 제한 시간이 있다.
  const startedNotice: Promise<void> = shell.pid
    ? readStartId(shell.pid, message.shell).then(startedAt => send({ type: 'started', pid: shell.pid, startedAt, supervisorPid: process.pid }))
    : Promise.resolve();

  let streamsClosed = false;
  shell.once('close', () => { streamsClosed = true; });
  shell.once('exit', (code, signal) => {
    void (async () => {
      const pid = shell.pid!;
      // 맥·리눅스: 셸이 끝나도 그룹에 남은 자식이 있으면 같은 사다리로 거둔다(성공·실패 모두). 그룹을 새로 만들어 빠져나간 자손은 보장하지 않는다.
      // 윈도우: 도우미가 이미 작업 개체에 남은 프로세스를 끝내 0 인지 확인하고 끝났다. 결과 파일이 없거나 오류면 확인하지 못한 것으로 둔다(죽은 셸 PID 로 taskkill 하지 않는다).
      const hold = windows ? readHold() : undefined;
      const reaped = windows ? hold?.clean === true : await killLadder(pid);
      if (windows && holdStatusFile) { try { rmSync(holdStatusFile, { force: true }); } catch { /* 임시 파일 */ } }
      // 셸이 이미 돌았을 수 있다: spawnError 문구는 유지하되 unconfirmed 로 알려 백엔드가 실행 전 실패로 착각하지 않게 한다.
      if (windows && hold?.error !== undefined) {
        await report({ spawnError: `작업 개체 도우미 오류: ${hold.error}`, spawnErrorI18n: { key: 'wireWork.command.holdHelperError', args: { detail: metaDetail(String(hold.error)) } } satisfies WireText, unconfirmed: true });
        return;
      }
      const until = Date.now() + STREAM_END_WAIT_MS;
      while (!streamsClosed && Date.now() < until) await delay(50);
      for (const decoder of decoders) tail += decoder.end();
      if (fd !== undefined) { try { closeSync(fd); } catch { /* 이미 닫혔다 */ } }
      const finalCode = windows ? (typeof hold?.code === 'number' ? hold.code : null) : code;
      const finalTail = tail.slice(-message.tailChars);
      const resultFileError = writeResultFile(message, { code: finalCode, signal, reaped, truncated, tail: finalTail, fileError, fileErrorI18n });
      await startedNotice;
      await report({ code: finalCode, signal, tail: finalTail, truncated, fileError, ...(fileError !== undefined && fileErrorI18n ? { fileErrorI18n } : {}), bytes: total, reaped,
        ...(resultFileError === undefined ? {} : { resultFileError }), ...(fd === undefined ? {} : { fileBytes: written }) });
    })();
  });
}

async function report(result: Record<string, unknown>): Promise<void> {
  if (finished) return;
  finished = true;
  await send({ type: 'exit', stopped: stopReason, ...result });
  process.exit(0);
}

/** 백엔드가 끊기거나 멈추라고 하면 그룹을 거두고 알린다. 셸이 없거나 이미 끝났으면 바로 끝낸다. */
function stop(reason: 'stop' | 'disconnect'): void {
  stopReason ??= reason;
  const pid = child?.pid;
  if (!pid || child!.exitCode !== null || child!.signalCode !== null) {
    // 셸이 끝났으면 exit 처리가 이미 사다리를 돌린다. 띄우기 전이면 바로 끝낸다.
    if (!child) process.exit(0);
    return;
  }
  void killLadder(pid).then(() => {
    // 셸 exit 이벤트가 보고한다. 보고가 오지 않는 경우를 위해 한 번 더 기다렸다 끝낸다.
    setTimeout(() => process.exit(0), STREAM_END_WAIT_MS + 1000).unref();
  });
}

process.on('message', (message: { type?: string }) => {
  if (message?.type === 'run') run(message as RunMessage);
  else if (message?.type === 'stop') stop('stop');
});
// 백엔드와 잇는 파이프가 끊겼다(앱 비정상 종료 포함). 남은 셸 그룹을 거두고 끝낸다.
process.on('disconnect', () => stop('disconnect'));
process.on('SIGTERM', () => stop('stop'));
process.on('SIGHUP', () => stop('stop'));
process.on('SIGINT', () => stop('stop'));
void send({ type: 'ready' });
