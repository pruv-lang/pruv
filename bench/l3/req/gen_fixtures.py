"""Writes the 160 fixed L3-9 fixtures (fixtures/<G>.json), ten per group, names per docs/ENGINE_IMPORT_REQUEST_SPEC.md section 8.
Each fixture: id, name, rule (spec section), world, instances?, script, expect (explicit assertions on the oracle observation), killer?, layer?.
Format: SCENARIO.md. Run: python3 gen_fixtures.py   (then python3 check_oracle.py)."""
import sys
sys.dont_write_bytecode = True
import copy
import json

from common import FIXTURES, GROUPS

ISO = '2023-11-14T22:13:20.000Z'
ACC1 = 'acc_0123456789abcdef'
ACC2 = 'acc_fedcba9876543210'
NAMES = {
    'S': 'home-order account-label nullish-label duplicate-source-first unknown-source source-throw no-home-leak fresh-per-request ambient-codex-home invalid-account-skipped',
    'F': 'folder-normalized folder-error-message folder-empty-message folder-nonerror folder-abort-name source-before-folder folder-before-same folder-before-known list-folder-no-timer open-folder-no-lock',
    'B': 'external-original external-fork terminal-default-codex slot-account-prefix first-session temporary-literal-true temporary-callback early-stop fresh-list-per-item empty-session-id',
    'L': 'unbound-title bound-title duplicate-title-last empty-title-fallback temporary-title-in-map zero-metadata undefined-metadata null-metadata drop-extra-fields truthy-maybeopen',
    'A': 'same-home same-account-refused same-grok-refused same-ai-refused fork-no-app-action grok-target-valid grok-target-unknown grok-default-target nongrok-target-ignored account-before-known',
    'O': 'new-session existing-session existing-preaborted empty-turns same-no-fork nongrok-fork grok-no-fork seed-account model-wanted fill-preserve-slots',
    'T': 'uuid-order single-now created-missing created-empty created-null completed-present tools-zero-positive clipped-truthy omitted-before last-plan-preserved',
    'U': 'external-title first-prompt empty-fallback whitespace-title-no-prompt-retry unicode-whitespace nonwhitespace-0085-200b utf16-split80 basename-fallback root-folder-fallback long-folder-unclipped',
    'G': 'copy-success home-error-wrap home-nonerror-wrap copy-exists copy-reject-no-handler-rm verify-success verify-reject verify-abort copy-cancel-signal copy-returned-path',
    'R': 'race-rm race-rm-reject cancel-after-copy race-query-throws date-throws uuid-throws model-throws create-throws-after-fill success-keeps-copy catch-rm-reject',
    'E': 'not-found codex-unsupported grok-exists grok-load-failed timeout engine-missing read-failed plain-abort-name unknown-eie-code passthrough-value',
    'C': 'preabort-unbound cancel-during-read deadline-read-reject deadline-read-ignored deadline-options verify-new-deadline cancel-reason-precedence cancel-during-race-rm cancel-during-catch-rm timer-listener-cleanup',
    'D': 'codex-list codex-open codex-prepare-throw opencode-list-no-locate opencode-open-binary opencode-no-binary locate-reject temp-root-reject env-signal-identity claude-qwen-no-preparation',
    'I': 'same-key-busy known-before-busy bind-during-read bind-after-copy different-keys different-folders-same-key different-sources-same-key different-instances release-after-failure no-release-without-acquire',
    'P': 'bad-version bad-kind wrong-request-id wrong-seq wrong-call-kind done-reentry packet-too-large active-request-limit sequence-limit resource-cleanup-after-copy',
    'X': 'empty-list-still-title-scan title-scan-throws item-scan-throws app-action-throws grok-accounts-throws abort-name-before-eie-code more-than50-items more-than200-turns duplicate-session-title-and-first-bound migrate-fork-cancel-contract',
}
NAMES = {g: v.split() for g, v in NAMES.items()}
KILLERS = {'S05': 'M01', 'F03': 'M02', 'B05': 'M03', 'L03': 'M04', 'L04': 'M05', 'A04': 'M06', 'A07': 'M07', 'I06': 'M08', 'I09': 'M09', 'C03': 'M10',
           'E08': 'M11', 'C07': 'M12', 'I03': 'M13', 'R01': 'M14', 'U07': 'M15', 'O08': 'M16'}
REGISTRY = {g: [] for g in GROUPS}


# ---- builders -----------------------------------------------------------------------------------------------------------
def K(suffix): return 'wireWork.engineImport.' + suffix
def err(ref, cls='Error', **kw): return {'class': cls, 'ref': ref, **kw}
def abort_err(ref='ab'): return err(ref, 'plain', name='AbortError')
def eie(ref, code, **kw): return err(ref, 'EngineImportError', code=code, **kw)
def UU(n): return f'00000000-0000-4000-8000-{n:012d}'
def turn(prompt='p1', reply='r1', tools=0, clipped=False, **kw): return {'prompt': prompt, 'reply': reply, 'tools': tools, 'clipped': clipped, **kw}
def record(title='T', turns=None, **kw): return {'title': title, 'turns': [turn()] if turns is None else turns, **kw}
def item(thread, title='Item', **kw): return {'thread': thread, 'title': title, **kw}
def sess(id, title='', **kw): return {'id': id, 'title': title, **kw}
def ext(engine, thread, forks=None, mode='fork', source=None):
    out = {'engine': engine, 'source': source or f'{engine}:home', 'thread': thread, 'mode': mode, 'importedAt': ISO}
    if forks is not None: out['forks'] = forks
    return out
def bound(id, engine, thread, title='', **kw): return sess(id, title, externalThread=ext(engine, thread), **kw)
def slots(mapping): return {k: {'threadId': v} for k, v in mapping.items()}
def readers(engine, read=None, list=None):
    out = {}
    if read is not None: out['read'] = read
    if list is not None: out['list'] = list
    return {'readers': {engine: out}}
def merge(*worlds):
    out = {}
    for w in worlds:
        for k, v in w.items():
            if k == 'readers':
                out.setdefault('readers', {})
                for e, ops in v.items(): out['readers'].setdefault(e, {}).update(ops)
            else:
                out[k] = v
    return out
def okread(engine='claude', rec=None): return readers(engine, read=[{'ok': rec if rec is not None else record()}])
def gworld(rec=None, copy_entry=None, **kw):
    return merge(okread('grok', rec), {'copy': [copy_entry or {'ok': {'path': '/grok/copy1', 'skipped': 0}, 'verify': True}]}, kw)
COPY_DEFER = {'ok': {'path': '/grok/copy1', 'skipped': 0}, 'verify': True, 'defer': 'cp'}

def OPEN(req, source='claude:home', thread='t1', folder='/proj', mode='fork', target=None, **kw):
    inp = {'source': source, 'folder': folder, 'thread': thread, 'mode': mode}
    if target is not None: inp['targetAccount'] = target
    return {'ev': 'start', 'req': req, 'op': 'open', 'input': inp, **kw}
def LIST(req, source='claude:home', folder='/proj', **kw): return {'ev': 'start', 'req': req, 'op': 'list', 'input': {'source': source, 'folder': folder}, **kw}
def SRC(req): return {'ev': 'start', 'req': req, 'op': 'sources'}
def SETTLE(label, **kw): return {'ev': 'settle', 'label': label, **kw}
def ADV(ms): return {'ev': 'advance', 'ms': ms}
def CANCEL(handle, reason=None):
    return {'ev': 'cancel', 'handle': handle, **({'reason': reason} if reason is not None else {})}
def MUT_ADD(session): return {'ev': 'mutate', 'op': 'add', 'session': session}

def wire_req(suffix, message=None, **kw):
    out = {'terminal': 'wire', 'key': K(suffix), **kw}
    if message is not None: out['message'] = message
    return out
def ok_new(sid='new-1'): return {'terminal': 'ok', 'value': {'sessionId': sid}, 'keys': ['sessionId']}
def ok_existing(sid): return {'terminal': 'ok', 'value': {'sessionId': sid, 'existing': True}, 'keys': ['sessionId', 'existing']}
def thrown(ref): return {'terminal': 'thrown', 'ref': ref}
def cancelled(handle, **kw): return {'terminal': 'thrown', 'reason': handle, **kw}
def rc(c, **kw): return {'c': c, **kw}                      # trace record spec
def reader_rec(op=None, **kw): return {'c': 'reader', **({'a': {'op': op, **kw}} if op or kw else {})}
def zero(*names): return {n: 0 for n in names}


def collect_errs(node, out):
    if isinstance(node, dict):
        if isinstance(node.get('class'), str) and isinstance(node.get('ref'), str): out[node['ref']] = node
        for v in node.values(): collect_errs(v, out)
    elif isinstance(node, list):
        for v in node: collect_errs(v, out)


def F(group, n, rule, world, script, expect, instances=None, layer=None, note=None):
    fid = f'{group}{n:02d}'
    assert len(REGISTRY[group]) == n - 1, fid
    fx = {'id': fid, 'name': NAMES[group][n - 1], 'rule': rule}
    if layer: fx['layer'] = layer
    if fid in KILLERS: fx['killer'] = KILLERS[fid]
    if note: fx['note'] = note
    world = copy.deepcopy(world)
    errs = {}
    collect_errs(world, errs); collect_errs(script, errs)
    if errs and 'errs' not in world: world['errs'] = [errs[k] for k in sorted(errs)]
    elif errs: world['errs'] = [*world['errs'], *[v for k, v in sorted(errs.items()) if v not in world['errs']]]
    fx['world'] = world
    if instances: fx['instances'] = instances
    fx['script'] = script
    fx['expect'] = expect
    REGISTRY[group].append(fx)


