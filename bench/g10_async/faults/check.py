"""Apply four independent compiler faults to disposable copies; never patch the working tree."""
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
 (V,'self.proc_need(st,g,e,code,&what,if n == "ev_remove" { k.ne_i(0) } else { k.eq(&z.int(expected)) });','if n != "ev_read" { self.proc_need(st,g,e,code,&what,if n == "ev_remove" { k.ne_i(0) } else { k.eq(&z.int(expected)) }); }')]),
 ('f3_remove_frame','remove_frame','accept',[
 ('pruvc/verify.py','r.count == (p.count - 1 if freed else p.count)','r.count == (p.count if n == "ev_remove" else (p.count - 1 if freed else p.count))'),
 (V,'r.count.eq(&if freed { p.count.sub(&z.int(1)) } else { p.count.clone() })','r.count.eq(&if n == "ev_remove" { p.count.clone() } else if freed { p.count.sub(&z.int(1)) } else { p.count.clone() })')]),
 ('f4_close_tag','close_tag','runtime',[
 ('pruvc/codegen.py','if l.first_err!=0 {Err(l.first_err)}else{Ok(n)}}','if l.first_err!=0 {Ok(l.first_err)}else{Err(n)}}'),
 (C+'codegen.rs','if l.first_err!=0 {Err(l.first_err)}else{Ok(n)}}','if l.first_err!=0 {Ok(l.first_err)}else{Err(n)}}')])]

def build_rs(tree,target):
    env=dict(os.environ,CARGO_TARGET_DIR=str(target),TMPDIR=str(P.WORK))
    return P.G.run([str(Path.home()/'.cargo/bin/cargo'),'build','--release','--offline','--bin','pruvc','--manifest-path',tree/'pruvc-rs/Cargo.toml'],timeout=600,env=env)

def probe(compiler,src,kind,backend,wd):
    old=P.G.G2.COMPILERS['probe'] if 'probe' in P.G.G2.COMPILERS else None
    P.G.G2.COMPILERS['probe']=compiler
    try: c=P.G.G2.compile_pruv('probe',str(src),str(wd),backend)
    finally:
        if old is None: P.G.G2.COMPILERS.pop('probe',None)
        else: P.G.G2.COMPILERS['probe']=old
    if kind=='reject':
        expected='async_unconsumed' if src.stem=='missing_close' else 'call_requires'
        return dict(c,passed=c['rc']==1 and any(e.get('code')==expected for e in c['errors']),valid=c['rc'] in (0,1) and not any(e.get('stage')=='internal' for e in c['errors']))
    if kind=='accept': return dict(c,passed=c['rc']==0,valid=c['rc'] in (0,1) and not any(e.get('stage')=='internal' for e in c['errors']))
    if c['rc']: return dict(c,passed=False,valid=False)
    exe,error=P.G.build(c,wd,'fn main() { assert_eq!(sol::close_tag(), 0); }')
    if error: return dict(c,passed=False,valid=False,build_error=error)
    r=P.G.run([exe])
    return dict(c,run=r,passed=r['rc']==0,valid=r['rc'] in (0,101) and ('assertion' in r['stderr'] or r['rc']==0))

def main():
    p=argparse.ArgumentParser(__doc__)
    p.add_argument('--compiler',choices=['py','rs','both'],default='both')
    p.add_argument('--backend',choices=['rust','llvm','both'],default='both')
    p.add_argument('--generate-only',action='store_true')
    a=p.parse_args(); rows=[]
    for name,test,kind,edits in FAULTS:
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
                    baseline[key]={'passed':False,'valid':False,'build':build} if build and build['rc'] else probe(cmd,HERE/(test+'.pruv'),kind,backend,wd)
            applied=P.G.run(['patch','--batch','-p1','-d',tree,'-i',path])
            for compiler in (['py','rs'] if a.compiler=='both' else [a.compiler]):
                build=build_rs(tree,target) if compiler=='rs' else None
                cmd=[P.G.G2.COMPILERS['py'][0],str(tree/'pruvc/pruvc.py')] if compiler=='py' else [str(target/'release/pruvc')]
                for backend in (['rust','llvm'] if a.backend=='both' else [a.backend]):
                    key=compiler+'_'+backend; wd=tree/('faulty_'+key); wd.mkdir()
                    faulty={'passed':False,'valid':False,'build':build} if applied['rc'] or (build and build['rc']) else probe(cmd,HERE/(test+'.pruv'),kind,backend,wd)
                    detected=baseline[key]['passed'] and faulty['valid'] and not faulty['passed']
                    row={'fault':name,'compiler':compiler,'backend':backend,'test':test,'kind':kind,'patch_sha256':hashlib.sha256(path.read_bytes()).hexdigest(),'patch_apply':applied,'baseline':baseline[key],'faulty':faulty,'status':'caught' if detected else 'NOT_CAUGHT'}
                    rows.append(row); print(name,compiler,backend,row['status'],flush=True)
    if not a.generate_only:
        out=P.HERE/'results/faults.json'
        selected={(r['compiler'],r['backend']) for r in rows}
        previous=json.loads(out.read_text())['rows'] if out.exists() else []
        retained=[r for r in previous if (r['compiler'],r['backend']) not in selected]
        out.write_text(json.dumps({'rows':retained+rows},indent=2))
    return int(any(r['status']!='caught' for r in rows))

if __name__=='__main__': sys.exit(main())
