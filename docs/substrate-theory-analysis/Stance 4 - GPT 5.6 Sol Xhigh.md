Verdict: **REQUEST CHANGES.** The core strategic error is conflating technical replaceability with adoption, and format survival with value capture. F2 is currently not falsifiable by the dogfood program.

| Item | Verdict | Strongest attack |
|---|---|---|
| [C10](/Users/josh/dev/Coppice/docs/substrate-theory.md:178) | **Refute** | The boundary has a legacy constituency: today’s bash-first harnesses, arbitrary subprocesses, credential conventions, filesystem layouts, and network assumptions. A microVM can preserve POSIX, but a broker-only door cannot preserve ambient authority without ecosystem adoption. |
| [C12](/Users/josh/dev/Coppice/docs/substrate-theory.md:204) | **Refute strategic inference** | OCI proves formats can survive their authors. It does not prove their authors capture the resulting value. Docker’s history is at least as much a warning as a precedent. |
| [F1](/Users/josh/dev/Coppice/docs/substrate-theory.md:235) | **Refine** | It tests semantic impossibility when economic refusal is enough to kill C10. The boundary can be replaceable yet remain owned by labs and clouds. |
| [F2](/Users/josh/dev/Coppice/docs/substrate-theory.md:239) | **Currently untestable** | Dogfooding measures denial false positives, while standing grants cannot yet be widened. “The user did not widen” is therefore mechanically predetermined. |
| [F5](/Users/josh/dev/Coppice/docs/substrate-theory.md:250) | **Refute as stale/misspecified** | Vertically integrated substrates already exist. They do not need proprietary edge protocols: vendors can embrace MCP/A2A while keeping identity, policy, history, and enforcement proprietary. |
| [C11](/Users/josh/dev/Coppice/docs/substrate-theory.md:190) | **Refine** | Before manifest-as-principal, this is an appliance built on existing microVM and policy machinery, not a separately purchasable OS category. The note has not identified its buyer. |

## C10/F1 — harness gravity is boundary loyalty

