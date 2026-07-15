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
publication and authority-bearing read-back site (the appendix at the end of
this ADR, with file:line anchors). The finding that motivates a written model:
the code already has one gold-standard publisher (`write_atomic_verified`,
every guard present) and three verifying read-backs — CAS and payload rehash
content, the recovery journal verifies its Ed25519 signature — but eleven other
sites carry partial guard sets whose *sufficiency cannot be judged without
knowing which tier they must survive*. Genuine gaps this model surfaces: R7
(unsigned-meta trust, proposed RF-41), the registered-SQLite symlink gap
(proposed RF-42), and the T2-unwinnable class; the rest are tier-appropriate
and this model is what lets us say so.

## The three-tier attacker model (candidate)

Each tier names a distinct adversary against fabric-home storage; every
guarantee in the spec and the code is labeled with the *weakest* tier under
which it still holds. T1 and T2 are cumulative in capability (T2 is T1 plus a
live descriptor/race); T3 is **orthogonal to both** — a legitimate edit is not
a weaker attacker but a non-attacker whose window (idle, between roots, or
*inside a publication*) cuts across T1/T2 (external review round 1, finding 1).
The honest answers do not share a mechanism: T1 is defeated by cryptography
(content-addressing for substitution, A23 for rollback), T2 by topology, T3 by
attribution — never by treating it as an attack.

**Out of scope (the trust boundary, stated so the tiers are not read as
total).** Two different-principal cases, correctly distinguished (external
review round 1, finding 3): a **local root / privileged-host** adversary is
strictly more capable than T2 and defeats its uid-scoped answer by
construction — **out of scope**; the trust boundary is the Unix account (P7),
and nothing here defends against root. An **ordinary (unprivileged)
different-uid** process is the opposite — it is *blocked* by the enforced
0700/0600 home permissions (the posture ledger's G-MULTITENANT boundary), so it
is *defended*, not out of scope, exactly as long as those permissions hold. The
out-of-scope line is root, not "any other uid"; naming it is what keeps
"distinct adversary / cumulative" from reading as a completeness claim it does
not make.

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
construction. Rollback's answer is **A23, not content-addressing**, and its
domain must be stated precisely (external review round 1, finding 2): layer 1's
authenticated monotonic `global_seq`/`VerifiedPrefix` detects only rollback
that is **inconsistent relative to a non-rolled-back expected terminal** — an
interior gap, or a reset that leaves a surviving caller pin ahead of it. A
*coherent suffix regression* — rolling `fabric.db` (and with it `global_seq`
and the local checkpoint) back to an internally-consistent older copy while no
external head or pin survives — verifies cleanly at layer 1; that is
**layer 2's** job (the external anchor / caller-pinned expected head), which
content-addressing and single-machine layer-1 both structurally cannot see. Under the
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

**The active-publication window (T3, unresolved — external review round 1,
finding 1; the item ADR 0007 R14 explicitly deferred here).** M8 covers edits
*between* attested roots and A24/R14 covers the crash-recovery downtime, but
neither covers an edit made **during a live promotion/revert**: the gate lock
(`kernel.rs`) serializes fabric *processes*, not a human with a text editor,
so a vault edit landing after the prepare-time capture-equals-`before` check
and before the apply's rename is overwritten by the rename with no CAS capture
and no drift event — T3's "never lost" violated inside the one window the model
had not addressed. This is **protocol-class** (its remedy either narrows the
topology — the human's edit surface is excluded from the publication window —
or adds a preservation/refusal step, a new commit point), so it is surfaced as
a decision point, not designed here. It is the direct continuation of the R14
note that named "the concurrent human-edit exposure … exactly SI-32 tier 3's
item — compose there."

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
- **Trust-root reads** (signing keys, KEK, and live credentials — kept
  distinct, external review round 1, finding 3): there is no address to verify
  against — the bytes *are* the trust root — so under T1 they hold **only by
  the storage boundary** (0o700/0o600 permissions, symlink-rejecting open), and
  0o600 does **not** constrain a same-uid T1 tamperer. Their substitution does
  *not* reliably "degrade to a signature failure": swapping a **signing key**
  *together with* the signed storage it verifies produces a **locally
  self-consistent forged home** (nothing external is trusted — SI-27's trust
  anchor is unresolved); swapping a **live credential** authenticates
  successfully *as a different account* rather than failing. So under same-uid
  T1 the trust root is **not defended by this model** — it is an
  **SI-27/RF-14-backed residual**, accepted under SU (the tamperer is the
  operator) and closed only when SI-27 lands an external trust anchor and RF-14
  moves key/credential custody behind an independent boundary. Labeled
  **T1-boundary (residual)**, never T1-cryptographic; whether to accept it now
  or scope T1 to a trusted verifier key is a decision point.
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
- **fs-tree stores:** the canonical grammar is exactly A24/R25's ratified
  tagged path-state domain — `absent | file(content hash, executable mode) |
  implicit-directory` (a directory is implicit as the proper ancestor of a
  tracked file; empty and untracked directories are outside the boundary), and
  any other kind (symlink, fifo, socket, device) is definitionally
  non-canonical (external review round 1, finding 5 — the directory/non-kind
  rules the filing asked for are A24's, carried here rather than re-derived).
  The rule per operation: **capture** rejects symlinks (`capture_fs`'s explicit
  `path_is_symlink` check) and folds implicit-directory structure by content;
  **recovery** runs A24/R24's kind-complete scan (every non-canonical live
  entry fails closed in place); **publication** writes through an
  O_EXCL|O_NOFOLLOW randomized sibling and rechecks the same non-symlink inode
  immediately before rename. Excluded directories (`.git`) are never written or
  deleted; the one inconsistency the inventory found — R3: `sync_store`'s fs
  walk omits the `.git` exclusion (and, unlike `capture_fs`, has no symlink
  guard at all) — is fsync-only with no integrity impact, and the fix is to
  bring it into line with capture's exclusion (the "no-follow open" framing is
  corrected: `capture_fs` rejects symlinks with an explicit check, it does not
  use an `O_NOFOLLOW` open, so no-follow on `sync_store` is *proposed
  hardening*, not an existing discipline `sync_store` alone violates).