# ==== S: sources ===================================================================================================================
def group_S():
    src5 = [{'source': 'claude:home', 'engine': 'claude', 'label': ''}, {'source': 'codex:home', 'engine': 'codex', 'label': ''}, {'source': 'grok:home', 'engine': 'grok', 'label': ''},
            {'source': 'opencode:home', 'engine': 'opencode', 'label': ''}, {'source': 'qwen:home', 'engine': 'qwen', 'label': ''}]
    F('S', 1, '3.1', {}, [SRC('r1')], {'req': {'r1': {'terminal': 'ok', 'value': {'sources': src5}, 'keys': ['sources']}}, 'counts': zero('timer.set', 'checkFolder', 'sessionList')})
    F('S', 2, '3.1', {'accounts': {'claude': {'ok': [{'id': ACC1, 'name': 'Work'}]}, 'codex': {'ok': [{'id': ACC2, 'name': 'Personal'}]}}}, [SRC('r1')],
      {'req': {'r1': {'terminal': 'ok', 'value': {'sources': [
          {'source': 'claude:home', 'engine': 'claude', 'label': ''}, {'source': f'claude:{ACC1}', 'engine': 'claude', 'account': ACC1, 'label': 'Work'},
          {'source': 'codex:home', 'engine': 'codex', 'label': ''}, {'source': f'codex:{ACC2}', 'engine': 'codex', 'account': ACC2, 'label': 'Personal'},
          *src5[2:]]}}}})
    F('S', 3, '3.1', {'accounts': {'claude': {'ok': [{'id': ACC1, 'name': None}, {'id': 'acc_1111111111111111', 'name': {'undef': 1}}, {'id': 'acc_2222222222222222', 'name': ''}]}}}, [SRC('r1')],
      {'req': {'r1': {'terminal': 'ok', 'value': {'sources': [
          src5[0], {'source': f'claude:{ACC1}', 'engine': 'claude', 'account': ACC1, 'label': ''}, {'source': 'claude:acc_1111111111111111', 'engine': 'claude', 'account': 'acc_1111111111111111', 'label': ''},
          {'source': 'claude:acc_2222222222222222', 'engine': 'claude', 'account': 'acc_2222222222222222', 'label': ''}, *src5[1:]]}}}})
    F('S', 4, '3.1', {'accounts': {'claude': {'ok': [{'id': ACC1, 'name': 'First'}, {'id': ACC1, 'name': 'Second'}]}}}, [SRC('r1')],
      {'req': {'r1': {'terminal': 'ok', 'value': {'sources': [
          src5[0], {'source': f'claude:{ACC1}', 'engine': 'claude', 'account': ACC1, 'label': 'First'}, {'source': f'claude:{ACC1}', 'engine': 'claude', 'account': ACC1, 'label': 'Second'}, *src5[1:]]}}}},
      note='Duplicates are kept in order; list/open take the first match (the first account name is not observable through list/open).')
    F('S', 5, '3.1', {}, [LIST('r1', source='claude:nope')],
      {'req': {'r1': wire_req('sourceUnknown', '고를 수 없는 출처야.')}, 'counts': zero('reader', 'checkFolder', 'sessionList', 'timer.set', 'makeCodexRunDeps')})
    F('S', 6, '3.1', {'accounts': {'claude': {'throw': err('e1', message='accounts down')}}}, [LIST('r1'), OPEN('r2')],
      {'req': {'r1': thrown('e1'), 'r2': thrown('e1')}, 'counts': zero('checkFolder', 'reader', 'timer.set', 'sessionList', 'opening.add')})
    F('S', 7, '3.1', {'env': {'CLAUDE_CONFIG_DIR': '/secret/claude', 'GROK_HOME': '/secret/grok'}, 'accounts': {'claude': {'ok': [{'id': ACC1, 'name': 'W'}]}}}, [SRC('r1')],
      {'req': {'r1': {'terminal': 'ok', 'keys': ['sources'], 'notContains': ['"home"', '"kind"', '"accountName"', '/secret', 'engine-homes']}}})
    F('S', 8, '3.1', {'accounts': {'claude': {'ok': [{'id': ACC1, 'name': 'W'}]}}}, [SRC('r1'), SRC('r2'), SRC('r3')],
      {'req': {'r1': {'terminal': 'ok'}, 'r2': {'terminal': 'ok'}, 'r3': {'terminal': 'ok'}},
       'matchCounts': [[{'c': 'prim', 'a': ['accounts', 'claude']}, 3], [{'c': 'prim', 'a': ['accounts', 'codex']}, 3]]},
      note='world.accounts is not a queue: freshness per request is visible only through the three accounts calls per engine.')
    F('S', 9, '3.1', {'ambient': {'ok': f'/data/engine-homes/codex/{ACC2}'}, 'accounts': {'codex': {'ok': [{'id': ACC2, 'name': 'Same as host'}, {'id': 'acc_3333333333333333', 'name': 'Other'}]}}}, [SRC('r1')],
      {'req': {'r1': {'terminal': 'ok', 'value': {'sources': [src5[0], src5[1], {'source': 'codex:acc_3333333333333333', 'engine': 'codex', 'account': 'acc_3333333333333333', 'label': 'Other'}, *src5[2:]]}}},
       'has': [{'c': 'prim', 'a': ['ambient', 'CODEX_HOME']}]})
    F('S', 10, '3.1', {'accounts': {'claude': {'ok': [{'id': 'bad id', 'name': 'x'}, {'id': ACC1, 'name': 'ok'}]}, 'codex': {'ok': [{'id': 'ACC_UPPER', 'name': 'y'}]}}}, [SRC('r1')],
      {'req': {'r1': {'terminal': 'ok', 'value': {'sources': [src5[0], {'source': f'claude:{ACC1}', 'engine': 'claude', 'account': ACC1, 'label': 'ok'}, *src5[1:]]}}}})


# ==== F: folder and preflight order ===============================================================================================
def group_F():
    F('F', 1, '3.2', merge(readers('claude', list=[{'ok': []}]), {'checkFolder': {'/p/./x': {'ok': '/p/x'}}}), [LIST('r1', folder='/p/./x')],
      {'req': {'r1': {'terminal': 'ok', 'value': {'items': []}}}, 'counts': {'checkFolder': 1}, 'has': [{'c': 'checkFolder', 'a': ['/p/./x'], 'o': {'ok': '/p/x'}}, reader_rec('list', folder='/p/x')]})
    F('F', 2, '3.2', {'checkFolder': {'*': {'throw': err('e1', message='outside project')}}}, [LIST('r1'), OPEN('r2')],
      {'req': {'r1': wire_req('folder', 'outside project'), 'r2': wire_req('folder', 'outside project')}, 'counts': zero('reader', 'timer.set', 'sessionList')})
    F('F', 3, '3.2', {'checkFolder': {'*': {'throw': err('e1', message='')}}}, [LIST('r1'), OPEN('r2')],
      {'req': {'r1': wire_req('folder', ''), 'r2': wire_req('folder', '')}, 'counts': zero('reader')},
      note='An empty Error message stays empty (no fallback text).')
    F('F', 4, '3.2', {'checkFolder': {'*': {'throw': err('p1', 'primitive', value='denied')}}}, [LIST('r1'), OPEN('r2')],
      {'req': {'r1': wire_req('folder', '이 폴더는 쓸 수 없어.'), 'r2': wire_req('folder', '이 폴더는 쓸 수 없어.')}, 'counts': zero('reader')})
    F('F', 5, '3.2', {'checkFolder': {'*': {'throw': err('ab', 'plain', name='AbortError', message='aborted')}}}, [OPEN('r1')],
      {'req': {'r1': wire_req('folder', '이 폴더는 쓸 수 없어.')}, 'counts': zero('reader', 'opening.add', 'timer.set')},
      note='The folder check is outside the open try: an AbortError-named value is a folder WireError, not a timeout.')
    F('F', 6, '3.2', {'checkFolder': {'*': {'throw': err('e1', message='never')}}}, [OPEN('r1', source='nope:home')],
      {'req': {'r1': wire_req('sourceUnknown')}, 'counts': zero('checkFolder')})
    F('F', 7, '3.2', {'checkFolder': {'*': {'throw': err('e1', message='bad folder')}}}, [OPEN('r1', source='grok:home', mode='same'), OPEN('r2', mode='same')],
      {'req': {'r1': wire_req('folder', 'bad folder'), 'r2': wire_req('folder', 'bad folder')}, 'counts': zero('fromAppAction', 'grokAccounts', 'sessionList')})
    F('F', 8, '3.2', {'checkFolder': {'*': {'throw': err('e1', message='bad folder')}}, 'sessions': [bound('s1', 'claude', 't1')]}, [OPEN('r1')],
      {'req': {'r1': wire_req('folder', 'bad folder')}, 'counts': zero('sessionList', 'isTemporary')})
    F('F', 9, '3.2', {'checkFolder': {'*': {'throw': err('e1', message='bad folder')}}}, [LIST('r1', source='codex:home'), LIST('r2', source='grok:home')],
      {'req': {'r1': wire_req('folder', 'bad folder'), 'r2': wire_req('folder', 'bad folder')}, 'counts': zero('timer.set', 'timer.clear', 'makeCodexRunDeps', 'reader', 'listener.add'), 'timers': 0})
    F('F', 10, '3.2', {'checkFolder': {'*': {'throw': err('e1', message='bad folder')}}}, [OPEN('r1'), OPEN('r2', thread='t2')],
      {'req': {'r1': wire_req('folder', 'bad folder'), 'r2': wire_req('folder', 'bad folder')}, 'counts': zero('opening.has', 'opening.add', 'opening.delete'), 'opening': {'A': []}, 'openingOps': []})


