# Landscape — the neighbor survey (2026-07)

Fine-grained survey of the projects building next door, one section
each. Not the strategic landscape claim (`docs/agent-state-fabric-brief.md`
§8 owns that, including the 2026-07-11 retirement of the naïve "nobody
has unified" framing) and not a follow-up tracker — the actionable
outputs of this survey ride in their own filings (noted at the end) so
this doc stays descriptive. This is the evidence file: what the
neighbors actually shipped, verified at source, so the whitespace claim
rests on pinned facts rather than remembered readmes.

Entry rule: every repo claim is verified at the named commit SHA at
filing time (external repos move fast; both GitHub neighbors shipped the
day before this filing); product-doc claims carry the read date;
press-derived claims name the outlet and are graded as-reported, never
silently blended with verified fact. Where this survey's collection
notes disagreed with the source, the source's names win and the
correction is recorded inline.

## The thesis this survey supports

Three independent builders — a hackathon team, a solo engineer, and a
venture-funded platform — converged within weeks of each other on the
same prevention kit: an authority boundary outside the agent's process,
credential isolation so the agent never holds the real secret, and (in
both open-source stacks) hash-chained tamper-evident audit. Those are
becoming genre table stakes, and ASF treats them as such (broker, C2
credential injection, signed per-span chains — built, dogfooding).

What none of the three touches is the delegation half: in-place undo of
the user's owned state with human-edit preservation (§5.3/A24/A27),
delegation manifests binding the four lineages, and zero-authorship
ratification. Every neighbor requires *more* upfront policy authoring —
a hand-written `policy.yaml`, pre-blessed plan hashes, allowlist and
stub rules — where the field-incident corpus
(`docs/field-incidents.md`, the 2026-07-15 Stripe row) shows authored
controls going un-authored is the norm the design must assume. And
every neighbor sells prevention, while the incidents the best-funded
neighbor markets with flow through legitimately granted authority —
the class prevention doesn't answer (see the positioning observation
under Runta). The delegation thesis these facts leave unoccupied is
the one the frontier log (F2′, `docs/dogfooding.md`, filed via PR #55)
now instruments.

## 1. custodian-kernel — spend governance from the same Hermes ecosystem

`github.com/KeyArgo/custodian-kernel`, verified at
`293bb761e98ba53cae83278ecbf6875b442727c4` (2026-07-15). Entry for the
Hermes Agent Accelerated Business Hackathon (NVIDIA × Stripe × Nous
Research) — the same agent ecosystem our wedge targets, which makes the
convergence a same-habitat data point, not a distant echo. Python,
ships on PyPI as `custodian-kernel`; their own stated limits: 1,346
tests but no third-party audit, SQLite-only storage, no multi-tenant
support, limited policy DSL expressiveness.

**Framing.** "The model proposes. The kernel decides." — the
authorization boundary lives outside the agent's process so the agent
cannot self-approve. That is the broker position, argued independently.

**Convergences.**
1. Out-of-band approvals: Twilio Verify SMS, and the verification code
   "is never written to any file the agent can read" — a daemon-owned
   approval surface that never transits the agent, our spec C2's
   day-one requirement arrived at independently.
2. Secret custody: the `caduceus` broker (vault, grants, receipts,
   audit, crypto modules) — the agent never holds the credential, brief
   principle 4.
3. Hash-chained audit: `caduceus/audit.py` is a "Hash-chained,
   HMAC-signed audit log for every broker decision" — `prev` digest
   linking, HMAC-SHA256 over prev + canonical body, genesis sentinel,
   walk-and-verify CLI. Tamper-evident decision logs as table stakes.
4. Egress and spend tripwires: built-in adapters
   `custodian/adapters/builtin/secret_leak_guard.py` and
   `spend_sentinel.py`. (Correction recorded: this survey's collection
   notes called this "LeakSentinel"; the as-shipped names at the pinned
   SHA are `secret_leak_guard` and `spend_sentinel`.)

**Opposite bets.**
1. Authority is a single linear scale — bands L0 (always-autonomous
   read-only) through L3 (always escalates), L4 reserved — not
   per-dimension conjunctive caveats; there is no subset-checking
   attenuation and no domain scoping.
2. Policy is hand-authored upfront: `policy.yaml` with `daily_envelope`,
   `margins`, `no_self_dealing` directives — the zero-authorship
   inversion. The Stripe incident row is the argument this does not get
   configured by real operators.
3. Expiry is optional: `Grant.expires_at: Optional[float] = None` with
   `None` meaning no expiry (`caduceus/grants.py` at the pinned SHA) —
   against our mandatory-expiry invariant.

**What it cannot do.** No state custody, no snapshot, no undo, no
recovery story of any kind — spend and secrets only. A failed delegation
here is prevented or it is permanent.

## 2. cyberware — solo execution-governance runtime

`github.com/rhCat/cyberware`, verified at
`88a94f07c80f66ae736a26f458867817b47dd71c` (2026-07-15). Solo-built
(single visible contributor), and the most spec-driven of the three:
normative documents with MUST-language, test vectors, an independent Go
verifier, and drill-style negative verification — a discipline kin to
our contracts lane.

**Framing.** "The agent proposes; nothing runs except through cyberware
— and every action that does is governed …, verifiable (provably the
blessed step, pinned by hash — not whatever the model improvised), and
ledgered." Engine/cartridge split: skills live in a separately-versioned
`skillChip` whose identity is the hash of its parts.

**Convergences — several land inside our open issues.**
1. Blessed, hash-pinned plans: execution is refused unless it is
   provably the pre-blessed step. This is the attestation half of our
   W-4 (intent/behavior attestation + containment) shipped as the core
   product primitive.
2. Value-free governor: the `govd` wire carries "only the claim (skill,
   perk, var KEYS) … never code, never secrets" — credential isolation
   pushed into the wire design itself; secrets and data never transit
   the control plane.
