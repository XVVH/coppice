# Agent State Fabric (ASF)

Neutral, agent-agnostic state fabric: delegation manifests binding four lineages
(state, authority, behavior, trace), snapshot-backed undo for owned state, brokered
capabilities for external effects, human-ratified compilation loops (skills up,
caveats down). Read `docs/agent-state-fabric-brief.md` (why/what, v0.2) and
`docs/asf-schema-spec.md` (the constitution, v0.6) before writing any code.
The spec wins over this file wherever they disagree. Spec ambiguities found
while implementing go to `docs/spec-issues.md` (never silently interpret);
SI-1…SI-21 are resolved (SI-20 → A20/M8 in v0.5; SI-21 → A21/M7 in v0.6), new issues start at SI-22.

## Current build target — Stage 1 (Kernel) + Stage 2 (Spine)

1. **Broker daemon**: MCP proxy in front of a Hermes agent; capability evaluation
   (conjunctive caveats, fail-closed on unknown dimensions); credential injection
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
of the spec or the non-negotiable fail posture below.

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

## Context

Design rationale lives in the originating design session (every schema
amendment A1–A14 traces to a specific exchange; docs carry the changelog).
Wedge: Hermes agent client; workflows = web research→vault, Discord message
management, vault maintenance; channels = terminal (local_session) + Telegram
(platform_oauth, sender-bound per C5).
