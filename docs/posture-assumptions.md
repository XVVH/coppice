# Posture assumptions — the shortcuts our current state relies on

> **What this file is.** An index of every *posture-dependent shortcut* in
> Coppice: a simplification, deferral, or trust assumption that is safe
> **only because** the current deployment posture holds, and becomes unsafe
> (or merely incomplete) when the project graduates out of its debug /
> dogfooding state. It exists because the individual items were mostly
> tracked, but the *set* had no home — so you could neither audit
> completeness nor gate a posture transition against a checklist.
>
> **What this file is NOT.** Not a findings tracker. Each row links to where
> its real fix lives: `review-findings.md` (RF-n), `spec-issues.md` (SI-n),
> `roadmap.md` (W-n / parked), `dogfooding.md` graduation criteria, the
> ADRs, or the security audit. The detail lives there; this is the map.
>
> **How to use it.** Before flipping any posture invariant — enabling
> egress, registering an actuation or third-party tool, adding a second
> human, going multi-session / multi-tenant / production, or publishing the
> spec — read that transition's **graduation gate** below. It lists exactly
> the shortcuts that event un-safes and what must be hardened first. The
> gates are the order-of-operations; the ledger is the backing index.
>
> **Discipline.** A new shortcut taken in code or a new deferral agreed in
> conversation joins this ledger in the same change — the roadmap's
> file-first rule, applied to safety debt. A shortcut you can't place under
> a graduation gate is a shortcut whose blast radius you haven't reasoned
> about yet.

Compiled 2026-07-11 from an exhaustive code + docs sweep. Source rows are
numbered `P-n` and cited by `file:line`.

---

## The current posture (the invariants that make the shortcuts safe)

These hold in the dogfooding deployment. Each is load-bearing: relaxing one
un-safes a specific set of shortcuts.

- **SU** — agent and operator are the **same Unix user**; no sandbox.
- **COOP** — the agent is **cooperative** (not adversarial or compromised);
  containment is client-side deny-rules, a convention, not topology.
- **LOCAL** — **local-only, no external egress**; every tool action is
  `side_effect: local`.
- **NOACT** — **no actuation**: no computer-use / shell / UI-control tool is
  registered.
- **1SESS** — **one active session** per fabric home at a time.
- **1HUMAN** — **one human** in a coherence group.
- **1TEN** — **single-tenant**, single-machine.
- **DEBUG** — **non-production**: "revert and shrug" is the accepted failure
  mode; no durability, HA, or forensic guarantee is claimed.

Two doctrines already encode this honestly and this file does not restate
them: `AGENTS.md`'s implementation-status boundary (fail-closed-not-fail-open,
logical-not-forensic shred) and `dogfooding.md`'s two-surface / scope-honesty
sections (cooperative-agent measurement). This ledger is their union plus the
items that had fallen outside both.

---

## Graduation gates — read the column before you flip the invariant

Each gate lists the shortcuts that transition un-safes, most-load-bearing
first. **NEW** marks a gate or item this sweep added because it was untracked
or noted-but-never-gated. `dogfooding.md` graduation criteria cross-reference
these.

### G-EGRESS — before the first `external_reach: live` tool  *(relaxes LOCAL)*
The single highest-consequence transition: "reads are irreversible" turns the
read surface into the exfiltration perimeter.
- **P9** Default grant leaves reads unscoped (`paths.write:["**"]`, no
  read-scope / read-volume / egress dim). → R2 read-authority family
  (roadmap parked; ADR 0005; `dogfooding.md` criterion).
- **P10** Broker/proxy **fails closed** on outage, contradicting the
  non-negotiable loud fail-open posture. → Broker-outage `on_broker_outage`
  (roadmap parked; audit; `AGENTS.md` boundary).