# ==== B: bound ===================================================================================================================
def b_open(n, sessions, expect_sid, extra_world=None, source='claude:home', rule='3.3', extra=None, steps=None, counts=None):
    ex = {'req': {'r1': ok_existing(expect_sid)}, 'counts': {'reader': 0, **(counts or {})}}
    ex.update(extra or {})
    F('B', n, rule, merge({'sessions': sessions}, extra_world or {}), steps or [OPEN('r1', source=source)], ex)

def group_B():
    b_open(1, [sess('s0'), bound('s1', 'claude', 't1')], 's1')
    b_open(2, [sess('s1', externalThread=ext('claude', 'orig', forks=['f1', 't1']))], 's1')
    F('B', 3, '3.3', merge({'sessions': [sess('s1', terminal={'cliSession': 't1'})]}, okread('claude')), [OPEN('r1', source='codex:home'), OPEN('r2', source='claude:home')],
      {'req': {'r1': ok_existing('s1'), 'r2': ok_new('new-1')}, 'counts': {'reader': 1, 'createImported': 1}})
    F('B', 4, '3.3', merge({'sessions': [sess('s1', engineThreads=slots({'claude:work': 't1'}))]}, okread('codex')), [OPEN('r1', source='claude:home'), OPEN('r2', source='codex:home')],
      {'req': {'r1': ok_existing('s1'), 'r2': ok_new('new-1')}, 'counts': {'reader': 1}})
    F('B', 5, '3.3', {'sessions': [bound('s1', 'claude', 't1'), bound('s2', 'claude', 't1')]}, [OPEN('r1')],
      {'req': {'r1': ok_existing('s1')}, 'counts': zero('reader', 'createImported')})
    F('B', 6, '3.3', {'sessions': [bound('s1', 'claude', 't1', temporary=True), bound('s2', 'claude', 't1')]}, [OPEN('r1')],
      {'req': {'r1': ok_existing('s2')}, 'counts': {'isTemporary': 1}, 'has': [{'c': 'isTemporary', 'a': ['s2']}], 'absent': [{'c': 'isTemporary', 'a': ['s1']}]})
    F('B', 7, '3.3', {'sessions': [bound('s1', 'claude', 't1'), bound('s2', 'claude', 't1')], 'isTemporary': {'s1': True}}, [OPEN('r1')],
      {'req': {'r1': ok_existing('s2')}, 'counts': {'isTemporary': 2}, 'order': [{'c': 'isTemporary', 'a': ['s1'], 'o': {'ok': True}}, {'c': 'isTemporary', 'a': ['s2']}]})
    F('B', 8, '3.3', {'sessions': [bound('s1', 'claude', 't1'), sess('s2'), sess('s3')]}, [OPEN('r1')],
      {'req': {'r1': ok_existing('s1')}, 'counts': {'isTemporary': 1, 'sessionList': 1, 'sessionList.next': 1, 'sessionList.return': 1, 'sessionList.done': 0},
       'has': [{'c': 'sessionList.return', 'a': {'consumed': 1}}]})
    F('B', 9, '3.3', merge(readers('claude', list=[{'ok': [item('t1', 'A'), item('t2', 'B')]}]), {'sessions': [bound('s1', 'claude', 't1', 'Lair A'), bound('s2', 'claude', 't2', 'Lair B')]}), [LIST('r1')],
      {'req': {'r1': {'terminal': 'ok', 'value': {'items': [{'thread': 't1', 'title': 'Lair A', 'boundSessionId': 's1'}, {'thread': 't2', 'title': 'Lair B', 'boundSessionId': 's2'}]}}},
       'counts': {'sessionList': 3}, 'order': [rc('reader'), {'c': 'sessionList', 'a': {'call': 1}}, {'c': 'sessionList', 'a': {'call': 2}}, {'c': 'sessionList', 'a': {'call': 3}}]})
    F('B', 10, '3.3', merge({'sessions': [sess('', 'Empty id', externalThread=ext('claude', 't1'))]}, okread('claude')), [OPEN('r1')],
      {'req': {'r1': ok_new('new-1')}, 'counts': {'reader': 1, 'createImported': 1, 'sessionList': 2, 'isTemporary': 2}, 'has': [{'c': 'isTemporary', 'a': ['']}]},
      note='An empty session id still stops the search but is falsy for the caller (initial search and race re-check).')


# ==== L: list =======================================================================================================================
def l_world(items, sessions=None, **kw):
    return merge(readers('claude', list=[{'ok': items}]), {'sessions': sessions or []}, kw)

def group_L():
    F('L', 1, '3.5', l_world([item('t1', 'Item A')]), [LIST('r1')],
      {'req': {'r1': {'terminal': 'ok', 'value': {'items': [{'thread': 't1', 'title': 'Item A'}]}, 'itemKeys': [['thread', 'title']]}}, 'counts': {'sessionList': 2}})
    F('L', 2, '3.5', l_world([item('t1', 'Item A')], [bound('s1', 'claude', 't1', 'Lair title')]), [LIST('r1')],
      {'req': {'r1': {'terminal': 'ok', 'value': {'items': [{'thread': 't1', 'title': 'Lair title', 'boundSessionId': 's1'}]}, 'itemKeys': [['thread', 'title', 'boundSessionId']]}}})
    F('L', 3, '3.5', l_world([item('t1', 'Item A')], [bound('s1', 'claude', 't1', 'first'), sess('s1', 'second')]), [LIST('r1')],
      {'req': {'r1': {'terminal': 'ok', 'value': {'items': [{'thread': 't1', 'title': 'second', 'boundSessionId': 's1'}]}}}})
    F('L', 4, '3.5', l_world([item('t1', 'Item A')], [bound('s1', 'claude', 't1', '')]), [LIST('r1')],
      {'req': {'r1': {'terminal': 'ok', 'value': {'items': [{'thread': 't1', 'title': 'Item A', 'boundSessionId': 's1'}]}}}})
    F('L', 5, '3.5', l_world([item('t1', 'Item A')], [bound('dup', 'claude', 't1', 'Real'), sess('dup', 'Temp title', temporary=True)]), [LIST('r1')],
      {'req': {'r1': {'terminal': 'ok', 'value': {'items': [{'thread': 't1', 'title': 'Temp title', 'boundSessionId': 'dup'}]}}}},
      note='The title map includes temporary sessions (last write wins) although the bound search excludes them.')
    F('L', 6, '3.5', l_world([item('t1', 'Z', updatedAt=0, size=0, messages=0, path='')]), [LIST('r1')],
      {'req': {'r1': {'terminal': 'ok', 'value': {'items': [{'thread': 't1', 'title': 'Z', 'updatedAt': 0, 'size': 0, 'messages': 0, 'path': ''}]}, 'itemKeys': [['thread', 'title', 'updatedAt', 'size', 'messages', 'path']]}}})
    F('L', 7, '3.5', l_world([item('t1', 'U', updatedAt={'undef': 1}, size={'undef': 1}, messages={'undef': 1}, path={'undef': 1})]), [LIST('r1')],
      {'req': {'r1': {'terminal': 'ok', 'itemKeys': [['thread', 'title']]}}})
    F('L', 8, '3.5', l_world([item('t1', 'N', updatedAt=None, size=None, messages=None, path=None)]), [LIST('r1')],
      {'req': {'r1': {'terminal': 'ok', 'value': {'items': [{'thread': 't1', 'title': 'N', 'updatedAt': None, 'size': None, 'messages': None, 'path': None}]}, 'itemKeys': [['thread', 'title', 'updatedAt', 'size', 'messages', 'path']]}}})
    F('L', 9, '3.5', l_world([item('t1', 'E', entrypoint='cli', origin='x', foo=1, kind='k', boundSessionId='fake')]), [LIST('r1')],
      {'req': {'r1': {'terminal': 'ok', 'value': {'items': [{'thread': 't1', 'title': 'E'}]}, 'itemKeys': [['thread', 'title']]}}})
    F('L', 10, '3.5', l_world([item('t1', 'A', maybeOpen=True), item('t2', 'B', maybeOpen=False), item('t3', 'C', maybeOpen=1), item('t4', 'D', maybeOpen='yes'), item('t5', 'E', maybeOpen=0), item('t6', 'F', maybeOpen={'undef': 1})]), [LIST('r1')],
      {'req': {'r1': {'terminal': 'ok', 'itemKeys': [['thread', 'title', 'maybeOpen'], ['thread', 'title'], ['thread', 'title', 'maybeOpen'], ['thread', 'title', 'maybeOpen'], ['thread', 'title'], ['thread', 'title']],
                      'value': {'items': [{'maybeOpen': True}, {'thread': 't2'}, {'maybeOpen': True}, {'maybeOpen': True}, {'thread': 't5'}, {'thread': 't6'}]}}}})


