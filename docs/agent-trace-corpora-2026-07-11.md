# Agent-trace corpora as ASF fixtures — survey and boundaries (2026-07-11)

> Research note backing **W-9 — foreign-trace corpus baseline**
> (candidate 2026-07-11; queued and DONE 2026-07-12, PR #29/#30 —
> baseline: `docs/baselines/w9-2026-07-12/`). Lanes 1+2 were W-9's
> scope; lane 3 measurement stays with the R2 trigger, lane 4 and the
> gate-replay throughput measurement with W-8's gate work.
> Every dataset fact below was verified against primary
> sources (HF dataset cards via API, GitHub repos, papers) on
> 2026-07-11; flags mark what could not be verified. This note records
> findings; the roadmap orders work.

## Why (and why not)

Public agent-trace corpora can exercise the **classifier surface and
the evaluator** — schema expressiveness, zero-authorship defaults,
containment vocabulary, operation classes — at a diversity dogfooding
cannot reach. Corpus-scale replay additionally provides the
performance baselines `scalability-analysis-2026-07-10.md` currently
reasons about abstractly — decision-time evaluator throughput and
end-to-end substrate ingest — before optimization pressure exists,
regressions visible after. (Promotion-gate replay throughput is NOT
part of W-9's measurement: the ingest lane is observed-mode and never
branches or promotes; that measurement lands with W-8's gate work,
where the closure semantics it must exercise live.)

They **cannot** validate the ratification loop, and must not feed it:
R1 requires k ≥ 3 founding examples, domain-matched (R6), from the
principal's own ratified history. Corpus examples are fixtures and
calibration inputs, never founding examples — seeding standing rules
from a dataset would be policy authored upfront: a zero-authorship
violation with a dataset as the author.

The gap that flips the premise: **no public corpus has longitudinal,
same-principal traces with real approve/deny events** (searched
2026-07-11; the closest is OpenHands' 275 consented sessions with
session-level thumbs ratings). Session logs with approval prompts
exist on millions of machines (Claude Code, Cursor, Cline); none are
published consented. The W-1 ledger is accumulating a data class that
does not exist publicly.

## The four lanes

1. **Zero-authorship default census** (spec §2 extensibility, §5.1).
   Run the real MCP tool-schema universe (measured: 9,795 after
   identity guards — MCP-Flow's four seen-test sets + Toucan's executed
   tools; the unseen sets can extend it on demand) through registration
   derivation: what fraction derives a
   domain / reversibility class / egress flag from registered metadata
   alone, and what fraction falls to the conservative floor
   (irreversible + egress + fail-closed)? That percentage is the
   approval-fatigue forecast, and the first empirical input the
   domain-taxonomy open problem has had. Input only — taxonomy
   governance stays open and named.
