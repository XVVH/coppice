# Substrate theory — ASF's theoretical basis and the operating-system endgame

**Status: THEORY NOTE — exploratory, non-normative. v2.** Nothing here is
spec, roadmap, or a work item; nothing here authorizes implementation.
Anything actionable that emerges from this document enters the system the
ordinary way — an SI, W, or P filing judged on its own merits — never by
citation to this note. Provenance: drafted 2026-07-15 in the side-session
thought experiment split off from the SI-32/SI-40 ratification cycle;
**amended to v2 the same day** after a five-stance adversarial panel
(GPT 5.6 Sol, xhigh reasoning; one stance per discipline) and a
fresh-context synthesis with steelman rulings — all outputs preserved in
`docs/substrate-theory-analysis/`. Per the house ADR norm, refuted text is
struck but preserved inline with its refutation; the panel's convergent
kills were C8 (the staging recipe — refuted independently from filesystem
semantics and from A27.1 conformance) and C11 (the agent-OS category —
refuted by three stances). Claims are numbered **C1–C13** so critique can
cite them.

Grounding documents: `docs/agent-state-fabric-brief.md` (the product thesis
this note must not contradict), `docs/asf-schema-spec.md` (A27 §5.3 — the
storage adversary model; §5.4–§5.5 — authority as event-derived views; §7 —
rules and trust records), `docs/spec-issues.md` (SI-32, SI-40, SI-23),
`docs/posture-assumptions.md` (P7, P29, the gate vocabulary).

---

## 1. What problem this architecture is actually an instance of

**C1 — Prompt injection is a confused-deputy-SHAPED exploit chain;
capability discipline bounds it but does not cure it.** *(v2: the v1
identity claim — struck: "the attack is unchanged" — was refuted by the
panel's historian: Hardy's deputy held authority from two sources and
misapplied its own authority to a caller-designated name, a
designation/authority confusion that capability discipline fully cures. An
injected agent can select action and target wholly inside authority
legitimately granted by the user — the defect is instruction provenance,
which authority bounds contain but cannot cure; CaMeL-style defenses need
both control-flow integrity and capabilities precisely because they answer
different halves.)* The refined claim: prompt injection produces a
confused-deputy-shaped chain — a privileged intermediary steered by its
input — and the object-capability tradition (Hardy 1988; KeyKOS, EROS, E,
Miller's *Robust Composition*; credential form: macaroons, the direct
ancestor of the §5 caveat grammar) supplies the blast-radius answer the
agent workload finally demands. Standing label (panel S1-B1): ASF's
capability is a broker-evaluated attenuated credential in the macaroon
lineage, **not** an ocap reference — designation and authority arrive
separately — so no ocap composition theorem may be invoked without a
property-by-property inheritance table, which is owed and does not yet
exist.

**C2 — ASF's contribution is the joined loop, not any single leg.** *(v2:
struck: "ocap has no undo" and "something the ocap tradition never had to
face" — refuted: KeyKOS/EROS shipped system-wide consistent checkpoints;
sagas paired committed steps with compensations; Ken composed recovery
across components; KeyKOS factories, EROS constructors, E's auditors, and
Nexus logical attestation are all behavior-conditioned authority.)* The
defensible claim: the novelty is the **conjunction** — recoverability
evidence, human ratification, and domain-scoped behavior-version
invalidation joined into one authority-accrual loop. Two honesty labels
bind it: (a) the coupling is today the brief's design intent ("reversal
earns authority"), **not yet the spec** — §7's ratchet consumes k ≥ 3
approved examples and raw counters, and nothing gates widening on a
recoverability-fidelity measure (the known-open fidelity-grades problem,
sharpened: until the ratchet consumes a declared fidelity grade, cheap
compensations can launder into broad grants — F7); (b) "consequence" is not
yet one theoretical quantity — byte-exact restore, saga compensation, and
post-hoc apology sit on no common scale.

## 2. The substrate anchoring

