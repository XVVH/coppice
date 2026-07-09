# ADR 0005 — Read authority (R2): constraints binding the future implementation

Status: accepted as **constraints** (2026-07-09). This ADR records what the
founding documents already commit us to, extracted while the note.list scope
discussion was fresh — it is NOT the implementation design. Implementation
is gated on the dogfooding graduation criterion: the R2 family lands before
the first `external_reach: live` tool joins a session.

## Why now

Dogfooding added enumeration (`note.list`, tool:vault@1.1) and immediately
raised "how is what-the-agent-sees constrained?" The temptation at this
moment is a proxy-local bandaid (a hardcoded list allowlist, a bespoke
scope knob). The founding documents answer the question already; this ADR
pins that answer so the eventual build starts from goals, not from code
convenience.

## What the founding documents commit us to

**1. The vocabulary is not ours to invent.** Spec §5.1 defines the R2
family: `read.scope` (what may be seen at all), `read.volume`
({max_records, max_bytes, max_pages, window} — the anti-bulk-exfiltration
meter), `read.sensitivity` ({max}, derived floors per §4), and
`taint.outbound` (deliverable content derived only from in-scope reads);
plus `taint.egress` on open surfaces ("vault-informed research without
vault exfiltration is this one caveat"). Implementation MUST implement
these dimensions, in this grammar — no proxy-local synonyms, no vocabulary
fork. Fail-closed-on-unknown-dims already protects version skew: a cap
carrying R2 caveats under an older evaluator denies rather than ignores.

**2. Zero authorship (brief §3 principle 9) forces the division of
labor.** Every dimension needs a no-configuration default derived from
registered metadata. That splits the R2 family into what is free and what
must be earned:

- **Free (derivable, ships as default): the taint wall.** The broker
  already traces every read — the session read-set is in the substrate by
  construction. "Outbound content derived only from benign sources" is a
  semi-mechanical data-flow check over that read-set (brief §5.4) needing
  zero user authoring. THE ANTI-EXFILTRATION PRIMITIVE IS THE TAINT WALL,
  NOT VAULT PARTITIONING.
- **Free: whole-store `read.scope`.** Registering the store IS the
  delegation act; the store boundary is the derivable scope default.
- **Free: egress-conditional `read.volume`.** Derivable from the session's
  registered tool surface: unbounded while every action is
  `side_effect: local`; a conservative bound auto-applies the moment an
  `external_reach: live` tool is in the grant.
- **Earned (ratchet-only): within-store partitions.** "May maintain
  projects/ but not see journal/" is authored policy. It may enter ONLY
  via the caveat ratchet — k ≥ 3 similar approvals, the least-general
  covering caveat, counterfactuals at ratification — never via a setup
  screen, a config file, or a hardcoded proxy allowlist. Dogfooding's job
  now: log every "I wish I could scope this" moment as a founding example.

**3. Attenuation stays mechanical (§5.2).** `read.scope` globs use the
A19 conservative dialect (subset only when provably so); `read.volume`
caps attenuate ≤, windows within; `read.sensitivity` no higher. Children
add dimensions, never widen.

**4. Enumeration is a read of metadata; names are content.**
`note.list` results MUST be governed by `read.scope` — filtered to scope
in the broker (the `filter_tools_result` pattern), broker-verified, never
delegated to the downstream. Out-of-scope existence must not leak through
error-shape differences (an out-of-scope read and a nonexistent path
should be indistinguishable to the agent). Known tension, resolved
conservatively: hidden paths cannot be asked about (JIT elicitation loses
one on-ramp) — confidentiality wins; widening happens at the human's
initiative.

**5. `read.sensitivity` waits for registered classifiers.** Derived
sensitivity requires the §4 provenance stamps and — per the A18 principle
that a heuristic class is a gameable class — registered, versioned
classifiers that rules can pin. Do not ship a heuristic sensitivity
dimension to feel complete; the dimension arrives with the clerk/judge
stage (brief roadmap stage 4).

**6. A3 stays named, not solved.** The taint wall covers the traced
read-set of file-shaped stores. The agent memory db launders provenance
across runs (brief: "the honest gap"); a session's outbound content can
derive from prior-run reads via memory with no stamp to check. R2
implementation MUST state this residual explicitly and MUST NOT claim the
wall closes it.

## Anti-goals (what a bandaid would look like)

- A read/list allowlist hardcoded in the proxy — authored policy smuggled
  into code, invisible to attenuation and the ledger.
- A proxy-only scope knob outside the §5.1 grammar.
- Partition prompts at setup ("which folders may the agent see?") — the
  design failure principle 9 names explicitly, regardless of how good the
  resulting policy would be.
- Trusting the downstream server to filter its own listings.
- A similarity-heuristic sensitivity class that rules can match.

## Consequences

- Workflows 1–2 (egress) remain gated on this family
  (`docs/dogfooding.md` graduation criteria).
- Until then, unscoped within-store reads are the correct, goal-faithful
  default — not a hole being tolerated.
- The dogfooding log is the collection instrument for partition founding
  examples; the ratchet, when built, compiles them.
