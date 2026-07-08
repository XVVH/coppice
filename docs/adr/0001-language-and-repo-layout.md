# ADR 0001 — Language and repo layout

**Status:** accepted — 2026-07-08

## Decision: Rust

Criteria from the milestone brief: single static daemon binary preferred; strong
sqlite + ed25519 + JCS/canonical-JSON story; low friction for an MCP proxy.

| Criterion | Rust answer |
|---|---|
| Single static binary | Native; musl target available if we ever need fully static Linux builds |
| sqlite | `rusqlite` with `bundled` sqlite (no system dependency, backup API included) |
| ed25519 | `ed25519-dalek` v2 (audited, the reference Rust implementation) |
| JCS (RFC 8785) | `serde_json_canonicalizer` (dedicated RFC 8785 crate) |
| MCP proxy (milestone 2) | `rmcp` — the official modelcontextprotocol Rust SDK |
| AEAD for payload DEKs | RustCrypto `aes-gcm` (AES-256-GCM) |

Go was the other serious candidate (stdlib ed25519, `gowebpki/jcs`, official Go
MCP SDK) and would have been acceptable; the tiebreakers were (a) Rust is
already installed on the dev machine, Go is not; (b) `rusqlite`/bundled sqlite
is a stronger story than pure-Go sqlite drivers; (c) the fabric is long-lived,
invariant-dense infrastructure where the type system pays for itself — the spec
is frozen, so Rust's slower iteration loop costs less than usual.

## Repo layout

```
Coppice/
  Cargo.toml               # workspace
  crates/
    asf-kernel/            # the library: everything Stage 1
      src/
        canon.rs           # JCS, object ids, signing (spec §0, §8.3)
        keys.rs            # key hierarchy, KEK/DEK wrap (spec §8.1, §8.2)
        payload.rs         # payload store, crypto-shredding (spec §1)
        trace.rs           # trace substrate (spec §6)
        snapshot.rs        # snapshot coordinator (brief §5.2)
        kernel.rs          # step-boundary manifests, drift, revert (spec §3)
    asf-cli/               # `asf` binary — demo / e2e driver; the broker
                           # daemon (`asfd`) becomes a sibling crate in M2
  docs/
    asf-schema-spec.md     # the constitution (v0.3)
    agent-state-fabric-brief.md
    spec-issues.md         # ambiguities found when code forced the question
    adr/
```

One library crate for the whole kernel, split only when the broker daemon
arrives (milestone 2). Modules map 1:1 to spec sections so spec-issues can cite
both a spec § and a file.

## Object representation

The authoritative form of every fabric object is its canonical JSON bytes.
Typed Rust structs are builders/views; verification always recomputes from raw
bytes so unknown fields are preserved and hashed (spec §0 extensibility rule).
