# Agent State Fabric — Schema Specification

**Draft v0.7 — July 2026 — Companion to the Architecture Brief**

*v0.3 integrated amendments A1–A14 from the wedge paper runs (Hermes agent; workflows: web research → vault distillation, Discord message management, vault maintenance). v0.4 integrated A15–A19 from the Stage 1–3 reference implementation (Coppice): the first amendments forced by running code rather than paper runs. Every A15–A19 decision traces to the implementation sessions via `docs/spec-issues.md` (SI-1…SI-19, all resolved in that version). v0.5 integrates A20 (SI-20, M8 attribution completeness) — the first amendment forced by dogfooding rather than by implementation. v0.6 integrates A21 (SI-21): brokered authority binds by mode declaration plus grant event, never by an embedded capability id — resolving the content-address cycle at the kernel object. v0.7 integrates A22 (SI-24): capabilities gain early closure — a signed `revoke` event as the permanent, prospective, descendant-closing dual of A21's `grant`, with liveness a pure event-derived view evaluated at the operation's durable authorization offset (§5.4). Changelog at end. Risk-review requirements carried since v0.1: **(R2)** read authority is first-class, **(R3)** payloads are hash-referenced and destroyable, **(R6)** trust is domain-scoped, never scalar.*

---

## 0. Conventions

- **Serialization:** canonical JSON (JCS, RFC 8785). Every object's `id` is `<prefix>:<hex sha256>` of its canonical body excluding **both `id` and `sig`** (A19); the Ed25519 signature covers those same body bytes, so id-check and signature-check attest identical content. Numbers in fabric objects MUST be integers with |n| < 2^53 and floats are forbidden (A16): RFC 8785 serializes numbers ECMAScript-style, which diverges across implementations beyond that range. All cross-references are by id; lineage is tamper-evident by construction.
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

**Cipher suite (A19).** v1 payload encryption and DEK wrapping are AES-256-GCM; each DEK record carries its `alg`, so the suite is per-payload upgradable without a schema change.

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
- **C6 — Auth-strength order (A19).** Wherever strengths are compared (`approval.min_auth`, expanding amendments): `unverified < platform_oauth < passkey = local_session` — C4 already groups the last two as the strong tier. Unrankable strengths fail closed.

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
  "authority": { "mode": "brokered" },
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
- **M7 — Authority mode and binding (A21; supersedes the A19 observed-run form).** `authority` declares the run's enforcement mode and never embeds a capability id: `{ "mode": "brokered" }`, or absent. Absent denotes an *observed* run — snapshots, trace, drift attribution, and revert apply in full; no broker enforcement is claimed, and the manifest makes that ledger-visible. The capability id cannot live in the body because authority is a *response* to the sealed fork (F1 mints against `man.id`), so the authority lineage is two acyclic edges: **backward**, `cap.bound_manifest == man.id` (M2, content-addressed — trustworthy exactly because mint follows seal); **forward**, the broker's signed `grant` event (§6) countersigning the capability into the substrate at its activation offset. The event is the proof; the field is the mode (the SI-10 pattern). **Fail-closed rule:** under `mode: "brokered"`, every effect must be attributed — each `tool_call` carries its capability, and a verified `grant` event binding that capability to this manifest must precede it in substrate order. A declared-brokered manifest whose effects cannot satisfy this is invalid at the gate; it never silently reclassifies as observed. Observed mode claims nothing and forbids nothing: capability-attributed calls, where present, are checked in full (M1/M2 bind whenever a capability exists) — the declaration only ever adds constraints. Multiple grants over one manifest are legal and expected (expiry re-mint, §5.2 attenuation); the authority lineage is the offset-ordered set of verified grants. Closure is the dual edge (A22): a signed `revoke` event ends the lineage prospectively at its offset, so the ordering check is grant-before-effect AND no closure of the capability or its verified ancestry before the effect's durable authorization offset (§5.4). Export note: the portable single-artifact form is a *derived* DelegationEnvelope — manifest + capabilities + their grant-event references, materialized on the §7.2 export pattern (object + referenced events) — never load-bearing; the chain is ground truth.
- **M8 — Attribution completeness (A20).** Every window of out-of-band divergence (the net change between consecutive root attestations) is recorded by exactly one drift event — carrying A12 attribution and an A13 operation-class summary — before the diverged state is consumed by any merge or attested as expected by any subsequent event. Ledger attribution never depends on when a change occurred relative to session lifetime. Stated as a property of the ledger, not of the gate, so no future code path reopens the window by other means: every consumer of live state (auto-promotion, approval-time re-merge, revert) attributes first, and gates serialize per §5.3.
- **Root encoding (A16 interim).** State roots become CIDv1 (F2, §9); `sha256:<hex>` is the sanctioned interim encoding until the F2 execution checkpoint. Readers MUST accept both during the transition. sqlite roots are the checkpointed main-file byte image in the interim; page-aligned chunked DAGs at F2 execution.

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

