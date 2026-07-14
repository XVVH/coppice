# Agent State Fabric (ASF)

Neutral, agent-agnostic state fabric: delegation manifests binding four lineages
(state, authority, behavior, trace), snapshot-backed undo for owned state, brokered
capabilities for external effects, human-ratified compilation loops (skills up,
caveats down). Read `docs/agent-state-fabric-brief.md` (why/what, v0.2) and
`docs/asf-schema-spec.md` (the constitution, v0.9) before writing any code.
The spec wins over this file wherever they disagree. Spec ambiguities found
while implementing go to `docs/spec-issues.md` (never silently interpret);
SI-1…SI-21 are resolved (SI-20 → A20/M8 in v0.5; SI-21 → A21/M7 in v0.6); SI-22 is interpreted (gate replay clock, W-2); SI-24 is resolved (A22/§5.4 in v0.7 — capability early closure; implemented by W-8 under its gated matrix: A22 two-sided contract, targeted `a22_*` mutation lane, W-9 verdict-invariance reproduced bit-for-bit); SI-23 (actuation vs approval surfaces) is OPEN — no actuation-scoped tool may register before SI-23 resolves. SI-25 is resolved (A23/§6.2 in v0.8 — authenticated global order: signed per-home global_seq layer 1 normative now, external anchor layer 2 graduation-gated; W-15 implements; ratified via W-20 determinations D1–D7/S1–S5 in ADR 0006). SI-26…SI-30 remain OPEN from the 2026-07-12 cryptographic mechanism audit (signed type/domain, key lifecycle, AEAD envelope, post-shred generations, redaction commitments). SI-31/SI-33/SI-34 are resolved (A24–A26 in v0.9 — owned-state transition protocol §5.3, consumed authority §5.5, approval candidate binding §6; W-20 retro-ratification of the merged W-14 protocols via PR #48, determinations D31/D33/D34 plus five rounds of review adjustments R1–R23 in ADR 0007 — round 2 added the recovery capture record and positional freshness, round 3 hardened them, round 4 extended temporal binding to both consumption consumers, rounds 4–5 surfaced as-built defects and completed canonical-entry identity; spec-code deltas tracked as RF-35–RF-37/RF-39 (adjusted beyond as-built) and RF-40 (as-built fs-pipeline defects), carried by W-15/W-22; G13 files the outstanding negatives; unenforced clauses bind at their carrier gates per the ledger). SI-32 (store publication/filesystem attacker model) is OPEN from the same 2026-07-13 PR #43 review-cycle analysis and is next in W-20's batch. SI-35 (workboard domain labels vs domain-taxonomy governance, recovered from codex/workboard-dogfood at the W-21 revival decision), SI-37 (TracePosition agreement predicate; W-15 enforces fail-closed from day one), and SI-38 (TracePosition genesis representation; W-15 implements a bootstrap-event floor provisionally) are OPEN; SI-36 (mid-run "actually do Y" amendments, §3.1/M1/M5) merged via PR #46. SI-39 (recovery-window divergence: automated multi-window preservation; filed by PR #48's round-3 review, the narrow-and-file remedy ratified as R14) is OPEN. New issues start at SI-40.

## Built — Stages 1–3 (kernel, spine, promotion gate). Current queue: docs/roadmap.md

The four components below exist and are dogfooding; work-in-flight and
ordering live in `docs/roadmap.md` (W-n), not here.

1. **Broker daemon**: MCP proxy in front of a Hermes agent; capability evaluation
   (conjunctive caveats, fail-closed on unknown dimensions); capability closure
   (A22/§5.4: signed `revoke`, event-derived liveness at decision + gate, operator
   `asf revoke` kill switch — never an MCP-reachable tool); credential injection
   (agent never sees secrets); daemon-owned approval surface — approvals NEVER pass
   through the agent's stdin or chat loop (spec C2, day-one requirement).
2. **Snapshot coordinator**: step-boundary, content-addressed snapshots over the
   knowledge vault (fs), Hermes skills dir, and Hermes memory DB (sqlite). All
   roots captured atomically per manifest; revert restores ALL roots together.
3. **Trace substrate**: append-only, per-span hash-chained, signed events (sqlite
   is fine); payloads separately encrypted per-payload (crypto-shredding).
4. **Promotion gate**: three-way merge, conflicts never auto-resolve in the
   agent's favor; operation classes with rename detection.

First integration: workflow 3 (vault maintenance) — no external tool
registrations needed; failure mode during dogfooding is "revert and shrug."

**Implementation-status boundary:** the dogfooding baseline currently provides
logical payload tombstoning, not forensic erasure across SQLite residue,
snapshots, and backups. It also fails closed when the broker/proxy is
unavailable; loud fail-open degradation and `on_broker_outage` remain deferred
until live-egress integration. These are tracked release gates, not relaxations
of the spec or the non-negotiable fail posture below. The full set of
posture-dependent shortcuts — every simplification that is safe only because
of the current same-user / cooperative / local-only / no-actuation / single-
tenant / debug posture — and the graduation gate each transition must clear is
`docs/posture-assumptions.md` (`P-n`). Add a row there when you take a new
shortcut; read the relevant gate before flipping a posture invariant.

## Non-negotiable invariants (do not relax in code)

- Conservative defaults are the free ones: undeclared reversibility = irreversible;
  undeclared egress = egress; unknown caveat dims fail closed.
- Zero authorship: no screen may ask the user to author policy upfront; policy
  enters only via the ratification loop.
- The agent never holds a real credential; anything in agent context is
  presumed exfiltratable.
- Capabilities: mandatory expiry, bound to their manifest (cap.bound_manifest),
  attenuation is per-dimension subset-checking, broker-minted (F1 resolved).
- k >= 3 founding examples per standing rule; counterfactuals mandatory at
  compensable-or-worse; domain match enforced; behavior pins are domain-scoped.
- Guards on destructive actions are broker_verified — never trust agent-supplied
  target metadata.
- Fail posture for this market: fail open with loud, ledger-visible degradation
  (per-capability invertible via on_broker_outage).

## Known open problems — do not silently "solve" these in passing

Cross-run memory taint (A3); compensation fidelity grades; multi-actor visibility
policy; domain taxonomy governance; F2 canonicalization (JCS vs IPLD — decide
before anything is published). If a design decision touches one, surface it.

## Two-sided verification discipline

Every new or renamed Rust test MUST be registered in `tests/contracts.tsv`
under an invariant contract. Every contract MUST retain at least one positive
test (valid behavior succeeds) and one negative test (invalid behavior fails
closed); `supporting` tests may supplement but never replace the pair. Negative
tests MUST assert that the protected effect did not occur, not merely that an
error was returned. `scripts/ci contracts` enforces the registry against the
compiled test inventory and rejects growth of `tests/contracts-baseline.txt`.
Changes to enforcement logic MUST also extend or exercise a stable targeted
mutation lane, or state in the PR why mutation testing cannot apply.

Cross-layer conformance (G9, founded by the PR #33 review — the lanes above
verify only what is written): spec-implementing changes MUST carry a
conformance sweep in the PR — every normative sentence in the touched spec
sections mapped to a named enforcing line plus a negative test, or an
explicit SI/G/P filing for why not. Deliberately asymmetric principles are
verified edge-by-edge and dimension-by-dimension, never by structural
analogy with a sibling code path. Operator-facing commands test their
process-level contract (exit status), not just reply bodies. Evidence
claims in PRs and summaries are scoped to what each lane measures.
Authority-surface PRs merge only after independent-context review; the
author's own fresh-eyes pass does not satisfy this.

Review-finding triage (founded by the PR #43 review cycle): a finding
whose remedy introduces a new persistent record, a new commit point, a
new reconstruction of authoritative state, or a change in which source
any consumer treats as authoritative is protocol-class — it MUST be
filed as an SI and its protocol ratified before implementation; it may
shrink the PR under review, never grow it. In-PR remediation is for
mechanism-class findings: a missing or wrong predicate inside
already-ratified semantics. The tiebreaker is whether the governing
clause already exists, not how structural the fix looks — RF-32
(tools/list liveness) trips the source-change test, yet §5.4 already
supplied its clause, so it was mechanism-class. Blocking review findings MUST cite the
written clause they enforce (spec section, ratified protocol/ADR, or
posture row); a finding with no citable clause is a filing (SI/RF/G/P),
not a blocker. Un-parking a posture row or a parked roadmap item is an
operator gate decision, never a review outcome.

## Session-end git contract

Work exists once it is on `origin`, not before. Push your branch before
the session ends — an unpushed branch in a private worktree is invisible
to every other session and to fresh clones (the roadmap's file-first
rule, applied to git; founding cases: ADR 0006 sat unpushed on
`agent/si25-authenticated-head-design` while later sessions cited it as
if on record, and the workboard profile sat unmerged for three days).
If the sandbox blocks the push, escalate that push outside the sandbox
before ending — never end a session with committed-but-unpushed work.
After a PR merges, its head branch is deleted (the repo auto-deletes
remote heads; delete the local copy too). Cite PR numbers, not commit
SHAs, in docs: squash-merge rewrites SHAs, so branch-local ids resolve
only while their branch or an archive tag survives (the RF-4/5/8/9
"fixed in 5350010" citations resolve via the `archive/testing-suite`
tag).

## Context

Design rationale lives in the originating design session (every schema
amendment A1–A14 traces to a specific exchange; docs carry the changelog).
Wedge: Hermes agent client; workflows = web research→vault, Discord message
management, vault maintenance; channels = terminal (local_session) + Telegram
(platform_oauth, sender-bound per C5).
