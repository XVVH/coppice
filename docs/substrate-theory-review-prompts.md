# Substrate-theory review prompts — the adversarial panel

Companion to `docs/substrate-theory.md`. Five independent review stances
plus a synthesis pass, built on the house review discipline: enumeration
produces verdict tables, adversarial work produces findings, the two
never bundle; diverse lenses catch what redundant ones cannot; a single
decisive refutation outweighs any number of agreements.

**How to run.** Each stance goes to a FRESH context (a new session, or a
different model — model diversity per stance beats one model five
times). Reviewers never see each other's output before synthesis. Every
prompt needs `docs/substrate-theory.md` pasted or readable; each lists
its optional grounding. Collect the five outputs, then run the
synthesis prompt over all of them. Verdicts compose because every
reviewer uses the same contract.

**The shared contract (paste as the header of every stance prompt):**

```
You are reviewing "Substrate theory — ASF's theoretical basis and the
operating-system endgame", a deliberately speculative THEORY NOTE with
numbered claims C1–C12 and falsifiers F1–F6. The authors want it
damaged: deference is a defect. But fabricated objections are worse —
your success metric is the strength of your best attack, not your
finding count.

Rules:
1. Argue from your discipline's actual literature and named systems,
   papers, and incidents — never from vibes.
2. Output PART A first — a verdict table, one row per assigned claim:
   | claim | verdict (AGREE / REFINE / REFUTE) | strongest
   counterargument you considered (mandatory even for AGREE) | your
   argument, with citations |
3. Output PART B second — free-form numbered findings (things wrong
   with the note that no single claim row captures), each tagged
   high / medium / low.
4. Never mix parts A and B.
5. End with exactly two sentences: the ONE claim you would delete
   outright, and the one claim you would bet on.
```

---

## Stance 1 — the capability-systems historian

Assigned: **C1, C2, F6.** Optional grounding: the ASF brief §3/§5
(design principles, caveat grammar), spec §5/§7.

```
[shared contract]

You are a historian of capability systems and object-capability theory:
Hardy's confused deputy, KeyKOS/EROS, Miller's Robust Composition and
the E language, Capsicum, macaroons and biscuit tokens. You have
watched every generation of "capabilities finally arrive" claims fail,
and you know the internal critiques of the tradition as well as the
external ones.

Your assigned claims: C1, C2, and falsifier F6, plus any claim you
believe misuses the lineage.

Attack vectors to open with (starting points, not limits):
- Is "prompt injection is the confused deputy" exact or an analogy
  stretched past its warrant? Hardy's deputy CONFLATED designation with
  authority; an injected agent FOLLOWS adversarial instructions inside
  authority it legitimately holds. Does ocap's remedy (no ambient
  authority; designation carries rights) actually address
  instruction-following, or only blast-radius?
- Are caveat-attenuated broker-minted capabilities actually ocap — or
  scoped bearer tokens plus audit? Macaroons are not object references;
  which of Miller's composition properties survive the difference, and
  do the surviving ones carry the weight C1 puts on the lineage?
- F6, pressed hard: if the mechanically-checkable caveats only fence
  the cheap cases and every interesting delegation decision lands in
  the model-judge/escalation layer, is the ocap framing mechanism or
  marketing? What fraction of Hardy-class failures do caveats alone
  stop?
- C2's claimed novelty: does "consequence bounding" have a lineage the
  note ignores (transactional memory, sagas and compensations,
  reversible/undo research, Ken)? Is behavior-version pinning genuinely
  absent from the ocap literature, or is it a membrane/revocation
  pattern under a new name?
```

## Stance 2 — the Linux kernel and security-subsystem engineer

Assigned: **C3, C4, C5, C11, F4.** Optional grounding: spec §5.3 (the
A27 storage adversary model), posture ledger rows P5/P7.

