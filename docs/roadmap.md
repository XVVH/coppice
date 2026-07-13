# Roadmap — in-flight and queued work

> **Boundary rule:** this file orders *work* — features, refactors,
> milestones, demos. It never restates findings: spec ambiguities live in
> `spec-issues.md` (SI-n), implementation defects in `review-findings.md`
> (RF-n), test gaps in `testing-theory.md` (G-n), dogfooding cases in
> `dogfooding-rubric.md` (DF-\*), posture shortcuts in
> `posture-assumptions.md` (P-n), measured baselines in
> `docs/baselines/`. A work item that spawns one of those links it; the
> detailed truth lives there.
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

## Queued (ordered)

**W-14 — typed object application + verified restore preparation.** One
expected-prefix/kind/signature boundary for every authority-bearing object;
update branch/revert/parent consumers; verify and stage every CAS dependency
before live mutation. Negative contracts leave all protected stores unchanged.
Closes RF-19/RF-20. *Provenance: 2026-07-12 cryptographic mechanism audit.*

**W-20 — kernel security protocol pass (spec v0.8).** Owner: operator +
agent. Write the operational stratum under the schema spec as one
ratified batch instead of five just-in-time designs. The spec's
invariants quantify over objects no ratified protocol yet constructs
("verified substrate prefix", "durable authorization offset", "atomic"
multi-root restore), so independent reviews re-derive the missing
semantics per PR — the PR #43 cycle designed three protocols inside one
remediation (RF-29…RF-32). Ordered contents: (1) ratify SI-25 — the
keystone every other protocol consumes; candidate is ADR 0006, recovered
from `agent/si25-authenticated-head-design` (W-15's design half);
(2) retro-ratify W-14's in-PR protocols — SI-31 owned-state
transition/recovery journal, SI-33 event-derived consumable authority,
SI-34 approval candidate binding — with PR #43's merged implementation
as candidate; (3) SI-32 store publication / filesystem attacker model;
(4) SI-26 with SI-28/SI-29 (type/domain transcript; payload envelope
AAD; post-shred generations — siblings, and SI-26 gates W-6); (5) SI-27
key lifecycle; (6) cross-cutting: a §0 posture-qualifier convention
binding each operational claim to the P-ledger vocabulary it holds
under, and the typed authority projection (G11/W-6 pulled forward for
authority consumers). Closes with one independent-context composition
review over the seams (journal freshness ↔ SI-25; AAD ↔ SI-26; rotation
↔ historical verification) — the batch exists so the protocols compose,
not merely each hold. Deliverables: spec v0.8 plus a companion kernel
security profile for syscall-level mechanism; SI-25…SI-34 resolved or
explicitly deferred with named triggers; ledgers updated. Gate:
W-15…W-18 implementation and any new authority-surface W item wait for
their protocol's ratification; W-14 merges first behind its
oracle-backed independent re-review (SI-31/SI-33/SI-34 are that
oracle); W-1 dogfooding continues unaffected. *Provenance: the
2026-07-13 spec-cohesion analysis of the PR #43 review cycle; sequences
the existing ratify-first clauses of W-15/W-16/W-17 as one campaign.*

**W-15 — authenticated global trace head (design then implementation).**
Ratify SI-25, then bind global order, completeness, home/epoch, export order,
and rollback freshness with an explicit recovery story. Closes RF-13/P15's
production and standing-authority gate. W-11 has landed; W-3 may collect
disposable examples but may not compile standing authority until W-15 lands.
*Provenance: RF-13 + 2026-07-12 cryptographic mechanism audit.*

**W-16 — payload envelope v2 and shred protocol.** Ratify SI-28/SI-29, then
bind canonical AAD and algorithm/version/key metadata, enforce the KEK wrap
lifecycle, decrypt against a complete signed `PayloadRef`, and make put/shred
crash-safe before tackling the separately parked forensic-media guarantee.
Closes RF-22/P19; composes with P13. *Provenance: 2026-07-12 cryptographic
mechanism audit.*

**W-17 — production key custody and lifecycle.** After SI-27, move user-root,
fabric, KEK, and credential custody behind the chosen independent OS/hardware
or remote boundary; implement trust anchors, rotation, recovery, and historical
verification. Closes RF-14's production boundary. No real credential ships
before this and W-18. *Provenance: RF-14 + 2026-07-12 cryptographic mechanism
audit.*

**W-18 — authenticated identity, approval, and credential containment.**
Resolve SI-23, holder/channel proof and user presence (RF-24), and the trusted
credential-adapter/response boundary (RF-23/P27). Composes with W-4 rather than
replacing containment. No actuation, third-party credential adapter, or live
credential ships first. *Provenance: SI-23 + 2026-07-12 cryptographic mechanism
audit.*

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
conformance suite"). Includes G11's per-kind signed-event body schemas and
negative fixtures. Includes: **LICENSE file** (Cargo.toml declares
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
  (The *internal* durability side of the same primitive is parked as the
  durable external-effect protocol below — the receipt is one stage of it.)
- **Provenance-aware store interface** — publish the interface that turns
  A3 (memory taint) from open problem into an ecosystem standard; sidecar
  lineage index as reference implementation.
- **Insurable delegation** — TrustRecord as actuarial object; manifest
  trail as underwriting evidence.
- **North-star metrics** — consequence-weighted verified work per minute
  of human attention; approval compression ratio; time-to-first-ratified-
  rule; % sessions fully silent. Adopt alongside the denial-FP rate when
  W-3 gives them substance.
- **Machine-checkable conformance annotations** — mechanize G9's sweep:
  normative spec sentences carry stable ids, `tests/contracts.tsv` rows
  reference them, `scripts/ci` fails on unreferenced normative ids in
  sections a change touches. Natural W-6 companion (the published
  conformance surface needs the same sentence ids for its vectors). From
  the PR #33 review cycle (2026-07-12).

## Parked (trigger-gated — do not start without the trigger)

- **Actuation registration remains trigger-blocked by SI-23/W-18.** W-18 is
  now queued to ratify and build the identity/approval/credential boundary;
  this parked gate still means no computer-use, shell, or UI-control tool may
  register before that work completes. W-4 remains its containment complement.
- **Checkpoint session boundary** — ADR 0004 tripwire (ops-per-promotion /
  conflict incidence).
- **R2 read-authority family** — before the first `external_reach: live`
  tool (ADR 0005 binds the shape; dogfooding graduation criterion).
- **Crash-atomic multi-root commit** — production/live-egress release gate;
  authenticated trace order/head moved to queued W-15 after SI-25 separated
  its cryptographic protocol from the state-commit protocol.
- **Broker-outage loud fail-open + `on_broker_outage`** — live-egress
  integration.
- **Durable external-effect protocol** — trigger: the first tool that
  produces a real *external* effect (a remote side effect can land before
  its result is recorded — Tier-1-local never has this problem). One
  coherent staged protocol, currently scattered across three trackers,
  consolidated here so it is reassembled as a unit rather than
  rediscovered piecemeal at first egress:
  `effect_intent → authority reservation → dispatch (idempotency key) →
  effect receipt → state commit → completion receipt`. Its pieces already
  live in: crash-atomic multi-root commit (parked, above), broker-outage
  posture (parked, above), effect receipts (candidate), and durable
  call-identity/idempotency (scalability analysis, "broker availability
  and in-flight effects"). Surfaced by the 2026-07-10 design review as a
  distinct synthesis; filed 2026-07-11 so first-egress work starts from
  one design, not four references. Composes with the R2 read-authority
  gate (both fire at first live-egress tool).
- **Forensic crypto-shredding** — post-dogfooding release gate.
- **Fabric-home at-rest protection / deployment floor (RF-15/P17)** — trigger:
  production, offline backups, privileged-host compromise in scope, or tenants
  sharing one uid/home. W-17 closes key custody, not plaintext CAS/branch/DB
  storage; choose an OS/full-disk deployment floor or application envelope
  before assigning an implementation item.
- **Verified recovery preselection (RF-27)** — trigger: the next RF-9 recovery
  hardening pass. Derive the promotion-event half from `VerifiedEvent`, retain
  the broker-owned promotions-table half, and prove anomalous unsigned selectors
  cannot make `asf recover` skip stranded work.
- **Multi-actor roots/visibility, HA topology, model judge/clerk-as-model,
  Tier-2/3 stores** — deferred until the W-1..W-3 loop produces pull
  (2026-07-10 review: both external reviews independently concurred).
- **SessionContext refactor** (replace home-global `current_manifest`/
  `current_span`) — trigger: second concurrent session in one home
  (scalability analysis), or opportunistically with W-4's proxy work.

## Done (recent — full history is git)

- **W-19 — integrity-aware operator ledger** — diagnostic accounting now
  consumes only a clean signature-verified, selector-matched, chain-valid
  retained-row view; malformed, foreign-signed, selector-mismatched,
  signed-sequence-reordered, and chain-anomalous rows fail loudly without
  drift/CAS/DB effects while decodable raw text remains inspectable. Closes
  RF-28 without claiming SI-25's cross-span order, completeness, rollback, or
  freshness. (PR #41, 2026-07-13)
- **W-13 — key continuity and explicit initialization** — separates new-home
  initialization from existing-home reopen; required fabric, user-root, and
  owner-KEK material is published as one fsynced directory entry and never
  regenerated or repaired on reopen. Partial/malformed homes, path
  substitutions, missing CAS/runtime state, and malformed secret storage fail
  closed. Independent review requested three corrections and approved the
  final implementation. Closes RF-18's immediate mechanism; SI-27/W-17 and
  RF-14 remain open. (PR #40, 2026-07-13)
- **W-12 — strict JCS input domain** — recursively enforces the integer-only
  `|n| < 2^53` domain at seal and verify, rejects duplicate names at every raw
  persisted fabric-object ingress, preserves valid signed bytes, and carries
  language-neutral boundary/collision vectors plus a protected dispatch
  negative and 36/36 targeted mutation result. Closes RF-17/P24; RF-6/SI-26
  remains separate. (PR #39, 2026-07-12)
- **W-10 — patched SQLite floor** — upgraded the bundled engine from affected
  SQLite 3.46.0 to 3.53.2, enforced a conservative 3.51.3 runtime floor before
  fabric-state creation, and added a two-sided `SQLITE-ENGINE` contract plus a
  stable targeted mutation lane (4/4 caught). Closes RF-21. (PR #37,
  2026-07-12)
- **W-11 — verified-event authority boundary** — authority consumers verify
  signed raw before filtering, require exact agreement for all seven
  signed/materialized selectors, take decision events/head from one statement
  view, use signed `seq` for within-span replay and branch-tip selection, bind
  grant parent exactly, and enforce registration placement. The first
  independent-context review returned REQUEST CHANGES; all five findings were
  corrected and re-review approved with non-blocking follow-ups. Closes
  RF-16/RF-25/RF-26; SI-25 and RF-27 remain explicit. (PR #36, 2026-07-12)
- **W-8 — capability closure + revocation (A22/§5.4)** — the signed
  `revoke` edge as the permanent, prospective, descendant-closing dual of
  A21's `grant`: pure event-derived liveness (`a22_*` predicates — the
  spec's `capability_state_at`) shared verbatim by decision time and gate
  replay at each effect's durable authorization offset; revokes resolved
  by capability id across manifests (M2 sub-agent chains); closure denials
  structural and non-escalatable; approvals/exemptions inert after closure
  while pre-revoke parked promotions stay approvable; mint/attenuate
  refuse closed parents and closed ids; operator kill switch `asf revoke`
  on the C2 socket + offline path, never advertised over MCP;
  `EVENT_KINDS` gains `revoke`, drops `expiry`. Evidence: A22 two-sided
  contract (19 tests: broker decision-time, gate matrix incl. both
  dispatch-vs-revoke orders + cross-manifest cascade + doubt-never-widens
  both edges, socket smoke), targeted `a22_*` mutation lane (36/36 caught;
  first run surfaced and killed a real observed-mode activation gap), W-9
  verdict baseline reproduced bit-for-bit under its pins
  (`885d7835…`, identical 84-row quarantine), and the gate-replay
  corpus-throughput measurement W-9 deferred (numbers in
  `docs/baselines/w9-2026-07-12/`). Wedge consequence kept honest: an
  in-flight branch at revoke conservatively strands (revert remains); the
  parked durable external-effect protocol supplies the signed dispatch
  point before any remote effect ships; RF-13/P21 remain the
  production/distributed gates against revoke-erasing rollback. Closes
  posture gap P25; composes with but does not subsume
  P10/P15/P20/P21/P22 or SI-23's P3/P4/P5/P12 cluster. Unblocks W-3's
  progression to standing authority. (PR #33, 2026-07-12)
- **W-9 — foreign-trace corpus baseline** — `asf corpus` harness
  (census/replay/ingest; fail-closed quarantine; CORPUS-FAIL-CLOSED
  contracts) and the pinned baseline at `docs/baselines/w9-2026-07-12/`
  — findings, numbers, and the W-8 regression contract live THERE, per
  the boundary rule. v1 PR #29; v2 corrections + hardening from the
  2026-07-12 fresh-eyes review PR #30 (which also filed P26/G-RATCHET).
  Deferred with named triggers: gate-replay throughput → W-8; containment
  measurement → R2; SWE op-class fixtures → W-8 gate work; ADP breadth on
  demand. (PR #29/#30, 2026-07-12)
- **W-2 — unified evaluator at the gate** — the gate replays every signed
  tool_call through the decision-time evaluator (all seven dimensions, not
  four; context from the registered action; meters/exemptions from signed
  events; clock = the event's `at` per SI-22, interpreted). Ships with the
  two-sided contract registry, the four m7 grant-activation adversarial
  cases, and the targeted A21/M7 mutation lane; the mutation-lane stretch
  goal was subsumed — gate dimension logic IS `evaluate.rs`, which the
  scheduled lane already covers (PR #20, 2026-07-10).
- **A21/SI-21** — authority binds by declaration + grant event; spec v0.6;
  adversarial gate coverage; RF-13 filed (PR #18, 2026-07-10).
- **Invariant-driven verification hardening** from the security audit
  (PR #17, 2026-07-10).
- **A20/M8 attribution completeness**; spec v0.5 (PR #16, 2026-07-09).
