"""Unit tests of compare.py with hand-made observations: python3 test_compare.py"""
import sys
sys.dont_write_bytecode = True
import copy
import json
import unittest

import compare as C
from compare import compare_observation as cmp, compare_batch, MISSING


def obs(**kw):
    o = {'id': 'X01', 'requests': [{'id': 'q1', 'terminal': 'ok', 'value': {'obj': [['k', 1]]}}],
         'trace': [{'c': 'request.start', 'r': 'q1', 'i': 'A', 'a': {'op': 'list'}},
                   {'c': 'timer.set', 'r': 'q1', 'i': 'A', 'a': {'id': 't1', 'delay': 20000}},
                   {'c': 'timer.clear', 'r': 'q1', 'i': 'A', 'a': {'id': 't1'}},
                   {'c': 'request.done', 'r': 'q1', 'i': 'A', 'a': {'terminal': 'ok'}}],
         'createdSessions': [], 'openingEvents': [], 'finalOpening': {'A': []}, 'activeTimers': [], 'activeListeners': {}, 'driver': None}
    o.update(kw)
    return o


def mod(o, f):
    o = copy.deepcopy(o)
    f(o)
    return o


class Exact(unittest.TestCase):
    def test_equal(self):
        r = cmp(obs(), obs())
        self.assertEqual(r, {'class': 'exact', 'path': None, 'expected': None, 'actual': None})

    def test_dict_key_order_irrelevant(self):
        a = obs()
        b = mod(a, lambda o: o.update(trace=[dict(reversed(list(t.items()))) for t in o['trace']]))
        self.assertEqual(cmp(a, b)['class'], 'exact')


class Distinctions(unittest.TestCase):
    def val(self, v):
        return obs(trace=[{'c': 'reader', 'r': 'q1', 'i': 'A', 'a': {'options': {'signal': v}}}])

    def test_pairwise_distinct(self):
        vals = [None, False, 0, 0.0, '', {'undef': 1}, MISSING, True, 1, 'null', [], {}, {'num': '8000000000000000'}]
        for i, x in enumerate(vals):
            for j, y in enumerate(vals):
                if i == j:
                    continue
                if x is MISSING:
                    e = obs(trace=[{'c': 'reader', 'r': 'q1', 'i': 'A', 'a': {'options': {}}}])
                else:
                    e = self.val(x)
                if y is MISSING:
                    a = obs(trace=[{'c': 'reader', 'r': 'q1', 'i': 'A', 'a': {'options': {}}}])
                else:
                    a = self.val(y)
                r = cmp(e, a)
                self.assertEqual(r['class'], 'unexplained', (x, y))
                self.assertTrue(r['path'].startswith('trace[0].a.options.signal'), (x, y, r['path']))

    def test_missing_vs_null_marker(self):
        r = cmp(self.val(None), obs(trace=[{'c': 'reader', 'r': 'q1', 'i': 'A', 'a': {'options': {}}}]))
        self.assertEqual((r['expected'], r['actual']), (None, MISSING))

    def test_absent_a_vs_null(self):
        e = obs(trace=[{'c': 'x', 'r': 'q1', 'i': 'A'}])
        a = obs(trace=[{'c': 'x', 'r': 'q1', 'i': 'A', 'a': None}])
        r = cmp(e, a)
        self.assertEqual((r['class'], r['path'], r['expected']), ('unexplained', 'trace[0].a', MISSING))

    def test_array_order(self):
        e = obs(trace=[{'c': 'a', 'r': 'q1', 'i': 'A'}, {'c': 'b', 'r': 'q1', 'i': 'A'}])
        a = obs(trace=list(reversed(e['trace'])))
        r = cmp(e, a)
        self.assertEqual((r['class'], r['path']), ('unexplained', 'trace[0].c'))

    def test_obj_entry_order(self):
        e = obs(requests=[{'id': 'q1', 'terminal': 'ok', 'value': {'obj': [['a', 1], ['b', 2]]}}])
        a = obs(requests=[{'id': 'q1', 'terminal': 'ok', 'value': {'obj': [['b', 2], ['a', 1]]}}])
        r = cmp(e, a)
        self.assertEqual((r['class'], r['path']), ('unexplained', 'requests[0].value.obj[0][0]'))

    def test_length_difference(self):
        a = mod(obs(), lambda o: o['trace'].pop())
        r = cmp(obs(), a)
        self.assertEqual((r['path'], r['actual']), ('trace[3]', MISSING))

    def test_first_difference_is_first_in_canonical_order(self):
        a = mod(obs(), lambda o: (o['trace'][3]['a'].update(terminal='wire'), o['requests'][0].update(terminal='wire')))
        self.assertEqual(cmp(obs(), a)['path'], 'requests[0].terminal')

    def test_whole_observation(self):
        for field, f in (('activeTimers', lambda o: o['activeTimers'].append({'id': 1, 'delay': 5, 'due': 5})),
                         ('activeListeners', lambda o: o['activeListeners'].update(h=1)),
                         ('finalOpening', lambda o: o['finalOpening']['A'].append('k')),
                         ('openingEvents', lambda o: o['openingEvents'].append({'i': 'A'})),
                         ('createdSessions', lambda o: o['createdSessions'].append({'id': 'n'})),
                         ('driver', lambda o: o.update(driver={'violations': ['v']})),
                         ('id', lambda o: o.update(id='Y'))):
            r = cmp(obs(), mod(obs(), f))
            self.assertEqual(r['class'], 'unexplained', field)
            self.assertTrue(r['path'].startswith(field), (field, r['path']))

    def test_int_float_bool_strict(self):
        self.assertEqual(cmp(obs(activeTimers=[{'due': 1}]), obs(activeTimers=[{'due': 1.0}]))['class'], 'unexplained')
        self.assertEqual(cmp(obs(activeTimers=[{'due': 1}]), obs(activeTimers=[{'due': True}]))['class'], 'unexplained')

    def test_path_quoting(self):
        e = obs(activeListeners={'a b': 1})
        r = cmp(e, obs(activeListeners={'a b': 2}))
        self.assertEqual(r['path'], 'activeListeners["a b"]')