OpenAI’s own Windows sandbox work is almost a direct falsification of C10’s economic premise: it found AppContainer’s capability model the wrong shape because Codex must drive shells, Git, Python, package managers, and arbitrary binaries. Anthropic likewise preserved bash and arbitrary subprocesses, adding transparent filesystem and network mediation around them. These are not merely tools “inside” the boundary; their assumptions define what the boundary must transparently reproduce. [OpenAI’s Windows sandbox analysis](https://openai.com/index/building-codex-windows-sandbox/), [Anthropic’s sandbox architecture](https://www.anthropic.com/engineering/claude-code-sandboxing).

Who must adopt ASF’s stronger boundary:

- **Labs and runtime operators** must bind launches to manifests, force external effects through the broker, and make their native approval/history systems subordinate to portable records. They want safer autonomy and enterprise sales, but have a contrary incentive against portable trust that weakens their control.
- **Tool and CLI authors** must expose structured action, target, reversibility, and credential semantics rather than rely on raw shell/network access. Their incentive is distribution, but MCP gateways already offer it with less schema burden.
- **Enterprise platform teams** must map IAM/Entra identities, secrets, audit, and incident procedures onto manifests. Their switching costs are substantial, and their incumbent vendor will offer a bundled mapping first.
- **Developers** must accept that some familiar shell workflows cannot receive ambient credentials or unrestricted network access. Fewer prompts are valuable; broken automation is not.

A better F1 is:

> If representative coding, research, and SaaS workflows cannot run through the broker without raw ambient credentials, unclassified shell effects, or routine bypass—and integrated vendor sandboxes deliver acceptable autonomy without manifest semantics—then the ASF boundary has lost economically even if it remains technically implementable.

The current vault-only dogfood, which forbids shell on the agent surface, cannot test this.

## C12 — OCI supports portability, not capture

Docker donated an already dominant technology: the OCI announcement cited more than 500 million image downloads and 40,000 public projects before neutralization. Distribution created the standard; kernel-friendly schema design did not create distribution. [OCI’s 2015 announcement](https://opencontainers.org/posts/announcements/2015-06-20-industry-leaders-unite-to-create-project-for-open-container-standard/).

Docker Hub also had a stronger accumulation story than ASF currently claims: publishers, consumers, official images, automation, namespaces, and millions of searchable artifacts. Yet cloud-attached registries and orchestrators could capture workloads while consuming the same format. Docker later sold its enterprise platform business to Mirantis. [Docker Hub](https://hub.docker.com/search), [Mirantis acquisition announcement](https://www.globenewswire.com/news-release/2019/11/13/1946550/0/en/Mirantis-Acquires-Docker-Enterprise-Platform-Business.html).

The trust ledger is weaker as a network asset:

- Records are private, user-specific, domain-scoped, and behavior-version-specific.
- Another customer’s history does not improve mine.
- Exportability deliberately lets the accumulated history leave with the customer.
- If independent runtimes can verify it, hosting becomes more substitutable, not less.

That is excellent trust architecture and poor evidence of a moat. A real accumulating asset would need something like relying-party recognition, portable tool attestations, insurer/auditor acceptance, cross-vendor reputation verification, or uniquely calibrated risk infrastructure. “Hosted storage plus UX” is bundleable by the clouds.

C12 should therefore say: **formats are the distribution and neutrality strategy; they are not the value-capture strategy.** The latter needs a separate claim and falsifier.

## F5 — the vertical race has already started

The credible field is:

| Candidate | Existing position | Format incentive |
|---|---|---|
| **Microsoft** | MXC supplies OS-enforced agent containment, local/Entra agent identity, Agent 365 governance, and Foundry hosting. | Open MCP/A2A for ecosystem ingress; retain authority and telemetry in Entra, Intune, Purview, Agent 365, and Foundry. [Windows agent platform](https://developer.microsoft.com/en-us/windows/agentic) |
| **AWS** | AgentCore already combines per-session microVMs, workload identity, gateway, policy, credential custody, memory, and tracing. | Support any framework/model plus MCP/A2A while making IAM, Cedar, ARNs, and AgentCore the authoritative control plane. [AgentCore Runtime](https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/agents-tools-runtime.html) |
| **Google** | Vertex Agent Engine combines managed runtime, sessions, memory, code execution, observability, and per-agent identity. | Promote open A2A/ADK while retaining Google IAM, gateway, runtime state, and Cloud telemetry. [Agent Engine](https://cloud.google.com/vertex-ai/generative-ai/docs/reasoning-engine/overview), [Agent Identity](https://docs.cloud.google.com/gemini-enterprise-agent-platform/govern/agent-identity-overview) |
| **OpenAI** | Codex already spans local/cloud sandboxes, approvals, network policy, and agent-native telemetry. | Support portable tools while keeping task history, approval policy, and execution controls attached to Codex. [Codex deployment controls](https://openai.com/index/running-codex-safely/) |
| **Anthropic** | Claude Code has sandboxed bash, credential proxies, permissions, checkpoints, and MCP distribution. | Keep MCP neutral while retaining Claude-specific history, checkpoint, and permission semantics. [Claude sandboxing](https://www.anthropic.com/engineering/claude-code-sandboxing) |
| **Apple** | Xcode hosts multiple agents through MCP/ACP; Apple controls the OS sandbox and App Intents action surface. | Open agent ingress, proprietary OS authority and app-action semantics. [Xcode agent integration](https://www.apple.com/newsroom/2026/06/apple-aids-app-development-with-new-intelligence-frameworks-and-advanced-tools/) |
| **Red Hat / Canonical / NVIDIA** | Kagenti AgentRuntime/AuthBridge and OpenShell-on-Ubuntu are already forming a Linux-native alternative. | Likely allies for neutral formats, but may standardize on Kubernetes, SPIFFE, OCI, and OpenShell and treat ASF as redundant. [Red Hat AgentRuntime](https://developers.redhat.com/articles/2026/04/14/deploying-agents-red-hat-ai-openclaw), [Ubuntu OpenShell](https://canonical.com/blog/nvidia-openshell-ubuntu-announcement) |

Thus F5’s “before a vertical substrate ships” window is closed. The remaining window is roughly **6–12 months to enter the neutral authority/governance standards conversation**, not several years to invent a standalone format. MCP already reports over 10,000 public servers, and A2A has broad cloud adoption; those are the channels with distribution gravity. [MCP’s foundation donation](https://www.anthropic.com/news/donating-the-model-context-protocol-and-establishing-of-the-agentic-ai-foundation), [A2A adoption](https://www.linuxfoundation.org/press/a2a-protocol-surpasses-150-organizations-lands-in-major-cloud-platforms-and-sees-enterprise-production-use-in-first-year).

A testable replacement for F5:

> If by 2027-06-30 fewer than two independent runtimes natively emit ASF records, fewer than two independent gates authorize from them, or no external relying party accepts portable trust across vendor boundaries, ASF has not accumulated format network effects. Export adapters and nominal schema support do not count.

## F2 — a 90-day disconfirming observation

Current dogfooding cannot test F2. Its primary metric is denial false-positive rate, not delegated authority, and every grant remains session-scoped until the ratchet exists. [Current metrics](/Users/josh/dev/Coppice/docs/dogfooding.md:116), [standing grants unavailable](/Users/josh/dev/Coppice/docs/dogfooding.md:165).

A minimally observable 90-day test would:

1. Pre-register three recurring workflow families and a delegation-frontier vector: path/action scope, grant duration, unattended runtime, auto-promotion, and approvals per successful operation.
2. Record the operator’s actual maximum grant for the next run before histories accrue.
3. Accumulate at least three clean runs and one successful forced restore per family.
4. Present the evidence and offer the least-general standing grant covering those runs.
5. Collect at least 24 actual accept/narrow/reject decisions, keeping behavior version and task family fixed.

Operational disconfirmation is: after clean histories and demonstrated recovery, the median frontier does not widen on any dimension, approval burden does not fall, and offers are repeatedly rejected for risks that recovery does not address—exfiltration, correctness, social consequences, or accountability.

That falsifies the loop for the design-center operator. It does **not** establish causality or generalize to a market; that would require a staged or randomized multi-user rollout. The note should distinguish those two claims.

## C11’s buyer

Before manifest-as-principal, nobody plausibly buys an “agent OS” as such:

- The solo operator buys an application or managed service.
- Labs and hyperscalers build the runtime because it is strategically controlling.
- Enterprise platform teams buy a certified control plane or appliance, but already possess VM/container pipelines, IAM, policy, secrets, observability, and incident tooling.

The switching cost from Firecracker-plus-policy is therefore not the guest image. It is migration of identities, policy, credential flows, network controls, logs, compliance evidence, deployment pipelines, and operational ownership. Firecracker already provides the microVM isolation boundary and explicitly leaves egress filtering to the host, making ASF a plausible control-plane addition without runtime replacement. [Firecracker design](https://github.com/firecracker-microvm/firecracker/blob/main/docs/design.md).

C11 is defensible only as **an ASF reference appliance or W-4 backend**. Its unique buying reason must be coherent cross-store recovery plus portable trust; without that demonstrated advantage, “agent-runtime OS” is packaging around an already commoditizing stack.
