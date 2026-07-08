//! Agent State Fabric — Stage 1 kernel.
//!
//! Read `docs/agent-state-fabric-brief.md` and `docs/asf-schema-spec.md`
//! (the constitution) before changing anything here. Interpretations of
//! spec ambiguities are tracked in `docs/spec-issues.md` as SI-n and cited
//! from the code they affect.

pub mod canon;
pub mod keys;
pub mod payload;
pub mod snapshot;
pub mod trace;
pub mod kernel;

/// RFC 3339 UTC timestamp for "now" (spec §0).
pub fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .expect("rfc3339 formatting cannot fail for utc")
}
