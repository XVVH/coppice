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
   drive a few note.read/write/move calls, quit the client (EOF → promotion),
   confirm trunk updated and `asf ledger --home …` explains every root.
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

## What to measure

**Primary — denial false-positive rate.** "Did the broker block good work?"
This is the product-thesis metric; a high rate kills the "delegate more,
sooner" promise. Capture: `asf ledger --home …` surfaces `verdict`
(source=broker) and `escalation`/`approval` events; for each block, judge by
hand whether the work was legitimate. Track the ratio over time — it should
fall as the caveat defaults prove themselves (and, later, as the Stage 3
ratchet compiles approvals into rules).

**Secondary UX signals.** Escalations per session; approval latency
(escalation → your decision); promotions auto-applied vs parked.

**Storage tripwires (ADR 0002 — F2 migration gate).** From `asf stats`:
CAS bytes/week and the sqlite-image vs fs-blob split; live memory-db size;
snapshot cadence; SI-6 false-drift noise (logically-idle sqlite touches
reading as drift). Any of: CAS > ~GB/month, memory-db > ~100 MB, or SI-6
noise polluting the ledger → execute the F2 chunking/CID migration.

**Coarse-session tripwire (ADR 0004).** ops-per-promotion and conflict
incidence. Routinely large or conflict-prone promotions → build the
`checkpoint` session boundary.

## Cadence

Baseline `asf stats` before the first real session. Snapshot `stats` weekly.
Skim `asf ledger` after any session that escalated, parked a promotion, or
felt wrong. Record the tripwire numbers where they'll be seen (a note in the
vault itself is fine — pleasingly recursive).

## Graduation criteria

- **Copy → live vault:** a few weeks of clean promotions, no revert
  surprises, denial-FP rate low and falling.
- **Build the checkpoint boundary:** the ADR 0004 tripwire fires.
- **Run the F2 migration:** an ADR 0002 storage tripwire fires (or spec
  publication nears, whichever first).
- **Widen a standing grant:** enough clean same-shape approvals accumulate to
  justify a rule — the Stage 3 caveat ratchet, human-ratified (not yet built;
  until then, every grant stays session-scoped).
- **Invite a second person:** only after multi-actor roots/visibility policy
  exists (§8.2 mechanism reserved, policy deferred) — not in v0.

## Failure mode

By design: "revert and shrug." `asf revert --home … <man:…>` restores all
roots of a manifest together. The git-backed corpus is the backstop beneath
that.
