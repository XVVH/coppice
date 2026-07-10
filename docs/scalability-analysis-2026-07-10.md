# Coppice scalability analysis — 2026-07-10

## Status and scope

This is a point-in-time architecture discovery note for Coppice at commit
`97e2b37`, reviewed on 2026-07-10. It assesses:

- suitability for growth in users and registered tools;
- availability and failover characteristics;
- likely performance hot spots;
- superlinear or otherwise excessive growth paths; and
- natural shard boundaries.

This is not an ADR, benchmark report, implementation plan, or proposal to relax
an ASF invariant. No fixes are selected here. Complexity estimates are based on
code and design inspection rather than load testing.

The governing references are `agent-state-fabric-brief.md` v0.2 and
`asf-schema-spec.md` v0.5. The schema spec wins wherever this note and the spec
appear to disagree. The implementation-status boundary in `AGENTS.md` also
applies: the current build is a dogfooding reference implementation, not a
production multi-tenant or adversarial security boundary.

## Executive assessment

The ASF model has a credible route to scaling across many independent users and
tools. Its strongest natural partition is a **fabric home**: one owner or
collaborative workspace, its coherent state roots, keys, trace, payloads, CAS,
capabilities, approvals, and promotion gate. Independent homes have little
runtime coupling and can eventually be distributed across processes or hosts.

The current Coppice implementation is much narrower. It is effectively a
single-operator, single-active-session system per fabric home. It has local
crash-recovery mechanisms, but no high-availability topology. Per-call full
state capture, a single SQLite control/payload database, lifetime ledger scans,
materialized branches, and a home-wide gate will limit throughput inside one
home well before the manifest and capability schemas become the limiting
factor.

The main distinction is therefore:

| Dimension | ASF architecture | Current Coppice implementation |
| --- | --- | --- |
| Many independent users | Natural horizontal partitioning by home | Separate manually operated home/process per user or workspace |
| Multiple users sharing one state set | Schema anticipates humans and agents as principals | Multi-actor visibility and roots are deferred |
| Many tools | Immutable, versioned, compositional registration model | Two trusted compile-time profiles and one downstream tool ref per session |
| High call throughput in one home | Possible with a different runtime/storage implementation | Serialized request loop plus full-state capture |
| High availability | Compatible with fenced ownership and replicated durable state | Not implemented |
| Long-lived storage | Content addressing and payload TTL model are helpful | No CAS/branch GC or automatic payload-retention worker |

## Current deployment and data shape

A fabric home currently contains:

- `fabric.db`, a SQLite database holding events, signed objects, mutable
  materialized metadata, payload ciphertext and DEKs, expected roots, meters,
  escalations, exemptions, and parked promotions;
- a local plaintext content-addressed snapshot store under `cas/`;
- local signing keys, the owner KEK, and credential storage under `keys/`;
- materialized session branches under `branches/`;
- a host-local gate lock and approval Unix socket; and
- references to the live Tier-1 roots that form trunk.

One proxy process creates a manifest and capability, materializes every root
into a session branch, spawns one downstream MCP server, brokers its calls, and
promotes or parks the branch at session end. The vault profile coordinates a
filesystem vault plus a SQLite memory database. The workboard profile
coordinates an opaque SQLite work database plus a filesystem evidence root.

This is a good shape for proving the kernel invariants at small scale. It is not
yet a multi-tenant service shape.

## Suitability for growth in users

### Independent users or workspaces

This is the favorable case. A separate fabric home gives each user or workspace
its own:

- root of authority and key material;
- state-coherence boundary;
- event and payload retention domain;
- capability meters and approval queue; and
- serialized promotion gate.

Homes can run independently. A failure, hot ledger, large vault, or expensive
promotion in one home need not affect another home once placement and routing
exist. This gives the architecture a credible horizontal scaling model for a
large number of small, independent tenants.

The current code does not supply the surrounding fleet machinery: tenant
provisioning, placement, routing, quotas, replication, backup policy, metrics,
or remote key custody. Scaling by home is therefore an architectural property,
not a currently packaged hosted service.

### Multiple concurrent sessions in one home

This is currently unsuitable.

`current_manifest` and `current_span` are mutable home-wide metadata. A later
session can replace those pointers while an earlier proxy remains alive. The
first session's manifest-bound capability will then fail the M2 check, or later
recording risks consulting global state that no longer describes that session.
The broker object is also protected by one process-wide mutex.

Promotion, approval-time re-merge, and revert deliberately serialize through a
single gate lock per fabric home. That serialization is correct under M8: the
gate consumes and attests the full root tuple. It does mean write concurrency in
one coherence domain ends at a single commit point.

