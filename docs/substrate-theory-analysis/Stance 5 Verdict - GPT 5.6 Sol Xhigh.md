Editorial verdict: major rewrite. The note’s best idea is retention-at-swap, but it prematurely presents an open SI-40 alternative as settled and then specifies a hardlink-based staging recipe that conflicts with the normative staged-bytes rule. That is exactly the epistemic failure the note says it exists to prevent.

I did not assess the external historical claims; this review is confined to argumentative economy, internal consistency, and fidelity to the grounding documents.

## Part A — C1–C12

| Claim | Role | Verdict | One-line justification |
|---|---|---|---|
| C1 | Supporting | MERGE | The confused-deputy lineage is useful setup for C2, but “the attack is unchanged” and the historical catalogue do no independent argumentative work. |
| C2 | Load-bearing | KEEP | This is the note’s actual theoretical thesis, but it must distinguish the brief’s aspirational consequence→authority coupling from what §7 currently specifies. |
| C3 | Supporting | MERGE | Same-uid weakness is a premise of C5; “this single fact generates most problems” is monocausal decoration that erases distinct trust-root, actuation, and policy failures. |
| C4 | Supporting | MERGE | The userspace-adapter tension belongs inside C5, while the additive/subtractive binary and “every enforcement claim” overstate A27’s specifically T2-scoped containment dependency. |
| C5 | Load-bearing | KEEP | Manifest-scoped execution is the indispensable OS bridge, but it does not by itself dissolve T2 or solve approval-surface reachability. |
| C6 | Supporting | MERGE | The taxonomy only scaffolds C7; “exactly three” is undefended, and detection remains necessary inside the purported indirection solution. |
| C7 | Load-bearing | KEEP | Retention rather than an earlier snapshot is the sharpest mechanism insight, but its descriptor residual defeats the unqualified “attributed, never lost” conclusion. |
| C8 | Decorative/speculative | MERGE | Keep the property/profile distinction in C7; remove the syscall recipe, cost claims, and registration behavior, which are unratified protocol work. |
| C9 | Supporting | MERGE | The substrate ladder usefully qualifies C7, but its branch-platform and hosted-infrastructure assertions are examples, not another claim. |
| C10 | Load-bearing | KEEP | Boundary compatibility economics is the necessary premise for the OS endgame and has a recognizable falsifier in F1. |
| C11 | Decorative/actionable | DELETE | It is an agent-OS roadmap disguised as a limit case and materially expands W-4 while claiming not to. |
| C12 | Load-bearing | KEEP | Portable formats provide the strategic bridge from current fabric to possible OS enforcement, provided this is framed as option value rather than prediction or design mandate. |

## The five-claim note

The survivors should be rewritten as:

1. **C1+C2 — Authority, consequence, and epistemic compilation.** Prompted delegation inherits the confused-deputy structure; ASF adds state-based consequence bounds, behavior-scoped evidence, and human-ratified compilation rather than merely another capability syntax.

2. **C3+C4+C5 — The substrate mismatch.** Current same-uid hosts do not natively express a delegation manifest as an execution principal, so W-4 emulates that boundary; manifest-as-principal is a hypothetical end state, not a solution to unresolved approval and actuation semantics.

3. **C6+C7+C8+C9 — The publication property.** In open-world shared state, substrate-assisted publication is useful only when an atomic namespace transition retains the outgoing generation long enough to capture and attribute divergence; implementations must state weaker profiles honestly. No recipe belongs here while SI-40 remains open.

4. **C10 — The compatibility wedge.** Agent workloads can retain POSIX inside the toolbox while replacing the trust boundary around it, making capability-oriented execution economically more plausible than earlier general-purpose capability systems.

5. **C12 — Format option value.** Substrate-independent, enforceable manifest/capability/trace semantics could survive into a future runtime or OS, but this creates an option rather than a roadmap.

What is lost: the historical name parade, the “exactly three families” flourish, the filesystem recipe, the hosted-substrate ladder, and the agent-OS mock-up. None is necessary to the inference. The SI-40 design material belongs in SI-40; the OS mock-up belongs in a separately labeled horizon memo.

## F1–F6 ranked by bite

| Rank | Falsifier | Assessment | Visible consequence |
|---:|---|---|---|
| 1 | F2 | **Real, but under-instrumented.** It attacks the central coupling claim through observable user behavior. | Remove consequence→authority from C2 and stop claiming recoverability is the product’s authority engine; “just backup” still overstates the result because the broker survives. |
| 2 | F6 | **Potentially devastating, not yet operational.** “Interesting decisions” can be redefined forever. | If repeated consequential work does not migrate from judge/human decisions into mechanical rules, recenter the theory on judgment rather than ocap. |
| 3 | F1 | **Real.** It kills the trust-boundary replacement thesis rather than merely weakening an implementation. | Treat W-4’s POSIX adapter as the permanent architecture and delete the OS-endgame claim. |
| 4 | F3 | **Real but local.** “Blow up” needs explicit latency, space-amplification, and churn thresholds. | Demote atomic-retention profiles or reintroduce boundary detection for affected stores; C2/C10/C12 remain intact. |
| 5 | F5 | **Performative.** A proprietary first mover changes commercial timing but does not falsify format durability or the Docker/OCI analogy. | At most it changes strategy and urgency, not the architectural claim. |
| 6 | F4 | **Not a falsifier.** Cross-host principals needing a trust root is an unsurprising boundary condition, not a refutation of a local execution principal. | Move it to “open weaknesses”; no stated claim necessarily changes. |

