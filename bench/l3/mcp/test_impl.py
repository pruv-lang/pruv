"""Only L3-3 builds and fixtures; no repository-wide suites."""
import sys
sys.dont_write_bytecode = True
import importlib.util,json,os,subprocess
from pathlib import Path
HERE=Path(__file__).resolve().parent
WORK=HERE/'work'
os.environ['MCP_FINGERPRINT_SCRIPT']=str(HERE/'fingerprint.cjs')
os.environ['MCP_NODE']=os.environ.get('MCP_NODE','/opt/homebrew/bin/node')
spec=importlib.util.spec_from_file_location('super_common',HERE.parent/'supervisor/common.py')
c=importlib.util.module_from_spec(spec);spec.loader.exec_module(c)
g,run=c.configure(WORK)
run([str(Path.home()/'.cargo/bin/cargo'),'build','--offline','--manifest-path',HERE/'host/Cargo.toml','--target-dir',WORK/'host'])
deps=WORK/'host/debug/deps'
lib=next(deps.glob('libmcp_host-*.rlib'));serde=next(deps.glob('libserde_json-*.rlib'))
adapter=WORK/'libpruv_str.rlib'
run([g.RUSTC,'--crate-name','pruv_str','--crate-type','rlib',HERE/'str_adapter.rs','-o',adapter])
LIB=HERE.parent/'lib'
flags=['--rust-extern',f'mcp_host={lib}','--rust-extern',f'pruv_str={adapter}','--lib',str(LIB)]
failures=[]
for compiler,backend in c.PAIRS:
 out=WORK/f'{compiler}_{backend}';out.mkdir(exist_ok=True)
 if '--reuse-verified' in sys.argv:
  built={'rc':0,'rs':str(out/'root_gen.rs'),'ll':str(out/'root_gen.ll') if backend=='llvm' else None}
  assert Path(built['rs']).is_file(), 'Run once without --reuse-verified first'
 else:
  built=g.compile_pruv(compiler,str(HERE/'root.pruv'),str(out),backend,flags)
 if built['rc']: raise RuntimeError(str(built))
 print(f'{compiler}/{backend}: '+('reusing verified Pruv emission' if '--reuse-verified' in sys.argv else 'verified'),flush=True)
 harness=out/'main.rs';harness.write_text((HERE/'harness.rs').read_text().replace('@SOL@',built['rs']).replace('@FINGERPRINT@',str(HERE/'fingerprint.rs')))
 extra=[]
 if built['ll']:
  extra,err=g.build_llvm_objects(built['ll'],'mcp',str(out));assert not err,err
 exe=out/'runner'
 run([g.RUSTC,'-O','--edition','2021',harness,'-o',exe,'--extern',f'mcp_host={lib}','--extern',f'pruv_str={adapter}','--extern',f'serde_json={serde}','-L',f'dependency={deps}',*extra])
 if '--verify-only' in sys.argv: continue
 cases=['normal','pages','many_pages','ping','fallback','reject','exit','long','hang','missing','bad_names','noise','cap','cut','flags','fingerprint','numeric_id','list_hang','list_error','late_exit','flood','stderr_space']
 passed=0
 for case in cases:
  try:
   cmd='/missing/mcp-command' if case=='missing' else sys.executable
   inp={'command':cmd,'args':[str(HERE/'fixtures/server.py'),case],'cwd':str(HERE),'timeout':300 if case in ('hang','list_hang','flood') else 5000}
   got=json.loads(run([exe],json.dumps(inp)+'\n'))
   if case in ['reject','exit','long','hang','missing','list_hang','list_error','late_exit','flood','stderr_space']:
    assert not got['ok'],(case,got)
    assert got['stage']==('start' if case=='missing' else 'list' if case in ('list_hang','list_error','late_exit') else 'connect'),(case,got)
    expected={'reject':'초기화를 거절했어: denied','exit':'서버가 시작 직후 끝났어(종료 코드 7): last error','long':'서버가 너무 긴 줄을 보냈어.','hang':'시간 초과: 0초 안에 끝나지 않았어.','missing':'명령을 찾지 못했어: /missing/mcp-command'}
    expected.update(flood='시간 초과: 0초 안에 끝나지 않았어.',stderr_space='서버가 시작 직후 끝났어(종료 코드 7): last error',list_hang='시간 초과: 0초 안에 끝나지 않았어.',list_error='서버 오류',late_exit='서버가 끝났어(종료 코드 8): last error')
    assert got['message']==expected[case],(case,got)
   else:
    assert got['ok'],(case,got)
    n={'pages':2,'many_pages':5,'bad_names':2}.get(case,1)
    assert len(got['tools'])==n,(case,got)
    assert all(len(t['fingerprint'])==16 for t in got['tools'])
    if case=='cut': assert got['tools'][0]['description']=='x'*65535
    if case=='flags': assert got['tools'][0]['readOnly'] is False and 'destructive' not in got['tools'][0] and 'title' not in got['tools'][0]
    if case in ('normal','cap'):
     import hashlib
     raw={'name':'hello','description':'test','annotations':{'readOnlyHint':True,'destructiveHint':False},'inputSchema':{'type':'object'}}
     want=hashlib.sha256(json.dumps(raw,sort_keys=True,separators=(',',':')).encode()).hexdigest()[:16]
     assert got['tools'][0]['fingerprint']==want
   passed+=1
  except AssertionError as error:
   failures.append((compiler,backend,case,str(error)))
   print(f"{compiler}/{backend}: FAIL {case}: {error}",flush=True)
 print(f'{compiler}/{backend}: {passed}/{len(cases)} passed',flush=True)
if failures: raise SystemExit(1)