Concurrent branch work could eventually be supported, but only if session
identity ceases to depend on global current pointers and the system accepts that
all completions still queue at the home gate. Whole-store SQLite roots will
produce conflicts whenever both trunk and branch change, even if their logical
rows are unrelated.

### Multiple humans sharing state

The schema already represents humans and agents uniformly as principals, and
the key model reserves a mechanism for asymmetric visibility. The policy is
explicitly deferred. Current dogfooding therefore requires a single human and
attributes out-of-span local drift to that human by default.

Adding a second person is not simply a capacity change. It activates unresolved
questions about:

- multi-actor roots and ownership;
- event and payload visibility;
- approval authority and user presence;
- attribution without surveillance; and
- shared versus per-person grants.

Those are correctness and product-policy questions, not database-sharding
details. A shared home should remain a later scaling stage.

## Suitability for growth in tools

The schema-level tool model scales better than the current runtime:

- registrations are immutable and versioned;
- action metadata is normalized into common dimensions;
- conservative defaults handle missing reversibility, egress, sensitivity, and
  domains;
- capabilities use one conjunctive grammar across tools; and
- registered definitions are signed objects that can eventually be cached.

The current proxy supports two trusted, compile-time profiles: vault and
workboard. A profile chooses one tool reference, one downstream process, its
store topology, and the default session caveats. There is no generic third-party
registration or multi-server routing layer yet.

Tool growth also exposes a current hot path. Every action lookup reconstructs
the live tool set by reading all events, discovering registration spans,
verifying those spans, loading registered objects, and then finding the action.
`tools/list` repeats action lookup while filtering the downstream catalog. With
`T` advertised actions and `E` lifetime events, catalog filtering can approach
`O(T * E)` work in the current shape.

The immutable model is cache-friendly, so this is an implementation bottleneck
rather than a schema defect. Domain taxonomy governance is a more fundamental
tool-scaling dependency: portable trust cannot safely scale across third-party
tools if broad or vendor-invented domains flatten the anti-trust-farming
boundary. That remains an explicitly named open problem.

## High availability

### What exists today: local resilience

Several choices help a single home recover on the same machine:

- SQLite WAL protects normal database crash recovery;
- snapshot and payload references are rehashed on read;
- CAS objects and signed fabric objects are immutable;
- session-live markers allow a later process to find and gate stranded
  branches; and
- catchable signals attempt to gate before process exit.

These are valuable restart and integrity properties. They are not service
availability or failover.

### Current single points of failure

| Component | Current availability consequence |
| --- | --- |
| Proxy/broker process | Brokered tool surface disappears; calls fail closed |
| Downstream child process | The active call fails or stalls; the proxy loop cannot make progress on another call |
| `fabric.db` | Events, objects, payloads, meters, approvals, and home metadata are all unavailable |
| Local CAS/branches | Snapshot, promotion, revert, or recovery can fail |
| Local keys and KEK | Signing or payload resolution becomes impossible |
| Approval Unix socket | Live approvals are unavailable; offline CLI still depends on the same home and database |
| Host-local `flock` | Provides no fencing between replicas on different hosts |
| Live Tier-1 roots | Loss or corruption cannot be repaired without an independent backup or replica |

There is no database replication, CAS replication, standby broker, remote lease,
distributed gate fencing, service discovery, external trace-head anchor, or
automatic failover.

### Commit-boundary risk

Promotion and revert prepare all roots before mutation, but commit them
sequentially. Filesystem apply is itself an in-place sequence of per-file
changes. The promotion/revert event and expected-root updates happen after
state mutation and are also separate writes.

A crash or later I/O failure can therefore leave roots at different epochs.
The operation is coherent on the successful path, but not crash-atomic across
all roots. This is already recorded as a production release gate in the
security/correctness audit.

Failover should not be layered on top of this unchanged. A standby needs a
durable way to distinguish:

- a gate that never started;
- a gate that staged but changed nothing;
- a partially applied multi-root gate;
- state applied but not attested in the ledger; and
- a fully committed gate whose acknowledgement was lost.

### Broker availability and in-flight effects

The current broker keeps allowed-but-not-yet-recorded calls in an in-memory
`pending` vector. A process failure loses those tickets. For local Tier-1 tools,
branch-tip verification prevents untraced branch state from silently reaching
trunk. For future external tools, however, the remote side effect may already
have happened before the result could be recorded.

