"""Apply seven independent compiler faults to disposable copies; never patch the working tree."""
import sys
sys.dont_write_bytecode = True
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import perf as P
import argparse
import difflib
import hashlib
import json
import os
import shutil
import tempfile

HERE=Path(__file__).resolve().parent
ROOT=P.ROOT
C='pruvc-rs/crates/pruv-core/src/'
V='pruvc-rs/crates/pruv-proof/src/symbolic.rs'
FAULTS=[
 ('f1_must_consume','missing_close','reject',[
 ('pruvc/foreign.py','if n in s.proc and st[n] != "dead":\n                    _handle_lost(x, s.proc[n], "%s %s is not consumed before return"','if n in s.proc and s.proc[n] != "Loop" and st[n] != "dead":\n                    _handle_lost(x, s.proc[n], "%s %s is not consumed before return"'),
 (C+'foreign.rs','if *v != V::Dead {\n                            return Err(handle_lost(kind, r.line','if *v != V::Dead && *kind != "Loop" {\n                            return Err(handle_lost(kind, r.line')]),
 ('f2_read_kind','read_listener','reject',[
 ('pruvc/verify.py','need(k != 0 if n == "ev_remove" else k == expected)','if n != "ev_read": need(k != 0 if n == "ev_remove" else k == expected)'),
 (V,'self.proc_need(st,g,e,code,&what,if n == "ev_remove" { k.ne_i(0) } else { k.raw_eq(&z.int(expected)) });','if n != "ev_read" { self.proc_need(st,g,e,code,&what,if n == "ev_remove" { k.ne_i(0) } else { k.raw_eq(&z.int(expected)) }); }')]),
 ('f3_remove_frame','remove_frame','accept',[
 ('pruvc/verify.py','r.count == (p.count - 1 if freed else p.count)','r.count == (p.count if n == "ev_remove" else (p.count - 1 if freed else p.count))'),
 (V,'r.count.eq(&if freed { p.count.sub(&z.int(1)) } else { p.count.clone() })','r.count.eq(&if n == "ev_remove" { p.count.clone() } else if freed { p.count.sub(&z.int(1)) } else { p.count.clone() })')]),
 ('f4_close_tag','close_tag','runtime',[
 ('pruvc/codegen.py','if l.first_err!=0 {Err(l.first_err)}else{Ok(n)}}','if l.first_err!=0 {Ok(l.first_err)}else{Err(n)}}'),
 (C+'codegen.rs','if l.first_err!=0 {Err(l.first_err)}else{Ok(n)}}','if l.first_err!=0 {Ok(l.first_err)}else{Err(n)}}')]),
 # G10-ASYNC-DEADLINE: the bounded wait ignores its deadline, a write deadline fails the slot, and the kept branch of
 # the wait_for encoding forgets that ev_exit is unchanged.
 ('f5_wait_for_blocks','tests/runtime/g10a_r23_wait_for','proc',[
 ('pruvc/codegen.py','Duration::from_millis(ms as u64); let mut step','Duration::from_millis(86400000); let mut step'),
 (C+'codegen.rs','Duration::from_millis(ms as u64); let mut step','Duration::from_millis(86400000); let mut step')]),
 ('f6_write_for_fails_slot','tests/runtime/g10a_r24_write_for','proc',[
 ('pruvc/codegen.py','if !ok { pruv_ev_half(&mut l,i); } l','if !ok { pruv_ev_mark(&mut l,i,7); } l'),
 (C+'codegen.rs','if !ok { pruv_ev_half(&mut l,i); } l','if !ok { pruv_ev_mark(&mut l,i,7); } l')]),
 ('f7_wait_for_frame','tests/g10_async/proof/ev_proc_wait_for','accept',[
 ('pruvc/verify.py','r.count == p.count, r.exit == p.exit)))','r.count == p.count)))'),
 (V,'r.count.eq(&p.count), r.exit.eq(&p.exit)])]','r.count.eq(&p.count)])]')])]

def build_rs(tree,target):
    env=dict(os.environ,CARGO_TARGET_DIR=str(target),TMPDIR=str(P.WORK))
    return P.G.run([str(Path.home()/'.cargo/bin/cargo'),'build','--release','--offline','--bin','pruvc','--manifest-path',tree/'pruvc-rs/Cargo.toml'],timeout=600,env=env)

def test_child():
    # the tests/runtime proc drivers expect the A2 fixture child
    exe=P.WORK/'test_child'; src=ROOT/'tests/g10_proc/child.rs'
    if not exe.exists() or exe.stat().st_mtime<src.stat().st_mtime:
        r=P.G.run([P.G.G2.RUSTC,'-O','--edition=2021',src,'-o',exe])
        if r['rc']: raise SystemExit('test child build failed: '+r['stderr'][-800:])
    return exe

def probe_source(test):
    return ROOT/(test+'.pruv') if '/' in test else HERE/(test+'.pruv')

