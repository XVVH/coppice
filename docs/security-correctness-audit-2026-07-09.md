# Coppice security and correctness audit — 2026-07-09

## Status

Originally a read-only audit of commit `7323fb8` against:

- `docs/agent-state-fabric-brief.md` v0.2
- `docs/asf-schema-spec.md` v0.5
- `AGENTS.md`
- the accepted ADRs and documented stage boundaries

No remediation was performed during the original review. A subsequent
dogfooding-readiness pass on branch `codex/dogfooding-audit-fixes`
(implementation commit `5350010`) addressed the findings that could be
corrected without prematurely fixing a protocol or post-dogfooding
trust-boundary decision. The original findings remain below as the audit
record; their current disposition is summarized here.

### Dogfooding-readiness remediation update

| Finding | Current disposition |
| --- | --- |
| Untraced branch state can be promoted | **Resolved.** Promotion now requires the immutable candidate root for every branch store to match the final signed `tool_call` root, or the manifest base when no call occurred. |
| Parked promotion contents can widen after preview | **Resolved.** The preview pins content-addressed branch roots; approval remerges current trunk only against those roots. |
| Approval budget meaning came from mutable escalation rows | **Resolved.** Signed approval events now carry the capability and caveat binding, and the gate verifies the substrate span before consuming them. |
| CAS and payload references were not verified on read | **Resolved for reference integrity.** CAS and decrypted payload bytes are rehashed before use. AEAD metadata binding remains a future format-version decision. |
| Tool objects were consumed without registration liveness/signature checks | **Resolved for the current tool-consumption paths.** Tool lookups now require a verified registration event and valid fabric signature. Universal liveness helpers for every future object reader remain follow-on hardening. |
| Attenuation expiry and child-manifest M1 defects | **Resolved.** Expiry is compared as an instant, malformed values fail closed, parent signatures are verified, and M1 is rechecked for the child manifest. |
| Ambient filesystem permissions and key handling | **Resolved for the dogfooding fabric boundary.** Fabric, CAS, and keystore directories are forced to `0700`; key and approval-socket files to `0600`; first-start key creation is exclusive; sensitive key buffers zeroize on drop. |
| Non-strict Ed25519 verification and unsafe snapshot paths | **Resolved.** Verification is strict and stored tree paths are validated before materialization. |
| Human approval authentication / agent sandbox boundary | **Accepted dogfooding limitation; post-dogfooding graduation gate.** |
| Crypto-shredding forensic guarantee | **Deferred.** Requires a storage/durability design that covers WAL, freelists, snapshots, and backups; logical dogfooding behavior is testable without claiming the stronger guarantee. |
| Brokered manifest authority semantics | **Deferred pending SI-21.** Correcting it changes the content-addressed object graph and requires a ratified cycle-breaking representation. |
| Tail anchoring, crash-atomic all-root commit, and broker-outage posture | **Deferred architectural work.** These remain production/live-egress gates, but changing their persistence and recovery protocols now would impede core dogfooding and invalidate observations if the protocols change. |
| JCS numeric interoperability and RF-6 type-prefix signature binding | **Deferred format-boundary work.** Resolve before published cross-implementation artifacts; changing signed bytes during core dogfooding would create avoidable incompatibility. |

## Executive summary

Coppice is a strong reference scaffold with unusually good invariant-driven
tests, but it should not yet be treated as fully spec-conformant or as a
production security boundary. At the audited commit, several central claims did
not hold end-to-end. Commit `5350010` closes the identified untraced-promotion
path, immutable approval scope, mutable approval-authority lookup, current tool
registration consumption, and content-reference integrity gaps. Forensic
crypto-shredding, authenticated human authority, trace head anchoring, and
crash-atomic multi-root state changes remain outstanding graduation work.

The current same-Unix-user trust boundary and cooperative-agent posture are
intentional dogfooding constraints, not accidental implementation drift. They
remain documented below because the controls that replace them are fundamental
post-dogfooding graduation requirements and must not be lost during later
sequencing.

The core algorithms are generally stronger than the orchestration around them.
Capability evaluation, conservative caveat handling, merge behavior, and strict
signature verification are well structured and well tested. The remaining most
serious risks appear at architectural commit boundaries: between filesystem
mutation and ledger mutation, between a local ledger and its durable external
head, and between logical deletion and every physical copy of encrypted data.