2. **Gate-replay conformance fuzzing** (W-2 alignment). One converter:
   foreign trajectory → manifest + `register` + `tool_call` events →
   replay through the decision-time evaluator under default probation
   caveats. Every event shape the schema cannot encode files as an SI
   (never silently interpreted). This lane is also the perf-baseline
   lane: corpus-scale replay through the decision-time evaluator plus
   end-to-end substrate ingest (gate-replay throughput deferred to
   W-8's gate work).
3. **Containment replay** (taint-wall evidence). AgentDojo / InjecAgent
   injection cases and MCPHunt canary-propagation traces through
   broker policy. Two-sided, same discipline as the contracts
   registry: injected actions blocked AND their benign twins pass — a
   public denial-FP number, the statistic W-1 tracks privately.
4. **Operation-class corpus** (A13/A17). SWE trajectories as real
   add/modify/delete/move/rename sequences: rename-detection fixtures
   (a reorganization must never render as mass deletion) and
   constructed trunk-divergence merge cases for the promotion gate.
   Vault maintenance is fs-shaped; SWE traces are its structural
   cousin.

## Verified shortlist

| Corpus | Size / license | ASF use |
|---|---|---|
| [Toucan-1.5M](https://huggingface.co/datasets/Agent-Ark/Toucan-1.5M) | 1.65M trajectories, 495 real MCP servers, 2,000+ tools; Apache-2.0 | Lanes 1+2 first target — MCP-native, same wire format the broker proxies |
| [MCP-Flow](https://github.com/wwh0411/MCP-Flow) | 1,166 servers / 11,536 tool schemas; **no license declared** | Lane 1 schema breadth (flag before any redistribution) |
| [MCPMark trajectory logs](https://huggingface.co/datasets/Jakumetsu/mcpmark-trajectory-log) | ~17 models × 5 real MCP services × 127 tasks, executed, with programmatic state verification; MIT | Real execution exhaust — rate-safe; lane 2 |
| [MCPHunt traces](https://huggingface.co/datasets/lihaonan0716/mcphunt-agent-traces) | 3,615 traces with canary-data cross-boundary propagation labels (2026-04); CC-BY-4.0 | Lane 3 — near ready-made `taint.egress` benchmark |
| [AgentDojo](https://github.com/ethz-spylab/agentdojo) | 97 user tasks + 629 injection cases with env state; MIT | Lane 3 two-sided containment (utility AND security checks per case) |
| [InjecAgent](https://github.com/uiuc-kang-lab/InjecAgent) | 1,054 injection cases; MIT | Lane 3 breadth |
| [SWE-smith trajectories](https://huggingface.co/datasets/SWE-bench/SWE-smith-trajectories) | ~26k episodes, tool calls + observations + final patches; MIT | Lane 4 operation classes / rename detection |
| [nebius/SWE-agent-trajectories](https://huggingface.co/datasets/nebius/SWE-agent-trajectories) | 80k episodes incl. failures, with eval logs; CC-BY-4.0 | Lane 4 scale; rate-safe (outcome-labeled) |
| [TheAgentCompany runs](https://github.com/TheAgentCompany/experiments) | 175 office-work tasks × many agents (GitLab/ownCloud/RocketChat); license unstated in experiments repo | Closest domain match to the wedge workflows; in-chat "humans" are LLM NPCs |
| [Agent Data Protocol](https://github.com/neulab/agent-data-protocol) | 13 datasets unified → 1.3M trajectories, one Pydantic schema (CMU, 2025-10); MIT | One ADP converter buys the long tail once the first converter exists |
| [TRAIL](https://huggingface.co/datasets/PatronusAI/TRAIL) | 148 OTel-format traces, 841 taxonomized errors; MIT (gated, auto-approve) | Error-taxonomy input for `verdict`/`drift` design |
| [OpenHands feedback](https://huggingface.co/datasets/OpenHands/openhands-feedback) | 275 consented real-user sessions with interleaved human turns; MIT | The entire public supply of consented human-in-the-loop traces |

Adjacent, below the line: BFCL v3/v4 (eval data, simulated stateful
envs), Nemotron-Agentic v1/v2 (huge but LLM-simulated tool env),
xLAM/APIGen-MT (NC license), AgentNet (22.6k human GUI demos, MIT),
WebLINX (human-human dialogue, NC), HAL leaderboard rollouts (21,730 —
encrypted, unlicensed). Standardization watch: OTel GenAI semconv
(agent/tool spans) is still Development-stability as of mid-2026 —
relevant to W-4's protocol-neutral delegation-lifecycle events; ADP is
the research-grade schema worth converging with today.

## Boundaries (hard)

- Fixtures and calibration only; **never founding examples** (R1, R6).
- Synthetic corpora (Toucan, Nemotron, ToolACE) are coverage material;
  rate-shaped claims come only from executed sets (MCPMark, SWE
  trajectory sets, AgentDojo runs).
- Environment state is absent from nearly every corpus (SWE sets ship
  final diffs only) → snapshot/revert semantics stay dogfooding-proven.
- Licenses: AgentHarm is eval-only (no training use); APIGen-MT and
  WebLINX are NC; HAL rollouts unlicensed. Internal analysis is fine;
  recheck at W-6 before anything derived is published.
- Two-surface hygiene: corpora live operator-side (dev repo /
  scratch). Nothing is ingested into the dogfood workspace or Hermes
  memory — A3 is exactly the taint class importing foreign traces
  would create.
- Offline, no egress, no actuation-scoped tool registration — no
  SI-23 conflict.

## Sequencing sketch

Toucan slice through one converter feeding lanes 1+2 (shared plumbing,
near-free MCP mapping) → AgentDojo + MCPHunt containment replay
(lane 3) → SWE-smith operation-class fixtures (lane 4) when gate work
next opens. ADP conversion when breadth is wanted. Each lane emits its
findings to the proper tracker (SIs for schema gaps, testing-theory
for fixture gaps); this note does not accumulate results.
