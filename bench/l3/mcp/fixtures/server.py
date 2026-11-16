"""Real stdio server with strict handshake checks and deterministic faults."""
import json,sys,time
mode=sys.argv[1]
def send(v): print(json.dumps(v),flush=True)
def recv(): return json.loads(sys.stdin.readline())
if mode=='flood':
 while True: print('not json',flush=True)
if mode=='stderr_space':
 sys.stderr.write('first\n\ufeff\xa0last\t error\ufeff\n');sys.exit(7)
if mode=='exit':
 sys.stderr.write('first\n\nlast error\n');sys.exit(7)
if mode=='hang': time.sleep(30);sys.exit()
if mode=='long': print('x'*(4*1024*1024+1),flush=True);time.sleep(30);sys.exit()
r=recv();assert r['id']==1 and r['params']=={'protocolVersion':'2025-11-25','capabilities':{},'clientInfo':{'name':'lair','version':'1'}}
if mode in ('fallback','reject'):
 send({'id':r['id'],'error':{'message':'old'}});r=recv();assert r['id']==2 and r['params']['protocolVersion']=='2025-06-18'
if mode=='reject': send({'id':r['id'],'error':{'message':'denied'}});time.sleep(30);sys.exit()
if mode=='noise':
 print('banner\n[]\n{bad}\n42',flush=True);send({'id':'1','result':{}});send({'id':999,'result':{}})
if mode=='ping':
 for method in ('ping','unsupported'):
  send({'id':'server-id','method':method});ans=recv();assert ans['id']=='server-id'
  assert ans.get('result')=={} if method=='ping' else ans['error']=={'code':-32601,'message':'Lair 앱은 이 요청을 지원하지 않아.'}
if mode=='numeric_id': print('{"id":1.0,"result":{}}',flush=True)
else: send({'id':r['id'],'result':{}})
assert recv()['method']=='notifications/initialized'
last=r['id']
for page in range(9):
 r=recv();assert r['id']==last+1 and r['method']=='tools/list';last=r['id']
 if page: assert r['params']['cursor']==str(page)
 if mode=='list_hang':time.sleep(30);sys.exit()
 if mode=='list_error':send({'id':r['id'],'error':{}});time.sleep(30);sys.exit()
 if mode=='late_exit':sys.stderr.write('last error\n');sys.exit(8)
 tool={'name':'hello','title':'Title','description':'test','annotations':{'readOnlyHint':True,'destructiveHint':False},'inputSchema':{'type':'object'}}
 items=[tool]
 if mode=='cap': items=[{}]*499+[tool,{'name':'overflow'}]
 if mode=='cut': tool['description']='x'*65535+'😀'+'tail'
 if mode=='flags': tool['title']='';tool['annotations']={'readOnlyHint':1,'destructiveHint':'yes'}
 if mode=='fingerprint':tool['inputSchema']={'10':1.0,'2':1e-7,'a':{'z':0,'b':True}}
 if mode=='bad_names': items=[None,[],{},*({'name':n} for n in ['', 'x'*201,'a\x00','a\x7f','a\x9f','a\u2028','a\u2029','😀'*101,'😀'*100,'ok'])]
 result={'tools':items}
 if mode=='many_pages' or (mode=='pages' and page==0):result['nextCursor']=str(page+1)
 send({'id':r['id'],'result':result})
 if 'nextCursor' not in result:time.sleep(30);break