3. RFC 8785 (JCS) canonicalization implemented in-tree
   (`infra/cwp/canonical.py`) with an independent Go verifier
   (`verifiers/go/jcs.go`) and generated test vectors — a shipped,
   cross-language JCS deployment one repo over. This is live evidence
   for the F2 canonicalization decision (JCS vs IPLD, CLAUDE.md open
   problem); surfaced here, decided nowhere but F2's own filing.
4. Crypto-shredding with the chain over ciphertext: personal fields
   stored as ciphertext under a subject-scoped DEK, "the chain MUST
   hash the ciphertext," erasure performed by destroying the DEK with
   the chain still verifying end-to-end, enforced by an erasure drill
   (`spec/privacy.md` §2, P1-V13). Same design family as our
   per-payload encryption + tombstoning, and direct evidence input for
   open SI-28 (AEAD envelope), SI-29 (post-shred returns), SI-30
   (redaction commitments). (Correction recorded: collection notes said
   per-record DEK; the spec says subject-scoped.)
5. Two-tier in-flight revocation (`spec/inflight.md` §1): ordinary
   revocation halts at the next step boundary so the ledger never
   records half a step; `severity: critical` kills the sandbox
   immediately, accepting the ledger seam because "letting the current
   step finish is itself the risk." A worked answer to the same
   latency-vs-consistency trade A22/§5.4 closure navigates.
6. Declared-vs-materialized verification: `govd` materializes a per-run
   workspace and `exod` re-hashes the whole materialized closure at
   time of use against the grant pin (their TOCTOU class), under a
   bwrap `SandboxProfile` with signed capability grants — containment
   evidence for W-4's other half.

**Opposite bets / what it cannot do.** Prevention via pre-approval:
plans must be blessed and hash-pinned before they run, which is *more*
authoring precision demanded upfront, not less — and mid-run "actually
do Y" amendments (our SI-36 territory) have no path except re-blessing.
And by its own best property it forecloses state custody: a value-free
governor never sees the state a custody fabric must capture, so
snapshots, undo, and merge of user-owned state are structurally outside
its design, not merely unbuilt.

## 3. Runta — the venture-funded runtime climbing toward authority

`runta.com/docs` (read 2026-07-16), plus The Information's July 2026
coverage (exclusive founder interview) — the press layer is
second-hand and not independently verifiable; it is reported here
as-reported, never as verified fact. Docs self-description: "an
execution layer for AI agents … scalable, governed runtimes with strong
control over state, access, credentials, and execution."

**As-reported (The Information, July 2026).** $20M seed at a $100M+
valuation led by Martin Casado (a16z); angels reported to include Jeff
Dean, Fei-Fei Li, Ali Ghodsi, Ram Shriram, Thomas Wolf. Founder Guanlan
Dai — ex-Cloudflare Edge Platform lead, founding engineering leader at
Kong; the background is corroborated by public profiles, the round is
not independently confirmed anywhere we can check. Stated pitch:
Modal-class sandboxes combined with Microsoft/Okta-class agent access
control, agent-native ("parent their AI agents") — stack-climbing into
the authority layer is the explicit plan, which makes Runta the
neighbor most likely to occupy adjacent ground rather than complement
it.

