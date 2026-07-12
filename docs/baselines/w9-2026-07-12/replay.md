# W-9 verdict baseline — deterministic replay

4922 trajectories, 23536 calls, 78 quarantined rows, 0 no-call rows; 1263 unregistered (hallucinated) calls.

Verdict vector `w9-verdict-vector-v1` sha256: `c65b80d81b4c0d276db4252c58948b2c054b5bd020e3ff8d091faf610fb63c8f` (94144 lines). Heuristic: `corpus-heuristic-v1` (non-normative).

**W-8 regression contract:** re-running this replay on the same corpus revision MUST reproduce this hash; no verdict may change on corpora containing no revoke events.

Perf: 94144 evaluations in 298 ms (~315919.0 evals/sec).

| cell | allow | deny | escalate | top failed dims |
|---|---|---|---|---|
| floor-t0 | 0 | 23536 | 0 | external_reach (23536), paths.write (23536), reversibility.max (23536) |
| floor-t1 | 0 | 23536 | 0 | reversibility.max (23536), action.allow (1263) |
| heuristic-t0 | 0 | 23536 | 0 | external_reach (23536), paths.write (10322), action.allow (1263) |
| heuristic-t1 | 22216 | 1314 | 6 | action.allow (1263), reversibility.max (199), budget.count:write (6) |

Cells: `floor` = §0 conservative defaults (normative zero-authorship result); `heuristic` = corpus-heuristic-v1 (non-normative, measures the floor↔classifier gap). `t0` = broker-demo probation capability; `t1` = working capability an integrator would mint for a path-less external tool.
