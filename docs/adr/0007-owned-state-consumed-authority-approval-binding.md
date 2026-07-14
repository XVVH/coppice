# ADR 0007 — Owned-state transition, consumed authority, and approval candidate binding (retro-ratification of W-14's in-PR protocols)

**Status: RATIFIED as amendments A24–A26 (spec v0.9, §5.3/§5.5/§6) on
2026-07-14 — determinations D31-1…D31-6, D33-1…D33-5, D34-1…D34-4 below,
**as adjusted by the post-review adjustments R1–R20 (review rounds 1–4;
the adjustments win where they differ**; D31-2, D31-4, D31-6, D33-3,
D33-4, D34-2, and D34-3 are read as adjusted). The spec text is the
normative *language*. Claims about the candidate code carry one of three
labels, kept distinct throughout: **ratified as built** (the merged W-14
mechanism is the ratified semantics), **adjusted beyond as-built** (the
ratified semantics exceed the code; RF-35/RF-36/RF-37/RF-39 track, W-15/
W-22 carry), and **as-built defect** (the code fails a clause ratified as
built; RF-40). A clause in the second or third category binds
implementations at its carrier gate per the ledger — the W-20 item (6)
posture-qualifier convention will systematize this status; this batch is
its founding evidence.
SI-31, SI-33, and SI-34 are RESOLVED. The candidate-code gaps are tracked
as RF-35 (recovery, carried by W-15), RF-36/RF-37 (gate binding parity
and re-merge outcome equality, carried by W-22), and RF-39 (exemption
candidate binding, carried by W-22); RF-38 files the anomaly-recovery
question. Round 1 of the independent-context review returned REQUEST
CHANGES (nine findings, four high), ratified as R1–R7. Round 2 returned
REQUEST CHANGES (seven findings, three high — a missing V source, a
non-durable capture, an unbound exemption candidate, plus ABA freshness
and a defective equality quantifier), all verified at source and ratified
as R8–R13 below, adding one new protocol artifact (the recovery capture
record) and positional freshness. Round 3 returned REQUEST CHANGES (five
findings, two high, all in the round-2 additions and evidence
bookkeeping — the round-2 protocol's own repeated-crash window, an
exact-set validation predicate, the R12 comparison basis), ratified as
R14–R16 below with SI-39 filed for the multi-window enhancement; the
reviewer confirmed no original SI decision point was silently dropped.
Round 4 returned REQUEST CHANGES (five findings, three high), ratified
as R14–R16's successors R17–R20: the temporal-binding gap turned out to
live at **both** consumers (RF-36 widened; the round-1 "gate weaker than
decision" record corrected), R14's quantifier was blind to deletions
(path-state union), two **as-built defects** surfaced in the merged fs
restore (RF-40 — mode-only skip, file↔directory deadlock: the first
code-fails-ratified-as-built findings of the cycle), and R16's claimed
merged-tree referent did not exist (previews gain per-store merged
roots). The corrected text awaits round 5.**

## Context

W-20 item (2). The candidate is not a design document but a **merged
implementation**: the three protocols W-14 (PR #43) designed inside its
remediation cycle, retro-filed as SI-31 (owned-state transition / recovery
journal, from RF-30), SI-33 (event-derived consumable authority, from
RF-29), and SI-34 (approval candidate binding, from RF-31). PR #43's
independent re-review (2026-07-13, APPROVE WITH NON-BLOCKING FOLLOW-UPS)
used these SIs as its oracle and verified conformance against source. The
review-finding triage rule (founded by that same cycle) says such protocols
are normative semantics that must be ratified, not inherited from an
implementation — this ADR is that ratification.

Inherited, not re-decided: ADR 0006's SI-25 × SI-31 durability seam
(**S1–S5**), the two-terminals rule (**R6**), and journal-before-stores /
checkpoint-is-a-row (**R8**). SI-31's "journal freshness against
database/home rollback" item is answered by composition where both journal
and database roll back together (S4: prefix membership; anchor-ahead is
layer 2's signal) and by determination D31-4 where the journal alone is
stale.

## Method

Challenge pass over the merged code (2026-07-14): implementation mapped to
line level (kernel.rs `commit_state_change_with_hook` /
`recover_pending_state_change`; broker.rs `w14_decision_authority` /
`resolve_escalation` / `w14_promotion_binding` /
`w14_verify_promotion_candidate` / `approve_promotion`; snapshot.rs
`apply_fs_in_place` / `commit_restore`), then a fresh-eyes source
verification of every load-bearing claim. The verification pass corrected
two errors in the challenge pass itself and surfaced one finding more
serious than either headline finding — recorded under "Corrections during
the pass" below, provenance-preserved, because a ratification record that
hides its own misfires teaches the next session to trust unverified
summaries.

## Determinations — SI-31 (owned-state transition)

**D31-1 — the state machine and commit point, ratified as built.**
Identity guard (every live root equals the transition's `before`; divergence
fails closed into M8's attribution) → forward *and* rollback images prepared
and CAS-hash-verified into immutable plans before any mutation → one
uncommitted SQLite transaction staging the signed event, expected-root
attestations, and companion rows → fabric-signed journal published and
fsynced (file + parent) *before* any store mutation (R8) → stores applied
and fsynced → `tx.commit()` as the single authoritative commit point, the
linked verified event naming the committed root tuple → journal removed.
Ordinary failure: every before-root restored and fsynced, transaction rolled
back, journal removed. Rollback-failure-during-rollback: loud compound
error, journal retained for reopen recovery. Commit-error: immediate
recovery attempt, else journal retained. A retried transition is a fresh
event; the journal authorizes completing or undoing exactly the transition
it names, never re-execution.

**D31-2 — the journal is a normatively defined protocol artifact, not a §6
event kind and not a database row.** W-14's explicit "ratify or amend"
choice, ratified with its rationale made citable. Not an event kind: §6
records are permanent substrate history; the journal's *removal* is
protocol-meaningful (S4's discriminator), an event-about-committing-events
recurses exactly where R8 killed the checkpoint-as-file, and the permanence
job is the transition event's. Not a row: the journal is the write-ahead
record for the transaction itself — a row inside the transaction vanishes in
precisely the crash it exists to recover. These are the two sides of one
principle: **one recovery artifact per side of the commit point, each
durable where its consumer looks** (journal: filesystem, consumed by reopen
before the DB is trusted; checkpoint: row, consumed by verifiers after
commit). Fields ratified: typed `state_change_recovery`, versioned, sealed
`txn:` under the fabric key, linking exactly one event id + kind + manifest,
full before/after root tuples. **Adjustment:** the journal gains its linked
event's `home`/`epoch` (TracePosition) at W-15 — H3's one-home/one-epoch
binding extended to recovery artifacts — and recovery in an epoch honors
only journals whose linked event lies in that epoch's activation prefix
(R9). No machinery exists to bind before W-15 (epochs are unimplemented);
single-epoch posture bounds the interim.

**D31-3 — fail-closed recovery validation, ratified as built.** Reopen
recovery runs under the gate lock before any other operation; recovery
failure keeps the home closed; recovery is idempotent and re-runnable.
Symlinked, non-file, mistyped, wrong-version, malformed-tuple,
unsupported-kind, and event/manifest-misbound journals are rejected before
any restore. Event-present-but-roots-mismatched fails closed. The journal
write side is `O_CREAT|O_EXCL|O_NOFOLLOW`.

**D31-4 — the freshness predicate (adjustment beyond as-built).** As
merged, the roll decision checks the journal against *itself* (linked event
present/absent; kind/manifest/tuple agreement with the journal's own
fields) and never against the substrate's current view — so a stale
retained journal (backup restore, copied home directory, adversarial
replant) whose linked event is committed rolls live stores back to a
historical tuple, and one whose event never committed rolls them back to
its `before` images: an unrecorded, kernel-executed state mutation.
Ratified predicate: recovery derives the current-roots view **V** from the
verified prefix — latest per store of transition roots (promotion `merged`,
revert `roots_restored`) and drift `observed_root`, manifest-attested at
genesis — and requires `journal.after == V` to roll forward and
`journal.before == V` to roll back; any other relation fails closed,
journal retained, loud. Both genuine crash cells satisfy the predicate by
construction (before-commit: V is the pre-transition view the identity
guard pinned `before` to, and drift cannot be recorded while the fabric is
down; after-commit: the committed event is the substrate's last word on
those stores). The `expected_roots` table is recognized as the unsigned
*cache* of V — so D31-4 is **A25's cache doctrine applied to recovery's
inputs**, the same "unsigned rows are never authorization inputs" rule at
its third consumer (decision, gate, recovery). Composes with parked RF-27
(recovery selectors from `VerifiedEvent`). Honesty note: under the current
posture the adversarial replant is moot — RF-14 leaves the fabric key
cleartext in the home, so a same-uid attacker forges rather than replants;
the predicate's near-term value is **accidental** staleness (backups,
copied homes — the roaming/backup reality), and its adversarial value
activates with W-17 key custody. Carried by W-15 (RF-35); the same function
already receives S4's prefix-bounding there.

**D31-5 — scope fences, ratified.** Tier-1-local only; the external-effect
analog is the parked durable external-effect protocol. Filesystem adversary
tiers are SI-32's question (next in the W-20 batch); the concurrent
human-edit-during-gate exposure is SI-32 tier 3's named item. The
hard-exit-at-each-syscall matrix remains G3 evidence work under any
ratified shape.

**D31-6 — capture-and-attribute before restore (adjustment beyond
as-built; the pass's most serious finding).** As merged,
`recover_pending_state_change` goes straight from validation to restore,
and the fs apply's first pass **deletes live files the target does not
contain**. A crash mid-transition followed by any human editing before
reopen — the fabric can be down for days, and the vault is precisely where
humans edit — has those edits overwritten or deleted with **no CAS copy**
(the fabric was down; nothing snapshotted them) **and no drift event**. The
in-code comment "drift detection attributes anything left over" is true
only for re-runs of the same interrupted restore, not for downtime edits.
Citable clause: M8 — recovery is a revert-family consumer of live state,
and "ledger attribution never depends on when a change occurred relative to
session lifetime." Ratified: before any restore mutation, recovery captures
every affected live root to the CAS and records an attributed drift event
(conservative class `unattributed` — the window is unattended; the quiet
`human_local` default is for attended windows); when live state already
equals the restore target, recovery MAY skip the restore and complete as
journal cleanup. Carried by W-15 (RF-35). The in-process ordinary-failure
rollback path shares the clobber mechanics but its window is gate-bounded;
that case composes at SI-32 tier 3 rather than duplicating here.

## Determinations — SI-33 (consumed authority)

**D33-1 — the doctrine, ratified as built.** Consumable authority is a
pure function of the verified event prefix plus the broker's declared
in-flight reservations; unsigned rows are caches never read for
authorization — A15/A22 extended from existence to consumption. As merged:
`w14_decision_authority` consumes only verified
escalation/approval/tool_call events plus pending reservations;
`broker_meters` is neither written nor read; `exemptions` is written,
never read. §5.1's "broker-metered" is redefined by reference to §5.5.

**D33-2 — the reservation object, two-stage, ratified with its residual
named.** The in-memory `PendingCall` (created at Allow, counted by every
subsequent decision, released when the signed `tool_call` records) is the
sanctioned Tier-1-local form. Its crash-loss residual is RF-33, accepted:
restart loses reservations for dispatched-but-unrecorded calls, bounded by
the gate's recount from signed events, RF-9 branch stranding, and revert.
At first live egress the reservation MUST become the durable
external-effect protocol's signed dispatch/reservation record (P22).

**D33-3 — exact-match approval binding and fail-closed edges, ratified as
built.** Headroom requires: approval strictly after its escalation in
substrate order — the normative order is A23's composite order, with
within-span signed `seq` the sanctioned comparison while both acts live on
the fabric-lifetime span per A15 (verified: both `enqueue_escalation` and
`resolve_escalation` append to the substrate span, so the comparison is
sound as built; W-15's TracePosition makes it uniform) — manifest,
capability, and caveat exactly equal on both events and against the
capability's M2 binding; auth-strength present and sufficient (C6);
resolution `approved`. Zero-use approvals are inert (ratified as the
semantics; creation-time refusal is optional hardening, rejected as a
normative requirement). Approval-without-escalation grants nothing.
Chain anomalies — conflicting bindings for one escalation id, multiple
resolutions of one escalation — fail the **entire consumption view**
closed, loudly, never degrading to row-skipping.

**D33-4 — one commit; decision/gate split, ratified as built.** The
approval event, escalation status, and exemption cache row commit in one
transaction — widening state never exists without its signed event.
Decision time enforces the view at the current local verified terminal
(R6); the gate's independent recount from signed events at each effect's
durable authorization offset is authoritative for what becomes durable
(§5.4's rule extended to consumption; composes with the accepted RF-3
residual).

**D33-5 — forward consumers, stated once.** §7 rule-health counters,
TrustRecord evidence, and W-3's k ≥ 3 founding-example counting are the
same doctrine's next consumers: counted over the `VerifiedPrefix` under
D2's terminal discipline, never over mutable rows. W-3 inherits instead of
re-deriving.

## Determinations — SI-34 (approval candidate binding)

**D34-1 — the general rule (WYSIWYA), ratified.** Every approval act binds
to the exact signed candidate presented: escalation-side signs candidate
identity, both resolutions (approved and denied) repeat it, consumers
recompute and verify before auth-strength, drift, or merge work; candidates
without their exact signed escalation are inert. C1 gave approvals
provenance; A26 gives them an object. Stated generally so every approval
surface inherits it — including W-3's rule ratification, where the
counterfactuals shown are candidate identity (R1's informed-consent
lineage), pre-declared here and implemented there. Rendering fidelity of
the approval surface itself (does the human *see* the candidate the digest
names) is the broker-owned surface's contract — C2 today, SI-23's question
under actuation.

**D34-2 — per-type candidate identity and policy-context versioning,
ratified as built with one description corrected.** Parked promotion:
promotion id + manifest + JCS digest of the exact preview + JCS digest of
the branch-root tuple + versioned policy context
(`default-policy/manual-review/v1` is the sanctioned zero-authorship value
until StandingRules replace the default policy). Verification is an exact
conjunction over the signed escalation; multiple matching bindings and
missing bindings are errors; already-resolved candidates are inert.
Escalation exemption: D33-3's tuple is this rule instantiated. Policy
change: the challenge pass initially described a superseded-policy approval
as "re-parks and re-presents" — **wrong**; as built (and as ratified) the
digest mismatch fails loud and nothing silently re-parks; re-presentation
is a fresh escalation with a fresh candidate under the new policy. Silent
re-application was rejected as the R1 fatigue machine in mechanism form.

**D34-3 — the re-merge boundary, ratified with its rationale made
normative.** `approve_promotion` re-merges the pinned branch roots against
*live* trunk and applies that plan without comparing it to the previewed
ops — ratified as correct: the approval authorizes the candidate, never a
trunk instant. Structure forces safety: drift is attributed first (M8, same
lock and check as the auto gate), conflicts resolve trunk-wins (A11), so
trunk movement can only *narrow* the applied agent-originated delta below
the preview — and narrowing is the free direction (§3.1). The citable
sentence: a re-merge that widens the applied delta beyond the signed
candidate is a spec violation. The ⊆-narrowing argument is design
rationale, not a proven theorem — G13 files the missing
trunk-drift-cannot-widen negative. Rejected: strict re-park on any trunk
movement (approvals would perpetually race the human's own edits; the
narrowing analysis shows the race is already safe).

**D34-4 — §6 bodies gain the candidate fields normatively, ratified.**
Escalation body (promotion type): `{promotion, manifest, candidate_digest,
branch_roots_digest, policy_context, caveat, count, sample}`; both
resolution bodies repeat `candidate_digest`; the approval commits
atomically inside the A24 transaction (the two protocols share the commit
point by construction). Digest construction is §0 JCS + SHA-256 — already
normative machinery; the digests are type-untagged JCS objects today and
inherit SI-26's transcript/type-binding reservation explicitly. Full
per-kind body schemas remain G11/W-6.

## Corrections during the pass (provenance)

The fresh-eyes source verification of the challenge pass found: (1) the
D34-2 "re-parks and re-presents" misstatement, corrected above; (2) the
first-draft D31-4 rollback arm was underspecified ("agree with the current
signed owned-state view" without defining the view) — grounded after
verifying that `check_drift` UPSERTs `expected_roots` *and* records the
drift event, which is what makes V fully signed-derivable and the table a
pure cache; (3) the replant threat framing needed the RF-14 honesty note
(forge beats replant until W-17); (4) D31-6 itself — missed by the
challenge pass, found by verifying Finding 1's mechanics at source; (5) the
escalation/approval span-placement concern dissolved on verification (both
on the fabric-lifetime span; no bug, wording precision only).

## Rejected alternatives (consolidated)

Journal as §6 event kind; journal as database row; trusting
`expected_roots` at recovery; skipping anomalous consumption rows instead
of failing the view; creation-time refusal of zero-use approvals as
normative; strict re-park on trunk movement; silent re-park/re-application
under a superseded policy context.

## Composition seams reserved

SI-32 owns the filesystem adversary tiers (publication discipline,
tier-3 concurrent human edits, journal+database co-tampering); SI-26 owns
the signing transcript / type binding for the candidate digests and the
journal; SI-27 owns key lifecycle (until then RF-14 bounds every
signed-artifact claim here); the durable external-effect protocol owns the
egress-side reservation and dispatch record; G3 owns the syscall-level
crash matrix; W-3 consumes D33-5/D34-1 when it lands.

## Post-review adjustments — round 1 (W-20, 2026-07-14 — operator-ratified)

The first independent-context review of the drafted text (PR #48) returned
REQUEST CHANGES: nine findings, four high, all nine verified at source by
the author before triage. Two high findings refuted determination *content*
(D34-3's narrowing theorem; the D31-4×D31-6 composition), one exposed a
false evidence claim on the authority surface (gate replay), one a
self-contradictory predicate (D31-2's epoch guard). The operator ratified
the following adjustments; the spec text and the determinations above are
read as adjusted.

**R1 — gate replay applies the same binding predicate (finding 1).**
`gate_trace_check` pre-aggregates approvals by (capability, caveat) with no
escalation binding, no order check (an approval later than a tool_call
retro-funds it), no manifest/M2/auth-strength verification, and no
double-resolution detection — so the gate is *weaker* than decision time,
inverting D33-4's authority hierarchy, and the round-0 G9 sweep cell
claiming A25 gate enforcement was false. Adjusted clause: the gate's
recount MUST apply the same exact-match binding predicate as decision time,
evaluated at each effect's durable authorization offset — one shared
reconstruction, never a weaker aggregate (the W-2 shared-evaluator
discipline extended to consumption). As-built gap filed as **RF-36**
(high; exploitable via foreign/replayed traces — the W-9 corpus surface;
posture-bounded locally because the broker is the sole approval producer).
Carried by W-22.

**R2 — re-merge outcome equality replaces the narrowing theorem
(finding 2).** The round-0 rationale — trunk drift can only narrow the
applied delta — is **refuted**; the reviewer's counterexample is preserved
here as the refutation record: preview against trunk `H` shows the branch
edit conflicting, resolved trunk-wins, *nothing applied*; trunk then
returns to base `B` before approval; the approval-time re-merge sees no
conflict and installs the full branch edit the human was shown not
landing. The error conflated "within the pinned branch candidate" with
"within the previewed outcome" — and the preview, conflict cards included,
is part of the signed candidate. Adjusted clause: the approval-time
re-merge MUST reproduce the previewed outcome exactly (same per-store
results, op-set, and conflict resolutions); any difference re-parks as a
fresh candidate with a fresh escalation and digest — fail-closed into
re-presentation, never silent application of an un-previewed outcome.
Outcome-preserving trunk movement proceeds; outcome-changing movement
re-asks, which is exactly when re-asking is right. This supersedes both
the refuted rationale and the round-0 rejection of re-parking (what was
rejected — correctly — was re-parking on *any* movement; outcome-
conditional re-parking does not race the human's edits). As-built, no
comparison exists — **RF-37**, carried by W-22. D34-3 is read as revised.

**R3 — the journal epoch guard splits by arm (finding 3).** Round 0's
universal predicate — "recovery honors only journals whose linked event
lies in the epoch's activation prefix" — contradicts the roll-back arm,
where the linked event is *by definition* absent (that absence is the
discriminator); a literal implementation fails closed on every genuine
pre-commit rollback. Adjusted: roll-forward requires the linked event in
the current epoch's activation prefix (R9); roll-back requires the
journal's **own** signed home/epoch (the W-15 TracePosition fields) to
name the current home and epoch. Event absence remains the roll-direction
discriminator; the epoch guard never reuses it. D31-2 is read as revised.

**R4 — total V and the recovery emission order (findings 4 and 5).**
As drafted, D31-6's pre-restore drift event *poisons* D31-4's V: appending
drift (observed = the mixed post-crash root P) makes P the newest signed
root, so post-recovery V disagrees with restored live state forever, the
kernel's restore is an unrecorded mutation, and a crash between drift
append and journal removal bricks the home (retry: neither journal tuple
matches V = P). The two determinations could not compose as written.
Adjusted, three parts. (a) **V made total:** per store, V is the latest
signed root attestation in composite order among manifest `snapshot`
events, promotion `merged`, revert `roots_restored`, drift
`observed_root`, and the closing record below; comparison **projects V
onto the journal's store set** (a manifest that deliberately scopes to a
subset of the home's stores journals only that subset); a journal naming a
store with no signed attestation fails closed. (b) **Emission after
restore:** freshness evaluates V once, against pre-recovery signed state;
live roots are CAS-captured before restore but **no event is emitted until
the restore completes**; then one atomic transaction appends the window
drift (V → captured root, `attribution: "unattributed"` — the
crash-to-reopen window is unattended) and the **closing record**
re-attesting the restored tuple; then the journal is removed. Every crash
cell converges on retry: before the emission transaction V is unchanged
and recovery re-runs from scratch (capture and restore are idempotent);
after it, V equals the restored tuple, freshness passes, and recovery
completes as cleanup. Post-recovery V ≡ live, always. (c) **The closing
record** is drift-kind with a new A12 attribution class
`fabric_recovery` — zero body-schema motion, and the ledger honestly
distinguishes the kernel's own in-band recovery restore from out-of-band
divergence; it never counts as a divergence window for M8 purposes.
D31-4 and D31-6 are read as revised; RF-35's items updated to match.

**R5 — exemption candidate identity stated precisely (finding 6).** The
original SI-34 filing asked whether an exemption candidate binds caveat,
action class, and uses; round 0 answered by reference to §5.5, which was
incomplete — the signed escalation carries no proposed `uses`. Adjusted:
the exemption's presented candidate is the signed escalation itself
(escalation id, capability, caveat with action class in the caveat key,
batch `count` and `sample`); **`uses` is resolution-authored human
input**, C1-attributed in the signed approval and verified by §5.5's
exact match — never a pre-presented candidate field; its rendering
fidelity is the C2 broker-owned surface's contract (SI-23's question under
actuation). The exemption's policy context is the immutable
content-addressed capability itself — a hash is a version — so exemption
approvals are policy-stable by construction, and A26's "always includes
versioned policy context" is scoped accordingly. D34-2 is read as revised.

**R6 — evidence corrections (finding 7).** The double-resolution branch of
the consumption view has no test — moved from claimed-covered to a G13
outstanding negative (RF-36's contract lane will exercise it, since gate
binding parity tests the same edges). G13(c) was wrong in the opposite
direction: a per-field `policy_context` negative *exists*
(`w14_malformed_signed_promotion_binding_cannot_reach_trunk` mutates it
alone); corrected to cite it.

**R7 — whole-view anomaly failure is intentional; recovery path filed
(finding 8).** `w14_decision_authority` scans all escalations before
filtering by capability, so one duplicate resolution denies every
capability in the home permanently. Ratified as intentional: a corrupted
authority chain is a home-level integrity incident (the W-19 pattern), and
scoping the view per-capability would let a poisoned chain keep granting
elsewhere. The missing piece — an operator recovery path short of an epoch
action — is filed as **RF-38** (low); if its remedy needs a new record
kind it graduates to an SI per the triage rule.

Finding 9 (stale `AGENTS.md` and roadmap version strings) was mechanical
and corrected directly.

## Post-review adjustments — round 2 (W-20, 2026-07-14 — operator-ratified)

The second review round returned REQUEST CHANGES: seven findings, three
high, all verified at source. Two are protocol additions the operator
examined and ratified explicitly (R9's second recovery artifact, R11's
positional binding); the reviewer's refuting scenarios are preserved as
the record.

**R8 — V gains signed `tool_call.state_root_after` (finding 1).** Round
1's "total" V omitted a real signed root source: unbranched tool calls
capture live stores and advance trunk expectations
(`record_tool_call` — non-`branch:` keys; branched runs capture under
`branch:` keys and never move trunk). Refuting scenario: snapshot attests
`B`; an unbranched tool call moves live to `C` (signed
`state_root_after = C`); a revert toward `B` crashes pre-commit; V says
`B`, the legitimate rollback arm requires `before(=C) == V(=B)`, and the
home bricks. Adjusted: V's source list is stated *intensionally* — every
signed event field attesting a store's live trunk root — and enumerated:
manifest `snapshot`, unbranched `tool_call.state_root_after` (non-
`branch:` keys only; a branch capture never joins trunk V), promotion
`merged`, revert `roots_restored`, drift `observed_root`, the closing
record.

**R9 — the recovery capture record (finding 2; protocol addition).**
Round 1's convergence claim was false across a double crash: the captured
downtime-edit root had no durable name until the post-restore emission
transaction, so a crash between restore and emission left the retry
seeing live == V with nothing to say — the divergence window vanished
unrecorded, violating M8. The forcing chain: evidence must be
ledger-visible (M8); emission before restore poisons V (round 1);
emission after restore has the durability hole. The only remaining
placement is a non-event artifact between capture and restore. Ratified:
the **recovery capture record** — a second fabric-signed, typed,
versioned recovery artifact naming the journal id and per-store captured
roots, published and fsynced after capture, before any restore mutation.
This is the write-ahead principle at its third boundary (journal↔stores
R8-ADR0006, database↔anchor R5-ADR0006, now capture↔restore), and it
crystallizes a two-sided rule: **intentions are write-ahead files;
attestations are write-behind events** (a pre-restore closing event
would attest an incomplete restore — a signed lie in the crash window;
a post-restore-only capture is non-durable). Pinned lifecycle:
**first-write-wins** (a retry never re-captures over an existing record —
the original capture is the evidence and live may be half-restored);
validated by the journal's fail-closed family (signature, type, version,
journal-id linkage, store set ⊆ the journal's) before being consulted;
removed **before** the journal (so "journal present, capture record
absent, closing records committed" is a legible cleanup cell and an
orphaned capture record is unreachable). A committed closing record
naming the journal id is the idempotency marker routing retries to
cleanup. Crash matrix: before the record → clean re-run; after record,
before/during restore → restore idempotently, emit from the record;
after restore, before emission (the refuting cell) → emit from the
record, evidence preserved; after emission → cleanup. Rejected: the
scope-down alternative (ratify R11 only, accept single-crash evidence
durability as a residual) — recovery windows are when crashes are least
rare, and it would ratify knowingly-unsound M8 semantics.

**R10 — exemption approvals bind the presented escalation version;
RF-39 (finding 3).** A9 batching appends a **new signed escalation event
per violation under one numeric id** with evolving `count`/`sample`, so
"the signed escalation" is ambiguous per id; the approval body named only
the numeric id; and the C2 listing surface renders from the mutable
`escalations` table unverified — the RF-31 defect class alive on the
exemption surface (a storage write can show benign samples; even without
tampering, nothing identifies which version the human reviewed).
Adjusted: the approval body gains **`escalation_event`** — the event id
of the exact signed escalation version presented; newer batch versions
never widen the grant (`uses` caps it) and remain visible; C2 listing
surfaces MUST render from, or verify against, the signed escalation
events, never the bare table (the RF-34/W-19 pattern). The round-1
"reservation to C2/SI-23" is narrowed to rendering *fidelity* (pixels vs
named candidate); data-source integrity is RF-39's, filed and carried by
W-22. D34-2/R5 are read as extended.

**R11 — positional freshness (finding 4; protocol addition).** Value
equality answers "same state?"; freshness asks "same *moment*?" — roots
recur (the ABA replay `B→A`, later `A→C→A`, replant: `after == V` passes
and a historical journal drives a restore), positions never do. This is
A23's own doctrine — offsets are order, values are claims — applied to
the last consumer still comparing values. Adjusted: the journal records,
per journaled store, the **event id of the latest root attestation at
prepare time** (well-defined: the journal is built inside the gate lock
after `check_drift`); roll-back requires each recorded position to still
be the latest attestation for its store; roll-forward requires the
linked event itself to be the latest; value agreement is retained as
belt-and-braces. **Pinned check order (normative):** the closing-record
idempotency check runs before the freshness predicate — after emission
the closing record *is* the latest attestation, and a naive positional
check would fail every post-emission retry. Rejected: binding a single
global head offset (works only under today's single-writer lease;
over-constrains under D5's concurrent-writer future; per-store positions
express exactly the journal's dependency and reuse V's computation —
V returns (root, position)). Residual honesty: joint journal+database
rollback still presents as internally consistent — SI-25 layer 2's
freshness question, reservation unchanged.

**R12 — outcome equality re-quantified (finding 5).** Round 1's "same
per-store results" meant whole-store merged-root equality — which
incorporates unrelated concurrent human trunk edits, so any human edit
re-parks every pending candidate: the blanket re-park R2 explicitly
rejects, recreated by quantifier error. Adjusted: equality is over the
**agent-originated applied op-set and the conflict decisions** (the A13
vocabulary — the same objects the preview renders); unrelated trunk-side
changes flow through under M8 as always; opaque stores compare conflict
classification plus installed-image identity (the branch image is
already digest-pinned). D34-3 is read as re-quantified; RF-37's clause
description corrected to match.

**R13 — per-store pairing and the V-preservation invariant (finding 6).**
A drift body is per-store; one singular "closing record" cannot attest a
multi-store tuple. Adjusted: per divergent store, one window drift
ordered before one closing record — **pair-or-neither** (a store whose
capture equals V emits nothing) — all pairs in the single emission
transaction; the closing record carries one additive field
`recovery: <journal id>` (§0 extensibility; the round-1 "zero body-schema
motion" claim softens to "one additive field"). Emergent invariant,
elevated to normative text: **recovery never moves V** — both arms'
restore target equals V by the freshness predicate and each pair nets V
to itself, so the round-1 poisoning is unrepresentable rather than merely
avoided.

Finding 7 (the v0.9 status line still said "adjusted in three places")
was mechanical and corrected directly.

## Post-review adjustments — round 3 (W-20, 2026-07-14 — operator-ratified)

The third round returned REQUEST CHANGES: five findings, two high — both
in the round-2 additions themselves, confirming the fix surface is
narrowing (round 1 broke determinations, round 2 broke the repairs,
round 3 broke only the round-2 material plus evidence bookkeeping). The
reviewer's completeness sweep confirmed no original SI decision point
was silently dropped.

**R14 — element-wise explanation on retry; the guarantee narrowed;
SI-39 filed (finding 1).** R9's first-write-wins — the fix for round 2's
evidence loss — made retries blind to *new* divergence arising during a
crashed recovery's own downtime: capture `C1`, crash mid-restore, human
edits `C2`, retry restores over `C2` with no durable name and no event.
Following the reviewer's narrow-and-file guidance rather than designing
capture generations in-PR: on retry with an existing capture record,
recovery verifies — per store, before any mutation — that **every
element of live state is explained by either the capture record's
version or the restore target's version** (per file for fs stores: a
partial tmp+rename restore leaves each file at exactly one of the two;
whole-image for sqlite: the swap is atomic). All-explained → an innocent
interrupted restore; complete idempotently as ratified. Any unexplained
element → a new divergence window → **fail closed in place**: no
mutation, the new bytes stay live and untouched, the home stays closed,
operator resolution. A24's guarantee is re-scoped honestly: automated
capture-and-attribute covers the divergence present when recovery first
begins; later-window divergence is detected and preserved-by-refusal,
never silently normalized. **SI-39** files the enhancement (automated
multi-window preservation — capture generations / recovery progress),
composing with the RF-34/RF-38 operator-surface pass for the resolution
ceremony. Cost note: the check is hash-walking against two CAS-resident
trees; cheap.

**R15 — capture-record validation is exact-set (finding 2).** Round 2's
"store set ⊆ the journal's" was a reflexive copy of the journal's own
tuple logic and wrong for this artifact: pair-or-neither needs every
journaled store's captured root to *prove* non-divergence, so a record
missing a journaled store cannot make that determination — and
first-write-wins forbids repairing it. Corrected predicate: the capture
record's store set MUST equal the journal's exactly; anything else is
invalid and fails closed (it can only arise from bug or tamper — the
writer captures all journaled stores before publishing once).

**R16 — the R12 comparison basis (finding 3).** For multi-path ops the
two candidate bases disagree: branch-relative ops (`rename(a,b)` still
reads as itself when trunk independently deleted `a`) versus literal
applied delta (`add(b)`). Ratified: **basis-1 with per-path scoping** —
equality is evaluated per previewed op over the op's full touched-path
set (`rename(a,b)` touches `{a, b}`); an op is preserved iff the
re-merged result at its touched paths equals the *previewed merged
result* at those paths (the previewed merged tree is part of the signed
candidate's stores, CAS-resident — the referent is mechanical) and its
conflict status is unchanged; paths touched by no previewed op or
conflict card are trunk's own and free to differ. The reviewer's rename
example correctly proceeds (`a` absent, `b = h` in both merged trees —
the human saw exactly what landed); round 2's refuting case still
correctly re-parks (the conflict card's path resolves differently).
Rejected: the applied-delta basis — it re-parks on trunk changes that
leave the approved outcome untouched, the blanket-re-park failure mode
in different clothes. Opaque stores unchanged from R12 (classification +
pinned installed image).

Finding 4 (the sweep mapped grouped journal-validation claims to
STATE-COMMIT tests that cover only some dimensions) is corrected in the
PR sweep, with the three uncovered dimensions filed as **G13(e)/(f)/(g)**
(invalid journal signature; pre-existing-sibling exclusive publication;
journal-roots-vs-committed-event disagreement) landing with W-15's RF-35
lane. Finding 5 (round-1 wording still controlling in the ADR status,
changelog intro and its V enumeration, and W-22's count/provenance) was
mechanical and corrected directly.

## Post-review adjustments — round 4 (W-20, 2026-07-14 — operator-ratified)

Round 4 returned REQUEST CHANGES: five findings, three high. Its
signature is different from rounds 1–3: two of the highs are **as-built
defects** — merged, dogfooding code failing clauses that round 0 had
certified as faithfully built — rather than holes in the ratified
protocol. That distinction is why the three-category labeling in the
status header exists: "ratified as built" is a per-dimension evidence
claim, not a per-protocol one, and RF-40 is what it looks like when a
dimension was missed.

**R17 — temporal binding at both consumers; RF-36 widened; the R1
record corrected (finding 1).** `w14_decision_authority` preloads every
approval into headroom before the consumption fold and never requires an
approval to precede the tool call that consumes it — so a signed history
containing tool_call-before-approval retro-funds the earlier effect at
decision time, the identical defect R1 recorded for gate replay alone.
The round-1 claim that the gate was weaker than decision time is
corrected: decision time had the binding tuple but not the temporal
edge; **neither consumer had it**. Ratified (the clause already exists —
A25's substrate order and the durable-authorization-offset rule; no new
SI): headroom available at offset `O` counts only approvals at offsets
before `O` — one position-ordered reconstruction, stated once, shared by
decision and gate. RF-36 is widened from gate-only to both consumers,
with a decision-time protected-dispatch negative under
SIGNED-DECISION-AUTH; carried by W-22.

**R18 — R14 re-quantified over path-state (finding 2).** "Every element
of live state" is vacuous for deletions: a path deleted during a crashed
recovery's downtime is not a live element, so the check passed and the
restore recreated the file with no capture and no drift — undetected,
violating R14's own promise and M8. Ratified: the explanation predicate
quantifies over the **union** of paths present in live, capture, and
target, with **absence as a value**; canonical entry identity is
(relative path, presence, content hash, executable mode) — content alone
is not identity; excluded directories and untracked empty directories
are scoped to the canonical store boundary, matching capture semantics.
This is squarely R14's detection duty, not SI-39's automation (SI-39
begins after detection; this window was undetected).

**R19 — RF-40 filed: the fs restore does not realize every signed
target root (finding 3; as-built defect).** Two defects in the merged
`apply_fs_in_place` against clauses ratified as built: (a) pass 2 skips
any file whose **content** hash matches, ignoring mode — a mode-only
transition commits a signed event naming a root (mode is part of the
root hash) that live state does not realize: the authoritative-name
claim of D31-1 silently broken; (b) pass 2's rename-over-target runs
before pass 3's directory pruning, so a file↔directory topology swap
fails deterministically and every recovery retry re-fails — a
legitimate recovery bricks (fail-closed, but wrongly). Mechanism-class
under existing ratified clauses (D31-1 exact realization, D31-3
idempotent recovery), not SI-32 attacker-model questions: filed as
**RF-40** (high), carried by W-15 alongside RF-35 (same files, same
lanes), with registered STATE-COMMIT negatives for mode-only exactness
and both file↔directory crash-recovery directions. RF-40 may constitute
the parked RF-27 recovery-hardening trigger; un-parking is an operator
gate decision, noted there and not made here. The spec's D31-1 text
gains the canonical-entry realization sentence; the G9 committed-tuple
row is qualified until the negatives land.

**R20 — R16's referent made real (finding 4).** R16 claimed the
previewed merged tree is "part of the signed candidate's stores,
CAS-resident" — it is not: the parked preview serializes only
`ops`/`conflicts`/`trace_check`/`branch_roots`, and the composed merged
root is discarded. Ratified: parked previews gain **per-store `merged`
root references, CAS-retained at escalation** — entering
`candidate_digest` by construction (the digest hashes the whole
preview), restoring the direct referent R16 intended and letting the
approval surface render from it. Rejected: deriving the expected
touched-path result at approval time from the signed manifest, branch
roots, and conflict cards — workable in principle, but re-deriving the
merge at approval reintroduces recomputation ambiguity, the very thing
the comparison exists to check. The opaque-store subcase stands as
reviewed (branch image pinned by `branch_roots`; trunk-wins image
CAS-captured in the conflict card). RF-37/W-22 implements.

Finding 5 (two sweep rows claimed `VerifiedPrefix`/local-terminal
enforcement against code that reads `verified_events`) is corrected in
the PR sweep: "structural logic present; A23 terminal/prefix enforcement
pending W-15" — matching the treatment already used for home/epoch
binding.

## Implementation deltas (RF-35/RF-40 → W-15; RF-36/RF-37/RF-39 → W-22)

1. D31-4 freshness predicate as adjusted through round 2: total V
   including unbranched `tool_call.state_root_after` (R8), positional
   per-store binding (R11), journal-store-set projection — in
   `recover_pending_state_change` (same function W-15 already upgrades to
   the `VerifiedPrefix` per S4). [RF-35 / W-15]
2. D31-6 as adjusted through round 4: CAS capture + the recovery capture
   record before restore (R9 — first-write-wins; exact-set validation
   per R15); the R14/R18 explanation check on retry — path-state over
   the union of live/capture/target, absence as a value,
   canonical-entry identity — with fail-closed-in-place on unexplained
   state; per-store window-drift/closing pairs with `recovery` linkage
   in one transaction after restore (R13); pinned removal and check
   orders; restore-skip when live == V. [RF-35 / W-15]
3. Journal `home`/`epoch` (TracePosition) binding with the R3 per-arm
   guard, plus the R11 per-store prior-attestation positions. [RF-35 /
   W-15]
4. Binding parity at both consumers (R1/R17): one position-ordered
   reconstruction — exact-match binding plus the temporal edge
   (approvals fund only later offsets) — shared by decision time and
   gate replay. [RF-36 / W-22]
5. Re-merge outcome-equality check, R12 quantifier on the R16 basis,
   with the R20 referent (previews gain CAS-retained per-store `merged`
   roots, digest-covered) and re-park-on-difference (R2). [RF-37 / W-22]
6. Exemption candidate binding (R10): `escalation_event` in the approval
   body; C2 listings render from or verify against signed escalation
   events. [RF-39 / W-22]
7. Exact canonical-entry realization in the fs apply (mode-only writes;
   file↔directory topology ordering) with mode-exactness and
   both-direction topology crash negatives. [RF-40 / W-15]

Every remaining clause of A24–A26 is enforced by the merged W-14
implementation, mapped line-by-line in the ratification PR's corrected G9
conformance sweep.