Recommended disposition: block production use and any live-egress integration
until the critical and high-severity findings below are resolved or explicitly
re-scoped by a ratified spec amendment.

## Findings

### Critical — untraced branch state can be promoted

**Remediation status: resolved on `codex/dogfooding-audit-fixes`.**

The promotion gate verifies the events that exist, then independently captures
and merges the current branch. It never proves that the branch root equals the
last recorded `state_root_after`. A direct branch edit, or a downstream tool
that completes after a crash or signal but before `record_result`, can therefore
auto-promote with no corresponding `tool_call` event.

The signal handler can invoke the gate while a downstream call is executing
because the broker lock is released during that call. Its comment says the call
is already traced, but result recording occurs only after the downstream
response.

Evidence:

- `crates/asf-cli/src/proxy.rs:423` — signal handler gates the session
- `crates/asf-cli/src/proxy.rs:513` — downstream execution precedes result recording
- `crates/asf-kernel/src/broker.rs:694` — gate verifies only recorded events
- `crates/asf-kernel/src/broker.rs:815` — merge captures the current branch independently

Impact: trace-vs-capability enforcement and M8 attribution can both report
success while untraced state reaches trunk. SIGKILL recovery has the same
exposure.

### Critical — a parked promotion's approved contents are mutable after preview

**Remediation status: resolved on `codex/dogfooding-audit-fixes`.**

A parked promotion stores branch paths and a JSON preview, not pinned branch
roots. Approval later recaptures whatever currently exists at those paths and
applies it without checking that the newly computed operation set is contained
by the reviewed preview.

Evidence:

- `crates/asf-kernel/src/broker.rs:115` — promotion rows store `branch_paths` and `preview`
- `crates/asf-kernel/src/broker.rs:650` — preview creation
- `crates/asf-kernel/src/broker.rs:954` — approval reloads paths and recomputes from live branch state

Impact: an agent, rogue process, or accidental edit can add arbitrary branch
changes after the human reviewed the card. The code comment asserting that the
approval-time remerge is "never wider than previewed" is true only for
trunk-side movement, not branch-side movement.

### Accepted staged limitation / future release gate — approval is not yet authenticated as human

This is an intentional property of the cooperative dogfooding phase, not a
current-stage defect. The finding records the boundary that must change before
the project graduates from dogfooding to a production or adversarial-agent
posture.

The Unix socket is topologically separate from MCP, but every request received
on it is labeled `local_session` without peer-credential validation, registered
channel lookup, proof of user presence, or a signed human response. The offline
CLI opens the fabric home directly and applies the same asserted provenance.

Evidence:

- `crates/asf-cli/src/proxy.rs:266` — approval socket
- `crates/asf-cli/src/proxy.rs:307` — commands are stamped `local_session`
- `crates/asf-cli/src/proxy.rs:609` — offline direct-database fallback
- `docs/dogfooding.md:67` — same-Unix-user collapse is acknowledged
- `docs/dogfooding.md:76` — same-user agents can reach the approval surface
- `docs/dogfooding.md:100` — v0 is explicitly cooperative-agent instrumentation

Present impact: any same-user process can approve its own escalation, so v0
measures a cooperative agent's experience rather than enforcement against a
compromised or non-cooperative agent. This is acceptable within the documented
dogfooding boundary.

Post-dogfooding graduation gate: before non-cooperative agents, live egress, or
production use, the architecture must supply all of the following:

- agent sandboxing that prevents direct access to trunk, session branches, the
  fabric home, the approval socket, and an unrestricted shell;
- broker-side validation that an approval came from a registered channel and
  authenticated human principal, with proof of user presence appropriate to
  the channel strength;
- separation of the user-root signing key from broker custody, so broker
  compromise cannot forge human authority acts;
- explicit filesystem and socket permissions plus local peer authentication;
- adversarial tests proving that an agent cannot self-approve, invoke the
  offline approval path, or bypass the broker through native state access.

### High — crypto-shredding does not reliably destroy the DEK

**Disposition: post-core-dogfooding architecture and release gate.**