Active-active or automatic retry will therefore require durable call identity,
idempotency semantics, and a decision about how the broker records intent,
dispatch, remote acknowledgement, and trace completion. This is also where the
deferred compensator/fidelity work becomes operationally important.

The intended market posture is loud, ledger-visible fail-open degradation,
invertible per capability. The current implementation instead fails closed
when the broker or proxy is unavailable. The documented deferral is appropriate
for local-only dogfooding, but it means current availability behavior should not
be mistaken for the intended production behavior.

### Likely HA topology

The architecture points more naturally to **active-passive ownership per fabric
home** than to unrestricted active-active mutation:

- many homes can be active across many nodes;
- one fenced leader owns the gate for a particular home;
- broker evaluation may later be replicated if meters and exemptions are
  linearizable per capability;
- immutable CAS and payload objects can be replicated independently;
- event spans can be distributed, subject to the home-ordering constraints
  described below; and
- failover must recover or complete any durable gate protocol before accepting
  new promotion work.

This preserves M8's coherent tuple while still scaling fleet-wide.

## Performance hot spots

### 1. Full-state capture on every successful tool call

`record_tool_call` stores the arguments and result, then captures every
registered branch store. This happens for read-only and write calls alike.

Filesystem capture walks the entire tree, reads every file, hashes it, and
constructs a flat tree object. Reusing an existing CAS address also re-reads and
rehashes the CAS blob for integrity, so unchanged data still creates substantial
source and CAS I/O.

SQLite capture opens the database, forces a checkpoint, reads the whole main
file, and hashes/stores the byte image.

If `C` is successful call count and `B` is the total bytes across coordinated
stores, the current capture cost is approximately `O(C * B)`, plus directory
walking and tree serialization. This is expected to dominate latency as state
grows.

### 2. Repeated captures at boundaries and gates

A step boundary calls drift detection, which captures every live store, and
then captures every store again while constructing the manifest. Promotion
captures live trunk during drift detection, branch roots when computing the
merge, and trunk roots again during the merge. Approval-time re-merge repeats
the live-root work.

Content addressing avoids storing duplicate bytes, but it does not eliminate
the repeated scan, read, checkpoint, canonicalization, and hashing cost.

### 3. Materialized branches

Creating a branch materializes the complete root tuple into ordinary files.
This is simple and correct for dogfooding, but session startup performs work
proportional to total state size. Disk use is roughly:

`branch bytes = branch count * materialized state size`

Branches are required to persist for parked promotions. The current
implementation also has no general cleanup or GC path for completed branches.

### 4. Single SQLite writer and mixed workloads

One `fabric.db` contains append-only trace records, signed objects, payload BLOBs,
DEKs, mutable expected roots, meters, exemptions, escalations, and promotions.
SQLite WAL permits readers alongside a writer but retains one write-serialization
point per home.

Large payload writes, event append, budget accounting, approvals, and promotion
metadata therefore contend in one database. Payload growth also increases backup,
checkpoint, and recovery cost for the control plane because the bulk data and
control metadata share a file.

### 5. Serialized proxy request loop

The proxy can route an intercepted response by JSON-RPC id, but the main loop
waits synchronously for the downstream response before reading the next client
request. Effective tool-call concurrency is one per proxy. A slow downstream
tool also blocks unrelated calls in that session.

The shared broker mutex separately serializes broker state changes and approval
operations inside the process.

### 6. History-wide scans on authorization and gate paths

Current examples include:

- tool action lookup reconstructing registration liveness from all events;
- promotion scanning all events for signed approval headroom;
- promotion verifying the entire fabric-lifetime substrate span;
- approval-time minimum-auth checks scanning all capability objects; and
- ledger rendering loading every event into memory.

An individual operation becomes `O(E)` in lifetime event count. If one such
scan happens for each newly appended call or run, cumulative work over the
home's life trends toward `O(E^2)`.

The trace table currently has a useful `(span, seq)` index, but not indexes or
materialized views for several of these lifetime queries. Signed immutable
evidence can support indexed materializations as long as consumers still verify
the evidence at the security boundary.

### 7. Filesystem merge algorithms

Three-way merge over path maps is broadly `O(F log F)` for `F` paths, but two
supporting routines are quadratic in adverse cases:

- rename/move detection repeatedly counts matching hashes across add/delete
  lists, approaching `O(A * D)`; and
- tree composition searches up to three entire source entry arrays for every
  merged path, approaching `O(F^2)`.

These are acceptable for a small note vault but will become visible in very
large repositories or during bulk reorganizations.

### 8. Human approval throughput