- **SQLite stores:** the whole file is the unit; the image is captured
  WAL-checkpointed (folded, no live WAL), published through the same atomic
  primitive, and its stale `-wal`/`-shm` sidecars removed after the swap so a
  restored image cannot be polluted by a leftover WAL. **As-built gap (external
  review round 1, finding 4):** only the fabric's *own* `fabric.db` is
  symlink-rejected before use (`w13_validate_existing_fabric_home`'s
  `symlink_metadata` no-follow); a **registered** SQLite store (e.g.
  `db:memory`) is captured by `capture_sqlite`, which uses `is_file` →
  `Connection::open` → `fs::read`, **all symlink-following** — so a symlinked
  registered store DB redirects capture to an attacker-chosen database. The
  candidate rule is that *every* SQLite store path, registered ones included,
  is symlink-rejected before capture/open; the merged code does this only for
  `fabric.db`. Proposed **RF-42**, with a no-effect negative after ratification.
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
| R3 `sync_store` fs walk omits `.git` exclusion (no symlink guard) | T1 | fsync-only, no integrity impact, but breaks "excluded means untouched" uniformity. **Candidate correction** (A27.3): match capture's `.git` exclusion; the no-follow half is proposed hardening, not a capture discipline it violates. Follow-up RF. |
| R4 `Cas::get` exists→read TOCTOU | T1 | Rehash makes substitution a denial. Holds (A27.2). |
| R5 `gate_lock` no O_EXCL/O_NOFOLLOW | T2 | Advisory flock; a pre-planted `gate.lock` symlink is a same-uid act → T2/COOP. Label; W-4 owns. |
| R6 key/secret read-back trusts pathname | T1-boundary (residual) | The trust root; substitution can forge a self-consistent home or authenticate as another account under same-uid T1. A27.2's trust-root class; **SI-27/RF-14 residual**, accepted under SU. |
| **R7 unsigned `fabric.db` meta rows trusted** | **T1** | **A genuine T1 authority gap.** A27.2's unsigned-index rule; **proposed RF-41**, posture-bounded under SU, real at G-MULTITENANT. |
| R8 `write_private_atomic` rename target no-follow | T1-boundary | Plaintext secret, in-memory source; rename replaces a symlink node. Bounded; note under A27.3. |
| R9 journal/sidecar remove no symlink guard | T1 | Fixed paths; removes the link not a target. Minor; note. |
| **R10 `capture_sqlite` follows symlinks (registered stores)** | **T1** | **A genuine gap** (external review round 1, finding 4): `is_file`/`Connection::open`/`fs::read` all follow a symlink; only `fabric.db` is symlink-checked, not registered SQLite stores. A27.3's SQLite rule; **proposed RF-42**, posture-bounded under SU. |
| R11 CLI/demo/tooling writes | n/a | Non-authority-bearing (sockets, agent working store, demo fixtures, reports). Out of scope; state so. |

