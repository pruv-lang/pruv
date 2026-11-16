// Q driver for the TS reference: one forked supervisor per case, as the Lair backend runs it.
// argv: cases.json results.json. Each case: {shell, args, cwd, out, limit, tail, mode, delay}.
// mode wait: only run; stop: send {type:'stop'} `delay` ms after `started`; close: disconnect `delay` ms after
// `started` (the exit message can then not arrive and is recorded as null).
import { fork } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const supervisor = join(here, 'backend', 'command-supervisor.ts');
const hooks = join(here, 'hooks.mjs');
const cases = JSON.parse(readFileSync(process.argv[2], 'utf8'));

function one(c) {
  return new Promise(resolve => {
    const child = fork(supervisor, [], { execArgv: ['--import', hooks], stdio: ['ignore', 'ignore', 'inherit', 'ipc'] });
    let exit = null;
    let started = false;
    const killer = setTimeout(() => child.kill('SIGKILL'), 60_000);
    child.on('message', m => {
      if (m.type === 'ready') {
        child.send({ type: 'run', shell: c.shell, args: c.args, cwd: c.cwd || process.cwd(), env: process.env,
          outputFile: c.out, fileLimit: c.limit, tailChars: c.tail });
      } else if (m.type === 'started') {
        started = true;
        if (c.mode === 'stop') setTimeout(() => child.connected && child.send({ type: 'stop' }), c.delay);
        if (c.mode === 'close') setTimeout(() => child.connected && child.disconnect(), c.delay);
      } else if (m.type === 'exit') {
        exit = m;
      }
    });
    child.on('exit', () => { clearTimeout(killer); resolve({ started, exit }); });
  });
}

const results = [];
for (const c of cases) results.push(await one(c));
writeFileSync(process.argv[3], JSON.stringify(results));
