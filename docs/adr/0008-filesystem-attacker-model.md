# ADR 0008 — Filesystem attacker model for store publication and read-back

**Status: PROPOSED — SI-32 ratification candidate, awaiting human
ratification. This ADR is not a specification amendment and authorizes no
implementation. SI-32 remains OPEN. The tier labels, the staged-bytes rule,
the per-kind entry rules, and the containment boundary below are candidate
normative language; none is normative until the human choices in
"Ratification decision points" are ratified and integrated into the schema
spec under the amendment discipline (candidate amendment id: A27). W-20 item
(3).**

## Context

The spec assumes content-addressed preparation and coherent restore but never
defines the filesystem adversary those operations run against. RF-20's
remediation (PR #43) implemented a publication discipline — retained verified
bytes, exclusive randomized no-follow siblings, rehash through the retained
handle, same-inode non-symlink recheck immediately before rename, fsync of
file and parent — **without a written threat model saying what those steps
must defeat**. Each review therefore re-derives the attacker and finds a new
residue (the RF-20 → RF-40 chain is five rounds of exactly this). SI-32 asks:
define the attacker in tiers, label every guarantee with the tier it holds
under, and state which publication-safety claims require containment before
they hold.

This candidate is grounded in a complete inventory of every filesystem
publication and authority-bearing read-back site (the "Publication site
inventory" appendix). The finding that motivates a written model: the code
already has one gold-standard publisher (`write_atomic_verified`, every guard
present) and three verifying read-backs (CAS, payload, recovery journal — all
rehash), but eleven other sites carry partial guard sets whose *sufficiency
cannot be judged without knowing which tier they must survive*. (Two verifying
read-backs — CAS and payload — rehash content; the third, the recovery
journal, verifies its Ed25519 signature.) Two are genuine gaps this model
surfaces (R7 unsigned-meta trust; the T2-unwinnable class); the rest are
tier-appropriate and this model is what lets us say so.

## The three-tier attacker model (candidate)

Each tier names a distinct adversary against fabric-home storage; every
guarantee in the spec and the code is labeled with the *weakest* tier under
which it still holds. Tiers are cumulative in capability but **not** in the
honest answer: T1 is defeated by cryptography (content-addressing for
substitution, A23 for rollback), T2 by topology, T3 is not an attack at all.

**Out of scope (the trust boundary, stated so the tiers are not read as
total).** A super-user or different-uid local adversary is **out of scope** —
it is strictly more capable than T2 and defeats T2's uid-scoped containment
answer by construction. The trust boundary is the Unix account (P7); nothing
in this model defends against local root. This is the deliberate boundary, not
a gap; naming it is what keeps "distinct adversary / cumulative" from reading
as a completeness claim it does not make.

**T1 — offline storage tampering between processes.** CAS blobs, branch
files, the SQLite databases, key material, or the recovery journal are
changed, substituted, or replanted while no fabric process holds them open
— the backup/restore case, the copied-home case, and the
same-uid-writes-while-idle case. The honest answer for **substitution,
corruption, and truncation** is **content-address verification on every
authority-bearing read-back**: a substituted or truncated byte string fails
its rehash and degrades to a denial, never an authority bypass. T1 is where
"verify what you read, never trust the pathname" wins, with **three
carve-outs** where content-addressing is not the answer: (a) the read target
is the trust root itself (keys — nothing to verify against); (b) an unsigned
index the code still trusts (the R7 gap); and (c) **rollback/replay to a
prior, genuinely-signed, genuinely-content-addressed whole-home state** —
where every blob rehashes correctly and every signature verifies, because the
old state was legitimately produced, so content-addressing is silent by
construction. Rollback's answer is **A23, not content-addressing**:
layer 1's authenticated monotonic `global_seq`/`VerifiedPrefix` detects
partial or internally-inconsistent rollback single-machine now, and the
graduation-gated external anchor (layer 2) detects **whole-home** rollback at
its gates — which content-addressing structurally cannot see. Under the
current **SU** posture the T1 tamperer and the operator are the same Unix
principal (so whole-home rollback is undetectable locally — the operator
controls any local head), making T1's live bite today *accidental* (backup
staleness, a copied home); its adversarial bite arrives at G-MULTITENANT /
when the home leaves the local filesystem boundary, and its confidentiality
half is P17/RF-15.

**T2 — active same-privilege writer across prepare→publish.** An adversary
of the *same* Unix uid holds an open descriptor, a hardlink, or a racing
thread across a publication's prepare→verify→rename window. Here **no
sequence of pathname checks can win**: the same-inode-before-rename recheck
narrows the race but a sufficiently determined same-uid attacker can still
swap an inode in the sub-microsecond gap, hold a descriptor that survives the
rename, or pre-plant a hardlink. The honest answer is **not a better check —
it is OS-enforced exclusion**: the fabric home reachable by exactly one
principal, and the agent reaching it only through the broker (the two-surface
convention made topology). T2 publication-safety claims therefore hold under
**COOP** (the agent is not adversarial) and **require W-4 containment** to
hold under **G-ADVERSARIAL**. This tier is P7's "trust boundary is the Unix
account," stated as a storage adversary.

**T3 — the legitimate concurrent human edit.** A human edits the vault while
the fabric is idle or between attested roots. This is **not an attack** and
must never be handled as corruption or silent loss: it lands in **M8 drift
attribution** (`human_local` quiet for the solo operator, `unattributed`
loud), and — during recovery's own downtime window — in the A24/R14
capture-and-attribute-before-restore path. A model that fails T3 closed
(bricking on an honest edit) or open (overwriting it unrecorded) is wrong in
both directions; T3's correctness criterion is *attributed, never lost*.

## Candidate normative rules (A27)

**A27.1 — The staged-bytes rule (normative; ratifies RF-20's discipline).**
A commit consumes bytes **verified in memory during prepare and never
re-reads mutable storage to source the bytes it installs**. Every
store-mutating commit path holds its verified image (fs: per-entry verified
content; sqlite: the whole verified image) in the prepared plan and installs
only those bytes; re-reading a CAS blob, a live file, or any pathname at
commit time *to obtain install bytes* is prohibited. Re-reading live storage
for **comparison or enumeration** is permitted and safe (the fs apply walks
the live tree to enumerate deletions and reads a live target only to skip an
identical-by-hash write — a mismatch triggers a write of already-verified
bytes, a match means the live bytes already equal the verified content):
the invariant is on the *provenance of installed bytes*, not on avoiding all
reads. This is the T1 defense at the write
boundary — it makes prepare the single verification point and commit a pure
function of already-trusted bytes. (As-built: `prepare_restore` retains
verified bytes; `commit_restore`/`write_atomic_verified` consume them; the
`RESTORE-INTEGRITY` contract proves a blob mutated after prepare cannot reach
the live store. This rule ratifies that as the general requirement, not an
implementation accident.)

**A27.2 — Verify-on-read-back (normative, tier-labeled).** Every read-back of
storage bytes that will *bear authority* MUST content-address-verify (rehash
to the requested address) before the bytes are consumed; a mismatch is a
loud denial, never a fallback. This holds against **T1**. Three classes are
explicitly outside "content-address-verify" and each carries its own rule:
- **Trust-root reads** (key/secret material): there is no address to verify
  against — the bytes *are* the trust root. These hold against T1 only by the
  storage boundary (0o600, symlink-rejecting open), and their substitution
  degrades to downstream signature failure, not silent authority. Labeled
  **T1-boundary**, not T1-cryptographic.
- **Signed-object reads** (events, manifests, capabilities, and the recovery
  journal / `state_change_recovery` txn): verified by Ed25519 signature, which
  subsumes content-addressing (A19). Holds against T1 substitution and, for
  order/completeness and the rollback carve-out (c), composes with A23's
  `VerifiedPrefix` and anchor.
- **Unsigned-index reads** (the R7 gap): the code reads store paths, spans,
  and other operational pointers from unsigned `meta`/index rows that a
  substituted `fabric.db` controls, while the *events* in the same DB are
  signature-verified. This is the one authority-bearing read-back that
  currently trusts a pathname under T1. **Candidate rule:** operational
  pointers consumed for authority (which store a capture/restore targets;
  which span is the substrate span) MUST derive from signed substrate or be
  bound into it, never from an unsigned row a T1 tamperer controls. Reach
  note: `substrate_span` is not merely a capture/restore pointer — it feeds
  broker authority evaluation (grant ordering, A22 closure), so the gap's
  reach is if anything understated. This is A23's seam from the read side: the
  anchor proves the head is current, but the store paths and span its events
  reference must themselves derive from signed substrate, or a T1 tamperer
  redirects them under an otherwise-fresh head. Surfaced as a gap (proposed
  RF-41); posture-bounded under SU (the tamperer is the operator) but a real
  T1 hole once the home leaves the boundary.

**A27.3 — Per-kind entry rules (candidate).** Publication safety is stated
per store kind because the fs tree and the SQLite file have different
substitution surfaces:
- **fs-tree stores:** capture and restore reject symlinks (a symlink is not a
  canonical entry — this composes with A24/R21's kind-complete recovery
  scan); the canonical grammar is regular files and directories only; the
  atomic publisher writes through an O_EXCL|O_NOFOLLOW randomized sibling
  and rechecks the same non-symlink inode immediately before rename. Excluded
  directories (`.git`) are never written, deleted, or (candidate correction)
  fsync-walked — the one inconsistency the inventory found (R3:
  `sync_store`'s fs walk omits the exclusion and the no-follow open) is a
  fsync-only path with no integrity impact but should be brought into line so
  "excluded means untouched" is uniform.
- **SQLite stores:** the whole file is the unit; the image is captured
  WAL-checkpointed (folded, no live WAL), published through the same atomic
  primitive, and its stale `-wal`/`-shm` sidecars removed after the swap so a
  restored image cannot be polluted by a leftover WAL. A symlinked store DB is
  rejected before use (`symlink_metadata` no-follow).
- **Hardlinks** (the filing's explicit third entry-kind ask): a hardlinked
  regular file is captured by *content* like any regular file (capture reads
  by value), so a hardlink to in-scope content adds no capture surface. A
  hardlink pre-planted at a *publication* target is a T2 same-uid act, not a
  pathname-checkable defense — and the atomic publisher already defeats the
  write-through-the-link variant structurally: it creates a randomized O_EXCL
  sibling and renames *over* the target, replacing the directory entry rather
  than writing through any existing link. So hardlinks need no new rule beyond
  A27.1/the atomic publisher (write-through variant, covered) and A27.4 (the
  pre-plant-and-race variant, COOP/W-4-bounded). Stated so the filing's ask is
  visibly closed, not silently dropped.

**A27.4 — The containment boundary (candidate, the load-bearing tier
statement).** Every T2 publication-safety claim is labeled **"holds under
COOP; requires W-4 containment at G-ADVERSARIAL."** No amount of pathname
checking is claimed to defeat a same-uid active adversary; the spec must not
imply it does. Concretely: `write_atomic_verified`'s same-inode recheck MUST be
documented as *race-narrowing, not race-closing* (as-built its comment claims
neither; this ADR is where the honest label originates), and the security
argument for it MUST name COOP as the assumption it rests on. This is the
honest sentence RF-20→RF-40 kept rediscovering the absence of.

## Residue disposition (inventory R1–R11 → tier → action)

| Site | Tier | Disposition |
|---|---|---|
| R1 `Cas::put` no fsync/O_EXCL | T1 (durability) + T1 (integrity) | Integrity held lazily by `Cas::get` rehash (A27.2). Durability fsync is **G-PRODUCTION** (DEBUG posture; same class as the WAL+NORMAL durability seam). Label, don't fix now. |
| R2 `materialize_*` plain write | T1 | Writes CAS-verified bytes into a **branch**, re-captured/re-hashed before bearing authority — outside A27.1's commit boundary. Holds; label as branch-scratch, not a publication. |
| R3 `sync_store` fs walk omits `.git` exclusion + no-follow | T1 | fsync-only, no integrity impact, but breaks "excluded means untouched" uniformity. **Candidate correction** (A27.3); small, file with the ratification or as a follow-up RF. |
| R4 `Cas::get` exists→read TOCTOU | T1 | Rehash makes substitution a denial. Holds (A27.2). |
| R5 `gate_lock` no O_EXCL/O_NOFOLLOW | T2 | Advisory flock; a pre-planted `gate.lock` symlink is a same-uid act → T2/COOP. Label; W-4 owns. |
| R6 key/secret read-back trusts pathname | T1-boundary | Inherent — the trust root. A27.2's trust-root class. Confidentiality is P17/RF-14. |
| **R7 unsigned `fabric.db` meta rows trusted** | **T1** | **The one genuine T1 authority gap.** A27.2's unsigned-index rule; **propose RF-41**, posture-bounded under SU, real at G-MULTITENANT. |
| R8 `write_private_atomic` rename target no-follow | T1-boundary | Plaintext secret, in-memory source; rename replaces a symlink node. Bounded; note under A27.3. |
| R9 journal/sidecar remove no symlink guard | T1 | Fixed paths; removes the link not a target. Minor; note. |
| R10 `capture_sqlite` read no O_NOFOLLOW | T1 | Fabric-internal path, trust-on-capture. Bounded; note. |
| R11 CLI/demo/tooling writes | n/a | Non-authority-bearing (sockets, agent working store, demo fixtures, reports). Out of scope; state so. |

## Ratification decision points (the human choices)

1. **Tier count and boundaries.** Three tiers as above, or split T1 into
   T1-offline vs T1-at-rest-confidentiality (the latter is P17/RF-15's, and
   this model currently folds confidentiality into T1's note rather than a
   fourth tier). Recommend: three tiers, confidentiality cross-referenced not
   re-tiered. Sub-choice: is the super-user/different-uid out-of-scope
   boundary (P7) stated correctly as a boundary rather than a gap?
2. **T1-rollback carve-out (the pre-review's blocking find).** Ratify that
   content-addressing answers T1 *substitution/corruption/truncation* but
   **not rollback**, whose answer is A23 (layer-1 `global_seq` now, the
   graduation-gated anchor for whole-home rollback) — and that under SU
   whole-home rollback is undetectable locally, an accepted accidental-only
   residual until G-MULTITENANT. This is the seam where SI-32 hands off to
   A23; ratifying it wrong (content-addressing "answers T1") ratifies an
   unsound tier. Recommend ratify as stated; the alternative is to declare
   rollback wholly out of SI-32's scope and purely A23's — cleaner boundary
   but leaves the SI-32 reader without the cross-reference.
3. **A27.1 staged-bytes as normative** — ratify RF-20's discipline as the
   general rule, or leave it implementation-internal? Recommend normative:
   it is the T1 write-boundary invariant and future stores (Tier-2/3) must
   inherit it.
4. **A27.2's unsigned-index rule and RF-41.** Is R7 a ratifiable gap to file
   now (recommend: yes, posture-bounded, carried when the home-boundary
   posture graduates), or folded into P17/RF-15's at-rest work? This decides
   whether SI-32 spawns a new RF or reuses one.
5. **A27.4 containment boundary wording** — the "COOP; W-4 at G-ADVERSARIAL"
   label on every T2 claim. Ratify as the standing sentence, or scope it
   per-site? Recommend standing sentence, since it is the same honest answer
   at every T2 site.
6. **R3 correction** — bring `sync_store` into the exclusion/no-follow
   discipline now (small), or file as a follow-up? Recommend follow-up RF
   (it is fsync-only; not worth growing a threat-model ratification with a
   mechanism change).
7. **Where this lands in the spec.** §5.3 (owned-state transition, where
   publication lives) plus a §9 fork note, or a new §-level "storage adversary
   model" subsection? Recommend a §5.3 subsection cross-referenced from §1
   (payload store) and §9 (F2 addressing), since publication is §5.3's and the
   tiers label claims spec-wide.

## Reserved seams (not resolved here)

W-4 owns the T2 containment topology (the sandbox whose only door is the
broker); this ADR states the *requirement* that T2 claims rest on it, not the
mechanism. P17/RF-14/RF-15 own at-rest confidentiality (T1's confidentiality
half). G-PRODUCTION owns durability (R1's fsync, the WAL+NORMAL seam). The F2
CID/DAG-CBOR transition (ADR 0002) changes the *addressing* of CAS blobs but
not this model — A27's rules are stated over content addresses generally, so
they survive the transition; the migration note belongs to W-6/F2, not here.
SI-26 owns the signed-transcript/type binding that would let A27.2's
signed-object class extend to the new §6.2 objects.

## Validation plan (candidate, on ratification)

Two-sided contracts per tier claim: a `STAGED-BYTES` contract (positive: a
commit installs the prepared bytes; negative: a blob/file/row mutated after
prepare cannot reach the live store — the existing `RESTORE-INTEGRITY` tests
are its core); a `READBACK-VERIFY` contract (positive: a valid address reads;
negative: every authority-bearing read-back rejects a substituted byte string
without effect — CAS, payload, journal, and the R7-fixed meta path once
RF-41 lands); and the per-kind entry negatives (symlink rejected at capture
and publication; sidecar-WAL removed; excluded-dir untouched including under
R3's correction). Every tier label in the ratified spec text carries a
conformance-sweep row naming its enforcing line or its RF/gate deferral. The
T2 claims explicitly carry *no* negative test that asserts race-closure —
their conformance is the documented COOP dependency plus W-4's future
topology, and the sweep says so rather than implying a test proves T2 safety.

## Rejected alternatives (candidate)

- **A single flat "trusted local filesystem" assumption** (what exists today,
  implicitly): it is why every review re-derives the attacker — an unlabeled
  claim cannot be checked, and the reviewer cannot tell a tier-appropriate
  gap from a real one.
- **Claiming pathname checks defeat T2**: the same-inode recheck is
  race-narrowing; asserting it closes the race is the SI-10/A21 lie surface
  (a guarantee enforcement does not provide), and it would let an
  actuation/G-ADVERSARIAL grant proceed on a false floor.
- **Folding T3 into the attacker model**: an honest human edit handled as
  corruption is the M8 gap A20 closed and A24/R14 hardened; re-tiering it as
  an attack would reopen that.
- **Deferring the whole model to W-4**: W-4 is the T2 *answer*, but T1 is
  live now (backups, copied homes, the R7 gap) and needs its cryptographic
  guarantees labeled independently of containment.
