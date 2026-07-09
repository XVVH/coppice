# Spec issues — ambiguities the code forced

> **v0.4 (2026-07-09): every issue below (SI-1…SI-19) is RESOLVED** —
> ratified individually (SI-11, SI-17, SI-19, F2/SI-6) or via the
> adjudicated A19 conformance sweep, and integrated into
> `asf-schema-spec.md` v0.4 (changelog A15–A19). This file is preserved
> as the amendment provenance record; per-entry statuses below are
> historical. New issues found under v0.4 start at **SI-20**.

Tracked per the handoff: where the spec is ambiguous or contradicts itself,
we record the question, the interpretation the kernel implements, and why —
we do not silently pick. Each issue cites spec § and the implementing file.
Settled decisions (F1, F4, fail-open) are not re-litigated here.

Status legend: **open** = needs a spec amendment or an explicit "fine as
interpreted" from the author; **interpreted** = kernel picked a reading and
tests encode it; flipping the reading is cheap.

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
