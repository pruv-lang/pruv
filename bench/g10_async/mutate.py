"""Named textual edits for G10 async Q gate 3; never count crashes as rejection."""
import sys
sys.dont_write_bytecode = True
import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
from pathlib import Path
import tempfile
import run as G

# problem, family, expected stage, unique old text, replacement
EXPLICIT = [
 ('q01','missing_close','static','  let r: Result<i64> = ev_close(move l);\n  return finished;', '  return finished;'),
 ('q01','overwrite_live_loop','static','  l = ev_add_listener(move l, move ls);','  l = ev_new(17);\n  l = ev_add_listener(move l, move ls);'),
 ('q01','close_one_branch','static','  let r: Result<i64> = ev_close(move l);\n  return finished;', '  if finished == budget { let r: Result<i64> = ev_close(move l); }\n  return finished;'),
 ('q01','discarded_temporary','static','  let mut l: Loop = ev_new(17);','  let discarded: i64 = ev_cap(ev_new(1));\n  let mut l: Loop = ev_new(17);'),
 ('q01','unchecked_ok_before_add','static','  l = ev_add_listener(move l, move ls);','  l = ev_add_listener(move l, move ls);\n  l = ev_add_timer(move l, 1);'),
 ('q01','slot_after_remove','static','            l = ev_remove(move l, i);','            l = ev_remove(move l, i);\n            l = ev_read(move l, i, 1);'),
 ('q01','read_listener','static','        l = ev_accept(move l, slot);','        l = ev_read(move l, slot, 1);'),
 ('q01','write_after_shutdown','static','                  l = ev_write(move l, slot, reply);','                  l = ev_shutdown_write(move l, slot);\n                  l = ev_write(move l, slot, reply);'),
 ('q01','cap_out_of_range','static','ev_new(17)','ev_new(0)'),
 ('q01','add_to_full','static','  let mut l: Loop = ev_new(17);','  let mut full: Loop = ev_new(1);\n  full = ev_add_timer(move full, 1);\n  full = ev_add_timer(move full, 1);\n  let closed: Result<i64> = ev_close(move full);\n  let mut l: Loop = ev_new(17);'),
 ('q01','eos_never_removed','oracle','              l = ev_remove(move l, slot);\n              timers[slot] = -1;\n              finished = finished + 1;','              timers[slot] = -1;'),
 ('q01','fired_timer_kept','static','          let i: i64 = owners[slot];','          l = ev_remove(move l, slot);\n          let i: i64 = owners[slot];'),
 ('q01','line_off_by_one','oracle','store_be(inout reply, 0, have, 2);','store_be(inout reply, 0, have + 1, 2);'),
 ('q01','queued_chunk_dropped','oracle','            let chunk: [u8] = ev_bytes(l);','            let chunk: [u8] = alloc(0, 0);'),
 ('q02','cancel_ignored','oracle','if ev_size(l) > 0 { outcome = 4; }','if ev_size(l) < 0 { outcome = 4; }'),
 ('q02','stderr_not_drained','oracle','          l = ev_proc_read(move l, pslot, 2, 4096);   // drop stderr so the child cannot stall','          // stderr intentionally not drained'),
]
CONTROLS = [
 ('q01','smaller_reads','accepted','l = ev_read(move l, slot, 4096);','l = ev_read(move l, slot, 2048);'),
 ('q02','longer_write_timeout','accepted','c = net_set_timeout(move c, 5000);','c = net_set_timeout(move c, 6000);'),
]

# Isolate these invalid operations in live helpers: otherwise the solver times out
# constructing a counterexample through the server's nested array/event state.
HELPERS = {
 'slot_after_remove': '''fn removed_read(take l: Loop, slot: i64) -> Loop ! async, net, proc, alloc
  requires ev_ok(l) && 0 <= slot && slot < ev_cap(l)
  requires ev_kind(l, slot) == 1
  ensures ev_ok(result) && ev_cap(result) == ev_cap(l)
{
  let mut q: Loop = ev_remove(move l, slot);
  return ev_read(move q, slot, 1);
}
''',
 'write_after_shutdown': '''fn shut_write(take l: Loop, slot: i64, data: [u8]) -> Loop ! async, net, alloc
  requires ev_ok(l) && 0 <= slot && slot < ev_cap(l)
  requires ev_kind(l, slot) == 1 && ev_write_open(l, slot)
  ensures ev_ok(result) && ev_cap(result) == ev_cap(l)
  ensures ev_kind(result, slot) == 1
{
  let mut q: Loop = ev_shutdown_write(move l, slot);
  return ev_write(move q, slot, data);
}
''',
}