- **P25 — CLOSED by W-8 (PR #33).** Capabilities had mandatory expiry but
  no early revocation despite F1 calling them revocable. Now: signed
  `revoke` events, event-derived liveness at decision time and gate
  replay, operator `asf revoke`. Residuals live in their own rows: P22
  (durable dispatch — revocation cannot recall an in-flight remote
  effect), P15/RF-13 (rollback can erase the latest revoke), P21
  (stale partitions).
- **P22** In-flight allowed-but-unrecorded calls held in memory; remote
  effect can land before it is recorded. → **Durable external-effect
  protocol** (roadmap parked); RF-3 (accepted residual).
- **P11** `external_reach` has no mock router (external ⇒ needs `live`). →
  same egress gate.
- **P27** Broker-injected credentials cross into the downstream tool and an
  arbitrary downstream result crosses back to the agent unchanged. A buggy or
  adversarial adapter can echo/encode the secret. → RF-23; W-18. No real
  credential crosses a third-party or live-egress adapter before containment.
- Companion: SI-23's contextual-strength and the taint wall (ADR 0005) also
  first bite here if egress and actuation land together.

### G-ACTUATION — before the first computer-use / shell / UI-control tool  *(relaxes NOACT)*
The approval mechanism itself is the vulnerable surface; the conservative
default cannot save this class retroactively.
- **P12** `auth_rank` is a flat total order (`local_session == passkey == 2`)
  "already believed wrong under live actuation." → **SI-23** (open, gated).
- **P5** The approval socket's "out-of-band" property is a 0600 Unix socket
  owned by the operator — reachable by granted hands. → SI-23 gap 1.
- **P25 — CLOSED by W-8 (PR #33):** the kill-switch *operation* exists
  (`asf revoke` closes subtrees definitionally). SI-23 still owns the
  *surface* — under actuation the door it rides on (P5's socket) must be
  unreachable by granted hands before this gate opens.
- **P3 / P4** *(NEW gate wiring)* Approval `auth_strength` is hardcoded
  `"local_session"`, **asserted, never authenticated** — no peer-cred check,
  no user-presence proof — on both the daemon socket and the offline
  `asf approve` path. Under actuation this is the self-approval hole. →
  audit accepted-limitation prose **+ new `dogfooding.md` gate** (was
  noted-not-gated).

### G-3P-TOOL — before any third-party (non-first-party) tool registers  *(relaxes COOP on tool metadata)*  **NEW gate**
- **P6** Tool action metadata (`reversibility`, `store`, `path_args`, class)
  is self-declared "trusted-mechanical" and never verified against tool
  behavior — a lying or buggy tool defeats every caveat keyed on it. Today
  only first-party compile-time profiles exist, so there is no gate. →
  W-4 containment is the eventual answer (a sandbox bounds a lying tool);
  domain-taxonomy governance (AGENTS.md open problem) bounds trust-farming.
  **No generic registration path may ship before both.**
- **P27** Credential injection trusts the downstream adapter not to reflect or
  transform a secret into its response. Current first-party vault tools carry
  no live credential. → RF-23; the same no-registration gate.

### G-ADVERSARIAL — before trusting a non-cooperative agent (or dropping the deny-rule convention)  *(relaxes COOP)*
- **P7** Agent and operator are the same uid, no sandbox; session branches
  are plain files; containment is client deny-rules, not topology. → W-4
  (roadmap); audit "trust boundary is the Unix account."
- **P3 / P4 / P5** Self-approvable escalations (above) — an agent with a
  shell/HID connects to the same socket and approves its own request.
- **P25 — CLOSED by W-8 (PR #33):** minted authority is now closable
  early; a compromised agent's capability subtree dies at the next
  decision or gate check past the revoke offset.
- **P8** Placeholder identity: principal pubkeys `"01"`/`"02"`, one reused
  static intent, behavior hash literal `"sha256:asfd-stage2"`, no M4
  attestation. → audit High (brokered-recorded-as-observed); W-4(a).

### G-RATCHET — before ledger events compile into standing authority (W-3)  *(no invariant relaxed — the substrate becomes policy-bearing)*
- **P15 / RF-16** Standing authority may not derive from a substrate whose
  authority view can be changed through unsigned index columns or whose head
  can be rolled back. W-11 closes verified-row use; SI-25/W-15 closes global
  completeness/freshness before W-3 proceeds beyond disposable experiments.
- **P26** Corpus-ingest homes (`asf corpus ingest`) sign placeholder
  principals and a placeholder behavior bundle into a REAL substrate —
  P8's class, second site. Nothing mechanical distinguishes a corpus
  home from a policy-bearing one; evidence quarantine is convention
  only (R1/R6 boundaries in `agent-trace-corpora-2026-07-11.md`,
  operator-side dirs, fresh-home guard). Safe while no clerk exists;
  the moment W-3 clusters approval events into rule candidates, a
  mechanical exclusion is required (corpus-home marker, or the clerk
  refuses substrates whose principals carry placeholder keys). →
  W-3; CORPUS-FAIL-CLOSED covers conversion, not home identity.

### G-2HUMAN — before a second human in one coherence group  *(relaxes 1HUMAN)*
- **P18** Out-of-span drift is attributed to the one human by default
  (`human_local`); `tool_known` / `unattributed` deferred. → multi-actor
  roots/visibility (roadmap parked); SI-20/A20 (timing resolved);
  `dogfooding.md` "invite a second person" gate.

### G-CONCURRENT — before a second concurrent session in one home  *(relaxes 1SESS)*
- **P20** `current_manifest` / `current_span` are mutable **home-global**
  pointers; a second session clobbers the first's M2 binding; the broker is
  behind one process mutex. → SessionContext refactor (roadmap parked).

### G-MULTITENANT — before tenants share one uid, home, or storage namespace  *(relaxes 1TEN / SU)*
Separate fabric homes under separate Unix identities retain today's enforced
0700/0600 confidentiality boundary; they require fleet machinery, but not a
new at-rest cryptographic boundary merely because another tenant exists. This
gate applies when tenants share a uid/home, when privileged-host compromise is
in scope, or when storage leaves that filesystem boundary.
- **P1 / P2** *(NEW tracking)* Keys and credentials are cleartext inside that
  boundary — `secrets.json` bodies and the owner KEK beside the ciphertext it
  protects. → **RF-14** (new).
- **P17** *(NEW tracking)* Fabric-home state (CAS, branches, `fabric.db`) is
  cleartext inside that boundary. → **RF-15** (new).
- **P14** Plaintext content hashes are a same-home confirmation oracle. They
  become a cross-tenant oracle only if a future deployment introduces global
  deduplication; keep addressing tenant-scoped or keyed in that topology. →
  RF-7 (accepted); scalability analysis.
- **P21** Gate serialization is host-local `flock` — no cross-host fencing.
  → HA topology (roadmap parked; scalability).

### G-PRODUCTION — before any durability / HA / forensic commitment  *(relaxes DEBUG)*
- **P16** Promotion/revert not crash-atomic across roots (sequential commit;
  in-place per-file apply; state mutated before ledger attestation). → audit
  High; crash-atomic multi-root commit (roadmap parked); scalability.
- **P15** Trace tamper-evidence incomplete: no durable/external signed head
  (tail-truncation and whole-span deletion undetectable); substrate `offset`
  is an unsigned rowid backing all cross-span ordering claims. → RF-13
  (open); audit High; trace-head anchoring (roadmap parked).
- **P13** Crypto-shred is logical only; WAL/freelist/snapshot/backup residue
  may retain old ciphertext and wrapped-DEK pairs, which RF-14's persistent
  KEK can decrypt. KEK survival alone cannot recreate a deleted random DEK.
  → forensic crypto-shredding (roadmap parked; audit; `AGENTS.md` boundary).
- **P19** AEAD does not bind hash/size/media-type/DEK-id as associated data
  → swappable row linkage. → audit High (future format-version decision).
- **P1 / P2 / P17** at-rest confidentiality (above) — production is also a
  disk-theft / backup surface.
- **P22** durable call identity / idempotency for external effects (also
  G-EGRESS).
- **P25 — CLOSED by W-8 (PR #33):** early closure and the kill-switch
  operation exist. Production still needs P15/RF-13 (a restored
  `fabric.db` tail must not erase the latest revoke) before revocation
  claims survive rollback.

### G-PUBLISH — before publishing the spec / any cross-implementation artifact  *(relaxes single-implementation)*
- **P24 — CLOSED by W-12 (PR #39):** JCS numeric constraint
  (`|n| < 2^53`, integer-only) enforced at seal/verify, with strict raw
  duplicate-name rejection and language-neutral input-domain vectors. W-6
  still owns the independent-implementation differential and RF-6/SI-26.
- **P23** Object `id` type-prefix not covered by the signature (defanged
  locally by full-id lookup + fail-closed field reads). → RF-6 (open).
- **P25 — CLOSED by W-8 (PR #33):** the published lifecycle has
  early-closure semantics AND a reference implementation; W-6 can now
  extract §5.4 conformance vectors from the A22 contract tests.
- SI-23 constraints and the brief §8 landscape claim should also be settled
  before publication (both already tracked).

---

## The shortcut ledger (backing index)

All 27 currently tracked, grouped by filing status. `SU/COOP/LOCAL/NOACT/
1SESS/1HUMAN/1TEN/DEBUG` = the invariant(s) that make each safe now.

### Tier 1 — items this sweep filed or newly gated

| P | Shortcut | Where | Safe-because | Now tracked as |
|---|----------|-------|--------------|----------------|
| P1 | Live credentials (`secrets.json`) plaintext at rest, never KEK-wrapped | `keys.rs:107-128` | SU 1TEN DEBUG | **RF-14 (new)**; G-MULTITENANT/G-PRODUCTION |
| P2 | Owner KEK cleartext beside live ciphertext → nullifies live-payload encryption against a copied home; enables residual-pair recovery after shred | `keys.rs:131-141`, `payload.rs:130-197` | SU 1TEN DEBUG | **RF-14 (new)** |
| P17 | Fabric-home state (CAS/branches/`fabric.db`) plaintext; perms-only confidentiality | `snapshot.rs:91-100,176-247`, `kernel.rs:95-107` | SU 1TEN DEBUG | **RF-15 (new)** — the audit's unfiled RF-5 expansion |
| P3 | Approval `auth_strength` hardcoded `local_session`, asserted-not-authenticated (socket) | `proxy.rs:334,339,347,351` | SU COOP | audit prose + **new G-ACTUATION/G-ADVERSARIAL gate** |
| P4 | Same, offline `asf approve` path (`chan:local`) | `proxy.rs:638-666` | SU COOP | same |
| P6 | Self-declared tool metadata unverified; no gate on third-party registration | `tools.rs:84-98`, `proxy.rs:48-69` | COOP (first-party only) | **new G-3P-TOOL gate**; W-4 + domain-taxonomy |
| P25 | ~~F1 calls capabilities revocable with expiry-only enforcement~~ **CLOSED by W-8 (PR #33)**: signed `revoke` (§5.4), `a22_*` event-derived liveness at decision + gate, `asf revoke` kill switch | `broker.rs` (`a22_*`, `revoke_capability`), `proxy.rs` revoke surface | — (implemented) | SI-24 → A22 (v0.7) → W-8; residuals: P22 (in-flight dispatch), P15/RF-13 (rollback erasure), P21 (partitions) |
| P26 | Corpus-ingest homes: placeholder identities/behavior signed into a real substrate; evidence-quarantined by convention only (second P8 site) | `asf-cli corpus/ingest.rs` (`placeholder_key`, behavior literal) | COOP SU | W-3 mechanical exclusion (G-RATCHET); `agent-trace-corpora-2026-07-11.md` boundaries |
| P27 | Injected credentials enter a downstream adapter whose response reaches the agent unchanged | `broker.rs:560-576`, `proxy.rs:501-571` | COOP LOCAL, first-party/no credential | RF-23; G-EGRESS/G-3P-TOOL; W-18 |

### Tier 2 — items already tracked (this ledger just indexes and gates them)

| P | Shortcut | Where | Safe-because | Tracked as |
|---|----------|-------|--------------|------------|
| P5 | Approval socket "out-of-band" = 0600 Unix socket, reachable by granted hands | `proxy.rs:280-297` | SU NOACT | SI-23 gap 1; `dogfooding.md` |
| P7 | Same-uid, no sandbox; branches are plain files; deny-rules ≠ topology | `dogfooding.md:66-107` | COOP | W-4; audit |
| P8 | Placeholder identity / no M4 attestation (`"01"`,`"02"`, reused intent, literal bundle) | `proxy.rs:211-226` | COOP | audit High; W-4(a) |
| P9 | Reads unscoped within store (no R2 dims) | `proxy.rs:75-87` | LOCAL | R2 (parked); ADR 0005; `dogfooding.md` |
| P10 | Fails closed on broker outage (spec wants fail-open-loud) | `proxy.rs:11-15,556-571` | LOCAL | roadmap parked; audit; `AGENTS.md` |
| P11 | No mock router; external ⇒ needs `live` | `evaluate.rs:150-151` | LOCAL | egress gate (`dogfooding.md`) |
| P12 | Flat `auth_rank` total order (`local_session==passkey`) | `capability.rs:47-58` | NOACT | SI-23 (open) |
| P13 | Logical-only crypto-shred; old ciphertext + wrapped-DEK pairs may survive | `payload.rs:4-5,172-197`, `keys.rs:131-141` | DEBUG 1TEN | roadmap parked; audit; `AGENTS.md` |
| P14 | Plaintext-hash confirmation oracle; cross-tenant only if future storage deduplicates globally | `payload.rs:71-100` | 1TEN | RF-7 (accepted; global-dedup topology flagged here) |
| P15 | No durable trace head; unsigned `offset` still backs cross-span ordering while W-11 closes denormalized-selector authority use | `trace.rs`; `broker.rs` | DEBUG 1TEN | RF-13/SI-25/W-15; RF-16 closed by W-11 |
| P16 | Promotion/revert not crash-atomic across roots | `snapshot.rs`, `kernel.rs`, `broker.rs` | DEBUG | audit High; roadmap parked; scalability |
| P18 | Drift attributed to the one human by default | `kernel.rs:448-453` | 1HUMAN | multi-actor (parked); SI-20; `dogfooding.md` |
| P19 | AEAD binds no associated data and stored algorithm/key-link metadata is not enforced | `keys.rs:210-269`, `payload.rs:122-156` | DEBUG 1TEN | RF-22; SI-28; W-16 |
| P20 | Home-global `current_manifest`/`current_span`; one broker mutex | `kernel.rs`; scalability | 1SESS | SessionContext (parked) |
| P21 | Host-local `flock` gate; no cross-host fencing | `kernel.rs:383-408` | 1TEN | HA topology (parked; scalability) |
| P22 | In-flight calls in memory; decision-time budget consumption | `broker.rs:355-364` | LOCAL | RF-3 (accepted); durable-effect protocol (parked) |
| P23 | `id` type-prefix not signature-covered | `canon.rs:103` | (single impl) | RF-6 (open) |
| P24 | ~~JCS numeric constraint unenforced; distinct exact integers can share signed bytes~~ **CLOSED by W-12 (PR #39):** strict recursive seal/verify domain plus duplicate-safe raw fabric parsing | `canon.rs` (`w12_*`, `parse_fabric_json`, `seal`, `verify`) | — (single-implementation differential remains) | RF-17; W-12; G2; residual RF-6/SI-26 |

---

## What this sweep changed

- **Filed** RF-14 (keys/secrets cleartext at rest) and RF-15 (state cleartext
  at rest — the audit's long-unfiled RF-5 expansion). Both were genuinely
  untracked as confidentiality findings; RF-14's owner-KEK item is the sharp
  one (it nullifies the payload encryption the project *did* build).
- **Added three graduation gates** that were noted-but-never-gated: approval
  authentication (G-ACTUATION / G-ADVERSARIAL), third-party tool registration
  (G-3P-TOOL), and at-rest confidentiality (G-MULTITENANT / G-PRODUCTION).
  `dogfooding.md` graduation criteria now reference them.
- **Indexed** the other 18 shortcuts against their existing trackers, so the
  set is auditable and each transition has a checklist. Nothing in Tier 2 was
  under-tracked; the value there is the order-of-operations, not new findings.
- **Follow-up (SI-24, 2026-07-11):** added P25 when the revocation design
  review found that F1's "revocable" claim had no early-closure mechanism.
- **Follow-up (W-9 review, 2026-07-12):** added P26 + G-RATCHET when the
  fresh-eyes review found the corpus harness had taken a P8-class shortcut
  (placeholder identities into a real substrate) without a ledger row —
  the ledger's own same-change rule, applied late.
- **Follow-up (A22 ratification, 2026-07-12):** SI-24 resolved — spec §5.4
  now specifies the closure lifecycle P25 flagged as missing.
- **Follow-up (W-8, 2026-07-12, PR #33):** P25 closed — §5.4 implemented
  (event-derived liveness shared by decision and gate, operator kill
  switch, closed-parent/closed-id refusals). First closed row in this
  ledger; the row is kept struck-through as the record. Residual exposure
  moved to where it already lived: P22, P15/RF-13, P21.