**C3 — The substrate gap is a missing manifest-instance identity, not a
missing fine-grained subject.** *(v2: struck: "this single fact [the uid]
generates most of the fabric's hard security problems" — refuted as a
causal claim: Linux offers finer-than-uid subjects (LSM labels, Landlock
domains, keyrings); the listed problems arise because the current posture
configures no boundary at all (P7), and the trust-root and actuation
problems are not uid-shaped.)* The refined claim, with the portability
caveat: what no substrate offers — and on the portable floor including
Darwin, where the Linux LSM arsenal does not exist, the uid really is the
only durable principal — is **one stable, authenticated,
application-addressable manifest-instance identity consumed uniformly by
mediation, broker peer-authentication, and audit**. That absence is why W-4
must *emulate* the boundary rather than merely configure it.

**C4 — The dangerous seam is an object-model mismatch, and the compiler is
where fail-open lives.** *(v2: struck: the additive/subtractive binary as
an essence claim ("subtractive fails open by construction; additive fails
closed by construction") — refuted: Smack/SELinux are explicit-allow,
seccomp can default-kill, and capability systems fail open via over-broad
grants. Preserved counter-note: the deployed container stack's
forgot-to-carve CVE pattern is real, and Landlock's handled-access design
defaults open for unhandled categories — the fail-open-on-unknown-dimension
shape ASF forbids.)* The refined claim: caveats speak recipients, budgets,
reversibility, and behavior versions; kernel controls speak tasks, inodes,
sockets, and syscalls. The compiler between those vocabularies is where
fail-open-by-omission lives, and A27.4's standing sentence is the honest
label on that adapter.

**C5 — Manifest-as-principal is the missing unifying binding — a diagnosis,
not a remedy.** *(v2: struck: "A27's T2 tier dissolves by construction" —
refuted: two manifest principals authorized to write one namespace still
race; the race closes by exclusion topology, which is A27's existing,
ratified answer — principals make topology kernel-expressible, never
unnecessary. Also struck: "C2's approval surface is enforced the way memory
protection is" — refuted via open SI-23: a self-satisfiable approval
arrives through a legitimate actuation grant, and principal separation
cannot distinguish granted synthetic input from a human; this note may not
silently resolve an open SI. The v1 "20% real kernel work" figure is
withdrawn: no security property has been named that only a new kernel
principal provides — a trusted launcher over existing mechanisms suffices
for local enforcement.)* The surviving claim: a signed delegation record
bound to a locally unforgeable workload identity, consumed uniformly by
mediation, broker authentication, and audit, is the abstraction W-4
emulates and a future substrate could make native. Two scope conditions
bind it: (a) the unit of isolation must be named — manifest-per-microVM
leaves a kernel principal jobless, while multi-manifest guests forfeit the
VM boundary between delegates; (b) cross-host identity stays **above** the
kernel at policy admission — attestation proves provenance, never freshness
or uniqueness, and cloned homes can each spend a one-use authority absent
A23's external head plus a writer lease (this absorbs v1's F4, demoted from
falsifier to scope condition).

## 3. Publication theory (what the SI-40 exploration generalized)

**C6 — At the write boundary, outcomes partition three ways; real systems
compose them.** *(v2: struck: "the solution space is exactly three
families" and the implication that mature systems chose family 3
exclusively — refuted: git composes expected-old validation, locking, and
atomic rename; MVCC composes retention with locks or validation; WALs are
orthogonal crash recovery.)* The refined claim: a concurrent write is
either **excluded**, **detected and handled**, or **redirected** so it
lands somewhere well-defined — design axes, not exclusive choices. The
load-bearing content is D32-4: exclusion is foreclosed for shared-state
stores, so the remaining design space is detection, redirection, or their
composition.

