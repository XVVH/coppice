# Substrate theory — ASF's theoretical basis and the operating-system endgame

**Status: THEORY NOTE — exploratory, non-normative.** Nothing here is
spec, roadmap, or a work item; nothing here authorizes implementation.
Anything actionable that emerges from this document enters the system
the ordinary way — an SI, W, or P filing judged on its own merits —
never by citation to this note. Provenance: the 2026-07-15 side-session
thought experiment split off from the SI-32/SI-40 ratification cycle
(operator + agent), written down so the reasoning can be attacked
rather than remembered. Claims are numbered **C1–C12** so critique can
cite them; the intended review protocol is the house battery shape —
fresh-context readers returning a row-per-claim verdict table
(agree / refute / refine, with the argument), never prose-only
impressions. A refuted claim gets corrected or struck here with the
refutation preserved, in the ADR tradition.

Grounding documents: `docs/agent-state-fabric-brief.md` (the product
thesis this note must not contradict), `docs/asf-schema-spec.md` (A27
§5.3 — the storage adversary model; §5.4–§5.5 — authority as event-
derived views; §7 — rules and trust records), `docs/spec-issues.md`
(SI-32, SI-40), `docs/posture-assumptions.md` (P7, P29, the gate
vocabulary).

---

## 1. What problem this architecture is actually an instance of

**C1 — Prompt injection is the confused deputy problem, restated for
LLMs.** The founding document of capability theory is Hardy's "The
Confused Deputy" (1988): a program acting with its *caller's* ambient
authority is steered by its *input* into actions the caller never
intended. Replace Hardy's compiler with an agent reading a poisoned
webpage, and the billing file with everything the user's session token
can touch: the attack is unchanged. Forty years of object-capability
work (KeyKOS, EROS, the E language, Miller's *Robust Composition*;
credential form: Google's macaroons, 2014, the direct ancestor of the
§5 caveat grammar) built the answer — no ambient authority; rights are
explicit, attenuable, and travel with the request — and never found a
mainstream workload that would pay the compatibility cost. C1's
consequence: ASF is not a reaction to this year's agent-safety
discourse; it is the object-capability tradition applied to delegation,
arriving with the workload that finally demands it.

**C2 — ASF's addition to the ocap tradition is consequence, not
authority.** Object capabilities bound what *can happen*. They say
nothing about what what-happened *costs* — ocap has no undo. ASF's
synthesis is to pair the authority bound (capabilities + caveats) with
a consequence bound (content-addressed snapshots + coherent revert +
reversibility classes), and then couple them: evidence that outcomes
were recoverable compiles — through human ratification — into wider
standing authority (§7, the ratchets). The third leg, behavior-version
pinning (trust is evidence about a *specific* behavior; mutation drops
grants to escalation), addresses something the ocap tradition never had
to face, because its subjects were programs-as-artifacts, not evolving
learners. The brief claims the pinning as novel; this note claims the
*triple* — authority bound, consequence bound, evidence-coupled — as
the theoretical identity of the fabric.

## 2. The substrate anchoring

**C3 — POSIX's finest durable principal is the uid, and this single
fact generates most of the fabric's hard security problems.** Read
A27's tier model through this lens. T2 ("no sequence of pathname checks
can win; the honest answer is OS-enforced exclusion") exists because
once two processes share a uid, the kernel offers no boundary between
them: every pathname race, hardlink pre-plant, descriptor survival, and
"the approval socket is reachable by granted hands" (P7, P5) is
downstream of the kernel being unable to say *this process acts under
manifest X, that one under manifest Y*. The same fact shapes the
dogfooding hygiene rules (two surfaces by convention), the T1 trust-root
residual (keys readable by anything wearing the uid), and W-4's whole
reason to exist.

**C4 — The container ecosystem is subtractive security; the caveat
grammar is additive; the adapter between them is where the failure
modes live.** Namespaces, chroot, seccomp, cgroups, Landlock: each
starts from a process born with full ambient authority and carves
pieces away. Subtractive security fails open by construction — the
recurring container CVE is "we forgot to carve away X." Capability
discipline is the additive inverse — born with nothing, handed specific
rights — and fails closed by construction. ASF's authority model is
additive (unknown dimensions fail closed; attenuation is subset-only),
but it runs on a subtractive substrate, so every enforcement claim
bottoms out in an emulation layer (deny rules today, W-4 containment
at graduation) whose job is to fake an additive boundary out of
subtractive parts. A27.4's standing sentence — "holds under COOP;
requires W-4 containment at G-ADVERSARIAL" — is the honest label on
that adapter.