class Refs(unittest.TestCase):
    def mk(self, n1, n2):
        return obs(requests=[{'id': 'q1', 'terminal': 'thrown', 'value': {'ref': n1}}, {'id': 'q2', 'terminal': 'thrown', 'value': {'ref': n2}}],
                   trace=[{'c': 'x', 'r': 'q1', 'i': 'A', 'a': {'e': {'ref': n1}}}])

    def test_renamed_consistently(self):
        self.assertEqual(cmp(self.mk('e1', 'e2'), self.mk('zz', 'yy'))['class'], 'exact')

    def test_inconsistent_rename(self):
        # same name twice in expected, two names in actual
        r = cmp(self.mk('e1', 'e2'), mod(self.mk('e1', 'e2'), lambda o: o['trace'][0]['a']['e'].update(ref='e2')))
        self.assertEqual((r['class'], r['path']), ('unexplained', 'trace[0].a.e.ref'))

    def test_shared_map_across_fields(self):
        # first appearance counts over the entire observation, not per field
        e = obs(requests=[{'id': 'q1', 'terminal': 'thrown', 'value': {'ref': 'a'}}], trace=[{'c': 'x', 'r': 'q1', 'i': 'A', 'a': {'ref': 'b'}}])
        a = obs(requests=[{'id': 'q1', 'terminal': 'thrown', 'value': {'ref': 'b'}}], trace=[{'c': 'x', 'r': 'q1', 'i': 'A', 'a': {'ref': 'b'}}])
        self.assertEqual(cmp(e, a)['class'], 'unexplained')

    def test_other_names_not_renamed(self):
        e = obs(trace=[{'c': 'abort', 'r': 'q1', 'i': 'A', 'a': {'signal': 'sig1', 'reason': {'cancelReason': 'h1'}}}])
        a = mod(e, lambda o: o['trace'][0]['a'].update(signal='sig2'))
        self.assertEqual(cmp(e, a)['path'], 'trace[0].a.signal')
        a = mod(e, lambda o: o['trace'][0]['a']['reason'].update(cancelReason='h2'))
        self.assertEqual(cmp(e, a)['class'], 'unexplained')
        a = mod(e, lambda o: o.update(requests=[dict(o['requests'][0], id='q9')]))
        self.assertEqual(cmp(e, a)['path'], 'requests[0].id')