# ==== A: same mode and target account ==============================================================================================
ACCTS = {'accounts': {'claude': {'ok': [{'id': ACC1, 'name': 'Work'}]}}}
def group_A():
    F('A', 1, '3.4', merge(okread('claude')), [OPEN('r1', mode='same')],
      {'req': {'r1': ok_new('new-1')}, 'counts': {'fromAppAction': 1, 'createImported': 1}, 'order': [rc('fromAppAction'), rc('reader'), rc('createImported')],
       'createdMeta': [{'seed': {'engine': 'claude', 'folder': '/proj'}}]})
    F('A', 2, '3.4', merge(ACCTS, okread('claude')), [OPEN('r1', source=f'claude:{ACC1}', mode='same')],
      {'req': {'r1': wire_req('sameRefused', '원래 대화에 그대로 이어 쓰기는 기본 Claude·Codex·OpenCode·Qwen 에서만 돼.')}, 'counts': zero('fromAppAction', 'reader', 'sessionList', 'opening.add')})
    F('A', 3, '3.4', merge(gworld(), {'grokAccounts': [{'id': 'g1'}]}), [OPEN('r1', source='grok:home', mode='same')],
      {'req': {'r1': wire_req('sameRefused')}, 'counts': zero('fromAppAction', 'grokAccounts', 'reader', 'sessionList')})
    F('A', 4, '3.4', merge(okread('claude')), [OPEN('r1', mode='same', fromAppAction=True)],
      {'req': {'r1': wire_req('sameByAi', '그대로 이어 쓰기는 사람이 확인해야 해서 AI 는 쓸 수 없어.')}, 'counts': {'fromAppAction': 1, 'reader': 0, 'sessionList': 0, 'createImported': 0, 'opening.add': 0}})
    F('A', 5, '3.4', merge({'fromAppAction': True}, okread('claude')), [OPEN('r1', mode='fork')],
      {'req': {'r1': ok_new('new-1')}, 'counts': {'fromAppAction': 0, 'createImported': 1}})
    F('A', 6, '3.4', merge(gworld(), {'grokAccounts': [{'id': 'g1'}, {'id': 'g2'}]}), [OPEN('r1', source='grok:home', target='g1')],
      {'req': {'r1': ok_new('new-1')}, 'counts': {'grokAccounts': 1}, 'has': [{'c': 'grokHomeFor', 'a': ['g1']}, {'c': 'modelFor', 'a': {'engine': 'grok', 'account': 'g1'}}],
       'createdMeta': [{'seed': {'engine': 'grok', 'account': 'g1'}, 'seedKeys': ['engine', 'account', 'model', 'folder', 'fill']}]})
    F('A', 7, '3.4', merge(gworld(), {'grokAccounts': [{'id': 'g1'}]}), [OPEN('r1', source='grok:home', target='nope')],
      {'req': {'r1': wire_req('accountUnknown', '고른 Grok 계정을 찾을 수 없어.')}, 'counts': {'grokAccounts': 1, 'reader': 0, 'sessionList': 0, 'opening.add': 0, 'createImported': 0}})
    F('A', 8, '3.4', merge(gworld(), {'grokAccounts': [{'id': 'g1'}]}), [OPEN('r1', source='grok:home', target='default'), OPEN('r2', source='grok:home', thread='t2', target='')],
      {'req': {'r1': ok_new('new-1'), 'r2': ok_new('new-2')}, 'counts': {'grokAccounts': 0},
       'createdMeta': [{'seedKeys': ['engine', 'model', 'folder', 'fill']}, {'seedKeys': ['engine', 'model', 'folder', 'fill']}]})
    F('A', 9, '3.4', merge(ACCTS, okread('claude'), {'grokAccounts': [{'id': 'g1'}]}), [OPEN('r1', source=f'claude:{ACC1}', target='g1'), OPEN('r2', source=f'claude:{ACC1}', thread='t2', target='nope')],
      {'req': {'r1': ok_new('new-1'), 'r2': ok_new('new-2')}, 'counts': {'grokAccounts': 0},
       'createdMeta': [{'seed': {'engine': 'claude', 'account': ACC1}}, {'seed': {'engine': 'claude', 'account': ACC1}}]})
    F('A', 10, '3.4', merge({'sessions': [bound('s1', 'grok', 't1')], 'grokAccounts': [{'id': 'g1'}]}), [OPEN('r1', source='grok:home', target='nope')],
      {'req': {'r1': wire_req('accountUnknown')}, 'counts': zero('sessionList', 'reader')})


# ==== O: opening result and created session ========================================================================================
def group_O():
    F('O', 1, '3.4', okread('claude', record('Hello', [turn('hi', 'there', 2, True, createdAt='2023-01-01T00:00:00.000Z', completedAt='2023-01-01T00:00:05.000Z')])), [OPEN('r1')],
      {'req': {'r1': ok_new('new-1')}, 'created': 1,
       'createdAfter': [{'id': 'new-1', 'title': 'Hello', 'turns': [{'id': UU(1), 'prompt': 'hi', 'reply': 'there', 'progress': '', 'status': 'completed', 'usage': None,
                                                                    'createdAt': '2023-01-01T00:00:00.000Z', 'completedAt': '2023-01-01T00:00:05.000Z', 'imported': {'engine': 'claude', 'tools': 2, 'clipped': True}}],
                        'externalThread': {'engine': 'claude', 'source': 'claude:home', 'thread': 't1', 'mode': 'fork', 'importedAt': ISO},
                        'engineThreads': {'claude': {'threadId': 't1', 'lastTurnId': UU(1), 'fork': True}}}],
       'createdKeys': [[0, ['turns', 0], ['id', 'prompt', 'reply', 'progress', 'status', 'usage', 'createdAt', 'completedAt', 'imported']], [0, ['externalThread'], ['engine', 'source', 'thread', 'mode', 'importedAt']],
                       [0, ['engineThreads', 'claude'], ['threadId', 'lastTurnId', 'fork']], [0, ['turns', 0, 'imported'], ['engine', 'tools', 'clipped']]]})
    F('O', 2, '3.4', merge({'sessions': [bound('s1', 'claude', 't1')]}, okread('claude')), [OPEN('r1')],
      {'req': {'r1': ok_existing('s1')}, 'counts': zero('reader', 'timer.set', 'opening.add', 'now', 'uuid', 'createImported'), 'created': 0})
    F('O', 3, '3.4', merge({'sessions': [bound('s1', 'claude', 't1')]}, okread('claude')), [{'ev': 'handle', 'name': 'c1'}, CANCEL('c1'), OPEN('r1', cancel='c1')],
      {'req': {'r1': ok_existing('s1')}, 'counts': zero('reader', 'listener.add', 'timer.set', 'createImported'),
       'note': 'The initial bound check precedes cancel handling.'})
    F('O', 4, '3.4', okread('claude', record('Empty', [])), [OPEN('r1')],
      {'req': {'r1': ok_new('new-1')}, 'counts': {'uuid': 0, 'now': 1, 'createImported': 1},
       'createdAfter': [{'title': 'Empty', 'turns': [], 'engineThreads': {'claude': {'threadId': 't1', 'fork': True}}}], 'createdKeys': [[0, ['engineThreads', 'claude'], ['threadId', 'fork']]]})
    F('O', 5, '3.4', okread('claude'), [OPEN('r1', mode='same')],
      {'req': {'r1': ok_new('new-1')}, 'createdKeys': [[0, ['engineThreads', 'claude'], ['threadId', 'lastTurnId']]], 'createdAfter': [{'externalThread': {'mode': 'same'}}]})
    F('O', 6, '3.4', merge(okread('codex'), {'accounts': {'codex': {'ok': [{'id': ACC2, 'name': 'C'}]}}}), [OPEN('r1', source='codex:home'), OPEN('r2', source=f'codex:{ACC2}', thread='t2')],
      {'req': {'r1': ok_new('new-1'), 'r2': ok_new('new-2')},
       'createdKeys': [[0, ['engineThreads', 'codex'], ['threadId', 'lastTurnId', 'fork']], [1, ['engineThreads', f'codex:{ACC2}'], ['threadId', 'lastTurnId', 'fork']]],
       'matchCounts': [[{'c': 'reader', 'a': {'engine': 'codex', 'op': 'read', 'options': {'keys': ['signal', 'env', 'codex']}}}, 2]]})
    F('O', 7, '3.4', gworld(), [OPEN('r1', source='grok:home', mode='fork')],
      {'req': {'r1': ok_new('new-1')}, 'createdKeys': [[0, ['engineThreads', 'grok'], ['threadId', 'lastTurnId']]], 'createdAfter': [{'externalThread': {'engine': 'grok', 'mode': 'fork'}}]})
    F('O', 8, '3.4', merge(okread('claude'), {'accounts': {'claude': {'ok': [{'id': ACC1, 'name': 'Work'}]}}, 'newSession': {'title': '', 'turns': [], 'engineThreads': slots({'claude': 'old'})}}),
      [OPEN('r1', source=f'claude:{ACC1}')],
      {'req': {'r1': ok_new('new-1')}, 'createdBefore': [{'engineThreads': {'claude': {'threadId': 'old'}}}],
       'createdAfter': [{'engineThreads': {'claude': {'threadId': 'old'}, f'claude:{ACC1}': {'threadId': 't1'}}}],
       'createdKeys': [[0, ['engineThreads'], ['claude', f'claude:{ACC1}']]],
       'createdMeta': [{'seed': {'account': ACC1}}]})
    F('O', 9, '3.4', merge(okread('claude', record('M', model='opus-x')), {'modelFor': [{'ok': 'mapped-model'}]}), [OPEN('r1')],
      {'req': {'r1': ok_new('new-1')}, 'has': [{'c': 'modelFor', 'a': {'engine': 'claude', 'wanted': 'opus-x'}, 'o': {'ok': 'mapped-model'}}], 'createdMeta': [{'seed': {'model': 'mapped-model'}}]})
    F('O', 10, '3.4', merge(okread('claude'), {'newSession': {'title': '', 'turns': [], 'engineThreads': {**slots({'codex': 'keep', 'claude:other': 'o2'})}}}), [OPEN('r1')],
      {'req': {'r1': ok_new('new-1')}, 'createdKeys': [[0, ['engineThreads'], ['codex', 'claude:other', 'claude']]],
       'createdAfter': [{'engineThreads': {'codex': {'threadId': 'keep'}, 'claude:other': {'threadId': 'o2'}, 'claude': {'threadId': 't1'}}}]})


