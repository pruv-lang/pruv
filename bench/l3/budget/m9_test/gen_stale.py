#!/usr/bin/env python3
"""Generate seeded FIRE scenarios using clock calls and world events."""

import argparse
import json
from pathlib import Path
import random
import sys

BUD = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BUD))
sys.dont_write_bytecode = True
from gen_fixtures import MIN, T0, Script, W, budget, iso, ref, sess


def jv(value):
    if isinstance(value, dict):
        return {"obj": [[k, jv(v)] for k, v in value.items()]}
    if isinstance(value, list):
        return {"arr": [jv(v) for v in value]}
    return value


def generate(seed, count):
    rng = random.Random(seed)
    kinds = ("replaced", "unlimited", "halted", "closed", "null", "waiting", "ended", "mixed")
    for i in range(count):
        kind = kinds[i % len(kinds)]
        sid = rng.choice(["s1", "s-long-key", "s-한글", "s-😀"])
        start = rng.choice([0, T0])
        limits = {"timeMinutes": 1}
        sessions = [sess(sid, budget(1, limits=limits))]
        script = Script()
        r = ref(sid, 1)
        script.call("runStarted", r, "main")
        for n in range(rng.randint(1, 4)):
            script.call("runStarted", r, f"waiting-{n}")
            script.call("runWaiting", r, f"waiting-{n}", True)
        script.adv(rng.randrange(1, 500))
        if kind in ("waiting", "ended"):
            if kind == "waiting":
                script.call("runWaiting", r, "main", True)
            else:
                script.call("runEnded", r, "main")
            script.timer("t1", captured=True)
        elif kind == "mixed":
            for n in (2, 3):
                other = f"other-{n}"
                sessions.append(sess(other, budget(n, limits=limits)))
                script.call("runStarted", ref(other, n), "main")
                script.put(sess(other, budget(n, state="closed", limits=limits)))
            script.put(sess(sid, budget(4, limits=limits)))
            script.adv(MIN)  # t1 is due first, then t2 and t3 tie.
            script.timer("t1", captured=True)
        else:
            replacement = None if kind == "null" else budget(
                2 if kind == "replaced" else 1,
                state=kind if kind in ("halted", "closed") else "open",
                unlimited=kind == "unlimited", limits=limits,
                activeSince=iso(start) if kind != "replaced" else None,
            )
            script.put({"id": sid, "budget": replacement})
            if rng.choice([True, False]):
                script.adv(MIN)
            else:
                script.timer("t1", captured=True)
            script.timer("t1", captured=True)
        for step in script.steps:
            if step["ev"] == "call":
                step["args"] = [jv(v) for v in step["args"]]
            elif step["ev"] == "putSession":
                step["session"] = jv(step["session"])
        world = W(sessions, clockStart=start)
        world["sessions"] = [jv(s) for s in sessions]
        world["unit"] = jv(world["unit"])
        yield {"id": f"stale-{kind}-{seed}-{i + 1}", "family": "stale", "seed": seed,
               "world": world, "script": script.steps}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seed", type=int, default=20261117)
    parser.add_argument("--count", type=int, default=300)
    parser.add_argument("--out", type=Path, default=BUD / "work/m9/cases_stale.jsonl")
    args = parser.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open("w", encoding="utf-8") as f:
        for case in generate(args.seed, args.count):
            f.write(json.dumps(case) + "\n")


if __name__ == "__main__":
    main()
