import importlib.util, os, sys, subprocess, shutil, json
from pathlib import Path
sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
WORK = HERE / "work"
def configure():
    WORK.mkdir(parents=True, exist_ok=True)
    os.environ['TMPDIR'] = str(WORK)
    spec = importlib.util.spec_from_file_location('g2', ROOT / 'bench/g2/run.py')
    g2 = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(g2)
    main = ROOT  # a worktree can read the main checkout's installed tools
    git = ROOT / '.git'
    if git.is_file() and git.read_text().startswith('gitdir: '):
        main = Path(git.read_text().strip()[8:]).resolve().parents[2]
    py = os.environ.get('PRUV_PYTHON') or next(
        (str(p) for p in [ROOT / '.venv/bin/python', main / '.venv/bin/python'] if p.exists()), sys.executable)
    binary = Path(os.environ.get('PRUVC_RS', ROOT / 'pruvc-rs/target/release/pruvc'))
    if not binary.exists():
        binary = main / 'pruvc-rs/target/release/pruvc'
    if not binary.exists():
        raise RuntimeError('pruvc-rs binary not found; build it or set PRUVC_RS')
    g2.COMPILERS = {'py': [py, str(ROOT / 'pruvc/pruvc.py')], 'rs': [str(binary)]}
    return g2


def run(cmd, data=None):
    p = subprocess.run([str(x) for x in cmd], input=data, text=True, capture_output=True, timeout=600)
    if p.returncode:
        raise RuntimeError(f'{cmd}: exit {p.returncode}\n{p.stdout}\n{p.stderr}')
    return p.stdout




def host_policy(exe,w):
 home=w/'policy-home';chats=home/'projects/-policy/chats';chats.mkdir(parents=True,exist_ok=True)
 f=chats/'recent.jsonl';f.write_text('{"type":"user","cwd":"/policy/","message":{"parts":[{"text":"Fallback"}]}}\n')
 os.utime(f,(1700000000,1700000000))
 got=json.loads(run([exe,'--list',home,'/policy']))
 assert len(got)==1 and got[0]['updatedAt']=='2023-11-14T22:13:20.000Z' and got[0]['title']=='Fallback' and 'maybeOpen' not in got[0],got
 os.utime(f,None)
 got=json.loads(run([exe,'--list',home,'/policy']))
 assert len(got)==1 and got[0]['maybeOpen'] is True,got

def main():
 g2=configure()
 rlib=WORK/'libpruv_str.rlib'
 run([g2.RUSTC,'--crate-type','rlib','--crate-name','pruv_str',HERE/'str_adapter.rs','-o',rlib])
 flags=['--rust-extern',f'pruv_str={rlib}','--rust-search',str(WORK)]
 rustflags=['--extern',f'pruv_str={rlib}','-L',f'dependency={WORK}']
 fixtures=sorted((HERE/'fixtures').glob('*.jsonl'))
 expected=[json.loads(f.with_suffix('.expected.json').read_text()) for f in fixtures]
 failed=False
 for compiler in ('py','rs'):
  for backend in ('rust','llvm'):
   key=f'{compiler}/{backend}'
   try:
    w=WORK/f'{compiler}_{backend}';w.mkdir(exist_ok=True)
    c=g2.compile_pruv(compiler,str(HERE/'root.pruv'),str(w),backend,flags)
    if c['rc']!=0:raise RuntimeError(str(c))
    print(f'{key}: verifier PASS',flush=True)
    harness=w/'main.rs';harness.write_text((HERE/'host.rs').read_text().replace('@SOL@',c['rs']))
    extra=[]
    if c['ll']:
     extra,error=g2.build_llvm_objects(c['ll'],'qwen',str(w))
     if error:raise RuntimeError(error)
    exe=w/'runner'
    run([g2.RUSTC,'-O','--edition','2021',harness,'-o',exe]+extra+rustflags)
    got=[json.loads(x) for x in run([exe]+fixtures).splitlines()]
    assert len(got)==len(expected)
    passed=0
    for path,want,actual in zip(fixtures,expected,got):
     if want==actual:passed+=1
     else:
      failed=True
      print(f'{key}: FAIL {path.name}: expected {str(want)[:500]}, got {str(actual)[:500]}',flush=True)
    names=[('/A_b/한😀','a_-',{'project':'-A-b----','valid':True}),('/tmp/x','../bad',{'project':'-tmp-x','valid':False}),('','a'*129,{'project':'','valid':False})]
    for folder,thread,want in names:
     actual=json.loads(run([exe,'--names',folder,thread]))
     assert actual==want,(actual,want)
     passed+=1
    home=w/'home';chats=home/'projects/-project/chats';chats.mkdir(parents=True,exist_ok=True)
    for child in chats.iterdir():child.unlink()
    # More than 50 rejected recent entries must not starve older valid items.
    for i in range(51):
     f=chats/f'empty{i}.jsonl';f.write_text('{}\n');os.utime(f,(1800000000,1800000000))
    for i in range(52):
     f=chats/f'valid{i:02}.jsonl';f.write_bytes((HERE/'fixtures/normal.jsonl').read_bytes());os.utime(f,(1700000000+i,1700000000+i))
    (chats/'link.jsonl').symlink_to(HERE/'fixtures/normal.jsonl')
    wrong=chats/'wrong.jsonl';wrong.write_text('{"type":"user","cwd":"/wrong","message":{"parts":[{"text":"no"}]}}\n')
    got=json.loads(run([exe,'--list',home,'/project']))
    assert len(got)==50 and [x['thread'] for x in got]==[f'valid{i:02}' for i in range(51,1,-1)],got
    assert all(x['title']=='Hello' and x['updatedAt']=='2026-11-14T01:02:03.000Z' and 'maybeOpen' not in x for x in got)
    passed+=1
    host_policy(exe,w);passed+=1
    print(f'{key}: {passed}/{len(fixtures)+5} passed',flush=True)
   except Exception as e:
    failed=True;print(f'{key}: ERROR {e}',flush=True)
 return int(failed)
if __name__=='__main__':
 if sys.argv[1:]==['--host-policy']:
  configure()
  for compiler in ('py','rs'):
   for backend in ('rust','llvm'):
    w=WORK/f'{compiler}_{backend}'
    host_policy(w/'runner',w)
    print(f'{compiler}/{backend}: host policy 1/1 passed',flush=True)
 else:
  raise SystemExit(main())