# ==== T: turns =====================================================================================================================
def tw(turns, **kw): return okread('claude', record('T', turns, **kw))
def tk(i, keys): return [0, ['turns', i], keys]
def group_T():
    F('T', 1, '3.4', tw([turn('a'), turn('b'), turn('c')]), [OPEN('r1')],
      {'req': {'r1': ok_new('new-1')}, 'counts': {'uuid': 3}, 'createdAfter': [{'turns': [{'id': UU(1), 'prompt': 'a'}, {'id': UU(2), 'prompt': 'b'}, {'id': UU(3), 'prompt': 'c'}],
                                                                           'engineThreads': {'claude': {'lastTurnId': UU(3)}}}]})
    F('T', 2, '3.4', tw([turn('a'), turn('b')]), [OPEN('r1')],
      {'req': {'r1': ok_new('new-1')}, 'counts': {'now': 1, 'uuid': 2}, 'order': [rc('now'), rc('uuid'), rc('uuid'), rc('createImported')],
       'createdAfter': [{'turns': [{'createdAt': ISO}, {'createdAt': ISO}], 'externalThread': {'importedAt': ISO}}]})
    F('T', 3, '3.4', tw([turn('a', createdAt='2020-05-05T05:05:05.000Z'), turn('b')]), [OPEN('r1')],
      {'req': {'r1': ok_new('new-1')}, 'createdAfter': [{'turns': [{'createdAt': '2020-05-05T05:05:05.000Z'}, {'createdAt': ISO}]}]})
    F('T', 4, '3.4', tw([turn('a', createdAt='')]), [OPEN('r1')],
      {'req': {'r1': ok_new('new-1')}, 'createdAfter': [{'turns': [{'createdAt': ''}]}]})
    F('T', 5, '3.4', tw([turn('a', createdAt=None)]), [OPEN('r1')],
      {'req': {'r1': ok_new('new-1')}, 'createdAfter': [{'turns': [{'createdAt': ISO}]}]})
    F('T', 6, '3.4', tw([turn('a', completedAt='2020-01-01T00:00:00.000Z'), turn('b', completedAt=''), turn('c', completedAt=None), turn('d')]), [OPEN('r1')],
      {'req': {'r1': ok_new('new-1')}, 'createdAfter': [{'turns': [{'completedAt': '2020-01-01T00:00:00.000Z'}, {'completedAt': ''}, {'completedAt': None}, {'prompt': 'd'}]}],
       'createdKeys': [tk(0, ['id', 'prompt', 'reply', 'progress', 'status', 'usage', 'createdAt', 'completedAt', 'imported']), tk(1, ['id', 'prompt', 'reply', 'progress', 'status', 'usage', 'createdAt', 'completedAt', 'imported']),
                       tk(2, ['id', 'prompt', 'reply', 'progress', 'status', 'usage', 'createdAt', 'completedAt', 'imported']), tk(3, ['id', 'prompt', 'reply', 'progress', 'status', 'usage', 'createdAt', 'imported'])]})
    F('T', 7, '3.4', tw([turn('a', tools=0), turn('b', tools=-1), turn('c', tools=3)]), [OPEN('r1')],
      {'req': {'r1': ok_new('new-1')}, 'createdKeys': [[0, ['turns', 0, 'imported'], ['engine']], [0, ['turns', 1, 'imported'], ['engine']], [0, ['turns', 2, 'imported'], ['engine', 'tools']]],
       'createdAfter': [{'turns': [{}, {}, {'imported': {'engine': 'claude', 'tools': 3}}]}]})
    F('T', 8, '3.4', tw([turn('a', clipped=True), turn('b', clipped=False), turn('c', clipped=1)]), [OPEN('r1')],
      {'req': {'r1': ok_new('new-1')}, 'createdKeys': [[0, ['turns', 0, 'imported'], ['engine', 'clipped']], [0, ['turns', 1, 'imported'], ['engine']], [0, ['turns', 2, 'imported'], ['engine', 'clipped']]],
       'createdAfter': [{'turns': [{'imported': {'clipped': True}}, {}, {'imported': {'clipped': True}}]}]})
    F('T', 9, '3.4', merge(readers('claude', read=[{'ok': record('T', omittedBefore=True)}, {'ok': record('T', omittedBefore=False)}])), [OPEN('r1'), OPEN('r2', thread='t2')],
      {'req': {'r1': ok_new('new-1'), 'r2': ok_new('new-2')}, 'createdKeys': [[0, ['externalThread'], ['engine', 'source', 'thread', 'mode', 'importedAt', 'omittedBefore']], [1, ['externalThread'], ['engine', 'source', 'thread', 'mode', 'importedAt']]],
       'createdAfter': [{'externalThread': {'omittedBefore': True}}, {}]})
    F('T', 10, '3.4', merge(readers('claude', read=[{'ok': record('T', lastPlan='step one')}, {'ok': record('T', lastPlan='')}, {'ok': record('T', lastPlan={'steps': [1, 2]})}])),
      [OPEN('r1'), OPEN('r2', thread='t2'), OPEN('r3', thread='t3')],
      {'req': {'r1': ok_new('new-1'), 'r2': ok_new('new-2'), 'r3': ok_new('new-3')},
       'createdKeys': [[0, ['externalThread'], ['engine', 'source', 'thread', 'mode', 'importedAt', 'lastPlan']], [1, ['externalThread'], ['engine', 'source', 'thread', 'mode', 'importedAt']],
                       [2, ['externalThread'], ['engine', 'source', 'thread', 'mode', 'importedAt', 'lastPlan']]],
       'createdAfter': [{'externalThread': {'lastPlan': 'step one'}}, None, {'externalThread': {'lastPlan': {'steps': [1, 2]}}}]})


# ==== U: title =====================================================================================================================
def tt(title, turns=None, folder='/proj', **kw):
    return merge(okread('claude', record(title, turns if turns is not None else [turn('first prompt')])), kw)
def title_fx(n, world, expected_title, folder='/proj', note=None, rule='3.4'):
    F('U', n, rule, world, [OPEN('r1', folder=folder)], {'req': {'r1': ok_new('new-1')}, 'createdAfter': [{'title': expected_title}], 'createdMeta': [{'seed': {'folder': folder}}]}, note=note)
def group_U():
    title_fx(1, tt('a   b\n  c\t d'), 'a b c d')
    title_fx(2, tt('', [turn('  hello   world ')]), 'hello world')
    title_fx(3, tt('', []), 'proj')
    title_fx(4, tt('   ', [turn('real prompt')]), 'proj', note='A whitespace-only title is truthy, so the first prompt is not tried; the fallback is the folder basename.')
    title_fx(5, tt('\u00a0 a\u3000b\ufeff\u2028c  \u2029'), 'a b c')
    title_fx(6, tt('a\u0085b\u200bc'), 'a\u0085b\u200bc')
    title_fx(7, tt('a' * 79 + '\U0001f600zz'), 'a' * 79 + '\ud83d', note='slice(0, 80) counts UTF-16 units: the astral character is cut to a lone high surrogate.')
    title_fx(8, tt('', []), 'proj', folder='/a/b/proj/')
    title_fx(9, tt('', []), '/', folder='/')
    long_name = 'b' * 50 + '  ' + 'c' * 50
    title_fx(10, tt('', []), long_name, folder='/p/' + long_name, note='The folder fallback is neither collapsed nor clipped to 80.')


# ==== G: Grok copy and verify ======================================================================================================
def gk(**kw): return merge(gworld(), kw)
def group_G():
    F('G', 1, '3.6', gworld(), [OPEN('r1', source='grok:home')],
      {'req': {'r1': ok_new('new-1')}, 'counts': {'rm': 0, 'copy': 1, 'copy.verify': 1, 'loadCheck': 1, 'grokHomeFor': 1},
       'order': [rc('reader'), rc('grokHomeFor'), rc('copy'), rc('copy.verify'), rc('loadCheck'), rc('now'), rc('createImported')],
       'has': [{'c': 'copy', 'a': {'source': 'grok:home', 'folder': '/proj', 'thread': 't1', 'targetHome': '/grok/home'}},
               {'c': 'loadCheck', 'a': {'binary': '/grok/bin', 'home': '/grok/home', 'folder': '/proj', 'thread': 't1', 'checkKeys': ['binary', 'env']}}]})
    F('G', 2, '3.6', gk(grokHomeFor=[{'reject': err('e1', message='no grok binary')}]), [OPEN('r1', source='grok:home')],
      {'req': {'r1': wire_req('engineMissing')}, 'counts': {'copy': 0, 'rm': 0, 'createImported': 0}})
    F('G', 3, '3.6', gk(grokHomeFor=[{'reject': err('p1', 'primitive', value='boom')}]), [OPEN('r1', source='grok:home')],
      {'req': {'r1': wire_req('engineMissing')}, 'counts': {'copy': 0, 'rm': 0, 'createImported': 0}})
    F('G', 4, '3.6', gk(copy=[{'reject': eie('e1', 'grok-exists')}]), [OPEN('r1', source='grok:home')],
      {'req': {'r1': wire_req('grokExists')}, 'counts': {'rm': 0, 'loadCheck': 0, 'createImported': 0, 'copy.verify': 0}})
    F('G', 5, '3.6', gk(copy=[{'reject': err('e1', message='disk full')}]), [OPEN('r1', source='grok:home')],
      {'req': {'r1': thrown('e1')}, 'counts': {'rm': 0, 'createImported': 0}})
    F('G', 6, '3.6', gk(), [OPEN('r1', source='grok:home')],
      {'req': {'r1': ok_new('new-1')}, 'matchCounts': [[{'c': 'timer.set', 'a': {'delay': 20000}}, 2], [rc('timer.clear'), 2]], 'timers': 0,
       'order': [{'c': 'timer.set'}, rc('reader'), {'c': 'timer.clear'}, {'c': 'timer.set'}, rc('loadCheck'), {'c': 'timer.clear'}]})
    F('G', 7, '3.6', gk(loadCheck=[{'reject': err('e1', message='cannot load')}]), [OPEN('r1', source='grok:home')],
      {'req': {'r1': wire_req('grokLoadFailed')}, 'counts': {'rm': 0, 'createImported': 0}})
    F('G', 8, '3.6', gk(loadCheck=[{'reject': abort_err()}]), [OPEN('r1', source='grok:home')],
      {'req': {'r1': wire_req('timeout', '터미널 대화를 읽는 데 시간이 너무 오래 걸렸어.')}, 'counts': {'rm': 0, 'createImported': 0}})
    F('G', 9, '3.6', gk(), [{'ev': 'handle', 'name': 'c1'}, OPEN('r1', source='grok:home', cancel='c1')],
      {'req': {'r1': ok_new('new-1')}, 'has': [{'c': 'copy', 'a': {'signal': 'c1'}}, {'c': 'loadCheck', 'a': {'signal': 'sig2'}}, {'c': 'reader', 'a': {'options': {'signal': 'sig1'}}}],
       'counts': {'listener.add': 2, 'listener.remove': 2}, 'listeners': {'c1': 0}})
    F('G', 10, '3.6', gk(createImported=[{'mode': 'fail-before-fill', 'throw': err('e1', message='create failed')}]), [OPEN('r1', source='grok:home')],
      {'req': {'r1': thrown('e1')}, 'counts': {'rm': 1}, 'has': [{'c': 'rm', 'a': {'path': '/grok/copy1', 'options': {'recursive': True, 'force': True}}}],
       'order': [rc('copy'), rc('createImported'), rc('rm')]})