**Capture proof (A19).** `captured_before` is a convenience field; the *proof* of pre-contamination is the substrate `intent` event (§6) countersigning the intent id at its actual offset. Verifiers MUST check the event, not trust the field.

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
- **Enforcement bindings (A19).** Each action declares `store` (which registered store it operates on — the input to the M1 check at mint time), `class` (its action class for `budget.count` metering and §5.3 `ops` matching), and `path_args` (which argument fields carry write paths, for `paths.write` extraction). Declared, trusted-mechanical, same tier as `writes`.
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

Per dimension, the child's permitted set MUST be a subset of the parent's: caps ≤, windows within, globs narrower, allowlists ⊆, reversibility no worse, expiry no later. Absent-in-parent = unrestricted there; children may add dimensions, never remove or widen. Verification is mechanical subset-checking; failure invalidates the capability outright. Attenuation is also subject to closure (§5.4, A22): a parent closed at the child's grant offset cannot produce a live child — refused at mint, and derived by the liveness view regardless of what rows were injected.

**Glob dialect (A19).** Path globs use `**` (any depth) and `*` (within one segment). "Narrower" is verified conservatively: a child glob counts as covered only when provably so (identical; parent `**`; parent `literal/**` whose prefix covers the child). Anything else is not-a-subset — fail closed; rewrite the child glob plainly rather than cleverly.

### 5.3 Promotion (A11, A12, A13)

Promotion merges a completed branch to trunk and is the only mutation in the system.

- **Merge semantics (A11).** Three-way merge: base = the manifest's state root; reconcile agent branch and current trunk against it. **Conflicts never auto-resolve in the agent's favor** — human trunk edits win by default; true collisions surface as conflict cards. Snapshot ancestry survives the merge: promoted changes remain revertible.
- **Opaque stores (A19).** Stores with no sub-file merge (the agent memory db) merge whole-store: a branch-only change installs the branch image (class `modify`); divergence on both sides is a single conflict card, trunk wins, the branch image stays CAS-reachable. Cross-run memory taint (A3) concentrates exactly at this promotion point; the gap remains open and named.
- **Coherent revert.** Revert restores *all* state roots of the manifest atomically — vault and agent memory together — so undo never gaslights the agent with a world its memory contradicts. (This is the cross-layer-consistency thesis, operational.)
- **Operation classes (A13, definitions A17).** Diffs and rules speak `add | modify | delete | move | rename`, with rename detection — a reorganization must never render as mass deletion; that failure of legibility either blocks good work or habituates users to scary diffs. Definitions: `rename` = same content, same parent directory, new name; `move` = same content, new parent directory. Classes are ordered `rename < move`: a rule allowing `move` allows `rename`, never the reverse. Detection at the authority layer is exact-hash and unambiguous-pairs-only; similarity heuristics MAY annotate diff cards for legibility but never produce a class rules can match — a heuristic class is a gameable class.
- **Class extensibility (A18).** The vocabulary extends by refinement only: `<root>.<refinement>` adds a predicate to (never replaces) one of the five structural roots, so a wrong predicate degrades to its parent class. Rules allowing a root allow its refinements; unknown classes fail closed at ratification and at the gate. Content-aware refinements require registered, versioned classifiers, and rules pin the classifier versions they were ratified under (the A1 pattern applied to classifiers).
- **Promotion rules (A13).** Gate decisions are caveat-shaped (paths, operation classes, sizes), so auto-promotion rules are StandingRules in the same grammar — e.g., auto-merge iff paths ⊆ {/inbox, /MOCs}, ops ⊆ {add, modify}, no deletes — ratcheted from early manual approvals. Low-stakes by construction: Tier-1 promotion is always revertible.
- **Trace-vs-capability check.** At the gate, the recorded trace is verified against the capability — did the run do anything its token shouldn't allow — before anything becomes durable.
- **Divergence attribution and gate serialization (A20, M8).** Before any gate consumes live trunk — auto-promotion, approval-time re-merge, or revert — divergence from the attested roots is recorded as drift (A12 attribution, A13 op summary) and only then merged or erased. Gates serialize per fabric home, cross-process; the divergence check, the merge, and the expected-root attestation are atomic with respect to other gates. Per-store locking is explicitly wrong grain: gates consume and attest all roots as one coherent tuple. Note the A3 consequence: SI-18's whole-store memory merges pass the same pre-check, so the surface where cross-run taint would ride into trunk always produces an attributable event first — a monitored gap, not a silent one.

