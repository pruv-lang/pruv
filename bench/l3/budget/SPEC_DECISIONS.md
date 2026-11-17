# L3-10 M0–M2 spec questions and the decisions taken

Q1. Scenario JSON details (§7.3 only names the operations). Decided: the concrete contract in SCENARIO.md
    (call args are JV; `{sess:"id"}` resolves to the stored session object; `{"ev": ...}` steps).
Q2. Dependency reply queues when exhausted (§7.4 silent). Decided: fixed defaults runningDelegationIds [],
    isQuiet false, releaseHeld false; the default use is recorded in the trace like any reply.
Q3. Native timer extremes (§15.2). Decided: the fake follows Node's rule `if (!(delay >= 1 && delay <= 2147483647)) delay = 1`
    for the due time and records the raw delay.
Q4. JV codec reuse. bench/l3/entry/reference-primitives.mjs exports enc/dec but imports the entry reference chain
    (engine-import tail). Moving the codec to bench/l3/lib would edit other pieces, outside this bundle's boundary.
    Decided: oracle-world.mjs has its own codec with identical conventions; fold into a shared lib in the L3 restructure bundle.
Q5. Excluded-domain marking (§0.5, §2.3) is not in the §7.5 observation shape. Decided: observation field `excluded: [names]`
    (empty when none); a scripted dependency throw adds `dep-throw`; a scenario `domain` tag is copied in.
Q6. KD-1 vs §7.2 per-call now overrides: overrides exist in the oracle for fixtures; Q never generates them.
Q7. §14 M0 asks for a "minimal in-memory seed stream": implemented as `oracle.py --smoke N` (not the Q generator).