Shredding deletes the wrapped-DEK row and clears ciphertext inside the same
WAL-mode SQLite database while the owner KEK remains persistent. Old wrapped
DEKs and ciphertext may survive in WAL files, freelist pages, filesystem
snapshots, or backups. Recovering both defeats shredding.

Evidence:

- `crates/asf-kernel/src/payload.rs:161` — row deletion and ciphertext clearing
- `crates/asf-kernel/src/kernel.rs:92` — fabric database uses WAL mode
- `crates/asf-kernel/src/keys.rs:114` — persistent owner KEK

The observed test database reported `journal_mode=wal` and
`secure_delete=FAST`. SQLite documents that `FAST` can leave forensic traces on
freelist pages: <https://www3.sqlite.org/pragma.html#pragma_secure_delete>.

Impact: the implementation currently provides logical deletion through SQLite,
not the spec's guarantee that DEK destruction renders content unrecoverable
everywhere at once.

### High — brokered manifests are recorded as observed runs

**Disposition: deferred pending ratification of SI-21.**

`step_boundary` always omits `authority`; the broker mints a capability only
afterward. Under M7, a manifest without `authority` explicitly denotes an
observed run in which no broker enforcement exists. Actual proxy sessions are
therefore mislabeled in their kernel object.

The live proxy also reuses one static generic intent, uses placeholder principal
public keys, and records `sha256:asfd-stage2` rather than a real behavior hash or
M4 runtime attestation.

Evidence:

- `crates/asf-kernel/src/kernel.rs:504` — manifest construction
- `crates/asf-kernel/src/kernel.rs:549` — authority explicitly omitted
- `crates/asf-cli/src/proxy.rs:192` — static identities and intent
- `crates/asf-cli/src/proxy.rs:215` — placeholder behavior bundle
- `docs/asf-schema-spec.md:94` — normative M7 meaning

There is a related spec-level issue: a content-addressed manifest referencing a
capability that references the manifest creates a hash cycle. The implementation
has silently selected one side of that cycle. Per the project instructions, this
requires a new spec issue beginning at SI-21 rather than an implicit exception.

### High — content-addressed data is not verified when read

**Remediation status: reference-integrity portion resolved; AEAD associated-data
format binding remains deferred.**

CAS reads return bytes without recomputing the requested hash. Payload reads
decrypt data but never verify that the plaintext hashes to the requested
`PayloadRef`. Payload hash, size, media type, and DEK identity are also not bound
as AES-GCM associated data.

Evidence:

- `crates/asf-kernel/src/snapshot.rs:114` — CAS `get`
- `crates/asf-kernel/src/payload.rs:116` — payload `get`
- `crates/asf-kernel/src/keys.rs:180` — DEK encryption without associated data

Impact: modifying a CAS blob silently changes restored state. Swapping payload
row linkages can make hash A resolve successfully to payload B. This breaks the
integrity meaning of state roots and payload references.

### High — trace tamper evidence is incomplete

**Remediation status: signed approval authority consumption resolved; durable
head/tail anchoring and database rollback detection remain deferred.**

Per-span verification detects altered events and middle deletions, but it cannot
detect tail truncation, deletion of an entire span, or rollback to an older
database copy because no signed or externally anchored head exists.
`verify_all_spans` enumerates only spans still present.

Gate budget accounting also reads approval events through `all_events` without
verifying the substrate span, then relies on mutable escalation rows to recover
what each approval meant.

Evidence:

- `crates/asf-kernel/src/trace.rs:236` — span verification
- `crates/asf-kernel/src/kernel.rs:765` — verification enumerates existing spans
- `crates/asf-kernel/src/broker.rs:698` — approval accounting consumes unverified events and mutable rows

Impact: a truncated ledger can still verify successfully, and authority
accounting can depend on data outside the signed event body.

### High — signed-object liveness is not enforced at use sites

**Remediation status: resolved for current tool consumption and manifest
signature checks at mint/promotion; universal object-registration liveness
remains follow-on hardening.**

The spec requires stored object rows to remain inert until their registration
event exists. Tool lookup instead scans raw `objects` rows without verifying the
tool signature or locating its registration event. Tool metadata controls
reversibility, store reach, action class, and write-path extraction.

Manifests used by minting, branching, promotion, and revert are likewise loaded
without consistent signature and liveness verification.

Evidence:

- `crates/asf-kernel/src/tools.rs:108` — raw tool lookup
- `crates/asf-kernel/src/trace.rs:301` — raw object retrieval
- `docs/asf-schema-spec.md:231` — normative materialized-view liveness rule

Impact: security-sensitive readers bypass the integrity mechanism that the
object format provides.

### High — all-root promotion and revert are not atomic

**Disposition: post-core-dogfooding persistence/recovery architecture.**

Stores are prepared first but committed sequentially. Filesystem restoration is
itself an in-place series of deletes and writes. A crash or later I/O failure can
leave vault and memory at different epochs. State is mutated before the
promotion or revert event and before expected-root attestations, which are also
written one store at a time.

Evidence:

- `crates/asf-kernel/src/snapshot.rs:407` — in-place filesystem apply
- `crates/asf-kernel/src/kernel.rs:687` — sequential multi-root revert
- `crates/asf-kernel/src/broker.rs:880` — sequential promotion apply
- `docs/asf-schema-spec.md:210` — coherent-revert atomicity requirement
- `docs/asf-schema-spec.md:215` — M8 check/merge/attestation atomicity requirement

The gate lock serializes cooperating fabric processes but cannot prevent a
human edit between `check_drift()` and the later trunk capture. SI-20's
unattributed-absorption window therefore still exists as a concurrent race.

Impact: the cross-layer consistency thesis holds on the successful happy path,
but not under crash, partial I/O failure, or a concurrent out-of-band edit.

### High — snapshot and branch confidentiality depends on ambient umask

**Remediation status: resolved for the dogfooding fabric boundary by restrictive
parent-directory and socket/key-file permissions.**

Fabric homes, CAS directories, branch directories, SQLite files, and plaintext
CAS blobs are created without restrictive modes. Under a normal `022` umask,
the audit observed directories at `0755` and CAS, branch, and database files at
`0644`. CAS and branch files contain plaintext vault and memory content.

Evidence:

- `crates/asf-kernel/src/kernel.rs:83` — fabric-home creation
- `crates/asf-kernel/src/snapshot.rs:85` — CAS creation
- `crates/asf-kernel/src/snapshot.rs:223` — branch materialization

Impact: on a system whose parent directories are traversable, other local users
can read data that the payload store otherwise treats as sensitive. This is
broader than RF-5's existing keystore-permission finding.

### Medium — attenuation retains two fail-open defects

**Remediation status: resolved on `codex/dogfooding-audit-fixes`.**

First, child expiry is compared lexically. A child ending at
`12:00:00.5Z` can pass under a parent ending at `12:00:00Z`, recreating the
subsecond boundary issue that RF-1 intended to eliminate.

Second, `attenuate` permits binding to a different manifest but never performs
M1 store-coverage validation for that new manifest.

Evidence:

- `crates/asf-kernel/src/capability.rs:263` — lexical expiry comparison
- `crates/asf-kernel/src/broker.rs:176` — attenuation without child-manifest M1 validation

Impact: a child can briefly outlive its parent, and an attenuated capability can
grant access to a store absent from its bound manifest.

### Medium — the JCS numeric constraint is not enforced

**Disposition: pre-publication interoperability gate, deferred during core
dogfooding.**

Fabric objects may be sealed and verified with floats or integers outside
`|n| < 2^53`; canonicalization performs no schema-level numeric validation.

Evidence:

- `crates/asf-kernel/src/canon.rs:33` — generic canonicalization
- `crates/asf-kernel/src/canon.rs:70` — sealing performs no numeric validation
- `docs/asf-schema-spec.md:11` — normative A16 constraint

Impact: cross-implementation object IDs and signatures may diverge, undermining
the neutrality and interoperability claims.

### Medium — the required broker-outage posture is absent

**Disposition: implement with live-egress integration, when degraded execution
semantics can be tested end-to-end.**

The governing instructions require fail-open behavior with loud,
ledger-visible degradation, invertible per capability. Current broker errors
return MCP tool failures, and broker or proxy outages remove the tool surface.
There is no `on_broker_outage` enforcement or degradation event.

Evidence:

- `crates/asf-cli/src/proxy.rs:531` — broker failures become tool errors
- `docs/agent-state-fabric-brief.md:184` — intended fail-open, loud posture
- `AGENTS.md:43` — current non-negotiable fail posture

