"""Seeded real stdio behaviours, separate from the unchanged fixed fixtures."""
import json, os, random, signal, sys
seed=int(sys.argv[1]); r=random.Random(seed); mode=sys.argv[2]
def send(v): print(json.dumps(v,ensure_ascii=True),flush=True)
def recv():
 s=sys.stdin.readline()
 if not s: sys.exit()
 return json.loads(s)
def wait():
 for _ in sys.stdin: pass
 sys.exit()
a=recv()
if mode=='exit':
 sys.stderr.write('first\n  failure '+str(seed)+'\n');sys.stderr.flush();sys.exit(7)
if mode=='signal': os.kill(os.getpid(),signal.SIGTERM)
if mode=='partial': sys.stdout.write('{"id":1,"result":{}}');sys.stdout.flush();wait()
if mode=='timeout': wait()
if mode=='byte_cap':
 sys.stdout.buffer.write(('é'*2200000+'\n').encode());sys.stdout.flush()
if mode=='decimal': print('{"id":1.00000000000000001,"result":{}}',flush=True)
elif mode=='utf8':
 sys.stdout.buffer.write(b'{"id":1,"result":{"note":"\xff"}}\n');sys.stdout.flush()
else:
 if seed%3==0:
  send({'id':a['id'],'error':{'message':'retry'}});a=recv()
 if mode=='rpc_init':
  send({'id':a['id'],'error':{'message':'denied'}})
  if a['id']==1:a=recv();send({'id':a['id'],'error':{'message':'denied'}})
  wait()
 if mode=='noise':
  print('banner\r\n[]\n'+(' '*r.randrange(10000))+'{broken}',flush=True)
  send({'id':999,'result':{}});send({'method':'notifications/progress','params':{}})
  send({'id':str(a['id']),'result':{}})
  send({'id':a['id'],'method':'ping'});assert recv()['result']=={}
 send({'id':a['id'],'result':{}})
assert recv()['method']=='notifications/initialized'
for page in range(5):
 a=recv()
 if mode=='rpc_list': send({'id':a['id'],'error':{'message':' error\n '+str(seed)}});wait()
 items=[]
 for i in range(r.randrange(0,9)):
  if r.randrange(5)==0: items.append(r.choice([None,[],{},42]));continue
  items.append({'name':r.choice(['ok'+str(i),'','a\x00','😀'*100,'x'*201,'한글','a\u2028']),
   'title':r.choice(['','Title',None,4]),'description':r.choice(['','hello\nworld','😀','x'*r.randrange(100)]),
   'annotations':{'readOnlyHint':r.choice([True,False,1,None]),'destructiveHint':r.choice([True,False,'yes'])},
   'inputSchema':{'type':'object','seed':seed,'page':page}})
 result={'tools':items}
 if r.randrange(3)!=0: result['nextCursor']='page'+str(page+1)
 send({'id':a['id'],'result':result})
 if 'nextCursor' not in result:wait()
wait()