To make F6 real: define a cohort of repeated workflows and a threshold for the fraction of consequential decisions still requiring judge/human intervention after ratification opportunities. To make F3 real: name the product budget beyond which the profile changes.

## §5’s fence is not credible

The status paragraph says nothing here is actionable, but the body repeatedly uses imperative or implementation-selecting language:

- “The bindings form a ladder discovered at store registration and recorded ledger-visibly…” ([C8](/Users/josh/dev/Coppice/docs/substrate-theory.md:142))
- “Staging recipe: unchanged entries hardlink from live … changed entries reflink from CAS…” ([C8](/Users/josh/dev/Coppice/docs/substrate-theory.md:151))
- “Design discipline that follows today: W-4’s emulation interface should be designed as if it were the future kernel interface…” ([C11](/Users/josh/dev/Coppice/docs/substrate-theory.md:200))
- “The endgame corollary: design the manifest, caveat, and trace schemas so that a kernel could enforce them…” ([C12](/Users/josh/dev/Coppice/docs/substrate-theory.md:204))

The third quotation is unambiguously a current work item. The first two specify a new registration protocol and publication implementation. The last imposes a schema design criterion.

More seriously, “What works: the atomic swap” selects SI-40’s substrate-assisted arm even though SI-40 calls preservation/refusal the leading candidate and says substrate-assisted preservation should be evaluated only when a trigger fires ([SI-40](/Users/josh/dev/Coppice/docs/spec-issues.md:74), [substrate alternative](/Users/josh/dev/Coppice/docs/spec-issues.md:91)). A disclaimer cannot neutralize imperative prose. Either remove those prescriptions or file and cross-reference the corresponding SI/W/P items.

## Grounding conflicts

The strongest conflicts are:

1. **C8 violates A27.1.** Hardlinking unchanged entries from the mutable live tree means an in-place edit can mutate the prepared incoming generation after verification. That conflicts with the requirement that forward images be immutable CAS-derived plans and that commit be a pure function of already-trusted bytes ([A24](/Users/josh/dev/Coppice/docs/asf-schema-spec.md:218), [A27.1](/Users/josh/dev/Coppice/docs/asf-schema-spec.md:223)). “Edits ride through” is precisely the forbidden mutation of the planned after-root.

2. **C7 claims more than its own residual permits.** It says “Nothing falls between” and “‘Attributed, never lost’ achieved by construction,” then admits an open descriptor can write into the retained generation after the swap. Unless reconciliation establishes a closing point against such writers, that write can miss capture and attribution. The residual refutes the preceding guarantee rather than merely taxing it.

3. **C8’s floor is not a profile of its stated property.** The property requires “one atomic namespace transition,” but the “universal floor” is explicitly a non-atomic two-rename protocol. That is a different, weaker property and must be labeled as such.

4. **C5 silently resolves SI-23.** Saying manifest-as-principal makes the approval surface memory-protected assumes the reference monitor already understands actuation reach and qualifying approval surfaces. SI-23 explicitly says W-4 containment does not solve legitimate granted actuation reaching that surface ([SI-23](/Users/josh/dev/Coppice/docs/spec-issues.md:1124)). Principal separation is necessary but not sufficient.

5. **C11 expands W-4 while denying expansion.** W-4 currently specifies attestation plus “a container/microVM whose only door is the proxy” ([roadmap](/Users/josh/dev/Coppice/docs/roadmap.md:373)). PID-1 adjacency, subvolume branches, caveat compilation, eBPF trace integration, and a kernel patch series are materially new scope.

6. **C2 overstates its §7 citation.** The brief does say “Reversal earns authority” ([brief](/Users/josh/dev/Coppice/docs/agent-state-fabric-brief.md:135)), but normative §7 constructs rules from k approved examples and records raw behavior-scoped counters; it does not require recoverability evidence before widening authority ([StandingRule](/Users/josh/dev/Coppice/docs/asf-schema-spec.md:339), [TrustRecord](/Users/josh/dev/Coppice/docs/asf-schema-spec.md:364)). The note promotes a product aspiration into the fabric’s “theoretical identity” without acknowledging the specification gap.

## The missing claim

The behavior/judge silence is a gap, not a legitimate scope fence.

C2 introduces behavior pinning as “the third leg,” then defines the triple as “authority bound, consequence bound, evidence-coupled”—behavior disappears from its own enumeration. Meanwhile the note claims to explain ASF’s theoretical basis as a whole, not merely filesystem publication. F6 itself concedes that the judge may determine whether the ocap framing is substance or marketing.

The missing claim should be something like:

> **C13 — Behavior is the unit of earned trust, and judgment is the compilation boundary.** Mechanical capabilities decide only stable, auditable predicates; a capability-less judge handles residual ambiguity, human ratification is the sole transition from judgment to standing authority, and evidence remains valid only for the behavior version that produced it. The loop succeeds only if repeated judgment shrinks into mechanical rules without losing behavior provenance.

Its falsifier is direct: if behavior-version changes do not predict materially different outcomes, or if repeated judge work does not shrink through ratification, the behavior lineage and judge are ceremonial rather than load-bearing.

Without that claim, this is a strong storage-and-capability horizon note mislabeled as ASF’s theoretical basis.
