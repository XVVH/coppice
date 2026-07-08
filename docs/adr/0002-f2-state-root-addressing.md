# ADR 0002 — F2: state-root addressing (JCS vs IPLD/CIDs)

**Status: APPROVED IN PRINCIPLE (author, 2026-07-08) — direction locked;
ratification and migration gated on the Stage 2 post-dogfooding checkpoint.**

F2's only hard deadline is spec publication, which is not imminent; the
internal forcing function (CAS growth) is empirical. So: nothing built in
milestone 2 may *conflict* with the CID/DAG-CBOR data plane (roots stay
opaque strings everywhere; no code may parse `sha256:` roots outside
`snapshot.rs`), but the cutover itself waits for dogfooding data.

**Checkpoint tripwires — measure during Stage 2 dogfooding:**
1. CAS bytes-on-disk growth per week of normal vault-maintenance use, and
   the share attributable to sqlite byte-images vs fs blobs.
2. Live size of the Hermes memory DB (whole-image cost scales with it —
   under ~tens of MB, images may stay cheap enough to defer chunking).
3. Snapshot cadence actually observed (M6 says re-manifest often; cost =
   size × cadence).
4. SI-6 noise: how often logically-idle sqlite touches read as root changes.

Any of: CAS growth > O(GB/month), memory DB > ~100MB, or SI-6 noise
polluting the drift ledger → execute the migration then. None tripped by
spec-publication time → migrate anyway (the deadline in §9 stands).

## Decision (proposed)

Split by plane:

- **Control plane — JCS stays.** All signed fabric objects (manifests,
  events, intents, capabilities, rules, trust records) remain canonical JSON
  per RFC 8785; ids remain `<prefix>:<hex sha256(JCS(body ∖ {id, sig}))>`.
  **Companion spec rule (required):** numbers in fabric objects MUST be
  integers with |n| < 2^53; floats are forbidden. (RFC 8785 uses ECMAScript
  number serialization; without this rule, independent implementations can
  hash large integers differently.)
- **Data plane — CIDs + DAG-CBOR.** `state.roots[].root` values become
  CIDv1 strings. Interior nodes (fs trees, chunk trees, mirror trees) are
  DAG-CBOR (floats forbidden); leaf blobs are `raw` codec. Large objects are
  chunked Merkle DAGs — sqlite images chunked on page boundaries (sqlite
  mutates in page units, so deltas re-store only touched pages + spine).
- **Bridge rule.** Fabric objects treat CIDs as opaque strings. No CBOR in
  the signing path; no JCS in the data plane. Object ids and state roots are
  deliberately asymmetric: ids are identities within the fabric's trust
  domain, roots are references into a bulk data plane verifiable by third
  parties with standard multiformats tooling.
- **Scope fence.** CIDs ≠ IPFS. This adopts an addressing/serialization
  format only — no DHT, no gateways, no sync protocol, no unixfs. Rust
  surface: `cid`, `multihash`, a DAG-CBOR encoder; chunking is ours.

## Why

1. **Chunking is forced regardless.** Whole-file sqlite images per step
   boundary are the kernel's real 2-year scaling debt (see SI-6 and the
   CAS-growth analysis). Once roots point at chunked DAGs, the choice is
   "the existing standard for content-addressed DAGs" vs "a proprietary
   equivalent"; the second is IPLD-lite without the ecosystem.
2. **Neutrality is checkable in formats** (brief §9). Snapshot verification
   via off-the-shelf multiformats libraries and CAR-file ledger export make
   the open-formats claim concrete for third parties.
3. **Hash agility.** CIDs carry the hash algorithm; migration off sha256 is
   additive, not schema-breaking.
4. **Determinism.** DAG-CBOR canonical form (bytewise-sorted keys, definite
   lengths, no floats by our rule) is tighter than JCS for binary-heavy
   structures and avoids hex-string inflation of hashes in tree nodes.

## Considered and rejected

- **JCS-everywhere** (current kernel: JSON tree objects + `sha256:` roots):
  simplest, fully greppable, already running — but proprietary once chunking
  lands, no hash agility, ~2× inflation on hash-dense nodes, and every third
  party must implement our tree encoding to verify a snapshot.
- **Git object model:** proven CoW trees and a huge ecosystem, but sha1
  legacy (sha256 mode thinly adopted), no native large-file chunking (LFS
  exists because of this), no codec concept.

## Sequencing

Decision now; migration lands with the chunking + CAS-GC work (required
before long-horizon dogfooding), and before spec publication per F2's own
deadline. Interim `sha256:<hex>` roots are cleanly parseable; cutover is
mechanical. Spec changes on ratification: §0 number rule, §3 root format,
§9 F2 → resolved (A15/A16 candidates alongside SI-11's event-kind
amendment).