# ==== R: race and cleanup ==========================================================================================================
def group_R():
    rm_has = {'c': 'rm', 'a': {'path': '/grok/copy1', 'options': {'recursive': True, 'force': True}}}
    F('R', 1, '3.6', gworld(copy_entry=COPY_DEFER), [OPEN('r1', source='grok:home'), MUT_ADD(bound('s9', 'grok', 't1', 'Raced')), SETTLE('cp')],
      {'req': {'r1': ok_existing('s9')}, 'counts': {'rm': 1, 'createImported': 0, 'now': 0, 'uuid': 0, 'modelFor': 0}, 'has': [rm_has], 'created': 0,
       'order': [rc('copy'), rc('rm')]})
    F('R', 2, '3.6', gworld(copy_entry=COPY_DEFER, rm=[{'reject': err('e2', message='rm failed')}]), [OPEN('r1', source='grok:home'), MUT_ADD(bound('s9', 'grok', 't1')), SETTLE('cp')],
      {'req': {'r1': ok_existing('s9')}, 'counts': {'rm': 1, 'createImported': 0}})
    F('R', 3, '3.6', gworld(copy_entry=COPY_DEFER), [{'ev': 'handle', 'name': 'c1'}, OPEN('r1', source='grok:home', cancel='c1'), CANCEL('c1', reason=err('R', message='stop')), SETTLE('cp')],
      {'req': {'r1': cancelled('c1', ref='R')}, 'counts': {'rm': 1, 'createImported': 0, 'now': 0}, 'has': [rm_has]})
    F('R', 4, '3.6', gworld(sessionList=[None, {'throw': err('e1', message='scan failed'), 'throwAt': 0}]), [OPEN('r1', source='grok:home')],
      {'req': {'r1': thrown('e1')}, 'counts': {'rm': 1, 'createImported': 0}, 'has': [rm_has]})
    F('R', 5, '3.6', gworld(clock={'throw': err('e1', message='clock broken')}), [OPEN('r1', source='grok:home')],
      {'req': {'r1': thrown('e1')}, 'counts': {'rm': 1, 'createImported': 0}, 'has': [rm_has]})
    F('R', 6, '3.6', gworld(uuids=[{'throw': err('e1', message='uuid broken')}]), [OPEN('r1', source='grok:home')],
      {'req': {'r1': thrown('e1')}, 'counts': {'rm': 1, 'createImported': 0}, 'has': [rm_has]})
    F('R', 7, '3.6', gworld(modelFor=[{'throw': err('e1', message='model broken')}]), [OPEN('r1', source='grok:home')],
      {'req': {'r1': thrown('e1')}, 'counts': {'rm': 1, 'createImported': 0}, 'has': [rm_has]})
    F('R', 8, '3.6', gworld(createImported=[{'mode': 'fail-after-fill', 'throw': err('e1', message='after fill')}]), [OPEN('r1', source='grok:home')],
      {'req': {'r1': thrown('e1')}, 'counts': {'rm': 1, 'createImported': 1}, 'has': [rm_has], 'createdMeta': [{'outcome': 'fail-after-fill'}]})
    F('R', 9, '3.6', gworld(), [OPEN('r1', source='grok:home')],
      {'req': {'r1': ok_new('new-1')}, 'counts': {'rm': 0}, 'createdMeta': [{'outcome': 'ok'}]})
    F('R', 10, '3.6', gworld(createImported=[{'mode': 'fail-before-fill', 'throw': err('e1', message='create failed')}], rm=[{'reject': err('e2', message='rm failed')}]), [OPEN('r1', source='grok:home')],
      {'req': {'r1': thrown('e1')}, 'counts': {'rm': 1}, 'has': [rm_has]})


# ==== E: error mapping =============================================================================================================
TIMEOUT_MSG = '터미널 대화를 읽는 데 시간이 너무 오래 걸렸어.'
def rd_reject(e, engine='claude'): return readers(engine, read=[{'reject': e}])
def group_E():
    def one(n, code, suffix, source='claude:home', engine='claude'):
        F('E', n, '3.7', rd_reject(eie('e1', code, message='x'), engine), [OPEN('r1', source=source)],
          {'req': {'r1': wire_req(suffix)}, 'counts': {'createImported': 0, 'opening.delete': 1}, 'opening': {'A': []}})
    one(1, 'not-found', 'notFound')
    one(2, 'codex-unsupported', 'codexUnsupported', 'codex:home', 'codex')
    one(3, 'grok-exists', 'grokExists')
    one(4, 'grok-load-failed', 'grokLoadFailed')
    one(5, 'timeout', 'timeout')
    one(6, 'engine-missing', 'engineMissing')
    F('E', 7, '3.7', readers('claude', read=[{'reject': eie('e1', 'read-failed')}, {'reject': err('e2', message='plain failure')}]), [OPEN('r1'), OPEN('r2', thread='t2')],
      {'req': {'r1': wire_req('readFailed'), 'r2': wire_req('readFailed')}, 'counts': {'createImported': 0},
       'note': 'The entry (E) wraps a plain Error from a reader as EngineImportError read-failed.'})
    F('E', 8, '3.7', rd_reject(abort_err()), [OPEN('r1')],
      {'req': {'r1': wire_req('timeout', TIMEOUT_MSG)}, 'counts': {'createImported': 0}})
    F('E', 9, '3.7', rd_reject(eie('e1', 'zzz', message='odd')), [OPEN('r1')],
      {'req': {'r1': thrown('e1')}, 'counts': {'createImported': 0}, 'note': 'An EngineImportError with an unmapped code is not converted to a wire error; it is rethrown as-is.'})
    F('E', 10, '3.7', merge(okread('grok'), {'copy': [{'reject': err('p1', 'primitive', value='raw string')}, {'reject': err('p2', 'plain', code=7)}]}),
      [OPEN('r1', source='grok:home'), OPEN('r2', source='grok:home', thread='t2')],
      {'req': {'r1': thrown('p1'), 'r2': thrown('p2')}, 'counts': {'rm': 0, 'createImported': 0}})


# ==== C: cancel and deadline =======================================================================================================
H1 = {'ev': 'handle', 'name': 'c1'}
def deferred_read(label='rd', on_abort=True, rec_=None):
    entry = {'defer': label, 'ok': rec_ if rec_ is not None else record()}
    if on_abort: entry['onAbort'] = abort_err()
    return readers('claude', read=[entry])
