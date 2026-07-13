# Testing theory — Stage 3 dogfooding baseline

This records what the suite is, what it deliberately is not yet, and which
kind of automation owns each claim. Updated 2026-07-10 with the full named
suite plus 576 shrinkable generated cases in the default run and deeper
scheduled automation. The promotion gate is complete; dogfooding is now the
product-signal lane, not a substitute for correctness testing.

## What the suite is now

**1. Spec-invariant mapping is the organizing discipline.** Test files open
with a coverage map naming which spec invariants they prove (see
`crates/asf-kernel/tests/e2e.rs` and `tests/broker.rs` headers). The spec
text is the source of truth for expected behavior; when a test encodes an
*interpretation* of ambiguous spec text, it cites the SI number. A test
that can't say which invariant it defends is suspect.

**2. Three altitudes, each catching what the one below can't.**
- *Unit* — pure logic in-module: JCS shapes, glob semantics, attenuation
  subset rules, evaluator outcomes, conservative registration defaults.
- *Integration* — real sqlite + real filesystem through the public API:
  the kernel e2e (manifest → mutate → drift → revert → ledger), the broker
  suite (mint → call → escalate → approve → attenuate).
- *Process-level* — `tests/proxy_smoke.rs` spawns the actual `asf proxy`
  and `asf vault-server` binaries and drives them over the real MCP wire,
  approvals over the real Unix socket, then inspects the ledger on disk.
  This is the only altitude that can catch wiring bugs (id routing,
  stdio framing, socket lifecycle) and the only one that proves C2
  topologically (an invented in-band approval method falls through and
  errors).

**3. The adversarial posture is load-bearing, not decorative.** The suite
attacks its own guarantees: dropping the append-only triggers and editing
history (chain verification must catch it), deleting a middle event,
re-sealing tampered objects under the wrong key, editing a stored
capability to widen its budget (F1 signature check must kill it), seven
distinct attenuation-widening attempts including the subtle one (omitting
a parent dimension), and offering an approval exemption to an unknown
caveat dimension (must stay failed). The rule of thumb: every "X is
impossible" claim in the spec gets a test that tries to do X.

**4. Demos are executable acceptance tests.** `asf demo` and
`asf broker-demo` narrate the milestone stories and `bail!` on any
deviation — they are the human-legible face of the same assertions. The
canonical `scripts/ci test` lane runs both after the Rust test targets.

**5. Negative space is tracked explicitly.** Invariants with no conformant
test because their machinery or ratified representation does not exist yet:
C3 (signed fabric→user messages), C4 (delivery ceilings), C5 (sender binding),
M3 (Tier-3 `as_of` surfacing), M4 runtime behavior attestation, the taint
dimensions, and StandingRule schema enforcement (k≥3, counterfactuals,
domain match). These are absences by
sequencing, not oversight; each activates with its milestone. (§5.4
capability closure left this list with W-8: the A22 contract below.) (M7's
brokered-authority edge left this list with A21/SI-21:
mode declaration + grant-event binding, tested in the gate suite.) An untracked
untested invariant is how "the spec is the source of truth" quietly stops
being true.

**6. Every new test joins a two-sided contract.** `tests/contracts.tsv` is the
machine-readable evidence index: each invariant contract has positive evidence
(a valid case succeeds) and negative evidence (an invalid case fails without
producing the protected effect). `scripts/ci contracts` compares that registry
with Rust's compiled `--list` inventory. Tests that predate the gate are named
individually in `tests/contracts-baseline.txt`; the baseline is frozen, so a new
or renamed test cannot pass the required gate until it joins a contract. This
turns the invariant matrix from review custom into an executable admission
rule while leaving semantic judgment — whether a negative attacks the right
failure — with review and mutation testing.

## Current invariant matrix

