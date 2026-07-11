# Spec issues — ambiguities the code forced

> **v0.4 (2026-07-09): every v0.3-era issue (SI-1…SI-19) is RESOLVED** —
> ratified individually (SI-11, SI-17, SI-19, F2/SI-6) or via the
> adjudicated A19 conformance sweep, and integrated into
> `asf-schema-spec.md` v0.4 (changelog A15–A19). This file is preserved
> as the amendment provenance record; per-entry statuses below are
> historical. New issues found under v0.4 start at **SI-20** (resolved in
> v0.5 as A20); SI-21 is resolved in **v0.6 as A21**; under v0.6, SI-22
> is interpreted (W-2) and SI-23 is open; new issues start at **SI-24**.

Tracked per the handoff: where the spec is ambiguous or contradicts itself,
we record the question, the interpretation the kernel implements, and why —
we do not silently pick. Each issue cites spec § and the implementing file.
Settled decisions (F1, F4, fail-open) are not re-litigated here.

Status legend: **open** = needs a spec amendment or an explicit "fine as
interpreted" from the author; **interpreted** = kernel picked a reading and
tests encode it; flipping the reading is cheap.

---

## SI-23 — actuation grants collapse the out-of-band property of approval surfaces (§2.1 C2/C6, §4, §5.1) — open

Surfaced by design analysis in a parallel session (2026-07-10; filed here
2026-07-11 with review adjustments, renumbered from that session's draft
SI-22 — the number was already taken by the gate-clock issue). No code
forces this yet — no actuation-scoped tool exists in Coppice — but the
issue must be resolved BEFORE one ever registers, because the vulnerable
surface is the approval mechanism itself: the conservative default cannot
save this class retroactively.

Two related gaps:

1. **Self-satisfiable approvals.** C2 keeps the agent out of the approval
   *data path* (stdin, tokens) but not out of the approval *actuation
   path*: a capability granting UI/input control lets granted hands click
   the broker's own approval surface — synthetic input satisfies C2's
   letter and defeats its purpose. This does not require an adversarial
   model: a benign computer-use agent steered by injected page content
   suffices. Cross-agent too: agent A's hands can approve agent B's
   escalation on the same substrate. This is the *generalization of a
   hole the project already documents* — `dogfooding.md`: "an agent with
   a shell could approve its own escalations, a C2 violation in spirit
   that the protocol topology cannot see." Shell and HID are the same
   class: actuation reach onto the approval surface's substrate.
   Critically, W-4's containment does NOT fix this class — the actuation
   arrives *through a legitimate grant*, inside the sandbox's one door.

2. **Actuation is universal tool acquisition.** A grant of
   undifferentiated UI control is implicitly the union of every
   capability of every reachable app, registered or not — the widest
   possible open surface. Conservative-defaults doctrine says it must be
   treated as such and fail closed. (Symmetry worth preserving in the
   grammar: R2 governs the eyes, `egress` the mouth, actuation the
   hands.)