### 5.4 Closure and revocation (A22)

Capability objects are immutable; **current validity is a materialized view of signed objects plus the verified substrate prefix** — the A15 rule applied to the authority lifecycle. A21 supplied the activation edge (`grant`); this section supplies closure. `expires_at` remains the load-bearing automatic closure; the `revoke` event (§6) is the signed early-closure edge. Early operator or security closure is always a `revoke`, so provenance and intent are never conflated with time passing.

**Liveness.** Liveness is necessary for authorization, not sufficient — §4 registration and the other structural rules of the decision pipeline still apply. Capability `C` is live for an operation whose durable authorization offset is `O` iff:

1. a verified `grant` binding `C` to its `bound_manifest` exists at offset `G < O` (M7's activation edge);
2. no verified `revoke` names `C` **or any ancestor in its attenuation chain** at offset `R < O` — quantified over *all* revokes, not merely those after the latest grant, so closure is **permanent per capability id**: a later grant of the same id activates nothing, and revoked-before-ever-granted (`R < G`) is equally dead;
3. `C.expires_at` and every ordinary caveat admit the operation at its signed event time (the SI-22 clock); and
4. the object and its ancestry verify fail-closed: every ancestor verifies under the broker key, and earliest grants are well-ordered along the chain (each ancestor's precedes its child's). Closure of ancestors is condition 2's job, stated once. §5.2 subset semantics remain mint-time-enforced and F1-signature-protected; verifiers MAY re-derive them, but liveness does not require it.

**Two clocks.** Activation and closure order by substrate offset (M7); time-shaped caveats and expiry evaluate at the signed event `at` (SI-22). Offsets are order; timestamps are claims. A revoke's offset is its effective point — no backdating field exists — and revocation is **prospective**: effects durably authorized before `R` remain historically valid.

**Durable authorization offset.** `O` is the offset of the first signed event that commits the fabric to the operation — today the `tool_call` event itself; under the durable external-effect protocol, its signed dispatch/reservation record. Decision time enforces the same view at the current verified head — that is what denies a post-revoke call before any effect; **the gate's re-evaluation at `O` is authoritative for what becomes durable**. Accepted wedge consequence: work recorded after a revoke conservatively fails the gate and the branch strands (revert remains available). An in-memory ticket is never authorization evidence.

**Cascade is definitional.** Descendant closure is not enumerated at revoke time; it follows from condition 2's ancestor quantifier: revoking any capability closes its entire descendant subtree at the same offset; revoking a child leaves parent and siblings live. The view resolves revokes by capability id across the entire verified substrate — never filtered by the evaluating manifest: a child bound to a hermetic sub-agent manifest (M2) still dies with its ancestor's revoke. Mint and attenuation apply the same view at the new grant's offset — a closed parent cannot produce a live child, even if an object row and grant event are injected afterward. Restoration is never reactivation: mint a new capability (new `issued_at`, therefore new content id) and grant it; the broker MUST refuse to grant a closed id, so a timestamp-colliding re-mint of an identical body fails loudly instead of silently issuing a dead token.

**Revocation authority.** Closure only narrows, so per §3.1's directionality a revocation request is accepted from **any registered human channel**; the event carries `channel` + `auth_strength` per C1. The broker MAY emit closure for a mechanically established structural cause (a tool disabled, a behavior-pin policy effect): `channel: null`, `auth_strength: null`, plus a mechanical `reason`. Reasons are documentary and never scope or weaken the closure. The holder cannot revoke through its work channel — text without channel provenance is data, never an authority act (C5's spirit); voluntary surrender, if ever wanted, arrives as a separately attributed broker request.

