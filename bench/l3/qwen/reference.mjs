// Real filesystem reference; subprocess and deferred plan dependencies are stubbed.
import './reference/hooks.mjs';
const {readQwen,readQwenHead,listQwen}=await import('./reference/qwen.ts');
const {tailInfo}=await import('./reference/tail-info.ts');
const {readEdgeLines}=await import('./reference/tail.ts');
const {readFile,stat}=await import('node:fs/promises');
const manifest=JSON.parse(await readFile(process.argv[2],'utf8'));
const options={signal:new AbortController().signal};
for(const item of manifest){
 const source={home:item.home};
 if(item.list){console.log(JSON.stringify(await listQwen(source,item.folder,options)));continue;}
 const size=(await stat(item.path)).size;
 const start=performance.now();
 let record;
 try { record=await readQwen(source,item.folder,item.thread,options); }
 catch(e) { if(e.code!=='not-found') throw e; record={error:e.code}; }
 if(item.measure){console.log(JSON.stringify({ms:performance.now()-start,turns:record.turns.length}));continue;}
 console.log(JSON.stringify({head:readQwenHead(await readEdgeLines(item.path,'head',size)),tail:tailInfo(await readEdgeLines(item.path,'tail',size)),record}));
}
