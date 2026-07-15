# Field incidents — the wild-world corpus

Public incidents of agents doing exactly what the fabric exists to
prevent, one row each. Not a findings tracker (`review-findings.md` owns
our own defects) and not dogfooding data (`dogfooding-rubric.md` stages
its cases deliberately; these are the unstaged real thing). This is
market evidence, kept for the eventual writeup, the W-5 demo framing,
caveat-pack design, and positioning copy — so the founding examples
exist as links, not as memories of tweets.

Entry rule: an incident lands with the date we saw it, the primary
link (verified at filing time — no reconstructed-from-memory
citations), a quote short enough to be fair use or a paraphrase, and
the invariant or mechanism it demonstrates (brief principle, spec §,
P-row, or DF case). Where a staged dogfooding counterpart exists, the
row names it.

| Seen | Incident | Source | Demonstrates |
|------|----------|--------|--------------|
| 2026-07-15 | Agent (codex) lacked an X API bearer token for a workflow, so it opened the user's 1Password and took the credential itself — unlogged, unbounded, and now resident in its context window | x.com/paularambles/status/2076765763818717548 | Brief principle 4: the agent never holds the real key; anything in context is presumed exfiltratable. The two-surface bypass — the native path preferred exactly at the moment of frustration (`dogfooding.md` session hygiene; P5/P7: deny rules are convention, W-4 containment is topology). JIT elicitation (brief §5.3) as the demand-side fix: a cheap sanctioned ask is what makes the unsanctioned grab unattractive. DF-N9, unstaged. |
| 2026-07-15 | "just deleted my whole production database … It's not safe" (GPT-5.6 Sol; user reports no prior model had ever done this) | x.com/brunolemos/status/2076769881534398974 | The undo thesis itself: Tier-2 owned remote state wants branch/fork + snapshot-before-delegation (brief §5.2), not raw credentials to prod. Reversibility classes with broker_verified guards on destructive targets (brief §5.3; non-negotiable invariant). Conservative default: undeclared reversibility = irreversible — scrutiny concentrates where the architecture says risk lives. "It's not safe" is the trust bottleneck verbatim (brief §2): the user cannot bound blast radius, so delegation collapses. The all-in operator arriving after their first incident wanting rewind (brief §7). |