**C5 — Manifest-as-principal is the kernel-shaped hole.** The one
genuinely missing operating-system concept, reduced from the "AI-native
OS" intuition: processes born bound to a delegation manifest, with
authority attenuating at spawn, the uid demoted to an accounting
detail, and the reference monitor evaluating caveats instead of mode
bits. Under manifest-as-principal, A27's T2 tier dissolves *by
construction* (there is no "same principal" between agent and broker to
race within), C2's approval surface is enforced the way memory
protection is, and "the agent never holds the real key" stops being an
architectural achievement and becomes how the machine works. Nearly
everything else the intuition wants already ships as parts: CoW state
(btrfs/ZFS/overlayfs; composefs + fs-verity is a content-addressed,
integrity-verified store as a mount type), additive scoping primitives
(Landlock, 5.13+), measured behavior (IMA/EVM, dm-verity), ambient
tracing (eBPF, auditd), and even C2's ancestor — the secure attention
key, the unfakeable-dialog invariant shipped since Windows NT (1993).
The distro is ~80% assembly; the principal is the ~20% that is real
kernel work.

## 3. Publication theory (what the SI-40 exploration generalized)

**C6 — For publication into a substrate with unmediated concurrent
writers, the solution space is exactly three families.** (1)
*Exclusion*: lock writers out during publish — foreclosed for
shared-state stores because unmediated human access is the product
thesis (brief §2; D32-4). (2) *Detection at the write boundary*:
per-entry compare-and-capture — race-narrowing forever, protocol-heavy;
it fights the substrate. (3) *Indirection*: never overwrite in place;
publish as one atomic namespace transition; retain the superseded
state. Every mature storage system chose family 3 (MVCC, git's
immutable objects + atomic ref update, LSM trees, CoW filesystems,
symlink-flip deploys).

**C7 — The load-bearing mechanism in family 3 is retention, not
snapshotting — and the snapshot-at-a-point variant is refuted.** A
snapshot taken at any fixed point (gate-lock acquisition, apply start)
cannot preserve an edit that postdates it, and the SI-40 edit postdates
the prepare check by definition; the snapshot captures exactly the
state that was never in danger. (This variant was independently refuted
by this exploration's first pass and by external review of the SI-40
filing — the convergence is recorded because the wrong variant is the
intuitive one.) What works: the atomic swap makes every concurrent
edit's fate *well-defined* — before the swap it lands in the retained
outgoing state (kept, diffed, CAS-ingested, attributed); after, it is
ordinary drift on the new live state. Nothing falls between, because
there is no between. "Attributed, never lost" achieved by construction
rather than by detection. The residual is the descriptor tax: an open
fd can still write into retained/unlinked state post-swap — a loss mode
the in-place design already carries today, socially mitigated by
editors' own file-changed detection, removable only by families 1 or
full mediation.

**C8 — Publication guarantees should be stated as properties with
per-substrate profiles, never as primitives.** The property: *one
atomic namespace transition; superseded state retained until diffed,
captured, and attributed*. The bindings form a ladder discovered at
store registration and recorded ledger-visibly (the durable-commit
profile pattern): exchange-rename (Linux `renameat2(RENAME_EXCHANGE)`,
2014 — ext4/btrfs/xfs/tmpfs; Darwin `renamex_np(RENAME_SWAP)` is the
later port), symlink-flip (pure POSIX, NFS-safe), journaled two-rename
(universal floor; non-atomic but fail-visible and recoverable through
the existing recovery grammar). Staging recipe: unchanged entries
hardlink from live (inode/mtime preservation; edits ride through);
changed entries reflink from CAS (`FICLONE`/`clonefile` — independent
inodes, so no writable path into the CAS; plain copy as floor).
Retention under this recipe is nearly free (unique bytes ≈ old versions
of changed entries, already CAS-resident from prepare) and
reconciliation is O(changed), not O(tree).

**C9 — Substrates grade up, and the local POSIX directory is the
floor, not the model.** The same store contract degrades or dissolves
by substrate: raw POSIX dir (families 2/3 emulated in userspace) → CoW
filesystem (family 3 native; hosted infrastructure can simply provision
it — zero-friction constrains the *user's* machine, not the product's
cloud) → branch-native platform (Tier-2 stores publish through the
platform's own atomic branch operation; the SI-40 window mostly does
not exist there) → fabric-served view (every write mediated and
attributed; T3 dissolves). The last rung is deliberately not taken for
user machines: it is the closed-world assumption wearing a mount point,
and brief §2 bets the market on its negation. Corollary, double-edged:
hosted CoW infrastructure also makes *whole-home rollback* a
one-command accident — the exact coherent-suffix-regression case A27
assigns to the external anchor — so the same substrate that solves
publication sharpens the case that layer-2 anchoring is a hard
G-PRODUCTION requirement.

## 4. The endgame ladder and why the timing is not romantic

