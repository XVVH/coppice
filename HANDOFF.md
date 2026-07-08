# HANDOFF — paste this to open the first Claude Code session

We're starting implementation of the Agent State Fabric. Read CLAUDE.md, then
docs/asf-schema-spec.md in full, then skim docs/agent-state-fabric-brief.md
sections 4–6 and 11. Design is frozen at brief v0.2 / spec v0.3; we are past
paper — do not re-litigate settled decisions (F1 broker-minted, F4 summary
minimization, fail-open posture), but DO flag anywhere the spec is ambiguous
or contradicts itself once real code forces the question. Track those as
spec-issues, don't silently pick an interpretation.

First milestone — the Stage 1 kernel, thinnest vertical slice:

1. Propose the repo layout and language choice (criteria: single static daemon
   binary preferred, strong sqlite + ed25519 + JCS/canonical-JSON story, low
   friction for an MCP proxy).
2. Implement, in order: (a) canonical serialization + object ids + signing;
   (b) the trace substrate (append-only sqlite, per-span hash chain, payload
   store with per-payload DEKs); (c) the snapshot coordinator over a test
   vault directory + a sqlite file, with atomic multi-root manifest capture
   and coherent revert; (d) manifest creation at step boundaries.
3. Prove it with one scripted end-to-end: create manifest -> mutate vault +
   sqlite -> trace events -> revert -> verify both stores restored and the
   ledger explains everything, including one injected out-of-band edit
   surfacing as attributed drift (single-human default).

Broker/capability evaluation is milestone 2 — do not start it until the
kernel round-trips. Write tests against the spec's invariants (M1–M6, C1–C5,
attenuation subset rules) as you go; the spec text is the source of truth
for expected behavior.