| Invariant | Automated evidence | Status |
| --- | --- | --- |
| M1 roots cover capability reach | `m1_mint_rejects_uncovered_store`, `attenuation_rechecks_m1_for_the_child_manifest` | covered |
| M2 capability binds manifest | `m2_capability_dies_with_its_manifest`, broker/gate suites | covered |
| M3 Tier-3 `as_of` in consent | none; Tier-3 absent | future milestone |
| M4 loaded behavior attestation | behavior lineage is recorded, but first-call runtime attestation is absent | open implementation gap |
| M5 re-manifest on behavior change | `m5_behavior_change_forces_remanifest` | covered |
| M6 cheap/frequent manifests | exercised throughout multi-boundary tests; guidance rather than a binary predicate | exercised |
| M7 authority mode + grant binding (A21) | `m7_brokered_manifest_rejects_unattributed_calls`, `m7_brokered_end_to_end_gates_clean` (grant-before-effect ordering), `m7_observed_manifest_tolerates_unattributed_calls`; the proxy suite now runs declared-brokered end-to-end | covered |
| M8 attribution completeness | e2e, gate, proxy timing tests, generated state-machine histories | covered for current consumers |
| §5.4/A22 capability closure | the A22 contract (26 tests): decision-time closure in tests/broker.rs, gate liveness-at-offset matrix in tests/gate.rs, exact grant-parent binding, socket kill switch + CLI exit-status contract in proxy_smoke; targeted `a22_*`, verified-event constructor, and W-11 consumer mutation lanes | corrected by W-11 after independent review; SI-25 cross-span order/head residual |
| C1 authority provenance | intent and escalation/approval integration tests | covered for current channels |
| C2 agent outside approval path | real proxy/socket topology plus invented in-band method rejection | covered |
| C3–C5 | channel/delivery machinery absent | future milestone |

Every new invariant or new consumer of live state must add a row or extend an
existing row in the same change that implements it.

## Known gaps — the deep-dive agenda

**G1. Property and model testing.** The core properties live in
`tests/properties.rs`: merge identities/conflict soundness over seeded random
tree triples, exhaustive bounded glob-cover soundness, the original semantic
subset generator, chronology comparison, and a shrinkable full-vocabulary
attenuation property covering every Stage-2 caveat dimension. The real-store
state machine in `tests/model.rs` generates interleaved human/agent histories
through broker calls, trace attestations, promotion, conflict approval, M8,
and ledger explanation. Remaining extensions:
- *Semantic subset property (the big one):* `verify_attenuation(parent,
  child) == Ok` must imply that for every call context, child-allows →
  parent-allows. Randomly generate caveat sets and call contexts; any
  counterexample is a privilege escalation. This tests the *meaning* of
  attenuation, not its implementation.
- *Algebraic properties:* transitivity (a⊆b ∧ b⊆c → a⊆c), reflexivity of
  attenuation-identity, monotonicity of adding dimensions.
- *Glob soundness:* `glob_covers(p, c)` must imply ∀path:
  `glob_matches(c, path) → glob_matches(p, path)` — fuzz paths against
  glob pairs. (Completeness is deliberately absent — SI-12's conservative
  rule — but soundness must be total.)
- attenuation transitivity and malformed/duplicate-dimension generation;
- generated delete/move/rename histories and multi-store sqlite histories;
- controlled crash and process-concurrency transitions (G3/G4).

**G2. Differential canonicalization.** Cross-implementation JCS vectors —
hash the same objects with a second RFC 8785 implementation (any language)
and compare. Becomes existential the moment a second implementation of the
spec exists, and it is the enforcement mechanism for the §0 number rule
proposed in ADR 0002. Deliverable shape: a language-neutral fixture file
(object JSON → expected id) that lives with the spec, not with this repo's
tests. RF-17 makes the input-domain half immediate: carry exact positives at
`±(2^53-1)`, negatives at `±2^53`, the reproduced adjacent-u64 collision,
floats at every nesting depth, and raw duplicate-property inputs. Rejection is
part of the vector result; canonical bytes alone are not conformance.