## Ratification decision points (the human choices)

1. **Tier count and boundaries.** Three tiers as above, or split T1 into
   T1-offline vs T1-at-rest-confidentiality (the latter is P17/RF-15's, and
   this model currently folds confidentiality into T1's note rather than a
   fourth tier). Recommend: three tiers, confidentiality cross-referenced not
   re-tiered. Sub-choice: is the root-vs-unprivileged-different-uid boundary
   (P7 / the permissions boundary) stated correctly — root out of scope, an
   unprivileged other uid defended by permissions?
2. **T1-rollback carve-out and its precise domain.** Ratify that
   content-addressing answers T1 *substitution/corruption/truncation* but
   **not rollback**, whose answer is A23 — layer 1 detecting only rollback
   *inconsistent relative to a surviving expected terminal*, and a coherent
   suffix regression (whole-DB rollback with no surviving pin) being layer 2's
   at its gates. Under SU it is undetectable locally, an accepted
   accidental-only residual until G-MULTITENANT. This is the seam where SI-32
   hands off to A23; ratifying "content-addressing answers T1" ratifies an
   unsound tier. Recommend ratify as stated; the alternative is to declare
   rollback wholly A23's and out of SI-32's scope.
3. **Trust-root substitution under same-uid T1 (external review finding 3).**
   Accept key/KEK/credential substitution as an **SI-27/RF-14 residual**
   (recommend: yes, posture-bounded under SU — the tamperer is the operator —
   closed when SI-27 lands an external anchor and RF-14 moves custody behind an
   independent boundary), or scope T1's guarantees to a *trusted verifier key*
   assumed outside the tamperable home. The first is honest about today; the
   second is the shape production takes.
4. **Active-publication-window human edit (external review finding 1;
   protocol-class).** T3's one uncovered window — a human edit during a live
   promotion/revert, overwritten by the apply with no capture or drift. Resolve
   by **topology** (the human edit surface is excluded from the publication
   window — e.g. the vault is not concurrently human-writable while a gate
   holds) or by a **ratified preservation/refusal protocol** (a new commit
   point that captures-or-refuses on an in-window edit, the A24/R14 shape
   extended to the in-process gate window). Protocol-class either way — file as
   an SI before implementation. Recommend: decide the topology question first;
   it may dissolve the protocol need.
5. **A27.1 staged-bytes as normative** — ratify RF-20's discipline as the
   general rule, or leave it implementation-internal? Recommend normative:
   it is the T1 write-boundary invariant and future stores (Tier-2/3) must
   inherit it.
6. **A27.2's unsigned-index rule and RF-41.** Is R7 a ratifiable gap to file
   now (recommend: yes, posture-bounded, carried when the home-boundary
   posture graduates), or folded into P17/RF-15's at-rest work? This decides
   whether SI-32 spawns a new RF or reuses one. (RF-42, the registered-SQLite
   symlink gap, is the same call — file now or fold.)
7. **A27.4 containment boundary wording** — the "COOP; W-4 at G-ADVERSARIAL"
   label on every T2 claim. Ratify as the standing sentence, or scope it
   per-site? Recommend standing sentence, since it is the same honest answer
   at every T2 site.
