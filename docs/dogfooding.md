# Dogfooding methodology — Stage 2/3, v0

How we run Coppice against a real vault and learn from it. Companion to
`testing-theory.md` (tests prove the broker denies what it *must*; only
dogfooding measures whether it denies what it *mustn't*). Boundary rule:
if a number can fail in CI it lives in testing-theory; if it can only fail in
real use it lives here.

Status: v0 — coarse app-lifetime sessions (ADR 0004), toy `vault-server`
downstream, single human, block-nothing-loudly posture (brief §5.2 Spine).

## The one decision that's yours: the corpus

Recommended: **a git-backed copy of your real vault**, not the live one, for
the first weeks.

Why a copy, not the live vault: the branch protects trunk *during* a session,
but **promotion mutates trunk**, and a bug in merge/promotion/revert could
corrupt it. Why git-backed specifically: it's the out-of-band undo for the
fabric itself — if the fabric mangles trunk, `git checkout` is the floor
under "revert and shrug." Why real content (a copy), not synthetic: the whole
point is to surface real filing judgment and real merge conflicts; synthetic
vaults won't. Note also **reads are irreversible (R2)**: if any downstream
tool ever gains egress, a read of private notes can't be undone — another
reason to start on a copy while the egress surface is still just local files.

Graduate to the live vault once a few weeks show clean promotions and no
revert surprises.

## Wiring (Phase 3)

Topology: MCP client → `asf proxy` (broker) → downstream `vault-server` →
session branch. The client never sees trunk; the agent never holds a key.

1. Build release: `cargo build --release` (the proxy spawns `asf` as its own
   downstream, so use the release binary path for both).
2. Point your MCP client at the proxy. For Claude Code, a project
   `.mcp.json` — in the **dogfood workspace**, never the Coppice dev repo
   (see "Session hygiene" below):
   ```json
   {
     "mcpServers": {
       "coppice-vault": {
         "command": "/ABS/PATH/target/release/asf",
         "args": ["proxy",
           "--home", "/ABS/PATH/asf-home",
           "--vault", "/ABS/PATH/vault-copy",
           "--downstream", "/ABS/PATH/target/release/asf",
             "vault-server", "--vault", "/ABS/PATH/vault-copy"]
       }
     }
   }
   ```
3. Smoke-test against a **sacrificial** vault copy first (a throwaway dir):
   drive a few note.read/write/move calls, quit the client (session end →
   promotion; if the client kills the daemon uncleanly, the next session's
   bootstrap — or `asf recover --home … --vault …` — gates the stranded
   branch, see RF-9), confirm trunk updated and `asf ledger --home …`
   explains every root.
4. Approvals happen on the daemon's own surface, never in the client:
   `asf approve --home … list | approve <id> --uses N | promotions |
   promote <id>`.

## Session hygiene — two surfaces, never mixed

The dev machine collapses a topology the design assumes: agent and operator
are the same Unix user. An MCP client with native filesystem tools has two
paths to the vault — brokered and direct — and it will prefer the direct one
*exactly when the broker frustrates it*, i.e. at a denial, i.e. at the event
being measured. Bypass converts would-be denials into silence (the FP rate
reads artificially clean), and direct trunk writes surface as drift
**attributed to the human** (single-human default) — so pollution corrupts
both the primary metric and the attribution ledger at once.

Nor is trunk the only direct path: session branches under the fabric home are
plain files, and the approval socket / `asf approve` are reachable by any
same-user process — an agent with a shell could approve its own escalations,
a C2 violation in spirit that the protocol topology cannot see.

The rule, until agent sandboxing makes it structural:

- **Agent surface** — dogfooding sessions run only in a dedicated workspace
  (`~/temp/coppice-dogfood-workspace`): carries the `coppice-vault`
  `.mcp.json`, a CLAUDE.md contract (denials are results to report, not
  obstacles to solve; approvals belong to the operator), and client deny
  rules blocking native file tools on the vault AND the fabric home, plus
  shell entirely (workflow 3 needs none). Client-side permission prompts for
  the brokered tools are deliberately OFF: governing calls is the broker's
  job, and double-governance would mask the UX being measured. One dogfood
  session at a time — concurrent proxies would contend for the fabric home
  and steal each other's approval socket.
- **Operator surface** — the Coppice dev repo session and your terminal:
  `asf approve`/`ledger`/`stats`, git forensics, native fs access by design.
  The dev repo intentionally carries **no** `.mcp.json`; never run
  workflow-3 tasks from it.
- **Detection backstop** — during dogfooding, any human-attributed drift you
  didn't make, or any trunk change unexplained by a promotion event, is a
  pollution incident: investigate before trusting that week's numbers.

Scope honesty for the eventual writeup: v0 measures a *cooperative* agent's
experience of the broker — the deny rules make cooperation structural, they
are not the enforcement story. Enforcement against a non-cooperative agent
is topological (a sandbox whose only door is the proxy), which is the
intended deployment shape and out of scope for v0. This section is a
recurring checklist, not a one-off: every new store, tool server, or channel
must answer "does the agent surface have a native path to this state?"
before it joins a dogfooding run.

## What to exercise

`dogfooding-rubric.md` enumerates the manual test batch — DF-P1…P8
(positives: must succeed cleanly; a denial there is a false positive) and
DF-N1…N9 (negatives: must be blocked/parked/attributed; those denials are
expected, not FPs). Log outcomes by id in the vault's dogfooding log.

## What to measure

**Primary — denial false-positive rate.** "Did the broker block good work?"
This is the **friction metric** — necessary but not the thesis (a high
rate kills the "delegate more, sooner" promise; a low rate proves only
that the fabric stays out of the way). The thesis metric is the frontier
log, below. Capture: `asf ledger --home …` surfaces `verdict`
(source=broker) and `escalation`/`approval` events; for each block, judge by
hand whether the work was legitimate. Track the ratio over time — it should
fall as the caveat defaults prove themselves (and, later, as the Stage 3
ratchet compiles approvals into rules).

**Secondary UX signals.** Escalations per session; approval latency
(escalation → your decision); promotions auto-applied vs parked.

**Capability gaps** (found by DF-P work dying with no verdict event): the
workflow failed in the tool registry, not the policy — the grant was never
consulted. Track separately from denial-FPs; each one is a missing tool
action (first instance: no enumeration → `note.list`, tool:vault@1.1).
The distinction matters because the fixes live in different layers: FPs
indict the caveat defaults, gaps indict the tool surface.

**Storage tripwires (ADR 0002 — F2 migration gate).** From `asf stats`:
CAS bytes/week and the sqlite-image vs fs-blob split; live memory-db size;
snapshot cadence; SI-6 false-drift noise (logically-idle sqlite touches
reading as drift). Any of: CAS > ~GB/month, memory-db > ~100 MB, or SI-6
noise polluting the ledger → execute the F2 chunking/CID migration.

**Coarse-session tripwire (ADR 0004).** ops-per-promotion and conflict
incidence. Routinely large or conflict-prone promotions → build the
`checkpoint` session boundary.

**The frontier log — the thesis metric (F2′).** Filed 2026-07-15 from
the substrate-theory review cycle (the panel's one actionable finding;
`docs/substrate-theory-analysis/`, PR #54). The loop the product claims
— reversal earns authority — is unobservable by the metrics above, and
its baseline is perishable: standing grants cannot widen until W-3
exists, so "the operator did not widen" is mechanically predetermined
today, while the pre-evidence counterfactual ("what would I grant
*without* the history?") is destroyed by every week of accumulating
familiarity. The instrument, per recurring workflow family (vault
maintenance now; workboard at W-21; others as they emerge):

1. **Baseline declaration — before W-15a→W-3 lands; about an hour.** A
   dated note in the vault (`coppice-frontier-log.md`, companion to the
   dogfooding log), one block per family:

   ```
   ## Frontier declaration — <family> — <date>
   Declared before reading ledger stats this session: yes/no
   Behavior bundle: <hash, or "placeholder">
   - Standing scope I would grant today (paths, action classes):
   - Grant duration:
   - Unattended runtime tolerated:
   - Auto-promotion classes:
   - Tolerable approvals per successful operation:
   - What evidence would move each line above:
   ```

   The last line is pre-registration: it makes later widening decisions
   comparable against *predicted* evidence, not post-hoc rationale.
2. **Re-declaration cadence.** Every two weeks, BEFORE reading ledger
   stats that session — drift is data, and stats-first contaminates the
   declaration. Pre-ratchet drift (habituation with no mechanism to act
   on it) is itself a useful control series.
3. **At W-3 — the treatment.** Every clerk proposal gets
   accept / narrow / reject plus a one-line reason, categorized
   **evidence-cited / fatigue-cited / other**; target ≥ 24 decisions at
   a fixed behavior version. No widening offer counts as evidence-backed
   for a family until ≥ 3 clean runs AND ≥ 1 *exercised* revert
   (DF-P7's class): clean streaks test "nothing broke"; only a real
   restore tests "breakage is survivable," and the thesis is about the
   second.
4. **Two legs, measured separately.** Leg A — frontier movement: the
   declared frontier widens, on evidence-cited reasons. Leg B —
   judgment displacement (F8): approvals per successful operation and
   the fraction of consequential operations still escalating must fall
   *because ratified rules absorbed them*; adopt the parked north-star
   counters (approval compression ratio, time-to-first-ratified-rule,
   % sessions fully silent) when W-3 lands.

Disconfirmation — the kill test: after clean histories and a
demonstrated restore, the frontier widens on no dimension, approval
burden does not fall, and rejections cite risks recovery does not
address (exfiltration, correctness, social consequence, accountability).
That kills the coupling thesis for the design-center operator — better
known before the trust-loop stages build out. The confound cuts both
ways: widening on fatigue-cited reasons is not success but the brief's
reflexive risk #1 (consent fatigue with receipts) wearing the thesis's
clothes; ratification-velocity-per-channel (§7.1) is the standing
tripwire. Known limits, stated up front: n = 1, within-subject,
experimenter-is-subject, and the designer's baseline is already
contaminated by internals-knowledge plus one prior session — tolerable
because the instrument measures *movement from a dated baseline*, not
absolute trust. Falsifies or supports for the design-center operator
only; market claims need a staged rollout, later.

## Cadence

Baseline `asf stats` before the first real session. Snapshot `stats` weekly.
Skim `asf ledger` after any session that escalated, parked a promotion, or
felt wrong. Record the tripwire numbers where they'll be seen (a note in the
vault itself is fine — pleasingly recursive).

## Graduation criteria

The full set of shortcuts each transition un-safes — and where each fix is
tracked — is the `posture-assumptions.md` ledger; the criteria below are the
dogfooding-facing subset. Before flipping any posture invariant, read that
transition's graduation gate there.

- **Copy → live vault:** a few weeks of clean promotions, no revert
  surprises, denial-FP rate low and falling.
- **Build the checkpoint boundary:** the ADR 0004 tripwire fires.
- **Run the F2 migration:** an ADR 0002 storage tripwire fires (or spec
  publication nears, whichever first).
- **Widen a standing grant:** enough clean same-shape approvals accumulate to
  justify a rule — the Stage 3 caveat ratchet, human-ratified (not yet built;
  until then, every grant stays session-scoped).
- **Wire the first egress tool (workflows 1–2):** only after `read.scope`
  (§5.2 R2) and a read-volume budget are implemented and minted in the
  default grant. Today reads are structurally unscoped within the store,
  which is harmless while every action is `side_effect: local` — but the
  first `external_reach: live` tool turns read scope into the exfiltration
  perimeter ("reads are irreversible"). Enumeration must be governed by
  the same scope (names are content: list results filter to scope, the
  `filter_tools_result` pattern). Adjacent open problem, not solved by
  this: A3 cross-run memory taint.
- **Register any actuation-scoped tool (computer use, shell, UI control):**
  only after SI-23 resolves and its approval-surface mechanisms exist (C7
  candidate: no self-satisfiable approvals). Actuation reach onto the
  approval surface is the vulnerable class — the "agent with a shell could
  approve its own escalations" hole above, generalized — and the
  conservative default cannot save it retroactively, so the gate is on
  registration itself. Same pattern as the egress/R2 criterion. Related
  precondition: **approval authentication** — today `auth_strength` is
  hardcoded `local_session`, asserted not proven (P3/P4 in the ledger); a
  real proof-of-human (hardware-attested presence) must exist before either
  actuation or an adversarial-agent posture, not just before SI-23's ranking
  change.
- **Register any third-party (non-first-party) tool:** only after tool
  containment (W-4 sandbox) and domain-taxonomy governance exist. Tool
  metadata (`reversibility`, `store`, `path_args`, class) is self-declared
  trusted-mechanical (P6); a lying or buggy tool defeats every caveat keyed
  on it, and there is no generic registration path today precisely because
  that gate is unbuilt.
- **Co-locate tenants inside one uid or fabric home:** only after tenant-
  scoped authorization and storage isolation exist. Encryption alone is not
  tenant isolation. Separate homes under separate Unix identities preserve
  today's enforced filesystem boundary, though the fleet machinery in the
  scalability analysis is still unbuilt.
- **Accept an offline disk-theft / untrusted-backup threat model:** only after
  at-rest confidentiality is addressed — keys and secrets (RF-14) and
  fabric-home state (RF-15) are plaintext inside the filesystem boundary;
  the owner KEK copied with the database nullifies live-payload encryption.
- **Invite a second person:** only after multi-actor roots/visibility policy
  exists (§8.2 mechanism reserved, policy deferred) — not in v0.

## Failure mode

By design: "revert and shrug." `asf revert --home … <man:…>` restores all
roots of a manifest together. The git-backed corpus is the backstop beneath
that.