Impact: the implementation currently fails closed and silently loses coverage
when the broker itself is unavailable, contrary to the stated market posture.

## Existing tracked findings confirmed

The original audit confirmed the following items in
`docs/review-findings.md`. Their disposition after the dogfooding-readiness
pass is:

- RF-4 — **resolved:** Ed25519 verification now uses `verify_strict`.
- RF-5 — **resolved for its recorded scope:** keystore permissions are private,
  key creation does not expose a chmod-after-write window, concurrent initial
  creation cannot overwrite a key, and sensitive buffers zeroize on drop.
- RF-6 — **deferred:** changing the signed byte format belongs with the
  pre-publication interoperability decision.
- RF-8 — **resolved:** snapshot tree entry paths are revalidated before
  materialization.

RF-8 becomes more consequential when combined with unverified CAS reads. RF-5
should be expanded during remediation to cover the plaintext fabric home, CAS,
and branches, not only the keystore.

## Cross-cutting themes

### Verification exists at creation but is not universal at consumption

The project creates signatures, hashes, and registration events, but multiple
security-sensitive readers trust stored bytes or mutable rows without
re-verifying them. The rule "verify immutable evidence immediately before use"
is not yet applied consistently.

### State mutation and ledger mutation are separate operations

Many findings share the same ordering problem: an external or local state
change occurs before its durable trace record, or the record is appended before
all expected-root state is durably updated. The trace consequently acts partly
as an after-the-fact narrative rather than as a commit protocol.

### Mutable pointers substitute for immutable evidence

Promotion paths, escalation rows, expected-root tables, and current
manifest/span metadata are mutable references used in decisions that the design
intends to be content-addressed and tamper-evident. Security decisions should
retain signed event IDs and pinned roots rather than mutable paths or lookup
rows.

### The effective trust boundary is the Unix account

Human, broker, agent, approval, and storage authority are still collapsed into
one operating-system user and one local filesystem hierarchy. This is an
intentional and coherent boundary for cooperative dogfooding, not a surprise
defect. It remains a cross-cutting audit theme because replacing it with the
architecture's intended principal-separated enforcement boundary is a
fundamental graduation gate, not optional hardening.

### Pure algorithms are tested more deeply than failure orchestration

The suite exercises caveat semantics, merges, signatures, and ordinary
round-trips well. The largest gaps occur under crash timing, signal handling,
partial I/O failure, rollback/truncation, concurrent mutation, and mutation
between preview and approval.

The shortest summary is: **Coppice is strongest inside modules and weakest
across commit boundaries.**

A unifying rule for a future remediation pass is:

> Every consequential decision consumes verified, immutable inputs; every
> consequential mutation has a durable pre-record and an unambiguous completion
> record.

## Verification performed

- 59 kernel unit tests passed.
- 13 broker integration tests passed.
- 5 kernel end-to-end tests passed.
- 10 promotion-gate tests passed.
- 4 property tests passed.
- 7 process-level proxy smoke tests passed outside the socket-restricted
  sandbox.
- Strict workspace Clippy passed with `-D warnings`.
- `cargo-audit 0.22.2` loaded 1,159 RustSec advisories and scanned 100 locked
  dependencies; it reported zero known vulnerabilities and exited successfully.

The resulting workspace run covers 98 passing tests. The passing suite is
evidence that the implemented happy-path, pure-function, and new fail-closed
regression contracts are stable. It does not negate the deliberately deferred
findings above, which concentrate in crash consistency, authority provenance,
forensic deletion, and format/protocol boundaries.

## Deliberately excluded from defect count

The following are declared future-stage work or named open problems and were
not counted as implementation defects in this report:

- Tier-2 and Tier-3 stores
- the R2 taint and read-authority family before live egress
- StandingRules and TrustRecords
- judge and clerk stages
- compensation fidelity grades
- multi-actor visibility policy
- domain taxonomy governance
- cross-run memory taint (A3)
- the interim `sha256:` state-root encoding pending the F2/CID checkpoint
- the same-Unix-user, cooperative-agent trust boundary during dogfooding

Their documented scope boundaries remain important: claims about current
security should not imply that these later-stage mechanisms already exist.