**C7 — Retention, not snapshotting, is the load-bearing mechanism — as a
profile, not a construction.** The core survives the panel intact: a
snapshot taken at any fixed point cannot preserve an edit that postdates it,
and the SI-40 edit postdates the prepare check by definition (this
refutation now has three independent derivations: this note's first pass,
the SI-40 external review, and the panel's storage stance). What works is
retention of the outgoing generation at swap time, followed by diff, CAS
ingestion, and attribution. *(v2: struck: "Nothing falls between, because
there is no between" and "'attributed, never lost' achieved by
construction" — refuted by the note's own residual plus two more boundary
crossings: an open descriptor writes into retained state post-swap; shared
inodes make an edit unattributable to a side of the swap; NFS clients under
default caching resolve the old target for tens of seconds.)* The refined
guarantee: **under an aliasing-free staging (independent inodes) on a
locally coherent filesystem**, the atomic swap makes every
pathname-addressed concurrent edit's fate well-defined — before the swap it
lands in the retained generation (kept, diffed, ingested, attributed);
after, it is ordinary drift on live state. The guarantee is a profile;
descriptors, shared inodes, and stale remote caches each cross it and each
must carry its own labeled residual. Publication guarantees generally are
**properties with per-substrate profiles, never primitives**; any concrete
staging recipe or registration protocol is SI-40/W-lane work, and none
appears in this note.

**Non-selection notice (v2, added after the panel's fence audit):** this
note does not select SI-40's remedy. SI-40's leading candidate is per-entry
capture-or-refuse; retention-at-swap is the filed deployment-floor
alternative, evaluated against it when a trigger fires. This section is the
theory of why the retention family is coherent — not a verdict between the
arms, which belongs to the SI's own ratification.

**C8 — STRUCK (v2).** The v1 staging recipe — unchanged entries hardlinked
from live, changed entries reflinked from CAS, exchange-rename /
symlink-flip / journaled two-rename ladder, O(changed) reconciliation,
"NFS-safe," "nearly free" — is struck in its entirety. *(Refutations
preserved: (1) hardlinking unchanged entries from the mutable live tree
shares writable inodes across generations — an in-place edit mutates both
trees, cannot be attributed to a side of the swap, and makes correctness a
function of editor save style; worse, it is verbatim the mutation A27.1
forbids — commit installing bytes sourced from mutable storage after
prepare-time verification — so the recipe contradicted a normative clause
ratified the same week, found independently by the storage stance and the
editor stance. (2) O(changed) reconciliation presumes a complete mutation
oracle; on the portable floor, proving a retained POSIX tree unchanged at
unknown paths costs an O(tree) walk or a change journal — family-2
detection under another name — and the hardlink farm itself costs O(N) to
build. (3) Root exchange is hostile to observers: inotify watches objects,
not future occupants of a pathname; FSEvents requires a full-tree rescan on
a moved root; a conforming watcher or sync daemon pays O(N) per
publication. (4) "NFS-safe" conflated server atomicity with client
coherence — default attribute caching lets clients resolve the old target
for up to ~60s. (5) The journaled two-rename "universal floor" is not a
profile of the stated atomic property but a weaker property that must be
labeled as such.)* The one surviving sentence is relocated into C7 above.
Consequence worth stating plainly: on raw local POSIX with watchers and
sync daemons present, the panel's crossover analysis shows per-entry
detection — SI-40's filed leading candidate — can beat retention-at-swap at
surprisingly small change ratios; the side thread's preference for the swap
arm was over-fitted to CoW-native substrates.

**C9 — Substrates grade on four independent axes, not one ladder.** *(v2:
the monotonic ladder is refined away: retention cost, namespace atomicity,
observer coherence, and mediation/attribution are independent — a CoW
filesystem buys retention, not watcher retargeting or multi-store
atomicity.)* The refined claim: publication capability is a **profile
matrix** discovered per store; hosted infrastructure can provision rungs on
some axes (real CoW datasets — zero-friction constrains the user's laptop,
not the product's cloud) while leaving others at the portable floor. Kept
unamended, untouched by any reviewer: the double edge — hosted CoW
infrastructure also makes whole-home rollback a one-command accident, the
exact coherent-suffix-regression case D32-2 assigns to A23 layer 2, so the
same substrate that eases publication sharpens the case that the external
anchor is a hard G-PRODUCTION requirement.

## 4. The endgame, demoted to its defensible core

**C10 — The trust boundary is technically replaceable; its constituency
moved rather than vanished.** *(v2: struck: "no POSIX loyalty at the trust
boundary" as an absence-of-constituency claim — refuted: harness
conventions (bash-first tooling, ambient credentials, network assumptions)
define what any boundary must transparently reproduce, and the vendor
sandbox work confirms it; also struck: the KeyKOS→EROS→Capsicum→seL4→
Fuchsia arrow-chain and "lost to compatibility economics" as settled
history — a typological list, not a genealogy; compatibility cost is one
tested factor.)* The refined claim: agent trust boundaries are already
being rebuilt without POSIX semantics — the boundary is *technically*
replaceable, confirmed by the refuting stance's own exhibits — but the
constituency now lives in harness conventions and in the
boundary-builders themselves, whose incentives oppose portable trust. For
this workload, the compatibility cost that defeated general-purpose
capability systems is an adapter-engineering cost, not a rewrite-the-world
cost; whether the neutral adapter or the vertically integrated one wins is
F1′'s economic question, not a semantic one. The near-term composition is a
manifest-attested ASF microVM appliance built from existing parts; whether
W-4's emulation interface should anticipate a future kernel interface is a
question to file through the ordinary channel, not a discipline this note
may impose.

**C11 — STRUCK (v2).** The v1 "agent-runtime OS" claim is struck. *(Refuted
by three independent stances: every named component is Firecracker/Kata
plus an ASF control plane, with no invariant a conventional microVM runtime
and policy engine cannot supply; no buyer exists for the category as such —
the purchasable object is a control plane or appliance; and the claim
materially expanded roadmap W-4's specified scope while its "design
discipline that follows today" sentence was a present-tense work item
inside a note whose §5 forbids them.)* What survives is the appliance
one-liner now housed under C10.

**C12 — Formats are option value — a neutrality-and-distribution strategy,
never a value-capture strategy.** *(v2: struck: the implication that
kernel-enforceable schema design positions the formats to become the
standard, and the imperative "design the schemas so that a kernel could
enforce them" — refuted/fenced: OCI proves formats can outlive their
assemblers, not that authors capture value; Docker donated an
already-dominant format — distribution, not design quality, made it
canonical — and later exited the business; the imperative was a schema
design criterion this note has no authority to issue.)* The refined claim:
substrate-free semantics with posture-bound implementations remain the
house discipline for the fabric's own reasons; the option this creates —
that the formats could survive into whoever's runtime wins — is real but
modest, and value capture requires its own claim and falsifier, which this
note deliberately does not supply. Honesty label carried from the panel:
the trust ledger is a weak *network* asset as currently conceived (private,
user-scoped, deliberately exportable — another customer's history does not
improve mine); that observation indicts nothing here but presses on the
brief's §9 moat story.

**C13 — Behavior is the unit of earned trust, and judgment is the
compilation boundary.** *(v2: added — the panel's completeness stance found
the note enumerated a "triple" in which behavior did not appear, while
claiming to state ASF's theoretical basis.)* Mechanical capabilities decide
only stable, auditable predicates; a capability-less judge handles residual
ambiguity; human ratification is the sole transition from judgment to
standing authority; and evidence remains valid only for the behavior
version that produced it. The loop succeeds only if repeated judgment
demonstrably shrinks into mechanical rules without losing behavior
provenance — if it does not, the behavior lineage and judge are ceremonial,
and this note is a storage-and-capability theory mislabeled as a theory of
trust (F8).

## 5. What this theory does NOT license

Stated to keep the note honest and the critique aimed — and rewritten in v2
after the panel showed the v1 fence was breached by its own body text
(imperative "design disciplines," a staging recipe, and a silent
pre-selection of SI-40's arm — all removed above):

- It does not reprioritize anything. The current queue (dogfooding, the
  ratchet, W-15a) is where the product lives; this note is a horizon, not a
  backlog.
- It does not weaken the open-world bet. C9's hosted-substrate observations
  describe what the product may *provision*; they never justify requiring
  anything from a user's laptop.
- It does not claim the fabric needs an OS to be valuable, and after the
  panel it no longer claims an OS category at all.
- It selects no SI-40 arm, imposes no schema criterion, and issues no
  design discipline. The single actionable item the review cycle surfaced —
  instrumenting F2′ before histories accrue — enters through an ordinary
  W-1/W-3 filing or not at all.

## 6. Falsifiers, re-ranked by the panel (attack here first)

- **F2′ (vs C2 — rank 1).** Currently *untestable*: the dogfood measures
  denial false-positives while standing grants cannot yet widen, so "users
  did not widen" is mechanically predetermined. Operational form (panel
  S4): pre-register recurring workflow families and a delegation-frontier
  vector (scope, duration, unattended runtime, auto-promotion, approvals
  per success); record the operator's maximum grant before histories
  accrue; accumulate ≥3 clean runs plus one forced restore per family;
  offer least-general standing grants; collect ≥24 accept/narrow/reject
  decisions at fixed behavior version. Disconfirmation: the frontier widens
  on no dimension and rejections cite risks recovery does not address.
  Falsifies for the design-center operator; market claims need a staged
  rollout.
- **F6′ (vs C1/C2/C13 — rank 2).** Two pre-registered arms: an *ablation*
  (assume the worker obeys every injection; disable judge and human;
  measure deterministic prevention across attacker-designated targets
  outside conveyed authority, targets inside a coarse grant but against
  intent, and prohibited flows — threshold chosen before results) and a
  *shrinkage* metric (over a fixed workflow cohort, the fraction of
  consequential decisions still requiring judge/human after ratification
  opportunities must fall).
- **F8 (vs C13 — rank 3, new).** If behavior-version changes do not predict
  materially different outcomes, or repeated judge work does not shrink
  through ratification, the behavior lineage and judge are ceremonial.
- **F1′ (vs C10 — rank 4).** Economic form: if representative
  coding/research/SaaS workflows cannot run through the broker without
  ambient credentials, unclassified shell effects, or routine bypass —
  while integrated vendor sandboxes deliver acceptable autonomy without
  manifest semantics — the ASF boundary has lost economically even though
  technically implementable.
- **F7 (vs C2 — rank 5, new).** If the ratchet widens standing grants on
  compensation evidence a declared fidelity grade would have excluded, the
  coupling is laundering, not learning — regardless of whether users widen.
- **F3′ (vs C7/C9 — rank 6, burden reversed).** "Cheap retention" is
  *unsupported* until measured: full scans triggered, metadata ops, bytes
  rehashed, conflict copies, retained unique bytes, stale-client duration —
  across inotify, FSEvents, NFS profiles, Syncthing, Dropbox. The panel's
  crossover model says watcher rescans dominate at small change ratios.
- **F5′ (vs C12 — rank 7, re-dated).** The verticals already shipped and
  embrace MCP/A2A while keeping identity, policy, and history proprietary.
  Replacement trigger: if by 2027-06-30 fewer than two independent runtimes
  natively emit ASF records, fewer than two independent gates authorize
  from them, or no external relying party accepts portable trust across
  vendor boundaries, the format bet has failed; export adapters and nominal
  schema support do not count.
- **F4 — demoted (v2).** Not a falsifier; recast as C5's scope condition
  (b): admission-time freshness, uniqueness, and canonical-fork selection —
  including the cloned-home double-spend — live above the kernel, at A23's
  anchor plus a writer lease.

## 7. Reading lineage

Hardy, "The Confused Deputy" (1988) · Miller, *Robust Composition* (2006)
and the E language · KeyKOS / EROS (checkpointing capability systems) ·
Watson et al., Capsicum (USENIX Security 2010) · seL4 · Fuchsia/Zircon ·
Birgisson et al., "Macaroons" (NDSS 2014) · Garcia-Molina & Salem, "Sagas"
(1987) · Yoo et al., Ken/composable reliability (USENIX ATC 2012) · Sirer
et al., logical attestation (Nexus) · Debenedetti et al., CaMeL —
"Defeating Prompt Injections by Design" (2025) · Landlock (Linux 5.13) ·
IMA/EVM, dm-verity, fs-verity, composefs · NixOS / ostree · Talos,
Bottlerocket · `renameat2(2)` / `renamex_np(2)` · Firecracker · OCI (the
format-outlives-assembler precedent, read in v2 as a warning as much as a
precedent).
