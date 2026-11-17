"""Shared helpers of the L3-9 request-handler tests (oracle.py, check_oracle.py, gen_fixtures.py, later test_impl.py)."""
import sys
sys.dont_write_bytecode = True
import json
import os
import shutil
from pathlib import Path

HERE = Path(__file__).resolve().parent
FIXTURES = HERE / 'fixtures'
NODE = os.environ.get('REQ_NODE') or shutil.which('node') or '/opt/homebrew/bin/node'
GROUPS = 'SFBLAOTUGRECDIPX'
# Fixture keys that are not part of the scenario handed to an implementation.
IGNORED_KEYS = ('name', 'rule', 'expect', 'layer', 'killer', 'note')


def load_group(letter):
    with open(FIXTURES / f'{letter}.json', encoding='utf-8') as f:
        return json.load(f)


def load_all():
    out = []
    for letter in GROUPS:
        out.extend(load_group(letter))
    return out


def strip_case(fixture):
    """The scenario of a fixture: everything except the annotation keys."""
    return {k: v for k, v in fixture.items() if k not in IGNORED_KEYS}