W-12 supplies the input-domain half in
`tests/vectors/jcs-input-domain.json`: exact accepts at `±(2^53-1)` and
integer-token `-0` with canonical bytes and SHA-256, exact rejects at `±2^53`, both members of the
reproduced adjacent-u64 collision, floats at the root and nested through
objects/arrays (including integer-shaped decimal/exponent forms), and
top-level/nested duplicate names. The Rust reference test
consumes the fixture without embedding Rust types in it. A second independent
RFC 8785 implementation remains W-6's publication work; W-12 does not claim
that differential half.

**W-12 G9 inverse conformance sweep (spec §0 Serialization + Extensibility).**

| Normative edge | Enforcement | Inverse / two-sided evidence |
|---|---|---|
| Fabric serialization is JCS | `canon::jcs_bytes`, `body_bytes` | `jcs_rfc8785_shapes`; language-neutral accepted vectors |
| `id` hashes the canonical body excluding both `id` and `sig` | `canon::hash_body`, `compute_id` | `id_excludes_id_and_sig_and_is_stable`; `verify_roundtrip_and_tamper_detection` |
| Ed25519 signs the same body bytes used by the id | `canon::seal`, `verify` via `body_bytes` | valid-boundary output equals the pre-W-12 signing path byte-for-byte; tamper/recomputed-id negatives remain rejected |
| Numbers are integers with `|n| < 2^53`; floats are forbidden recursively | `w12_raw_number_is_in_domain` preserves lexical integer `-0` while rejecting decimal/exponent forms; `w12_number_is_in_domain`, `w12_validate_value`; unavoidable through raw parse, `jcs_bytes`, `seal`, `compute_id`, and `verify` | `JCS-DOMAIN` positive/negative/supporting tests; G2 exact bounds, `-0`, collision, root/nested/integer-shaped floats; targeted W-12 mutation lane |
| Raw JCS input cannot contain duplicate object names | `canon::parse_fabric_json` + `w12_duplicate_key`, before `Value` construction | duplicate vectors at root/nesting; protected tool-dispatch negative for both a stored signed object and event |
| Unknown fields are preserved and hashed | the strict visitor recursively builds every array/object member; `hash_body` removes only `id`/`sig` | `unknown_fields_are_hashed`; `unknown_fields_survive_reserialize` |
| Cross-references remain by id and lineage remains tamper-evident | unchanged: `trace::get_object` is full-id lookup; `verify_verified_chain` checks signed per-span lineage | existing id/unknown-field/tamper and `TRACE-CHAIN` contracts; W-12 changes no reference or lineage semantics |

The targeted stable-predicate lane caught all 36 generated mutants (36/36;
zero missed, unviable, timed out, or excluded) over exact numeric comparisons,
lexical-number scanning/classification, mandatory raw-parser invocation,
recursive value traversal, and duplicate-key detection. This certifies those
written predicates; the protected dispatch test supplies the independent
consumer-boundary evidence.

Every raw persisted/imported **fabric-object** ingress is routed before
`Value` construction: `trace::row_to_event` covers `events_in_span`,
`all_events`, and `event_at_offset`; `trace::get_object` covers the common
stored-object path; `Broker::capability_descendants` and
`Broker::check_min_auth_for_manifest` cover the only direct `objects.raw`
queries that bypass `get_object`. There is currently no separate fabric-object
import API. Promotion previews, escalation samples, store-topology metadata,
snapshot tree nodes, MCP messages, and corpus records are not signed fabric
objects and therefore are not silently subjected to the §0 fabric domain.

Explicit scope fence: RF-6/SI-26's signed type/domain transcript is unchanged.
W-12 preserves the current valid transcript bytes and does not treat input
validation as type binding.

