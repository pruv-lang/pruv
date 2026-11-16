"""Checks v8sort.py against Node 26.4.0 (V8 14.6): same final order and the same
comparator call sequence on 3000 seeded cases, including comparators that are not
total orders (the Grok listing comparator returns 0 whenever a date is missing)."""
import json, os, random, subprocess, sys
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import v8sort
from datetime import datetime, timezone
rng = random.Random(20261116)
cases = []
for k in range(3000):
    n = rng.choice([0,1,2,3,5,8,13,31,32,33,63,64,65,100,127,128,129,200,257,500,1000,2000,rng.randint(0,3000)])
    if k % 2 == 0:
        style = rng.choice(['rand','few','sorted','rev','runs'])
        dates = []
        base = 1.7e12
        for i in range(n):
            if rng.random() < rng.choice([0.0,0.1,0.5,0.9]): dates.append(None); continue
            if style == 'rand': t = base + rng.randint(0, 10**9)
            elif style == 'few': t = base + rng.randint(0, 3) * 1000
            elif style == 'sorted': t = base + i * 1000
            elif style == 'rev': t = base - i * 1000
            else: t = base + (i // rng.randint(1, 50)) * 1000 * rng.choice([1,-1])
            dates.append(datetime.fromtimestamp(t/1000, timezone.utc).strftime('%Y-%m-%dT%H:%M:%S.%f')[:-3]+'Z')
        cases.append({'n': n, 'kind': 'grok', 'dates': dates})
    else:
        tl = rng.randint(1, 50)
        cases.append({'n': n, 'kind': 'table', 'dates': [None]*n, 'salt': rng.randint(0,10**6),
                      'table': [rng.choice([-1, 0, 1, -0.5, 2]) for _ in range(tl)]})
p = subprocess.run(['/opt/homebrew/bin/node', os.path.join(HERE, 'probe.mjs')], input=json.dumps(cases), capture_output=True, text=True, check=True)
node = json.loads(p.stdout)
def parse(d):
    if d is None: return float('nan')
    return datetime.strptime(d, '%Y-%m-%dT%H:%M:%S.%fZ').replace(tzinfo=timezone.utc).timestamp()*1000
bad = 0
for c, r in zip(cases, node):
    calls = []
    def cmp(a, b):
        calls.extend([a, b])
        if c['kind'] == 'grok':
            v = parse(c['dates'][b]) - parse(c['dates'][a])
            return 0 if v != v or v == 0 else v
        return c['table'][(a*7919 + b*104729 + c['salt']) % len(c['table'])]
    order = v8sort.sort(list(range(c['n'])), cmp)
    if order != r['order'] or calls != r['calls']:
        bad += 1
        if bad <= 3: print('MISMATCH', c['n'], c['kind'], len(calls), len(r['calls']))
print('cases', len(cases), 'mismatch', bad, 'totalcalls', sum(len(r['calls'])//2 for r in node))
sys.exit(1 if bad else 0)