def native(msg):
    return obs(requests=[{'id': 'q1', 'terminal': 'native', 'value': {'native': {'name': 'Error', 'message': msg}}}],
               trace=[{'c': 'reader', 'r': 'q1', 'i': 'A', 'a': {'op': 'read'}, 'o': {'native': {'name': 'Error', 'message': msg}}}])


class P5(unittest.TestCase):
    POL = {'P5': {'paths': ['requests[*].value.native.message', 'trace[*].o.native.message']}}

    def test_listed_prose(self):
        r = cmp(native('ENOENT: no such file'), native('No such file or directory'), self.POL)
        self.assertEqual(r['class'], 'P5')
        self.assertEqual(r['path'], 'requests[0].value.native.message')

    def test_no_policy(self):
        self.assertEqual(cmp(native('a'), native('b'))['class'], 'unexplained')

    def test_other_field_differs(self):
        b = mod(native('b'), lambda o: o['requests'][0]['value']['native'].update(name='TypeError'))
        self.assertEqual(cmp(native('a'), b, self.POL)['class'], 'unexplained')

    def test_unlisted_path(self):
        b = mod(native('b'), lambda o: o['trace'][0].update(a={'op': 'list'}))
        self.assertEqual(cmp(native('a'), b, self.POL)['class'], 'unexplained')

    def test_not_prose_value(self):
        b = mod(native('a'), lambda o: o['requests'][0]['value']['native'].update(message=5))
        self.assertEqual(cmp(native('a'), b, self.POL)['class'], 'unexplained')

    def test_wire_message_not_excused(self):
        e = obs(requests=[{'id': 'q1', 'terminal': 'wire', 'value': {'wire': {'key': 'k', 'message': 'm1'}}}])
        a = obs(requests=[{'id': 'q1', 'terminal': 'wire', 'value': {'wire': {'key': 'k', 'message': 'm2'}}}])
        self.assertEqual(cmp(e, a, self.POL)['class'], 'unexplained')

    def test_empty_policy(self):
        self.assertEqual(cmp(native('a'), native('b'), {'P5': {'paths': []}})['class'], 'unexplained')


def many(n):
    reqs = [{'id': 'q%d' % i, 'terminal': 'ok', 'value': {'obj': [['sessionId', 'new-%d' % i]]}} for i in range(1, n + 1)]
    tr = []
    for i in range(1, n + 1):
        tr += [{'c': 'request.start', 'r': 'q%d' % i, 'i': 'A', 'a': {'op': 'open'}}, {'c': 'prim', 'r': 'q%d' % i, 'i': 'A', 'a': ['homedir']},
               {'c': 'request.done', 'r': 'q%d' % i, 'i': 'A', 'a': {'terminal': 'ok'}}]
    ev = [{'i': 'A', 'r': 'q%d' % i, 'op': 'add', 'key': 'k%d' % i, 'result': True} for i in range(1, n + 1)]
    cs = [{'id': 'new-%d' % i} for i in range(1, n + 1)]
    return obs(id='P08', requests=reqs, trace=tr, openingEvents=ev, createdSessions=cs)


def limited(o, limit=32):
    reqs = []
    for r in o['requests']:
        n = int(r['id'][1:])
        reqs.append(r if n <= limit else {'id': r['id'], 'terminal': 'core', 'value': {'code': 'InputTooLarge'}})
    keep = lambda t: int(t['r'][1:]) <= limit
    return mod(o, lambda x: x.update(requests=reqs, trace=[t for t in o['trace'] if keep(t)] + [t for t in o['trace'] if not keep(t) and t['c'] != 'prim'],
                                     openingEvents=[t for t in o['openingEvents'] if keep(t)], createdSessions=o['createdSessions'][:limit]))