**G3. Crash consistency.** Preparation failure, injected interruption, hard
process exit during an in-place filesystem apply, reopen, and idempotent replay
are covered; session-boundary SIGKILL recovery is covered too. W-14 adds the
multi-root protocol and deterministic barriers after journal publication,
after a later-store mutation, before database commit, and after database
commit. Ordinary later-store and event-append failures prove no partial root or
unrecorded mutation survives; reopen proves exact rollback without the linked
event and exact roll-forward with it. Remaining same family: a subprocess
hard-exit matrix at each filesystem syscall and WAL/-shm sidecars under a
crashed reader. W-13 covers complete
key-directory publication, concurrent initialization, required-key
loss/malformed reopen, database-only, keys-only, mixed partial homes,
missing/substituted database, CAS, and runtime-store paths, and malformed
credential storage; every negative snapshots protected material and proves no
replacement identity, repair, external-target write, or overwrite occurred. Still pending from
RF-20/RF-22: hard-process exit at each key-publication syscall, every boundary
of payload put/shred, signed shred-event sequencing, and corrupt/missing CAS
objects during prepare.

**G4. Concurrency.** Generated model histories cover logical interleavings. A
programmable downstream barrier now forces both in-flight timing directions:
queued EOF records the call before promotion, while SIGTERM after mutation but
before result recording rejects the untraced branch and retains its recovery
marker. Approval and retry start simultaneously and preserve exactly one
bounded use. A self-spawned helper process proves the fabric-home `flock`
excludes a second gate process. Still needed: out-of-order MCP responses,
simultaneous supported proxies sharing one home, and barriers inside the future
multi-root commit protocol.

**G5. Coverage honesty.** Scheduled mutation testing covers authority
evaluation and promotion policy. The first evaluator sweep found two surviving
mutations (external-reach direction and empty write-path extraction); explicit
semantic tests were added, and the repeat killed all 32 viable mutants (one was
compiler-rejected). The exact scheduled evaluator+merge lane catches 58
mutants, with three compiler-rejected and zero missed/timeouts. Capability glob
mutations remain outside the blocking mutation lane: exhaustive/property tests
cover their semantics, while several deliberately broken matchers do not
terminate and make mutation-run exit status noisy. Since W-2, the gate's
dimension logic IS `evaluate.rs` (the gate replays the decision-time
evaluator over signed records), so the scheduled evaluator lane's mutants
now guard gate-time semantics too — a single hand-rolled recheck drifting
out of sync is no longer a representable bug.

W-13 adds a stable targeted lane over the `w13_*` boundaries in proxy, kernel,
keystore, and CAS: explicit new/existing-home classification, identity-before-
runtime ordering, non-repair of missing runtime/CAS state, path-shape and
symlink rejection, exact required-material load, secret-file classification,
and malformed credential-store rejection. Its contract negatives assert
absence of replacement/repair files and byte-for-byte preservation of external
targets, so a killed mutant means the protected effect stayed absent—not merely
that an error changed. The final targeted run caught all 35 viable mutants;
five whole-function replacements were compiler-unviable and none survived.
Iterative runs exposed missing exact boundaries, fail-closed-but-late proxy
effects, path substitution, and a shared predicate whose failures masked one
another; the final lane separates and kills each meaningful predicate.
directory-entry enumeration now treats dangling symlinks as malformed rather
than absent.

A21/M7 has its own stable targeted mutation surface:
`m7_grant_offsets`, `m7_effect_capability`, and `m7_verify_grant`. The mutation
lane names those functions rather than source lines, so refactoring cannot
silently move the authority-binding predicates outside the scheduled check.
It catches all 20 viable mutations. Two exclusions, both documented in
`scripts/ci`: the `<` to `<=` mutation is equivalent because substrate
offsets are globally unique (a grant and its effect cannot occupy the same
offset), and — since W-8 — the ordering-guard→true mutation is masked by
the a22 leaf-activation check running on the same gate path: under honest
emission the two layers are equivalent (grants land only on the substrate
span; both views take earliest), the redundancy is deliberate defense in
depth, and the ordering property is killed independently in the a22 lane.

