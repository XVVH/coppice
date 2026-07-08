# Coppice — Agent State Fabric

Reference implementation of the Agent State Fabric. Start with
[docs/agent-state-fabric-brief.md](docs/agent-state-fabric-brief.md) (why/what)
and [docs/asf-schema-spec.md](docs/asf-schema-spec.md) (the constitution —
it wins over everything else). Spec ambiguities found during implementation
are tracked in [docs/spec-issues.md](docs/spec-issues.md).

## Status

**Milestone 1 — Stage 1 kernel: complete.**
Canonical objects (JCS/RFC 8785, sha256 ids, Ed25519), append-only per-span
hash-chained trace substrate, payload store with per-payload DEKs and
crypto-shredding, snapshot coordinator over fs + sqlite Tier-1 roots with
coherent multi-root revert, step-boundary delegation manifests, drift
attribution (single-human default).

**Milestone 2 — broker / capability evaluation: not started.** Manifests
carry no `authority` yet (see SI-7); M1/M2/C1–C5/attenuation tests activate
there.

## Build & run

```sh
cargo test --workspace         # unit + invariant + e2e tests
cargo run -p asf -- demo       # narrated kernel round-trip in a temp dir
cargo run -p asf -- demo /tmp/asf-home   # …kept on disk for inspection
```

The demo: registers principals and a `local_session` channel, captures a
signed intent, manifests at a step boundary, runs a traced vault+memory
mutation, re-manifests, injects an out-of-band human edit (surfaces as
`human_local` drift), reverts both stores coherently to manifest 1, verifies
every hash chain and signature, prints the ledger, and crypto-shreds the
intent text.

## Layout

See [docs/adr/0001-language-and-repo-layout.md](docs/adr/0001-language-and-repo-layout.md)
for the language decision (Rust) and module map.
