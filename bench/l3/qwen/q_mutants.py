"""Hand mutants in work only; verifier rejection or exact fixture-output kill."""
import sys
sys.dont_write_bytecode=True
import json, shutil
from pathlib import Path
import test_impl as t

MUTANTS=[
 ('human-role','entry.pruv','"type", "user"','"type", "assistant"'),
 ('provenance','entry.pruv','"provenance", "real_user"','"provenance", "tool"'),
 ('human-thought','entry.pruv','if thought == 1 { return Block::Ignore; }','if thought == 0 { return Block::Ignore; }'),
 ('custom-title','entry.pruv','"subtype", "custom_title"','"subtype", "other"'),
 ('assistant-thought','import.pruv','if thought != 1 {','if thought == 1 {'),
 ('function-call','import.pruv','"functionCall"','"absentCall"'),
 ('model','import.pruv','root, "model"','root, "absentModel"'),
 ('chain-cut','chain.pruv','lost = true; done = true;','lost = false; done = true;'),
 ('chain-filter','chain.pruv','active: leaf.found, cut: lost, bad: leaf.bad','active: false, cut: lost, bad: leaf.bad'),
 ('leaf-parent','chain.pruv','if count > 0 { parent = copy(f.parent); }','if count >= 0 { parent = copy(f.parent); }'),
 ('last-title','import.pruv','return rank >= best;','return rank > best;'),
 ('turn-cap','turns.pruv','if avail > 200 { avail = 200; }','if avail > 199 { avail = 199; }'),
]

def main():
 g=t.configure();base=t.WORK/'mutants';base.mkdir(exist_ok=True);report=[]
 flags=['--rust-extern',f'pruv_str={t.WORK / "libpruv_str.rlib"}','--rust-search',str(t.WORK)]
 fixtures=sorted((t.HERE/'fixtures').glob('*.jsonl'));expected=[json.loads(f.with_suffix('.expected.json').read_text()) for f in fixtures]
 for name,file,old,new in MUTANTS:
  w=base/name;w.mkdir(exist_ok=True)
  for p in t.HERE.glob('*.pruv'):shutil.copy2(p,w/p.name)
  p=w/file;s=p.read_text();assert s.count(old)==1,(name,s.count(old));p.write_text(s.replace(old,new))
  c=g.compile_pruv('rs',str(w/'q_root.pruv'),str(w),'rust',flags)
  (w/'compile.json').write_text(json.dumps(c,default=str,indent=2))
  row={'name':name,'file':file,'old':old,'new':new}
  if c['rc']:
   detail=str(c.get('errors') or c.get('stderr'))
   # A parser/tool error is not credited as a semantic kill.
   row.update(result='verifier-rejected' if any(x in detail.lower() for x in ['ensures','invariant','verification','unproved','timeout']) else 'build-error',detail=detail[-2000:])
  else:
   host=w/'main.rs';host.write_text((t.HERE/'host.rs').read_text().replace('@SOL@',c['rs']));exe=w/'runner'
   t.run([g.RUSTC,'-O','--edition','2021',host,'-o',exe,'--extern',f'pruv_str={t.WORK / "libpruv_str.rlib"}'])
   actual=[json.loads(x) for x in t.run([exe]+fixtures).splitlines()]
   kills=[f.stem for f,a,b in zip(fixtures,actual,expected) if a!=b]
   row.update(result='differential-killed' if kills else 'survived',fixtures=kills)
   if not kills and name=='leaf-parent':
    row['reason']='Fixed fixtures lack a later metadata entry duplicating the actual leaf UUID with a different parent; generated cases use unique UUIDs.'
  report.append(row);(base/'report.json').write_text(json.dumps(report,indent=2));print(name,row['result'],flush=True)
 return int(any(x['result']=='build-error' for x in report))
if __name__=='__main__':raise SystemExit(main())
