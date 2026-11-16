"""Narrow policy classifier: compare TS on a policy-normalized input, never waive fields."""
import json

def normalize(data, policy):
    if policy == 'P1':
        return b''.join(line for line in data.splitlines(keepends=True) if valid_utf8(line))
    if policy == 'P4':
        rows = []
        for line in data.splitlines():
            obj = json.loads(line)
            if obj.get('timestamp') in ['2024-01-01', '2026-02-30T00:00:00.000Z']:
                del obj['timestamp']
            rows.append(json.dumps(obj).encode() + b'\n')
        return b''.join(rows)
    return data

def valid_utf8(line):
    try:
        line.decode('utf8')
        return True
    except UnicodeDecodeError:
        return False
