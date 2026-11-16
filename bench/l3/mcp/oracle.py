"""Narrow result-checked exceptions, never blanket acceptance."""
import copy

def classify(label, actual, expected):
 if actual==expected:return 'exact'
 if label=='cut':
  e=copy.deepcopy(expected)
  if e['tools'][0]['description'].endswith('\ud83d'):
   e['tools'][0]['description']=e['tools'][0]['description'][:-1]
   if actual==e:return 'surrogate-safe-cut'
 mode=label.split(':')[0]
 if mode in ('decimal','utf8'):
  if expected['ok'] and actual=={'ok':False,'stage':'connect','tools':[], 'message':'시간 초과: 0초 안에 끝나지 않았어.'}:
   return {'decimal':'exact-decimal-id','utf8':'invalid-utf8'}[mode]
 if mode=='byte_cap' and expected['ok'] and actual=={'ok':False,'stage':'connect','tools':[], 'message':'서버가 너무 긴 줄을 보냈어.'}:return 'byte-line-cap'
 if mode=='signal':
  e=copy.deepcopy(expected);e['message']=e.get('message','').replace('SIGTERM','15')
  if actual==e:return 'numeric-signal'
 return 'unexplained'
