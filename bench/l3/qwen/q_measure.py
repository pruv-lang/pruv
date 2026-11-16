"""One size sweep: cold single-call import time, long 8192-byte linear-chain entries."""
import sys
sys.dont_write_bytecode=True
import json
import test_impl as t
from q import WORK, reference, line, msg

def main():
 g=t.configure();w=t.WORK/'measure';w.mkdir(exist_ok=True);built=t.WORK/'rs_llvm'
 host=w/'main.rs'
 host.write_text('''#[path = "'''+str(built/'root_gen.rs')+'''"] mod sol;
fn main(){let a:Vec<String>=std::env::args().collect();let b=std::fs::read(&a[1]).unwrap();
let start=std::time::Instant::now();let v=std::hint::black_box(sol::window_packet(&b,0));
println!("{} {}",start.elapsed().as_secs_f64()*1000.0,v[7]);}
''')
 exe=w/'runner'
 t.run([g.RUSTC,'-O','--edition','2021',host,'-o',exe,'--extern',f'pruv_str={t.WORK / "libpruv_str.rlib"}','-C',f'link-arg={built / "qwen_gen.o"}','-C',f'link-arg={built / "qwen_rt.o"}'])
 report=[]
 for n in [256,512,1024]:
  home=w/f'home{n}';path=home/'projects/-project/chats/chain.jsonl';path.parent.mkdir(parents=True,exist_ok=True)
  data=[]
  for i in range(n):
   x=msg('user' if i%2==0 else 'assistant','x',uuid=str(i),parentUuid=str(i-1) if i else '')
   x['padding']='';raw=line(x);x['padding']='p'*(8192-len(raw));data.append(line(x))
  path.write_bytes(b''.join(data));assert path.stat().st_size==n*8192
  item=dict(home=str(home),folder='/project',thread='chain',path=str(path),measure=True)
  ts=reference([item],f'measure-{n}')[0]
  ms,turns=t.run([exe,path]).split();assert int(turns)==ts['turns']
  row=dict(bytes=n*8192,chain=n,pruv_ms=float(ms),ts_ms=ts['ms'],ratio=float(ms)/ts['ms']);report.append(row)
  (w/'report.json').write_text(json.dumps(report,indent=2));print(row,flush=True)
if __name__=='__main__':main()