class P6(unittest.TestCase):
    POL = C.default_policy('P08')

    def test_p08_unbounded_unexplained_without_trigger(self):
        r = cmp(many(33), many(33), self.POL)
        self.assertEqual(r['class'], 'unexplained')
        self.assertIn('not enforced', r['actual'])

    def test_p08_p6(self):
        r = cmp(many(33), limited(many(33)), self.POL)
        self.assertEqual(r['class'], 'P6')

    def test_p08_no_policy_unexplained(self):
        self.assertEqual(cmp(many(33), limited(many(33)))['class'], 'unexplained')

    def test_p08_wrong_extra_difference(self):
        a = limited(many(33))
        a['trace'][1]['a'] = ['other']
        self.assertEqual(cmp(many(33), a, self.POL)['class'], 'unexplained')

    def test_p08_rejects_too_early(self):
        self.assertEqual(cmp(many(33), limited(many(33), 31), self.POL)['class'], 'unexplained')

    def test_p08_effect_of_rejected(self):
        a = limited(many(33))
        a['trace'].append({'c': 'reader', 'r': 'q33', 'i': 'A'})
        self.assertEqual(cmp(many(33), a, self.POL)['class'], 'unexplained')

    def held(self, n, limit, settle_missing=None, counter_shift=0):
        """n requests each with one reader hold and a sessionList counter; the driver settles every hold."""
        def build(rej, missing=None):
            reqs, tr = [], []
            ctr = 0
            for i in range(1, n + 1):
                r = 'q%d' % i
                tr.append({'c': 'request.start', 'r': r, 'i': 'A', 'a': {'op': 'open'}})
                if i in rej:
                    reqs.append({'id': r, 'terminal': 'core', 'value': {'code': 'InputTooLarge'}})
                    tr.append({'c': 'request.done', 'r': r, 'i': 'A', 'a': {'terminal': 'core'}})
                    continue
                ctr += 1
                tr += [{'c': 'sessionList', 'r': r, 'i': 'A', 'a': {'call': ctr, 'size': 0}},
                       {'c': 'reader', 'r': r, 'i': 'A', 'a': {'engine': 'claude', 'op': 'read'}}]
                reqs.append({'id': r, 'terminal': 'ok'})
            for i in range(1, n + 1):
                if i in rej or i == missing:
                    continue
                tr.append({'c': 'settle', 'r': None, 'i': None, 'a': {'label': 'r' if i == 1 else 'r#%d' % i, 'call': 'reader:claude.read'}, 'o': {'ok': True}})
                tr.append({'c': 'sessionList', 'r': 'q%d' % i, 'i': 'A', 'a': {'call': ctr + i + counter_shift, 'size': 0}})
                tr.append({'c': 'request.done', 'r': 'q%d' % i, 'i': 'A', 'a': {'terminal': 'ok'}})
            return obs(id='P08', requests=reqs, trace=tr, openingEvents=[], createdSessions=[])
        rej = set(range(limit + 1, n + 1))
        return build(set()), build(rej, settle_missing)

    def test_p08_rejected_hold_and_counters(self):
        e, a = self.held(5, 3)
        pol = {'P6': {'trigger': {'name': 'active-requests', 'limit': 3, 'reduced': True}}}
        self.assertEqual(cmp(e, a, pol)['class'], 'P6')

    def test_p08_missing_settle_of_kept_request(self):
        e, a = self.held(5, 3, settle_missing=2)
        pol = {'P6': {'trigger': {'name': 'active-requests', 'limit': 3, 'reduced': True}}}
        self.assertEqual(cmp(e, a, pol)['class'], 'unexplained')

    def test_renumber_keeps_sharing(self):
        t = [{'c': 'sessionList', 'a': {'call': 5}}, {'c': 'sessionList.next', 'a': {'call': 9}}, {'c': 'sessionList.done', 'a': {'call': 5}}]
        u = [{'c': 'sessionList', 'a': {'call': 5}}, {'c': 'sessionList.next', 'a': {'call': 5}}, {'c': 'sessionList.done', 'a': {'call': 5}}]
        self.assertNotEqual(C._renumber(t), C._renumber(u))

    def test_p08_bad_limit(self):
        pol = {'P6': {'trigger': {'name': 'active-requests', 'limit': 31}}}
        self.assertEqual(cmp(many(33), limited(many(33)), pol)['class'], 'unexplained')

    def test_reduced_budget(self):
        pol = {'P6': {'trigger': {'name': 'active-requests', 'limit': 3, 'reduced': True}}}
        self.assertEqual(cmp(many(5), limited(many(5), 3), pol)['class'], 'P6')

    def test_unnamed_trigger(self):
        self.assertEqual(cmp(many(33), limited(many(33)), {'P6': {'trigger': {'name': 'whatever', 'limit': 32}}})['class'], 'unexplained')

    def bounded(self, extra_trace=(), timers=None):
        e = obs(requests=[{'id': 'q1', 'terminal': 'ok', 'value': {'obj': []}}],
                trace=[{'c': 'request.start', 'r': 'q1', 'i': 'A'}, {'c': 'copy', 'r': 'q1', 'i': 'A'}, {'c': 'rm', 'r': 'q1', 'i': 'A'},
                       {'c': 'request.done', 'r': 'q1', 'i': 'A'}])
        a = obs(requests=[{'id': 'q1', 'terminal': 'core', 'value': {'code': 'InputTooLarge'}}],
                trace=[{'c': 'request.start', 'r': 'q1', 'i': 'A'}, {'c': 'copy', 'r': 'q1', 'i': 'A'}] + list(extra_trace),
                activeTimers=timers or [])
        return e, a

    def test_packet_words_prefix(self):
        e, a = self.bounded([{'c': 'rm', 'r': 'q1', 'i': 'A'}])
        pol = {'P6': {'trigger': {'name': 'packet-words', 'limit': 16777216}}}
        self.assertEqual(cmp(e, a, pol)['class'], 'P6')

    def test_packet_words_not_a_prefix(self):
        e, a = self.bounded([{'c': 'uuid', 'r': 'q1', 'i': 'A'}])
        pol = {'P6': {'trigger': {'name': 'packet-words', 'limit': 16777216}}}
        self.assertEqual(cmp(e, a, pol)['class'], 'unexplained')

    def test_missing_cleanup(self):
        e, a = self.bounded([{'c': 'timer.set', 'r': 'q1', 'i': 'A'}])
        e['trace'].insert(2, {'c': 'timer.set', 'r': 'q1', 'i': 'A'})
        pol = {'P6': {'trigger': {'name': 'sequence', 'limit': 1000000}}}
        r = cmp(e, a, pol)
        self.assertEqual(r['class'], 'unexplained')
        self.assertIn('cleanup', r['path'])
        e, a = self.bounded([], timers=[{'id': 1, 'delay': 1, 'due': 1}])
        self.assertEqual(cmp(e, a, pol)['class'], 'unexplained')

    def test_no_input_too_large(self):
        e, a = self.bounded()
        a['requests'][0] = {'id': 'q1', 'terminal': 'ok', 'value': {'obj': [['x', 1]]}}
        self.assertEqual(cmp(e, a, {'P6': {'trigger': {'name': 'sequence', 'limit': 1000000}}})['class'], 'unexplained')


