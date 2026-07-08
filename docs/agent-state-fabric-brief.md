# The Agent State Fabric

**Architecture Brief — Draft v0.2 — July 2026**

*v0.2 incorporates the reflexive-risk review (risks created by the architecture's own success), the front-door model (user↔agent channels as registered, authenticated objects), the zero-authorship rule, and the results of three wedge paper runs (Hermes agent: web research → vault distillation; Discord message management; knowledge-vault maintenance) that produced schema amendments A1–A14. Companion document: the Schema Specification, now at v0.3.*

---

## 1. Thesis

Agentic computing needs a state fabric whose kernel object is the **delegation manifest**: a signed record binding four lineages — state, authority, behavior, and trace — at every agent handoff. Reversibility (snapshots) covers everything the user owns; bounded capabilities cover everything they don't; and two human-ratified compilation loops turn accumulated traces into competence (skills) and calibrated authority (caveats). The fabric is agent-agnostic by construction: it wraps whatever agents the user already runs, and its neutrality is checkable in its formats rather than promised in its marketing.

The product this architecture yields is not a governance system that happens to support undo. It is an undo system that happens to yield governance — a management layer for organizations whose employees are agents, compressed into artifacts one human can supervise.

## 2. Why now

The move to agentic computing does not obsolete storage media or core data structures. Copy-on-write trees, WAL-based page servers, and object stores are fine. What breaks is the **control plane**: the metadata, lifecycle, identity, and policy machinery above the stores, which was architected for human cadence and human cardinality.

Three shifts drive this:

**Cardinality and cadence.** A human developer snapshots occasionally; an agent fleet wants snapshot-before-every-delegation across thousands of concurrent ephemeral workspaces. The primitives (snapshots, branches, event logs, scoped credentials) are decades old; the demand is for them at four to five orders of magnitude higher invocation rates, by principals that live for seconds. Market evidence: the majority of databases on branch-native platforms are now provisioned by agents rather than humans, and the major clouds are re-plumbing object storage to serve as the *environment* where agent work happens rather than the destination where it lands.

**The trust bottleneck.** The binding constraint on delegation is not model capability but justified caution: users cannot bound the blast radius of an agent's actions, so they either under-delegate or over-expose. Non-human identities already vastly outnumber human ones and are growing fast, while the security model for them remains identity-static rather than task-scoped.

**Shared, open-world state.** Outside the enterprise, agents and humans operate on the same state — local files, spreadsheets, SaaS accounts — with no platform team to force every write through a sanctioned path. Any architecture that assumes a closed world (every mutation mediated) is unbuildable for this market.

## 3. Design principles

1. **Delegation is the atomic event.** Every handoff — user to agent, agent to sub-agent — forks state, attenuates authority, pins behavior, and opens a trace span, bound in one signed manifest.
2. **Immutable by default; promotion is the only mutation.** Working state lives on branches. Nothing merges to the durable trunk without passing a gate. Everything else is append.
3. **Snapshot-heavy, log-light.** Snapshots are load-bearing (undo never depends on log completeness); the trace log annotates and attributes. Content-addressing makes log gaps *detectable* even where they are not preventable: the log can never lie without getting caught.
4. **The agent never holds the real key.** Credentials live in a vault; a broker injects them at call time; agents hold opaque, attenuated capabilities. Anything in an agent's context window is presumed exfiltratable.
5. **Authority is bound to behavior version.** Trust is evidence about a specific behavior. When the behavior (skill/prompt bundle) mutates, standing grants earned under the old version drop back to escalation until trust re-accrues.
6. **Humans ratify; models draft.** Model judgment may propose rules and pre-filter escalations, but no model verdict silently creates standing authority. Ratification is a permanent load-bearing wall, not a training wheel.
7. **Zero friction by default.** Observation, snapshotting, and tracing at no perceived cost; enforcement is configuration, not substrate. The fabric must be architecturally incapable of slowing down a user who wants full speed.
8. **Neutrality is structural.** Open formats, exportable ledgers, no first-party agent. The commitments that matter are the ones that would be visibly broken, not the ones promised.
9. **Zero authorship.** Every policy dimension ships with a no-configuration default derived from registered metadata (channel strength, tool domain, reversibility class); user-authored policy enters the system only through the ratification loop, never through upfront configuration screens. A control that requires the user to author policy in advance is a design failure regardless of how good the policy would be.
10. **The front door is inside the perimeter.** The user↔agent conversation mints all authority, so its carriers are registered channels with authentication strength — sender-bound, not endpoint-bound. Approval flows terminate at the broker and never pass through the agent; instructions without channel provenance are data, not commands. The fabric does not own the conversation (that is the agent vendor's product); it owns four moments of it — consent, escalation, ratification, undo — delivered over any registered channel at sufficient strength.

## 4. The kernel: the delegation manifest

The manifest is a small, signed, immutable record created at every delegation, binding four lineages:

- **State lineage** — content-addressed roots of the forked workspace: filesystem snapshot hash, database branch id, index versions, and (for external systems) shadow-mirror root hashes. Parent manifest reference gives full ancestry.
- **Authority lineage** — the attenuated capability granted to the delegate: caveats expressed as budgets, counts, recipients, paths, time windows, and reversibility classes. Attenuation-only: a child can never hold more authority than its parent chain granted.
- **Behavior lineage** — the content hash of the skill/prompt bundle the delegate runs. This is what standing grants pin against.
- **Trace lineage** — the span id under which every subsequent tool call, judge verdict, and escalation is recorded.

Because agents act in discrete tool-call steps, delegations and snapshots occur only at step boundaries, where nothing is in flight. The classical distributed-snapshot problem degenerates into metadata bookkeeping: at the boundary, collect each store's current root, write the tuple, sign it. This is what makes cross-layer consistency buildable now, on top of existing stores, without touching their internals.

Completion runs the loop in reverse: the child's result branch either passes the promotion gate — where the trace is also checked against the capability ("did this agent do anything its token shouldn't allow") — and merges to the parent's trunk, or is abandoned and garbage-collected with its capability already expired.

## 5. Layer specifications

### 5.1 Trace substrate

An append-only, signed event log of the records the fabric itself emits: manifests, capability grants and expirations, tool-call records (inputs, outputs, state-root pointer), judge verdicts, escalations, ratifications, promotions, and reverts. This narrow set is fully event-sourced (Option A semantics): the fabric owns these schemas, so completeness is enforceable and cheap. Everything else in the system — the big messy stores underneath — is observed rather than mediated, with content-addressed snapshots making unobserved drift detectable ("state changed between steps 12 and 13 with no recorded cause").

The trace is the single substrate feeding everything above it: undo attribution, audit, replay, the judge's viewport, and both compilation ratchets.

### 5.2 Snapshot layer (owned state)

Cheap content-addressed, copy-on-write captures at every step boundary, across three tiers of decreasing guarantee:

- **Tier 1 — owned local state** (workspace files, local databases): full CoW snapshots; undo is mechanical and unconditional.
- **Tier 2 — owned remote state** (user's cloud databases, repos): branch/fork primitives of the underlying platform, recorded as branch ids in the manifest.
- **Tier 3 — SaaS shadow state** (email, accounting, sheets held by third parties): a synced local mirror is the snapshot substrate; the broker's write log against the real system is authoritative for what changed remotely. Weaker guarantees, by design rather than omission.

Undo is state-based and never depends on why something changed. Diffs between snapshots, joined against the trace, produce attribution: every change explained by a recorded action, or flagged as external (a human edit, a rogue process) — which in a shared-workspace market is information, not failure.

### 5.3 Capability broker (external effects)

The broker is the single enforcement chokepoint between agents and the world, MCP-shaped so it drops in front of any agent client without vendor cooperation. Its responsibilities:

**Credential custody.** Real secrets live in the vault and are injected into tool calls at runtime; neither the agent nor its context window ever sees them. Rich internal caveat semantics compile down at call time to whatever the far side accepts — a scoped OAuth token, a held API key, a virtual card.

**Caveat enforcement.** Grants are legible in human vocabulary — budgets, counts, known-recipients, paths, deadlines — evaluated deterministically per call. The design target for the caveat language is the consent screen first, the enforcement engine second.

**Reversibility classes and compensation.** Every registered tool action declares a class: *reversible* (delete the calendar event), *compensable* (void the invoice, send the correction), or *irreversible* (payment settled). "Undo this run" is then a mixed operation: owned state rolls back mechanically, compensable effects generate drafted compensations for one-click approval, irreversible effects surface with an honest account. Judge scrutiny scales with the declared class.

**Intent binding.** The user's original request is captured as a signed intent artifact before untrusted data enters the run, generalizing what payment networks built for transactions to all consequential actions.

**Just-in-time elicitation.** When a task needs an ungranted scope, execution pauses, a granular consent surface appears, and the run resumes — lazily acquired authority for workflows too variable to pre-configure. Escalations batch per (caveat, action-class) — one legible approval covers twenty-eight uniform violations — because per-item pings are the consent-fatigue machine rebuilt by accident.

**Object guards and preconditions.** Destructive actions carry broker-verified predicates over their targets (not pinned, older than ninety days, author not admin) — the broker resolves target attributes itself against the mirror and never trusts agent-supplied metadata — plus environmental preconditions, canonically mirror freshness before irreversible deletes, since for platforms with no undelete the Tier-3 mirror is the only undo that exists. Guards fence the mechanically checkable eligible set so that the skill's judgment errors land only on the harmless.

**Surface classes.** Open surfaces (arbitrary web hosts, where a URL itself can exfiltrate) get the full egress caveat family — method restriction, anonymous-by-default sessions, taint walls on outbound content; fixed API surfaces are external-but-contained and relax to recipient and budget control. Delivery to the user's own channels is also egress: derived-sensitive content inherits a ceiling from the channel's authentication strength (full detail on strong surfaces, summary-with-deep-link on weak ones), with exceptions authored only by the ratchet.

### 5.4 The gate stack

Actions pass through layers of increasing cost and decreasing determinism, each seeing only what the layer below could not decide:

1. **Mechanical caveats** — deterministic, injection-proof, always enforced.
2. **Provenance / taint checks** — semi-mechanical data-flow policies over the broker's known read-set: outbound content derived from out-of-scope reads, recipients introduced by untrusted input rather than by the user.
3. **Model judge** — differently situated, not a second opinion. The judge sees only the manifest's contents: signed intent, the proposed action as structured arguments, the trace and diff so far, and the trust record. It never ingests the untrusted material the worker read, collapsing the injection surface to the artifact under review. It holds no capabilities and can only emit allow / deny / escalate; every verdict is traced and retrospectively scorable. Its job is calibration, not correctness: the only catastrophic cell is the false allow on an irreversible action, and the escalation path (a push notification to the owner) is cheap enough to tune hard against that cell.
4. **Human** — final authority on the judge's residual, at seconds per decision.

For reversible actions the stack barely engages; undo is the safety net. Scrutiny concentrates where the architecture says risk lives: externalized, irreversible effects.

### 5.5 The two ratchets

Both loops consume the same trace substrate and share one mechanism — *traces in, human-ratified legible artifact out* — pointed at different variables:

**Skill ratchet (competence compiles up).** Solved workflows crystallize into versioned skill artifacts; evolution proposals are drafted from execution traces and merged only through review. Skills are files in the versioned workspace, so their content hashes appear in every manifest automatically.

**Caveat ratchet (authority compiles down).** Escalations approved by the human are labeled examples of rules the grant did not yet express. From k similar approvals (never one), the drafting clerk — the model's first authority-adjacent role — proposes the *least general* caveat covering them, presented with counterfactuals ("this rule would also allow X and Y") for ratification. Each ratified caveat converts a class of judgment calls into a deterministic check, permanently shrinking the judge's docket for that workflow. The judge's steady-state role is frontier probe: it exists to make itself unnecessary wherever workflows repeat.

Caveats age like skills rot: usage decay auto-expires unused rules back to escalation, drift detection flags rules whose matching actions increasingly diverge from their founding examples, and high-blast-radius rules require periodic re-ratification. Nothing is deleted; everything archives with lineage.

**The coupling.** Trust was earned by a particular behavior. Standing caveats pin the skill versions that earned them; when a pinned skill mutates, affected grants drop to escalation (or a flagged probation tier) until trust re-accrues under the new version. Corrupting a skill file therefore automatically revokes the authority the old skill earned — the two ratchets cannot outrun each other. The pin is **domain-scoped** (a rule pins only skills whose declared domains intersect its own): self-evolving clients churn their bundles as normal operation, and whole-bundle pinning would mean permanent probation, while domain-scoped pinning preserves the invariant at survivable granularity — the calendar skill evolving never touches the finance rules. This invariant, authority-bound-to-behavior-version, appears to be novel in the current landscape.

## 6. Replay

Three tiers with distinct determinism requirements and costs:

- **Forensic replay** — fold over the trace to reconstruct what state was at step N. No re-execution; the audit case.
- **Deterministic re-execution** — replay orchestration logic with every side effect and LLM call stubbed by its recorded result (the Temporal model). Nondeterminism is irrelevant because nothing is re-sampled; enables step-through debugging of exactly what happened.
- **Counterfactual replay** — fork all state from checkpoint N, change something, re-run live against the branch, under a capability with zero external reach (or reach only into mocks). Exploration, not reproduction; safe by construction because the capability references the state branch.

## 7. Product vision

**Version control, not a firewall.** Git made developers fearless, not cautious, because any change became survivable — and fearless developers outran careful ones. The product promise is symmetrical: delegate more, sooner, than you would otherwise dare. Brakes are why cars go fast.

**Design center: the all-in operator.** The one-person, agent-heavy shop is the stress test — maximal delegation, zero governance apparatus, highest blast-radius-per-human, zero tolerance for friction. Designing for them forces the zero-friction default; the cautious buyer gets what they need by turning dials up on the same kernel. The reverse ordering (checkpoint system sanded down) is architecturally impossible. The two segments are one product entered through different emotions: the all-in operator arrives after their first incident wanting rewind; the cautious company arrives before their first delegation wanting bounds; both live in the same loop — delegate, watch diffs, ratify, widen.

**The delegation tree is the org chart.** A solo operator running agents is an organization with no management infrastructure. The fabric supplies it: trust ledgers are the personnel file, budget caveats are spend control, the broker is provisioning, manifests and traces are audit and post-mortem, skill-pinned authority is performance review. This is the mental model for the entire UX — the user is not configuring policy; they are managing staff. It also dissolves the multi-actor question for SMBs: humans and agents are principals in one system, because that is what an org chart is.

**The spine (what a user feels in week one):**
1. Universal undo — any run, any time, one gesture.
2. A legible ledger — the diff-attribution view of what happened and why.
3. Grants that read like virtual cards — "$50, this vendor, this week" — never permission scopes.

Everything else (behavior pinning, the drafting clerk, counterfactual replay, compensations) is real, later, and invisible until the spine has earned daily use.

**Progressive trust.** New agents run in ask-every-time mode. Every run produces a manifest, a diff, and an outcome — verifiable history. After a demonstrated clean record, the system shows the evidence and offers a standing grant. Reversal earns authority: the consumer-legible form of least privilege, accrued from inspectable behavior rather than declared in a policy document. No agent vendor can build this loop, because their capability layer has no diff layer beneath it feeding it evidence.

**Cold start.** Two mitigations compose: workflow-template caveat packs (starter rules ratified in one review, not twenty escalations) and a shadow period in which the agent proposes actions with full would-be manifests and diffs before holding any live authority — converting the cold-start liability into an onboarding ritual that teaches the mental model.

## 8. Landscape and whitespace

The market is building every layer separately; nobody has unified them.

**Workspace/filesystem:** hyperscalers re-plumbing object storage into live agent filesystems (S3 Files); enterprise NAS vendors exposing snapshot/clone/provision operations to agents via MCP.

**Sandbox/execution state:** sub-second VM forking and branch-per-attempt patterns (Morph), pause/resume with memory and filesystem state (E2B, Modal), lifecycle-policy persistence (Fly, Daytona). Snapshot-before-delegate exists here as VM lifecycle, not as a cross-store primitive.

**Data layer:** branch-native Postgres at agent cadence (Neon/Databricks Lakebase; majority of provisioning already agent-initiated), agent-payable provisioning (Stripe Projects), git-semantics data stores (Dolt, lakeFS), branching in mainstream platforms (PlanetScale, Supabase).

**Auth/enforcement:** brokered-credential runtimes where agents never see secrets (Arcade, Composio), MCP gateways doing token exchange, JIT elicitation now in the MCP spec, enterprise NHI governance racing on identity.

**Payments (most advanced primitives):** multi-caveat scoped tokens with observable lifecycles (Stripe SPTs), agent-identity-bound tokens (Mastercard Agentic Tokens), cryptographically signed intent traveling with authorization (Verifiable Intent, AP2 Mandates).

**Skills/self-evolution:** trace-driven skill creation and evolution with curator maintenance and PR-gated rewrites (Hermes agent ecosystem).

**Whitespace claimed by this architecture:**
1. Cross-layer consistent snapshots correlated to delegation events (the manifest).
2. Intent-bound, multi-caveat authorization generalized beyond payments to all consequential actions.
3. Authority pinned to behavior version — trust that automatically re-earns across skill evolution.
4. A neutral, portable trust ledger spanning agents, devices, and services.
5. The unified loop: reversal generating the evidence that justifies widening authority.

## 9. Strategy (secondary, but load-bearing)

**Neutrality is the position, enforced by architecture.** The precedent is git/GitHub, not Plaid: radically open formats (manifest, caveat, trace schemas — anyone can read, write, host, fork; exit costs near zero) with the business built on the accumulating asset above them (the broker runtime, ratification UX, hosted trust ledger). The trust ledger is exportable — the moat is not custody but accumulation, and exit rights are the trust product's trust story.

**No first-party agent, ever.** The moment the company fields an agent, every agent vendor reclassifies it from infrastructure to competitor. The gatekeeper cannot work for the party being gated — independence is the category's premise, and it is also why the labs structurally cannot occupy this position: trust earned under one vendor's agent dies at the vendor boundary, and no lab can neutrally custody the record that keeps its own agent honest. The labs are complements: the fabric unlocks delegation their assurance cannot, because the assurance only counts from a party with no agent in the game. Lead with capability ("delegate more, sooner"), let lock-in relief be discovered.

**The wedge is an MCP proxy.** Every major agent client already passes through MCP; a gateway is a drop-in requiring zero vendor cooperation — "works with all agents" true by mechanism, not partnership. Participate in the protocol's auth working groups so the wire converges toward manifest semantics.

**Monetization sits on the closed side of the line.** Charge for runtime, hosted ledger, multi-actor org features; never for spec access or format tolls. Plan the governance handoff early: reference implementation open source from day one, spec donated to neutral governance once adoption makes capture attempts self-defeating.

**Named risks.** (1) Timing: vendor-integrated permission dialogs shipping as defaults train user habits now; default beats neutral when neutral arrives late — the proxy must ship embarrassingly early, undo-first. (2) Credibility: for a no-thumb-on-the-scale business, the cap table and governance are product features; a strategic investment from any single lab converts Switzerland into a forward position. (3) Ceiling: neutrality trades the platform-shaped outcome for the Visa/git shape — smaller surface, deeper moat. This trade should be made explicitly and defended, since most pressure will be to quietly un-make it.

## 10. Risks and open problems

### 10.1 Reflexive risks (created by the architecture's own success)

The v0.1 risks were external (timing, competitors, absorption); these are the shadows cast by the design's own virtues, each answered by a specific structural decision now carried in the schema:

1. **Consent fatigue with receipts.** The ratification loop rewards widening authority with silence, and every over-broad grant carries the user's signature — fatigue laundered into signed over-delegation. Countered structurally: k ≥ 3 founding examples, mandatory interactive counterfactuals for compensable-or-worse grants, batch escalations, channel-stamped approvals making rubber-stamp velocity ledger-visible.
2. **Reads are irreversible.** Exfiltration has no undo; an undo-first product structurally under-attends it. Countered: read scope, read *volume*, derived sensitivity, and egress taint walls as first-class caveat dimensions.
3. **Concentration.** The fabric aggregates exactly what it protects — vault, trace, ledger — into one target, one subpoena magnet, one GDPR collision. Countered: hash-referenced payloads with per-payload keys; erasure is crypto-shredding that preserves lineage (substance destroyable, structure not); windowed replay retention.
4. **Liability flows to the approver.** Intent binding exists in payments to allocate blame; generalizing it inherits disputes without the indemnity structure. Terms-of-service and insurance posture must be designed alongside the architecture; done right, the manifest trail becomes the user's proof of diligence.
5. **Bypass economics.** A slow or down broker gets routed around at the highest-stress moments, leaving invisible coverage holes. Countered by posture: fail open with loud ledger-visible degradation for this market (per-capability invertible), latency as a hard product requirement, honest coverage display.
6. **Monoculture and trust farming.** A winning format makes evaluator bugs systemic, and portable reputation invites farming cheap domains to spend elsewhere. Countered: domain-scoped, behavior-pinned, never-scalar trust records — a thousand clean calendar runs buy nothing in finance.
7. **Accidental surveillance.** In multi-actor mode, diff attribution of human principals is employee monitoring by another name. Countered in mechanism (per-payload key distribution enables asymmetric visibility); policy deferred to the multi-actor design, deliberately.

### 10.2 Open problems (post-wedge status)

1. **Cross-run memory taint (A3) — the honest gap.** Derived-content provenance stamps cover file-shaped stores (vault frontmatter), but opaque stores — the agent's memory database — launder untrusted provenance across sessions. Unsolved; named.
2. **SaaS shadow fidelity.** Mirror lag and API coverage bound Tier-3 guarantees; `as_of` must be legible everywhere, and compensation-by-mirror is *partial* (content survives, identifiers and positions don't) — compensation fidelity grades remain unschematized.
3. **Multi-actor roots.** Per-person roots with shared grants (v1) versus org-root with humans as principals (end state); schema anticipates the latter, policy deferred.
4. **Generalization drafting.** Least-general extraction from k examples is judgment; counterfactual presentation quality decides whether ratification is informed consent.
5. **Judge Goodharting.** Mitigated by the shrinking docket and structured-argument review; not eliminated.
6. **Domain taxonomy governance.** Tools declare their own trust domains; a lazy `misc.general` flattens anti-farming. A small fabric-owned root taxonomy with vendor subdomains is needed before third-party registrations.
7. ~~The schema itself~~ — **done**: Schema Specification v0.3, validated against three dissimilar paper workflows (amendments throughout, redesign never — the amendments cluster on granularity, not missing concepts).

## 11. Sequencing

**Wedge (decided):** client = Hermes agent (MCP-speaking, self-evolving — the behavior-pinning stress test built in); workflows = web research → vault distillation, Discord message management (fixed-surface external effects, including irreversible deletes), and knowledge-vault maintenance (shared human/agent state, the fabric's home turf). Channels: terminal (`local_session`) and Telegram (`platform_oauth`, sender-bound). All three validated on paper against Schema v0.3.

1. **Kernel** — manifest + trace substrate + step-boundary snapshot coordinator over the vault, Hermes' skills and memory; promotion gate with three-way merge.
2. **Spine** — universal undo (coherent across vault *and* memory), diff-attribution ledger with drift attribution classes, virtual-card-style grants through the MCP proxy broker with its own C2-compliant approval surface (day-one requirement, not polish). Ship here; observe-everything, block-nothing defaults. First integration target: workflow 3 — no external registrations needed, and its dogfooding failure mode is "revert and shrug."
3. **Trust loop** — escalation-and-ratification with the human as judge; batch escalations; caveat compilation from approvals; shadow-period onboarding; template caveat packs; StandingIntents for scheduled runs.
4. **Clerk and judge** — model drafts caveats for ratification; model pre-filters the escalation queue; provenance/taint layer including derived-content stamps.
5. **Deep fabric** — compensating-action ledger with fidelity grades, counterfactual replay, caveat aging/curation, multi-actor org model, memory-taint resolution.

Each stage is a working product; each later stage is a feature on the kernel rather than a system beside it.

---

*Companion: Schema Specification v0.3 (objects, caveat grammar, signing and lineage, amendments A1–A14 with per-workflow provenance). The conversation that produced both documents serves as their design rationale.*