def group_C():
    F('C', 1, '3.8', okread('claude'), [H1, CANCEL('c1'), OPEN('r1', cancel='c1')],
      {'req': {'r1': cancelled('c1')}, 'counts': {'createImported': 0, 'now': 0, 'uuid': 0, 'rm': 0}, 'has': [{'c': 'reader', 'a': {'options': {'aborted': True}}}],
       'created': 0, 'opening': {'A': []}})
    F('C', 2, '3.8', deferred_read(), [H1, OPEN('r1', cancel='c1'), CANCEL('c1')],
      {'req': {'r1': cancelled('c1')}, 'has': [{'c': 'abort', 'a': {'signal': 'c1', 'reason': {'cancelReason': 'c1'}}}, {'c': 'abort', 'a': {'signal': 'sig1', 'reason': {'domAbort': 1}}}],
       'counts': {'createImported': 0}, 'opening': {'A': []}})
    F('C', 3, '3.8', deferred_read(), [OPEN('r1'), ADV(19999), ADV(1)],
      {'req': {'r1': wire_req('timeout', TIMEOUT_MSG)},
       'has': [{'c': 'timer.set', 'a': {'delay': 20000, 'due': 20000}}, {'c': 'timer.fire', 'a': {'at': 20000}}, {'c': 'abort', 'a': {'signal': 'sig1'}}],
       'order': [{'c': 'advance', 'a': {'ms': 19999}}, {'c': 'advance', 'a': {'ms': 1}}, rc('timer.fire')],
       'noneBetween': [[{'c': 'timer.set'}, {'c': 'advance', 'a': {'ms': 1}}, rc('timer.fire')]], 'counts': {'timer.fire': 1}})
    F('C', 4, '3.8', deferred_read(on_abort=False), [OPEN('r1'), ADV(20000), SETTLE('rd')],
      {'req': {'r1': ok_new('new-1')}, 'has': [rc('timer.fire'), {'c': 'abort', 'a': {'signal': 'sig1'}}], 'order': [rc('timer.fire'), rc('settle'), rc('createImported')]})
    F('C', 5, '3.8', merge(okread('codex'), {'makeCodexRunDeps': [{'ok': {'ref': 'codexRun'}, 'defer': 'mk'}]}), [OPEN('r1', source='codex:home'), ADV(20000), SETTLE('mk')],
      {'req': {'r1': ok_new('new-1')}, 'order': [rc('timer.set'), rc('makeCodexRunDeps'), rc('timer.fire'), rc('settle'), rc('reader')],
       'has': [{'c': 'reader', 'a': {'engine': 'codex', 'options': {'aborted': True, 'keys': ['signal', 'env', 'codex']}}}]})
    F('C', 6, '3.8', merge(okread('grok'), {'copy': [{**COPY_DEFER}], 'loadCheck': [{'defer': 'lc', 'onAbort': abort_err()}]}),
      [OPEN('r1', source='grok:home'), ADV(5000), SETTLE('cp'), ADV(19999), ADV(1)],
      {'req': {'r1': wire_req('timeout')},
       'has': [{'c': 'timer.set', 'a': {'id': 2, 'due': 25000}}, {'c': 'timer.fire', 'a': {'id': 2, 'at': 25000}}], 'counts': {'timer.set': 2, 'timer.fire': 1, 'rm': 0, 'createImported': 0},
       'order': [{'c': 'timer.set', 'a': {'id': 1}}, {'c': 'timer.clear', 'a': {'id': 1}}, {'c': 'timer.set', 'a': {'id': 2}}]})
    F('C', 7, '3.8', deferred_read(), [H1, OPEN('r1', cancel='c1'), CANCEL('c1', reason=err('R', message='user stop'))],
      {'req': {'r1': cancelled('c1', ref='R')}, 'counts': {'createImported': 0}, 'has': [{'c': 'abort', 'a': {'signal': 'sig1'}}]})
    F('C', 8, '3.8', merge(gworld(copy_entry=COPY_DEFER, rm=[{'defer': 'rmd'}])), [H1, OPEN('r1', source='grok:home', cancel='c1'), MUT_ADD(bound('s9', 'grok', 't1')), SETTLE('cp'), CANCEL('c1'), SETTLE('rmd')],
      {'req': {'r1': ok_existing('s9')}, 'counts': {'rm': 1, 'createImported': 0}, 'order': [rc('rm'), rc('abort'), rc('settle')]})
    F('C', 9, '3.8', gworld(createImported=[{'mode': 'fail-before-fill', 'throw': err('e1', message='create failed')}], rm=[{'defer': 'rmd'}]),
      [H1, OPEN('r1', source='grok:home', cancel='c1'), CANCEL('c1'), SETTLE('rmd')],
      {'req': {'r1': cancelled('c1')}, 'counts': {'rm': 1, 'createImported': 1}})
    F('C', 10, '3.8', gworld(createImported=[{'mode': 'ok'}, {'mode': 'fail-before-fill', 'throw': err('e1', message='create failed')}]),
      [H1, OPEN('r1', source='grok:home', cancel='c1'), OPEN('r2', source='grok:home', thread='t2', cancel='c1')],
      {'req': {'r1': ok_new('new-1'), 'r2': thrown('e1')}, 'matchCounts': [[rc('timer.set'), 4], [rc('timer.clear'), 4], [rc('listener.add'), 4], [rc('listener.remove'), 4]],
       'timers': 0, 'listeners': {'c1': 0}})


# ==== D: reader options ============================================================================================================
def group_D():
    ro = lambda engine, keys, **kw: {'c': 'reader', 'a': {'engine': engine, 'options': {'keys': keys, 'env': 'deps.env', **kw}}}
    F('D', 1, '3.9', readers('codex', list=[{'ok': []}]), [LIST('r1', source='codex:home')],
      {'req': {'r1': {'terminal': 'ok', 'value': {'items': []}}}, 'counts': {'makeCodexRunDeps': 1}, 'has': [{'c': 'makeCodexRunDeps', 'a': {'source': 'codex:home', 'dataDir': '/data'}}, ro('codex', ['signal', 'env', 'codex'])]})
    F('D', 2, '3.9', okread('codex'), [OPEN('r1', source='codex:home')],
      {'req': {'r1': ok_new('new-1')}, 'counts': {'makeCodexRunDeps': 1, 'locateAcp': 0, 'tempRoot': 0}, 'has': [ro('codex', ['signal', 'env', 'codex'], codex='@ref:codexRun')]})
    F('D', 3, '3.9', merge(okread('codex'), {'makeCodexRunDeps': [{'reject': err('e1', message='no codex')}]}), [OPEN('r1', source='codex:home')],
      {'req': {'r1': thrown('e1')}, 'counts': {'reader': 0, 'createImported': 0}})
    F('D', 4, '3.9', readers('opencode', list=[{'ok': []}]), [LIST('r1', source='opencode:home')],
      {'req': {'r1': {'terminal': 'ok'}}, 'counts': {'locateAcp': 0, 'tempRoot': 0}, 'has': [ro('opencode', ['signal', 'env'])]})
    F('D', 5, '3.9', okread('opencode'), [OPEN('r1', source='opencode:home')],
      {'req': {'r1': ok_new('new-1')}, 'counts': {'locateAcp': 1, 'tempRoot': 1}, 'has': [{'c': 'locateAcp', 'a': ['opencode']}, ro('opencode', ['signal', 'env', 'opencode'], opencode={'binary': '/bin/opencode', 'tempRoot': '/tmp/lair-root'})],
       'order': [rc('locateAcp'), rc('tempRoot'), rc('reader')]})
    F('D', 6, '3.9', merge(okread('opencode'), {'locateAcp': [{'ok': {'binary': ''}}, {'ok': {'binary': {'undef': 1}}}]}), [OPEN('r1', source='opencode:home'), OPEN('r2', source='opencode:home', thread='t2')],
      {'req': {'r1': ok_new('new-1'), 'r2': ok_new('new-2')}, 'counts': {'locateAcp': 2, 'tempRoot': 0}, 'matchCounts': [[ro('opencode', ['signal', 'env']), 2]]})
    F('D', 7, '3.9', merge(okread('opencode'), {'locateAcp': [{'reject': err('e1', message='locate failed')}]}), [OPEN('r1', source='opencode:home')],
      {'req': {'r1': thrown('e1')}, 'counts': {'reader': 0, 'tempRoot': 0}})
    F('D', 8, '3.9', merge(okread('opencode'), {'tempRoot': [{'reject': err('e1', message='tmp failed')}]}), [OPEN('r1', source='opencode:home')],
      {'req': {'r1': thrown('e1')}, 'counts': {'reader': 0, 'locateAcp': 1}})
    F('D', 9, '3.9', merge(readers('claude', list=[{'ok': []}], read=[{'ok': record()}])), [LIST('r1'), H1, OPEN('r2', cancel='c1')],
      {'req': {'r1': {'terminal': 'ok'}, 'r2': ok_new('new-1')},
       'matchCounts': [[{'c': 'reader', 'a': {'op': 'list', 'options': {'signal': 'sig1', 'env': 'deps.env'}}}, 1], [{'c': 'reader', 'a': {'op': 'read', 'options': {'signal': 'sig2', 'env': 'deps.env'}}}, 1],
                       [{'c': 'reader', 'a': {'options': {'signal': 'c1'}}}, 0]]})
    both = lambda e: readers(e, read=[{'ok': record()}], list=[{'ok': []}])
    F('D', 10, '3.9', merge(both('claude'), both('qwen')), [LIST('r1'), OPEN('r2'), LIST('r3', source='qwen:home'), OPEN('r4', source='qwen:home')],
      {'req': {'r1': {'terminal': 'ok'}, 'r2': ok_new('new-1'), 'r3': {'terminal': 'ok'}, 'r4': ok_new('new-2')},
       'matchCounts': [[ro('claude', ['signal', 'env']), 2], [ro('qwen', ['signal', 'env']), 2]], 'counts': zero('makeCodexRunDeps', 'locateAcp', 'tempRoot')})