class Excluded(unittest.TestCase):
    def test_named(self):
        e = obs(requests=[{'id': 'q1', 'terminal': 'ok', 'value': {'obj': [['v', 'a']]}}])
        a = obs(requests=[{'id': 'q1', 'terminal': 'ok', 'value': {'obj': [['v', 'b']]}}])
        pol = {'excluded-domain': {'name': 'forbidden-value', 'paths': ['requests[*].value'], 'reached': True, 'boundary': 'ok'}}
        self.assertEqual(cmp(e, a, pol)['class'], 'excluded-domain')
        self.assertEqual(cmp(e, a, {'excluded-domain': {'paths': ['requests[*].value'], 'reached': True}})['class'], 'unexplained')
        self.assertEqual(cmp(e, a, {'excluded-domain': {'name': 'n', 'paths': ['requests[*].value']}})['class'], 'unexplained')
        self.assertEqual(cmp(e, mod(a, lambda o: o['trace'].pop()), pol)['class'], 'unexplained')


class Engine(unittest.TestCase):
    E = native('a')
    A = native('b')
    POL = {'engine-policy': {'id': 'EP1', 'trigger': 't', 'fields': ['requests[*].value'], 'realRunner': True}}

    def test_real_runner(self):
        a = mod(self.E, lambda o: o['requests'][0]['value']['native'].update(message='b'))
        self.assertEqual(cmp(self.E, a, self.POL)['class'], 'engine-policy')

    def test_common_fake_never(self):
        a = mod(self.E, lambda o: o['requests'][0]['value']['native'].update(message='b'))
        self.assertEqual(cmp(self.E, a, dict(self.POL, commonFake=True))['class'], 'unexplained')
        pol = {'engine-policy': dict(self.POL['engine-policy'], realRunner=False)}
        self.assertEqual(cmp(self.E, a, pol)['class'], 'unexplained')
        self.assertEqual(cmp(self.E, a, {'engine-policy': {'realRunner': True}})['class'], 'unexplained')


