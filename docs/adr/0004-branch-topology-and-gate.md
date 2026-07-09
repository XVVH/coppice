# ADR 0004 — Branch topology and the promotion gate

**Status:** accepted — 2026-07-08 (milestone 3)

## Context

Spec §5.3 makes promotion "the only mutation in the system," which requires
agents to work on a fork. Milestones 1–2 had the agent writing live stores
directly (observe-everything posture). Milestone 3 introduces the fork.

## Decisions

1. **Branch = materialized copy under the fabric home.** At session start
   the proxy creates `<home>/fabric/branches/<manifest>/<store>` by
   materializing the manifest's roots from the CAS. Tier-1 stores are
   small (a vault, a memory db); a physical copy is simple, obviously
   correct, and CoW-optimizable later without API change. The live store
   is trunk: touched only by the human and by promotion.
2. **The downstream tool server is pointed at the branch by argument
   rewriting.** Any downstream argument equal to the vault path is
   rewritten to the branch path. The tool server needs no fabric
   awareness; the agent never learns where trunk lives.
3. **tool_call events capture branch roots under `branch:<store>` keys.**
   Trunk expectations (drift detection) move only at promotion; ledger
   accounting treats branch roots as the agent's world and trunk as the
   human's until the promotion event reconciles them.
4. **Promotion runs at session end (proxy stdin EOF)** — the M6 shape:
   short sessions, frequent gates. Order inside the gate: (a) span chain
   + signature verification, then trace-vs-capability re-verification
   (every recorded call rechecked against the capability that authorized
   it: action allowlist, reversibility ceiling, path scope, budget totals
   net of channel-stamped approvals) — a violating trace merges NOTHING;
   (b) per-store three-way merge; (c) policy.
5. **Zero-authorship default policy:** auto-promote iff no conflicts and
   every op is `add`/`modify`; any delete/move/rename or conflict parks
   for approval on the C2 surface (`asf approve … promotions|promote`).
   A branch-only sqlite (memory) change is a whole-store `modify` and
   auto-promotes — the agent updating its own memory every run is the
   zero-friction default. When StandingRules land (Stage 3), ratified
   promotion rules replace this constant in the same caveat vocabulary.
6. **Approval re-merges against current trunk.** A parked promotion
   stores only manifest + branch paths; apply-time recomputes the merge,
   so trunk movement between park and approval resolves trunk-wins —
   never wider than previewed, possibly narrower.
7. **Opaque stores merge whole-store (SI-18).** sqlite has no sub-file
   merge: branch-only change installs the branch image; both-changed is a
   conflict card, trunk wins, the branch image stays reachable in the CAS.

## Consequences

- Promoted changes remain revertible (`revert_to` a pre-run manifest
  rolls the promotion back out) — snapshot ancestry survives the merge.
- Branch directories persist after parked promotions so offline approval
  can apply them; CAS GC (future, ADR 0002 tripwires) must treat parked
  branches as roots.
- Concurrent sessions over the same stores are not yet coordinated —
  one session at a time is the Stage 3 dogfooding assumption.

## Session granularity — v0 decision (2026-07-09)

Promotion fires when the proxy session ends (`proxy.rs` — stdin EOF or a
catchable signal; sessions ended by SIGKILL are gated at the next bootstrap
or via `asf recover`, see RF-9), so one proxy process is one branch is one
promotion. A real MCP client keeps that process alive for its
whole app run, so **"one session" currently equals "one app lifetime"**: many
unrelated tasks pile onto one branch and promote as a single large merge on
quit. This is in tension with M6 (small manifests).

**Decision: accept coarse app-lifetime sessions for v0 dogfooding** rather
than build a finer session boundary now. Rationale: whether coarse sessions
actually hurt (unreadable merges, avoidable conflicts) is an empirical
question, and dogfooding is the instrument that answers it — build the fix
when the data shows the pain, not before. The candidate fix, if needed, is an
agent-callable `checkpoint` verb (+ `asf checkpoint` CLI) forcing a
step-boundary + promotion mid-connection.

**Tripwire (see `docs/dogfooding.md`):** track ops-per-promotion and conflict
incidence. If promotions become routinely large or conflict-prone, implement
the checkpoint boundary.
