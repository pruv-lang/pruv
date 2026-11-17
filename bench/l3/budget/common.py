"""Shared helpers of the L3-10 request-budget corpus (oracle.py, check_oracle.py, gen_fixtures.py, q_gen.py, later q.py/test_impl.py).
Other scripts import these exact names: HERE, FIXTURES, WORK, NODE, GROUPS, IGNORED_KEYS, strip_case, load_group, load_all, TS_ENV."""
import sys
sys.dont_write_bytecode = True
import json
import os
import shutil
from pathlib import Path

HERE = Path(__file__).resolve().parent
FIXTURES = HERE / 'fixtures'
WORK = HERE / 'work'
NODE = os.environ.get('BUDGET_NODE') or shutil.which('node') or '/opt/homebrew/bin/node'
GROUPS = 'ABCDEFGHIJKLMN'
# Fixture keys that are annotations, not part of the scenario handed to an oracle or an implementation.
IGNORED_KEYS = ('name', 'rule', 'expect', 'layer', 'killer', 'note', 'kd')


def load_group(letter):
    with open(FIXTURES / f'{letter}.json', encoding='utf-8') as f:
        return json.load(f)


def load_all():
    out = []
    for letter in GROUPS:
        out.extend(load_group(letter))
    return out


def strip_case(fixture):
    """The scenario of a fixture: everything except the annotation keys (id stays)."""
    return {k: v for k, v in fixture.items() if k not in IGNORED_KEYS}


def TS_ENV():
    """Environment for running the pinned TypeScript through Node: GROK_TS_TRANSPILER defaults to the Lair checkout's typescript package."""
    env = dict(os.environ)
    env.setdefault('GROK_TS_TRANSPILER', '/Users/spica4345/Projects/lair/node_modules/typescript')
    return env
