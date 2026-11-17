"""Runs the pinned-TypeScript oracle (reference.mjs) over a batch of scenarios: one node process per batch.
  run_oracle(fixtures_or_scenarios) -> list of observation dicts (id, requests, trace, createdSessions, openingEvents, finalOpening,
  activeTimers, activeListeners, driver)
CLI: oracle.py [scenarios.jsonl [observations.jsonl]]  (stdin / stdout when omitted)."""
import sys
sys.dont_write_bytecode = True
import json
import subprocess

from common import HERE, NODE, strip_case


def run_oracle(cases, env=None, timeout=300):
    stdin = ''.join(json.dumps(strip_case(c), ensure_ascii=True) + '\n' for c in cases)
    proc = subprocess.run(
        [NODE, '--experimental-strip-types', '--no-warnings', '--import', str(HERE / 'reference-hooks.mjs'), str(HERE / 'reference.mjs')],
        input=stdin, capture_output=True, text=True, encoding='utf-8', cwd=HERE, env=env, timeout=timeout)
    if proc.returncode != 0:
        raise RuntimeError(f'oracle failed ({proc.returncode}): {proc.stderr[-2000:]}')
    results = [json.loads(line) for line in proc.stdout.split('\n') if line.strip()]
    if len(results) != len(cases):
        raise RuntimeError(f'oracle returned {len(results)} results for {len(cases)} cases: {proc.stderr[-2000:]}')
    return results


def main(argv):
    src = open(argv[1], encoding='utf-8') if len(argv) > 1 else sys.stdin
    cases = [json.loads(line) for line in src if line.strip()]
    out = open(argv[2], 'w', encoding='utf-8') if len(argv) > 2 else sys.stdout
    for result in run_oracle(cases):
        out.write(json.dumps(result, ensure_ascii=False) + '\n')


if __name__ == '__main__':
    main(sys.argv)