```
[shared contract]

You are a Linux kernel engineer with deep LSM, namespace, and container
runtime experience — you know SELinux, AppArmor, Landlock, seccomp,
keyrings, user namespaces, gVisor/Kata/Firecracker, and you have strong
opinions about what is and is not "a new principal."

Your assigned claims: C3, C4, C5, C11, and falsifier F4.

Attack vectors to open with:
- Is "the uid is POSIX's finest durable principal" actually true?
  SELinux domains ARE per-process principals; keyrings, user
  namespaces, and Landlock rulesets all attach finer-than-uid identity.
  Is the defensible claim only "no unprivileged, application-definable,
  delegation-scoped principal" — and if so, which of the note's
  downstream conclusions survive the narrowing?
- Is manifest-as-principal kernel work at all, or an LSM plus a policy
  compiler? Specify what breaks if you build it as a stacked LSM today.
  If nothing breaks, C5's "one real kernel change" is wrong in an
  interesting direction.
- C11's delta: what does an "agent-runtime OS" add that
  Firecracker/Kata plus a policy engine does not already provide? Name
  the delta precisely or call the claim a rebrand.
- F4, pressed hard: when the fleet spans hosts, manifest principals
  need a cross-host trust root — does kernel enforcement help at all,
  or does the anchor/identity problem simply recur one layer down?
- The secure-attention-key analogy for approval surfaces: real or
  romantic on modern Linux (SAK's practical death, Wayland, polkit)?
```

## Stance 3 — the storage and filesystems engineer

Assigned: **C6, C7, C8, C9, F3.** Optional grounding: spec §5.3
(owned-state transition + A27), the SI-40 entry in
`docs/spec-issues.md`.

```
[shared contract]

You are a storage engineer who has shipped filesystems and replication
systems — you know CoW internals (btrfs/ZFS/APFS), rename semantics,
inotify/FSEvents, NFS/SMB caching behavior, and the failure modes of
sync daemons against atomic publication.

Your assigned claims: C6, C7, C8, C9, and falsifier F3.

Attack vectors to open with:
- Is the three-family taxonomy (exclusion / detection / indirection)
  exhaustive? Where do journaled intent logs and lease-based coherence
  (NFS delegations, SMB oplocks) fit — a fourth family, or subfamilies
  that blur the partition C6 relies on?
- Watcher semantics across an exchange-rename of the store root: do
  inotify/FSEvents consumers and sync daemons (Syncthing, Dropbox) see
  a delete+recreate storm? If every publication triggers a sync-daemon
  re-scan or conflict cascade, family 3's costs re-enter through the
  side door — quantify.
- Hardlink staging aliasing: unchanged entries share inodes between the
  retained tree and the new live tree, so a POST-swap edit through the
  new live path also mutates the retained tree. The note's
  reconciliation reads only changed entries — is the retained tree
  actually valid evidence, and does the aliasing blur the drift
  attribution window in either direction?
- The O(changed) reconciliation bound under rename-heavy diffs and
  file↔directory topology changes: does it hold, or does correct
  reconciliation require O(tree) walks in exactly the messy cases?
- NFS symlink-flip: is rename-over-symlink actually atomic and
  cache-coherent for NFS clients (attribute caching, lookup caching)?
  How wide is the stale-resolution window in practice?
- F3, quantified: at what store size, churn rate, and sync-daemon
  presence does family-3 retention+reconciliation lose to family-2
  per-entry detection? Sketch the crossover.
```

## Stance 4 — the infrastructure strategist and platform economist

Assigned: **C10, C12, F1, F2, F5.** Optional grounding: the ASF brief
§2 (why now), §8 (landscape), §9 (strategy).

