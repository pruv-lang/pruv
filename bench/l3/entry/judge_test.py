"""Self-check of the strict excluded-domain judgement (audit R8): an InvalidPacket with the oracle trace is excluded-domain
only when forbidden_reached(case) holds; an excluded or never-reached forbidden session stays unexplained."""
import sys
from common import compare, forbidden_reached

BAD = {'id': 's0', 'externalThread': {'engine': {'ref': 'badeng'}, 'thread': 'x'}}
ORACLE = {'id': 'X', 'terminal': 'ok', 'value': [], 'trace': [['isTemporary', 's0']]}
ACTUAL = {'id': 'X', 'terminal': 'boundary_error', 'value': {'code': 'InvalidPacket'}, 'trace': [['isTemporary', 's0']]}


def mk(sessions, answers):
    return {'op': 'bound_session_index', 'tag': 'excluded_ok', 'args': {'sessions': sessions},
            'script': {'isTemporary': answers}}


ROWS = [
    ('forbidden session excluded by isTemporary', mk([BAD], [{'ok': True}]), 'unexplained'),
    ('forbidden session excluded by temporary === true', mk([dict(BAD, temporary=True)], []), 'unexplained'),
    ('earlier isTemporary answer throws', mk([{'id': 's1'}, BAD], [{'throw': {}}]), 'unexplained'),
    ('non-boolean answer is not modelled', mk([BAD], [{'ok': {'undef': 1}}]), 'unexplained'),
    ('untagged case', dict(mk([BAD], [{'ok': False}]), tag=None), 'unexplained'),
    ('reached with answer false', mk([BAD], [{'ok': False}]), 'excluded-domain'),
    ('reached with default answer', mk([BAD], []), 'excluded-domain'),
    ('reached after a temporary session', mk([{'id': 's1', 'temporary': True}, BAD], [{'ok': False}]), 'excluded-domain'),
]
fails = 0
for name, case, want in ROWS:
    got = compare(ORACLE, ACTUAL, excluded=forbidden_reached(case))[0]
    print('%-50s %-16s %s' % (name, got, 'ok' if got == want else 'FAIL (want %s)' % want))
    fails += got != want
print('failures: %d' % fails)
sys.exit(1 if fails else 0)
