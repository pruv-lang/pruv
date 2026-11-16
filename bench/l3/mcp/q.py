"""Real-child differential, four pairs; only total wall time is measured."""
import sys
sys.dont_write_bytecode=True
import collections,json,os,subprocess,time
from pathlib import Path
from oracle import classify
HERE=Path(__file__).resolve().parent;WORK=HERE/'work'
os.environ['MCP_NODE']=os.environ.get('MCP_NODE','/opt/homebrew/bin/node')
os.environ['MCP_FINGERPRINT_SCRIPT']=str(HERE/'fingerprint.cjs')
os.environ['TMPDIR']=str(WORK)
FIXED='normal pages many_pages ping fallback reject exit long hang missing bad_names noise cap cut flags fingerprint numeric_id list_hang list_error late_exit flood stderr_space'.split()
def cases(n=2000):
 out=[]
 for name in FIXED:
  out.append((name,{'command':'/missing/mcp-command' if name=='missing' else sys.executable,'args':[str(HERE/'fixtures/server.py'),name],'cwd':str(HERE),'timeout':300 if name in ('hang','list_hang','flood') else 5000}))
 for i in range(n):
  mode=['random','noise','random','random','exit','rpc_list','rpc_init'][i%7]
  out.append((f'{mode}:{i}',{'command':sys.executable,'args':[str(HERE/'q_server.py'),str(i),mode],'cwd':str(HERE),'timeout':5000}))
 for mode in ['decimal','utf8','byte_cap','signal','partial','timeout']:
  for i in range(3):out.append((f'{mode}:{i}',{'command':sys.executable,'args':[str(HERE/'q_server.py'),str(i),mode],'cwd':str(HERE),'timeout':300 if mode!='byte_cap' else 5000}))
 return out
def run(command,requests):
 started=time.monotonic()
 p=subprocess.run(list(map(str,command)),input=''.join(json.dumps(v)+'\n' for _,v in requests),text=True,capture_output=True,timeout=max(60,len(requests)*7),env=os.environ)
 if p.returncode:raise RuntimeError(p.stderr[-4000:])
 rows=[json.loads(s) for s in p.stdout.splitlines()];assert len(rows)==len(requests),(len(rows),len(requests))
 return rows,time.monotonic()-started
if __name__=='__main__':
 requests=cases(int(sys.argv[1]) if len(sys.argv)>1 else 2000)
 (WORK/'cases.json').write_text(json.dumps(requests))
 reference,seconds=run([os.environ['MCP_NODE'],'--experimental-strip-types',HERE/'reference.mjs'],requests)
 (WORK/'reference.json').write_text(json.dumps(reference));print('reference',len(reference),round(seconds,2),flush=True)
 report={}
 for compiler in ('py','rs'):
  for backend in ('rust','llvm'):
   actual,seconds=run([WORK/f'{compiler}_{backend}'/'runner'],requests)
   classes=collections.Counter(classify(k,a,e) for (k,_),a,e in zip(requests,actual,reference))
   bad=[{'case':k,'actual':a,'expected':e} for (k,_),a,e in zip(requests,actual,reference) if classify(k,a,e)=='unexplained']
   key=f'{compiler}/{backend}';report[key]={'cases':len(requests),'classes':dict(classes),'wall_seconds':round(seconds,3),'unexplained':bad}
   (WORK/f'{compiler}_{backend}_q.json').write_text(json.dumps(actual))
   (WORK/'q-report.json').write_text(json.dumps(report,indent=2));print(key,classes,round(seconds,2),flush=True)
 raise SystemExit(any(r['unexplained'] for r in report.values()))
