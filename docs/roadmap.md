# Roadmap — in-flight and queued work

> **Boundary rule:** this file orders *work* — features, refactors,
> milestones, demos. It never restates findings: spec ambiguities live in
> `spec-issues.md` (SI-n), implementation defects in `review-findings.md`
> (RF-n), test gaps in `testing-theory.md` (G-n), dogfooding cases in
> `dogfooding-rubric.md` (DF-\*). A work item that spawns one of those
> links it; the detailed truth lives there.
>
> **Discipline:** a priority agreed in conversation that is not filed here
> before the session ends is considered lost — file first, build second
> (the SI rule, applied to sequencing). Every PR that completes an item
> moves it to the done log with the PR number.

Status legend: **in-flight** (owner working now) · **queued** (ordered,
not started) · **blocked** (named blocker) · **parked** (deliberately
deferred; un-park trigger named). Ids are `W-n`, stable once assigned.

---

## In flight

**W-1 — Phase 5 dogfooding: real workflow-3 sessions.** Owner: operator.
Drive vault-maintenance sessions through the brokered tools per
`dogfooding.md`; track denial-FP rate, capability gaps, tripwires. First
post-A21 session doubles as live verification of the brokered mode
declaration (ledger shows `authority: {"mode":"brokered"}` + the grant).
Approvals accumulate as founding examples for W-3 regardless of when the
clerk lands. *Provenance: standing plan; re-confirmed 2026-07-10.*

**W-2 — Unified evaluator at the gate.** Done — see the done log. The
mutation-lane stretch goal was subsumed: gate dimension logic now IS
`evaluate.rs`, which the scheduled mutation lane already covers, and the
parallel A21/M7 lane covers the grant-binding helpers.

## Queued (ordered)

**W-3 — Start the accretion counters: dumb clerk + TrustRecords.**
Deterministic, no model. `asf rules candidates`: cluster approval events
by (caveat, action-class, domain); at k ≥ 3 propose the least-general
covering rule with ledger-derived counterfactuals for ratification (§7.1
schema). TrustRecords as counters over existing events per (principal,
domain, behavior version) (§7.2). Sequenced after W-1 produces real
approvals to cluster — but not far after; this is where authority stops
evaporating at session end. *Provenance: 2026-07-10 review (highest-
confidence convergent recommendation: ratchet before judge).*

**W-4 — Attestation + containment, as a pair.** The intent/behavior
analog of what A21 did for authority. (a) Per-lineage assurance classes
generalizing M7's observed/brokered split — the proxy today records a
placeholder behavior bundle and a reused intent (`proxy.rs`), which
assurance labeling makes ledger-visible instead of silently nominal;
plus the delegation-lifecycle event family (`delegation.begin`,
`behavior.changed`, `checkpoint`, `end`) as an open, protocol-neutral
extension a cooperating client can emit. (b) The sandbox profile: a
container/microVM whose only door is the proxy, converting the two-surface
hygiene convention (`dogfooding.md`) into topology. Either alone leaves a
hole the other covers. Needs a spec conversation before code (amendment-
sized). *Provenance: 2026-07-10 review; SOL's attestation framing +
containment counterpart.*

**W-5 — Cross-boundary recovery demo.** One run touching the vault + a
fixed external surface + a human-visible message; one gesture: mechanical
rollback + drafted compensation + honest irreversible listing, with
fidelity grades (A7 margin note, schematized enough to render). Mocked
external side is acceptable — the demo communicates the category. Framing:
"rewind what can be rewound, compensate what can be compensated, prove
what cannot." *Provenance: 2026-07-10 review (both reviews converged).*

**W-6 — Conformance surface.** Machine-readable schemas, canonical JCS
fixtures (G2's language-neutral vectors), attenuation pass/fail pairs —
the published form of the invariant-mapped tests (testing-theory §"future
conformance suite"). Includes: **LICENSE file** (Cargo.toml declares
Apache-2.0; no license text ships — blocked on owner: copyright holder
name), and the F2 execution checkpoint sits on this path (spec §9: decide
before anything is published). *Provenance: 2026-07-10 review; neutrality
strategy (brief §9).*

**W-7 — Commercial/public naming decision.** Before anything ships
publicly. "Coppice" has name-adjacent products (coppiceapp.com — Mac
note-canvas tool, adjacent space); no active collision verified at
coppice.ai (claim from external review did not verify, 2026-07-10).
Owner: operator. *Provenance: 2026-07-10 review.*

## Candidates (ideas from the design reviews — not yet committed work)

Recorded so they aren't lost to conversation; promote to queued by
decision, not by drift. From the 2026-07-10 review sessions:

- **Taint wall as the security headline** — R2's `taint.egress` is the
  zero-config anti-exfiltration primitive (ADR 0005); productize the
  framing when egress lands.
- **Counterfactual replay as a product ("audition")** — run a new
  agent/skill/model against last week's real manifests under
  `external_reach: none`; diff against what happened. Feeds TrustRecords;
  re-earns pins after skill mutation. Primitives already exist.
- **Effect receipts** — broker-signed verifiable receipts (manifest +
  intent + capability refs) per external effect; the network-effect play.
- **Provenance-aware store interface** — publish the interface that turns
  A3 (memory taint) from open problem into an ecosystem standard; sidecar
  lineage index as reference implementation.
- **Insurable delegation** — TrustRecord as actuarial object; manifest
  trail as underwriting evidence.
- **North-star metrics** — consequence-weighted verified work per minute
  of human attention; approval compression ratio; time-to-first-ratified-
  rule; % sessions fully silent. Adopt alongside the denial-FP rate when
  W-3 gives them substance.

## Parked (trigger-gated — do not start without the trigger)

- **F2 CID/DAG migration** — ADR 0002 tripwires (CAS growth, memory-db
  size, SI-6 noise) or spec publication (W-6), whichever first.
- **Checkpoint session boundary** — ADR 0004 tripwire (ops-per-promotion /
  conflict incidence).
- **R2 read-authority family** — before the first `external_reach: live`
  tool (ADR 0005 binds the shape; dogfooding graduation criterion).
- **Trace-head anchoring + crash-atomic multi-root commit + RF-13** —
  bundle; production/live-egress release gates (security audit).
- **Broker-outage loud fail-open + `on_broker_outage`** — live-egress
  integration.
- **Forensic crypto-shredding** — post-dogfooding release gate.
- **Multi-actor roots/visibility, HA topology, model judge/clerk-as-model,
  Tier-2/3 stores** — deferred until the W-1..W-3 loop produces pull
  (2026-07-10 review: both external reviews independently concurred).
- **SessionContext refactor** (replace home-global `current_manifest`/
  `current_span`) — trigger: second concurrent session in one home
  (scalability analysis), or opportunistically with W-4's proxy work.

## Done (recent — full history is git)

- **W-2 — unified evaluator at the gate** — the gate replays every signed
  tool_call through the decision-time evaluator (all seven dimensions, not
  four; context from the registered action; meters/exemptions from signed
  events; clock = the event's `at` per SI-22, interpreted). Ships with the
  two-sided contract registry, the four m7 grant-activation adversarial
  cases, and the targeted A21/M7 mutation lane (PR #20, 2026-07-10).
- **A21/SI-21** — authority binds by declaration + grant event; spec v0.6;
  adversarial gate coverage; RF-13 filed (PR #18, 2026-07-10).
- **Invariant-driven verification hardening** from the security audit
  (PR #17, 2026-07-10).
- **A20/M8 attribution completeness**; spec v0.5 (PR #16, 2026-07-09).
