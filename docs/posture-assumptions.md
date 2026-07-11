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
- **P22** In-flight allowed-but-unrecorded calls held in memory; remote
  effect can land before it is recorded. → **Durable external-effect
  protocol** (roadmap parked); RF-3 (accepted residual).
- **P11** `external_reach` has no mock router (external ⇒ needs `live`). →
  same egress gate.
- Companion: SI-23's contextual-strength and the taint wall (ADR 0005) also
  first bite here if egress and actuation land together.

### G-ACTUATION — before the first computer-use / shell / UI-control tool  *(relaxes NOACT)*
The approval mechanism itself is the vulnerable surface; the conservative
default cannot save this class retroactively.
- **P12** `auth_rank` is a flat total order (`local_session == passkey == 2`)
  "already believed wrong under live actuation." → **SI-23** (open, gated).
- **P5** The approval socket's "out-of-band" property is a 0600 Unix socket
  owned by the operator — reachable by granted hands. → SI-23 gap 1.
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

### G-ADVERSARIAL — before trusting a non-cooperative agent (or dropping the deny-rule convention)  *(relaxes COOP)*
- **P7** Agent and operator are the same uid, no sandbox; session branches
  are plain files; containment is client deny-rules, not topology. → W-4
  (roadmap); audit "trust boundary is the Unix account."
- **P3 / P4 / P5** Self-approvable escalations (above) — an agent with a
  shell/HID connects to the same socket and approves its own request.
- **P8** Placeholder identity: principal pubkeys `"01"`/`"02"`, one reused
  static intent, behavior hash literal `"sha256:asfd-stage2"`, no M4
  attestation. → audit High (brokered-recorded-as-observed); W-4(a).

### G-2HUMAN — before a second human in one coherence group  *(relaxes 1HUMAN)*
- **P18** Out-of-span drift is attributed to the one human by default
  (`human_local`); `tool_known` / `unattributed` deferred. → multi-actor
  roots/visibility (roadmap parked); SI-20/A20 (timing resolved);
  `dogfooding.md` "invite a second person" gate.

### G-CONCURRENT — before a second concurrent session in one home  *(relaxes 1SESS)*
- **P20** `current_manifest` / `current_span` are mutable **home-global**
  pointers; a second session clobbers the first's M2 binding; the broker is
  behind one process mutex. → SessionContext refactor (roadmap parked).

### G-MULTITENANT — before a multi-tenant / multi-user host  *(relaxes 1TEN / SU)*
- **P1 / P2** *(NEW tracking)* Keys and credentials in cleartext at rest —
  `secrets.json` bodies and the owner KEK beside the ciphertext it protects.
  → **RF-14** (new).
- **P17** *(NEW tracking)* Fabric-home state (CAS, branches, `fabric.db`) in
  cleartext; confidentiality is filesystem perms only. → **RF-15** (new).
- **P14** Plaintext content-hash addressing is a cross-tenant existence
  oracle (global dedup discloses whether a known blob exists elsewhere). →
  RF-7 (accepted; the multi-tenant angle is un-gated — flagged here).
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
  survives (and RF-14's KEK survival defeats it regardless). → forensic
  crypto-shredding (roadmap parked; audit; `AGENTS.md` boundary).
- **P19** AEAD does not bind hash/size/media-type/DEK-id as associated data
  → swappable row linkage. → audit High (future format-version decision).
- **P1 / P2 / P17** at-rest confidentiality (above) — production is also a
  disk-theft / backup surface.
- **P22** durable call identity / idempotency for external effects (also
  G-EGRESS).

### G-PUBLISH — before publishing the spec / any cross-implementation artifact  *(relaxes single-implementation)*
- **P24** JCS numeric constraint (`|n| < 2^53`) not enforced at seal/verify.
  → audit Medium; W-6 conformance (G2 differential vectors).
- **P23** Object `id` type-prefix not covered by the signature (defanged
  locally by full-id lookup + fail-closed field reads). → RF-6 (open).
- SI-23 constraints and the brief §8 landscape claim should also be settled
  before publication (both already tracked).

---

## The shortcut ledger (backing index)

All 24, grouped by tracking status at time of sweep. `SU/COOP/LOCAL/NOACT/
1SESS/1HUMAN/1TEN/DEBUG` = the invariant(s) that make each safe now.

### Tier 1 — items this sweep filed or newly gated

| P | Shortcut | Where | Safe-because | Now tracked as |
|---|----------|-------|--------------|----------------|
| P1 | Live credentials (`secrets.json`) plaintext at rest, never KEK-wrapped | `keys.rs:107-119` | SU 1TEN DEBUG | **RF-14 (new)**; G-MULTITENANT/G-PRODUCTION |
| P2 | Owner KEK cleartext beside the ciphertext it protects → nullifies payload encryption + shred | `keys.rs:133-141`, `payload.rs:148` | SU 1TEN DEBUG | **RF-14 (new)** |
| P17 | Fabric-home state (CAS/branches/`fabric.db`) plaintext; perms-only confidentiality | `snapshot.rs:85,223`, `kernel.rs:83` | SU 1TEN DEBUG | **RF-15 (new)** — the audit's unfiled RF-5 expansion |
| P3 | Approval `auth_strength` hardcoded `local_session`, asserted-not-authenticated (socket) | `proxy.rs:334,339,347,351` | SU COOP | audit prose + **new G-ACTUATION/G-ADVERSARIAL gate** |
| P4 | Same, offline `asf approve` path (`chan:local`) | `proxy.rs:638-666` | SU COOP | same |
| P6 | Self-declared tool metadata unverified; no gate on third-party registration | `tools.rs:84-98`, `proxy.rs:48-69` | COOP (first-party only) | **new G-3P-TOOL gate**; W-4 + domain-taxonomy |

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
| P13 | Logical-only crypto-shred; residue survives | `payload.rs:4-5`, `keys.rs:114` | DEBUG 1TEN | roadmap parked; audit; `AGENTS.md` |
| P14 | Plaintext-hash confirmation oracle; cross-tenant existence disclosure | `payload.rs:74` | 1TEN | RF-7 (accepted; multi-tenant angle flagged here) |
| P15 | No durable trace head; unsigned `offset` backs ordering claims | `trace.rs:236,266`; `broker.rs` | DEBUG 1TEN | RF-13 (open); audit High |
| P16 | Promotion/revert not crash-atomic across roots | `snapshot.rs`, `kernel.rs`, `broker.rs` | DEBUG | audit High; roadmap parked; scalability |
| P18 | Drift attributed to the one human by default | `kernel.rs:448-453` | 1HUMAN | multi-actor (parked); SI-20; `dogfooding.md` |
| P19 | AEAD binds no associated data → swappable row linkage | `keys.rs:252` | DEBUG 1TEN | audit High (format-version decision) |
| P20 | Home-global `current_manifest`/`current_span`; one broker mutex | `kernel.rs`; scalability | 1SESS | SessionContext (parked) |
| P21 | Host-local `flock` gate; no cross-host fencing | `kernel.rs:383-408` | 1TEN | HA topology (parked; scalability) |
| P22 | In-flight calls in memory; decision-time budget consumption | `broker.rs:355-364` | LOCAL | RF-3 (accepted); durable-effect protocol (parked) |
| P23 | `id` type-prefix not signature-covered | `canon.rs:103` | (single impl) | RF-6 (open) |
| P24 | JCS numeric constraint unenforced | `canon.rs:33,70` | (single impl) | audit Medium; W-6 |

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
