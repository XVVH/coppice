# ADR 0006 — Authenticated global trace order and rollback anchor

**Status: PROPOSED — SI-25 design candidate, awaiting human ratification.**

This ADR is not a specification amendment and authorizes no implementation.
SI-25 remains open. The field names below are candidate wire shapes, not a
published format. If ratified, the decision must be integrated into the schema
specification under the repository's amendment discipline before W-15 changes
runtime code.

> **Recovery note (2026-07-13, appended at import — not part of the
> original draft).** This ADR was drafted at PR #38 and recovered from
> the unpushed `agent/si25-authenticated-head-design` branch; W-12,
> W-13, W-19 (PRs #39–#41) and the W-14 protocols (PR #43) postdate it.
> Four seams for the ratification session, from the 2026-07-13
> fresh-eyes review:
>
> 1. The non-goals list parks P16, but PR #43 closes P16 with the
>    signed state-change recovery journal (SI-31). Composition clauses
>    needed at ratification: the append protocol's checkpoint and
>    anchor-outbox join the same SQLite transaction as the W-14 staged
>    event/roots; reopen recovery consults the anchor before trusting
>    the journal (the journal is mutable local metadata under the
>    anchor-ahead recovery row); and the reason "rolled back but
>    already anchored" is unreachable — event and checkpoint share one
>    transaction — should become a stated clause, not a coincidence.
> 2. `TraceEpoch` creation (`reason: "initialize"`) and the accept-once
>    initial anchor CAS belong inside W-13's `Fabric::initialize`
>    boundary, unreachable from `open_existing`; migration runs under
>    that same boundary.
> 3. H15's operator diagnostics land on W-19's ledger integrity view —
>    global-gap, epoch-mismatch, anchor-ahead, and fork findings
>    surface there under the G9 process-level exit-status contract; the
>    validation plan should name that surface.
> 4. W-15 implementation re-pins the W-9 corpus baseline (new signed
>    event fields change ingest output) while corpus replay verdicts
>    stay invariant (A22 adjustment 8); the validation plan should
>    state both.

## Decision summary

Adopt two deliberately separate mechanisms:

1. Every new trace event participates in one signed, gapless, per-home global
   chain in addition to its existing per-span chain. The signed event body binds
   `home`, `epoch`, `global_seq`, and `global_prev`. SQLite `offset` remains a
   storage locator and carries no authority.
2. A signed checkpoint commits a terminal global event. A monotonic anchor
   outside the rollback domain of the fabric home advances by compare-and-swap
   and returns a receipt for that checkpoint. Production and standing-authority
   claims require a qualifying anchor. A local signed checkpoint without one
   is useful integrity evidence, but cannot establish freshness after full-disk
   rollback.

The recommended first qualifying anchor is a small remote witness that stores
only home/epoch/checkpoint identifiers and signed checkpoint hashes. The
protocol is an `AnchorStore` interface so a hardware monotonic facility can
provide the same semantics for an offline deployment. A sibling file, ordinary
keychain entry without a documented anti-rollback guarantee, or another table
in `fabric.db` is not a production anchor.

The construction makes every event globally ordered and makes omission
detectable relative to a trusted terminal checkpoint. It does not make the
fabric key trustworthy, provide a trusted wall clock, make local state commits
atomic, or make a self-contained restored disk image prove that it is current.

## Context

The current event object signs `span`, `seq`, and `prev`, so it authenticates
the order of retained events within a span. Cross-span consumers instead use
SQLite's unsigned `offset`: M7 grant-before-effect, A22 revoke-before-effect,
intent capture, approval headroom, drift windows, and durable-effect ordering.
W-11 ensures retained row selectors agree with signed raw and uses signed
`seq` inside a span. It intentionally cannot detect a deleted suffix, a deleted
whole span, a restored old database, or a cross-span offset rewrite.

Those gaps are RF-13/P15 and SI-25. They block standing-authority compilation
and production rollback claims. They are not safely repaired by adding one more
check around the current rowid: order, completeness, and freshness need an
explicit protocol and recovery model.

## Scope and non-goals

This candidate covers:

- global event order across all spans in one fabric home;
- completeness relative to a trusted checkpoint;
- home and epoch binding;
- checkpoint publication and crash recovery;
- rollback and fork detection through an independent monotonic anchor;
- deterministic export/import order;
- the ordering boundary for future durable external effects;
- migration of current databases without retroactive security claims; and
- verification, adversary, mutation, crash, and performance requirements.

It does not resolve:

- SI-26's object-type and protocol-domain transcript for all signed objects;
- SI-27's user-root/fabric-key trust, rotation, loss, compromise, or recovery
  ceremony;
- P16's crash-atomic multi-root state commit;
- P21's cross-host leader fencing;
- P22's durable external-effect protocol or idempotency behavior;
- trusted time for expiry and time caveats; or
- CAS/object application integrity, owned by W-14.

The design states the seams to those items so their later implementations do
not reopen trace ordering.

## Threat model

### In scope

The storage adversary may, without the signing key:

- update, insert, reorder, or delete SQLite rows after bypassing triggers;
- change every denormalized selector, including `offset`;
- delete a tail, a middle segment, the latest authority event, or every event
  in a span;
- restore `fabric.db`, the whole fabric home, or a backup to an older image;
- copy one home or database and run two writers from the same apparent state;
- inject malformed JSON, foreign-signed rows, duplicate sequence values, or an
  unbounded number of invalid rows; and
- crash the process or machine at every boundary between local commit, anchor
  publication, receipt persistence, dispatch, and acknowledgement.

Verifiers may receive a hostile export. The network to an anchor may be
unavailable, reordered, replayed, or controlled, but its signatures cannot be
forged and its monotonic state cannot be rolled back under the selected
deployment profile.

### Out of scope, but surfaced

A live compromise that can use the fabric signing key can create valid new
events. An anchor limits equivocation after an anchored checkpoint, but does
not tell whether a key-authorized event was legitimate. SI-27 owns key trust
and compromise recovery. A compromised anchor can lie about freshness unless
another independently trusted anchor or transparency observer detects it.
Traffic analysis can reveal checkpoint cadence even though the witness sees no
event bodies.

No cryptographic chain proves that an absent real-world action never occurred.
The future durable-effect protocol must make dispatch pass through this chain;
unmediated effects remain outside the proof.

## Assurance labels: do not collapse these claims

| Evidence available | What a verifier may claim | What it may not claim |
|---|---|---|
| Retained signed global chain only | listed events have one authenticated order; gaps or edits before the presented head are detectable | presented head is the latest head; a suffix, whole newer database, or later epoch was not removed |
| Chain plus caller-pinned checkpoint | history is complete through that exact checkpoint | checkpoint was the latest one ever issued |
| Chain plus receipt named by a fresh, challenge-bound anchor status | history reaches the anchor's latest checkpoint for this home/epoch at the status response; rollback below it and anchored forks are detectable | trusted event time; honesty after signing-key or anchor compromise |
| Frozen export plus receipt current when exported | export was complete through the receipted checkpoint at export time | the live home has not advanced since export |

A checkpoint signed by the same key and stored beside `fabric.db` is in the
first row, not the third. Full-disk rollback restores the checkpoint and the
database together. This is the central SI-25 distinction.

## Required invariants

**H1 — one signed global order.** Every v1 event in an epoch contains the
same `home` and `epoch`, a contiguous `global_seq` beginning at zero, and
`global_prev` equal to the preceding global event id (null only at genesis).
Per-span `seq`/`prev` remains independently verified.

**H2 — offsets are never authority.** Database rowid/`offset` may locate or
display a row but may not decide order, liveness, capture, drift, approval
headroom, export order, or branch tips. All such consumers use authenticated
global sequence or per-span sequence as appropriate.

**H3 — one home, one epoch.** Event, epoch, checkpoint, and anchor receipt all
bind the same home and epoch. A record from another pair cannot contribute
authority or closure. A clone cannot silently become a new home, and an old
database cannot silently start a new epoch.

**H4 — completeness is relative to an expected head.** Verification succeeds
only when it reaches the terminal event named by the caller's checkpoint or by
the latest qualifying anchor receipt. Enumerating whatever spans remain is not
completeness.

**H5 — monotonic anchored publication.** An anchor advances only from its
current checkpoint to a signed descendant checkpoint for the same home/epoch.
Same-value publication is idempotent. Regression, sibling publication, and
epoch substitution fail closed and surface as rollback/fork findings.

**H6 — authority reads one authenticated snapshot.** Decision, gate replay,
ratchet compilation, and recovery each consume one transactionally consistent
event set whose terminal authenticated sequence is explicit. No separately
read mutable `MAX(offset)` may extend that view.

**H7 — no acknowledged unanchored tail in the production profile.** An event
group is not reported durable until its local commit is durable and the
required anchor has accepted a checkpoint containing it. Events may be batched
inside one atomic operation, but not left in an acknowledged gap. In
particular, a revoke or other closure is never acknowledged early. Locally
committed but unanchored closure still narrows local decisions; recovery must
finish publication before protected work resumes. A development profile may
acknowledge unanchored events only with the local-integrity assurance label and
therefore does not close SI-25/P15.

**H8 — durable dispatch shares the order.** A future external-effect dispatch
or reservation is an event in the same global chain. The side effect may be
sent only after the dispatch checkpoint is durably anchored. Grant, revoke,
approval, and dispatch therefore have one authenticated order.

**H9 — no silent degraded authority.** If the qualifying anchor cannot be
read or advanced, the system cannot label the view fresh or compile/use
standing authority. Any later fail-open behavior requires the separately
ratified, per-capability `on_broker_outage` semantics and must emit a durable
degradation record when recording is possible. W-15 does not invent that
policy.

**H10 — crash recovery is monotonic and idempotent.** Recovery can publish an
already committed checkpoint, recover an already issued receipt, or restore
missing local data to the anchor's head. It cannot replace an epoch, discard an
anchored head, or guess which fork won.

**H11 — export preserves authenticated order.** Exports order events by
`(home, epoch lineage, global_seq)`, include the epoch record and terminal
checkpoint/receipts, and never serialize rowids as ordering authority.

**H12 — legacy history is not retroactively authenticated.** Migration may
commit to the exact legacy rows and ordering observed at cutover, but it may
not claim that this was their original or complete historical order. Legacy
events cannot become standing-authority evidence merely by being checkpointed.

**H13 — key history is explicit.** Every checkpoint identifies its signer.
Rotation is accepted only through SI-27's trusted key-history chain, with the
transition ordered and anchored before new-key events are authoritative.

**H14 — structural records are not pruned.** v1 provides no history-pruning
protocol. Removing globally chained structural events is corruption even when
payloads have been shredded. A future accumulator/pruning design requires a
separate amendment and proof format.

**H15 — resource exhaustion fails closed.** Invalid rows, gaps, duplicate
sequences, oversized segments, anchor replay, or witness timeouts cannot cause
partial verification to be treated as authority. Operators receive bounded,
specific diagnostics; protected effects and standing compilation do not occur.

## Candidate signed representations

All fields shown below are inside the content hashed for the id and signed.
The explicit `protocol` field gives these new artifacts an internal version
marker, but does not resolve SI-26's general signing-transcript question.
Ratification must coordinate the final transcript with SI-26 before any
cross-implementation publication. Integers remain inside the spec's JCS domain.

### Epoch record

An epoch is a continuity boundary, not an excuse to reset history:

```json
TraceEpoch {
  "id": "te:...",
  "protocol": "asf.trace-epoch/v1",
  "home": "home:<random-256-bit-id>",
  "epoch": "epoch:<random-128-bit-id>",
  "ordinal": 3,
  "prior": {
    "epoch": "epoch:...",
    "checkpoint": "th:...",
    "anchor_receipts": ["ar:..."]
  } | null,
  "reason": "initialize" | "migration" | "key_rotation" | "recovery",
  "signing_key": "key:...",
  "created_at": "RFC3339",
  "legacy_commitment": {
    "protocol": "asf.legacy-order/v1",
    "count": 123,
    "order_digest": "sha256:..."
  } | null,
  "sig": { ... }
}
```

`home` is stable for the logical fabric home. `epoch` changes only through an
explicit transition. `ordinal` is checked against trusted epoch lineage but is
not trusted alone. A normal transition references the prior anchored
checkpoint and is authorized under SI-27's key history. Recovery without the
old fabric key needs the future user-root recovery ceremony; it cannot be
defined as "generate a key and increment ordinal."

### Globally ordered event

The existing event gains a signed protocol marker and four global-order
fields:

```json
Event {
  "id": "evt:...",
  "protocol": "asf.trace-event/v1",
  "home": "home:...",
  "epoch": "epoch:...",
  "global_seq": 41,
  "global_prev": "evt:..." | null,
  "span": "span:...",
  "seq": 7,
  "prev": "evt:..." | null,
  "manifest": "man:..." | null,
  "at": "...",
  "kind": "...",
  "body": { ... },
  "sig": { ... }
}
```

The global and per-span predecessors are event ids, so one signature binds
both orderings. `global_seq` is the substrate offset in the normative sense;
the current SQLite `offset` is renamed conceptually to `storage_rowid`. Epoch
rollover is mandatory before `global_seq` reaches the JCS integer ceiling.

Every event kind and every span, including the fabric-lifetime span, enters
this one chain. There is no special unchained "checkpoint event" that could be
deleted without affecting order.

### Checkpoint

A checkpoint is a signed claim about an already committed terminal event; it
is not itself an event and therefore does not create a recursion:

```json
TraceCheckpoint {
  "id": "th:...",
  "protocol": "asf.trace-checkpoint/v1",
  "home": "home:...",
  "epoch": "epoch:...",
  "through_seq": 41,
  "head_event": "evt:...",
  "prior_checkpoint": "th:..." | null,
  "signing_key": "key:...",
  "sig": { ... }
}
```

The event chain commits the full prefix, so a second Merkle tree is not needed
for correctness. Checkpoints provide bounded verification starts, publication
units, and stable export pins. Their signed content is deterministic for a
given prior checkpoint and event head, allowing recovery to recreate the same
id after a crash. The production profile checkpoints every committed event
group; multiple events may share one checkpoint only when they commit as one
operation and none is acknowledged before the checkpoint is anchored.

### Anchor receipt

```json
AnchorReceipt {
  "id": "ar:...",
  "protocol": "asf.trace-anchor-receipt/v1",
  "anchor": "witness:...",
  "home": "home:...",
  "epoch": "epoch:...",
  "checkpoint": "th:...",
  "anchor_revision": "rev:19",
  "prior_receipt": "ar:..." | null,
  "accepted_at": "anchor-controlled-time",
  "sig": { ... }
}
```

`accepted_at` is witness metadata, not the SI-22 event clock. The anchor's
state is keyed by `home`; it accepts an epoch transition only when the new
epoch record references the checkpoint currently anchored for the prior epoch.
Within an epoch, compare-and-swap requires the current checkpoint/receipt as
the parent. A receipt returned twice for the same checkpoint is identical or
semantically idempotent.

Initial publication is compare-and-swap from an absent home record and is
accepted only once. The witness verifies the artifact signature only after the
deployment supplies the trusted initial key binding; SI-27 owns that binding.
The witness's monotonic storage does not itself decide who owns a home.

A static receipt proves acceptance, not that no later receipt exists. Live
rollback checks use a nonce-bound status response so a network attacker cannot
replay an old valid receipt:

```json
AnchorStatus {
  "protocol": "asf.trace-anchor-status/v1",
  "anchor": "witness:...",
  "home": "home:...",
  "current_receipt": "ar:..." | null,
  "challenge": "caller-random-256-bit-value",
  "issued_at": "anchor-controlled-time",
  "sig": { ... }
}
```

The caller accepts the status only for its exact fresh challenge and configured
anchor identity. `issued_at` labels the witness observation for export; it is
not substituted for event time or a caller nonce.

## Append and checkpoint protocol

All appenders for a home share one single-writer/fenced critical section. The
current per-span transaction is insufficient because two spans can otherwise
read the same global tip.

1. Acquire the per-home writer fence. P21 must replace the host-local fence
   before multi-host writing; W-15 must not imply that a local lock fences a
   remote clone.
2. Begin an SQLite immediate transaction. Read the epoch and cached global tip.
3. Verify the cached tip against the referenced event. Treat the table as a
   cache, never as evidence.
4. Construct and sign the event with the next global and span predecessors.
5. Insert the event and update the cached tip in the same transaction. Sign the
   operation's terminal checkpoint and insert it plus a durable anchor outbox
   item in this transaction.
6. Commit with the production durability profile (`synchronous=FULL` and the
   required file/directory durability checks). Only then may publication begin.
7. Publish the checkpoint by anchor compare-and-swap while retaining the home
   writer fence. Store the receipt in a new local transaction.
8. Release the fence only after the operation's acknowledgement rule is met.

The network call is deliberately outside the SQLite transaction: holding a
database write transaction across an unavailable witness is a denial-of-service
and recovery hazard. The home writer fence prevents another local append from
creating ambiguity while the committed checkpoint is pending. A fenced remote
leader is required before the same home can be active on multiple hosts.

Multiple events in one operation may be batched into one checkpoint, but a
production event group is not acknowledged until that checkpoint is anchored.
The rule is especially load-bearing before:

- acknowledging grant, revoke, approval, ratification, or epoch transition;
- evaluating or compiling standing authority from a prefix;
- emitting a durable external-effect dispatch/reservation;
- claiming a production-complete export; or
- executing recovery that would consume ledger-derived authority.

If the database has advanced beyond the anchor, a protected read first anchors
the current head. If the anchor is ahead of local storage, protected work stops
and recovery begins. If both name different descendants of one parent, the
home is forked and no automatic winner is chosen.

## Migration of offset-consuming fields

Ratification must replace every normative cross-span integer offset with an
authenticated position, not merely change the event table:

```text
TracePosition { home, epoch, global_seq, event }
```

- `Manifest.trace.substrate_offset` becomes a position at the sealed step
  boundary.
- `Intent.captured_before` becomes a position; as today, the countersigning
  intent event remains the proof, while the field is a convenience claim.
- drift `between` endpoints become positions in one home/epoch.
- M7/A22 grant, revoke, approval, and effect comparisons use `global_seq` from
  one `VerifiedPrefix`; cross-epoch comparisons require explicit epoch lineage
  and never compare bare integers.
- approval headroom and ratchet inputs are bounded by an exact terminal
  checkpoint.
- operator lookup accepts event id or `TracePosition`; a legacy rowid flag may
  remain diagnostic but cannot enter exports or authority decisions.

Within-span branch-tip selection continues to use signed span `seq`. Global
position is used only when a normative edge crosses spans or needs an expected
terminal prefix.

## Decision-time and gate semantics

The current `VerifiedEventSnapshot { events, head }` becomes an authenticated
prefix view:

```text
VerifiedPrefix {
    home, epoch,
    events_in_global_sequence,
    terminal_seq, terminal_event,
    checkpoint, anchor_receipts,
    assurance
}
```

Construction verifies the epoch record, latest required receipt, checkpoint,
the global chain through the checkpoint, every per-span chain represented in
that prefix, and W-11's raw/index agreement. Decision and gate helpers accept
this type, not an event vector plus an integer.

For a protected decision, the home writer fence is held while the current head
is anchored and the prefix is evaluated. For the current no-actuation wedge,
an allowed in-memory ticket can still be stranded by a later revoke exactly as
A22 specifies. Before live external effects, the future dispatch record must
replace the ticket as the durable authorization offset.

Gate replay evaluates grant/revoke/effect order by signed `global_seq`. It may
approve pre-revoke work because the authenticated order proves the effect's
dispatch preceded the revoke. It rejects any effect beyond the checkpoint it
was asked to verify; a mutable rowid cannot move the boundary.

Standing-rule compilation consumes only an anchored prefix. Founding examples
outside that prefix, from a legacy segment, or from a local-integrity-only home
do not count toward `k`.

## Durable external-effect interaction

W-15 supplies order, not the entire P22 protocol. The required composition is:

```text
effect_intent
  -> authority reservation
  -> signed global dispatch event (idempotency key = event id)
  -> checkpoint + qualifying anchor receipt
  -> remote dispatch
  -> effect receipt
  -> state commit
  -> completion receipt
```

The authority reservation and dispatch event are created under the same home
writer fence used by revocation. Therefore exactly one of these orders exists:

- revoke first: the dispatch liveness check sees closure and no dispatch is
  recorded or sent; or
- dispatch first: its anchored record proves prospective authorization, and a
  later revoke does not rewrite history.

A crash after anchored dispatch but before sending is recovered by idempotent
send/query. A crash after sending but before effect receipt uses the same event
id to query or retry. A system that cannot provide idempotency or effect-status
reconciliation must classify the ambiguity honestly and apply its compensation
policy; it cannot issue a second fresh dispatch.

No remote effect is sent after merely signing or locally committing dispatch.
Otherwise a full-disk rollback could erase both the dispatch and its local
head, leaving a real effect with no durable authorization evidence.

## Crash and partial-publication recovery

Recovery begins by reading the anchor before trusting mutable local metadata.

| Crash point | Durable state | Recovery action |
|---|---|---|
| before local event transaction commits | old local/anchor head | retry from old head; no new event exists |
| after an ordinary/local-profile event commit, before checkpoint commit | globally chained uncheckpointed tail | verify tail; create the deterministic next checkpoint/outbox; no production acknowledgement or protected use yet; production authority operations commit checkpoint/outbox atomically with the event |
| after checkpoint/outbox commit, before anchor accepts | local pending checkpoint | idempotently publish by compare-and-swap |
| after anchor accepts, before local receipt commit | anchor ahead by the pending checkpoint | fetch current receipt; verify it names the local checkpoint; persist it |
| after receipt commit, before caller acknowledgement | fully durable operation | retry returns existing result/receipt; never append a semantically conflicting replacement |
| anchor ahead, local event/checkpoint missing | local loss or rollback | fail closed; restore a backup containing the anchored prefix, or enter explicit epoch-recovery ceremony |
| local and anchor are sibling descendants | fork/equivocation | quarantine both; require operator/security recovery; never choose longest/highest rowid |
| witness unavailable | freshness unknown | retain local narrowing; do not acknowledge anchor-required writes or perform protected effects/standing compilation |

Anchor publication is idempotent by checkpoint id. If an anchor has advanced to
a descendant while a stale process retries, the process must fetch and verify
that descendant from local storage; absence means it is stale or rolled back,
not permission to overwrite the anchor.

An epoch may be abandoned after unrecoverable data loss only through an
explicit recovery record that references the last anchor receipt and states
the gap. The new epoch does not make missing events reappear, and assurance
surfaces must display the discontinuity.

## Attack outcomes

| Storage attack | Required result |
|---|---|
| edit signed event or signed global fields | signature/id verification fails; no protected effect |
| change denormalized fields | W-11 mismatch failure; no protected effect |
| reorder SQLite rowids | no semantic change; verification/export uses `global_seq` |
| reorder global sequence columns only | mismatch or signed-chain failure |
| delete a middle event | next `global_prev`/sequence fails |
| delete every event in one span | global gap/head failure even though the span disappears from enumeration |
| delete latest authority event but retain later rows | global gap/prev failure |
| delete a suffix | trusted checkpoint/anchor terminal head is absent |
| restore old database/home image | current anchor is ahead; fail closed and recover |
| replay old valid receipt/status | challenge mismatch rejects stale status; a fresh anchor status exposes the current receipt |
| run two cloned writers | only one checkpoint descendant wins anchor CAS; loser stops as stale/forked |
| inject foreign/invalid rows | they contribute no authority; bounded verification reports corruption/DoS |
| remove anchor receipts locally | anchor re-fetch restores them; offline verification loses freshness label, never widens authority |

## Export and import

An export contains:

1. home identity and complete epoch lineage required for the exported range;
2. events sorted by signed `global_seq`, with per-span order derivable from the
   same set;
3. referenced signed objects and payload/CAS availability metadata;
4. the terminal checkpoint and all receipts needed to a configured trust
   anchor; and
5. an assurance label stating whether the terminal checkpoint was current at
   export time, merely caller-pinned, or local-integrity-only.

The manifest must name its exact terminal `(home, epoch, global_seq,
head_event, checkpoint)`. Filesystem order and SQLite rowids are irrelevant.
An exporter first synchronously anchors the head if it claims
`current-at-export`.

Import verifies before storing. A foreign export is read-only evidence under
its source home/epoch; it is never inserted into the local event chain with
renumbered sequence values. A receiving home may append a signed import
reference to the frozen source checkpoint; continuing the source home itself
requires source-authorized epoch continuity. Neither form can claim the source
witness attested to later receiver events.

Combining exports is a set of separately authenticated home/epoch streams, not
a fabricated total order between homes. Cross-home causality must be expressed
by signed references in later events; file concatenation order proves nothing.

## Anchor options and recommendation

All qualifying anchors implement conceptually:

```text
get(home, fresh_challenge) -> signed challenge-bound current status
compare_and_swap(home, expected_receipt, signed_checkpoint) -> receipt
```

They authenticate requests/responses, durably reject regression and siblings,
and retain enough history to diagnose a fork.

| Option | Full-home rollback detection | Portability / third-party proof | Cost and limitation | Disposition |
|---|---:|---:|---|---|
| `fabric.db` head table | no | no | rolls back with events | cache only |
| signed sidecar in home | no | self-contained only | rolls back with whole home | development integrity only |
| separate ordinary local file/keychain | deployment-dependent | weak | often backup-restorable; anti-rollback not implied | qualifying only with documented monotonic guarantee |
| TPM/secure-element monotonic state | yes against disk rollback | device-bound | wear, provisioning, recovery, clone semantics | valid offline profile behind interface |
| remote compare-and-swap witness | yes if independent | strong, receipt is portable | availability, privacy/cadence, service trust | recommended first production profile |
| public transparency log plus monitors | yes; strongest equivocation visibility | strongest | operational complexity and metadata exposure | optional higher-assurance profile |
| human-pinned export hash | for that frozen export | portable if pin is trusted | not a live-home freshness mechanism | supported verification input |

The remote witness should see only identifiers, sequence numbers, checkpoint
hashes, signer ids, and prior receipt links. It needs no event body, manifest,
capability, or payload. Deployments needing concealment may derive a
witness-scoped opaque home handle rather than expose the internal home id, so
long as binding is stable and signed.

Multiple anchors may countersign one checkpoint. Policy must identify which
set is required; "one of several happened to answer" must not silently weaken a
threshold. Multi-anchor quorum is a future deployment policy, not required for
the first implementation.

## Key rotation and historical verification

This protocol binds signer ids but defers trust in them to SI-27.

The minimum compatibility requirements for SI-27 are:

- the epoch record identifies the initial authorized fabric key;
- a rotation certificate is authorized by the old key or the user-root
  recovery authority, globally ordered, and anchored before an event under the
  new key contributes authority;
- verifiers retain or can obtain historical public keys and their validity
  intervals in global-sequence terms, not only wall-clock terms;
- loss recovery creates an explicit epoch transition from the last anchored
  checkpoint; it does not silently regenerate authority; and
- compromise recovery states which earlier signatures remain trusted. An
  anchor prevents rewriting before its checkpoint but cannot prove that a
  compromised signer used legitimately before that point.

Key rotation need not reset global order. An epoch transition is appropriate
when trust/recovery semantics change or when sequence exhaustion/migration
requires it; routine key rotation can remain inside an epoch if SI-27 supplies
the ordered certificate.

## Performance and denial of service

The current full-ledger scans per decision are not acceptable under hostile
storage or long-lived homes. The immutable chain permits safe incremental
verification:

- index `(home, epoch, global_seq)` uniquely and index event id;
- verify once from a trusted checkpoint and cache the verified prefix keyed by
  terminal event/checkpoint;
- on reopen, obtain a fresh challenge-bound anchor status first, then verify
  only the extension from the last locally cached checkpoint;
- keep derived grant/revoke/registration views keyed by the verified terminal
  checkpoint, so a cache never claims a head it did not verify;
- cap rows, bytes, JSON depth, ancestry depth, and witness response size per
  verification operation; and
- stream verification/export instead of materializing lifetime history.

The cache is disposable and untrusted. A forged cache entry cannot skip chain
verification from a trusted checkpoint. A production deployment may batch all
events committed by one step/tool operation under one checkpoint, but cannot
acknowledge an unanchored tail. Remote witness latency is therefore on the
commit path unless a qualifying local hardware anchor supplies the synchronous
receipt; asynchronous remote witnessing can then add portability and
equivocation evidence. The assurance label must state which anchor protected
the operation.

A storage attacker can still cause denial of service by deleting required
events or injecting excessive garbage. W-15 promises fail-closed integrity,
not availability under a writable-database adversary. Quotas, bounded parsing,
incremental reads, and precise recovery diagnostics keep that denial bounded.

## Migration from current databases

Existing event ids cannot gain signed global fields without changing every id
and signature. Their historical cross-span order also cannot be recovered as a
cryptographic fact. Migration therefore creates a new epoch rather than
rewriting old events.

1. Enter offline maintenance under the existing home trust boundary; prevent
   concurrent appends.
2. Run W-11 verification for every retained row and every discoverable span.
   Any failure aborts migration.
3. Deterministically stream the retained legacy event ids in observed SQLite
   offset order into a versioned `legacy_commitment` digest and count. Preserve
   the original rows unchanged.
4. Create and sign a migration epoch record containing that commitment and an
   explicit `legacy_order_unverified` assurance marker in the final ratified
   schema.
5. Append the first v1 globally ordered event at `global_seq = 0`, checkpoint
   it, and publish it to the configured anchor before reopening protected work.
6. From that point, all new events require v1 global fields. Mixed unmarked
   events in the new epoch fail closed.

The legacy commitment proves only "these exact retained ids in this observed
order were present at migration." It does not prove no earlier deletion or
reordering occurred. Pre-migration approvals, grants, revokes, and examples
remain inspectable historical evidence, but cannot supply standing authority in
the new epoch without a fresh, anchored authority act. In particular, migration
must not resurrect a capability whose legacy revoke may have been omitted;
existing capability ids are not carried forward as live authority.

A backup restored from before migration cannot independently reinitialize the
same home. The anchor already names the migration epoch, so reopen fails closed
and directs the operator to restore a post-migration backup or perform the
explicit recovery ceremony.

## Validation plan

Ratification must turn each invariant above into the G9 map for the touched
spec sentences. Every negative asserts that the protected call, promotion,
standing-rule compilation, export claim, or remote dispatch did not occur.

### Two-sided contracts

- valid interleaved spans produce one contiguous global chain and retain valid
  independent per-span chains;
- a clean reopen reaches the anchor head and protected decision succeeds;
- a correctly linked epoch transition and key transition verify;
- a receipted export round-trips without rowids and preserves the exact head;
- crash recovery completes every partial-publication state idempotently; and
- a pre-revoke anchored dispatch remains historically valid while a later one
  is denied.

Negative contracts cover each H invariant, especially home mismatch, epoch
substitution, global gap/duplicate/prev mismatch, missing expected head,
receipt replay, anchor-ahead rollback, sibling fork, unanchored authority use,
legacy evidence used for standing compilation, and dispatch before receipt.

### G10 authenticated-storage matrix

For each signed event mutate `home`, `epoch`, `global_seq`, `global_prev`, and
all seven W-11 selectors alone and in combinations. Reorder rowids. Delete a
middle event, tail, latest grant, latest revoke, latest dispatch, and every row
of one span. Restore old database-only and whole-home images. Inject foreign
epochs, old valid receipts, malformed JSON, duplicate sequences, and very large
invalid prefixes. Repeat at decision, gate, ratchet, recovery, export, and
durable-dispatch callers.

### Mutation lanes

Stable pure targets should include:

- global predecessor/sequence/home/epoch verification;
- expected-terminal-head enforcement;
- checkpoint-to-event binding;
- anchor compare-and-swap and receipt-chain verification;
- assurance gating for standing authority and dispatch;
- epoch transition and migration/legacy exclusion; and
- recovery-state classification.

Mutate every equality, predecessor comparison, sequence increment, assurance
branch, and fail-closed arm. Constructor-only failures are supporting evidence;
consumer-level protected-effect tests are the contract evidence.

### Crash and concurrency matrix

Inject failure before and after every numbered append/publication step, fsync,
anchor request, anchor acceptance, receipt persistence, caller acknowledgement,
dispatch send, and effect receipt. Restart after each injection and prove the
state converges to exactly one classified outcome. Run two processes and two
database clones from the same anchored head; exactly one descendant may advance
the anchor, and neither loser may dispatch or compile authority.

### Differential and model tests

Publish language-neutral v1 vectors only after SI-26 coordination. A reference
model folds epoch records, event global order, checkpoints, receipts, and
authority edges; generated histories compare runtime decisions to the model.
Deep/soak runs measure verification extension cost, checkpoint cadence, anchor
latency, receipt growth, and recovery under thousands to millions of events.

### Independent review

W-15 changes an authority surface and therefore requires independent-context
review before merge. The review brief must include SI-25, this ADR after
ratification, the complete G9 map, G10 results, mutation report, crash matrix,
and an explicit demonstration that full-disk rollback is detected only with a
qualifying external/monotonic anchor.

## Rejected alternatives

### Keep unsigned offset and sign periodic `(span, seq) -> offset` maps

This makes checkpoints large, leaves order between checkpoints dependent on
rowids, complicates append/recovery, and creates two ordering authorities. A
single signed global predecessor is smaller and gives every event one native
position.

### Only check offset monotonicity inside each span

This catches one class of rowid swaps but cannot authenticate inter-span order,
whole-span deletion, or rollback. It is defense-in-depth at best and is no
longer needed for authority once global sequence exists.

### Per-span heads in a Merkle checkpoint, without global event order

This can prove retained span completeness at checkpoint time but does not
define whether a grant/revoke/approval preceded an event on another span unless
the checkpoint also commits a global ordering map. Adding that map recreates a
global chain less directly.

### A signed head stored only in `fabric.db` or the fabric home

It detects edits before the presented head but rolls back with the database.
It cannot satisfy freshness or P15's old-image attack.

### Periodic remote anchoring with authority allowed in the unanchored gap

A revoke in the gap can be erased by rollback to the prior receipt, and an
effect can then fail open. Authority writes/reads and dispatch must force a
synchronous checkpoint; batching is safe only for claims that do not depend on
the unanchored tail.

### Timestamp ordering

Signed timestamps are claims, may collide or move backward, and SI-22 already
separates event time from substrate order. They cannot replace sequence and
predecessor linkage.

### Longest chain wins after a fork

Both branches can be validly signed, and an attacker can extend the harmful
one. Only the monotonic anchor (or an explicit operator recovery ceremony)
chooses continuity. Length is not authority.

### Re-sign all legacy events into the new chain

Re-signing converts an observation at migration into a false claim about
historic order/completeness, changes ids and references, and risks resurrecting
legacy authority. The migration commitment must remain explicitly weaker.

### Make checkpoints trace events

A checkpoint event would need to commit a head that either excludes itself or
creates a recursive id. A separate signed artifact is simpler; its anchor
receipt supplies the external publication edge.

### Require a public transparency log for every deployment

It provides strong equivocation visibility but imposes availability, metadata,
and operational costs inconsistent with offline homes. The protocol requires
monotonic semantics and portable assurance labels, allowing remote,
hardware-backed, and higher-assurance transparency profiles.

## Consequences

- All event append paths share a per-home global critical section.
- `offset` APIs become diagnostic compatibility surfaces, not security APIs.
- Authority consumers operate on `VerifiedPrefix`, which can be incrementally
  cached and is more precise than today's event vector/head pair.
- Production/standing authority needs an independent service or hardware
  dependency; this is unavoidable if full-home rollback is in scope.
- Offline operation without a qualifying local hardware anchor retains signed
  local integrity but loses freshness-dependent authority claims.
- Current authority does not migrate. Operators must mint/ratify again after
  the new anchored epoch.
- Export becomes deterministic and independently verifiable, but cross-home
  total order remains intentionally undefined.

## Human ratification choices

The following choices are intentionally unresolved until SI-25 ratification:

1. Is a remote compare-and-swap witness the required first production profile,
   or must the first release also support an offline TPM/secure-element anchor?
2. What is the maximum atomic event group/checkpoint size, and what commit-path
   anchor latency budget is acceptable? The candidate recommends no
   acknowledged unanchored tail rather than an operation allowlist.
3. Does one qualifying anchor suffice, or do high-risk deployments require a
   configured threshold of independent receipts?
4. What exact outage behavior applies while the anchor is unavailable? This
   must compose with the future per-capability `on_broker_outage` rule without
   silently replacing the project's loud fail-open product posture.
5. Does routine fabric-key rotation stay within an epoch, and which SI-27
   certificate authorizes it?
6. What signed object transcript/prefixes finalize `TraceEpoch`,
   `TraceCheckpoint`, `AnchorReceipt`, and `AnchorStatus` under SI-26?
7. What event or object vocabulary represents import continuation and recovery
   discontinuity in the ratified schema?
8. Are all pre-migration capabilities and standing examples invalidated, as
   recommended, or is there a separately human-ratified carry-forward ceremony?
9. Which production SQLite/fsync profile is normative across supported
   platforms, and how is durability conformance tested?

## Recommended ratification

Ratify the two-layer construction and H1-H15, with the remote
compare-and-swap witness as the first qualifying production anchor, all
pre-migration authority invalidated, and no acknowledged unanchored event tail
in the production profile. Keep the anchor interface generic so an offline
hardware profile can be ratified without changing event/checkpoint formats.

The outage rule and key-transition certificate should be ratified jointly with
their owning issues rather than improvised in W-15. Until those choices are
made, local-only signed checkpoints may be implemented only as explicitly
non-production scaffolding and must not close SI-25, RF-13, or P15.
