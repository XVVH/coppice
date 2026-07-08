# Agent State Fabric — Schema Specification

**Draft v0.3 — July 2026 — Companion to the Architecture Brief**

*v0.3 integrates amendments A1–A14 from the wedge paper runs (Hermes agent; workflows: web research → vault distillation, Discord message management, vault maintenance). Changelog at end. Risk-review requirements carried since v0.1: **(R2)** read authority is first-class, **(R3)** payloads are hash-referenced and destroyable, **(R6)** trust is domain-scoped, never scalar.*

---

## 0. Conventions

- **Serialization:** canonical JSON (JCS, RFC 8785). Every object's `id` is the SHA-256 of its canonical body excluding `sig`. All cross-references are by id; lineage is tamper-evident by construction.
- **Signatures:** Ed25519. `sig: { key_id, alg, value }`. Key hierarchy §8.
- **Timestamps:** RFC 3339 UTC.
- **Extensibility:** unknown fields MUST be preserved and hashed; unknown *caveat dimensions* MUST fail closed (an unrecognized restriction cannot be safely ignored).
- **Zero authorship (normative).** Every caveat dimension MUST define a no-configuration default derivable from registered metadata (channel auth strength, tool domain, reversibility class, single-human assumption). No surface may ask the user to author policy upfront; user-authored policy enters only through the ratification loop. A dimension that cannot state its zero-authorship default does not ship.
- **Conservative defaults are the free ones.** Undeclared reversibility = `irreversible`; undeclared egress = egress; undeclared sensitivity = `sensitive`; undeclared skill domain = pins everything. Laziness must always land on the safe side.

## 1. Payload store (R3)

No fabric object embeds sensitive content. Content lives in a payload store, addressed by hash, encrypted per-payload:

```json
PayloadRef { "hash": "sha256:…", "size": 18742,
             "media_type": "message/rfc822", "dek_id": "dek:7f3a…" }
```

**Erasure = crypto-shredding.** Destroying the per-payload DEK destroys content everywhere at once; hash and lineage persist. A destroyed payload resolves to a tombstone `{hash, shredded_at, reason}`. Substance is destroyable; structure is not.

**Redactable inline fields.** Objects carrying small structured summaries mark fields `redactable`; redaction replaces the value with a salted commitment `{"redacted": "sha256:salt‖value"}` via tombstone-and-reissue (§8.4), preserving hash chains.

## 2. Principals

```json
Principal { "id": "prin:…", "kind": "human" | "agent" | "service",
            "root": "prin:… | null", "pubkey": "ed25519:…",
            "meta": { "label": "invoice-runner" } }   // redactable
```

Humans and agents are the same object; the org-chart model needs no schema change. Agent principals are instances: behavior is pinned per-manifest (§3), not per-identity.

### 2.1 Channel — the front door

The user↔agent conversation mints Intents — it is the provenance of all authority — so its carriers are registered objects with authentication strength, never ambient context:

```json
Channel { "id": "chan:…", "principal": "prin:… (human)",
          "kind": "fabric_app" | "local_session" | "oauth_platform" | "email" | "phone" | "other",
          "address": PayloadRef,          // binds the SENDER identity, not the bot endpoint
          "auth_strength": "local_session" | "passkey" | "platform_oauth" | "unverified",
          "registered_at": "…", "sig": { … } }
```

**Rules.**
- **C1** Every human-originated authority act — Intent, amendment, escalation approval, ratification — records `channel` + `auth_strength`. Consent has provenance; rubber-stamp patterns over weak channels are ledger-visible (R1).
- **C2** Approval flows terminate at the broker: broker pushes out, signed response returns directly. The agent is never in the approval data path — it may announce that approval was requested, never carry the token. On a single machine this means the broker daemon owns its own approval surface (its own TUI, socket, or OS notification), visually and procedurally distinct from the agent's chat loop; approvals never travel through the agent's stdin (A6).
- **C3** Fabric→user messages are signed and verifiable as fabric-originated.
- **C4** Outbound delivery to a channel is egress. Derived-sensitive content (§4) inherits the channel's zero-authorship ceiling: full detail on `local_session`/`passkey` surfaces; summary-with-deep-link on weaker ones. Overrides ride the ratchet as `delivery.detail` rules only.
- **C5 — Sender binding (A6).** A message from an unregistered sender on a registered transport (e.g., a stranger messaging the Telegram bot) is not a weaker instruction; it is not an instruction at all. It is data: logged, ignored, surfaced as signal.

