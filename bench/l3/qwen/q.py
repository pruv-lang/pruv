"""Default Q corpus. Run test_impl.py first to build all verified pairs; no network."""
import sys
sys.dont_write_bytecode = True
import argparse, json, os, random, subprocess
from pathlib import Path
from oracle import normalize
HERE = Path(__file__).resolve().parent
WORK = HERE / 'work' / 'q'
NODE = '/opt/homebrew/bin/node'
PAIRS = ['py/rust', 'py/llvm', 'rs/rust', 'rs/llvm']

def run(cmd):
    p = subprocess.run([str(x) for x in cmd], capture_output=True, text=True, timeout=1800)
    if p.returncode: raise RuntimeError(p.stderr[-3000:])
    return [json.loads(x) for x in p.stdout.splitlines()]

def line(x): return json.dumps(x, ensure_ascii=False, separators=(',', ':')).encode() + b'\n'
def msg(role, text, **kw): return dict(type=role, message={'parts':[{'text':text}]}, **kw)

def corpus(count, seed):
    r = random.Random(seed)
    for f in sorted((HERE/'fixtures').glob('*.jsonl')):
        yield f.stem, f.read_bytes(), 'P4' if f.stem=='invalid-times' else None
    # Target total retained bytes independently of the turn-count bound.
    yield 'total-limit', b''.join(line(msg(role, 'x'*32768)) for i in range(40) for role in ['user','assistant']), None
    for i in range(count):
        rows=[]
        for j in range(r.randint(2,12)):
            role=r.choice(['user','assistant','tool_result','system'])
            x=msg(role,r.choice(['hello','한국어 😀','  ','a\nb','x'])+str(i))
            x['timestamp']=r.choice([1700000000000+j, 'invalid', None, '2026-11-14T01:02:03.000Z'])
            x['message']['parts'] += r.sample([{'thought':True,'text':'hidden'}, {'thought':1,'text':'visible'}, {'functionCall':{}}, {'text':'wins','functionCall':{}}, {'text':5}, None], r.randrange(7))
            if r.random()<.65:
                x['uuid']='u'+str(j)
                x['parentUuid']=r.choice(['u'+str(r.randrange(j+2)), '', 'missing'])
            if r.random()<.4: x['provenance']=r.choice(['real_user','tool',None])
            if r.random()<.4: x['model']=r.choice(['m1','m2','',None])
            if role=='system': x.update(subtype='custom_title',systemPayload={'customTitle':r.choice(['title','  ','A\nB'])})
            rows.append(x)
        policy=None
        data=b''.join(map(line,rows))
        if i%100==0:
            policy='P1';data+=b'{"type":"user","message":{"parts":[{"text":"bad\xff"}]}}\n'
        elif i%100==1:
            policy='P4';data+=line(msg('user','policy',timestamp='2024-01-01'))
        yield f'r{i:05}',data,policy

def prepare(count,seed):
    WORK.mkdir(parents=True,exist_ok=True)
    items=[]; normal=[]
    for name,data,policy in corpus(count,seed):
        # Separate homes permit real list/read paths without filesystem stubs.
        home=WORK/'homes'/name;path=home/'projects/-project/chats'/f'{name}.jsonl'
        path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(data);os.utime(path,(1700000000,1700000000))
        item=dict(home=str(home),folder='/project',thread=name,path=str(path),policy=policy)
        items.append(item)
        if policy:
            n=dict(item);n['home']=str(WORK/'normalized'/name);n['path']=str(Path(n['home'])/'projects/-project/chats'/f'{name}.jsonl')
            Path(n['path']).parent.mkdir(parents=True,exist_ok=True);Path(n['path']).write_bytes(normalize(data,policy));normal.append(n)
    return items,normal

def reference(items,name):
    manifest=WORK/(name+'.json');manifest.write_text(json.dumps(items))
    return run([NODE,'--experimental-strip-types',HERE/'reference.mjs',manifest])

def main():
    ap=argparse.ArgumentParser();ap.add_argument('--count',type=int,default=20000);ap.add_argument('--seed',type=int,default=20261116);a=ap.parse_args()
    items,normal=prepare(a.count,a.seed)
    expected=reference(items,'manifest');norm=dict(zip([x['thread'] for x in normal],reference(normal,'normalized')))
    # Compare real list entries as well as head/tail/read; fixed edges include large files.
    listed=[dict(x,list=True) for x in items[:21]]
    home=WORK/'list-home';chats=home/'projects/-project/chats';chats.mkdir(parents=True,exist_ok=True)
    for i in range(103):
        f=chats/f'item{i:03}.jsonl'
        f.write_bytes(b'{}\n' if i>=52 else (HERE/'fixtures/normal.jsonl').read_bytes())
        os.utime(f,(1700000000+i,1700000000+i))
    listed.append(dict(home=str(home),folder='/project',thread='list-cap',list=True,policy=None))
    expected_lists=reference(listed,'lists')
    report={'seed':a.seed,'transcripts':len(items),'list_scenarios':len(listed),'pairs':{}}
    for pair in PAIRS:
        exe=HERE/'work'/pair.replace('/','_')/'runner';stats={'same':0,'P1':0,'P4':0,'P5':0,'unexplained':0,'list_same':0,'list_P4':0};bad=[]
        for start in range(0,len(items),100):
            batch=items[start:start+100];got=run([exe]+[x['path'] for x in batch])
            assert len(got)==len(batch)
            for k,(item,actual) in enumerate(zip(batch,got)):
                want=expected[start+k]
                if actual==want: stats['same']+=1
                elif item['policy'] and actual==norm[item['thread']]: stats[item['policy']]+=1
                elif item['thread']=='empty-cwd' and want['record']=={'error':'not-found'} and actual['head']==want['head'] and actual['tail']==want['tail'] and actual==json.loads((HERE/'fixtures/empty-cwd.expected.json').read_text()): stats['P5']+=1
                else:
                    stats['unexplained']+=1;bad.append({'item':item['thread'],'actual':actual,'reference':want})
        for item,want in zip(listed,expected_lists):
            actual=run([exe,'--list',item['home'],item['folder']])[0]
            if actual==want:stats['list_same']+=1
            elif item['policy']=='P4' and actual==[dict(x,updatedAt=norm[item['thread']]['tail']['lastAt']) for x in want]:stats['list_P4']+=1
            else:stats['unexplained']+=1;bad.append({'list':item['thread'],'actual':actual,'reference':want})
        report['pairs'][pair]=stats
        (WORK/(pair.replace('/','_')+'-differences.json')).write_text(json.dumps(bad,ensure_ascii=False,indent=2))
        (WORK/'report.json').write_text(json.dumps(report,indent=2));print(pair,stats,flush=True)
    return int(any(x['unexplained'] for x in report['pairs'].values()))
if __name__=='__main__':raise SystemExit(main())