A22/§5.4 closure has the parallel lane over the `a22_*` predicates
(`a22_revoke_offsets|grant_bindings|ancestry|state_at`). Its first run
did its job: 35/38 caught, with the three misses triaged as two
equivalent `<`→`<=` mutants in `a22_state_at` (the m7 argument again —
offsets are globally unique, and decision time evaluates at head+1,
which no existing event occupies; excluded with `-E` and documented in
`scripts/ci`) and one REAL gap — `&&`→`||` in `a22_grant_bindings`
survived because brokered-mode tests mask the a22 activation path behind
the m7 check. The killer
(`a22_non_grant_event_cannot_activate_capability_in_observed_mode`)
pins the observed-mode path where a22 is the only activation check. The
pre-merge review then added the span-placement clause to
`a22_grant_bindings` (grants activate only from the fabric-lifetime
span), whose mutants
`a22_grant_on_session_span_activates_nothing` kills on the same
observed-mode surface; the current lane catches all 38 viable mutants.
RF-16 found a boundary outside those predicates: their inputs are
denormalized `EventRow` fields selected before signed-raw agreement is proved.
W-11 extends the stable mutation/test surface through the verified-event
constructor so predicate coverage cannot certify an unverified caller again.

**G6. Soak / growth.** Scheduled CI runs release mode with deeper generated
case counts. A true thousands-of-events/files run remains: ledger size, WAL
behavior, meter growth, verification latency, and ADR 0002 `asf stats`
baselines.

**G7. Process-harness fidelity.** Proxy tests use bounded pipe/socket reads,
bounded child exit, captured stderr, and child status in failures. They can now
substitute a barrier-controlled downstream instead of relying only on the
in-tree vault server. The remaining adversarial MCP corpus is malformed large
frames, duplicate/out-of-order ids, unsolicited notifications, and downstream
death at each protocol phase. RF-27 adds a recovery-specific case: unsigned
promotion selectors must not make `asf recover` skip a stranded manifest.