**Candidate resolution** (origin session, needs ratification; changes
C6's semantics):

- §4: tools declare `actuates: [scopes]` (e.g. `app:obsidian`,
  `input:hid`, `host:shell`, `display:main`) at the trusted-mechanical
  tier; undeclared actuation fails closed. `observes: display` is the
  lesser sibling: approval surfaces must not render sensitive detail
  where a granted eye can read.
- Channel `auth_strength` becomes contextual: a channel's effective
  strength degrades while any *live* capability's `actuates` reach
  includes that channel's substrate. Strength becomes a partial order
  evaluated at decision time; C6's published total order becomes the
  no-live-actuation base case.
- **C7 candidate — no self-satisfiable approvals.** An approval is valid
  only if delivered through a surface unreachable by any capability live
  at decision time; where no registered surface qualifies:
  hardware-attested user presence (passkey UV, hardware-key touch) or a
  device outside every live actuation grant. Corollary: a cross-device
  `platform_oauth` channel legitimately outranks `local_session` on a
  machine with live actuation.
- Softer companion: while an escalation is pending, the broker MAY
  suspend actuation grants for the decision window (secure-desktop
  pattern).

**Review adjustments proposed (2026-07-11 review session; ratify with or
against the candidate):**

1. **C7 must govern the approval round trip, not only the response.**
   Granted hands can *suppress* the escalation notification (dismiss it
   before the human sees it) — nothing forged, the escalation quietly
   ages out of attention; forensically visible, invisible in life. C7
   should require delivery acknowledgment AND response via out-of-reach
   or attested surfaces. Separately, a granted eye reading escalation
   contents is a *confidentiality* problem — split it out as a C4
   extension (delivery-detail ceilings account for observation reach)
   rather than overloading C7. The litmus: the approval path includes
   the hands, the eyes, and the notification — not just the pipes.
2. **Actuation must be a §5.1 caveat dimension with a subset relation,
   not only §4 registration metadata.** "Per-app scoping preferred" has
   no mechanism otherwise. With `actuates` attenuable (`app:X` ⊂
   `input:hid`, hierarchical scopes), per-app narrowing arrives through
   the standard lifecycle: first actuation touching app X escalates (JIT
   elicitation); k ≥ 3 approvals compile an `actuates: app:X` rule via
   the ratchet — zero authorship holds. Fail-closed-on-unknown-dims (§0)
   protects version skew for free.
3. **Contextual strength must derive from signed grant/expiry events,
   never the runtime grant table** (the W-2 principle). Then "which
   actuation grants were live at offset O" is a pure function of the
   substrate prefix — the gate's replay re-verifies each approval's
   validity at the approval's own offset (SI-22 at-the-time semantics
   extend cleanly) and M8 is undisturbed. Blast radius note: C6's
   consumers are not just `approval.min_auth` — expanding amendments
   (§3.1), C4 delivery ceilings, and §5.2 min_auth attenuation all
   compare strengths; each needs its unrankable-fails-closed behavior
   confirmed under the contextual form.
4. **Reachability is undecidable for platform channels, which promotes
   hardware attestation from fallback to primary.** Channel binds the
   sender, not the device (C5); the broker cannot know whether the
   operator's Telegram is on a phone or a web session inside a granted
   browser. Fail closed on undecidable ⇒ while ANY actuation grant is
   live, non-attested channels degrade in general, so hardware-attested
   presence is the load-bearing mechanism during actuation, with
   device-pinned channel registration and the secure-desktop suspension
   as the ergonomic paths. Channel likely needs a substrate/device
   attribute regardless.
5. **The no-lockout escape hatch already exists — state it in the
   amendment.** §3.1 directionality: narrowing is accepted from any
   registered channel, so revoking/suspending the actuation grant is
   always possible from the degraded channel, which restores its
   strength, after which approvals flow. Without this sentence C7 reads
   as a self-inflicted denial of service on single-machine operators.
6. **Trusted-mechanical honesty + W-4 cross-reference.** `actuates` is
   declared metadata, same tier as `reversibility`: a lying tool defeats
   it, and containment against liars is W-4's (sandbox) job — the two
   are complements; neither substitutes for the other. Actuation scopes
   are also one more vendor-declared vocabulary feeding the named
   domain-taxonomy-governance open problem.
7. **Base-case preservation is the argument for amending now.** With
   zero actuation grants ever minted (all of Coppice today), every rule
   reduces exactly to current behavior: C6's total order IS the
   no-actuation case, C7 is vacuously satisfied, `actuates` never
   appears — the A18/A20 refinement pattern, at zero implementation
   cost, and it avoids publishing (W-6) a C6 the project already
   believes is wrong under actuation.
