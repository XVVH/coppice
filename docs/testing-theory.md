# Testing theory — Stage 3 dogfooding baseline

This records what the suite is, what it deliberately is not yet, and which
kind of automation owns each claim. Updated 2026-07-10 at 117 named tests,
plus 576 shrinkable generated cases in the default run and deeper scheduled
CI. The promotion gate is complete; dogfooding is now the product-signal lane,
not a substitute for correctness testing.

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

**5. Negative space is tracked explicitly.** Invariants with no conformant
test because their machinery or ratified representation does not exist yet:
C3 (signed fabric→user messages), C4 (delivery ceilings), C5 (sender binding),
M3 (Tier-3 `as_of` surfacing), M4 runtime behavior attestation, M7's brokered
manifest authority edge (SI-21), the taint dimensions, and StandingRule schema
enforcement (k≥3, counterfactuals, domain match). These are absences by
sequencing, not oversight; each activates with its milestone. An untracked
untested invariant is how "the spec is the source of truth" quietly stops
being true.

## Current invariant matrix

| Invariant | Automated evidence | Status |
| --- | --- | --- |
| M1 roots cover capability reach | `m1_mint_rejects_uncovered_store`, `attenuation_rechecks_m1_for_the_child_manifest` | covered |
| M2 capability binds manifest | `m2_capability_dies_with_its_manifest`, broker/gate suites | covered |
| M3 Tier-3 `as_of` in consent | none; Tier-3 absent | future milestone |
| M4 loaded behavior attestation | behavior lineage is recorded, but first-call runtime attestation is absent | open implementation gap |
| M5 re-manifest on behavior change | `m5_behavior_change_forces_remanifest` | covered |
| M6 cheap/frequent manifests | exercised throughout multi-boundary tests; guidance rather than a binary predicate | exercised |
| M7 observed vs brokered authority | SI-21 records the unresolved content-address cycle | blocked on ratification |
| M8 attribution completeness | e2e, gate, proxy timing tests, generated state-machine histories | covered for current consumers |
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
tests.

**G3. Crash consistency.** Preparation failure, injected interruption, hard
process exit during an in-place filesystem apply, reopen, and idempotent replay
are covered; session-boundary SIGKILL recovery is covered too. Nothing yet kills
a process *during* a multi-root promotion/revert commit, between state mutation
and event append, or between event append and expected-root updates. Add that
full subprocess crash matrix when the atomic commit protocol is designed. Same
family: WAL/-shm sidecars under a crashed reader.

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
terminate and make mutation-run exit status noisy.

**G6. Soak / growth.** Scheduled CI runs release mode with deeper generated
case counts. A true thousands-of-events/files run remains: ledger size, WAL
behavior, meter growth, verification latency, and ADR 0002 `asf stats`
baselines.

**G7. Process-harness fidelity.** Proxy tests use bounded pipe/socket reads,
bounded child exit, captured stderr, and child status in failures. They can now
substitute a barrier-controlled downstream instead of relying only on the
in-tree vault server. The remaining adversarial MCP corpus is malformed large
frames, duplicate/out-of-order ids, unsolicited notifications, and downstream
death at each protocol phase.

## Automation lanes

| Lane | Purpose |
| --- | --- |
| Pull request | strict Clippy; full tests on Linux and macOS; deterministic/exhaustive and bounded shrinkable properties; RustSec audit |
| Weekly/manual deep | release-mode suite with 4,096 authority cases and 512 real-store model histories |
| Future fault/soak | full process crash matrix, adversarial MCP corpus, thousands-of-events/storage growth |
| Dogfooding | denial false-positive judgment, legibility, approval latency, bypass behavior, and real-corpus tripwires |

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