**G8. Revocation lifecycle (SI-24 → A22, spec §5.4) — RF-16 remediation
implemented in W-11; independent re-review approved.** The
event-derived closure view is implemented as the `a22_*` predicate family
in broker.rs (`a22_revoke_offsets`/`a22_grant_bindings`/`a22_ancestry`/
`a22_state_at` — the spec's `capability_state_at`, decomposed): a
structural precondition in front of caveat evaluation, never a caveat
dimension, shared verbatim by decision time (at the current head) and gate
replay (at each effect's own offset). The A22 two-sided contract (26
tests) covers the full matrix below; the scheduled `a22_*` mutation lane
guards the predicates. Matrix, all landed:
call before revoke succeeds; direct and ancestor revoke deny later calls —
including across manifest boundaries (an M2 sub-agent child dies with its
ancestor's revoke; revokes resolve by capability id, never filtered by the
evaluating manifest); child-only revoke preserves parent/siblings; revoke is
non-retroactive and parked promotions of pre-revoke work remain approvable;
wholly unsigned rows move nothing. RF-16 disproved the broader claim that
the view was fully event-derived: signed rows could be concealed by changing
their unsigned `kind`/`span` indexes before the view. The W-11
`VerifiedEvent` boundary now verifies raw and proves agreement for all seven
materialized selectors before filtering; the combined concealment negative and
per-field protected-effect matrix are green. The first independent review then
found four missing enforcement edges: unsigned-offset branch-tip selection,
split events/head decision reads, grant-parent mismatch, and registration
placement. W-11 uses signed `seq` for every within-span consumer, derives the
event set plus head from one SQLite statement snapshot, binds grant parent
exactly, and requires registration on the fabric-lifetime span
with `manifest:null`. Mutation results: constructor 7 caught + 1 unviable;
W-11 consumers 15 caught + 2 unviable; A22 predicates 29/29 caught, zero
survivors. SI-25 still owns unsigned cross-span `offset`,
expected-head/completeness, deletion/signature invalidation, rollback, and
freshness. A revoke naming
an id no capability bears affects no other capability, while a
verified-but-anomalous revoke (wrong `manifest` field, unexpected span,
unpaired C1 provenance) still closes its target, loudly — each anomaly
kind surfaced in `trace_check.closure_anomalies` — §5.4's
doubt-never-widens, two-sided; the activation edge keeps the opposite
posture: grants activate ONLY from the fabric-lifetime span, so a signed
grant parked on a session span activates nothing at decision time or (in
observed mode, where a22 is the sole activation check) at the gate;
same-id re-grant (including revoked-before-first-grant) and post-revoke
attenuation cannot reactivate, and the broker refuses to grant a closed id;
ancestor earliest-grants must be well-ordered;
approvals/exemptions cannot resurrect and closure denials are
non-escalatable; a `revoke` event is accepted and an `expiry` event is no
longer emittable (the kind left §6 with A22); `asf revoke` exits non-zero
when the revocation did not take effect (the kill switch's scripting
contract) and zero on idempotent re-kills;
and dispatch vs revoke has one substrate order whose global authentication is
still SI-25/RF-13. The external-effect form
waits for the durable dispatch protocol rather than testing an in-memory
ticket as if it were a receipt. The W-9 corpus verdict-invariance
regression belongs to this matrix too: replaying the pinned corpus
baseline (`docs/baselines/w9-2026-07-12/`) under its recorded pins must
reproduce its verdict-vector hash — revocation-free corpora may not
change a single verdict — with the fixture-scale twin enforced in CI as
a pinned constant (`crates/asf-cli/tests/corpus.rs`).

**G9. Cross-layer conformance (founded by the PR #33 review).** Three
pre-merge findings on W-8 shared one root class: a normative spec sentence
with no enforcing line anywhere (§6 records grants on the fabric-lifetime
span; the a22 view accepted any span), a documented promise with
half-implemented observability (anomaly loudness named two kinds, the scan
surfaced one), and an operator contract below the spec's ontology
(`asf revoke` exited 0 on failure). Every existing lane verifies what is
WRITTEN — contracts attack enumerated invariants, mutation testing mutates
only code that exists, baselines pin recorded behavior — so none can flag
an unwritten condition or an untested layer, and the author's own review
replays the interpretive move that created the gap instead of challenging
it. Standing counters, mandatory for spec-implementing changes (the MUST
form lives in AGENTS.md):
- *Conformance sweep, inverse coverage:* test-file coverage maps point
  test → invariant; before merge, sweep the other direction — every
  normative sentence in the spec sections the change touches gets a named
  enforcing line plus a negative test in the PR, or an explicit SI/G/P
  filing for why it is not yet enforceable. Silence is the failure mode.
- *Asymmetric principles get the full matrix:* when a rule is
  deliberately two-sided (doubt-never-widens: closure tolerates placement
  anomalies, activation demands exact form), enumerate edge × anomaly
  dimension (kind, span, manifest, provenance, …) and demand a
  cell-by-cell verdict and test. A property enforced implicitly by one
  code path's shape does not transfer to a parallel path — state it as a
  property or lose it (the m7 span filter did not transfer to a22).
- *Operator binaries are contract surfaces:* every operator-facing
  command carries at least one test asserting its process-level contract
  — exit status, not just the reply body (G7's fidelity discipline,
  extended above the wire). Founding example:
  `revoke_cli_exit_status_reflects_outcome`; `asf approve` inherited the
  same fix, having carried the same defect since Stage 2.
- *Independent-context review before merge* for authority-surface
  changes: same-context review — even a deliberate fresh-eyes pass —
  reliably catches internal inconsistencies and reliably misses
  cross-layer conformance. Three independent-review cycles each produced
  real findings (W-9 v1→v2 in PR #30, the #31 docs sweep, the #33 W-8
  review): k ≥ 3, the project's own founding-example bar, so this is now
  a rule rather than a habit.
- *Evidence claims scope to their lane:* "36/36 mutants caught" certifies
  the written predicates, not the design; a PR summary states what each
  green lane measures and claims nothing wider.

W-11's first independent-context application validated this discipline: the
review found unsigned-offset branch-tip selection, an events/head TOCTOU,
missing signed grant-parent and registration-placement predicates, and a
constructor-only negative misclassified as protected-effect evidence. All
ordinary and targeted lanes were green before that review. The corrections
therefore extend the conformance map, consumer-level contracts, and mutation
surface rather than treating the findings as isolated lines.

The PR #43 (W-14) cycle founded this discipline's intake counterpart:
review-finding triage. All six REQUEST-CHANGES findings were remediated
in-PR within hours; three were protocol-class — their remedies
introduced a new persistent record and commit point (the recovery
journal, RF-30), a new reconstruction of authoritative state
(event-derived meters/exemptions, RF-29), or a new binding for what a
consumer treats as authoritative (signed candidate digests, RF-31).
Those are unratified semantics with no written clause to be conformant
to — exactly the condition under which independent review cannot
terminate, since each reviewer re-derives the missing protocol and
finds different edges. RF-32 (tools/list advertisement liveness) is the
instructive boundary case: it trips the structural test — a consumer's
authoritative source changed — but its governing clause was already
ratified (§5.4 liveness/M7; advertisement was a consumer that ignored
it), so it is mechanism-class and remediates in-PR. The tiebreaker is
whether the clause exists, not how structural the fix looks. The MUST
form lives in AGENTS.md: protocol-class findings file an SI and ratify
before implementation (the three map to SI-31/SI-33/SI-34, and SI-32
files the publication attacker model RF-20's remediation implied; W-20
batches their ratification); mechanism-class findings — a missing or
wrong predicate inside already-ratified semantics — remediate in-PR as
before; blocking findings cite the written clause they enforce or they
are filings, not blockers.

**G10. Authenticated-storage adversary matrix (cryptographic audit,
2026-07-12).** Treat SQLite/CAS as attacker-controlled materialized storage
while the signing key remains unavailable. For every signed event, mutate each
denormalized column independently and in combinations (`id`, `span`, `seq`,
`prev`, `manifest`, `at`, `kind`, `offset`); drop a tail, a whole span, and the
latest authority event; restore an older database image; inject malformed raw
JSON. W-11 now rejects disagreement in the seven signed-materialized
selectors. Unsigned offset reordering within one span is neutralized by using
signed `seq`; cross-span `offset`, deletion/completeness, rollback, and
freshness remain explicitly classified under SI-25's authenticated global
head. RF-28 extends the same matrix to the operator ledger: diagnostic
accounting consumes only a clean verified view, while malformed JSON,
foreign/invalid signatures, signed/materialized disagreement, unknown signed
kinds, signed-sequence/offset inversions, and broken chains are individually surfaced rather than trusted or
silently omitted. The process-level negative snapshots the complete fabric
home and proves integrity failure already present in the observed view occurs
before drift attribution or CAS capture can mutate it; concurrent replacement
after that view remains SI-25 freshness. The targeted `w19_*` mutation lane
guards the new classification and no-accounting-on-doubt predicates. W-14's
first submitted matrix covered typed objects and filesystem CAS preparation;
independent review demonstrated that its passing 18-mutant lane omitted live
decision caches, SQLite staging, multi-root transactionality, parked candidate
binding, promotion-strength aggregation, and advertisement liveness. G12
records the corrected cross-consumer matrix and expanded mutation surface.
Independent-context re-review remains the merge gate. The matrix is a caller-
boundary complement to G5 mutation testing, not a substitute for it.

**G11. Universal signed-event body conformance.** W-19 closes the retained-row
cryptographic and envelope boundary for operator diagnostics: strict JSON,
signature, all seven materialized selectors, known kind, object-shaped body,
and per-span chain. It does not invent a complete per-kind schema validator.
Current producers and consumers validate the fields they emit or require, but
there is no language-neutral negative corpus proving every §6 body rejects
missing, mistyped, extra-semantic, or cross-kind fields at one shared boundary.
That is a conformance/publication gap rather than a storage-only forgery—the
current attacker cannot sign the malformed event—but G9 requires it to remain
explicit. W-6 owns machine-readable per-kind schemas and pass/fail fixtures;
authority consumers continue to fail closed on missing fields in the meantime.

**G12. W-14 cross-consumer assurance gap — remediation in review.** The first
independent review correctly found that the original inverse map cited an
escalation-approval test for promotion-strength aggregation, claimed immutable
SQLite preparation while retaining a mutable pathname, and mutated only the
shared loader plus filesystem helper. The corrected evidence adds direct
promotion-strength relabeling, meter reset/exemption injection, failed approval
append, SQLite post-prepare substitution, parked-candidate swap, both
promotion/revert event failures, ordinary later-store rollback, both reopen
recovery directions, and revoked/ungranted tools/list. The W-14 mutation lane
now includes the broker, kernel, tools, proxy, trace, and snapshot enforcement
boundaries. The corrected run covered 152 mutants: 140 caught, 12 compiler-
unviable, zero survivors or timeouts. Required CI passed 24 contracts / 131
contracted tests / 93 frozen tests, all 224 workspace tests, and both demos;
deep passed 4,096 authority cases and 512 model histories. Independent-context
re-review remains the merge gate.

## Automation lanes

| Lane | Purpose |
| --- | --- |
| Local required / pre-push | strict Clippy; two-sided contract validation; every workspace target; deterministic/exhaustive and bounded shrinkable properties; both executable acceptance demos |
| Local full | required lane plus the networked RustSec advisory audit |
| Weekly/manual deep | release-mode suite with 4,096 authority cases and 512 real-store model histories |
| Weekly/manual mutation | scoped evaluator, promotion, A21/M7, A22, verified-event authority, W-14 typed-object/restore integrity, and W-19 operator-ledger integrity mutation runs |
| Future fault/soak | full process crash matrix, adversarial MCP corpus, thousands-of-events/storage growth |
| Dogfooding | denial false-positive judgment, legibility, approval latency, bypass behavior, and real-corpus tripwires |

`scripts/ci` is the canonical definition of every implemented lane. The
repository-owned pre-push hook runs `scripts/ci required`; enable it per clone
with `scripts/install-hooks`. GitHub Actions is a Linux/macOS and scheduling
mirror that calls the same commands, not the source of their meaning. Push/PR
runs may cancel superseded runs, while scheduled/manual deep runs use separate
concurrency groups and are never cancelled by an ordinary `main` push.

The P7 meta-test compares the caveat dimensions emitted by its full-vocabulary
generator with `capability::KNOWN_DIMS`. Adding an evaluator dimension without
adding generated semantic-subset coverage therefore fails the ordinary suite
instead of silently weakening the claim.

The contract gate is intentionally repository-owned and dependency-free. It
requires workspace-unique test names, rejects ignored Rust tests, verifies that
every registered or grandfathered name exists in the compiled inventory, and
fails on any unclassified addition. The legacy baseline is migration debt, not
an extension point: move entries out as their invariants are contracted; never
add entries for new work.

## Tests as the future conformance suite

The neutrality strategy (brief §9) eventually requires a published
conformance suite — open formats without one are open in name only. The
invariant-mapped tests are its draft. Two style consequences now:
- Keep assertions phrased against spec behavior, not implementation
  detail, wherever the two can be separated.
- Prefer fixtures that could be extracted language-neutrally (canonical
  JSON vectors, sealed-object examples, attenuation pass/fail pairs) over
  assertions that only make sense inside this codebase.

## Relationship to dogfooding (separate deep-dive)

Tests prove the broker denies what it must; only dogfooding measures
whether it denies what it mustn't. The denial false-positive rate —
"the broker blocked good work" — is a product-thesis metric, not a test
assertion, and belongs to the dogfooding methodology doc (future), along
with the ADR 0002 tripwires (CAS growth, memory-db size, snapshot cadence,
SI-6 false-drift noise) and UX counters (escalations/session, approval
latency). Boundary rule: if a number can fail in CI, it belongs here; if
it can only fail in real use, it belongs there.
