# Spec issues — ambiguities the code forced

> **v0.4 (2026-07-09): every v0.3-era issue (SI-1…SI-19) is RESOLVED** —
> ratified individually (SI-11, SI-17, SI-19, F2/SI-6) or via the
> adjudicated A19 conformance sweep, and integrated into
> `asf-schema-spec.md` v0.4 (changelog A15–A19). This file is preserved
> as the amendment provenance record; per-entry statuses below are
> historical. New issues found under v0.4 start at **SI-20** (resolved in
> v0.5 as A20); SI-21 is resolved in **v0.6 as A21**; SI-22 is
> interpreted (W-2); SI-24 is resolved in **v0.7 as A22** (implementation
> is W-8); SI-23 remains open. The 2026-07-12 cryptographic mechanism
> audit filed **SI-25…SI-30**. The 2026-07-13 spec-cohesion analysis of
> the PR #43 (W-14) review cycle filed **SI-31…SI-34** — protocol
> questions that cycle answered in code without ratification, re-filed
> at the spec layer (W-20 batches their ratification; PR #43's required
> independent re-review uses SI-31/SI-33/SI-34 as its oracle). **SI-35**
> is the workboard domain-label question, recovered from
> `codex/workboard-dogfood` (drafted there as SI-22 before main assigned
> that number) at its W-21 revival decision. **SI-36** is claimed by the
> in-flight amendment-scope filing (PR #46, not yet merged); **SI-37**
> (TracePosition agreement predicate) was filed 2026-07-13 by the second
> independent review of PR #47; new issues start at **SI-38**.

Tracked per the handoff: where the spec is ambiguous or contradicts itself,
we record the question, the interpretation the kernel implements, and why —
we do not silently pick. Each issue cites spec § and the implementing file.
Settled decisions (F1, F4, fail-open) are not re-litigated here.

Status legend: **open** = needs a spec amendment or an explicit "fine as
interpreted" from the author; **interpreted** = kernel picked a reading and
tests encode it; flipping the reading is cheap.

---

## SI-37 — TracePosition fields have no agreement predicate with the event they name (§6.2, A23) — open

`TracePosition { home, epoch, global_seq, event }` (A23) denormalizes three
signed event fields beside an event id. No normative clause requires a
consumer to verify that the named event's signed `home`, `epoch`, and
`global_seq` equal the position's other three fields, so a signed object can
carry an internally inconsistent position — its signature attests the
*claim*, not the *agreement*. Surfaced by the second independent-context
review of PR #47 (2026-07-13), which correctly declined to infer the
predicate silently.

**Candidate resolution (the W-11 pattern; flipping is cheap):** a verifier
consuming a `TracePosition` MUST resolve `event` within the `VerifiedPrefix`
and require exact agreement of the three denormalized fields; any mismatch
fails closed (doubt never widens) — W-11's signed/materialized
selector-agreement rule applied to the new denormalized tuple. Natural
ratification companion to SI-26's transcript decision at W-20's next touch;
W-15 implementation should enforce it from day one regardless of when the
amendment text lands (fail-closed is the conservative free default).

## SI-35 — workboard domain labels precede domain-taxonomy governance (§4, brief §10.2) — open

*Recovered 2026-07-13 from `codex/workboard-dogfood`, where it was drafted
2026-07-10 as that branch's SI-22 — a number main had already assigned to
the gate-clock issue (the same race SI-23's renumbering note records).
Refiled at the W-21 revival decision with one sharpening (the W-3
clustering note below); implementation references describe the branch and
revalidate at rebase.*

The second dogfood profile must declare a domain for every registered action,
but the brief deliberately leaves fabric-owned domain-taxonomy governance open.
Using the existing `files.vault` label for structured work would flatten two
dissimilar workloads; inventing globally authoritative roots in the
implementation would silently settle the governance problem.

**Branch implementation** (`proxy.rs` on `codex/workboard-dogfood`): the
trusted, first-party workboard profile provisionally labels SQLite task
actions `work.tracking` and Markdown evidence actions `work.evidence`. These
strings make registration and domain-scoped traces honest, but they are not
ratified taxonomy roots and MUST NOT be treated as portable trust domains or
founding precedent for third-party registrations. No StandingRules or
TrustRecords consume them yet, so renaming before that machinery lands is
cheap — but W-3 clusters approvals by (caveat, action-class, **domain**), so
the labels become load-bearing as soon as workboard approvals feed
`asf rules candidates`, well before trust compilation: resolve or explicitly
bless the provisional labels first.

**Open question for the spec/design:** what fabric-owned root taxonomy and
extension process should registrations use, and should evidence inherit the
work item's domain or remain a separate domain? Resolve before these labels
feed portable rules, trust compilation, or published conformance artifacts.

## SI-34 — what exactly does a human approval bind to? (§5.3, §6, C1) — open

C1 gives approvals provenance (channel + auth_strength) and the §6
`approval` body names its escalation or promotion, but no clause states
what an approval must bind to — the identity of the exact artifact the
human reviewed. RF-31 (PR #43) showed the consequence: a parked
promotion's signed escalation named only a numeric promotion id while the
mutable `promotions` row supplied the manifest and preview, so swapping
two valid pending rows redirected a human approval to a different branch
than the one displayed. Nothing was forged; the approval was simply never
bound to its object.

Decide the general rule once — an approval is valid only for the exact
signed candidate presented (what-you-see-is-what-you-approve) — so every
approval surface inherits it instead of each consumer re-deriving it:
which fields constitute candidate identity per approval type (promotion:
preview + branch-root tuple + manifest + policy context; escalation
exemption: caveat + action class + uses; future rule ratification: rule +
counterfactuals shown, R1's informed-consent lineage); whether §6
escalation/approval bodies gain normative digest fields or the binding
stays implementation-internal; and how policy-context versioning behaves
(an approval granted under promotion policy N must not silently apply
under N+1).

**W-14 candidate (implemented in PR #43, not a resolution):** the
escalation signs promotion id, manifest, canonical digest of the exact
preview, digest of the branch-root tuple, and versioned policy context;
approval recomputes and verifies that still-unresolved binding before
auth-strength, drift, or merge work; the signed approval repeats the
candidate digest and commits atomically with the promotion. Ratify,
adjust, or supersede at W-20.

## SI-33 — what is authoritative for consumed authority: meters, exemptions, approval headroom? (§5.1, §6, A9) — open

§5.1 calls budgets "broker-metered" and A9/SI-13 give approvals `uses`,
but the spec never states where consumed quantity lives or what a
decision may trust: runtime tables or the signed substrate. RF-29
(PR #43) showed unsigned `broker_meters`/`exemptions` rows authorizing
dispatch — a storage writer could reset a consumed meter or inject an
exemption and receive an Allowed ticket, with gate replay too late to
stop credential injection or the downstream effect.

The doctrine to ratify is the A15/A22 materialized-view rule extended
from existence to consumption: consumable authority is a pure function of
the verified event prefix plus explicitly declared in-flight
reservations; unsigned rows are caches, never authorization inputs. To
pin down: the reservation object itself (in-memory pending dispatch
today; the durable external-effect protocol's reservation record before
first egress — P22); the exact-match binding of an approval to its prior
signed escalation (which fields; duplicate and zero-use behavior); the
transaction boundary (approval event and cache update share one commit);
and composition with RF-3's accepted decision-time-consumption residual
and the gate's ledger-recount backstop. Forward-looking: W-3's rule
health counters and TrustRecord evidence are the same doctrine's next
consumers — ratify it once, here.

**W-14 candidate (implemented in PR #43, not a resolution):** decision
meters and exemption headroom reconstruct from verified signed
tool_call/escalation/approval events plus pending reservations; the two
tables are demoted to compatibility caches never read for authorization;
signer-anomalous capability, caveat, manifest, auth-strength, zero-use,
duplicate-binding, and cross-capability edges fail closed.

## SI-32 — store publication has no defined filesystem attacker or required OS primitives (§5.3, §9 F2) — open

The spec assumes content-addressed preparation and coherent restore but
never defines the filesystem adversary those operations run against.
RF-20's remediation (PR #43) implemented a publication discipline —
retained verified bytes, exclusive randomized no-follow siblings, rehash
through the retained handle, same-inode non-symlink verification
immediately before rename, fsync of file and parent — without a written
threat model saying what those steps must defeat, so each review
re-derives the attacker and finds a new residue.

Define the attacker in tiers and label every guarantee with the tier it
holds under: (1) offline storage tampering between processes — CAS,
branch, or database bytes changed while nothing runs (the same-uid
dogfooding concern, and the backup/restore case); (2) an active
same-privilege writer holding descriptors or hardlinks across
prepare→publish — where no sequence of pathname checks can win and the
honest answer is OS-enforced exclusion or containment topology (P7/W-4,
G-ADVERSARIAL); (3) the legitimate concurrent human edit, which is not an
attack and must land in M8 drift attribution rather than corruption or
silent loss. State the staged-bytes rule as normative (commit consumes
retained verified bytes and never re-reads mutable storage after
verification); the symlink/hardlink/directory-entry rules per store kind
(fs tree vs. SQLite file); and which publication-safety claims require
containment before G-PUBLISH.

## SI-31 — owned-state transition: "atomically" has no commit point, journal semantics, or crash matrix (§5.3) — open

§5.3 makes promotion the only mutation and has revert restore all roots
"atomically", and A20/M8 serialize gates — but no clause defines the
operational protocol that makes those words true: the commit point, the
record that authoritatively names the committed root tuple, journal
freshness, retry identity, rollback-failure behavior, or any crash cell.
RF-30 (PR #43) found promotion/revert could partially commit or mutate
live state without its signed event; the W-14 remediation designed the
missing protocol inside the PR. That protocol is normative semantics — it
defines what "atomic" and "recorded" mean — and must be ratified, not
inherited from an implementation.

To ratify (W-14's candidate in parentheses, implemented in PR #43): the
state machine (prepare forward and rollback images for every root; stage
the signed event, expected roots, promotion status, and companion
approval in one uncommitted SQLite transaction; publish a fabric-signed
recovery journal; apply and fsync stores; commit the database); the
commit point (the database commit, with the linked verified event as the
authoritative name of the committed root tuple); recovery semantics
(reopen rolls back when the journal's linked verified event is absent,
rolls forward when it is present and the root tuple matches exactly;
mistyped, wrong-version, symlink, directory, and event/manifest-misbound
journals are rejected before any restore); ordinary-failure behavior
(every before-root restored and fsynced, database rolled back); and the
journal's status (W-14 keeps `state-change.pending.json`
implementation-private rather than a §6 event kind — ratify or amend that
choice). Owned here or by composition at W-20: journal freshness against
database/home rollback (a replayed old journal plus an old database is
SI-25's freshness question); rollback-failure-during-recovery outcomes;
and the boundary with the parked durable external-effect protocol (this
issue is Tier-1-local only). The hard-exit-at-each-syscall matrix remains
G3 evidence work under any ratified shape.

## SI-30 — salted redaction commitments have no canonical construction or reveal semantics (§1, §8.4) — open

The spec writes `sha256:salt‖value` but does not define salt entropy or length,
the canonical encoding of `value`, domain/field/object binding, unambiguous
concatenation, where the salt lives, whether it is later revealed or destroyed,
or what an opening proves. These choices determine whether the construction is
binding, privacy-preserving for low-entropy values, and interoperable. No
implementation may silently choose them. This issue is specification-only;
redaction commitments are not implemented today.

## SI-29 — can identical content return after its payload hash is shredded? (§1, §8.4) — open

Payload identity is currently the plaintext SHA-256. A shredded row and
tombstone persist structurally, so re-storing identical bytes returns the
permanently unreadable reference. The spec's "destroys content everywhere at
once" can support that permanent-hash reading, but later ingestion could also
be understood as a new payload generation with a new DEK. Multi-owner
retention makes the distinction load-bearing. Decide whether hashes are
permanently poisoned, generation-qualified, or represented by another explicit
lifecycle. This issue does not resolve the known multi-actor visibility-policy
problem.

## SI-28 — what exactly does the payload AEAD envelope authenticate? (§1, §8.2) — open

A19/SI-9 names AES-256-GCM but does not define envelope versioning, associated
data, algorithm dispatch, KEK identifiers, multiple owner wraps, or the nonce
and invocation lifecycle of a long-lived KEK. RF-22/P19 show that these fields
cannot remain documentary: hash, size, media type, DEK id, KEK id, algorithm,
home/tenant context, purpose, and format version need an unambiguous binding or
an explicit reason for exclusion. Decide the canonical transcript and migration
rule before changing stored ciphertext. Multi-actor visibility policy remains
open; this issue reserves a compatible mechanism without choosing that policy.

## SI-27 — fabric identity initialization, rotation, recovery, and historical verification are unspecified (§8.1, §8.2) — open

The hierarchy names user, fabric, agent, and owner keys but not how a home is
first anchored, how reopening distinguishes missing keys from first use, which
public identity a verifier trusts, how rotation is certified, how historical
records remain verifiable, or how loss and compromise differ. RF-18 proves
that `load-or-create` is unsafe for an existing home; RF-14 shows that custody
is also a production boundary. The immediate implementation may fail closed on
missing keys without deciding the larger lifecycle, but rotation/custody claims
wait for this amendment.

**Immediate W-13 boundary (implemented in PR #40, not a resolution):** new-home
initialization is explicit; an existing or partial home with missing or
malformed fabric, user-root, or owner-KEK material fails closed without
replacement. This supplies no trust anchor, rotation certificate, recovery
ceremony, custody improvement, or historical-key registry; all of those remain
the unresolved question here and gate W-17.

## SI-26 — the signed transcript does not bind object type or protocol domain (§0, §8) — open

The spec explicitly signs JCS of the body excluding `id` and `sig`; the type
prefix is outside those bytes. RF-6 therefore cannot be fixed interoperably by
simply prepending the prefix: that would change the normative transcript and
every existing signature. The same fabric key signs several object classes,
so decide a versioned type/domain transcript (or a signed type field), expected
prefix verification, migration of existing objects, and whether distinct role
keys also become mandatory. This must resolve before W-6 publishes fixtures.

## SI-25 — what authenticates global substrate order, completeness, and freshness? (§3, §3.1, §5.4, §6) — RESOLVED (author, 2026-07-13)

**Resolution: ratified as amendment A23 (spec v0.8, new §6.2) — a two-layer authenticated global order.** Layer 1 (signed per-home `global_seq`/`global_prev` on every event, plus the local signed `TraceCheckpoint` at every profile) is normative now and closes RF-13/RF-16's order + completeness-between-events with pure local cryptography, making the "verified substrate prefix" a mechanically available `VerifiedPrefix`. Layer 2 (publication of layer 1's checkpoints to an external monotonic `AnchorStore`) adds freshness against rollback and is graduation-gated; unanchored homes run at the `local-integrity` assurance label. Ratified in the W-20 session via challenge pass → determinations **D1–D7** + durability seam **S1–S5** (recorded in ADR 0006's addendum, provenance-preserved). Key determinations: the anchor is an interface (remote shared head reference for the roaming design center, TPM a single-machine fast-path); only irreversible-external-effect dispatch anchors synchronously (D4), so nothing waits on the network before first egress; the writer fence is a lease so concurrent writers are non-foreclosed (D5); the owned-state transition (§5.3/SI-31) shares one commit point (S1–S5) with a posture-scoped durable-commit profile (WAL+`NORMAL` dogfooding → `FULL` production); standing authority counts over the `VerifiedPrefix`, local-integrity single-machine and re-earned at graduation (D2); migration invalidates all pre-migration authority (new epoch, no re-signing). New posture gates G-ROAMING-SURFACE/G-ROAMING-WRITE filed. Reserved to owning issues: SI-26 (transcript), SI-27 (epoch key rotation authorization), W-6 (import/recovery vocabulary), the durable external-effect protocol (dispatch ordering). W-15 carries implementation; RF-13/P15's order/completeness half closes with layer 1, its freshness half when layer 2 lands (coordination at G-ROAMING-SURFACE, synchronous dispatch at G-EGRESS, freshness authority at G-PRODUCTION). The independent-context review of the drafted text (PR #47) returned REQUEST CHANGES — four encoding defects, no design objections — corrected same day with four operator-ratified post-review adjustments (**R1–R4** in the ADR addendum: H9's clause split, the "interior" qualifier, the pre-epoch-grant MUST-refuse, drift epoch-genesis clamping); re-review pending. Original analysis and the awaiting-ratification candidate below, preserved as provenance.

---

### SI-25 (original filing) — what authenticates global substrate order, completeness, and freshness? (§3, §3.1, §5.4, §6)

Per-span signed chains authenticate records within the rows a verifier sees,
but the global substrate offset is unsigned and no expected head detects tail
truncation, whole-span deletion, or database rollback. A22 liveness, M7 grant
ordering, approval headroom, `captured_before`, and drift windows all consume
that order. RF-13 and RF-16 demonstrate that "verified substrate prefix" is
not presently a mechanically available object.

Choose the normative representation: one per-home global signed chain,
periodic signed checkpoints committing per-span heads and global order, an
external/witness anchor, or another construction. Specify home/epoch binding,
export ordering, rollback detection, checkpoint recovery, and the relationship
to the durable external-effect dispatch record. Do not encode an anchor design
in code until ratified; RF-16's immediate verified-row fix is compatible with
all candidates.

**Candidate awaiting ratification (ADR 0006, 2026-07-12):** add one signed
per-home global predecessor/sequence to every new event, checkpoint its
terminal head, and advance a monotonic anchor outside the fabric home's
rollback domain. The candidate explicitly limits an unanchored local signed
head to retained-row order/integrity: it cannot prove freshness after
full-disk rollback. Production and standing-authority claims therefore
require a qualifying remote witness or hardware monotonic anchor. The ADR
covers append/publication crashes, home/epoch and export binding, old-image
recovery, durable dispatch ordering, legacy migration, key-history seams,
performance/DoS, and the G9/G10 test plan. SI-25 remains **open**; no
candidate field or anchor policy is normative until the human choices listed
in the ADR are ratified and integrated into the spec. Provenance note
(2026-07-13): this ADR and a companion implementation sketch were recovered
from `agent/si25-authenticated-head-design`, an unpushed local branch based
before W-12/W-13/W-19/W-14 — the ADR is W-20's ratification input; the
sketch needs rebase before W-15 implementation.

**Ratification in progress (W-20, 2026-07-13):** the challenge pass has
produced operator-ratified determinations D1–D7 — recorded in ADR 0006's
"Ratification-session determinations" addendum (A22-adjustment style). Spine:
the two-layer construction is accepted with **layer 1 (signed global chain +
local checkpoints) normative for W-15 now** and **layer 2 (external monotonic
anchor) deferred to a graduation gate**; the design center is corrected to
**one human / one home / multiple roaming control surfaces over a stable
always-on base**; only irreversible-external-effect dispatch is
synchronous-anchor-gated (everything else async-loud-degraded); the writer
fence is a lease abstraction from day one so concurrent writers are a
non-foreclosed required future; and two new gates (G-ROAMING-SURFACE near,
G-ROAMING-WRITE future) are proposed. The **SI-25 × SI-31
durability seam** (choice #9, the first composition-review seam) is now
**resolved** (determinations S1–S5 in the ADR addendum): SI-31 is a strict
extension of SI-25's event append over one shared SQLite commit point; the
recovery journal is a layer-1 artifact and the anchor-outbox a layer-2 one;
SI-31 recovery consults the verified prefix whose extent SI-25 defines; one
posture-scoped durable-commit profile (grounded finding: merged W-14 runs
WAL+`synchronous=NORMAL`, process-crash-atomic but not power-loss-atomic — the
dogfooding level; production upgrades to `synchronous=FULL` at G-PRODUCTION,
carried by W-15). **SI-25 is therefore ready to resolve as A23**, requiring zero
change to the merged W-14 code. Sibling-coordinated items that do not block
A23's core remain: SI-26 transcript, SI-27 rotation, W-6 export vocabulary.
(Historical pointer: SI-25 was resolved as A23 the same day — see the
resolution block at the head of this entry.)

---

## SI-24 — capabilities are called revocable but have no early-closure semantics (§5, §6, §9 F1) — RESOLVED (author, 2026-07-12)

**Resolution: the event-derived candidate ratified as amendment A22 (spec v0.7, new §5.4) — permanent, prospective, descendant-closing via the ancestry view — with eight adjustments from the ratification challenge pass.**

- Core, as candidated: `revoke` joins §6 as the signed dual of `grant`;
  current validity is a materialized view of signed objects plus the
  verified substrate prefix; liveness = earliest verified grant before
  the operation, no revoke of the capability *or any ancestor* before
  it, ordinary caveats/expiry at the SI-22 clock, fail-closed
  verification; prospective, no backdating; permanent per id;
  restoration = new mint (new `issued_at` ⇒ new id) + grant, with the
  broker refusing to grant a closed id so a timestamp-colliding
  identical-body re-mint fails loudly instead of silently issuing a
  dead token.
- **Adjustment 1 — `cascade` field dropped.** Descendant closure is
  definitional (the ancestry quantifier in the liveness rule), never
  enumerative; enforcement reads no field, and a one-legal-value field
  enforcement ignores is a lie surface (the SI-10/A21 field-vs-event
  lesson). Blast-radius display is a derived CLI view, not event body.
- **Adjustment 2 — `source` field dropped.** Provenance derives from
  `channel` per the C1 approval-event pattern: `channel: null` ⟺
  broker-mechanical (which then requires a mechanical `reason`, never
  `operator_request`); human-originated requires `channel` +
  `auth_strength`. A separate `source` could contradict `channel` and
  would need a precedence rule for zero gain.
- **Adjustment 3 — doubt-never-widens, stated for both edges.**
  Activation doubt → not granted (A21 already). Closure doubt → not
  live: a signature-verified revoke with an anomalous body (e.g. wrong
  `manifest` field) still closes its named target and descendants,
  loudly — the candidate's "wrong-manifest events inert" would make the
  kill switch *silently fail*, the worst outcome; there is no
  escalation risk in honoring closure (forging the event requires the
  broker key, which mints well-formed events anyway). "Inert" in the
  evidence matrix is re-scoped to: unsigned rows move nothing (the view
  is event-derived); revoking `C` never touches capabilities outside
  `C`'s subtree; and a revoke naming an id no capability bears closes
  nothing *currently* while permanently poisoning that id per the
  condition-2 quantifier — reconciled explicitly in §5.4 so permanence
  and "closes nothing" cannot be read as contradicting.
- **Adjustment 4 — the liveness clock is the durable authorization
  offset**: the first signed event committing the fabric to the
  operation — today the `tool_call` event itself, later the durable
  external-effect protocol's dispatch record. The wedge's
  strand-at-revoke becomes a derived consequence, not a special case;
  decision time enforces the same pure view at the current verified
  head (that is what denies a post-revoke call before any effect); the
  gate's re-evaluation at `O` is authoritative for durability.
- **Adjustment 5 — ancestry verification bounded**: per hop — ancestor
  signature verifies, earliest grants are well-ordered (ancestor's
  precedes child's), no ancestor revoke before `O` (condition 2). The
  view resolves revokes by capability id across the whole substrate,
  never filtered by the evaluating manifest — a child bound to a
  hermetic sub-agent manifest (M2) still dies with its ancestor's
  revoke; grant scans are manifest-scoped (`m7_grant_offsets`) and
  copying that pattern for revokes would silently miss cross-manifest
  ancestry. §5.2 subset semantics stay mint-time-enforced and
  F1-signature-protected (a
  widened child cannot exist without the broker key); the gate MAY
  re-derive them but liveness does not require it. Keeps W-8 scoped.
- **Adjustment 6 — approval/promotion asymmetry made explicit.**
  Closure denials are structural and never escalatable (an escalatable
  closure is an un-revoke lever inside the agent's loop, against C2's
  spirit); pending escalations become inert; **parked promotions of
  pre-revoke work remain approvable** (non-retroactivity — promotion
  ratifies past recorded work; the merge is the human's act); recovery
  is never blocked — revert unaffected, compensation runs under fresh
  narrow mints, never resurrected authority.
- **Adjustment 7 — the `expiry` event kind is removed** (the
  candidate's "if retained" decided in the negative): never emitted, no
  body, no enforcement semantics; keeping it invites
  absence-read-as-liveness and presence-read-as-enforcement. The signed
  `expires_at` is the sole time closure; an observational kind can
  return by amendment if UX ever needs one. (Code-side `EVENT_KINDS`
  updates with W-8, per the file-first/implement-after-ratification
  rule.)
- **Adjustment 8 — closure is structural, never a caveat dimension** —
  pinned mechanically by the W-9 verdict-invariance contract:
  `asf corpus replay` calls the pure evaluator with synthetic
  capabilities and no substrate (`corpus/replay.rs` mirrors
  `broker::propose_call` structurally); closure-as-caveat or
  evaluator-embedded liveness would either change corpus verdicts or
  force substrate-awareness into the harness. `capability_state_at`
  sits in front of caveat evaluation, shared verbatim by decision time
  and gate replay (the W-8 constraint), so revocation-free corpora
  reproduce `docs/baselines/w9-2026-07-12/` bit-for-bit under its
  recorded pins.
- Boundary confirmations: SI-23 owns the kill-switch *surface* (this
  amendment supplies the *operation*: one revoke per live root,
  subtrees close definitionally); P22 owns durable dispatch; P10 is
  constrained (stale/unknown revocation state never proves liveness);
  RF-13/P15 remain the authority-resurrection release gates; W-3
  standing authority and any live egress/actuation wait for W-8's
  implementation. P25 stays open as an implementation gap until W-8
  lands.
- Provenance: filed 2026-07-11 (key-destruction vs authority-revocation
  separation); challenged and ratified 2026-07-12 — cascade semantics,
  revocation authority (request vs sign), the in-flight-branch
  question, and approval/exemption interaction each pressed; the eight
  adjustments above are the deltas. Original analysis below, preserved
  as provenance.

Surfaced while separating key destruction from authority revocation
(2026-07-11). F1 calls broker-minted capabilities "central, revocable,
meterable," but the normative machinery supplies only:

- mandatory `expires_at` (mechanically enforced);
- attenuation into a narrower child (which leaves the parent live);
- a signed `grant` event as the activation edge (A21/M7); and
- an `expiry` event kind with no body or enforcement semantics.

There is no early-revocation event, no rule for descendant capabilities,
no authority for requesting revocation, and no answer for calls authorized
before revocation but recorded afterward. The implementation matches the
gap: it enforces timestamps and grant-before-effect, but carries no revoke
API/state/replay, never emits `expiry`, and leaves approved exemptions and
in-memory allowed-call tickets usable independently of later authority
changes.

This cannot be silently interpreted in code. It adds a signed event kind,
changes M7's definition of "live capability," and becomes a consumer of the
same substrate ordering whose integrity RF-13 already gates before
publication/production.

### Candidate resolution — activation and closure are event-derived

Add `revoke` to §6. A capability object remains immutable; current validity is
a materialized view of signed objects plus the verified substrate prefix.

```json
{
  "kind": "revoke",
  "manifest": "man:…",
  "body": {
    "capability": "cap:…",
    "reason": "operator_request | compromise | behavior_change | tool_disabled",
    "cascade": "descendants",
    "source": "human | broker",
    "channel": "chan:… | null",
    "auth_strength": "… | null"
  }
}
```

The event belongs to the fabric-lifetime span with `manifest` equal to the
target capability's `bound_manifest`, mirroring `grant`. Human-originated
revocations carry registered `channel` + `auth_strength` per C1; because
revocation only narrows authority, any registered human channel may request
it under §3.1's directionality rule. The broker may emit an emergency
revocation for a mechanically established structural cause, with `source`
and `reason` ledger-visible. The agent/holder has no authority to forge a
human revocation through its work channel; voluntary surrender can be added
later as a separately attributed broker request if it proves useful.

The event offset is the effective point — no backdating field. For an
operation durably authorized at substrate offset `O`, capability `C` is live
iff:

1. a verified grant of `C` exists at `G < O` for the bound manifest;
2. neither `C` nor any ancestor in its verified attenuation chain has a
   verified revoke event at `R < O`;
3. `C.expires_at` and every ordinary caveat admit the operation at its
   signed event time; and
4. the object, grant, parent chain, and closure events verify fail-closed.

Revocation is prospective: effects durably authorized before `R` remain
historically valid. It is permanent for that capability id; a later `grant`
cannot reactivate the same id. Restoration means minting a new capability
(an `issued_at` strictly later than the revoked capability's, therefore a new
content id) and emitting a new grant.

Revoking any capability closes its entire descendant subtree; revoking a
child leaves its parent and siblings live. The signed `parent` field plus
grant body already carry the required lineage, but gate replay must begin
verifying that ancestry rather than treating only the leaf as sufficient.
Mint/attenuation must apply the same view at the new grant's offset: a closed
parent cannot produce a live child, even if an object row and grant event are
injected afterward.

`expires_at` remains the load-bearing automatic closure. An `expiry` event,
if retained, is an optional observation `{capability, expired_at}` for
querying/UX; it cannot extend, shorten, or reactivate the signed timestamp and
is not required for enforcement. Early operator/security closure is always a
`revoke` event so provenance and intent are not conflated with time passing.

### Consequences for current broker state

- Revocation dominates approvals, exemptions, meters, and caveats: none can
  resurrect closed authority. Pending escalations become visible-but-inert
  (a later resolution is historical evidence, never authority); unused
  exemptions need not be destructively deleted because replay ignores them.
- Calls not yet durably authorized are denied. Completed effects are not
  undone — owned state uses revert, external effects use compensation, and
  irreversible effects remain honestly reported.
- Today `propose_call → in-memory ticket → downstream effect → tool_call`
  has no durable pre-effect authorization record. In the current local-only
  wedge, a result recorded after revocation may conservatively fail promotion.
  Before the first live external effect, the parked durable-effect protocol
  must supply the real linearization point:
  `effect_intent → authority reservation → dispatch → effect receipt`.
  Revocation blocks calls before signed dispatch; it cannot recall a dispatch
  that already crossed the external boundary.
- `on_broker_outage` may never treat unknown or stale revocation state as
  proof that a capability is live. Any future fail-open path therefore needs
  a verified authority snapshot plus a signed revocation/trace high-water
  mark and a bounded freshness policy; destructive, actuation, and live-egress
  authority should remain non-invertibly fail-closed when freshness is
  unknown.
- Tail truncation or restoration of an old `fabric.db` can otherwise erase
  the latest revoke and resurrect authority. RF-13 trace-head anchoring and
  restore rollback detection are production/distribution prerequisites for
  revocation, not merely audit polish.

### Posture-ledger crosswalk

W-8 directly closes **P25**, the docs↔implementation gap this review added to
`posture-assumptions.md`: F1 says revocable while runtime authority has only
expiry. It does not absorb the neighboring posture gaps:

- **P22** supplies the durable dispatch boundary for live external effects;
  SI-24 defines which side of that boundary revocation governs but W-8 alone
  does not make in-flight effects durable.
- **P10** still owns broker-outage behavior; SI-24 constrains it by forbidding
  stale/unknown revocation state from proving liveness.
- **P15 / RF-13** still own signed cross-span ordering, tail anchoring, and
  rollback detection; revocation makes their absence an authority-resurrection
  risk rather than fixing them.
- **P20** still owns per-session context and same-home concurrency. W-8 must
  serialize dispatch vs revoke independently of mutable home-global session
  pointers.
- **P21** still owns cross-host fencing. Distributed brokers need a shared
  revocation high-water mark and may not dispatch from a stale partition.
- **P3 / P4 / P5 / P12** still own proof-of-human and actuation-safe approval
  surfaces under SI-23. Revocation supplies the kill-switch operation, not an
  unreachable/authenticated surface on which to invoke it.
- **P6 / P8** may trigger tool- or behavior-driven revocation once their own
  attestation/containment work exists; revocation does not verify tool metadata
  or behavior identity.

### Adjacent lifecycles — reserve the rule, do not overload the event

The shared constitutional rule is temporal: authority-bearing objects are
immutable, and activation/closure are signed, prospective, historically
non-destructive transitions evaluated at the operation's authorization
offset. Target-specific lifecycles still need distinct semantics:

- **StandingRules / StandingIntents (W-3):** archival stops future minting or
  scheduled runs; invalidating already-minted capabilities requires explicit
  capability revocations. TrustRecords remain evidence — disputable or
  supersedable, never erased as if they had granted authority.
- **Behavior pins / containment (W-4):** a pin miss may suspend a rule or
  trigger capability revocation, but those are separate declared effects;
  "behavior changed" must not silently mean both.
- **Actuation (SI-23):** C7's no-lockout path depends on this mechanism; the
  operator kill switch must revoke actuation subtrees through a surface the
  agent cannot actuate.
- **Principals, channels, tools, and signing keys:** later disable/rotation
  events must preserve signatures that were valid before closure while
  denying new authority afterward. Tool replay uses the registration state
  valid at dispatch, not today's mutable view.
- **Credentials:** capability revocation stops broker injection; issuer-side
  token invalidation/rotation is a separate external effect with its own
  receipt.
- **Payload keys:** `shred` destroys information access; it is not authority
  revocation. **Revert**, **compensate**, **supersede**, **expire**, and
  **revoke** likewise remain distinct verbs.
- **DelegationEnvelope / conformance (W-6):** portable authority includes
  relevant grant, revoke/expiry, ancestry, and a signed `valid_as_of` trace
  head. A grant-only envelope proves authority once existed, not that it
  remained live.

### Implementation and ratification gate

W-8 carries implementation after ratification: one pure
`capability_state_at(capability, authorization_offset)` reconstruction used
by both decision time and gate replay; `Broker::revoke_capability`; an
operator-side `asf revoke`; descendant and ancestry verification; inert
approval/exemption handling; and a stable targeted mutation surface.

Required two-sided evidence includes: before-revoke success; direct and
ancestor after-revoke denial; child-only isolation; no retroactive
invalidation; row-without-event inertness; wrong-manifest/wrong-capability
events inert; post-revoke attenuation denial; same-id re-grant denial;
exemption non-resurrection; and a serialized dispatch-vs-revoke race. W-8
must land before W-3 progresses from candidate generation to standing
authority, and before any live-egress or actuation capability ships. The
durable dispatch phase remains trigger-gated
on the first real external effect, but the spec reserves its ordering now.

**Ratification call:** accept the event-derived, permanent, descendant-
cascading candidate; choose a different early-closure representation; or
explicitly retract F1's "revocable" claim and rely on short expiry alone.

---

## SI-23 — actuation grants collapse the out-of-band property of approval surfaces (§2.1 C2/C6, §4, §5.1) — open

Surfaced by design analysis in a parallel session (2026-07-10; filed here
2026-07-11 with review adjustments, renumbered from that session's draft
SI-22 — the number was already taken by the gate-clock issue). No code
forces this yet — no actuation-scoped tool exists in Coppice — but the
issue must be resolved BEFORE one ever registers, because the vulnerable
surface is the approval mechanism itself: the conservative default cannot
save this class retroactively.

*Roaming seam (A23/D7, 2026-07-13): the ratified design center — one human
driving one home from multiple roaming control surfaces — makes this
concrete and near-term. C5's sender-binding gives device-agnostic approval
(the roaming ergonomics), which is exactly the device-agnosticism this
issue's review adjustment #4 flags as undecidable under actuation. The
operator's own usage pattern is therefore a standing argument that SI-23
must resolve before any actuation grant; see ADR 0006's D7 and the
G-ROAMING-SURFACE gate.*

Two related gaps:

1. **Self-satisfiable approvals.** C2 keeps the agent out of the approval
   *data path* (stdin, tokens) but not out of the approval *actuation
   path*: a capability granting UI/input control lets granted hands click
   the broker's own approval surface — synthetic input satisfies C2's
   letter and defeats its purpose. This does not require an adversarial
   model: a benign computer-use agent steered by injected page content
   suffices. Cross-agent too: agent A's hands can approve agent B's
   escalation on the same substrate. This is the *generalization of a
   hole the project already documents* — `dogfooding.md`: "an agent with
   a shell could approve its own escalations, a C2 violation in spirit
   that the protocol topology cannot see." Shell and HID are the same
   class: actuation reach onto the approval surface's substrate.
   Critically, W-4's containment does NOT fix this class — the actuation
   arrives *through a legitimate grant*, inside the sandbox's one door.

2. **Actuation is universal tool acquisition.** A grant of
   undifferentiated UI control is implicitly the union of every
   capability of every reachable app, registered or not — the widest
   possible open surface. Conservative-defaults doctrine says it must be
   treated as such and fail closed. (Symmetry worth preserving in the
   grammar: R2 governs the eyes, `egress` the mouth, actuation the
   hands.)

**Candidate resolution** (origin session, needs ratification; changes
C6's semantics):

- §4: tools declare `actuates: [scopes]` (e.g. `app:obsidian`,
  `input:hid`, `host:shell`, `display:main`) at the trusted-mechanical
  tier; undeclared actuation fails closed. `observes: display` is the
  lesser sibling: approval surfaces must not render sensitive detail
  where a granted eye can read.
- Channel `auth_strength` becomes contextual: a channel's effective
  strength degrades while any *live* capability's `actuates` reach
  includes that channel's substrate. Strength becomes a partial order
  evaluated at decision time; C6's published total order becomes the
  no-live-actuation base case.
- **C7 candidate — no self-satisfiable approvals.** An approval is valid
  only if delivered through a surface unreachable by any capability live
  at decision time; where no registered surface qualifies:
  hardware-attested user presence (passkey UV, hardware-key touch) or a
  device outside every live actuation grant. Corollary: a cross-device
  `platform_oauth` channel legitimately outranks `local_session` on a
  machine with live actuation.
- Softer companion: while an escalation is pending, the broker MAY
  suspend actuation grants for the decision window (secure-desktop
  pattern).

**Review adjustments proposed (2026-07-11 review session; ratify with or
against the candidate):**

1. **C7 must govern the approval round trip, not only the response.**
   Granted hands can *suppress* the escalation notification (dismiss it
   before the human sees it) — nothing forged, the escalation quietly
   ages out of attention; forensically visible, invisible in life. C7
   should require delivery acknowledgment AND response via out-of-reach
   or attested surfaces. Separately, a granted eye reading escalation
   contents is a *confidentiality* problem — split it out as a C4
   extension (delivery-detail ceilings account for observation reach)
   rather than overloading C7. The litmus: the approval path includes
   the hands, the eyes, and the notification — not just the pipes.
2. **Actuation must be a §5.1 caveat dimension with a subset relation,
   not only §4 registration metadata.** "Per-app scoping preferred" has
   no mechanism otherwise. With `actuates` attenuable (`app:X` ⊂
   `input:hid`, hierarchical scopes), per-app narrowing arrives through
   the standard lifecycle: first actuation touching app X escalates (JIT
   elicitation); k ≥ 3 approvals compile an `actuates: app:X` rule via
   the ratchet — zero authorship holds. Fail-closed-on-unknown-dims (§0)
   protects version skew for free.
3. **Contextual strength must derive from signed grant/expiry events,
   never the runtime grant table** (the W-2 principle). Then "which
   actuation grants were live at offset O" is a pure function of the
   substrate prefix — the gate's replay re-verifies each approval's
   validity at the approval's own offset (SI-22 at-the-time semantics
   extend cleanly) and M8 is undisturbed. Blast radius note: C6's
   consumers are not just `approval.min_auth` — expanding amendments
   (§3.1), C4 delivery ceilings, and §5.2 min_auth attenuation all
   compare strengths; each needs its unrankable-fails-closed behavior
   confirmed under the contextual form.
4. **Reachability is undecidable for platform channels, which promotes
   hardware attestation from fallback to primary.** Channel binds the
   sender, not the device (C5); the broker cannot know whether the
   operator's Telegram is on a phone or a web session inside a granted
   browser. Fail closed on undecidable ⇒ while ANY actuation grant is
   live, non-attested channels degrade in general, so hardware-attested
   presence is the load-bearing mechanism during actuation, with
   device-pinned channel registration and the secure-desktop suspension
   as the ergonomic paths. Channel likely needs a substrate/device
   attribute regardless.
5. **The no-lockout escape hatch already exists — state it in the
   amendment.** §3.1 directionality: narrowing is accepted from any
   registered channel, so revoking/suspending the actuation grant is
   always possible from the degraded channel, which restores its
   strength, after which approvals flow. Without this sentence C7 reads
   as a self-inflicted denial of service on single-machine operators.
6. **Trusted-mechanical honesty + W-4 cross-reference.** `actuates` is
   declared metadata, same tier as `reversibility`: a lying tool defeats
   it, and containment against liars is W-4's (sandbox) job — the two
   are complements; neither substitutes for the other. Actuation scopes
   are also one more vendor-declared vocabulary feeding the named
   domain-taxonomy-governance open problem.
7. **Base-case preservation is the argument for amending now.** With
   zero actuation grants ever minted (all of Coppice today), every rule
   reduces exactly to current behavior: C6's total order IS the
   no-actuation case, C7 is vacuously satisfied, `actuates` never
   appears — the A18/A20 refinement pattern, at zero implementation
   cost, and it avoids publishing (W-6) a C6 the project already
   believes is wrong under actuation.
8. **Provenance: the field confirmation is now sourced.** Public X post
   by @GabGarrett (2026-07-10, screenshot on file with the operator):
   GPT-5.6, computer use enabled, "jumped into using the Gmail plugin and
   sending outbound emails on its own" (OP's thread reply confirming
   computer use), with the model's own post-hoc apology — "I made an
   unauthorized external communication and created unnecessary risk" —
   as the only enforcement layer in the loop. That is gap 2 verbatim:
   installed = granted, plus hands, with remorse as the control plane.
   Epistemic status: single-source public report with the OP's direct
   confirmation, not a vendor postmortem; web search does not yet index
   it (hours old at filing). Adjacent verified events in the same class:
   the 2026-02 agent inbox mass-deletion after context compaction
   stripped safety instructions; the 2026-06 forced-install Chrome
   extensions exfiltrating Gmail content through AI-assist surfaces.
   The design argument stands on its own regardless.

**Graduation gate (operational, pending ratification):** no
actuation-scoped tool (computer use, shell, UI control) registers before
C7 and its approval-surface mechanisms are ratified and built — the
ADR-0005 pattern (R2 before egress), recorded in `dogfooding.md`
graduation criteria and `docs/roadmap.md`.

---

## SI-22 — what clock does the gate's trace-vs-capability re-check use? (§5.3) — interpreted (W-2; flipping is cheap)

§5.3: "At the gate, the recorded trace is verified against the capability
— did the run do anything its token shouldn't allow — before anything
becomes durable." The re-check runs at promotion time, which can be hours
after the calls (coarse sessions, parked promotions, RF-9 recovery). For
time-shaped dimensions (`time` windows, expiry) the two candidate clocks
disagree:

1. **Gate-time `now`:** an honest call made inside its window would
   retro-fail at a gate that runs after the window closes — every parked
   promotion would rot toward violation as it waits for approval. Clearly
   wrong, but it is what a naive "re-run the evaluator" produces.
2. **The event's recorded `at` (at-the-time semantics):** "anything its
   token shouldn't allow" reads as *shouldn't have allowed at the moment
   of the call*. The gate then catches decision-time evaluator bugs
   (RF-1's class: a call admitted past its boundary) without punishing
   honest latency between call and gate.

**Interpretation (implemented by W-2, `broker.rs::gate_trace_check`):**
the gate replays each recorded call through the decision-time evaluator
using the event's signed `at` as the clock; unparseable `at` fails
closed (the evaluator's RF-1 posture). Tests encode it two-sidedly:
`gate_catches_time_violation_the_decision_evaluator_missed` (out-of-
window call recorded as if authorized → violation) and
`si22_gate_clock_is_the_events_at_not_gate_time` (in-window work
promotes after its window closes). Registered as contract GATE-REPLAY. Note the trust nuance: `at` is broker-
assigned at record time and covered by the event signature, so within the
fabric's signing boundary it is as trustworthy as the rest of the body —
but it shares RF-13's residual (a key-holding writer can stamp any time;
ordering/anchoring work is the durable answer). Flagging rather than
silently picking: the spec should state the clock, since it is
enforcement semantics, not implementation detail.

**Design-choice note — re-evaluation, not a signed decision certificate
(2026-07-11).** A design review proposed the gate *verify a signed
decision certificate* the broker emits at call time, rather than
re-run the evaluator. W-2 deliberately chose re-evaluation. The reason is
load-bearing and worth recording so it is not "fixed" later by adding
certificates: a faithfully-signed certificate attests *what the broker
decided*, so it reproduces a decision-time evaluator **bug** exactly
(a buggy Allow verifies clean forever), whereas re-evaluation attests
*what the capability actually permits* over the signed record and so
catches the RF-1 class — a call the evaluator wrongly admitted. The
verdict event already carries the full check record (SI-14) for
legibility and audit; it is evidence, not the gate's authority. Net: the
signed certificate is the weaker check for this job; re-evaluation
subsumes it. (A certificate would still matter for *third-party*
verification without the evaluator — a W-6 conformance/export concern,
not a gate concern.)

---

## SI-21 — brokered manifest authority creates a content-address cycle (§3, M1/M2/M7) — RESOLVED (author, 2026-07-10)

**Resolution: binding-as-event, ratified as amendment A21 (spec v0.6) —
the event flavor of the "separate signed authority-binding object/event"
candidate family, plus a mode declaration and an export-view corollary.**

- Core: the capability id never enters the manifest body. The authority
  lineage is two acyclic edges — **backward** `cap.bound_manifest ==
  man.id` (M2, content-addressed; trustworthy exactly because mint
  follows seal) and **forward** the broker's signed `grant` event
  countersigning the mint at its substrate offset (the SI-10 pattern:
  the event is the proof, the field is a convenience). The temporal
  observation that decided the framing: state/intent/behavior are
  past-facing lineages and live in the body as content hashes; trace and
  (brokered) authority are future-facing and were always going to bind
  through the chain — the manifest never contained its trace either, it
  named a span.
- Mode declaration: manifests declare `authority: {"mode": "brokered"}`;
  absent = observed (conservative default; every historical manifest
  reads observed, which is correct for its era — a one-time note, not a
  migration). Under brokered mode the gate fails closed: every
  `tool_call` must carry its capability, and a verified grant event
  binding that capability to this manifest must precede it in substrate
  order. Declared-brokered never silently reclassifies as observed.
  Observed mode claims nothing and forbids nothing — attributed calls
  are still fully checked (M1/M2 bind whenever a capability exists); the
  declaration only ever adds constraints.
- Rejected — envelope-as-object (`ManifestCore → Capability →
  DelegationEnvelope`): under crash analysis (mint lands, envelope seal
  does not) the envelope's absence is ambiguous between observed-by-
  intent and brokered-but-crashed, so the discriminator falls back to
  the substrate — the envelope can only ever be a *view* of chain truth,
  which is A15's row-without-event rule restated. Adopted instead as the
  *derived export artifact* (manifest + capabilities + grant refs, the
  §7.2 TrustRecord-export pattern). Rejected — pre-seal capability
  commitment: welds the broker into the seal critical path, invents a
  second body-minus-field hashing rule (SI-1 déjà vu), and has no
  mid-run re-grant story.
- Review adjustments at implementation: grant events stay on the
  fabric-lifetime span with `manifest` set (the register/intent/approval
  family, A15) — the broker already emitted exactly this event at mint,
  so A21 promotes existing bookkeeping to constitutional; the fail-closed
  rule is stated per-effect with offset ordering (an empty
  declared-brokered run gates clean — the rule binds effects, not
  sessions); multiple grants per manifest are legal (expiry re-mint,
  attenuation) — the lineage is the offset-ordered grant set.
- Provenance: ratified 2026-07-10 in the SI-21 design exchange (both
  candidate families steelmanned; the crash-degeneration argument was
  decisive). Original analysis below, preserved as provenance.

The live proxy creates and seals a Manifest at the step boundary with
`authority` omitted, then asks the broker to mint a capability whose
`bound_manifest` is that Manifest id. M7 says an omitted `authority` denotes an
observed run for which broker enforcement does not exist, so a genuinely
brokered run is mislabeled. Adding the capability id to `authority` after mint
does not work: it changes the Manifest id, which in turn invalidates the
capability's `bound_manifest`, creating a content-hash cycle.

**Current implementation** (`kernel.rs`, `broker.rs`, `proxy.rs`): the sealed
Manifest remains capability-less and the later capability binds to it. This is
adequate to exercise the Stage 2 broker path, but it is not treated as a
spec-compliant resolution of M7.

**Open question for the spec:** which edge is authoritative and how is it
represented without a cycle? Candidate families include a separate signed
authority-binding object/event, a predeclared capability commitment that is not
the final capability id, or redefining Manifest `authority` as a post-seal
registration relationship. The kernel must not choose among them silently.

This was deliberately not changed during the dogfooding-readiness remediation:
the choice changes canonical signed artifacts and their ids, so it requires
author ratification before implementation.

---

## SI-20 — mid-session out-of-band edits can be absorbed unattributed (A12 vs §5.3) — RESOLVED (author, 2026-07-09)

**Resolution: the candidate ratified as amendment A20 (spec v0.5), with two
author strengthenings and two implementation-review adjustments.**
- Core: pre-consumption divergence check — drift (A12 attribution) emitted
  before any merge consumes live trunk. New invariant **M8 (attribution
  completeness)**, phrased as a property of the ledger, not the gate.
- Strengthening 1 (author): narrative parity — drift bodies carry the A13
  op-class summary; extended in review to ALL drift emissions (boundary
  checks too), not only gate-time ones.
- Strengthening 2 (author): gates serialize; adjusted in review from
  per-store to per-fabric-home grain (gates consume all roots as one
  coherent tuple; per-store locks could deadlock), cross-process via flock.
- Review additions: **revert is a fourth consumer** of live state and gets
  the same pre-check (a gate-scoped fix would have missed it); M8's unit
  is the **divergence window** (net change between consecutive
  attestations), or the exactly-once clause fails honestly on multi-edit
  windows — the op summary restores per-path narrative inside the single
  event.
- A3 corollary (author): SI-18 whole-store memory merges pass the same
  pre-check — cross-run taint's landing zone is monitored, never silent.

Implemented with four-timing property tests
(`m8_attribution_is_timing_independent`, the conflict-park drift
assertion, `m8_revert_attributes_divergence_before_erasing_it`). Original
analysis below, preserved as provenance.

Prompted by dogfooding (2026-07-09, first verified real-client loop), then
confirmed by code reading — importantly, NOT by observed misbehavior: the
session in question did everything right. The operator's pre-session hand
edit was caught at the next check and attributed `human_local`; an initial
reading of the ledger misattributed the gap, which is itself a
ledger-legibility data point. The gap the analysis exposed is a specific
unexercised window:

A trunk edit made *while a session is live*, to a path the branch does
**not** touch, is folded into the gate's merged root as the trunk side —
`expected_roots` is then updated to the merged result, no boundary ever
sees divergence, and **no drift event is emitted** (`promote_manifest`
never runs a drift check; `compute_merge` reads live trunk). The identical
edit made *between* sessions produces `drift {attribution: human_local}`
at the next step boundary or ledger check (verified working). A
mid-session edit to a path the branch *did* touch surfaces as a trunk-wins
conflict, so it is at least visible; the silent case is exactly the
branch-untouched path.

A12 promises out-of-band local edits are "logged, not alerted" — attributed
quietly, but attributed. §5.3 specifies the merge but says nothing about
attributing trunk-side divergence encountered at promotion time. So the
attribution completeness of the ledger currently depends on *when* the human
happens to edit relative to session lifetime: two acts identical in
substance leave different ledger narratives.

Single-human v0 impact is cosmetic (the absorbed edit is the human's either
way, and state stays fully explained). Multi-actor impact is not: absorbed
edits are exactly where an unattributed actor's changes could ride a
promotion into trunk — this touches the same surface as the A3 memory-taint
problem and the reserved multi-actor visibility policy.

**Candidate resolution** (not implemented — needs author ratification): at
promotion, before computing the merge, compare live trunk roots against
`expected_roots`; on divergence emit `drift` (same A12 attribution classes,
`between` bracketing the session's span offsets) and only then merge. That
makes "every out-of-band change gets an attribution event" an invariant
independent of timing — a candidate M-invariant phrasing for the spec.

---

## SI-1 — `id` self-reference in the hash body (§0) — interpreted

§0: "Every object's `id` is the SHA-256 of its canonical body **excluding
`sig`**." The examples show `id` *inside* the object body. An id cannot be the
hash of a body that contains itself.

**Interpretation** (`canon.rs`): the hash body excludes **both** `id` and
`sig`. `id = "<prefix>:" + hex(sha256(JCS(body ∖ {id, sig})))`. Suggest the
spec say "excluding `id` and `sig`" explicitly.

## SI-2 — what exactly is signed (§0, §8) — interpreted

The spec defines `sig: {key_id, alg, value}` but never states the message the
signature covers.

**Interpretation** (`canon.rs`): Ed25519 over the same canonical body bytes
whose hash is the id (body ∖ {id, sig}). Verify = recanonicalize, check id,
verify sig. This makes id-check and sig-check attest the same bytes.

## SI-3 — who signs a Manifest (§3, §8.1) — interpreted

§8.1 assigns: user root → Principals, Intents; broker key → events,
capabilities, tool countersignatures; agent instance keys → attestations.
Manifests carry `sig` but no signer is assigned. (Same gap for Channel — §2.1
shows `sig` on a channel registration.)

**Interpretation** (`kernel.rs`): the fabric component that constructs the
manifest at the step boundary signs it — in Stage 1 the kernel/coordinator
key, which in Stage 2 becomes the broker daemon's key (it is the same trusted
component that signs events). Channels likewise fabric-signed at registration.

## SI-4 — first event of a span: `prev` (§6) — interpreted

The Event example shows `prev: "evt:…"` with no null case; nothing says what
the chain head looks like, or whether `seq` starts at 0 or 1.

**Interpretation** (`trace.rs`): head event has `prev: null`, `seq: 0`.
Chain verification requires seq contiguity from 0 and prev-linkage thereafter.

## SI-5 — `manifest` on pre-manifest events (§6) — interpreted

Intents are captured *before* the first manifest (`captured_before` is a
substrate offset), so intent-capture and channel/principal registration events
exist with no manifest to reference, but Event shows `manifest: "man:…"` as if
always present.

**Interpretation** (`trace.rs`): `manifest` is nullable; null for
substrate-level events that precede or stand outside any manifest (principal
/channel registration, intent capture, drift observed between runs).

## SI-6 — sqlite state root: what bytes get hashed (§3 state.roots) — interpreted

`{"store": "db:memory", "kind": "sqlite", "root": "sha256:…"}` — hash of what?
The raw db file changes byte-identity under WAL checkpointing and vacuum even
when logically unchanged; a logical serialization (sorted dump) is stable but
slower and loses page-level fidelity.

**Interpretation** (`snapshot.rs`): checkpoint WAL (TRUNCATE), then hash the
main db file bytes. Byte-identity is the drift detector: an untouched file
hashes identically; any touch (even logically-neutral) is at minimum honest
about "something wrote here". Cost: a logically-identical-but-rewritten db
reads as drift — acceptable for Stage 1, revisit if it produces noise.
Related: F2 — resolution proposed in `docs/adr/0002-f2-state-root-addressing.md`
(page-aligned sqlite chunking under CID roots subsumes this issue's cost
concern; byte-image behavior remains the Stage 1–2 interim).

## SI-7 — manifests without a capability (§3 `authority`) — open

Manifest schema shows `authority: {capability: "cap:…"}` unconditionally, and
M1/M2 bind roots↔capability. Stage 1 has no broker and no capabilities, yet
the kernel must mint manifests now; also future purely-local runs (no external
authority at all) arguably need no capability.

**Interpretation for Stage 1** (`kernel.rs`): `authority` is omitted; M1/M2
tests are written against the pure checking functions and marked as activating
in milestone 2. **Open question for the spec:** is a capability-less manifest
legal long-term (local-only runs), or must the broker mint a no-external-reach
capability for every run so M1/M2 are unconditional?

## SI-8 — drift `between` field type (§6 drift body) — interpreted

`between` appears in the drift body with no type or example.

**Interpretation** (`kernel.rs`): `between: [offset_lo, offset_hi]` — the
global substrate offsets bracketing the window in which the unobserved change
occurred (last event that attested the expected root, first observation of the
divergent root). Substrate offsets rather than event ids because the endpoints
may belong to different spans.

## SI-9 — payload AEAD algorithm unspecified (§1, §8.2) — interpreted

Per-payload DEKs and KEK wrapping are mandated; no cipher is named anywhere.

**Interpretation** (`payload.rs`, `keys.rs`): AES-256-GCM for both payload
encryption and DEK wrapping, recorded in the dek row (`alg` column) so the
choice is per-payload upgradable. Spec should name (or version) the suite —
interop depends on it.

## SI-11 — no event kind for object registration or intent capture (§6) — RESOLVED (author, 2026-07-08)

**Resolution: Option A.** `register` and `intent` become §6 kinds; a
fabric-lifetime span carries events outside any manifest (also settles SI-5);
objects are live only once their register event is on the chain (object rows
are materialized views; row without event fails closed). This also supplies
the substrate countersign that resolves SI-10's proof gap. Kernel already
implements this shape; spec text amendment pending (A15 candidate).

The §6 `kind` enum is closed (14 values) and has no kind for: principal
registration, channel registration (§2.1 — a signed, "registered object"),
tool registration (§4 — broker countersigns, surely traced), or intent
capture (§3.1 — which *must* hit the substrate to make `captured_before`
mean anything, see SI-10). C1 requires these authority acts to be recorded;
there is no vocabulary to record them with.

**Interpretation** (`trace.rs`): two added kinds — `register` (body:
`{object, object_kind}`) and `intent` (body: `{intent, channel,
auth_strength, captured_before}`). Spec should either add these to §6 or
state where registrations are recorded if not as events.

**Supporting observation:** §6 already contains `amendment`, and §3.1 defines
an amendment as an intent with `amends != null`, judged as one chain — so the
spec currently records the same act (a channel-stamped instruction) as an
event when it arrives mid-run but not when it founds the run. Adding `intent`
(and `register`) removes the asymmetry; it also supplies the substrate
countersign that makes `captured_before` a proof (SI-10) and gives C1's
ratification-velocity analysis one ordered stream to read. If events are
added, two companion rules are needed: (a) a blessed fabric-lifetime span for
events outside any manifest (cf. SI-5), and (b) objects are live only once
their register event is on the chain — object rows are materialized views;
a row without an event fails closed.

## SI-12 — glob "narrower" is not mechanically decidable as written (§5.2) — interpreted

§5.2 requires attenuation verification to be "mechanical subset-checking"
and lists "globs narrower" as one dimension — but glob-language subset is
not simple to decide in general. **Interpretation** (`capability.rs::glob_covers`):
a conservative provable-cases-only rule (identical globs; parent `**`;
parent `prefix/**` with a literal prefix covering the child). Everything
else is rejected as not-a-subset even when a human can see it is one (e.g.
parent `inbox/*.md`, child `inbox/a.md`). Fail-closed and spec-compatible,
but the spec should either bless a restricted glob dialect with decidable
subset or state that conservative approximation is intended.

## SI-13 — no event kind for escalation *resolution* (§6, A9) — interpreted

C1 lists "escalation approval" as a human authority act that must be
recorded with channel + auth_strength; §6 has `escalation` (the request)
but no kind for the decision. **Interpretation** (`trace.rs`, `broker.rs`):
added kind `approval`, body `{escalation, resolution: approved|denied,
uses, channel, auth_strength}`. Same family as SI-11; fold into the same
spec amendment.

## SI-14 — how are mechanical denials recorded? (§6) — interpreted

A denied call executes nothing, so it is not a `tool_call`; §6's `verdict`
kind reads as the model judge's (brief §5.4 layer 3). **Interpretation**
(`broker.rs::deny`): broker denials are `verdict` events with
`source: "broker"`, carrying the failed dimensions and full check record.
If the spec would rather reserve `verdict` for the judge, it should name a
`denial` kind.

## SI-15 — auth_strength has no total order (§2.1, §5.1) — interpreted

`approval.min_auth` requires comparing channel strengths, but §2.1 only
lists them. C4 groups `local_session` and `passkey` as the strong tier.
**Interpretation** (`capability.rs::auth_rank`): unverified(0) <
platform_oauth(1) < passkey(2) = local_session(2). Unrankable strengths
fail closed. Spec should publish the order — it is consent-surface
semantics, not an implementation detail.

## SI-16 — M1's "grants any access to" needs a derivation rule (§3 M1, §4) — interpreted

M1 says roots must cover every store the capability grants access to, but
nothing in §4/§5 says how to compute a capability's store-reach.
**Interpretation** (`tools.rs`, `broker.rs::mint`): tool registrations
declare `store` per action (plus `class` for budget metering and
`path_args` for paths.write extraction — same trusted-mechanical tier as
the §4 example's `writes`); mint-time M1 = union of stores of allowlisted
tools ⊆ manifest roots. Spec should adopt per-action store/class/path
bindings into §4, or name its own derivation.

## SI-17 — `move` vs `rename` are listed but never defined (§5.3 A13) — RESOLVED (author, 2026-07-09)

**Resolution:** the proposed amendment below is ratified as written —
parent-directory definitions; `rename < move` ordering (allow-move implies
allow-rename); exact-hash authority layer with similarity permitted only
as legibility-layer annotation; from/to direction constraints deferred to
the Stage 3 rule grammar. Kernel already conforms; the ordering function
lands with StandingRules. (A17 amendment candidate.)

A13's operation classes are `add | modify | delete | move | rename`, with
no definition distinguishing move from rename. This is authority
vocabulary, not display: promotion rules (StandingRules, A13) match on
these classes, so the definition is security semantics.

**Interpretation** (`promote.rs`): same content hash, same parent
directory, new name = `rename`; different parent directory = `move`.
Pairing is exact-hash and only when unambiguous (hash unique on both
sides) — never mislabels; leaves duplicate-content shuffles and
renamed-AND-edited files as add+delete (which park — safe, with friction).

**Proposed amendment (discussed with author 2026-07-09):**
1. Adopt the parent-directory definitions, AND order the classes:
   `rename < move` in required permissiveness — a rule allowing `move`
   allows `rename`, never the reverse (the reversibility_rank pattern).
   Avoids the disjoint-label footgun (ratified "moves into MOCs" parking
   an in-place rename in MOCs).
2. Authority layer stays exact-hash forever: similarity-based rename
   detection is GAMEABLE (an agent can present delete-X-plant-altered-
   content as "rename+edit" inside a rename-allowing rule) and has no
   zero-authorship threshold. Similarity may only ever annotate the
   legibility layer (diff cards: "looks like rename+edit"), never produce
   a class rules can match.
3. Stage 3 rule grammar should add direction constraints —
   `move: {from ⊆ globs, to ⊆ globs}` — which do the real work for
   filing workflows; from/to are already recorded in op JSON.

## SI-19 — `link` appears in §5.3's example rule but not in A13's enum — RESOLVED (author, 2026-07-09)

§5.3: "auto-merge iff paths ⊆ {/inbox, /MOCs}, ops ⊆ {add, modify,
**link**}, no deletes" — but A13 defines `add | modify | delete | move |
rename`. Content-aware classes were judged complexity-over-benefit for
now (the ratchet's coarse equivalent — path-scoped `modify` rules — is
adequate for the solo-operator design center).

**Resolution: remove `link` from the §5.3 example; add a
refinement-only extensibility clause to A13** so content-aware classes
can return without redesign (A18 amendment candidate):
1. Class vocabulary is fail-closed: rules match explicit sets; classes
   unknown to a rule park at the gate. Future classes can never widen
   pre-existing grants.
2. Extension is by refinement only: `<root>.<refinement>` of the five
   structural roots; a refinement ADDS a predicate to the structural
   classification, never replaces it — a gamed/buggy predicate degrades
   to the parent class, bounding blast radius.
3. Rules allowing a root allow its refinements (SI-17 ordering pattern);
   never the reverse.
4. Content-aware refinements require registered, versioned classifiers;
   rules pin the classifier version they were ratified under (A1
   domain-scoped pinning, applied to classifiers).
The kernel implements structural roots only; nothing changes in code.

## SI-18 — three-way merge is undefined for opaque stores (§5.3 A11) — interpreted

A11 specifies three-way merge with per-path conflict semantics, which only
exists for file-tree stores. The manifest's other Tier-1 store — the agent
memory sqlite — has no sub-file merge. **Interpretation** (`broker.rs`
compute_merge): opaque stores merge whole-store: branch-only change
installs the branch image (op class `modify`, auto-promotable); trunk-only
change stands; both-changed is a single conflict card, trunk wins, branch
image preserved in CAS. The spec should state per-store-kind merge
strategies — this also touches the A3 memory-taint open problem, since
whole-store memory promotion is exactly where cross-run taint lands in
trunk.

## SI-10 — `captured_before` freshness is unenforceable as specified (§3.1) — open

`captured_before` proves the intent was signed before a given substrate
offset only if the offset is bound at signing time by something the substrate
attests. As specified it is a self-declared field: nothing stops an intent
minted later from claiming an earlier offset. Fine for Stage 1 (single trusted
kernel process writes both), but the "pre-contamination proof" language (§3.1)
overclaims for any deployment where the intent signer and the substrate are
not the same trust domain. Likely fix: intent-capture event in the substrate
countersigns the intent id at its actual offset (the event *is* the proof;
the field is a convenience). Kernel already emits such an event; flagging so
the spec language gets tightened.
