# W-9 baseline — first run (2026-07-12)

Produced by `asf corpus baseline` (crates/asf-cli/src/corpus/) over
corpora fetched by `scripts/fetch-corpora`. Method, lanes, and hard
boundaries: `docs/agent-trace-corpora-2026-07-11.md`. Corpus material
is fixtures/calibration only — never founding examples (R1/R6) — and
stays operator-side (`corpora/` is gitignored; MCP-Flow declares no
license, so only aggregates appear here).

## Input pins

- **Toucan-1.5M** (Apache-2.0): config `Kimi-K2`, split `train`, rows
  0–4999, HF revision `0df3cf37f2abefb380370cfb02eabea2a35ae782`.
- **MCP-Flow** (no license declared): revision
  `c7e3f6b859490c559d18ba2c3f426b3c4a2ff23c`, the four
  `*_seen_test_tool10.json` schema sets (deepnlp, glama, mcphub, mcpso).
- Per-file sha256 manifests live in each corpus dir's `FETCH.json`
  (operator-side). The Toucan `/rows` endpoint cannot pin a revision;
  the fetch script records and enforces revision equality instead.

## Census headline (census.json / census.md)

**9,807 tools across 1,921 servers. Zero MCP ToolAnnotations anywhere.**
Reversibility, action class, side-effect/locality, egress, store
binding (SI-16), and path arguments are all **0% derivable** from what
real tools declare; only domain-shaped metadata is present (100%, but
shallow — marketplace/category strings, and the taxonomy governance
problem stays open). The §0 conservative floor therefore claims **100%
of the real-world universe**: every tool is irreversible-egress until
the ratification loop reclassifies it.

Serialization caveat, stated plainly: both corpora serialize tools in
OpenAI function form, which structurally drops MCP annotations — so
"zero annotations" conflates "never declared" with "dropped in the
serialization agents actually see." Either way the declared-metadata
channel §4 could derive from is empty in practice, and MCP itself marks
those hints untrusted. Whether annotations may ever count as declared
registration metadata is a spec question, not a harness decision.

## Verdict baseline (replay.json / replay.md)

4,922 replayable trajectories (of 5,000 rows: 66 quarantined for
malformed call-argument JSON — a generator artifact worth knowing
about, 1.3% — and 12 for ambiguous tool→server mapping; zero no-call
rows), 23,536 calls, 94,144 evaluations across four cells:

| cell | allow | deny | escalate | dominant failures |
|---|---|---|---|---|
| floor-t0 | 0 (0%) | 23,536 | 0 | external_reach, paths.write, reversibility.max ×23,536 |
| floor-t1 | 0 (0%) | 23,536 | 0 | reversibility.max ×23,536 |
| heuristic-t0 | 0 (0%) | 23,536 | 0 | external_reach ×23,536; paths.write ×10,322 |
| heuristic-t1 | 22,216 (94.4%) | 1,314 | 6 | action.allow ×1,263; reversibility.max ×199; budget ×6 |

Readings: under zero-authorship defaults the broker denies **all** real
MCP traffic in every posture — the ratchet loop is not an optimization,
it is the product. The entire distance from 0% to 94.4% is one
non-normative classifier (`corpus-heuristic-v1`) plus a working-tier
capability — i.e. exactly the caveats the compilation loop is designed
to earn, one ratified rule at a time. 1,263 calls (5.4%) name tools not
offered in-context (hallucinated calls); action.allow catches every one
mechanically. All three evaluator outcomes occur (6 escalations =
budget exhaustion as the sole failure in >20-write trajectories).

**Verdict vector** (`w9-verdict-vector-v1`, 94,144 lines):

```
sha256: c65b80d81b4c0d276db4252c58948b2c054b5bd020e3ff8d091faf610fb63c8f
```

**W-8 regression contract:** after capability closure/revocation lands,
re-running `asf corpus replay` on the same pinned corpus MUST reproduce
this hash bit-for-bit — corpora containing no revoke events may not
change a single verdict. Regenerate with `--vector` to diff line-level
on any mismatch.

## Performance (release build, single thread)

- Pure evaluator: **~316k evaluations/sec** (94,144 evals in 298 ms) —
  decision-time checking is nowhere near the bottleneck at wedge scale.
- Substrate ingest: **~3.5k events/sec** end-to-end (1,000 observed-mode
  manifests, 5,537 signed tool_call events with per-payload encryption,
  JCS canonicalization, hash-chaining, and per-call store snapshots;
  8,697 events verified across 1,001 spans afterwards; 32 MB fabric.db).
  First measured input to `scalability-analysis-2026-07-10.md` and the
  F2 decision on the W-6 path.

## Encodability result

**Zero spec-level gaps.** Every parseable trajectory expressed cleanly
as an observed-mode manifest (M7) with §6 `register`/`intent`/
`tool_call` events; ledger accounting explained every root; no new SIs
filed. The quarantined 1.6% failed on corpus-side malformations (gap
kinds and bounded samples in gaps.json), not on ASF schema limits.

## Regenerate

```
scripts/fetch-corpora all 5000        # + mcpflow-testdata (idempotent, pinned)
cargo build -p asf --release
./target/release/asf corpus baseline --corpora corpora \
    --out docs/baselines/w9-2026-07-12 --home <scratch> --ingest-limit 1000
```