# ==== I: concurrency (spec 6.3) ====================================================================================================
def op(o, key, result): return f'{o}:{key}:{str(result).lower()}'
def group_I():
    K1 = 'claude:t1'
    F('I', 1, '3.10', deferred_read(on_abort=False), [OPEN('r1'), OPEN('r2'), SETTLE('rd')],
      {'req': {'r1': ok_new('new-1'), 'r2': wire_req('busy', '같은 터미널 대화를 이미 여는 중이야.')}, 'counts': {'reader': 1, 'createImported': 1},
       'openingOps': [op('has', K1, False), op('add', K1, True), op('has', K1, True), op('delete', K1, True)], 'opening': {'A': []}})
    F('I', 2, '3.10', deferred_read(on_abort=False), [OPEN('r1'), MUT_ADD(bound('s9', 'claude', 't1')), OPEN('r2'), SETTLE('rd')],
      {'req': {'r1': ok_existing('s9'), 'r2': ok_existing('s9')}, 'created': 0, 'openingOps': [op('has', K1, False), op('add', K1, True), op('delete', K1, True)], 'counts': {'reader': 1}})
    F('I', 3, '3.10', deferred_read(on_abort=False), [OPEN('r1'), MUT_ADD(bound('s9', 'claude', 't1')), SETTLE('rd')],
      {'req': {'r1': ok_existing('s9')}, 'counts': {'createImported': 0, 'now': 0, 'uuid': 0, 'modelFor': 0}, 'created': 0, 'order': [rc('reader'), rc('settle'), {'c': 'sessionList', 'a': {'call': 2}}]})
    F('I', 4, '3.10', gworld(copy_entry=COPY_DEFER), [OPEN('r1', source='grok:home'), OPEN('r2', source='grok:home'), MUT_ADD(bound('s9', 'grok', 't1')), SETTLE('cp')],
      {'req': {'r1': ok_existing('s9'), 'r2': wire_req('busy')}, 'counts': {'rm': 1, 'createImported': 0, 'copy': 1}, 'openingOps': [op('has', 'grok:t1', False), op('add', 'grok:t1', True), op('has', 'grok:t1', True), op('delete', 'grok:t1', True)]})
    F('I', 5, '3.10', readers('claude', read=[{'defer': 'rd', 'ok': record()}]), [OPEN('r1'), OPEN('r2', thread='t2'), SETTLE('rd#2'), SETTLE('rd')],
      {'req': {'r1': ok_new('new-2'), 'r2': ok_new('new-1')}, 'counts': {'opening.add': 2, 'opening.delete': 2, 'reader': 2}, 'opening': {'A': []}})
    F('I', 6, '3.10', deferred_read(on_abort=False), [OPEN('r1', folder='/p1'), OPEN('r2', folder='/p2'), SETTLE('rd')],
      {'req': {'r1': ok_new('new-1'), 'r2': wire_req('busy')}, 'counts': {'reader': 1, 'createImported': 1}, 'has': [{'c': 'createImported', 'a': {'folder': '/p1'}}]})
    F('I', 7, '3.10', merge(ACCTS, deferred_read(on_abort=False), okread('codex')),
      [OPEN('r1'), OPEN('r2', source=f'claude:{ACC1}'), OPEN('r3', source='codex:home'), SETTLE('rd')],
      {'req': {'r1': ok_new('new-2'), 'r2': wire_req('busy'), 'r3': ok_new('new-1')}, 'counts': {'reader': 2}})
    F('I', 8, '3.10', deferred_read(on_abort=False), [OPEN('r1', i='A'), OPEN('r2', i='B'), SETTLE('rd#2'), SETTLE('rd')],
      {'req': {'r1': ok_existing('new-1'), 'r2': ok_new('new-1')}, 'created': 1, 'counts': {'reader': 2},
       'opening': {'A': [], 'B': []}},
      instances=['A', 'B'])
    F('I', 9, '3.10', readers('claude', read=[{'reject': eie('e1', 'read-failed')}, {'ok': record()}]), [OPEN('r1'), OPEN('r2')],
      {'req': {'r1': wire_req('readFailed'), 'r2': ok_new('new-1')}, 'opening': {'A': []},
       'openingOps': [op('has', K1, False), op('add', K1, True), op('delete', K1, True), op('has', K1, False), op('add', K1, True), op('delete', K1, True)]})
    F('I', 10, '3.10', merge({'sessions': [bound('s4', 'claude', 't2')]}, deferred_read(on_abort=False)), [OPEN('r1'), OPEN('r2'), OPEN('r3'), OPEN('r4', thread='t2'), SETTLE('rd')],
      {'req': {'r1': ok_new('new-1'), 'r2': wire_req('busy'), 'r3': wire_req('busy'), 'r4': ok_existing('s4')},
       'openingOps': [op('has', K1, False), op('add', K1, True), op('has', K1, True), op('has', K1, True), op('delete', K1, True)], 'opening': {'A': []}})


# ==== P: packet layer (honest scenarios; packet faults belong to the packet-level tests) ===========================================
def packet(n, world, script, expect, note):
    F('P', n, '3.11', world, script, expect, layer='packet', note=note)
def group_P():
    packet(1, {}, [SRC('r1')], {'req': {'r1': {'terminal': 'ok', 'keys': ['sources']}}}, 'Honest request for bad-version: the oracle runs the request handler only; the packet layer rejects a bad version before any handler.')
    packet(2, okread('claude'), [OPEN('r1')], {'req': {'r1': ok_new('new-1')}}, 'Honest request for bad-kind.')
    packet(3, deferred_read(on_abort=False), [OPEN('r1'), SETTLE('rd')], {'req': {'r1': ok_new('new-1')}}, 'Honest request for wrong-request-id: the handler result is the same whatever id carries it.')
    packet(4, deferred_read(on_abort=False), [OPEN('r1'), SETTLE('rd')], {'req': {'r1': ok_new('new-1')}}, 'Honest request for wrong-seq.')
    packet(5, deferred_read(on_abort=False), [OPEN('r1'), SETTLE('rd')], {'req': {'r1': ok_new('new-1')}}, 'Honest request for wrong-call-kind.')
    packet(6, okread('claude'), [OPEN('r1')], {'req': {'r1': ok_new('new-1')}, 'opening': {'A': []}}, 'Honest request for done-reentry: one completion.')
    packet(7, readers('claude', list=[{'ok': [item('t1', 'x' * 300), item('t2', 'y' * 200)]}]), [LIST('r1')],
           {'req': {'r1': {'terminal': 'ok', 'value': {'items': [{'thread': 't1', 'title': 'x' * 200}, {'thread': 't2', 'title': 'y' * 200}]}}}}, 'Honest request for packet-too-large: list titles over 200 characters are cut by the entry reader wrapper.')
    packet(8, readers('claude', read=[{'defer': 'r', 'ok': record()}]), [OPEN(f'q{i}', thread=f't{i}') for i in range(1, 34)] + [SETTLE('r' if i == 1 else f'r#{i}') for i in range(1, 34)],
           {'req': {f'q{i}': {'terminal': 'ok'} for i in range(1, 34)}, 'created': 33, 'opening': {'A': []}}, 'Honest 33 concurrent opens for active-request-limit (the handler has no limit of its own).')
    packet(9, okread('claude', record('T', [turn(f'p{i}', f'r{i}') for i in range(20)])), [OPEN('r1')],
           {'req': {'r1': ok_new('new-1')}, 'createdLen': [[0, ['turns'], 20]]}, 'Honest 20-turn request for sequence-limit.')
    packet(10, gworld(createImported=[{'mode': 'fail-before-fill', 'throw': err('e1', message='create failed')}]), [OPEN('r1', source='grok:home')],
           {'req': {'r1': thrown('e1')}, 'counts': {'rm': 1}}, 'Honest request for resource-cleanup-after-copy: a failure after the copy removes the copy.')


# ==== X: extra edges ===============================================================================================================
def group_X():
    F('X', 1, '3.5', readers('claude', list=[{'ok': []}]), [LIST('r1')], {'req': {'r1': {'terminal': 'ok', 'value': {'items': []}}}, 'counts': {'sessionList': 1}})
    F('X', 2, '3.5', merge(readers('claude', list=[{'ok': [item('t1')]}]), {'sessionList': [{'throw': err('e1', message='scan failed')}]}), [LIST('r1')],
      {'req': {'r1': thrown('e1')}})
    F('X', 3, '3.5', merge(readers('claude', list=[{'ok': [item('t1')]}]), {'sessionList': [None, {'throw': err('e1', message='scan failed'), 'throwAt': 0}]}, {'sessions': [sess('s1')]}), [LIST('r1')],
      {'req': {'r1': thrown('e1')}})
    F('X', 4, '3.4', merge(okread('claude'), {'fromAppAction': {'throw': err('e1', message='no human state')}}), [OPEN('r1', mode='same')],
      {'req': {'r1': thrown('e1')}, 'counts': zero('reader', 'opening.add', 'sessionList')})
    F('X', 5, '3.4', merge(gworld(), {'grokAccounts': {'throw': err('e1', message='accounts failed')}}), [OPEN('r1', source='grok:home', target='g1')],
      {'req': {'r1': thrown('e1')}, 'counts': zero('reader', 'opening.add', 'sessionList')})
    F('X', 6, '3.7', rd_reject(eie('e1', 'not-found', name='AbortError')), [OPEN('r1')],
      {'req': {'r1': wire_req('timeout', TIMEOUT_MSG)}})
    F('X', 7, '3.5', readers('claude', list=[{'ok': [item(f't{i}', f'Title {i}') for i in range(51)]}]), [LIST('r1')],
      {'req': {'r1': {'terminal': 'ok', 'itemCount': 51}}, 'counts': {'sessionList': 52}})
    F('X', 8, '3.4', okread('claude', record('Big', [turn(f'p{i}', f'r{i}') for i in range(201)])), [OPEN('r1')],
      {'req': {'r1': ok_new('new-1')}, 'createdLen': [[0, ['turns'], 201]], 'counts': {'uuid': 201, 'now': 1}})
    F('X', 9, '3.5', merge(readers('claude', list=[{'ok': [item('t1', 'Item')]}]), {'sessions': [bound('s1', 'claude', 't1', 'T1a'), bound('s2', 'claude', 't1', 'T2'), sess('s1', 'T1b')]}), [LIST('r1')],
      {'req': {'r1': {'terminal': 'ok', 'value': {'items': [{'thread': 't1', 'title': 'T1b', 'boundSessionId': 's1'}]}}}})
    F('X', 10, '3.8', deferred_read(), [H1, OPEN('r1', cancel='c1', remote=False, tempVisible=False), CANCEL('c1')],
      {'req': {'r1': cancelled('c1')}, 'has': [{'c': 'abort', 'a': {'signal': 'c1', 'reason': {'cancelReason': 'c1'}}}], 'counts': {'createImported': 0, 'rm': 0}, 'opening': {'A': []},
       'note': 'The migration path aborts its signal with no reason: the request throws that DOMException (cancel.reason), not a wire error.'})


# ==== main (groups defined above; see the tail of the file for the rest) ===========================================================
def write_all():
    FIXTURES.mkdir(exist_ok=True)
    for g in GROUPS:
        fns = globals().get('group_' + g)
        if fns is None: continue
        fns()
        assert len(REGISTRY[g]) == 10, (g, len(REGISTRY[g]))
        with open(FIXTURES / f'{g}.json', 'w', encoding='utf-8') as out:
            json.dump(REGISTRY[g], out, indent=1, ensure_ascii=True)
            out.write('\n')
    print('wrote', sum(len(v) for v in REGISTRY.values()), 'fixtures')


if __name__ == '__main__':
    write_all()