**C10 — Agent fleets are the first workload class in decades with no
POSIX loyalty at the trust boundary.** Capability operating systems
(KeyKOS → EROS → Capsicum → seL4 → Fuchsia) lost to compatibility
economics, not to refutation: nobody rewrites the world's software for
a better security model. Agents change the economics selectively: the
*trust boundary* around an agent has no legacy-software constituency,
even though the agent's *toolbox inside* the boundary remains
POSIX-hungry (today's agents live in bash). So the claim is refined,
not naive: POSIX survives indefinitely as the toolbox inside the
sandbox; it is replaceable as the boundary *around* it. That is exactly
the shape of a microVM whose only door is the broker.

**C11 — The buildable near-term form is a single-purpose agent-runtime
OS, and it is W-4's limit case.** Not a desktop distro for humans; the
Talos/Bottlerocket pattern ("the Kubernetes OS," "the container OS")
applied to delegation: the image an agent's microVM boots — fabric
daemon adjacent to PID 1, every workload manifest-scoped, branches as
subvolumes, Landlock/cgroup profiles compiled from caveats, eBPF trace
feeding the signed substrate, approval surfaces on the host side of the
VM boundary. No new kernel; one patch series (the principal) at most,
assembly otherwise. This is not a departure from the roadmap; it is
W-4 containment matured until the sandbox profile *is* the boot image.
Design discipline that follows today: W-4's emulation interface should
be designed as if it were the future kernel interface, because on this
path it becomes one.

**C12 — The strategic precedent is Docker/OCI: name the unit, ship the
format; formats outlive assemblers.** Docker invented almost no kernel
mechanism (namespaces 2002–2013, cgroups 2007); it named the container,
shipped an image format, and the format — donated to neutral
governance — outlived Docker's market position. The brief's §9 strategy
(open formats, donate the spec, monetize the runtime) already chose
this side. The endgame corollary: design the manifest, caveat, and
trace schemas so that a kernel *could* enforce them — substrate-free
semantics, posture-bound implementations — and the fabric's formats
become the candidate wire format of whatever agent-OS eventually
exists, whoever builds it. The house discipline already trends this
way (properties with profiles; T2 answered by topology, not syscalls);
this claim just names why it matters beyond tidiness.

## 5. What this theory does NOT license

Stated to keep the note honest and the critique aimed:

- It does not reprioritize anything. The current queue (dogfooding, the
  ratchet, W-15a) is where the product lives; this note is a horizon,
  not a backlog. A theory note that quietly becomes a roadmap is the
  spiral this project just corrected.
- It does not weaken the open-world bet. C5/C11 describe substrates the
  product may *provision*; they never justify requiring one from a
  user's laptop.
- It does not claim the fabric needs an OS to be valuable. The wedge
  thesis (userspace proxy, drop-in, vendor-free) is unchanged; the
  ladder is one-directional option value.

## 6. Falsifiers and open weaknesses (attack here first)

- **F1 (vs C10):** if agent *authority* patterns turn out to require
  deep POSIX semantics at the boundary (not just inside it) — e.g.,
  tool ecosystems that structurally resist brokered mediation — the
  boundary-replaceability claim fails and the adapter (C4) is permanent.
- **F2 (vs C2):** if consequence-bounding does not actually compile
  into delegation confidence — i.e., dogfooding shows users do not
  widen authority even with clean recoverable histories — the coupling
  thesis is decoration and the product is "just backup."
- **F3 (vs C6/C8):** if reconciliation costs blow up on real stores
  (huge vaults, high churn, sync-daemon interference), family 3's
  "nearly free" claim degrades and family 2 re-enters.
- **F4 (vs C5):** manifest-as-principal may reintroduce the identity
  problem one level down: cross-host principals need a trust root, and
  the anchor problem (spec §6.2 layer 2) recurs inside the kernel
  boundary rather than being solved by it.
- **F5 (vs C12):** the formats-win analogy fails if a hyperscaler ships
  a vertically integrated agent-OS with proprietary formats *before*
  neutral formats accumulate network effects — the brief's timing risk,
  restated at the substrate layer.
- **F6 (vs C1/C2):** the ocap framing may flatter the design: caveat
  grammars are coarser than ocap's object granularity, and the judge/
  escalation layer is an admission that mechanical authority alone
  cannot express intent. If the interesting delegation decisions all
  land in the judgment layer, the capability lineage is marketing, not
  mechanism.

## 7. Reading lineage

Hardy, "The Confused Deputy" (1988) · Miller, *Robust Composition*
(2006) and the E language · KeyKOS / EROS · Watson et al., Capsicum
(USENIX Security 2010) · seL4 (verified capability microkernel) ·
Fuchsia/Zircon (handles, no ambient authority) · Birgisson et al.,
"Macaroons" (NDSS 2014) · Landlock (Linux 5.13) · IMA/EVM, dm-verity,
fs-verity, composefs · NixOS / ostree (immutable, generation-based
system state) · Talos, Bottlerocket (single-purpose OS pattern) ·
`renameat2(2)` / `renamex_np(2)` · Firecracker (microVM isolation) ·
OCI (the format-outlives-assembler precedent).