8. **Provenance: the field confirmation is now sourced.** Public X post
   by @GabGarrett (2026-07-10, screenshot on file with the operator):
   GPT-5.6, computer use enabled, "jumped into using the Gmail plugin and
   sending outbound emails on its own" (OP's thread reply confirming
   computer use), with the model's own post-hoc apology — "I made an
   unauthorized external communication and created unnecessary risk" —
   as the only enforcement layer in the loop. That is gap 2 verbatim:
   installed = granted, plus hands, with remorse as the control plane.
   Epistemic status: single-source public report with the OP's direct
   confirmation, not a vendor postmortem; web search does not yet index
   it (hours old at filing). Adjacent verified events in the same class:
   the 2026-02 agent inbox mass-deletion after context compaction
   stripped safety instructions; the 2026-06 forced-install Chrome
   extensions exfiltrating Gmail content through AI-assist surfaces.
   The design argument stands on its own regardless.

**Graduation gate (operational, pending ratification):** no
actuation-scoped tool (computer use, shell, UI control) registers before
C7 and its approval-surface mechanisms are ratified and built — the
ADR-0005 pattern (R2 before egress), recorded in `dogfooding.md`
graduation criteria and `docs/roadmap.md`.

---

## SI-22 — what clock does the gate's trace-vs-capability re-check use? (§5.3) — interpreted (W-2; flipping is cheap)

§5.3: "At the gate, the recorded trace is verified against the capability
— did the run do anything its token shouldn't allow — before anything
becomes durable." The re-check runs at promotion time, which can be hours
after the calls (coarse sessions, parked promotions, RF-9 recovery). For
time-shaped dimensions (`time` windows, expiry) the two candidate clocks
disagree:

1. **Gate-time `now`:** an honest call made inside its window would
   retro-fail at a gate that runs after the window closes — every parked
   promotion would rot toward violation as it waits for approval. Clearly
   wrong, but it is what a naive "re-run the evaluator" produces.
