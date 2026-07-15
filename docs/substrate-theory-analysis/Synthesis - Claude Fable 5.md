# Synthesis: five-stance adversarial review of `docs/substrate-theory.md`

*Fresh-context synthesis (Claude Fable 5), 2026-07-15. Method: cross-matrix →
steelman rulings on convergent refutations → amendment list → survival
report. Reviewer citations verified at source before ruling: SI-40's
candidate ordering, A27.1's staged-bytes rule, §7's StandingRule/TrustRecord,
SI-23's self-satisfiable-approval finding, and A27's T2 determination. Every
reviewer citation checked was accurate.*

---

## 1. Cross-matrix

| Claim | S1 (ocap historian) | S2 (kernel/security) | S3 (storage) | S4 (strategist) | S5 (editor) | Classification of the REFUTE/REFINE content |
|---|---|---|---|---|---|---|
| C1 | **REFUTE** (identity claim false; analogy useful) | — | — | — | MERGE (setup only; "attack is unchanged" does no work) | **Stance-dependent** (S1's lens) but decisive on merits; S5 semi-converges on demotion |
| C2 | **REFINE** ("ocap has no undo" historically indefensible; narrow to the joined loop) + B2 (consequence bound not one quantity) | — | — | — | KEEP, but must separate brief aspiration from §7 spec (grounding conflict #6) | **Convergent demotion on two independent axes**: S1 attacks the history, S5 attacks the internal grounding |
| C3 | — | **REFUTE** (Linux has finer-than-uid subjects; real gap = no manifest-instance identity; problems are P7's unconfigured boundary) | — | — | MERGE ("single fact generates most problems" is monocausal decoration) | **Convergent** on the monocausal causal claim |
| C4 | — | **REFUTE** (subtractive/additive not an ecosystem property; surviving insight = object-model mismatch) | — | — | MERGE (binary and "every enforcement claim" overstate A27's T2-scoped dependency) | **Convergent** on the binary; the adapter-danger conclusion not contested |
| C5 | — | **REFUTE** (T2 does not dissolve; launcher over existing mechanisms suffices; B2 scheduling unit; B3 SI-23) | — | — | KEEP as load-bearing, but "does not dissolve T2 or solve approval-surface reachability" (conflict #4: silently resolves SI-23) | **Convergent** on both downstream conclusions; S2's B2 (scheduling unit) and B4 (clone/consumption) are **singletons, live** |
| C6 | — | — | **REFUTE as worded** (not exhaustive/disjoint; mature systems compose families) | — | MERGE ("exactly three" undefended; detection persists inside indirection) | **Convergent** |
| C7 | — | — | **REFUTE** (hardlink aliasing blurs the temporal cut; correctness depends on editor save style) | — | KEEP core, but residual refutes "nothing falls between" (conflict #2); note pre-selects SI-40's arm | **Convergent** on the by-construction absolutism; S5's SI-40 fence breach is a **decisive singleton** (§2.7) |
| C8 | — | — | **REFUTE as written** (hardlinks; O(changed) needs a mutation oracle; watcher O(N); "NFS-safe" false) | — | MERGE (recipe is unratified protocol work; conflict #1: violates A27.1; conflict #3: floor is not a profile of the property) | **Convergent and decisive** — two stances kill the recipe from independent directions (fs semantics; spec conformance) |
| C9 | — | — | **REFINE** (four independent axes, not one ladder) | — | MERGE (examples, not a claim) | **Convergent** refinement |
| C10 | B3: genealogy typological, monocausal (medium) | — | — | **REFUTE** (boundary has a harness-convention constituency; adoption economics) | KEEP (necessary premise, falsifiable via F1) | **Convergent demotion** on two axes (history: S1; economics: S4); technical core uncontested |
| C11 | — | **REFINE** (Firecracker/Kata + control plane; name the unique invariant or call it an appliance) | — | **REFINE** (no buyer; defensible only as reference appliance / W-4 backend) | **DELETE** (roadmap disguised as limit case; expands W-4 while denying it; fence breach) | **Convergent ×3** — the strongest convergence in the set |
| C12 | — | — | — | **REFUTE strategic inference** (OCI proves survival, not capture; distribution, not design, made the format canonical) | KEEP (as option value, not prediction; the "corollary" is imperative — fence breach) | **Stance-dependent** but the inference-refutation is decisive; S5 converges on the fence problem |
| F1 | — | — | — | **REFINE** (tests semantic impossibility; economic refusal suffices; replacement supplied) | Rank 3, real | **Convergent** on operationalization |
| F2 | — | — | — | **Currently untestable** (dogfood measures denial FPs; widening mechanically impossible pre-ratchet; 90-day protocol supplied) | Rank 1, real but under-instrumented | **Convergent**: top falsifier, not yet operational |
| F3 | — | — | **Valid but non-operational** (crossover model; "nearly free" *currently unsupported*, burden reversed) | — | Rank 4, real but local, needs thresholds | **Convergent** on operationalization; S3's burden-reversal is the sharper form |
| F4 | — | **REFINE** (not a falsifier; problem stays *above* the kernel at policy admission; B4: cloning breaks consumption, not just identity) | — | — | Rank 6, not a falsifier; move to open weaknesses | **Convergent demotion**; S2's B4 upgrade is a **singleton, live** |
| F5 | — | — | — | **REFUTE as stale** (vertical substrates already shipped; vendors embrace MCP/A2A while keeping authority proprietary; dated replacement supplied) | Rank 5, performative | **Convergent demotion**; S4's mechanism (open protocol, proprietary authority) is the decisive part |
| F6 | **REFINE** (right threat, unfalsifiable as written; pre-registered ablation supplied) | — | — | — | Rank 2, potentially devastating, not operational; shrinkage metric supplied | **Convergent** on operationalization |

**Part B singletons not lens-dismissible (live):** S1-B1 (three meanings of
"capability" laundered — ocap reference vs macaroon credential vs ASF broker
grant); S1-B2 (consequence bound is not one quantity — fidelity laundering);
S2-B2 (C5/C11 jointly never name the unit of isolation); S2-B4 (cloned homes
double-spend one-use authority — attestation proves provenance, not freshness
or uniqueness); S3 watcher finding (root exchange makes conforming observers
pay O(N) per publication); S5's §5-fence audit and the missing behavior/
judgment claim (C13).

---

## 2. Steelman rulings on the convergent refutations

**2.1 C5's downstream dissolutions (S2 + S5) — STANDS.**
Steelman: under manifest-as-principal, "same-uid writer" is definitionally
gone — T2 *as named* dissolves, and the kernel can now express the exclusion
topology as first-class policy rather than emulation. Ruling: the steelman
concedes the point. The race class survives wherever two principals both
hold write authority to one namespace; what removes it is exclusion topology
— A27's *existing, ratified* answer. Principals make topology
kernel-expressible; they do not make it unnecessary. The approval-surface
sentence fails harder: SI-23's documented failure mode is synthetic input
arriving *through a legitimate actuation grant*, which principal separation
cannot distinguish from a human — the note contradicts an OPEN SI. Both
sentences struck.

**2.2 C8's staging recipe (S3 + S5) — STANDS, decisively.**
Steelman: hardlinking unchanged entries is a deliberate T3-friendliness
feature, and reconciliation could diff against the prepare-time CAS image,
so retained-tree immutability is not required. Ruling: fails twice. (a) With
shared inodes an in-place edit mutates *both* generations — a diff cannot
tell which side of the swap the edit landed on; attribution fails, and
correctness becomes a function of editor save style. (b) "Edits ride
through" is verbatim the mutation A27.1 forbids — commit installing bytes
sourced from mutable live storage after prepare-time verification. The
cleanest kill in the set: two reviewers who never met, one from filesystem
semantics, one from spec conformance, converged on the same defect.

**2.3 C11 as an OS category (S2 + S4 + S5) — STANDS.**
Steelman: C11 is framed as a horizon, and Talos/Bottlerocket show
composition alone can constitute an OS category. Ruling: three independent
failures survive. S2: every named component is Firecracker/Kata plus an ASF
control plane; no invariant named that a conventional microVM runtime plus
policy engine cannot provide. S4: no buyer for the category as such. S5
(verified): W-4 specifies attestation plus a microVM whose only door is the
proxy; PID-1 adjacency, subvolume branches, caveat compilation, eBPF
integration, and a patch series are materially new scope — and "design
discipline that follows today" is a present-tense work item inside a note
whose §5 promises there are none. Struck; the engineering core survives one
sentence.

**2.4 C7's "attributed, never lost, by construction" (S3 + S5) — PARTIALLY
STANDS.** The mechanism insight survives fully — including the
snapshot-at-a-point refutation, now with three independent derivations. But
"nothing falls between" is contradicted by the note's own descriptor
residual, and S3 adds two more boundary crossings (shared inodes; NFS
clients resolving the old target for tens of seconds under default caching).
The guarantee must be scoped to a profile; "by construction," unqualified,
is struck.

**2.5 C4's additive/subtractive binary (S2 + S5) — PARTIALLY STANDS.**
The steelman saves the deployed-ecosystem observation (and notes Landlock
defaults open for *unhandled* access categories — the fail-open-on-unknown-
dimension shape ASF forbids), but not the essentialist binary: Smack/SELinux
are explicit-allow; seccomp can default-kill; capability systems fail open
via over-broad grants. S2's object-model-mismatch replacement is stronger
for the note: it locates the danger at the compiler without a false
dichotomy to defend. Refined, not struck. (S2's "REFUTE" verdict overstates
its own finding.)

**2.6 C3's monocausal uid claim (S2 + S5) — PARTIALLY STANDS.**
Steelman: the portable floor includes Darwin — the actual dogfood host —
where S2's counter-arsenal does not exist. Ruling: blunts the REFUTE to a
refine, but S2's causal correction stands: the listed problems arise because
the current posture configures *no* boundary (P7), and the trust-root and
actuation problems are not uid-shaped. S2's replacement sentence adopted,
Darwin caveat restored.

**2.7 The §5 fence breach and SI-40 arm pre-selection (S5 alone) — STANDS;
a decisive singleton that outranks convergence.** Verified fact, not
judgment: SI-40 names per-entry capture-or-refuse the leading candidate and
files substrate-assisted retention as a deployment-floor option; the note's
C7 declares "What works: the atomic swap." A theory note that quietly
pre-resolves an open SI's design contest is precisely the failure its own §5
promises to prevent. Every C7/C8 amendment carries explicit non-selection
language as a consequence.

**2.8 C10's economics (S4 + S1-B3) — PARTIALLY STANDS.**
The technical claim survives on the refuter's own exhibits (vendors shipping
non-POSIX sandbox boundaries confirms replaceability). What falls is the
inference: the boundary has a constituency after all — harness conventions
define what any boundary must transparently reproduce, and the
boundary-builders' incentives oppose portable manifest semantics. S1's
genealogy correction also stands.

**2.9 C6, C9, F1, F2, F3, F4, F5, F6 — STAND as refinements** (folded into
§3). Reviewer errors named: S3's C6 "REFUTE" lands on the exclusivity
flourish; the trichotomy survives as an outcome partition at the write
boundary (WALs are orthogonal crash recovery). S5's F4 assessment slightly
understates S2-B4 (clone double-spend), which forces a scope sentence into
C5.

**2.10 C12's strategic inference (S4) — PARTIALLY STANDS.**
The option-value core survives; the causal comfort does not. OCI's record
shows pre-donation distribution dominance made the format canonical — design
quality was not the causal variable, and the precedent's author exited the
value. S4's trust-ledger-as-weak-network-asset finding is correct but is a
finding against the *brief*, carried here as an honesty label.

---

## 3. Amendment list

Applied to `docs/substrate-theory.md` v2 — KEEP / REFINE (replacement
written out) / STRIKE (refutation preserved inline): C1 refined (analogy
kept, identity struck; S1-B1 credential-vs-ocap label added); C2 refined
(narrowed to the joined loop, with the §7-gap and fidelity-laundering
honesty labels); C3 refined (S2's manifest-instance-identity sentence +
Darwin caveat); C4 refined (object-model mismatch replaces the binary); C5
refined (diagnosis kept; both dissolutions struck; unit-of-isolation and
clone/double-spend scope conditions added; "20% kernel work" withdrawn); C6
refined (outcome partition, not exclusive choice); C7 refined
(profile-scoped guarantee; SI-40 non-selection paragraph added); **C8
STRUCK** (five refutations preserved; property/profile sentence relocated to
C7); C9 refined (four-axis profile matrix; rollback corollary kept —
untouched by any reviewer); C10 refined (harness-gravity replacement;
genealogy corrected); **C11 STRUCK** (three refutations preserved; appliance
one-liner survives under C10); C12 refined (option value, never capture);
**C13 ADDED** (behavior as the unit of earned trust; judgment as the
compilation boundary).

Falsifiers, re-ranked: **F2′** (top; S4's 90-day pre-registered
delegation-frontier protocol — currently untestable, and instrumenting it is
the note's single legitimately actionable item), **F6′** (S1's ablation +
S5's shrinkage metric), **F8** (new: behavior lineage/judge ceremonial if
version changes don't predict outcomes or judge work doesn't shrink),
**F1′** (economic form), **F7** (new: fidelity laundering), **F3′** (burden
reversed: "nearly free" is unsupported until the measurement matrix exists),
**F5′** (dated replacement: two independent runtimes emitting ASF records by
2027-06-30), **F4** demoted to C5's scope condition.

---

## 4. What the theory survived

- **C7's mechanism core**: the most hostile technical reviewer destroyed the
  recipe and kept the property. Retention-at-swap beats snapshot-at-a-point,
  with three independent derivations of the latter's refutation.
- **C2's narrowed loop**: the historian found ancestry for every leg
  individually and then bet on the conjunction. Close to the strongest
  support a priority claim can get.
- **C5's diagnosis (not its remedy)**: S2 refuted the kernel-work conclusion
  and volunteered that the narrower thesis is stronger.
- **C10's technical half, on the refuter's own exhibits.**
- **The note's own protocol**: numbered claims produced genuine cross-stance
  convergence (the hardlink kill arrived twice, independently), and the one
  thing convergence could not catch — the SI-40 fence breach — was caught by
  the one stance assigned to check grounding.

The pattern: the survivors are the parts written closest to ratified work;
the casualties are the parts written from analogy. The theory is strongest
where it touches the code and weakest where it romanticizes — a usable
editing rule for every future horizon note.

## 5. What it cannot survive

One question decides whether this note is a theory or an ornament: **does
demonstrated recoverability, presented as evidence, actually cause the
design-center operator to widen standing authority — and does the widened
authority displace judge and human intervention rather than accumulate
beside it?** (F2′ and F8, jointly.) If yes, the triple joined by
ratification is a real theoretical object and the substrate/format claims
are its logistics. If no, the ocap lineage is marketing, the ratchet is
decoration on a well-built broker-plus-backup, and the OS endgame is a
distro in search of a reason. The uncomfortable finding: the current dogfood
*cannot observe the answer* — it measures denial false-positives while
standing grants do not yet exist, so "users did not widen" is mechanically
predetermined. Instrumenting F2′ before histories accrue is the single
actionable item this note generates, and by its own §5 it must enter as an
ordinary filing, never by citation to this synthesis.
