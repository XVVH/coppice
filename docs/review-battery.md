# The review battery — codified pre-review discipline

Authority-surface and spec-ratification changes in this repo go through
independent adversarial review (the CLAUDE.md rule). This document codifies the
**author-side battery** that runs *before* a change reaches that review, so the
review spends its cost on genuine soundness questions rather than on gaps a
checklist would have caught.

**Why this exists (founding case).** PR #50 (the SI-32 candidate) was drafted
after a self-administered "audit battery" plus one internal adversarial
subagent. An external review then found 8 issues, 5 of them the battery should
have killed: a code-fact claim that contradicted source (`capture_sqlite`
follows symlinks), three deliverables the originating filing *named* but the
draft dropped or hand-waved (directory rules, G-PUBLISH, active-window edits), a
referenced appendix that did not exist, and a branch based on the wrong parent.
Root cause: the battery lived in the author's head, was **collapsed into one
subagent** carrying five lenses at once, had **no artifact-hygiene tier**, and
handed **checklist tasks off as adversarial prose** — so enumeration got sampled
instead of enumerated. This is the same failure G9 fixed for test coverage
("green lanes still missed three PR #33 findings"), and the fix is the same:
make the battery a **required artifact plus a mechanical lane**, not a thing the
author remembers.

## The one structural rule

**Enumeration tasks produce tables; adversarial tasks produce findings; never
bundle them.** A completeness or code-fact pass must emit a row-per-item table,
because a missing or `OPEN` row is a *visible* miss a reviewer or a script can
catch — prose hides omissions, which is exactly how named deliverables get
sampled out. The adversarial pass stays free-form and is reserved for what it is
good at: soundness, tier-completeness, over/under-deciding. Enumeration and
adversary run as **separate passes**, not five bullets in one prompt.

## When the battery applies

Any PR that touches the spec, an ADR, the SI/RF/P/G ledgers as load-bearing
records, or any authority surface. Trivial docs edits and code-only changes
already covered by the two-sided contract discipline do not need Tier 2/3, but
Tier 1 runs on every push regardless (it is in `scripts/ci required`).

## Tier 1 — mechanical (enforced, cannot be skipped)

**Repo-state checks — `scripts/ci hygiene`** (`scripts/check-doc-hygiene`), wired
into `run_test` so it runs in `contracts`/`test`/`required`/`full`/`all` and in
GitHub Actions. Dependency-free POSIX shell; two-sided by construction
(`--self-test` proves each check fires on a broken fixture and passes on a good
one — the negative side; the real docs tree is the positive side). Mutation
testing does not apply (shell enforcement, not Rust — the two-sided contract
discipline's stated escape hatch). It checks:

1. **doc-refs** — every referenced `docs/<path>.md` and `ADR 0NNN` resolves to a
   file that exists. `roadmap.md` is exempt from *file-existence* refs only
   (by its own boundary rule it names planned and unmerged-branch artifacts);
   ADR-number refs are enforced everywhere.
2. **list-number** — within a section, a top-level ordered list runs 1,2,3,…; a
   reset to `1.` starts a new list; a duplicate or a skip fails.
3. **self-appendix** — a doc that names an "appendix" must contain an appendix
   heading (a referenced-but-absent inline section).

**PR-context checks — author + reviewer step** (need the PR as source of truth,
so they are required steps, not part of the local lane):

- **Declared scope matches the diff.** The PR body lists the files it means to
  touch; `git diff main...HEAD --stat` must match. A stray file (e.g. an
  unrelated ADR carried in because the branch was cut from the wrong parent) is
  a finding. The reviewer checks this first.
- **Branch based on current `main`.** `git merge-base --is-ancestor origin/main
  HEAD` after `git fetch`; no commits from another open PR ride along. Cut the
  branch from `main`, not from a sibling feature branch.
- **PR numbers, not SHAs, in docs** (the session-end git contract, applied to
  citations).

## Tier 2 — required PR artifacts (reviewer checks they are complete)

Every applicable PR carries these two tables in its body, the same way the G9
conformance sweep is already required. Their value is the **blank/OPEN row**: an
unaddressed item is visible instead of silently absent.

- **Completeness table** — one row per deliverable the originating filing
  (SI-n / RF-n / the roadmap item) named → the ADR/spec line that closes it, or
  an explicit `OPEN` / deferral with an owner. Extract the deliverables from the
  filing *first*, before drafting, so the draft is written against the list.
- **Code-fact table** — one row per "as-built" / `file:line` claim the artifact
  makes → confirmed-at-source or refuted, with the anchor. Every claim gets a
  row; a claim with no verified anchor does not ship as fact.

## Tier 3 — adversarial passes

- **Internal adversarial pre-review** (ratification candidates, before any paid
  external round): a fresh-context subagent briefed on the accumulated
  failure-mode profile, scoped to soundness / tier-completeness / shape
  (over/under-deciding) — *not* asked to also be the exhaustive checker; Tiers
  1–2 own that. Its findings are applied and recorded before the artifact is
  pushed.
- **Independent-context review** (the existing CLAUDE.md authority-surface rule):
  the external round. The battery's job is to make this round return only
  genuine soundness seams, not completeness or code-fact debris.

## Stopping rule — external rounds end by rule, not by momentum

The default for a ratification or authority-surface artifact is the battery
(Tiers 1–3) and **one** external round. A further round is bought only while
the latest round returned a blocking finding — a high-severity defect or a
finding against the determination skeleton — and each further round is
**scoped to the previous round's delta**, not the whole artifact (the PR #48
round-7 pattern). The first round that returns no blocking finding closes the
cycle; remaining mediums are folded by the author under the battery, without
a confirming round. Two corollaries:

- **New protocol questions never extend a cycle.** A finding whose remedy is
  protocol-class files an SI and queues at its own trigger (the R14/SI-39
  narrow-and-file precedent); the artifact under review narrows, and the
  cycle converges instead of tracking an open-ended frontier.
- **Buying past the rule is an operator gate decision**, recorded as one
  (with what the extra round is expected to find) — the same discipline as
  un-parking. It is never the silent default.

Founding case: PR #48 ran seven external rounds with no written stopping
condition — each next round was the default rather than a decision (adopted
2026-07-15, the heading check). The battery is where exhaustiveness lives
(the internal pre-review caught 13 findings at a fraction of a round's
cost); external rounds are for soundness seams.

## Running order

1. Extract the filing's deliverables into the completeness table (drives
   drafting).
2. Draft.
3. Fill the code-fact table by verifying each claim at source.
4. `scripts/ci hygiene` (and the PR-context checks against the intended diff).
5. Internal adversarial pre-review; apply findings.
6. Open the PR with both tables; independent-context review.

A round that still finds completeness or code-fact gaps means a Tier-1/2 pass
was skipped or run as prose — that is a process miss to name, not just a finding
to fix.