def probe(compiler,src,kind,backend,wd):
    old=P.G.G2.COMPILERS['probe'] if 'probe' in P.G.G2.COMPILERS else None
    P.G.G2.COMPILERS['probe']=compiler
    try: c=P.G.G2.compile_pruv('probe',str(src),str(wd),backend,['--no-verify'] if kind=='proc' else [])
    finally:
        if old is None: P.G.G2.COMPILERS.pop('probe',None)
        else: P.G.G2.COMPILERS['probe']=old
    if kind=='reject':
        expected='async_unconsumed' if src.stem=='missing_close' else 'call_requires'
        return dict(c,passed=c['rc']==1 and any(e.get('code')==expected for e in c['errors']),valid=c['rc'] in (0,1) and not any(e.get('stage')=='internal' for e in c['errors']))
    if kind=='accept': return dict(c,passed=c['rc']==0,valid=c['rc'] in (0,1) and not any(e.get('stage')=='internal' for e in c['errors']))
    if c['rc']: return dict(c,passed=False,valid=False)
    if kind=='proc':
        exe,error=P.G.build(c,wd,src.with_suffix('.rs').read_text())
        if error: return dict(c,passed=False,valid=False,build_error=error)
        r=P.G.run([exe],timeout=300,env=dict(os.environ,PRUV_PROC_CHILD=str(test_child())),cwd=wd)
        return dict(c,run=r,passed=r['rc']==0,valid=r['rc'] in (0,101))
    exe,error=P.G.build(c,wd,'fn main() { assert_eq!(sol::close_tag(), 0); }')
    if error: return dict(c,passed=False,valid=False,build_error=error)
    r=P.G.run([exe])
    return dict(c,run=r,passed=r['rc']==0,valid=r['rc'] in (0,101) and ('assertion' in r['stderr'] or r['rc']==0))

def main():
    p=argparse.ArgumentParser(__doc__)
    p.add_argument('--compiler',choices=['py','rs','both'],default='both')
    p.add_argument('--backend',choices=['rust','llvm','both'],default='both')
    p.add_argument('--generate-only',action='store_true')
    p.add_argument('--only',action='append',help='run only this fault (repeatable)')
    a=p.parse_args(); rows=[]
    for name,test,kind,edits in FAULTS:
        if a.only and name not in a.only and not a.generate_only: continue
        patch=''
        for file,old,new in edits:
            original=(ROOT/file).read_text()
            assert original.count(old)>=1,(name,file,original.count(old))
            patch+=''.join(difflib.unified_diff(original.splitlines(True),original.replace(old,new,1).splitlines(True),fromfile='a/'+file,tofile='b/'+file))
        path=HERE/(name+'.diff')
        if a.generate_only: path.write_text(patch); continue
        # Compare the checked-in patch to the current sources before applying it.
        assert path.read_text()==patch, 'stale patch: '+name
        with tempfile.TemporaryDirectory(prefix=name+'_',dir=P.WORK) as td:
            tree=Path(td)
            for folder in ['pruvc','pruvc-rs','stdlib']:
                shutil.copytree(ROOT/folder,tree/folder,ignore=shutil.ignore_patterns('target','__pycache__','*.pyc'))
            for source in (tree/'pruvc-rs/crates').rglob('*.rs'):
                source.touch()
            target=P.WORK/'fault-target'
            baseline={}
            for compiler in (['py','rs'] if a.compiler=='both' else [a.compiler]):
                build=build_rs(tree,target) if compiler=='rs' else None
                cmd=[P.G.G2.COMPILERS['py'][0],str(tree/'pruvc/pruvc.py')] if compiler=='py' else [str(target/'release/pruvc')]
                for backend in (['rust','llvm'] if a.backend=='both' else [a.backend]):
                    key=compiler+'_'+backend; wd=tree/('clean_'+key); wd.mkdir()
                    baseline[key]={'passed':False,'valid':False,'build':build} if build and build['rc'] else probe(cmd,probe_source(test),kind,backend,wd)
            applied=P.G.run(['patch','--batch','-p1','-d',tree,'-i',path])
            for compiler in (['py','rs'] if a.compiler=='both' else [a.compiler]):
                build=build_rs(tree,target) if compiler=='rs' else None
                cmd=[P.G.G2.COMPILERS['py'][0],str(tree/'pruvc/pruvc.py')] if compiler=='py' else [str(target/'release/pruvc')]
                for backend in (['rust','llvm'] if a.backend=='both' else [a.backend]):
                    key=compiler+'_'+backend; wd=tree/('faulty_'+key); wd.mkdir()
                    faulty={'passed':False,'valid':False,'build':build} if applied['rc'] or (build and build['rc']) else probe(cmd,probe_source(test),kind,backend,wd)
                    detected=baseline[key]['passed'] and faulty['valid'] and not faulty['passed']
                    row={'fault':name,'compiler':compiler,'backend':backend,'test':test,'kind':kind,'patch_sha256':hashlib.sha256(path.read_bytes()).hexdigest(),'patch_apply':applied,'baseline':baseline[key],'faulty':faulty,'status':'caught' if detected else 'NOT_CAUGHT'}
                    rows.append(row); print(name,compiler,backend,row['status'],flush=True)
    if not a.generate_only:
        out=P.HERE/'results/faults.json'
        selected={(r['fault'],r['compiler'],r['backend']) for r in rows}
        previous=json.loads(out.read_text())['rows'] if out.exists() else []
        retained=[r for r in previous if (r['fault'],r['compiler'],r['backend']) not in selected]
        out.write_text(json.dumps({'rows':retained+rows},indent=2))
    return int(any(r['status']!='caught' for r in rows))

if __name__=='__main__': sys.exit(main())