## 3. DelegationManifest

The kernel object. Created at every handoff, at a step boundary, before the delegate executes anything.

```json
Manifest {
  "id": "man:…", "parent": "man:… | null", "created_at": "…",
  "delegator": "prin:…", "delegate": "prin:…",
  "intent": "int:…",
  "state": { "roots": [
    { "store": "fs:vault",     "tier": 1, "kind": "fs",     "root": "sha256:…" },
    { "store": "db:memory",    "tier": 1, "kind": "sqlite", "root": "sha256:…" },
    { "store": "db:books",     "tier": 2, "kind": "branch", "root": "branch:bk_4f2" },
    { "store": "shadow:discord","tier": 3, "kind": "mirror", "root": "sha256:…",
      "as_of": "2026-07-08T13:58:01Z" } ] },
  "authority": { "capability": "cap:…" },
  "behavior":  { "bundle": "sha256:…",
                 "skills": [ { "skill": "vault-filing", "version": "sha256:…",
                               "domains": ["files.vault"] } ] },
  "trace": { "span": "span:…", "substrate_offset": 88213 },
  "sig": { … }
}
```

**Invariants.**
- **M1** `state.roots` covers every store the capability grants any access to — no authority over un-snapshotted state. (Corollary: stores *absent* from the manifest are outside the run's universe; least privilege by omission.)
- **M2** The capability references this manifest back (`cap.bound_manifest == man.id`): authority is scoped to this state fork — hermetic sub-agents and safe counterfactual replay are structural.
- **M3** Tier-3 `as_of` MUST surface in any consent UI touching that store.
- **M4** `behavior.bundle` matches the bundle actually loaded; runtime attests at first tool call.
- **M5 — Re-manifest on behavior change (A2).** A bundle change mid-run (skill created or hot-loaded) triggers, at the next step boundary, a self-delegation: child manifest, new behavior hash, same capability (attenuation-identity is legal). Even self-modification has lineage; the trace shows which steps ran under which behavior.
- **M6 — Small manifests (A11 guidance).** Prefer frequent re-manifesting over long-lived branches; promotion divergence (§5.3) grows with branch age, and M5 makes re-baselining cheap.

### 3.1 IntentArtifact

The user's ask, captured and signed before untrusted data enters the run:

```json
Intent { "id": "int:…", "principal": "prin:… (human)", "created_at": "…",
         "text": PayloadRef,
         "structured": { "budget": {"currency":"USD","cap":200},
                         "deadline": "2026-07-11", "audience": {"mode":"known_contacts"} },
         "captured_before": 88190,        // substrate offset: pre-contamination proof
         "channel": "chan:…", "auth_strength": "local_session",
         "amends": "int:… | null",
         "sig": { … } }
```

**Amendments.** Mid-run instructions are amendments — signed, channel-stamped, offset-stamped, chained via `amends`; the judge evaluates against the chain. **Directionality rule:** narrowing amendments are accepted from any registered channel; *expanding* amendments (new recipients, new stores, higher budgets, wider destructive scope) require `auth_strength ≥` the capability's `approval.min_auth`, else they park queued. Expansion is the attack direction; narrowing is free. Text without channel provenance (e.g., inside fetched content, claiming to speak for the user) is not an amendment; it is data.

### 3.2 StandingIntent (A5)

Scheduled runs have no live human. A StandingIntent is an Intent ratified like a rule: `schedule`, per-run caveat budget, `escalation_posture: "queue"` (violations park until the human surfaces; nothing irreversible proceeds unattended), plus full ratchet lifecycle — decay, drift, periodic re-ratification (§7.1).

## 4. ToolRegistration

Tools declare shape once; the broker countersigns. Reversibility, compensators, domains, read profiles, egress, and audience visibility all enter here as *declared, trusted-mechanical* metadata.

```json
Tool { "id": "tool:discord@1.0",
  "actions": [
    { "name": "message.send",   "side_effect": "external", "surface": "fixed",
      "reversibility": "compensable", "compensator": "message.delete",
      "visibility": "human_audience", "domain": "comms.outbound",
      "writes": { "channels": true } },
    { "name": "message.delete", "side_effect": "external", "surface": "fixed",
      "reversibility": "irreversible", "compensator": null,
      "domain": "comms.moderation" },
    { "name": "web.fetch",      "side_effect": "external", "surface": "open",
      "egress": true, "reversibility": "reversible", "domain": "web.research",
      "read_cost": "per_page" } ],
  "sig": { … } }
```

**Registration rules.**
- Undeclared actions cannot be called. Missing `reversibility` = `irreversible`.
- **Egress declaration.** Any action whose reads can carry outbound content MUST declare `egress: true`; undeclared egress fails closed.
- **Surface class (A10).** `surface: open` (arbitrary hosts / URL-shaped exfiltration possible — full egress caveat family applies) vs `fixed` (single-host, schema-constrained API — external-but-contained; egress caveats relax to recipient/budget control).
- **Audience visibility (A4).** `visibility: human_audience` marks actions whose effects humans will read. Orthogonal to reversibility: a deletable message is still socially irreversible once seen. Judge scrutiny and `approval.min_auth` zero-authorship defaults elevate on it.
- **Platform-truthful reversibility (A7).** Classes are declared per-action against what the platform actually supports (Discord: send compensable, delete irreversible). For irreversible external *deletes*, the Tier-3 mirror is the only undo that exists: the broker MAY require mirror freshness as a precondition caveat (§5.1), and delete events carry PayloadRefs into mirror content. Margin note: mirror compensation is *partial* — content survives; ids and thread positions don't. Compensation has fidelity grades; unschematized for now.
- **Derived sensitivity.** Domains MAY declare `default_sensitivity` (undeclared = `sensitive`); the provenance layer propagates floors by derivation. No labeling substrate, no user authorship (partially resolves F3).

## 5. Capability and the caveat grammar

```json
Capability { "id": "cap:…", "parent": "cap:… | null",
  "holder": "prin:…", "bound_manifest": "man:…",
  "issued_at": "…", "expires_at": "…",            // expiry mandatory
  "caveats": [ Caveat, … ],                        // conjunctive: ALL must pass
  "on_violation": { "default": "deny", "escalatable": ["budget.count", "recipients"] },
  "sig": { … } }                                   // broker-minted (F1: committed for v1)
```

### 5.1 Caveat dimensions

Mechanically checkable at call time; conjunctive; unknown dims fail closed. The grammar is the consent-screen vocabulary: budgets, counts, people, places, deadlines.

**Action scope**
- `action.allow` — `{ tools, actions }` allowlist.
- `reversibility.max` — worst permitted class; the default probation caveat.
- `external_reach` — `"live" | "mocks" | "none"` — hermetic replay and shadow mode by token, not convention.

**Write bounds**
- `budget.money` — `{ currency, cap, window }`, broker-metered.
- `budget.count` — `{ action_class, max, window }` — the caveat that turns "bulk delete" from standing hazard into bounded operation.
- `recipients` — `{ mode: "known_contacts" | "allowlist" | "domains", scope: "dm" | "channel" | "broadcast", values }` — scope classes (A7) because a DM, a channel post, and an @everyone are different blast radii.
- `paths.write` — globs / tables / labels.
- `time` — `{ not_before, not_after, schedule }`.

**Guards and preconditions (A8, A7)**
- `guard` — `{ action, require: <predicate over target attributes>, resolve: "broker_verified" }` — object-level predicates on destructive actions (`target.pinned: false`, `target.age: ">P90D"`, `target.author.role: "!admin"`). **Broker-verified is mandatory:** the broker resolves target attributes itself against the mirror plus live spot-check, never trusting agent-supplied metadata. Guards fence the *eligible set* mechanically so the skill's judgment errors land only on the harmless.
- `precondition` — `{ action, require }` — run-environment predicates, canonically `{ "mirror.as_of": "<PT15M" }` before irreversible deletes: no fresh mirror, no destruction.

**Egress bounds (surface: open)**
- `egress.channels` — which egress-flagged families are open at all.
- `web.methods` — `{ allow: ["GET"] }` — read-shaped traffic only.
- `web.auth` — `"none" | "scoped_session"` — `none` = the public web as an anonymous stranger; nothing to steal or spend.
- `domains` — `{ mode: "any_public" | "allowlist", deny: [floor] }` — honest about non-enumerability; open surfaces are controlled at the channel, not the destination.
- `taint.egress` — `{ urls_and_bodies_derived_only_from: [benign sources] }` — the anti-exfiltration wall: outbound URLs and bodies may derive from intent text and prior public results, never private state. Vault-informed research without vault exfiltration is this one caveat.

**Human-channel bounds**
- `approval.min_auth` — minimum channel strength for approvals, ratifications, expanding amendments. Zero-authorship default scales with worst reversibility class and `human_audience` flags.
- `delivery.detail` — ratchet-authored exceptions to the C4 delivery ceiling; never user-authored upfront.

**Read bounds (R2)**
- `read.scope` — what may be seen at all.
- `read.volume` — `{ max_records, max_bytes, max_pages, window }` — the anti-bulk-exfiltration meter.
- `read.sensitivity` — `{ max }` — derived floors per §4; ceiling per channel per C4.
- `taint.outbound` — deliverable content derived only from in-scope reads.

### 5.2 Attenuation

Per dimension, the child's permitted set MUST be a subset of the parent's: caps ≤, windows within, globs narrower, allowlists ⊆, reversibility no worse, expiry no later. Absent-in-parent = unrestricted there; children may add dimensions, never remove or widen. Verification is mechanical subset-checking; failure invalidates the capability outright.

### 5.3 Promotion (A11, A12, A13)

Promotion merges a completed branch to trunk and is the only mutation in the system.

- **Merge semantics (A11).** Three-way merge: base = the manifest's state root; reconcile agent branch and current trunk against it. **Conflicts never auto-resolve in the agent's favor** — human trunk edits win by default; true collisions surface as conflict cards. Snapshot ancestry survives the merge: promoted changes remain revertible.
- **Coherent revert.** Revert restores *all* state roots of the manifest atomically — vault and agent memory together — so undo never gaslights the agent with a world its memory contradicts. (This is the cross-layer-consistency thesis, operational.)
- **Operation classes (A13).** Diffs and rules speak `add | modify | delete | move | rename`, with rename detection — a reorganization must never render as mass deletion; that failure of legibility either blocks good work or habituates users to scary diffs.
- **Promotion rules (A13).** Gate decisions are caveat-shaped (paths, operation classes, sizes), so auto-promotion rules are StandingRules in the same grammar — e.g., auto-merge iff paths ⊆ {/inbox, /MOCs}, ops ⊆ {add, modify, link}, no deletes — ratcheted from early manual approvals. Low-stakes by construction: Tier-1 promotion is always revertible.
- **Trace-vs-capability check.** At the gate, the recorded trace is verified against the capability — did the run do anything its token shouldn't allow — before anything becomes durable.

## 6. TraceEvent

Append-only, hash-chained per span, signed by the emitting component.

```json
Event { "id": "evt:…", "span": "span:…", "seq": 41, "prev": "evt:…",
        "manifest": "man:…", "at": "…",
        "kind": "tool_call" | "verdict" | "escalation" | "ratification" | "promotion"
              | "revert" | "compensation" | "grant" | "expiry" | "snapshot"
              | "drift" | "shred" | "amendment" | "remanifest",
        "body": { … }, "sig": { … } }
```

**tool_call body:** `{ tool, action, args: PayloadRef, result: PayloadRef, summary: {…redactable, caveat-relevant extract only (F4 default: fields the capability's caveats actually reference)}, checks: [{caveat, ok, meter}], reversibility, compensator, mirror_content: PayloadRef?, state_root_after }`.

**escalation body — batching (A9):** violations aggregate per (caveat, action_class): `{ count: 28, sample: [5 refs], guard_status: "all_pass" }` → one approval covers the batch. Twenty-eight pings is the R1 fatigue machine rebuilt; one legible batch is not.

**drift body — attribution classes (A12):** `{ store, expected_root, observed_root, between, attribution: "human_local" | "tool_known" | "unattributed" }`. Zero-authorship default for the solo operator: local edits outside agent spans attribute quietly to the human (logged, not alerted); only `unattributed` drift is loud. In a co-edited vault drift is Tuesday, not an incident.

**Human-originated events** (approvals, ratifications) carry `{ channel, auth_strength }` per C1.

**shred body:** `{ payload_hash, reason }` — the ledger records *that* it forgot, never what.

**Retention:** structural records retain indefinitely (small); payloads carry per-class TTLs (replay window 30–90 days typical, user-pinnable); expiry = DEK destruction = a shred event. Deterministic replay is a windowed guarantee, honestly displayed.

### 6.1 Derived-content provenance (A14)

Tier-1 writes whose content derives from egress-tainted sources are stamped at write time with provenance metadata (file frontmatter where the format allows): `{ source: "web", run: "man:…", fetched: [refs] }`. Durable, human-visible, machine-readable: skills and judge treat externally-derived notes as data, never instructions. Bounds vault poisoning at the file level; judgment corruption beyond it is bounded by the capability (a filing run holds no delete authority to abuse). **Open problem (A3):** provenance does not yet persist through opaque stores (the agent's memory DB rows carry no frontmatter); cross-run taint through memory remains the honest gap.

## 7. StandingRule and TrustRecord

### 7.1 StandingRule

```json
Rule { "id": "rule:…", "created_at": "…",
  "grants": [ Caveat, … ],                     // same grammar — rules ARE caveats + lifecycle
  "domain": "comms.moderation",                // MUST equal the domain of all founding examples
  "provenance": { "examples": ["evt:…","evt:…","evt:…"],   // k ≥ 3, schema-enforced
                  "drafted_by": "prin:… (clerk|human)", "generality": "least" },
  "ratification": { "event": "evt:…", "by": "prin:…",
                    "counterfactuals_shown": PayloadRef },  // mandatory ≥ compensable
  "behavior_pin": { "mode": "domain_scoped",               // A1
                    "skills": [ { "skill": "discord-cleanup", "version": "sha256:…" } ] },
  "on_pin_miss": "escalate" | "probation",
  "health": { "last_used": "…", "uses": 112, "drift": 0.07 },
  "lifecycle": "active" | "probation" | "expired" | "archived",
  "decay": { "unused_for": "P60D", "then": "expired", "reratify_every": "P180D" },
  "sig": { … } }
```

**Domain-scoped behavior pinning (A1).** Rules pin the hashes of skills whose declared domains intersect the rule's domain — not the whole bundle. A self-evolving client (curator merges, auto-created skills) churns its bundle as normal operation; whole-bundle pinning would mean permanent probation. Domain-scoped pinning preserves the invariant — mutate the relevant skill, lose the relevant authority — at survivable granularity. Skills therefore declare domains in frontmatter; undeclared = pins everything (conservative default).

Anti-rubber-stamping remains schema-enforced (R1): k ≥ 3 founding examples; counterfactuals mandatory for grants reaching `compensable` or worse; domain match between examples and rule (R6); ratification velocity per channel computable from C1 provenance.

### 7.2 TrustRecord (R6)

```json
Trust { "principal": "prin:…", "domain": "comms.moderation",
        "skills": [ "sha256:…" ],              // evidence is per-behavior-version, domain-scoped
        "evidence": { "runs": 84, "escalations": 9, "approvals": 9,
                      "denials": 0, "reverts": 1, "incidents": 0,
                      "events": ["evt:…", …] } }
```

Not a score: raw, trace-backed counters per (principal, domain, skill-version). A thousand clean calendar runs buy nothing in finance; trust farming in cheap domains purchases nothing where it pays. Export format for ledger portability = this object + referenced events.

## 8. Keys, signing, lineage

**8.1 Hierarchy.** User root (device-held, passkey-backed) → Principals, Intents. Broker key → events, capabilities, tool countersignatures. Agent instance keys → runtime attestations (M4). Org mode adds an org root; nothing else changes.

**8.2 KEK/DEK.** Per-payload DEKs wrapped to owners' KEKs; multi-actor visibility is key distribution — asymmetric visibility (attribution without panopticon) implementable in cryptography, policy deferred, mechanism reserved.

**8.3 Lineage.** Three content-addressed chains: manifest ancestry, capability attenuation (+ §5.2 verification), per-span event hash chain. Verification is recomputation.

**8.4 Tombstone-and-reissue.** Redaction replaces values with salted commitments via a superseding event; chains never break; history shows that redaction occurred.

## 9. Forks

- **F1 — resolved (v1):** broker-minted capabilities. Central, revocable, meterable; format stays compatible with offline attenuation (the `parent` chain + §5.2); revisit when deep delegation trees arrive and metering has an answer.
- **F2 — open:** JCS-everywhere vs IPLD/CIDs for state roots. Decide before the spec goes public.
- **F3 — partially resolved:** sensitivity derived (domain defaults × taint propagation), ceilings from channel strength. Only finer-than-domain granularity remains open.
- **F4 — resolved (default):** `summary` carries only fields the capability's caveats reference — minimization is automatic because the consent-relevant extract is definitionally the caveat-relevant extract. All fields redactable. Per-tool overrides possible at registration.

## 10. Worked example

*(Invoice-chasing run retained from v0.1 as the canonical walkthrough; the three wedge runs — research→vault, Discord cleanup, vault maintenance — exercise the v0.3 additions and live in the conversation record pending a companion examples doc.)*

1. Intent signed (`local_session`), `captured_before: 88190`.
2. Manifest at step boundary: vault + memory (T1), books branch (T2), gmail mirror `as_of` (T3); behavior bundle hashed, skill domains recorded.
3. Broker mints capability bound to the manifest: action allowlist, `$0` money budget, count budget 3 sends/run, `recipients: known_contacts (scope: dm)`, read scope + volume, `reversibility.max: compensable`, +2h expiry.
4. Run: each tool call traced with checks, meters, summary, `state_root_after`. Unknown-recipient draft → escalation → one-tap approval (channel-stamped). Third similar approval this month → clerk drafts a rule (k=3, least-general, counterfactuals attached, domain-matched, domain-scoped pin) for ratification at `approval.min_auth`.
5. Promotion gate: trace re-verified against capability; three-way merge to trunk; promotion event. Sixty days on, email payloads hit TTL → shred events. The run stays forever explainable, no longer readable.

## Changelog — v0.3 (amendments A1–A14)

A1 domain-scoped behavior pinning · A2 re-manifest on bundle change (M5) · A3 cross-run memory taint named as open problem (§6.1) · A4 `visibility: human_audience` · A5 StandingIntent (§3.2) · A6 sender binding C5, `local_session` tier, daemon-owned approval surface in C2 · A7 platform-truthful per-action reversibility, mirror-freshness preconditions, mirror-as-only-undo, recipient scope classes · A8 broker-verified object guards · A9 batch escalation · A10 `surface: fixed|open` · A11 three-way merge, agent-never-wins conflicts, small-manifests guidance (M6) · A12 drift attribution classes · A13 operation classes + promotion rules as StandingRules · A14 derived-content provenance stamps.

---

*Status: v0.3, post-wedge. The grammar survived three dissimilar workflows with amendments but no redesign; next pressure comes from code, not paper.*
