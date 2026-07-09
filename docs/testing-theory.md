# Testing theory — Stage 2 snapshot

Hinge document for the milestone-3 testing deep-dive. This records what the
suite *is*, what it deliberately is not yet, and the questions the deep-dive
must answer — so that conversation starts from analysis, not archaeology.
Written 2026-07-08, at 55 tests / milestones 1–2 complete.

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
deviation — they are the human-legible face of the same assertions.

**5. Negative space is tracked explicitly.** Invariants with NO test today,
because their machinery doesn't exist yet: C3 (signed fabric→user
messages), C4 (delivery ceilings), C5 (sender binding), M3 (Tier-3 as_of
surfacing), the taint dimensions (currently proven only to fail closed),
promotion-gate invariants (§5.3: three-way merge, agent-never-wins,
trace-vs-capability check), StandingRule schema enforcement (k≥3,
counterfactuals, domain match). These are absences by sequencing, not
oversight; each activates with its milestone. The deep-dive should keep
this list current — an untracked untested invariant is how "the spec is
the source of truth" quietly stops being true.

## Known gaps — the deep-dive agenda

**G1. Property-based testing.** *(Status update, milestone 3: the core
properties landed in `tests/properties.rs` — merge identities/conflict
soundness over seeded random tree triples, exhaustive glob-cover
soundness, and the semantic-subset property below over random capability
pairs. Implementation is seeded-xorshift + exhaustive enumeration, no
proptest dependency; counterexample shrinking quality is the remaining
open question for the deep-dive.)* The highest-value target in the
codebase, because attenuation is an algebra and hand-picked cases
undersample it:
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
- The promotion gate's three-way merge, when it lands, is the second big
  candidate (merge properties: human-trunk-wins, rename detection never
  renders as delete).

**G2. Differential canonicalization.** Cross-implementation JCS vectors —
hash the same objects with a second RFC 8785 implementation (any language)
and compare. Becomes existential the moment a second implementation of the
spec exists, and it is the enforcement mechanism for the §0 number rule
proposed in ADR 0002. Deliverable shape: a language-neutral fixture file
(object JSON → expected id) that lives with the spec, not with this repo's
tests.

**G3. Crash consistency.** Revert is prepare-all-then-swap-all; the claim
is that a crash mid-swap leaves recoverable staging, never a half-written
store. Nothing kills a process mid-revert today. Same family: WAL/-shm
sidecar handling when a sqlite store is restored under a crashed reader.

**G4. Concurrency.** `Mutex<Broker>` serializes decisions, but nothing
tests interleavings: concurrent in-flight tools/calls through the proxy,
approval-socket resolutions racing metered calls, meter increments under
contention. The id-routing map in the proxy is tested only implicitly.

**G5. Coverage honesty.** No mutation testing; assertion strength is
unmeasured. A cheap first pass: mutate the evaluator's comparison
operators and confirm the suite notices.

**G6. Soak / growth.** Thousands-of-events runs: ledger size, WAL
behavior, meter table growth, verify_all_spans latency over real volume.
This is also where the ADR 0002 tripwire instrumentation (`asf stats`)
gets its baseline numbers before dogfooding starts.

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