The human is a deliberate final authority rather than an implementation detail.
Batch escalation reduces notification count, and the caveat ratchet is intended
to compile repeated approvals into deterministic rules. That ratchet is not yet
built.

Cold start, a large tool catalog, many new users, or frequent relevant skill
version changes can therefore create approval load proportional to the active
user/tool/workflow frontier. Domain-scoped behavior pins prevent unrelated
skill changes from invalidating every rule, but organizational approval capacity
will still be a real scaling limit.

## Excessive and superlinear growth paths

Most ASF records grow linearly with actual work. The current implementation has
several multiplicative or superlinear paths:

| Path | Growth characteristic | Cause |
| --- | --- | --- |
| Structural trace | `O(events)` indefinitely | Structural records retain permanently |
| Payload storage | `O(unique argument/result bytes)` until shredding | No automatic TTL worker yet |
| Flat filesystem trees | `O(changed snapshots * file count)` metadata | Each root stores a complete entry list |
| SQLite CAS data | `O(changed captures * DB size)` | Whole-file byte-image addressing |
| Materialized branches | `O(branches * state size)` | Physical branch copies and no general GC |
| Repeated lifetime scans | Cumulative `O(events^2)` | One `O(E)` scan on each new call/run |
| Rename and tree composition | Worst-case `O(files^2)` | Repeated linear searches |
| Tool listing | Up to `O(tool actions * lifetime events)` | Per-action reconstruction of live registrations |

### Actual exponential cases

There is no broad inherent exponential growth in manifests, trace events,
capabilities, or trust records. Two cases deserve explicit attention:

1. The recursive path-glob matcher branches at every `**` between consuming
   zero path segments and consuming one. Patterns containing multiple `**`
   segments and a failing suffix can produce exponential backtracking in pattern
   and path length. Current simple `**` grants behave effectively linearly, but
   a larger ratcheted or third-party tool ecosystem makes this a potential denial
   of service surface.
2. A delegation tree with branching factor `k` and depth `d` naturally contains
   up to `k^d` delegates. This is workload fan-out rather than accidental ASF
   metadata amplification. The current materialized-copy branch strategy makes
   Coppice pay close to full state materialization for every node, so deep agent
   fan-out would expose it quickly.

### Retention and garbage collection

The schema has an appropriate distinction: small structural evidence persists,
while sensitive payloads have TTLs and become tombstones after DEK destruction.
The current implementation has manual logical shredding but no scheduler for
retention classes. CAS GC is future work, and it must treat manifests, reverts,
parked promotions, branches, and retained replay windows as roots.

Without those lifecycle processes, a long-running home grows monotonically even
if its live user state remains stable.

## Natural shard boundaries

### 1. Fabric home / state-coherence group

This is the primary and safest shard boundary. It contains one coherent root
tuple and its authority, trace, payload ownership, keys, and gate. Route the home
to one owning placement, and scale the fleet by distributing homes.

A home does not have to equal one person forever. It is better understood as a
**coherence group**: the smallest unit whose roots must promote and revert
together. Independent projects or workspaces should use separate homes when
they do not require atomic cross-project undo.

### 2. Manifest, capability, and session

Call evaluation is largely local to a manifest-bound capability. Run-window
meters and exemptions are also capability scoped. Broker workers can therefore
be partitioned by `(home, capability)` if they share a linearizable store for
metering and durable call state.

Session traces already use independent hash chains. Their execution work can be
parallel even though successful completion converges on the home gate.

### 3. Trace spans

Per-span hash chains are a natural event-storage partition. Most run events can
be routed by `(home, span)`.

The complication is ordering outside a single run. Intent capture uses a
substrate offset as pre-contamination evidence, and registrations, approvals,
drift, and other home-lifetime events share the fabric-lifetime span. A sharded
trace therefore still needs a per-home ordering or anchoring mechanism for:

- the fabric-lifetime chain;
- intent `captured_before` proofs;
- exact drift windows; and
- materialized-object liveness.

This is not a reason to avoid span partitioning, but it prevents treating the
event store as completely order-free.

### 4. CAS objects

CAS blobs are naturally distributable by `(tenant, hash prefix)` and can live in
an object store. The planned CID/DAG-CBOR data plane strengthens this boundary by
making trees and large SQLite values chunk-addressed.

Tenant scope matters. Global plaintext-hash deduplication can disclose whether a
known payload or state blob exists in another tenant. Encryption, retention,
forensic erasure, and residency requirements may also differ per owner.

### 5. Payload content and key metadata

