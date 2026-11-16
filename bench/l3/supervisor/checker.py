"""Independent checker for the L3-2 result packet (docs/COMMAND_SUPERVISOR_SPEC.md, boundary representation)."""

FIELDS = ['status', 'kind', 'exit_status', 'exit_known', 'stopped', 'reaped', 'truncated', 'file_error',
          'file_open', 'total', 'file_bytes']


class Bad(Exception):
    pass


def parse_packet(vals, tail_chars=65536):
    """Return a dict of the fields plus 'tail' (bytes); raise Bad on any shape or range violation."""
    if len(vals) < 12:
        raise Bad('short packet')
    p = dict(zip(FIELDS, vals[:11]))
    n = vals[11]
    if n < 0 or len(vals) != 12 + n:
        raise Bad('tail length')
    tail = vals[12:]
    if any(not 0 <= b <= 255 for b in tail):
        raise Bad('tail byte')
    p['tail'] = bytes(tail)
    if p['status'] not in (0, 1) or p['kind'] not in (1, 2, 3, 4, 5):
        raise Bad('status/kind')
    if (p['status'] == 1) != (p['kind'] == 5):
        raise Bad('status 1 iff bad input')
    for f in ('exit_known', 'reaped', 'truncated', 'file_open'):
        if p[f] not in (0, 1):
            raise Bad(f)
    if p['stopped'] not in (0, 1, 2) or p['file_error'] not in (0, 1, 2):
        raise Bad('stopped/file_error')
    if p['total'] < 0 or not 0 <= p['file_bytes'] <= p['total']:
        raise Bad('byte counts')
    if p['kind'] != 1:
        if p['exit_known'] or p['stopped'] or p['reaped'] or p['total'] or p['file_bytes'] or n:
            raise Bad('run fields on a non-run packet')
        if p['kind'] == 2 and not 1 <= p['exit_status'] <= 6:
            raise Bad('spawn error code')
        if p['kind'] in (4, 5) and (p['exit_status'] or p['file_open'] or p['file_error'] or p['truncated']):
            raise Bad('file fields before the file was touched')
    else:
        if not p['exit_known'] and p['exit_status']:
            raise Bad('unknown exit with a status')
        if p['stopped'] == 0 and not p['exit_known']:
            raise Bad('unstopped run without an exit status')
        if p['exit_known'] and not p['reaped']:
            raise Bad('known exit without reap')
    if p['file_error'] == 1 and (p['file_open'] or p['file_bytes']):
        raise Bad('create failure with an open file')
    if not p['file_open'] and p['file_error'] != 1 and p['kind'] in (1, 2, 3):
        raise Bad('closed file without a create error')
    try:
        text = p['tail'].decode('utf-8')
    except UnicodeDecodeError:
        raise Bad('tail is not UTF-8')
    if len(text.encode('utf-16-le')) // 2 > tail_chars:
        raise Bad('tail longer than tail_chars')
    return p
