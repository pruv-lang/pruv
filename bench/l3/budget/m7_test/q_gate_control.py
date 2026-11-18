#!/usr/bin/env python3
"""Feed a wholly unsupported case to both Q runners and check their exit codes."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile

import run_m7


def main():
    work = Path(run_m7.WORK)
    work.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=work) as tmp:
        cases = Path(tmp) / "unsupported.jsonl"
        oracle = Path(tmp) / "oracle.jsonl"
        case = {"id": "unsupported", "world": {"sessions": [], "unit": {"obj": []}},
                "script": [{"ev": "call", "req": "r0", "op": "hasCard", "args": []}]}
        cases.write_text(json.dumps(case) + "\n")
        oracle.write_text(json.dumps({"id": case["id"], "requests": [], "trace": []}) + "\n")
        rejected = True
        for milestone in ("m7", "m10"):
            runner = Path(run_m7.BUD) / f"{milestone}_test/run_{milestone}.py"
            result = subprocess.run([sys.executable, str(runner), "q", "--cases", str(cases),
                                     "--oracle", str(oracle)], capture_output=True, text=True)
            print(result.stdout, end="")
            print(result.stderr, end="")
            if result.returncode != 1 or "FAIL Q every case is not-implemented" not in result.stdout:
                rejected = False
        if rejected:
            print("PASS both negative Q runs exited 1")
            return 0
        print("FAIL a negative Q run did not reach the gate")
        return 1


if __name__ == "__main__":
    sys.exit(main())