Bulk encrypted payload ciphertext can be separated from the control database and
partitioned by owner and payload hash. DEK metadata and tombstones need a durable
owner-scoped key service. Crypto-shredding semantics require deletion to cover
replicas, backups, caches, and key-service history rather than only the primary
row.

### 6. Tool catalog

Immutable tool definitions can be stored and cached globally by tool ID/version.
Per-home registration, countersignature, availability, and standing grants form
a tenant overlay. This avoids rebuilding the global definition for every call
while preserving the spec rule that an object is live only through its
registration event.

### 7. External mirrors and connectors

Tier-3 mirrors naturally partition by tenant, provider, and external account.
Provider-specific workers can enforce API rate limits and mirror freshness.
Broker-verified guards then route to the authoritative mirror/account shard and,
where required, perform a live spot-check.

### 8. Trust records and standing rules

The intended identity `(principal, domain, skill version)` is naturally
partitionable by organization and domain. The underlying evidence remains
references to signed events in trace shards. Keeping trust non-scalar prevents a
global reputation service from becoming both a semantic and scaling bottleneck.

### Boundaries that should not be split casually

The root tuple in one manifest is not a safe independent-write shard boundary.
M8 requires the gate to inspect, merge, apply, and attest the whole fabric-home
tuple coherently. The spec explicitly rejects per-store gate locking because it
can produce deadlocks or attest combinations that never existed together.

If a deployment outgrows one gate, the preferred first question is whether the
state can be separated into independent coherence groups. Splitting roots that
must still share atomic undo introduces a distributed commit problem and should
not be hidden behind ordinary store sharding.

Opaque SQLite stores are also indivisible under SI-18. A row-level or logical
shard is possible only after its authority, merge, conflict, and revert semantics
are explicitly ratified; it must not be inferred from database convenience.

## Scaling triggers and measurements

Existing project tripwires remain appropriate:

- CAS growth above roughly GB/month, memory DB above roughly 100 MB, or noisy
  logically-idle SQLite roots triggers the F2 chunked CID/DAG migration;
- routinely large or conflict-prone promotions trigger a finer checkpoint/session
  boundary; and
- frequent whole-store SQLite conflicts are evidence that the opaque merge unit
  is too coarse, not permission to invent row merges.

Additional measurements that would make the next scaling decision evidence-led:

| Signal | What it reveals |
| --- | --- |
| Capture milliseconds and bytes hashed per tool call | Whether snapshot work dominates user-visible latency |
| Source bytes read versus new CAS bytes stored | How much dedup saves storage but not I/O |
| Events scanned per authorization and promotion | When materialized indexes/caches become necessary |
| Promotion duration versus path count and changed-path count | When quadratic merge helpers become visible |
| Branch bytes and branch age by status | GC pressure and parked-promotion retention cost |
| `fabric.db` payload bytes versus control-plane bytes | Whether payload separation should precede other database work |
| Concurrent sessions attempted per home | Demand for session-local state and gate queuing |
| SQLite conflict and stale-revision incidence | Whether opaque store semantics fit the workload |
| Approvals and escalations per user-hour | Human throughput and ratchet urgency |
| Broker/downstream failure and recovery outcomes | Required HA and durable in-flight-call semantics |

Several qualitative events are immediate architectural triggers rather than
mere performance signals:

- a second concurrent session in one home requires session-local manifest/span
  state;
- a second human in one coherence group requires the multi-actor visibility and
  authority design;
- any availability commitment beyond manual restart/restore requires a durable,
  fenced multi-root gate recovery protocol;
- the first live-egress tool still requires the R2 read-authority family and an
  honest treatment of the A3 cross-run memory-taint gap; and
- a third-party tool ecosystem requires domain taxonomy governance and generic
  registration before portable trust can safely consume those domains.

## Conclusion

Coppice is appropriately shaped for its current dogfooding target: a solo
operator or a few isolated homes, local Tier-1 tools, low call concurrency, and
"revert and shrug" recovery. The reference implementation should not presently
be described as highly available, multi-user, or high-throughput.

The underlying ASF model is more scalable than the current implementation. Its
immutable objects, per-span chains, content addressing, manifest-bound authority,
and domain-scoped trust all give useful partition keys. The decisive architectural
advantage is that independent state-coherence groups can scale horizontally.

The decisive constraint is the inverse: everything inside one coherence group
converges on a single attested root tuple. Full-state capture, global mutable
session pointers, history-wide scans, whole-image SQLite handling, and home-wide
gate serialization will become limiting before the schema does. Future scaling
work should preserve the coherent-tuple invariant and make the fabric home an
explicit placement and fencing unit rather than trying to distribute individual
roots invisibly.