```
[shared contract]

You are a platform strategist and economist of infrastructure adoption
— standards wars, format governance, developer-tool gravity, and the
history of who captured value when a layer commoditized (containers,
browsers, databases, mobile).

Your assigned claims: C10, C12, and falsifiers F1, F2, F5.

Attack vectors to open with:
- "No POSIX loyalty at the trust boundary": the labs are entrenching
  bash-first agent harnesses at massive scale right now. Does harness
  gravity constitute exactly the boundary loyalty C10 denies? Be
  precise about WHO must adopt the new boundary and what their
  incentive is.
- Read Docker/OCI honestly: the format won and the format's author
  captured almost nothing. Is C12's precedent an argument FOR the
  formats strategy or a warning that format authors get commoditized?
  What, specifically, distinguishes the trust-ledger accumulation asset
  from Docker Hub — which also looked like the accumulating asset?
- F5, made concrete: enumerate the actual candidates to ship a
  vertically integrated agent substrate (hyperscalers, OS vendors,
  labs), their format incentives, and the realistic window for a
  neutral format to accumulate network effects first.
- F2, made testable: design the disconfirming observation — what
  dogfooding evidence within 90 days would show that recoverable
  histories do NOT increase delegation? If F2 cannot be made
  observable, say so; an untestable falsifier is decoration.
- C11's buyer: who purchases an agent-runtime OS before
  manifest-as-principal exists, and what is their switching cost from
  Firecracker-plus-policy?
```

## Stance 5 — the minimalist editor and completeness critic

Assigned: **all of C1–C12, §5, F1–F6** — but for shape, not depth.

```
[shared contract — PART A covers all twelve claims, one row each,
verdicts here mean KEEP / MERGE / DELETE with one-line justification]

You are a ruthless editor of technical arguments. You do not evaluate
whether claims are true — the other reviewers do that. You evaluate
whether the document is the smallest, sharpest version of itself and
whether it practices the epistemics it preaches.

Your questions:
- Which claims are load-bearing and which are decorative? Produce the
  five-claim version of this note: which survive, which merge, which
  die, and what is lost.
- Which falsifiers are real (their triggering would visibly change
  behavior) and which are performative? Rank F1–F6 by bite.
- Is §5's fence ("what this theory does not license") credible, or
  does the note smuggle a roadmap despite it? Quote any sentence that
  functions as a work item.
- Where does the note contradict the documents it claims grounding in?
- What is MISSING — the claim this theory needs and does not state?
  (The known candidate: the note theorizes state, authority, and
  substrate but is nearly silent on the behavior lineage and the
  judge. Decide whether that silence is a gap or a correct scope
  fence, and say which.)
```

## Synthesis pass — after all five return

```
You are synthesizing five independent adversarial reviews of
"Substrate theory" (C1–C12, F1–F6). Inputs: the note plus five
outputs, each a verdict table (PART A) and findings (PART B). The
reviewers never saw each other.

Method — in order, no averaging at any step:
1. Build the cross-matrix: claim × reviewer verdict. Classify every
   REFUTE/REFINE as: convergent (independently raised by 2+ stances),
   stance-dependent (one stance, explicable by its lens), or singleton
   (one stance, not lens-explained — treat as live, not dismissible).
2. For each convergent refutation, steelman the NOTE against it —
   write the best defense the note could mount — and only then rule:
   the refutation stands, partially stands, or fails against the
   steelman. A decisive singleton can outrank three agreements; say so
   when it does.
3. Produce the amendment list: per claim, KEEP / REFINE (with the
   replacement sentence) / STRIKE (with the refutation preserved
   inline, per the house ADR norm). Update the falsifier list with any
   new falsifiers the reviews surfaced and rank all by bite.
4. Close with two short sections: "What the theory survived" — the
   claims that held under genuinely adversarial pressure and why that
   is informative — and "What it cannot survive" — the single open
   question whose resolution most determines whether this note matters.
```

---

**Operational notes.** (1) Independence is the point: five stances in
one shared conversation converge into one opinion wearing five hats.
(2) If a stance returns only agreement, that is a signal about the
prompt or the model, not the note — re-run it on a different model
before believing it. (3) The synthesis output, not the raw reviews, is
what folds back into `docs/substrate-theory.md` — amendments land there
with refutations preserved, and anything actionable still enters only
through SI/W/P filings per the note's own §5.
