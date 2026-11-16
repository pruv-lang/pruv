import './reference/hooks.mjs';
import { createInterface } from 'node:readline';
const { checkMcp } = await import('./reference/backend/mcp-client.ts');
for await (const line of createInterface({input: process.stdin})) {
 const v = JSON.parse(line);
 const result = await checkMcp({transport:{kind:'stdio',command:v.command,args:v.args},
 cwd:v.cwd || '', env:{}, headers:{}, baseEnv:process.env}, v.timeout ?? 15000);
 delete result.messageI18n; // No localization metadata in the packet interface.
 console.log(JSON.stringify(result));
}