**Doubt never widens.** Verification doubt resolves against authority on both edges. Activation: a grant that fails verification, or whose binding does not match exactly, activates nothing (A21). Closure: a **signature-verified revoke closes the capability it names and its descendants even when its body or placement is anomalous** (e.g. a `manifest` field disagreeing with the target's `bound_manifest`, or emission on an unexpected span); the anomaly surfaces as a loud substrate-integrity finding, but the closure holds — a kill switch that silently no-ops on malformed emission is the worst outcome. Unsigned state moves nothing in either direction: the view derives from verified events only — a revocation cache row without its event is not consulted, and revoking `C` never affects capabilities outside `C`'s descendant subtree. A revoke naming an id no capability bears currently closes nothing — and per condition 2's quantifier it permanently poisons that id, so a capability minted to it later is born dead (the MUST-refuse rule above makes that failure loud).

**Closure dominates.** A closed capability's denial is structural and **never escalatable** — an escalatable closure would be an un-revoke lever inside the agent's loop. Approvals, exemptions, meters, and caveats cannot resurrect closed authority: pending escalations become visible-but-inert (a post-revoke resolution is historical evidence, never authority; unused exemptions need no destructive deletion — closure denies structurally before exemptions are consulted). Non-retroactivity cuts the other way for promotion: a **parked promotion whose recorded work was durably authorized before `R` remains approvable** — promotion ratifies past work, and the merge is the human's act. Revocation never blocks recovery: revert is operator-side and unaffected; compensation of already-landed effects runs under freshly minted, narrowly scoped capabilities, never resurrected ones.

**Structural, not a caveat.** Closure is a structural precondition of the decision pipeline and of gate replay — like signature, M2 binding, and mandatory expiry — evaluated by one pure event-derived reconstruction shared by both. It is never a §5.1 caveat dimension.

**Kill switch.** "Close everything" is one revoke per live root capability; subtrees close definitionally. The surface on which an operator invokes this under actuation — unreachable by granted hands — is SI-23's question (its C7 candidate); this section supplies the operation, not the surface.

## 6. TraceEvent

Append-only, hash-chained per span, signed by the emitting component.

```json
Event { "id": "evt:…", "span": "span:…", "seq": 41, "prev": "evt:…",
        "manifest": "man:…", "at": "…",
        "kind": "tool_call" | "verdict" | "escalation" | "ratification" | "promotion"
              | "revert" | "compensation" | "grant" | "revoke" | "snapshot"
              | "drift" | "shred" | "amendment" | "remanifest"
              | "register" | "intent" | "approval",
        "body": { … }, "sig": { … } }
```

**Registration and capture events (A15).** `register` records an object joining the fabric — `{object, object_kind}` for principals, channels, tools. `intent` countersigns intent capture into the substrate — `{intent, channel, auth_strength, captured_before}` (this event is what makes §3.1's capture proof real). `approval` records the resolution of an escalation or parked promotion — `{escalation | promotion, resolution, uses?, channel, auth_strength}` per C1. **Objects are materialized views of the event stream:** an object is live only once its register event is on a chain; a stored object row without one fails closed. Events standing outside any manifest (registrations, intent capture, drift between runs) belong to the fabric-lifetime span with `manifest: null`. Chain heads have `prev: null`, `seq: 0`.

**tool_call body:** `{ tool, action, args: PayloadRef, result: PayloadRef, summary: {…redactable, caveat-relevant extract only (F4 default: fields the capability's caveats actually reference)}, checks: [{caveat, ok, meter}], reversibility, compensator, mirror_content: PayloadRef?, state_root_after }`.

**verdict body (A19):** carries `source` — `"broker"` for layer-1 mechanical denials (full `checks` attached), `"judge"` for layer-3 model verdicts (brief §5.4). One kind, one query surface, per-layer attribution.

**escalation body — batching (A9):** violations aggregate per (caveat, action_class): `{ count: 28, sample: [5 refs], guard_status: "all_pass" }` → one approval covers the batch. Twenty-eight pings is the R1 fatigue machine rebuilt; one legible batch is not.

**grant body (A21):** `{ capability, parent }` — the broker countersigns every mint (and every §5.2 attenuation) into the substrate. Recorded on the fabric-lifetime span with `manifest` set to the capability's `bound_manifest`, alongside the other authority acts (`register`/`intent`/`approval`, A15). Its offset is the capability's activation point; M7's ordering check compares it against effect offsets. A capability object row without a verified grant event fails closed — the A15 materialized-view rule applied to authority.

**revoke body (A22):** `{ capability, reason, channel, auth_strength }` — the signed early-closure edge, dual of `grant` (§5.4). Recorded on the fabric-lifetime span with `manifest` set to the target capability's `bound_manifest`, mirroring `grant`; per §5.4 a manifest-field anomaly never weakens the closure. Human-originated revocations carry `channel` + `auth_strength` per C1; broker-originated mechanical closures carry `channel: null`, `auth_strength: null`, and a mechanical `reason` (never `operator_request`). Reason vocabulary (documentary, open): `operator_request | compromise | behavior_change | tool_disabled`. Its offset closes the named capability and its entire descendant subtree, permanently for those ids (§5.4). The never-emitted `expiry` kind is removed in the same amendment: `expires_at` in the signed object is the sole time closure, and an unenforced closure-shaped kind is two bug surfaces (absence read as liveness, presence read as enforcement).

**drift body — attribution classes (A12), narrative (A20):** `{ store, expected_root, observed_root, between: [offset_lo, offset_hi], attribution: "human_local" | "tool_known" | "unattributed", ops: [A13 operation-class summary] }` — `between` brackets the unobserved change in substrate offsets (A19). `ops` names what changed in the same vocabulary rules and promotion previews speak (rename/move detection per A17; opaque stores degrade to whole-store `modify` per SI-18), so the ledger narrative is equivalent no matter when the edit happened (M8) — a drift that names its paths is much harder to misread than one that names two hashes. Paths land in the plaintext substrate: the same exposure promotion event bodies already accept; any future substrate-minimization pass treats both together. Zero-authorship default for the solo operator: local edits outside agent spans attribute quietly to the human (logged, not alerted); only `unattributed` drift is loud. In a co-edited vault drift is Tuesday, not an incident.

**Human-originated events** (approvals, ratifications, revocations) carry `{ channel, auth_strength }` per C1.

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

**8.1 Hierarchy.** User root (device-held, passkey-backed) → Principals (human), Intents. Broker/fabric key → events, capabilities, tool countersignatures, **manifests, channel registrations, non-human principals** (A19 — the component that constructs a record at the step boundary signs it). Agent instance keys → runtime attestations (M4). Org mode adds an org root; nothing else changes.

**8.2 KEK/DEK.** Per-payload DEKs wrapped to owners' KEKs; multi-actor visibility is key distribution — asymmetric visibility (attribution without panopticon) implementable in cryptography, policy deferred, mechanism reserved.

**8.3 Lineage.** Three content-addressed chains: manifest ancestry, capability attenuation (+ §5.2 verification), per-span event hash chain. Verification is recomputation.

**8.4 Tombstone-and-reissue.** Redaction replaces values with salted commitments via a superseding event; chains never break; history shows that redaction occurred.

## 9. Forks

- **F1 — resolved (v1):** broker-minted capabilities. Central, revocable, meterable; format stays compatible with offline attenuation (the `parent` chain + §5.2); revisit when deep delegation trees arrive and metering has an answer. "Revocable" has normative semantics since A22 (§5.4): signed `revoke` events, permanent per id, prospective, descendant-closing through the ancestry view.
- **F2 — resolved in direction (A16; ADR 0002 in the reference implementation):** the control plane stays JCS (with the §0 integer rule); state roots become CIDv1 over DAG-CBOR tree nodes with raw leaf blobs and chunked large objects (sqlite chunked on page boundaries). `sha256:<hex>` is the sanctioned interim root encoding until the execution checkpoint (post-dogfooding storage tripwires); readers MUST accept both during the transition. Scope fence: CIDs ≠ IPFS — an addressing/serialization format only; no DHT, no gateways, no sync protocol.
- **F3 — partially resolved:** sensitivity derived (domain defaults × taint propagation), ceilings from channel strength. Only finer-than-domain granularity remains open.
- **F4 — resolved (default):** `summary` carries only fields the capability's caveats reference — minimization is automatic because the consent-relevant extract is definitionally the caveat-relevant extract. All fields redactable. Per-tool overrides possible at registration.

## 10. Worked example

*(Invoice-chasing run retained from v0.1 as the canonical walkthrough; the three wedge runs — research→vault, Discord cleanup, vault maintenance — exercise the v0.3 additions and live in the conversation record pending a companion examples doc.)*

1. Intent signed (`local_session`), `captured_before: 88190`.
2. Manifest at step boundary: vault + memory (T1), books branch (T2), gmail mirror `as_of` (T3); behavior bundle hashed, skill domains recorded.
3. Broker mints capability bound to the manifest: action allowlist, `$0` money budget, count budget 3 sends/run, `recipients: known_contacts (scope: dm)`, read scope + volume, `reversibility.max: compensable`, +2h expiry.
4. Run: each tool call traced with checks, meters, summary, `state_root_after`. Unknown-recipient draft → escalation → one-tap approval (channel-stamped). Third similar approval this month → clerk drafts a rule (k=3, least-general, counterfactuals attached, domain-matched, domain-scoped pin) for ratification at `approval.min_auth`.
5. Promotion gate: trace re-verified against capability; three-way merge to trunk; promotion event. Sixty days on, email payloads hit TTL → shred events. The run stays forever explainable, no longer readable.

## Changelog — v0.7 (amendment A22)

*Resolves SI-24 (filed 2026-07-11 by the revocation design review — separating key destruction from authority closure; challenged and ratified 2026-07-12; provenance in `docs/spec-issues.md`).*

A22 — capability early closure: new §5.4 — validity is a materialized view of signed objects plus the verified substrate prefix; `revoke` joins §6 as the signed, permanent, prospective dual of `grant` (body `{capability, reason, channel, auth_strength}`; fabric-lifetime span, `manifest` = target's `bound_manifest`). Liveness at the operation's **durable authorization offset** (today the `tool_call` event; the durable-effect protocol's dispatch record when it exists) — the current wedge conservatively strands branches recorded after a revoke. Descendant cascade is **definitional** via the ancestry condition, never enumerative — the candidate's `cascade` and `source` fields were dropped at ratification (enforcement reads neither; provenance derives from `channel` per the C1 approval pattern). **Doubt never widens**, stated for both edges: activation doubt → not granted (A21); closure doubt → not live — a verified-but-anomalous revoke still closes, loudly; unsigned rows move nothing. Closure denials are structural and non-escalatable; approvals/exemptions/meters cannot resurrect; parked promotions of pre-revoke work remain approvable; recovery (revert; compensation under fresh narrow mints) is never blocked. Ancestry verification bounded to per-hop signature + well-ordered earliest grants + closure absence (§5.2 subset stays mint-time + F1-signature-protected). Closure is a structural precondition, never a §5.1 caveat dimension (pinned by the W-9 corpus verdict-invariance contract). Permanence is the condition-2 quantifier: all revokes before `O`, including revoked-before-granted; restoration = new mint (new `issued_at` ⇒ new id) + grant, and the broker refuses to grant a closed id (a timestamp-colliding identical-body re-mint fails loudly instead of silently issuing a dead token). The never-emitted `expiry` event kind is removed. Rejected: revocable-flag-on-object (mutates a signed object); closure-as-caveat (holder-declared grammar carrying substrate state; breaks corpus verdict invariance); non-cascading parent revoke (leaves derived authority standing on a revoked base — re-mint the child instead); escalatable/un-revoke closure (an un-revoke lever inside the agent's loop); retaining `expiry` as an observational kind (two bug surfaces, zero function); and the ratification call's fallback of retracting F1's "revocable" claim to rely on short expiry alone (leaves a compromised long-lived capability unstoppable — P25's exact gap).

## Changelog — v0.6 (amendment A21)

*Resolves SI-21 (filed 2026-07-09 by the dogfooding-readiness review; ratified 2026-07-10; provenance in `docs/spec-issues.md`).*

A21 — brokered authority binds without a content-address cycle: manifest `authority` becomes a mode declaration (`{"mode":"brokered"}`; absent = observed), never an embedded capability id; the authority lineage is two edges — backward `cap.bound_manifest` (M2, content-addressed) and forward the signed `grant` event (§6) at its substrate offset; M7 rewritten with the fail-closed rule (brokered effects require an attributed capability whose grant precedes them in substrate order; declared-brokered never silently reclassifies as observed; observed mode only ever loses constraints, not checks); grant body specified; DelegationEnvelope named as a derived export view on the §7.2 pattern. Candidate families considered and rejected: envelope-as-object (under crash analysis its absence is ambiguous, so the discriminator falls back to the substrate — A15's row-without-event rule already governs; adopted as export view only) and pre-seal capability commitment (welds the broker into the seal critical path, invents a second body-minus-field hash rule, no re-grant story).

## Changelog — v0.5 (amendment A20)

*The first amendment forced by dogfooding (SI-20, found and ratified 2026-07-09; provenance in `docs/spec-issues.md`).*

A20 — M8 attribution completeness: every consumer of live state (auto-promotion, approval-time re-merge, revert) records out-of-band divergence as drift — A12 attribution + A13 op-class summary — before consuming it; one drift event per divergence window; drift bodies gain `ops` for narrative parity across timings; gates serialize per fabric home (cross-process), with check/merge/attestation atomic with respect to other gates. Corollary: SI-18 whole-store memory merges always produce an attributable event before cross-run taint (A3) could land — the gap becomes monitored, not silent.

## Changelog — v0.4 (amendments A15–A19)

*First amendments forced by running code (Coppice, Stage 1–3). Full per-issue provenance in `docs/spec-issues.md` (SI-1…SI-19); design exchanges in the implementation sessions of 2026-07-08/09.*

A15 registration/capture/approval event kinds, objects-as-materialized-views fail-closed rule, fabric-lifetime span, chain-head `prev: null`/`seq: 0` (SI-4, SI-5, SI-10, SI-11, SI-13) · A16 F2 resolved in direction — JCS control plane with the §0 integer rule, CID/DAG-CBOR data plane, interim `sha256:` root encoding (SI-6; F2) · A17 move/rename definitions, `rename < move` ordering, exact-hash authority layer with similarity as display-annotation only (SI-17) · A18 `link` removed from the §5.3 example; refinement-only class extensibility with pinned classifiers (SI-19) · A19 conformance sweep — id/signature hashing precision (SI-1, SI-2), signer assignments (SI-3), cipher suite naming (SI-9), auth-strength order C6 (SI-15), observed-run manifests M7 (SI-7), §4 enforcement bindings (SI-16), conservative glob dialect (SI-12), verdict `source` field (SI-14), drift `between` offsets (SI-8), opaque-store merge (SI-18).

## Changelog — v0.3 (amendments A1–A14)

A1 domain-scoped behavior pinning · A2 re-manifest on bundle change (M5) · A3 cross-run memory taint named as open problem (§6.1) · A4 `visibility: human_audience` · A5 StandingIntent (§3.2) · A6 sender binding C5, `local_session` tier, daemon-owned approval surface in C2 · A7 platform-truthful per-action reversibility, mirror-freshness preconditions, mirror-as-only-undo, recipient scope classes · A8 broker-verified object guards · A9 batch escalation · A10 `surface: fixed|open` · A11 three-way merge, agent-never-wins conflicts, small-manifests guidance (M6) · A12 drift attribution classes · A13 operation classes + promotion rules as StandingRules · A14 derived-content provenance stamps.

---

*Status: v0.7, mid-dogfooding. v0.3's grammar survived three dissimilar paper workflows with amendments but no redesign; v0.4's amendments came from running code; A20 came from dogfooding, A21 from its readiness review, and A22 from the revocation design review that separated key destruction from authority closure ahead of first egress — every wave clusters on precision, not missing concepts; the compositional-grammar thesis is holding.*