class Batch(unittest.TestCase):
    def setUp(self):
        self.e = [obs(id='A1'), obs(id='A2')]

    def run_b(self, actual, **kw):
        return compare_batch(self.e, actual, **kw)

    def test_ok(self):
        r = self.run_b([obs(id='A1'), json.dumps(obs(id='A2'))])
        self.assertTrue(r['ok'])
        self.assertEqual([x['id'] for x in r['results']], ['A1', 'A2'])
        self.assertEqual(r['counts']['exact'], 2)

    def reason(self, actual):
        r = self.run_b(actual)
        self.assertFalse(r['ok'])
        return [x.get('reason') for x in r['results'] if x['class'] == 'unexplained']

    def test_missing(self):
        self.assertEqual(self.reason([obs(id='A1')]), ['missing'])

    def test_crash(self):
        self.assertEqual(self.reason(RuntimeError('boom')), ['crash', 'crash'])
        self.assertEqual(self.reason([obs(id='A1'), None.__class__ and RuntimeError('x')]), ['crash'])

    def test_malformed(self):
        self.assertEqual(self.reason([obs(id='A1'), '{nope']), ['malformed'])
        self.assertEqual(self.reason([obs(id='A1'), {'id': 'A2'}]), ['malformed'])

    def test_order_and_ids(self):
        self.assertEqual(self.reason([obs(id='A2'), obs(id='A1')]), ['id-mismatch', 'id-mismatch'])

    def test_extra(self):
        self.assertEqual(self.reason([obs(id='A1'), obs(id='A2'), obs(id='A3')]), ['extra'])

    def test_driver(self):
        self.assertEqual(self.reason([obs(id='A1'), obs(id='A2', driver={'violations': ['x']})]), ['driver'])

    def test_internal_invariant(self):
        a = obs(id='A2', requests=[{'id': 'q1', 'terminal': 'core', 'value': {'code': 'InternalInvariant'}}])
        self.assertEqual(self.reason([obs(id='A1'), a]), ['internal-invariant'])

    def test_difference_counts(self):
        r = self.run_b([obs(id='A1'), mod(obs(id='A2'), lambda o: o['trace'].pop())])
        self.assertEqual(r['counts']['unexplained'], 1)
        self.assertEqual(r['results'][1]['path'], 'trace[3]')

    def test_default_policy_p08(self):
        r = compare_batch([many(33)], [limited(many(33))])
        self.assertEqual(r['counts']['P6'], 1)
        self.assertTrue(r['ok'])
        r = compare_batch([many(33)], [many(33)])
        self.assertFalse(r['ok'])

    def test_policies_callable(self):
        r = compare_batch([native('a')], [native('b')], policies=lambda i, e: P5.POL)
        self.assertEqual(r['counts']['P5'], 1)


if __name__ == '__main__':
    unittest.main(verbosity=1)