**Convergences (verified in product docs).**
1. Secret Stubs: host/path-matched injection rules — "the literal
   `${credential}` placeholder is replaced with the stored secret value
   when the egress gateway injects the outbound request," so Runta
   "can authenticate your agent's request without exposing real
   credentials to your agents." That is our C2 credential injection
   implemented at the network plane — the third independent instance of
   agent-never-holds-the-secret in this survey.
2. Egress control as a first-class, per-runtime feature: hostname and
   wildcard-host policies with allowlist and denylist modes.
3. Checkpoints: point-in-time capture of runtime state, filesystem and
   running processes included.

**Opposite bets (verified in product docs).**
1. DEFAULT-OPEN egress: "An empty denylist is the default open policy"
   — and resetting policy returns to open. Our conservative-defaults
   invariant, inverted: their default answers the adoption question,
   ours answers the safety question, and their posture means an
   un-authored deployment exfiltrates freely.
2. Checkpoints fork, never revert: "Restoring a checkpoint creates a
   new runtime," restorable many times to fork many runtimes. There is
   no in-place revert, no three-way merge, no human-edit preservation —
   recovery is VM lifecycle (the brief §8 sandbox-layer observation),
   not state custody. Divergent copies of your state are the product's
   answer, reconciling them is your problem.
3. Token X-Ray is cost observability ("identify potential token wasting
   patterns"), not authority observability — the spend lens without the
   decision lens.

**Positioning observation.** The incidents Runta's founder cites in the
same coverage (both filed as the 2026-07-16 grade-B rows in
`docs/field-incidents.md`) are an authorized agent deleting production
files and an injected agent executing malicious code. The deletion
incident is not claimed cleanly for undo — it sits on the custody
boundary, and the report doesn't say which side: fabric-custody state
is the snapshot-backed-undo case, external infrastructure reached
through granted authority is the candidate-bound-approvals case with
compensation fidelity (open) past an approved-but-wrong action; the
field-incidents row records the split. What the softening does not
blunt: in every reading of both incidents the damage flows through
legitimately granted authority, so isolation reaches none of it — a
default-open egress posture and fork-only checkpoints answer no branch
of either. A prevention vendor marketing with incidents whose every
reading calls for the delegation stack — undo where custody exists,
candidate-bound approvals where it doesn't, compensation past that —
is market evidence that the delegation half is the unserved demand.

## Synthesis

Table stakes (build-assumed, differentiate-nothing): authority boundary
outside the agent process (all three), credential isolation from agent
context (all three, three different planes — process broker, value-free
wire, egress gateway), hash-chained tamper-evident audit (both
open-source stacks; Runta's docs show no tamper-evidence story). ASF
ships all three today; none of them is the moat.

Unoccupied (the delegation half, no neighbor within reach):

1. In-place owned-state undo — snapshot-backed revert with three-way
   merge and human-edit preservation (§5.3/A24, A27 staged-bytes).
   Runta forks instead of reverting; cyberware cannot see the state;
   custodian-kernel has no state story at all.
2. Delegation manifests binding the four lineages — no neighbor binds
   even two; cyberware's blessed plans bind behavior to authorization
   but neither to state nor to a portable trace of what the authority
   earned.
3. Zero-authorship ratification — every neighbor demands more upfront
   authoring (policy.yaml, blessed hashes, allowlists and stub rules);
   none has policy entering through a ratification loop over lived
   examples, and the corpus says the authored kind goes unwritten.
4. Recovery as the product — all three sell prevention; the neighbor
   with the most money markets prevention using incidents whose every
   reading calls for the delegation stack (undo where custody exists,
   candidate-bound approvals where it doesn't, compensation fidelity —
   open — past that).

Follow-ups ride elsewhere, deliberately: a roadmap candidate for a
custodian-style egress tripwire on the broker (their
`secret_leak_guard` adapter, filed from this survey), and an evidence
survey feeding cyberware's JCS/crypto-shredding/chain-over-ciphertext/
revocation designs into SI-26…SI-30 and the F2 decision (filed from
this survey). This doc records what exists; those filings argue what to
do about it.
