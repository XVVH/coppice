# W-9 verdict baseline — deterministic replay

4916 trajectories, 23479 calls, 84 quarantined rows, 0 no-call rows; 1246 unregistered (hallucinated) calls.

Verdict vector `w9-verdict-vector-v1` sha256: `885d783528a0735c8615ee230645d8b0bb3cddd76e9cf7cfd143f07573f0e367` (93916 lines). Heuristic: `corpus-heuristic-v1` (non-normative).

**W-8 regression contract:** re-running this replay on the same corpus revision MUST reproduce this hash; no verdict may change on corpora containing no revoke events.

Perf: 88932 evaluations in 462 ms (~192494.0 evals/sec).

| cell | allow | deny | escalate | top failed dims |
|---|---|---|---|---|
| floor-t0 | 0 | 23479 | 0 | external_reach (22233), paths.write (22233), reversibility.max (22233) |
| floor-t1 | 0 | 23479 | 0 | reversibility.max (22233), structural:undeclared_action (1246) |
| heuristic-t0 | 0 | 23479 | 0 | external_reach (22233), paths.write (9742), structural:undeclared_action (1246) |
| heuristic-t1 | 22176 | 1297 | 6 | structural:undeclared_action (1246), reversibility.max (51), budget.count:write (6) |

Cells: `floor` = §0 conservative defaults (normative zero-authorship result); `heuristic` = corpus-heuristic-v1 (non-normative, measures the floor↔classifier gap). `t0` = broker-demo probation capability; `t1` = working capability an integrator would mint for a path-less external tool.