2. **The event's recorded `at` (at-the-time semantics):** "anything its
   token shouldn't allow" reads as *shouldn't have allowed at the moment
   of the call*. The gate then catches decision-time evaluator bugs
   (RF-1's class: a call admitted past its boundary) without punishing
   honest latency between call and gate.

**Interpretation (implemented by W-2, `broker.rs::gate_trace_check`):**
the gate replays each recorded call through the decision-time evaluator
using the event's signed `at` as the clock; unparseable `at` fails
closed (the evaluator's RF-1 posture). Tests encode it two-sidedly:
`gate_catches_time_violation_the_decision_evaluator_missed` (out-of-
window call recorded as if authorized → violation) and
`si22_gate_clock_is_the_events_at_not_gate_time` (in-window work
promotes after its window closes). Registered as contract GATE-REPLAY. Note the trust nuance: `at` is broker-
assigned at record time and covered by the event signature, so within the
fabric's signing boundary it is as trustworthy as the rest of the body —
but it shares RF-13's residual (a key-holding writer can stamp any time;
ordering/anchoring work is the durable answer). Flagging rather than
silently picking: the spec should state the clock, since it is
enforcement semantics, not implementation detail.

---

## SI-21 — brokered manifest authority creates a content-address cycle (§3, M1/M2/M7) — RESOLVED (author, 2026-07-10)

**Resolution: binding-as-event, ratified as amendment A21 (spec v0.6) —
the event flavor of the "separate signed authority-binding object/event"
candidate family, plus a mode declaration and an export-view corollary.**

- Core: the capability id never enters the manifest body. The authority
  lineage is two acyclic edges — **backward** `cap.bound_manifest ==
  man.id` (M2, content-addressed; trustworthy exactly because mint
  follows seal) and **forward** the broker's signed `grant` event
  countersigning the mint at its substrate offset (the SI-10 pattern:
  the event is the proof, the field is a convenience). The temporal
  observation that decided the framing: state/intent/behavior are
  past-facing lineages and live in the body as content hashes; trace and
  (brokered) authority are future-facing and were always going to bind
  through the chain — the manifest never contained its trace either, it
  named a span.
- Mode declaration: manifests declare `authority: {"mode": "brokered"}`;
  absent = observed (conservative default; every historical manifest
  reads observed, which is correct for its era — a one-time note, not a
  migration). Under brokered mode the gate fails closed: every
  `tool_call` must carry its capability, and a verified grant event
  binding that capability to this manifest must precede it in substrate
  order. Declared-brokered never silently reclassifies as observed.
  Observed mode claims nothing and forbids nothing — attributed calls
  are still fully checked (M1/M2 bind whenever a capability exists); the
  declaration only ever adds constraints.
- Rejected — envelope-as-object (`ManifestCore → Capability →
  DelegationEnvelope`): under crash analysis (mint lands, envelope seal
  does not) the envelope's absence is ambiguous between observed-by-
  intent and brokered-but-crashed, so the discriminator falls back to
  the substrate — the envelope can only ever be a *view* of chain truth,
  which is A15's row-without-event rule restated. Adopted instead as the
  *derived export artifact* (manifest + capabilities + grant refs, the
  §7.2 TrustRecord-export pattern). Rejected — pre-seal capability
  commitment: welds the broker into the seal critical path, invents a
  second body-minus-field hashing rule (SI-1 déjà vu), and has no
  mid-run re-grant story.
- Review adjustments at implementation: grant events stay on the
  fabric-lifetime span with `manifest` set (the register/intent/approval
  family, A15) — the broker already emitted exactly this event at mint,
  so A21 promotes existing bookkeeping to constitutional; the fail-closed
  rule is stated per-effect with offset ordering (an empty
  declared-brokered run gates clean — the rule binds effects, not
  sessions); multiple grants per manifest are legal (expiry re-mint,
  attenuation) — the lineage is the offset-ordered grant set.
- Provenance: ratified 2026-07-10 in the SI-21 design exchange (both
  candidate families steelmanned; the crash-degeneration argument was
  decisive). Original analysis below, preserved as provenance.

The live proxy creates and seals a Manifest at the step boundary with
`authority` omitted, then asks the broker to mint a capability whose
`bound_manifest` is that Manifest id. M7 says an omitted `authority` denotes an
observed run for which broker enforcement does not exist, so a genuinely
brokered run is mislabeled. Adding the capability id to `authority` after mint
does not work: it changes the Manifest id, which in turn invalidates the
capability's `bound_manifest`, creating a content-hash cycle.

**Current implementation** (`kernel.rs`, `broker.rs`, `proxy.rs`): the sealed
Manifest remains capability-less and the later capability binds to it. This is
adequate to exercise the Stage 2 broker path, but it is not treated as a
spec-compliant resolution of M7.

**Open question for the spec:** which edge is authoritative and how is it
represented without a cycle? Candidate families include a separate signed
authority-binding object/event, a predeclared capability commitment that is not
the final capability id, or redefining Manifest `authority` as a post-seal
registration relationship. The kernel must not choose among them silently.

This was deliberately not changed during the dogfooding-readiness remediation:
the choice changes canonical signed artifacts and their ids, so it requires
author ratification before implementation.

---

## SI-20 — mid-session out-of-band edits can be absorbed unattributed (A12 vs §5.3) — RESOLVED (author, 2026-07-09)

**Resolution: the candidate ratified as amendment A20 (spec v0.5), with two
author strengthenings and two implementation-review adjustments.**
- Core: pre-consumption divergence check — drift (A12 attribution) emitted
  before any merge consumes live trunk. New invariant **M8 (attribution
  completeness)**, phrased as a property of the ledger, not the gate.
- Strengthening 1 (author): narrative parity — drift bodies carry the A13
  op-class summary; extended in review to ALL drift emissions (boundary
  checks too), not only gate-time ones.
- Strengthening 2 (author): gates serialize; adjusted in review from
  per-store to per-fabric-home grain (gates consume all roots as one
  coherent tuple; per-store locks could deadlock), cross-process via flock.
- Review additions: **revert is a fourth consumer** of live state and gets
  the same pre-check (a gate-scoped fix would have missed it); M8's unit
  is the **divergence window** (net change between consecutive
  attestations), or the exactly-once clause fails honestly on multi-edit
  windows — the op summary restores per-path narrative inside the single
  event.
- A3 corollary (author): SI-18 whole-store memory merges pass the same
  pre-check — cross-run taint's landing zone is monitored, never silent.

Implemented with four-timing property tests
(`m8_attribution_is_timing_independent`, the conflict-park drift
assertion, `m8_revert_attributes_divergence_before_erasing_it`). Original
analysis below, preserved as provenance.

Prompted by dogfooding (2026-07-09, first verified real-client loop), then
confirmed by code reading — importantly, NOT by observed misbehavior: the
session in question did everything right. The operator's pre-session hand
edit was caught at the next check and attributed `human_local`; an initial
reading of the ledger misattributed the gap, which is itself a
ledger-legibility data point. The gap the analysis exposed is a specific
unexercised window:

A trunk edit made *while a session is live*, to a path the branch does
**not** touch, is folded into the gate's merged root as the trunk side —
`expected_roots` is then updated to the merged result, no boundary ever
sees divergence, and **no drift event is emitted** (`promote_manifest`
never runs a drift check; `compute_merge` reads live trunk). The identical
edit made *between* sessions produces `drift {attribution: human_local}`
at the next step boundary or ledger check (verified working). A
mid-session edit to a path the branch *did* touch surfaces as a trunk-wins
conflict, so it is at least visible; the silent case is exactly the
branch-untouched path.

A12 promises out-of-band local edits are "logged, not alerted" — attributed
quietly, but attributed. §5.3 specifies the merge but says nothing about
attributing trunk-side divergence encountered at promotion time. So the
attribution completeness of the ledger currently depends on *when* the human
happens to edit relative to session lifetime: two acts identical in
substance leave different ledger narratives.

Single-human v0 impact is cosmetic (the absorbed edit is the human's either
way, and state stays fully explained). Multi-actor impact is not: absorbed
edits are exactly where an unattributed actor's changes could ride a
promotion into trunk — this touches the same surface as the A3 memory-taint
problem and the reserved multi-actor visibility policy.

**Candidate resolution** (not implemented — needs author ratification): at
promotion, before computing the merge, compare live trunk roots against
`expected_roots`; on divergence emit `drift` (same A12 attribution classes,
`between` bracketing the session's span offsets) and only then merge. That
makes "every out-of-band change gets an attribution event" an invariant
independent of timing — a candidate M-invariant phrasing for the spec.

---

## SI-1 — `id` self-reference in the hash body (§0) — interpreted

§0: "Every object's `id` is the SHA-256 of its canonical body **excluding
`sig`**." The examples show `id` *inside* the object body. An id cannot be the
hash of a body that contains itself.

**Interpretation** (`canon.rs`): the hash body excludes **both** `id` and
`sig`. `id = "<prefix>:" + hex(sha256(JCS(body ∖ {id, sig})))`. Suggest the
spec say "excluding `id` and `sig`" explicitly.

## SI-2 — what exactly is signed (§0, §8) — interpreted

The spec defines `sig: {key_id, alg, value}` but never states the message the
signature covers.

**Interpretation** (`canon.rs`): Ed25519 over the same canonical body bytes
whose hash is the id (body ∖ {id, sig}). Verify = recanonicalize, check id,
verify sig. This makes id-check and sig-check attest the same bytes.

## SI-3 — who signs a Manifest (§3, §8.1) — interpreted

§8.1 assigns: user root → Principals, Intents; broker key → events,
capabilities, tool countersignatures; agent instance keys → attestations.
Manifests carry `sig` but no signer is assigned. (Same gap for Channel — §2.1
shows `sig` on a channel registration.)

**Interpretation** (`kernel.rs`): the fabric component that constructs the
manifest at the step boundary signs it — in Stage 1 the kernel/coordinator
key, which in Stage 2 becomes the broker daemon's key (it is the same trusted
component that signs events). Channels likewise fabric-signed at registration.

## SI-4 — first event of a span: `prev` (§6) — interpreted

The Event example shows `prev: "evt:…"` with no null case; nothing says what
the chain head looks like, or whether `seq` starts at 0 or 1.

**Interpretation** (`trace.rs`): head event has `prev: null`, `seq: 0`.
Chain verification requires seq contiguity from 0 and prev-linkage thereafter.

## SI-5 — `manifest` on pre-manifest events (§6) — interpreted

Intents are captured *before* the first manifest (`captured_before` is a
substrate offset), so intent-capture and channel/principal registration events
exist with no manifest to reference, but Event shows `manifest: "man:…"` as if
always present.

**Interpretation** (`trace.rs`): `manifest` is nullable; null for
substrate-level events that precede or stand outside any manifest (principal
/channel registration, intent capture, drift observed between runs).

## SI-6 — sqlite state root: what bytes get hashed (§3 state.roots) — interpreted

`{"store": "db:memory", "kind": "sqlite", "root": "sha256:…"}` — hash of what?
The raw db file changes byte-identity under WAL checkpointing and vacuum even
when logically unchanged; a logical serialization (sorted dump) is stable but
slower and loses page-level fidelity.

**Interpretation** (`snapshot.rs`): checkpoint WAL (TRUNCATE), then hash the
main db file bytes. Byte-identity is the drift detector: an untouched file
hashes identically; any touch (even logically-neutral) is at minimum honest
about "something wrote here". Cost: a logically-identical-but-rewritten db
reads as drift — acceptable for Stage 1, revisit if it produces noise.
Related: F2 — resolution proposed in `docs/adr/0002-f2-state-root-addressing.md`
(page-aligned sqlite chunking under CID roots subsumes this issue's cost
concern; byte-image behavior remains the Stage 1–2 interim).

## SI-7 — manifests without a capability (§3 `authority`) — open

Manifest schema shows `authority: {capability: "cap:…"}` unconditionally, and
M1/M2 bind roots↔capability. Stage 1 has no broker and no capabilities, yet
the kernel must mint manifests now; also future purely-local runs (no external
authority at all) arguably need no capability.

**Interpretation for Stage 1** (`kernel.rs`): `authority` is omitted; M1/M2
tests are written against the pure checking functions and marked as activating
in milestone 2. **Open question for the spec:** is a capability-less manifest
legal long-term (local-only runs), or must the broker mint a no-external-reach
capability for every run so M1/M2 are unconditional?

## SI-8 — drift `between` field type (§6 drift body) — interpreted

`between` appears in the drift body with no type or example.

**Interpretation** (`kernel.rs`): `between: [offset_lo, offset_hi]` — the
global substrate offsets bracketing the window in which the unobserved change
occurred (last event that attested the expected root, first observation of the
divergent root). Substrate offsets rather than event ids because the endpoints
may belong to different spans.

## SI-9 — payload AEAD algorithm unspecified (§1, §8.2) — interpreted

Per-payload DEKs and KEK wrapping are mandated; no cipher is named anywhere.

**Interpretation** (`payload.rs`, `keys.rs`): AES-256-GCM for both payload
encryption and DEK wrapping, recorded in the dek row (`alg` column) so the
choice is per-payload upgradable. Spec should name (or version) the suite —
interop depends on it.

## SI-11 — no event kind for object registration or intent capture (§6) — RESOLVED (author, 2026-07-08)

**Resolution: Option A.** `register` and `intent` become §6 kinds; a
fabric-lifetime span carries events outside any manifest (also settles SI-5);
objects are live only once their register event is on the chain (object rows
are materialized views; row without event fails closed). This also supplies
the substrate countersign that resolves SI-10's proof gap. Kernel already
implements this shape; spec text amendment pending (A15 candidate).

The §6 `kind` enum is closed (14 values) and has no kind for: principal
registration, channel registration (§2.1 — a signed, "registered object"),
tool registration (§4 — broker countersigns, surely traced), or intent
capture (§3.1 — which *must* hit the substrate to make `captured_before`
mean anything, see SI-10). C1 requires these authority acts to be recorded;
there is no vocabulary to record them with.

**Interpretation** (`trace.rs`): two added kinds — `register` (body:
`{object, object_kind}`) and `intent` (body: `{intent, channel,
auth_strength, captured_before}`). Spec should either add these to §6 or
state where registrations are recorded if not as events.

**Supporting observation:** §6 already contains `amendment`, and §3.1 defines
an amendment as an intent with `amends != null`, judged as one chain — so the
spec currently records the same act (a channel-stamped instruction) as an
event when it arrives mid-run but not when it founds the run. Adding `intent`
(and `register`) removes the asymmetry; it also supplies the substrate
countersign that makes `captured_before` a proof (SI-10) and gives C1's
ratification-velocity analysis one ordered stream to read. If events are
added, two companion rules are needed: (a) a blessed fabric-lifetime span for
events outside any manifest (cf. SI-5), and (b) objects are live only once
their register event is on the chain — object rows are materialized views;
a row without an event fails closed.

## SI-12 — glob "narrower" is not mechanically decidable as written (§5.2) — interpreted

§5.2 requires attenuation verification to be "mechanical subset-checking"
and lists "globs narrower" as one dimension — but glob-language subset is
not simple to decide in general. **Interpretation** (`capability.rs::glob_covers`):
a conservative provable-cases-only rule (identical globs; parent `**`;
parent `prefix/**` with a literal prefix covering the child). Everything
else is rejected as not-a-subset even when a human can see it is one (e.g.
parent `inbox/*.md`, child `inbox/a.md`). Fail-closed and spec-compatible,
but the spec should either bless a restricted glob dialect with decidable
subset or state that conservative approximation is intended.

## SI-13 — no event kind for escalation *resolution* (§6, A9) — interpreted

C1 lists "escalation approval" as a human authority act that must be
recorded with channel + auth_strength; §6 has `escalation` (the request)
but no kind for the decision. **Interpretation** (`trace.rs`, `broker.rs`):
added kind `approval`, body `{escalation, resolution: approved|denied,
uses, channel, auth_strength}`. Same family as SI-11; fold into the same
spec amendment.

## SI-14 — how are mechanical denials recorded? (§6) — interpreted

A denied call executes nothing, so it is not a `tool_call`; §6's `verdict`
kind reads as the model judge's (brief §5.4 layer 3). **Interpretation**
(`broker.rs::deny`): broker denials are `verdict` events with
`source: "broker"`, carrying the failed dimensions and full check record.
If the spec would rather reserve `verdict` for the judge, it should name a
`denial` kind.

## SI-15 — auth_strength has no total order (§2.1, §5.1) — interpreted

`approval.min_auth` requires comparing channel strengths, but §2.1 only
lists them. C4 groups `local_session` and `passkey` as the strong tier.
**Interpretation** (`capability.rs::auth_rank`): unverified(0) <
platform_oauth(1) < passkey(2) = local_session(2). Unrankable strengths
fail closed. Spec should publish the order — it is consent-surface
semantics, not an implementation detail.

## SI-16 — M1's "grants any access to" needs a derivation rule (§3 M1, §4) — interpreted

M1 says roots must cover every store the capability grants access to, but
nothing in §4/§5 says how to compute a capability's store-reach.
**Interpretation** (`tools.rs`, `broker.rs::mint`): tool registrations
declare `store` per action (plus `class` for budget metering and
`path_args` for paths.write extraction — same trusted-mechanical tier as
the §4 example's `writes`); mint-time M1 = union of stores of allowlisted
tools ⊆ manifest roots. Spec should adopt per-action store/class/path
bindings into §4, or name its own derivation.

## SI-17 — `move` vs `rename` are listed but never defined (§5.3 A13) — RESOLVED (author, 2026-07-09)

**Resolution:** the proposed amendment below is ratified as written —
parent-directory definitions; `rename < move` ordering (allow-move implies
allow-rename); exact-hash authority layer with similarity permitted only
as legibility-layer annotation; from/to direction constraints deferred to
the Stage 3 rule grammar. Kernel already conforms; the ordering function
lands with StandingRules. (A17 amendment candidate.)

A13's operation classes are `add | modify | delete | move | rename`, with
no definition distinguishing move from rename. This is authority
vocabulary, not display: promotion rules (StandingRules, A13) match on
these classes, so the definition is security semantics.

**Interpretation** (`promote.rs`): same content hash, same parent
directory, new name = `rename`; different parent directory = `move`.
Pairing is exact-hash and only when unambiguous (hash unique on both
sides) — never mislabels; leaves duplicate-content shuffles and
renamed-AND-edited files as add+delete (which park — safe, with friction).

**Proposed amendment (discussed with author 2026-07-09):**
1. Adopt the parent-directory definitions, AND order the classes:
   `rename < move` in required permissiveness — a rule allowing `move`
   allows `rename`, never the reverse (the reversibility_rank pattern).
   Avoids the disjoint-label footgun (ratified "moves into MOCs" parking
   an in-place rename in MOCs).
2. Authority layer stays exact-hash forever: similarity-based rename
   detection is GAMEABLE (an agent can present delete-X-plant-altered-
   content as "rename+edit" inside a rename-allowing rule) and has no
   zero-authorship threshold. Similarity may only ever annotate the
   legibility layer (diff cards: "looks like rename+edit"), never produce
   a class rules can match.
3. Stage 3 rule grammar should add direction constraints —
   `move: {from ⊆ globs, to ⊆ globs}` — which do the real work for
   filing workflows; from/to are already recorded in op JSON.

## SI-19 — `link` appears in §5.3's example rule but not in A13's enum — RESOLVED (author, 2026-07-09)

§5.3: "auto-merge iff paths ⊆ {/inbox, /MOCs}, ops ⊆ {add, modify,
**link**}, no deletes" — but A13 defines `add | modify | delete | move |
rename`. Content-aware classes were judged complexity-over-benefit for
now (the ratchet's coarse equivalent — path-scoped `modify` rules — is
adequate for the solo-operator design center).

**Resolution: remove `link` from the §5.3 example; add a
refinement-only extensibility clause to A13** so content-aware classes
can return without redesign (A18 amendment candidate):
1. Class vocabulary is fail-closed: rules match explicit sets; classes
   unknown to a rule park at the gate. Future classes can never widen
   pre-existing grants.
2. Extension is by refinement only: `<root>.<refinement>` of the five
   structural roots; a refinement ADDS a predicate to the structural
   classification, never replaces it — a gamed/buggy predicate degrades
   to the parent class, bounding blast radius.
3. Rules allowing a root allow its refinements (SI-17 ordering pattern);
   never the reverse.
4. Content-aware refinements require registered, versioned classifiers;
   rules pin the classifier version they were ratified under (A1
   domain-scoped pinning, applied to classifiers).
The kernel implements structural roots only; nothing changes in code.

## SI-18 — three-way merge is undefined for opaque stores (§5.3 A11) — interpreted

A11 specifies three-way merge with per-path conflict semantics, which only
exists for file-tree stores. The manifest's other Tier-1 store — the agent
memory sqlite — has no sub-file merge. **Interpretation** (`broker.rs`
compute_merge): opaque stores merge whole-store: branch-only change
installs the branch image (op class `modify`, auto-promotable); trunk-only
change stands; both-changed is a single conflict card, trunk wins, branch
image preserved in CAS. The spec should state per-store-kind merge
strategies — this also touches the A3 memory-taint open problem, since
whole-store memory promotion is exactly where cross-run taint lands in
trunk.

## SI-10 — `captured_before` freshness is unenforceable as specified (§3.1) — open

`captured_before` proves the intent was signed before a given substrate
offset only if the offset is bound at signing time by something the substrate
attests. As specified it is a self-declared field: nothing stops an intent
minted later from claiming an earlier offset. Fine for Stage 1 (single trusted
kernel process writes both), but the "pre-contamination proof" language (§3.1)
overclaims for any deployment where the intent signer and the substrate are
not the same trust domain. Likely fix: intent-capture event in the substrate
countersigns the intent id at its actual offset (the event *is* the proof;
the field is a convenience). Kernel already emits such an event; flagging so
the spec language gets tightened.
