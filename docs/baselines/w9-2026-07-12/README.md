# W-9 baseline — v2 (2026-07-12)

Produced by `asf corpus baseline` (crates/asf-cli/src/corpus/) over
corpora fetched by `scripts/fetch-corpora`. Method, lanes, and hard
boundaries: `docs/agent-trace-corpora-2026-07-11.md`. Corpus material
is fixtures/calibration only — never founding examples (R1/R6) — and
stays operator-side (`corpora/` is gitignored; MCP-Flow declares no
license, so only aggregates appear here).

> **v2 (2026-07-12, same day).** The fresh-eyes review after v1 merged
> (PR #29) found a mistranscribed MCP-Flow revision pin, a census
> derivability figure inflated by harness-injected provenance tags, and
> several converter fidelity gaps. v2 re-runs on the corrected harness:
> slug-identity and result-name guards (18 more rows quarantine instead
> of silently merging/mis-pairing), broker-faithful structural denial
> for undeclared actions, slug-space prefix stripping (≈150 heuristic
> misclassifications fixed), split declared-categories vs provenance,
> and load-time hash verification of every corpus file against
> FETCH.json. v1's vector hash, retained for the record only:
> `c65b80d81b4c0d276db4252c58948b2c054b5bd020e3ff8d091faf610fb63c8f`.

## Input pins (machine-recorded; never hand-transcribed)

- **Toucan-1.5M** (Apache-2.0): config `Kimi-K2`, split `train`, rows
  0–4999, HF revision `0df3cf37f2abefb380370cfb02eabea2a35ae782`.
- **MCP-Flow** (no license declared): revision
  `c7e3f6b85949d844a6cebddd627e13eb3bd6d4ca`, the four
  `*_seen_test_tool10.json` schema sets (deepnlp, glama, mcphub, mcpso).
- Authoritative copies of both pins live in each corpus dir's
  `FETCH.json` (per-file sha256 included) and are echoed into
  `census.json .inputs` / `replay.json .corpus_revision` by the harness
  itself. The loader hash-verifies every consumed file against
  FETCH.json; `scripts/fetch-corpora verify` checks everything at rest.
  (The Toucan `/rows` endpoint cannot pin a revision; the fetch script
  records and enforces revision equality instead.)

## Census headline (census.json / census.md)

**9,795 tools; zero MCP ToolAnnotations anywhere.** Reversibility,
action class, side-effect/locality, egress, store binding (SI-16), and
path arguments are all **0% derivable** from anything the tools or
servers declare. Domain-shaped signal exists for **22.0%** of the
universe (2,155 Toucan tools) — and even that is third-party labeling
(marketplace/crawler categories), not §4 self-declared registration
metadata; the MCP-Flow schema sets declare no categories at all
(harness provenance tags count as nothing). The §0 conservative floor
therefore claims **100% of the real-world universe**: every tool is
irreversible-egress until the ratification loop reclassifies it.

Server counts are per-source upper bounds, not distinct servers:
Toucan 358 (slug-guarded); MCP-Flow ≈1,563 by best-effort name-prefix
recovery, which both over-splits (underscore-less tool names become
their own "server") and exceeds upstream's declared 1,166 total —
see `servers_note` in census.json. No cross-source dedup is attempted.

Serialization caveat, stated plainly: both corpora serialize tools in
OpenAI function form, which structurally drops MCP annotations — so
"zero annotations" conflates "never declared" with "dropped in the
serialization agents actually see." Either way the declared-metadata
channel §4 could derive from is empty in practice, and MCP itself marks
those hints untrusted. Whether annotations may ever count as declared
registration metadata is a spec question, not a harness decision.

## Verdict baseline (replay.json / replay.md)

4,916 replayable trajectories (of 5,000 rows: 66 quarantined for
malformed call-argument JSON — a generator artifact worth knowing
about — and 18 for unsluggable server identities; 84 total, 1.7%; zero
no-call rows), 23,479 calls, 93,916 vector lines across four cells
(88,932 evaluator invocations; 1,246 hallucinated calls per cell deny
structurally before evaluation, exactly as `broker::propose_call`
would):

| cell | allow | deny | escalate | dominant failures |
|---|---|---|---|---|
| floor-t0 | 0 (0%) | 23,479 | 0 | reversibility.max / external_reach / paths.write ×22,233; structural ×1,246 |
| floor-t1 | 0 (0%) | 23,479 | 0 | reversibility.max ×22,233; structural ×1,246 |
| heuristic-t0 | 0 (0%) | 23,479 | 0 | external_reach ×22,233; paths.write ×9,742; structural ×1,246 |
| heuristic-t1 | 22,176 (94.5%) | 1,297 | 6 | structural ×1,246; reversibility.max ×51; budget ×6 |

Readings: under zero-authorship defaults the broker denies **all** real
MCP traffic in every posture — the ratchet loop is not an optimization,
it is the product. The entire distance from 0% to 94.5% is one
non-normative classifier (`corpus-heuristic-v1`) plus a working-tier
capability — i.e. exactly the caveats the compilation loop is designed
to earn, one ratified rule at a time. 1,246 calls (5.3%) name tools not
offered in-context; every one is structurally uncallable (§4) before
any caveat is consulted. All three evaluator outcomes occur (6
escalations = budget exhaustion as the sole failure in >20-write
trajectories).

**Verdict vector** (`w9-verdict-vector-v1`, 93,916 lines):

```
sha256: 885d783528a0735c8615ee230645d8b0bb3cddd76e9cf7cfd143f07573f0e367
```

**W-8 regression contract — everything it pins:** after capability
closure/revocation lands, re-running `asf corpus replay` MUST reproduce
this hash bit-for-bit under: (1) the corpus revisions above per
FETCH.json (loader-enforced), (2) the same harness commit both sides of
the kernel change (a harness edit is a different experiment — rerun
both sides on one commit), (3) `corpus-heuristic-v1` and
`w9-verdict-vector-v1` as recorded in replay.json, and (4) the
identical 84-row quarantine set recorded in replay.json. Under those
pins, corpora containing no revoke events may not change a single
verdict. Regenerate with `--vector` to diff line-level on any mismatch.
The fixture-scale twin of this contract is enforced in CI: the
committed test fixture's vector hash is pinned as a constant in
`crates/asf-cli/tests/corpus.rs`.

## Performance (release build, single thread)

- Pure evaluator: **~192k evaluator invocations/sec** (88,932 in
  462 ms, structural short-circuits excluded from the count; wall
  includes per-call derivation/slugging) — decision-time checking is
  nowhere near the bottleneck at wedge scale.
- Substrate ingest: **~3.6k events/sec** end-to-end (1,000
  observed-mode manifests, 5,514 signed tool_call events with
  per-payload encryption, JCS canonicalization, hash-chaining, and
  per-call store snapshots; 8,673 events verified across 1,001 spans;
  32 MB fabric.db).
- **Not measured here:** promotion-gate replay throughput (three-way
  merge + trace-vs-capability at the gate). The ingest lane is
  observed-mode and never branches or promotes; the gate-side corpus
  measurement is deferred to W-8's gate work, where it lands together
  with the closure semantics it must exercise.

## W-8 verification (2026-07-12, same day — closure landed)

- **Verdict invariance held:** with A22/§5.4 capability closure
  implemented (decision-time liveness + gate liveness-at-offset), the
  full replay under the pins above reproduced the v2 vector hash
  **bit-for-bit**
  (`885d783528a0735c8615ee230645d8b0bb3cddd76e9cf7cfd143f07573f0e367`,
  93,916 lines, identical 84-row quarantine). Revocation-free corpora changed zero verdicts, as the
  contract requires. Harness note: W-8 added a `gate-replay` subcommand
  (CLI plumbing only); the replay/derivation path is byte-identical.
- **Gate-replay throughput (the measurement deferred above), landed via
  `asf corpus gate-replay` (gate-replay.json):** 1,000 ingested
  manifests re-verified read-only (span verification, M7 activation,
  §5.4 closure at each effect's offset, full caveat re-evaluation) in
  128.2 s — **~8 manifests/s, ~43 replayed calls/s, ~128 ms per gate
  call** on the 8,673-event substrate. Reading: a single
  promotion-time gate on a wedge-scale substrate costs ~130 ms
  (invisible per session); batch replay is quadratic in substrate size
  because each gate call re-verifies the whole substrate span and
  reloads all events. That is tripwire data (ADR 0002 family), not a
  defect: incremental span verification / an authority-event index is
  future work triggered by substrate growth, not by this number.

## Encodability result

**Zero spec-level gaps.** Every parseable trajectory expressed cleanly
as an observed-mode manifest (M7) with §6 `register`/`intent`/
`tool_call` events; ledger accounting explained every root; no new SIs
filed. The quarantined 1.7% failed on corpus-side malformations and
identity ambiguities the mapping refuses to guess about (gap kinds and
bounded samples in gaps.json), not on ASF schema limits.

## Regenerate

```
scripts/fetch-corpora all 5000        # idempotent, revision-pinned
scripts/fetch-corpora verify          # hash-check everything at rest
cargo build -p asf --release
./target/release/asf corpus baseline --corpora corpora \
    --out docs/baselines/w9-2026-07-12 --home <fresh scratch dir> \
    --ingest-limit 1000
```
