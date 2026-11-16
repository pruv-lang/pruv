"""Single source replacements in work only, rs/rust; original TS oracle."""
import sys
sys.dont_write_bytecode=True
import importlib.util,json,os,shutil
from q import HERE,WORK,cases,run
from oracle import classify
MUTANTS=[
 ('page-contract','root.pruv','ensures result == 5 { return 5; }','ensures result == 5 { return 4; }'),
 ('tool-contract','root.pruv','ensures result == 500 { return 500; }','ensures result == 500 { return 499; }'),
 ('old-protocol','root.pruv','2025-06-18','2025-06-17'),
 ('new-protocol','root.pruv','2025-11-25','2025-11-24'),
 ('no-retry','root.pruv','!ready && id == 1','!ready && id == 2'),
 ('ping','root.pruv','method,"ping"','method,"pong"'),
 ('early-pages','root.pruv','page >= 5 ||','page >= 1 ||'),
 ('missing-command','root.pruv','if error == 2 && command_missing','if error == 3 && command_missing'),
 ('drop-title','packet.pruv','key="title"','key="absentTitle"'),
 ('description-cap','packet.pruv','cap=65536','cap=10'),
 ('readonly','packet.pruv','p[base+6]=1','p[base+6]=0'),
 ('destructive','packet.pruv','p[base+7]=destructive','p[base+7]=-1'),
]
def main():
 spec=importlib.util.spec_from_file_location('common',HERE.parent/'supervisor/common.py')
 c=importlib.util.module_from_spec(spec);spec.loader.exec_module(c);g,buildrun=c.configure(WORK)
 deps=WORK/'host/debug/deps';lib=next(deps.glob('libmcp_host-*.rlib'));serde=next(deps.glob('libserde_json-*.rlib'));adapter=WORK/'libpruv_str.rlib'
 manifest=json.loads((HERE/'pruv.json').read_text())
 flags=['--rust-extern',f'mcp_host={lib}','--rust-extern',f'pruv_str={adapter}']
 requests=cases(0)[:22];expected=json.loads((WORK/'reference.json').read_text())[:22]
 report=[]
 for name,file,old,new in MUTANTS:
  w=WORK/'mutants'/name;shutil.rmtree(w,ignore_errors=True);w.mkdir(parents=True,exist_ok=True)
  for p in HERE.glob('*.pruv'):shutil.copy2(p,w/p.name)
  # manifest copy next to the root: dependency paths are relative to the manifest, so point them at the real packages
  mutated=json.loads(json.dumps(manifest))
  for dep in mutated.get('dependencies',{}).values():dep['path']=os.path.relpath((HERE/dep['path']).resolve(),w.resolve())
  (w/'pruv.json').write_text(json.dumps(mutated)+'\n')
  p=w/file;s=p.read_text();assert s.count(old)==1,(name,s.count(old));p.write_text(s.replace(old,new))
  built=g.compile_pruv('rs',str(w/'root.pruv'),str(w),'rust',flags+['--manifest',str(w/'pruv.json')])
  (w/'compile.json').write_text(json.dumps(built,default=str,indent=2))
  row={'name':name,'file':file,'old':old,'new':new}
  if built['rc']:
   detail=str(built.get('errors') or built.get('stderr'))
   row.update(result='verifier-rejected' if any(x in detail.lower() for x in ('ensures','invariant','unproved','verification')) else 'build-error',detail=detail[-2000:])
  else:
   harness=w/'main.rs';harness.write_text((HERE/'harness.rs').read_text().replace('@SOL@',built['rs']).replace('@FINGERPRINT@',str(HERE/'fingerprint.rs')))
   exe=w/'runner';buildrun([g.RUSTC,'-O','--edition','2021',harness,'-o',exe,'--extern',f'mcp_host={lib}','--extern',f'pruv_str={adapter}','--extern',f'serde_json={serde}','-L',f'dependency={deps}'])
   actual,_=run([exe],requests)
   kills=[k for (k,_),a,e in zip(requests,actual,expected) if classify(k,a,e)=='unexplained']
   row.update(result='differential-killed' if kills else 'survived',fixtures=kills)
   if not kills:row['reason']='No distinguishing output in the 22 fixed fixtures; not a claim of equivalence.'
  report.append(row);(WORK/'mutants/report.json').write_text(json.dumps(report,indent=2));print(name,row['result'],flush=True)
 return int(any(x['result']=='build-error' for x in report))
if __name__=='__main__':raise SystemExit(main())