def catalogue():
    items=[]
    for n,(pid,family,expected,old,new) in enumerate(EXPLICIT+CONTROLS,1):
        text=(G.HERE/'solutions'/(pid+'.pruv')).read_text()
        assert text.count(old)==1,(pid,family,text.count(old))
        helper = HELPERS.get(family, '')
        if family == 'slot_after_remove':
            new = '            l = removed_read(move l, i);'
        if family == 'write_after_shutdown':
            new = '                  l = shut_write(move l, slot, reply);'
        source = helper + text.replace(old,new)
        items.append(dict(id=f'm{n:03}',pid=pid,family=family,expected=expected,old=old,new=new,
                          helper=helper,source=source,intent='incorrect' if n<=16 else 'control',
                          sha256=hashlib.sha256(source.encode()).hexdigest()))
    assert len({i['sha256'] for i in items})==18
    return items

def classify(r):
    if r['status']=='ok': return 'accepted'
    if r['status']=='bind_permission': return 'bind_permission'
    if r['status']=='wrong':
        # A timeout, panic, or crash is unresolved, never an oracle catch.
        return 'wrong_but_proved' if r['rc']==0 and r.get('tests_total',-1)>0 else 'runtime_failure'
    if r['status']=='pruv_error':
        errors=r.get('errors',[])
        if r['rc']==-1: return 'timeout'
        if r['rc']!=1 or not errors or any(e.get('stage')=='internal' for e in errors): return 'compiler_crash'
        if any(e.get('code') in ('solver_unknown','solver_timeout','inference_timeout','verify_unknown') for e in errors): return 'unknown'
        if any(e.get('stage')=='codegen' for e in errors): return 'unsupported'
        return 'rejected_verify' if all(e.get('stage')=='verify' for e in errors) else 'rejected_check'
    return 'unresolved_'+r['status']

EXPECTED={'static':('rejected_check','rejected_verify'),'oracle':('wrong_but_proved',),'accepted':('accepted',)}

def measure(item,compiler,backend):
    with tempfile.TemporaryDirectory(prefix=item['id']+'_',dir=G.WORK) as tmp:
        src=Path(tmp)/'root.pruv'; src.write_text(item['source'])
        r=G.grade_one(src,item['pid'],compiler,backend)
        return {k:v for k,v in item.items() if k!='source'} | {'runs':{compiler+'-'+backend:{'class':classify(r),'grader':r}}}

def main():
    p=argparse.ArgumentParser(__doc__)
    p.add_argument('out',type=Path,nargs='?')
    p.add_argument('--compiler',choices=['py','rs','both'],default='both')
    p.add_argument('--backend',choices=['rust','llvm','both'],default='both')
    p.add_argument('--jobs',type=int,default=2)
    p.add_argument('--self-test',action='store_true')
    p.add_argument('--list',action='store_true')
    a=p.parse_args(); items=catalogue()
    if a.self_test:
        assert len(EXPLICIT)==16 and len(CONTROLS)==2
        print('self-test: PASS; 16 explicit incorrect mutants; 2 controls'); return 0
    if a.list:
        for i in items: print(i['id'],i['pid'],i['expected'],i['family'])
        return 0
    failed=False
    for compiler in ['py','rs'] if a.compiler=='both' else [a.compiler]:
        for backend in ['rust','llvm'] if a.backend=='both' else [a.backend]:
            with ThreadPoolExecutor(max_workers=max(1,a.jobs)) as ex:
                rows=list(ex.map(lambda i:measure(i,compiler,backend),items))
            key=compiler+'-'+backend
            counts=Counter(i['runs'][key]['class'] for i in rows)
            unexpected=[i['id'] for i in rows if i['runs'][key]['class'] not in EXPECTED[i['expected']] and i['runs'][key]['class']!='bind_permission']
            summary={'total':18,'explicit_incorrect':16,'controls':2,
                     'by_expected':{s:dict(Counter(i['runs'][key]['class'] for i in rows if i['expected']==s)) for s in EXPECTED},
                     'unexpected':unexpected,'blocked':[i['id'] for i in rows if i['runs'][key]['class']=='bind_permission']}
            out=a.out or G.HERE/'results'/('mutants_'+compiler+'_'+backend+'.json')
            out.parent.mkdir(exist_ok=True)
            out.write_text(json.dumps({'tools':G.G2.COMPILERS,'summary':summary,'items':rows},indent=2))
            print('SUMMARY '+key+' '+json.dumps(summary,sort_keys=True),flush=True)
            failed |= bool(unexpected or summary['blocked'])
    return int(failed)

if __name__=='__main__': sys.exit(main())
