"""L3-10 M5: generated effect programs (stand-in for the semantic core, M7-M12, which does not exist yet).
  python3 m5_test/gen_prog.py observations.jsonl programs.jsonl
For every oracle observation, writes one program line {"id", "segs", "final"} (format: M5_NOTES.md, "Effect programs").
A program carries only what the core decides: which dependency calls it makes and with which arguments, which timers it
arms or clears (by an opaque key), the request terminal and return value, and the core-owned clocks / toolSeen maps.
Everything the host owns is dropped from the program and must be recomputed by the host: call.start/call.end records,
script-event records, `now`/`unit`/`uuid` values, findSession results, query replies, timer tokens/dues/firing order,
timer.unref, timer.fire, dep-throw records, final sessions and active timers.
The core also owns the budget attachment of a session (TS writes session.budget in place, with no trace record): before
every host read of a session (call.start arguments, findSession result, snapshot, final sessions) whose budget differs from
the last value the host has seen, the program carries a {"k": "bset", "s": label, "b"?: budget JV} item."""
import sys
sys.dont_write_bytecode = True
import json

HOST_EVENTS = {'advance', 'setTime', 'setUnit', 'putSession', 'removeSession', 'setDepReply', 'snapshot'}
QUERIES = {'runningDelegationIds', 'isQuiet', 'releaseHeld'}
EFFECTS = {'save', 'changed', 'stopMain', 'stopDelegation', 'stopAttachedTurns', 'holdQueued', 'carryHeld', 'queueContinue',
           'dropHeld', 'cancelRecovery', 'announce'}


def field(obj_jv, key):
    for k, v in obj_jv['obj']:
        if k == key:
            return v
    raise KeyError(key)


def item_of(rec, trace, j):
    """Program item of one core-originated trace record (None when the record is host-owned)."""
    c = rec['c']
    if c == 'now':
        return {'k': 'now'}
    if c == 'unit':
        return {'k': 'unit'}
    if c == 'uuid':
        return {'k': 'uuid'}
    if c == 'findSession':
        return {'k': 'find', 'id': rec['a'][0]}
    if c in EFFECTS:
        return {'k': 'dep', 'c': c, 'a': rec.get('a', [])}
    if c in QUERIES:
        return {'k': 'q', 'c': c, 'a': rec.get('a', [])}
    if c == 'timer.set':
        token = field(rec['a'], 'token')
        nxt = trace[j + 1] if j + 1 < len(trace) else None
        unref = nxt is not None and nxt['c'] == 'timer.unref' and field(nxt['a'], 'token') == token
        return {'k': 'tset', 'key': token, 'delay': field(rec['a'], 'delay'), 'unref': unref}
    if c == 'timer.unref':
        return None
    if c == 'timer.clear':
        return {'k': 'tclear', 'key': field(rec['a'], 'token')}
    if c == 'timer.throw':
        return {'k': 'tthrow', 'o': rec['o']}
    raise ValueError('unknown trace record ' + c)


def srefs(j, out):
    """(label, v) of every session occurrence in a JV, not descending into a session (as the host's write-through)."""
    if isinstance(j, list):
        for x in j:
            srefs(x, out)
    elif isinstance(j, dict):
        if 'sref' in j and 'v' in j:
            out.append((j['sref'], j['v']))
        else:
            for x in j.values():
                srefs(x, out)
    return out


def budget_of(v):
    for k, x in v.get('obj', []):
        if k == 'budget':
            return ('set', x)
    return ('del', None)


class Attach:
    """Last budget attachment the host has seen per session label."""
    def __init__(self):
        self.seen = {}

    def note(self, j):
        for lab, v in srefs(j, []):
            self.seen[lab] = budget_of(v)

    def reads(self, j):
        items = []
        for lab, v in srefs(j, []):
            b = budget_of(v)
            if lab in self.seen and self.seen[lab] != b:
                items.append({'k': 'bset', 's': lab, 'b': b[1]} if b[0] == 'set' else {'k': 'bset', 's': lab})
            self.seen[lab] = b
        return items


def last_active(segs):
    for sg in reversed(segs):
        if sg['kind'] in ('call', 'fire'):
            return sg
    return None


def program(obs):
    trace = obs['trace']
    reqs = {r['id']: r for r in obs['requests']}
    segs = []
    cur = None
    att = Attach()

    def before_read(j, where):
        items = att.reads(j)
        if items:
            if where is None:
                raise ValueError('budget attachment changed before any core segment')
            where['items'].extend(items)

    for j, rec in enumerate(trace):
        c = rec['c']
        if c == 'call.start':
            before_read(rec.get('a'), last_active(segs))
            cur = {'kind': 'call', 'r': rec['r'], 'items': []}
            segs.append(cur)
        elif c == 'call.end':
            r = reqs[cur['r']]
            cur['terminal'] = r['terminal']
            cur['value'] = r['value']
            att.note(r['value'])
            cur = None
        elif c == 'timer.fire':
            cur = {'kind': 'fire', 'key': field(rec['a'], 'token'), 'items': []}
            segs.append(cur)
        elif c in HOST_EVENTS:
            if cur is not None and cur['kind'] == 'call':
                raise ValueError('host event inside a call')
            cur = None
            if c == 'snapshot':
                o = rec['o']
                before_read(field(o, 'sessions'), last_active(segs))
                segs.append({'kind': 'snap', 'clocks': field(o, 'clocks'), 'toolSeen': field(o, 'toolSeen')})
        else:
            if cur is None:
                raise ValueError('core record outside a call or fire: ' + c)
            # a scripted dep throw is host-owned: the item stays, the host decides that this call throws
            if c == 'findSession' and 'o' in rec:
                before_read(rec['o'], cur)
            it = item_of(rec, trace, j)
            if it is not None:
                cur['items'].append(it)
            att.note(rec.get('a'))
            att.note(rec.get('o'))
    final = {'clocks': obs['finalClocks'], 'toolSeen': obs['finalToolSeen'], 'bsets': att.reads(obs['finalSessions'])}
    return {'id': obs['id'], 'segs': segs, 'final': final}


def main(argv):
    src, dst = argv[1], argv[2]
    n = 0
    with open(src, encoding='utf-8') as f, open(dst, 'w', encoding='utf-8') as out:
        for line in f:
            if line.strip():
                obs = json.loads(line)
                out.write(json.dumps(program(obs), ensure_ascii=True) + '\n')
                n += 1
    print(f'{n} programs -> {dst}')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