8. **G-PUBLISH containment mapping (external review finding 6; the filing's
   explicit ask).** The filing asked *which publication-safety claims require
   containment before G-PUBLISH*. The real fork: may the spec **publish the
   conditional T2 claims** (labeled "holds under COOP; W-4 at G-ADVERSARIAL")
   with W-4 deferred, or must **W-4 containment land before publication** so no
   published claim rests on an unbuilt topology? Recommend: publish the
   conditional claims with the label — the label *is* the honest disclosure —
   and record the determination as a **G-PUBLISH row in the posture ledger**
   (T2 publication-safety claims are conditional-published, W-4-gated), so a
   reader of the published spec sees the dependency.
9. **R3 correction** — bring `sync_store` into the `.git` exclusion now
   (small), or file as a follow-up? Recommend follow-up RF (it is fsync-only;
   not worth growing a threat-model ratification with a mechanism change).
10. **Where this lands in the spec.** §5.3 (owned-state transition, where
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
RF-41 lands); and the per-kind entry negatives (symlink rejected at fs-tree
capture and publication *and at registered-SQLite capture* once RF-42 lands;
non-canonical kinds — fifo/socket/device — fail closed per A24/R24; directory
path-states resolve per A24/R25; sidecar-WAL removed; excluded-dir untouched
including under R3's correction). Every tier label in the ratified spec text carries a
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

## Appendix — publication site inventory

Grounded against the PR #48 merge base. Every filesystem publication and
authority-bearing read-back, by artifact class, with file:line anchors;
`write_atomic_verified` is the gold standard the residue table measures against.
R-numbers match the residue disposition table.

- **Gold-standard publisher.** `write_atomic_verified` / `_with_hook`
  (`snapshot.rs:534`/`543`, the guard sequence in the hook variant):
  randomized `O_EXCL|O_NOFOLLOW` sibling, mode set, file fsync, retained-handle
  rehash-before-rename, same-inode non-symlink recheck, atomic rename, parent
  fsync. `commit_restore` Swap and `apply_fs_in_place` pass 2 route through it.
- **Verifying read-backs (authority-bearing).** `Cas::get` (`snapshot.rs:152`)
  and `payload::get` (`payload.rs:122`) rehash; `recover_pending_state_change`
  (`kernel.rs:816`) verifies the journal signature + type; key/secret reads
  (`keys.rs:186`) trust by pathname — the trust root (R6).
- **CAS blobs (R1, R4).** `Cas::put` (`snapshot.rs:128`): randomized tmp +
  rename, no fsync/`O_EXCL`/`O_NOFOLLOW`; dedup branch re-verifies via `get`;
  integrity enforced lazily on read.
- **Recovery journal (R9).** `publish_state_change_journal` (`kernel.rs:122`):
  `O_EXCL|O_NOFOLLOW`, fsync file + parent, fixed path (presence = pending bit).
- **Keys/secrets (R6, R8).** `Keystore::initialize` (`keys.rs:67`): staged dir +
  atomic rename; per-file `create_private_new` (`keys.rs:261`) = `O_EXCL` +
  0o600 + fsync.
- **SQLite (R7, R10).** ledger `fabric.db` WAL+NORMAL (`kernel.rs:432`);
  `w13_validate_existing_fabric_home` (`kernel.rs:328`) symlink-checks
  `fabric.db` **only**; `capture_sqlite` (`snapshot.rs:268`) follows symlinks
  (RF-42); meta `stores`/`substrate_span` read unsigned (`kernel.rs:505`,
  RF-41). Payload bytes live in `fabric.db`, none on the filesystem.
- **Branch / lock / sync / CLI (R2, R3, R5, R11).** `materialize_*`
  (`snapshot.rs:337`) plain write into branch scratch; `gate_lock`
  (`kernel.rs:1015`) advisory `flock`, no `O_EXCL`/`O_NOFOLLOW`; `sync_store` fs
  walk (`snapshot.rs:608`) omits the `.git` exclusion (R3); CLI/demo/tooling
  writes are non-authority-bearing (R11).
