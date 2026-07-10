# Coppice — Agent State Fabric

Reference implementation of the Agent State Fabric. Start with
[docs/agent-state-fabric-brief.md](docs/agent-state-fabric-brief.md) (why/what)
and [docs/asf-schema-spec.md](docs/asf-schema-spec.md) (the constitution —
it wins over everything else). Spec ambiguities found during implementation
are tracked in [docs/spec-issues.md](docs/spec-issues.md).

## Status

**Milestone 1 — Stage 1 kernel: dogfooding baseline complete.**
Canonical objects (JCS/RFC 8785, sha256 ids, Ed25519), append-only per-span
hash-chained trace substrate, payload store with per-payload DEKs and
logical shredding (normal resolution becomes tombstoned and unreadable),
snapshot coordinator over fs + sqlite Tier-1 roots with coherent multi-root
revert, step-boundary delegation manifests, drift attribution (single-human
default). Forensic erasure across SQLite WAL/freelists, filesystem snapshots,
and backups remains a post-dogfooding release gate; the current implementation
does not claim the spec's everywhere-at-once destruction guarantee.

**Milestone 2 — broker / capability evaluation: dogfooding baseline complete.**
Broker-minted capabilities (F1) with mandatory expiry and manifest binding
(M2), conjunctive caveat evaluation with unknown dimensions failing closed,
§5.2 attenuation subset-checking, tool registration with conservative
defaults (§4), run-window budget meters, A9 batch escalations, a
daemon-owned approval surface (C2: Unix socket + `asf approve`; the MCP
stream has no approval verb), credential injection that never touches the
ledger, mint-time M1 enforcement, and the MCP stdio proxy (`asf proxy`)
fronting a downstream server. ADR 0003 covers the hand-rolled passthrough.
The current proxy fails closed when the broker or proxy is unavailable. The
required loud, ledger-visible fail-open degradation path and per-capability
`on_broker_outage` inversion remain deferred until live-egress integration.

**Milestone 3 — promotion gate + branch topology: complete.**
Sessions run on a branch (materialized fork of the manifest's roots);
trunk changes only at promotion. The gate (§5.3): span verification →
trace-vs-capability re-check (a run that exceeded its token merges
nothing) → per-store three-way merge with trunk-wins conflicts and
add/modify/delete/move/rename operation classes (rename detection: a
reorganization never renders as mass deletion) → zero-authorship policy
(clean additive runs auto-promote; anything destructive or conflicted
parks for `asf approve`). Promoted changes remain revertible. Operator
commands: `asf revert / ledger / stats` (stats prints the ADR 0002
tripwire numbers). Property tests cover merge identities, conflict
soundness, glob-cover soundness (exhaustive), and the attenuation
semantic-subset property (ADR 0004, testing-theory G1).

Still open for later milestones: Tier-2/3 stores, judge/clerk,
StandingRules/TrustRecords, taint dimensions (currently fail closed by
design), `tools/list_changed` on mid-session re-manifest, forensic payload
erasure, and broker-outage degraded execution.

## Build & run

```sh
./scripts/ci required               # canonical offline gate: Clippy, tests, demos
./scripts/ci full                   # required gate + RustSec advisory audit
./scripts/ci deep                   # elevated property/model cases, release mode
./scripts/ci mutation               # scoped evaluator + promotion mutation testing
./scripts/install-hooks             # optional: run `required` before every push

cargo run -p asf -- demo            # milestone 1: kernel round-trip, narrated
cargo run -p asf -- broker-demo     # milestone 2: broker pipeline, narrated

# The real daemon topology (three terminals):
cargo run -p asf -- proxy --home /tmp/asf-home --vault /tmp/vault \
    --downstream target/debug/asf vault-server --vault /tmp/vault
cargo run -p asf -- approve --home /tmp/asf-home list      # the C2 surface
cargo run -p asf -- approve --home /tmp/asf-home approve 1 --uses 2
```

The repository-owned `scripts/ci` harness is the definition of a verified
change. GitHub Actions invokes the same lanes for Linux compatibility and
scheduled coverage; it does not carry a separate copy of the test commands.
The pre-push hook is intentionally deterministic and offline. `full` needs
`cargo-audit` 0.22.2, and `mutation` needs `cargo-mutants` 27.1.0; the harness
prints the pinned installation command when either is absent. Formatting is
not yet part of the gate because the existing tree is not rustfmt-clean.

`demo` proves the kernel's live behavior: manifest → traced mutations →
out-of-band edit surfacing as attributed drift → coherent two-store revert →
verified chains → logical payload tombstoning. It does not prove forensic
erasure from storage residue or backups. `broker-demo` proves the available
broker path: virtual-card grant →
checked calls → denials → one batched escalation for three violations →
channel-stamped approval with bounded uses → attenuation → fail-closed
unknown dimensions. It does not exercise broker-outage degradation.

## Layout

See [docs/adr/0001-language-and-repo-layout.md](docs/adr/0001-language-and-repo-layout.md)
for the language decision (Rust) and module map. Testing philosophy and the
deep-dive agenda live in [docs/testing-theory.md](docs/testing-theory.md).
Open implementation findings are tracked in
[docs/review-findings.md](docs/review-findings.md) (`RF-n`); spec ambiguities
in [docs/spec-issues.md](docs/spec-issues.md) (`SI-n`). Dogfooding setup and
methodology (corpus, wiring, metrics, graduation criteria) live in
[docs/dogfooding.md](docs/dogfooding.md).
