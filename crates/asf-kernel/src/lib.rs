//! Agent State Fabric — Stage 1 kernel.
//!
//! Read `docs/agent-state-fabric-brief.md` and `docs/asf-schema-spec.md`
//! (the constitution) before changing anything here. Interpretations of
//! spec ambiguities are tracked in `docs/spec-issues.md` as SI-n and cited
//! from the code they affect.

pub mod broker;
pub mod canon;
pub mod capability;
pub mod evaluate;
pub mod keys;
pub mod kernel;
pub mod payload;
pub mod promote;
pub mod snapshot;
pub mod tools;
pub mod trace;

/// RFC 3339 UTC timestamp for "now" (spec §0).
pub fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .expect("rfc3339 formatting cannot fail for utc")
}

/// Parse an RFC 3339 timestamp into a comparable instant; `None` if malformed.
///
/// All authority-relevant timestamp comparisons (expiry, `time` caveats,
/// attenuation `time` subset) MUST go through this. Lexical comparison of
/// RFC 3339 strings is wrong for mixed sub-second precision — `"…00Z"` sorts
/// *after* `"…00.5Z"` because `'Z' > '.'` — which fails **open** at a bound
/// (a call ~1s past `not_after`/expiry compares as within it). See RF-1.
/// Callers treat `None` as fail-closed.
pub fn parse_instant(s: &str) -> Option<time::OffsetDateTime> {
    time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok()
}
