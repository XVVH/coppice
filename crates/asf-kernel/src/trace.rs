//! Trace substrate. Spec §6, brief §5.1.
//!
//! Append-only sqlite event log: hash-chained per span (`prev` + `seq`),
//! signed by the emitting component (Stage 1: the fabric key, SI-3). The
//! global rowid is the substrate offset used by `captured_before` (§3.1) and
//! `trace.substrate_offset` (§3). Append-only is enforced in-database with
//! triggers; verification recomputes ids/signatures and cross-checks every
//! materialized selector, so edits to retained signed rows are detectable even
//! if triggers are bypassed. SI-25/RF-13 track tail/whole-span deletion,
//! rollback, and authenticated global order/head.
//!
//! The substrate also stores fabric objects (principals, channels, intents,
//! manifests) — brief §5.1 counts these among the records the fabric itself
//! emits and fully owns.

use crate::canon::{self, CanonError};
use ed25519_dalek::{SigningKey, VerifyingKey};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{Map, Value};

#[derive(Debug, thiserror::Error)]
pub enum TraceError {
    #[error("sqlite: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("canon: {0}")]
    Canon(#[from] CanonError),
    #[error("unknown event kind {0}")]
    UnknownKind(String),
    #[error("span {span} chain broken at seq {seq}: {detail}")]
    ChainBroken {
        span: String,
        seq: i64,
        detail: String,
    },
    #[error("signed event {id} has malformed `{field}`")]
    MalformedEventField { id: String, field: &'static str },
    #[error(
        "event {id} index column `{field}` disagrees with the signed object (RF-16; fail closed)"
    )]
    EventIndexMismatch { id: String, field: &'static str },
    #[error("ledger integrity failure: {0}")]
    LedgerIntegrity(String),
    #[error("object {0} not found")]
    ObjectNotFound(String),
    #[error("object row {requested} contains signed object {signed} (RF-19; fail closed)")]
    ObjectIdMismatch { requested: String, signed: String },
    #[error("object {id} has prefix {actual}, expected {expected} (RF-19; fail closed)")]
    ObjectPrefixMismatch {
        id: String,
        expected: String,
        actual: String,
    },
    #[error("object {id} has stored kind {actual}, expected {expected} (RF-19; fail closed)")]
    ObjectKindMismatch {
        id: String,
        expected: String,
        actual: String,
    },
}

/// Spec §6 event kinds, plus Stage-1 extensions flagged as SI-11:
/// `register` (principal/channel/tool object registration) and
/// `intent` (intent capture — the substrate countersign of SI-10).
/// A22 (§5.4): `revoke` is the signed early-closure dual of `grant`; the
/// never-emitted `expiry` kind left the spec in the same amendment —
/// `expires_at` in the signed object is the sole time closure.
pub const EVENT_KINDS: &[&str] = &[
    "tool_call",
    "verdict",
    "escalation",
    "ratification",
    "promotion",
    "revert",
    "compensation",
    "grant",
    "revoke",
    "snapshot",
    "drift",
    "shred",
    "amendment",
    "remanifest",
    // SI-11 extensions (Option A ratified 2026-07-08):
    "register",
    "intent",
    // SI-13 extension: escalation resolution (C1-stamped human approval/denial).
    "approval",
];

pub fn init(conn: &Connection) -> Result<(), TraceError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS events (
            offset   INTEGER PRIMARY KEY AUTOINCREMENT,
            id       TEXT NOT NULL UNIQUE,
            span     TEXT NOT NULL,
            seq      INTEGER NOT NULL,
            prev     TEXT,
            manifest TEXT,
            at       TEXT NOT NULL,
            kind     TEXT NOT NULL,
            raw      TEXT NOT NULL,
            UNIQUE (span, seq)
        );
        CREATE INDEX IF NOT EXISTS events_span ON events (span, seq);
        CREATE TRIGGER IF NOT EXISTS events_append_only_u
            BEFORE UPDATE ON events
            BEGIN SELECT RAISE(ABORT, 'trace substrate is append-only'); END;
        CREATE TRIGGER IF NOT EXISTS events_append_only_d
            BEFORE DELETE ON events
            BEGIN SELECT RAISE(ABORT, 'trace substrate is append-only'); END;
        CREATE TABLE IF NOT EXISTS objects (
            id         TEXT PRIMARY KEY,
            kind       TEXT NOT NULL,
            created_at TEXT NOT NULL,
            raw        TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS meta (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    )?;
    Ok(())
}

/// Mint a fresh span id.
pub fn new_span() -> String {
    let mut b = [0u8; 16];
    use rand::RngCore;
    rand::rngs::OsRng.fill_bytes(&mut b);
    format!("span:{}", hex::encode(b))
}

#[derive(Debug, Clone)]
pub struct Appended {
    pub id: String,
    pub offset: i64,
    pub span: String,
    pub seq: i64,
}

/// Current head of the global substrate (max offset; 0 if empty).
pub fn head_offset(conn: &Connection) -> Result<i64, TraceError> {
    Ok(conn.query_row(
        "SELECT COALESCE(MAX(offset), 0) FROM events",
        [],
        |r| r.get(0),
    )?)
}

/// Append a signed event to a span's chain. `seq` starts at 0 with
/// `prev: null` (SI-4); `manifest` is nullable for substrate-level events
/// (SI-5). The whole operation is one transaction: chain-tip read and insert
/// cannot interleave.
pub fn append(
    conn: &mut Connection,
    sk: &SigningKey,
    span: &str,
    manifest: Option<&str>,
    kind: &str,
    body: Value,
    at: &str,
) -> Result<Appended, TraceError> {
    let tx = conn.transaction()?;
    let appended = append_in_tx(&tx, sk, span, manifest, kind, body, at)?;
    tx.commit()?;
    Ok(appended)
}

/// Append within a caller-owned SQLite transaction. State-changing callers
/// use this to make the signed event, expected-root tuple, and mutable caches
/// one database commit; the caller remains responsible for committing.
pub fn append_in_tx(
    tx: &Transaction<'_>,
    sk: &SigningKey,
    span: &str,
    manifest: Option<&str>,
    kind: &str,
    body: Value,
    at: &str,
) -> Result<Appended, TraceError> {
    if !EVENT_KINDS.contains(&kind) {
        return Err(TraceError::UnknownKind(kind.into()));
    }
    let tip: Option<(String, i64)> = tx
        .query_row(
            "SELECT id, seq FROM events WHERE span = ?1 ORDER BY seq DESC LIMIT 1",
            [span],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (prev, seq) = match tip {
        Some((prev_id, prev_seq)) => (Value::String(prev_id), prev_seq + 1),
        None => (Value::Null, 0),
    };

    let mut obj = Map::new();
    obj.insert("span".into(), Value::String(span.into()));
    obj.insert("seq".into(), Value::from(seq));
    obj.insert("prev".into(), prev.clone());
    obj.insert(
        "manifest".into(),
        manifest.map(|m| Value::String(m.into())).unwrap_or(Value::Null),
    );
    obj.insert("at".into(), Value::String(at.into()));
    obj.insert("kind".into(), Value::String(kind.into()));
    obj.insert("body".into(), body);
    let sealed = canon::seal("evt", obj, sk)?;
    let id = sealed["id"].as_str().expect("sealed").to_string();
    let raw = serde_json::to_string(&Value::Object(sealed)).expect("serialize");

    tx.execute(
        "INSERT INTO events (id, span, seq, prev, manifest, at, kind, raw)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            id,
            span,
            seq,
            prev.as_str(),
            manifest,
            at,
            kind,
            raw
        ],
    )?;
    let offset = tx.last_insert_rowid();
    Ok(Appended {
        id,
        offset,
        span: span.into(),
        seq,
    })
}

/// A decoded event row.
#[derive(Debug, Clone)]
pub struct EventRow {
    pub offset: i64,
    pub id: String,
    pub span: String,
    pub seq: i64,
    pub prev: Option<String>,
    pub manifest: Option<String>,
    pub at: String,
    pub kind: String,
    pub raw: Value,
}

/// An event whose signed raw object has been verified and whose every
/// denormalized SQLite column agrees with that object.
///
/// `offset` remains substrate metadata until SI-25 defines authenticated
/// global order. Every other field below is sourced from signed `raw`, never
/// from the row index. Authority consumers accept this type, not `EventRow`.
#[derive(Debug, Clone)]
pub struct VerifiedEvent {
    pub offset: i64,
    pub id: String,
    pub span: String,
    pub seq: i64,
    pub prev: Option<String>,
    pub manifest: Option<String>,
    pub at: String,
    pub kind: String,
    pub raw: Value,
}

/// One SQLite statement snapshot of the event rows and the head observed by
/// that exact statement. The head is still unsigned pending SI-25, but it
/// cannot race ahead of the event set used to reconstruct decision-time
/// authority.
#[derive(Debug, Clone)]
pub struct VerifiedEventSnapshot {
    pub events: Vec<VerifiedEvent>,
    pub head: i64,
}

/// One retained-row anomaly found by the operator ledger's diagnostic view.
///
/// `offset` and `span` are locations for forensic inspection, not authenticated
/// claims. SI-25 remains responsible for authenticating global offset order,
/// completeness, rollback, and freshness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceIntegrityFinding {
    pub offset: Option<i64>,
    pub span: Option<String>,
    pub detail: String,
}

impl std::fmt::Display for TraceIntegrityFinding {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.offset, self.span.as_deref()) {
            (Some(offset), Some(span)) => {
                write!(formatter, "offset {offset}, span location {span}: {}", self.detail)
            }
            (Some(offset), None) => write!(formatter, "offset {offset}: {}", self.detail),
            (None, Some(span)) => write!(formatter, "span {span}: {}", self.detail),
            (None, None) => formatter.write_str(&self.detail),
        }
    }
}

/// Operator-facing retained-row view.
///
/// Unlike [`verified_events`], which deliberately omits wholly unsigned or
/// foreign-signed rows because they carry no authority, this diagnostic view
/// preserves every decodable anomaly as a finding; an incompatible SQLite
/// storage type rejects construction before accounting. `events` contains
/// individually signature- and selector-verified records for forensic display;
/// callers MUST call [`LedgerEventView::w19_require_clean`] before using them
/// for accounting.
#[derive(Debug, Clone)]
pub struct LedgerEventView {
    pub events: Vec<VerifiedEvent>,
    pub findings: Vec<TraceIntegrityFinding>,
    pub records: Vec<LedgerStoredRecord>,
}

/// Exact raw text observed for one retained row in the same SQLite statement
/// snapshot as [`LedgerEventView::events`] and [`LedgerEventView::findings`].
#[derive(Debug, Clone)]
pub struct LedgerStoredRecord {
    pub offset: i64,
    pub raw: String,
}

impl LedgerEventView {
    /// Stable W-19 enforcement predicate: no diagnostic accounting or
    /// drift-attribution write may proceed while any retained-row doubt exists.
    pub fn w19_require_clean(&self) -> Result<(), TraceError> {
        if self.findings.is_empty() {
            return Ok(());
        }
        Err(TraceError::LedgerIntegrity(
            self.findings
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; "),
        ))
    }
}

#[derive(Debug, Clone)]
struct W19StoredEventRow {
    offset: i64,
    id: String,
    span: String,
    seq: i64,
    prev: Option<String>,
    manifest: Option<String>,
    at: String,
    kind: String,
    raw: String,
}

fn signed_string(raw: &Value, event_id: &str, field: &'static str) -> Result<String, TraceError> {
    raw.get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| TraceError::MalformedEventField { id: event_id.to_string(), field })
}

fn signed_optional_string(
    raw: &Value,
    event_id: &str,
    field: &'static str,
) -> Result<Option<String>, TraceError> {
    match raw.get(field) {
        Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        _ => Err(TraceError::MalformedEventField { id: event_id.to_string(), field }),
    }
}

/// RF-16's stable enforcement boundary: verify signed raw first, then prove
/// that every materialized selector agrees. Keep the comparisons explicit so
/// the targeted mutation lane can mutate and kill each field independently.
fn verified_event_from_row(row: &EventRow, vk: &VerifyingKey) -> Result<VerifiedEvent, TraceError> {
    canon::verify(&row.raw, vk)?;
    let id = signed_string(&row.raw, &row.id, "id")?;
    let span = signed_string(&row.raw, &id, "span")?;
    let seq = row.raw.get("seq").and_then(Value::as_i64).ok_or_else(|| {
        TraceError::MalformedEventField { id: id.clone(), field: "seq" }
    })?;
    let prev = signed_optional_string(&row.raw, &id, "prev")?;
    let manifest = signed_optional_string(&row.raw, &id, "manifest")?;
    let at = signed_string(&row.raw, &id, "at")?;
    let kind = signed_string(&row.raw, &id, "kind")?;

    if row.id != id {
        return Err(TraceError::EventIndexMismatch { id, field: "id" });
    }
    if row.span != span {
        return Err(TraceError::EventIndexMismatch { id, field: "span" });
    }
    if row.seq != seq {
        return Err(TraceError::EventIndexMismatch { id, field: "seq" });
    }
    if row.prev != prev {
        return Err(TraceError::EventIndexMismatch { id, field: "prev" });
    }
    if row.manifest != manifest {
        return Err(TraceError::EventIndexMismatch { id, field: "manifest" });
    }
    if row.at != at {
        return Err(TraceError::EventIndexMismatch { id, field: "at" });
    }
    if row.kind != kind {
        return Err(TraceError::EventIndexMismatch { id, field: "kind" });
    }

    Ok(VerifiedEvent {
        offset: row.offset, id, span, seq, prev, manifest, at, kind, raw: row.raw.clone(),
    })
}

fn row_to_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<EventRow> {
    let raw_json = row.get::<_, String>(8)?;
    let raw = canon::parse_fabric_json(&raw_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            8,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })?;
    Ok(EventRow {
        offset: row.get(0)?,
        id: row.get(1)?,
        span: row.get(2)?,
        seq: row.get(3)?,
        prev: row.get(4)?,
        manifest: row.get(5)?,
        at: row.get(6)?,
        kind: row.get(7)?,
        raw,
    })
}

fn w19_stored_event_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<W19StoredEventRow> {
    Ok(W19StoredEventRow {
        offset: row.get(0)?,
        id: row.get(1)?,
        span: row.get(2)?,
        seq: row.get(3)?,
        prev: row.get(4)?,
        manifest: row.get(5)?,
        at: row.get(6)?,
        kind: row.get(7)?,
        raw: row.get(8)?,
    })
}

const EVENT_COLS: &str = "offset, id, span, seq, prev, manifest, at, kind, raw";

pub fn events_in_span(conn: &Connection, span: &str) -> Result<Vec<EventRow>, TraceError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {EVENT_COLS} FROM events WHERE span = ?1 ORDER BY seq"
    ))?;
    let rows = stmt.query_map([span], row_to_event)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn all_events(conn: &Connection) -> Result<Vec<EventRow>, TraceError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {EVENT_COLS} FROM events ORDER BY offset"
    ))?;
    let rows = stmt.query_map([], row_to_event)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Fetch one event by its global substrate offset for operator inspection.
pub fn event_at_offset(
    conn: &Connection,
    offset: i64,
) -> Result<Option<EventRow>, TraceError> {
    conn.query_row(
        &format!("SELECT {EVENT_COLS} FROM events WHERE offset = ?1"),
        [offset],
        row_to_event,
    )
    .optional()
    .map_err(TraceError::from)
}

fn verify_verified_chain(span: &str, events: &[&VerifiedEvent]) -> Result<(), TraceError> {
    let mut ordered = events.to_vec();
    ordered.sort_by_key(|event| event.seq);
    let mut expected_prev: Option<&str> = None;
    for (index, event) in ordered.iter().enumerate() {
        let fail = |detail: String| TraceError::ChainBroken {
            span: span.to_string(), seq: event.seq, detail,
        };
        if event.seq != index as i64 {
            return Err(fail(format!("expected seq {index}, found {}", event.seq)));
        }
        if event.prev.as_deref() != expected_prev {
            return Err(fail(format!(
                "prev {:?} != expected {:?}", event.prev, expected_prev
            )));
        }
        expected_prev = Some(&event.id);
    }
    Ok(())
}

fn w19_event_is_known_kind(event: &VerifiedEvent) -> bool {
    EVENT_KINDS.contains(&event.kind.as_str())
}

fn w19_event_has_object_body(event: &VerifiedEvent) -> bool {
    event.raw.get("body").is_some_and(Value::is_object)
}

fn w19_signed_sequence_follows_storage_order(events: &[&VerifiedEvent]) -> bool {
    events
        .windows(2)
        .all(|pair| pair[0].seq.checked_add(1) == Some(pair[1].seq))
}

fn w19_ledger_event_view_from_rows(
    rows: Vec<W19StoredEventRow>,
    vk: &VerifyingKey,
) -> LedgerEventView {
    let records = rows
        .iter()
        .map(|row| LedgerStoredRecord {
            offset: row.offset,
            raw: row.raw.clone(),
        })
        .collect();
    let mut events = Vec::new();
    let mut findings = Vec::new();

    for stored in rows {
        let raw = match canon::parse_fabric_json(&stored.raw) {
            Ok(raw) => raw,
            Err(error) => {
                findings.push(TraceIntegrityFinding {
                    offset: Some(stored.offset),
                    span: Some(stored.span),
                    detail: format!("raw fabric JSON is malformed: {error}"),
                });
                continue;
            }
        };
        let row = EventRow {
            offset: stored.offset,
            id: stored.id,
            span: stored.span,
            seq: stored.seq,
            prev: stored.prev,
            manifest: stored.manifest,
            at: stored.at,
            kind: stored.kind,
            raw,
        };
        let materialized_span = row.span.clone();
        match verified_event_from_row(&row, vk) {
            Ok(event) => {
                if !w19_event_is_known_kind(&event) {
                    findings.push(TraceIntegrityFinding {
                        offset: Some(event.offset),
                        span: Some(event.span.clone()),
                        detail: format!("signed event has unknown kind `{}`", event.kind),
                    });
                }
                if !w19_event_has_object_body(&event) {
                    findings.push(TraceIntegrityFinding {
                        offset: Some(event.offset),
                        span: Some(event.span.clone()),
                        detail: "signed event body is not an object".into(),
                    });
                }
                events.push(event);
            }
            Err(error) => findings.push(TraceIntegrityFinding {
                offset: Some(row.offset),
                span: Some(materialized_span),
                detail: format!("event verification failed: {error}"),
            }),
        }
    }

    let mut spans: std::collections::BTreeMap<String, Vec<&VerifiedEvent>> =
        std::collections::BTreeMap::new();
    for event in &events {
        spans.entry(event.span.clone()).or_default().push(event);
    }
    for (span, span_events) in spans {
        if !w19_signed_sequence_follows_storage_order(&span_events) {
            let inversion = span_events
                .windows(2)
                .find(|pair| pair[0].seq.checked_add(1) != Some(pair[1].seq))
                .expect("sequence-order predicate found an inversion");
            findings.push(TraceIntegrityFinding {
                offset: Some(inversion[1].offset),
                span: Some(span.clone()),
                detail: format!(
                    "unsigned offset order contradicts signed per-span sequence: seq {} precedes seq {}",
                    inversion[0].seq, inversion[1].seq
                ),
            });
        }
        if let Err(error) = verify_verified_chain(&span, &span_events) {
            let offset = match &error {
                TraceError::ChainBroken { seq, .. } => span_events
                    .iter()
                    .find(|event| event.seq == *seq)
                    .map(|event| event.offset),
                _ => None,
            }
            .or_else(|| span_events.first().map(|event| event.offset));
            findings.push(TraceIntegrityFinding {
                offset,
                span: Some(span),
                detail: format!("per-span chain verification failed: {error}"),
            });
        }
    }

    LedgerEventView {
        events,
        findings,
        records,
    }
}

/// Build the operator ledger's integrity-aware retained-row view in one SQLite
/// statement snapshot. Every decodable row is either represented by a verified
/// event or by an explicit finding; storage-type errors reject the whole view,
/// so nothing unverified can be silently used for accounting.
pub fn ledger_event_view(
    conn: &Connection,
    vk: &VerifyingKey,
) -> Result<LedgerEventView, TraceError> {
    let mut statement = conn.prepare(&format!(
        "SELECT {EVENT_COLS} FROM events ORDER BY offset"
    ))?;
    let rows = statement.query_map([], w19_stored_event_row)?;
    let rows = rows.collect::<Result<Vec<_>, _>>()?;
    Ok(w19_ledger_event_view_from_rows(rows, vk))
}

/// Build the signed event materialized view used by authority consumers.
///
/// Rows whose raw object does not verify under `vk` contribute no authority
/// (A22's unsigned-row rule). Once raw DOES verify, any disagreement with its
/// row selectors is a structural failure rather than an ignorable anomaly — a
/// signed revoke can otherwise be concealed by changing only `kind`/`span`
/// (RF-16). All signature-verified spans are then checked for contiguous
/// sequence and prev linkage. SI-25 remains responsible for an expected global
/// head, tail/whole-span deletion, rollback, and authenticated `offset`.
fn w11_observed_head(rows: &[EventRow]) -> i64 {
    rows.last().map(|row| row.offset).unwrap_or(0)
}

fn verified_snapshot_from_rows(
    rows: Vec<EventRow>,
    vk: &VerifyingKey,
) -> Result<VerifiedEventSnapshot, TraceError> {
    // `all_events` returns one SQLite statement snapshot ordered by offset.
    // Deriving head from those same rows eliminates the events/head TOCTOU
    // without claiming that offset itself is authenticated (SI-25).
    let head = w11_observed_head(&rows);
    let mut verified = Vec::new();
    for row in rows {
        match verified_event_from_row(&row, vk) {
            Ok(event) => verified.push(event),
            Err(TraceError::Canon(_)) => {
                // A wholly unsigned/foreign row moves no authority. Relevant
                // span verifiers remain strict, so inserting one into a live
                // chain fails at the gate rather than being laundered away.
            }
            Err(error) => return Err(error),
        }
    }

    let mut spans: std::collections::BTreeMap<String, Vec<&VerifiedEvent>> =
        std::collections::BTreeMap::new();
    for event in &verified {
        spans.entry(event.span.clone()).or_default().push(event);
    }
    for (span, events) in spans {
        verify_verified_chain(&span, &events)?;
    }
    Ok(VerifiedEventSnapshot {
        events: verified,
        head,
    })
}

/// Build one consistent retained-row event view and its observed head.
pub fn verified_event_snapshot(
    conn: &Connection,
    vk: &VerifyingKey,
) -> Result<VerifiedEventSnapshot, TraceError> {
    verified_snapshot_from_rows(all_events(conn)?, vk)
}

/// Build the signed event materialized view used by consumers that do not
/// also need the decision-time observed head.
pub fn verified_events(
    conn: &Connection,
    vk: &VerifyingKey,
) -> Result<Vec<VerifiedEvent>, TraceError> {
    Ok(verified_event_snapshot(conn, vk)?.events)
}

/// Verify a span's chain end-to-end: seq contiguity from 0, prev-linkage,
/// id recomputation from raw bytes, and signature by the emitting component.
/// Returns the number of verified events.
pub fn verify_span(
    conn: &Connection,
    vk: &VerifyingKey,
    span: &str,
) -> Result<usize, TraceError> {
    let rows = events_in_span(conn, span)?;
    let verified = rows.iter().map(|row| {
        verified_event_from_row(row, vk).map_err(|error| match error {
            TraceError::Canon(canon) => TraceError::ChainBroken {
                span: span.to_string(), seq: row.seq, detail: canon.to_string(),
            },
            other => other,
        })
    }).collect::<Result<Vec<_>, _>>()?;
    let refs = verified.iter().collect::<Vec<_>>();
    verify_verified_chain(span, &refs)?;
    Ok(verified.len())
}

// ---- objects ----------------------------------------------------------

/// Store a sealed fabric object (principal, channel, intent, manifest…).
pub fn put_object(
    conn: &Connection,
    kind: &str,
    sealed: &Map<String, Value>,
    now: &str,
) -> Result<String, TraceError> {
    let id = sealed
        .get("id")
        .and_then(Value::as_str)
        .expect("sealed object has id")
        .to_string();
    let raw = serde_json::to_string(&Value::Object(sealed.clone())).expect("serialize");
    conn.execute(
        "INSERT OR IGNORE INTO objects (id, kind, created_at, raw) VALUES (?1,?2,?3,?4)",
        params![id, kind, now, raw],
    )?;
    Ok(id)
}

/// Parse a stored object without establishing authority. This is for
/// diagnostics, demonstrations, and adversarial test setup only; production
/// consumers that apply object fields use [`load_verified_object`].
pub fn get_object(conn: &Connection, id: &str) -> Result<Value, TraceError> {
    let raw = conn.query_row("SELECT raw FROM objects WHERE id = ?1", [id], |r| {
        r.get::<_, String>(0)
    })
    .optional()?
    .ok_or_else(|| TraceError::ObjectNotFound(id.into()))?;
    Ok(canon::parse_fabric_json(&raw)?)
}

fn w14_object_id_matches(requested: &str, signed: &str) -> bool {
    requested == signed
}

fn w14_object_prefix_matches(signed: &str, expected: &str) -> bool {
    signed.split_once(':').map(|(prefix, _)| prefix) == Some(expected)
}

fn w14_object_kind_matches(stored: &str, expected: &str) -> bool {
    stored == expected
}

/// Load an authority-bearing fabric object through one typed verification
/// boundary (RF-19/W-14).
///
/// The raw object must verify under `vk`, its signed id must equal the row id
/// requested by the caller, and both the signed id namespace and the unsigned
/// materialized `kind` column must match the caller's expected type. No caller
/// may apply roots, caveats, tool metadata, or lineage fields before this
/// function returns successfully.
pub fn load_verified_object(
    conn: &Connection,
    id: &str,
    expected_prefix: &str,
    expected_kind: &str,
    vk: &VerifyingKey,
) -> Result<Value, TraceError> {
    let (stored_kind, raw): (String, String) = conn
        .query_row(
            "SELECT kind, raw FROM objects WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| TraceError::ObjectNotFound(id.into()))?;
    let object = canon::parse_fabric_json(&raw)?;

    // Verify signed raw before trusting or applying either its fields or the
    // denormalized row kind. A valid object of the wrong type is still inert
    // at this caller boundary.
    canon::verify(&object, vk)?;
    let signed_id = object
        .get("id")
        .and_then(Value::as_str)
        .ok_or(CanonError::MissingField("id"))?;
    if !w14_object_id_matches(id, signed_id) {
        return Err(TraceError::ObjectIdMismatch {
            requested: id.into(),
            signed: signed_id.into(),
        });
    }
    let actual_prefix = signed_id
        .split_once(':')
        .map(|(prefix, _)| prefix)
        .unwrap_or_default();
    if !w14_object_prefix_matches(signed_id, expected_prefix) {
        return Err(TraceError::ObjectPrefixMismatch {
            id: id.into(),
            expected: expected_prefix.into(),
            actual: actual_prefix.into(),
        });
    }
    if !w14_object_kind_matches(&stored_kind, expected_kind) {
        return Err(TraceError::ObjectKindMismatch {
            id: id.into(),
            expected: expected_kind.into(),
            actual: stored_kind,
        });
    }
    Ok(object)
}

pub fn meta_get(conn: &Connection, key: &str) -> Result<Option<String>, TraceError> {
    Ok(conn
        .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .optional()?)
}

pub fn meta_set(conn: &Connection, key: &str, value: &str) -> Result<(), TraceError> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn meta_del(conn: &Connection, key: &str) -> Result<(), TraceError> {
    conn.execute("DELETE FROM meta WHERE key = ?1", [key])?;
    Ok(())
}

/// All meta entries whose key starts with `prefix`, ordered by key.
pub fn meta_scan(conn: &Connection, prefix: &str) -> Result<Vec<(String, String)>, TraceError> {
    let mut stmt = conn.prepare(
        "SELECT key, value FROM meta WHERE key >= ?1 AND key < ?1 || x'ff' ORDER BY key",
    )?;
    let rows = stmt.query_map([prefix], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    fn setup() -> (Connection, SigningKey) {
        let conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        (conn, SigningKey::generate(&mut OsRng))
    }

    #[test]
    fn chain_links_and_offsets_are_global() {
        let (mut conn, sk) = setup();
        let s1 = new_span();
        let s2 = new_span();
        let a = append(&mut conn, &sk, &s1, None, "snapshot", serde_json::json!({"n":1}), "t1").unwrap();
        let b = append(&mut conn, &sk, &s2, None, "snapshot", serde_json::json!({"n":2}), "t2").unwrap();
        let c = append(&mut conn, &sk, &s1, Some("man:x"), "tool_call", serde_json::json!({"n":3}), "t3").unwrap();
        // Per-span seq; global offset.
        assert_eq!((a.seq, b.seq, c.seq), (0, 0, 1));
        assert_eq!((a.offset, b.offset, c.offset), (1, 2, 3));
        assert_eq!(head_offset(&conn).unwrap(), 3);
        // Head has prev null; second links to first (SI-4).
        let evs = events_in_span(&conn, &s1).unwrap();
        assert_eq!(evs[0].raw["prev"], Value::Null);
        assert_eq!(evs[1].raw["prev"], Value::String(a.id.clone()));
        assert_eq!(verify_span(&conn, &sk.verifying_key(), &s1).unwrap(), 2);
        assert_eq!(verify_span(&conn, &sk.verifying_key(), &s2).unwrap(), 1);
    }

    #[test]
    fn unknown_kind_rejected() {
        let (mut conn, sk) = setup();
        let s = new_span();
        assert!(matches!(
            append(&mut conn, &sk, &s, None, "made_up", serde_json::json!({}), "t"),
            Err(TraceError::UnknownKind(_))
        ));
    }

    #[test]
    fn revoke_kind_accepted_and_expiry_kind_removed() {
        // A22 (§5.4): the closure edge is emittable; the never-emitted
        // `expiry` kind left §6 in the same amendment and an attempt to
        // emit one must fail without landing on any chain.
        let (mut conn, sk) = setup();
        let s = new_span();
        append(
            &mut conn,
            &sk,
            &s,
            Some("man:x"),
            "revoke",
            serde_json::json!({
                "capability": "cap:x", "reason": "operator_request",
                "channel": "chan:y", "auth_strength": "local_session"
            }),
            "t",
        )
        .unwrap();
        assert!(matches!(
            append(&mut conn, &sk, &s, None, "expiry", serde_json::json!({}), "t"),
            Err(TraceError::UnknownKind(_))
        ));
        // The failed append left no residue: exactly the one revoke event.
        let evs = events_in_span(&conn, &s).unwrap();
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].kind, "revoke");
        assert_eq!(verify_span(&conn, &sk.verifying_key(), &s).unwrap(), 1);
    }

    #[test]
    fn append_only_enforced_by_triggers() {
        let (mut conn, sk) = setup();
        let s = new_span();
        append(&mut conn, &sk, &s, None, "snapshot", serde_json::json!({}), "t").unwrap();
        assert!(conn
            .execute("UPDATE events SET kind = 'revert' WHERE seq = 0", [])
            .is_err());
        assert!(conn.execute("DELETE FROM events", []).is_err());
    }

    #[test]
    fn tamper_after_bypassing_triggers_is_detected() {
        let (mut conn, sk) = setup();
        let s = new_span();
        append(&mut conn, &sk, &s, None, "snapshot", serde_json::json!({"v":1}), "t1").unwrap();
        append(&mut conn, &sk, &s, None, "tool_call", serde_json::json!({"v":2}), "t2").unwrap();
        // An attacker with raw db access drops the triggers and edits history.
        conn.execute_batch(
            "DROP TRIGGER events_append_only_u;
             UPDATE events SET raw = replace(raw, '\"v\":2', '\"v\":999') WHERE seq = 1;",
        )
        .unwrap();
        let err = verify_span(&conn, &sk.verifying_key(), &s).unwrap_err();
        assert!(matches!(err, TraceError::ChainBroken { seq: 1, .. }));
    }

    #[test]
    fn signed_event_index_mismatch_is_rejected_for_every_materialized_field() {
        // RF-16: once raw verifies, no denormalized selector may disagree.
        // Each case gets a fresh chain so one mutation cannot mask another.
        let cases = [
            ("id", "id = 'evt:index-tamper'"),
            ("span", "span = 'span:index-tamper'"),
            ("seq", "seq = 9"),
            ("prev", "prev = 'evt:index-tamper'"),
            ("manifest", "manifest = 'man:index-tamper'"),
            ("at", "at = 't-index-tamper'"),
            ("kind", "kind = 'grant'"),
        ];

        for (field, assignment) in cases {
            let (mut conn, sk) = setup();
            let span = new_span();
            append(
                &mut conn,
                &sk,
                &span,
                Some("man:original"),
                "revoke",
                serde_json::json!({"capability":"cap:x", "reason":"test"}),
                "t-original",
            )
            .unwrap();
            conn.execute_batch(&format!(
                "DROP TRIGGER events_append_only_u; UPDATE events SET {assignment} WHERE offset = 1;"
            ))
            .unwrap();

            match verified_events(&conn, &sk.verifying_key()) {
                Err(TraceError::EventIndexMismatch { field: observed, .. }) => {
                    assert_eq!(observed, field)
                }
                other => panic!("{field} mismatch must fail at verified-event boundary: {other:?}"),
            }
        }
    }

    #[test]
    fn w19_ledger_view_classifies_verified_and_anomalous_rows() {
        fn assert_finding(conn: &Connection, sk: &SigningKey, needle: &str) {
            let view = ledger_event_view(conn, &sk.verifying_key()).unwrap();
            assert!(
                view.findings.iter().any(|finding| finding.detail.contains(needle)),
                "missing `{needle}` finding: {:?}",
                view.findings
            );
            assert!(view.w19_require_clean().is_err());
        }

        fn insert_signed_row(
            conn: &Connection,
            sk: &SigningKey,
            kind: &str,
            body: Value,
        ) {
            let span = new_span();
            let mut object = Map::new();
            object.insert("span".into(), Value::String(span.clone()));
            object.insert("seq".into(), Value::from(0));
            object.insert("prev".into(), Value::Null);
            object.insert("manifest".into(), Value::Null);
            object.insert("at".into(), Value::String("t".into()));
            object.insert("kind".into(), Value::String(kind.into()));
            object.insert("body".into(), body);
            let sealed = canon::seal("evt", object, sk).unwrap();
            let id = sealed["id"].as_str().unwrap().to_string();
            let raw = serde_json::to_string(&Value::Object(sealed)).unwrap();
            conn.execute(
                "INSERT INTO events (id, span, seq, prev, manifest, at, kind, raw)
                 VALUES (?1, ?2, 0, NULL, NULL, 't', ?3, ?4)",
                params![id, span, kind, raw],
            )
            .unwrap();
        }

        let (mut conn, sk) = setup();
        let span = new_span();
        append(
            &mut conn,
            &sk,
            &span,
            None,
            "snapshot",
            serde_json::json!({"n":1}),
            "t",
        )
        .unwrap();
        let clean = ledger_event_view(&conn, &sk.verifying_key()).unwrap();
        assert_eq!(clean.events.len(), 1);
        assert_eq!(clean.records.len(), 1);
        assert!(clean.findings.is_empty());
        clean.w19_require_clean().unwrap();

        conn.execute_batch(
            "DROP TRIGGER events_append_only_u;
             UPDATE events SET raw = replace(raw, '\"n\":1', '\"n\":9');",
        )
        .unwrap();
        assert_finding(&conn, &sk, "event verification failed");

        let (mut conn, sk) = setup();
        let span = new_span();
        append(
            &mut conn,
            &sk,
            &span,
            None,
            "snapshot",
            serde_json::json!({}),
            "t",
        )
        .unwrap();
        conn.execute_batch(
            "DROP TRIGGER events_append_only_u;
             UPDATE events SET kind = 'grant';",
        )
        .unwrap();
        assert_finding(&conn, &sk, "index column `kind`");

        let (mut conn, sk) = setup();
        let span = new_span();
        append(
            &mut conn,
            &sk,
            &span,
            None,
            "snapshot",
            serde_json::json!({}),
            "t",
        )
        .unwrap();
        conn.execute_batch(
            "DROP TRIGGER events_append_only_u;
             UPDATE events SET raw = '{';",
        )
        .unwrap();
        assert_finding(&conn, &sk, "raw fabric JSON is malformed");

        let (mut conn, sk) = setup();
        let span = new_span();
        append(
            &mut conn,
            &sk,
            &span,
            None,
            "snapshot",
            serde_json::json!({"n":1}),
            "t1",
        )
        .unwrap();
        append(
            &mut conn,
            &sk,
            &span,
            None,
            "snapshot",
            serde_json::json!({"n":2}),
            "t2",
        )
        .unwrap();
        append(
            &mut conn,
            &sk,
            &span,
            None,
            "snapshot",
            serde_json::json!({"n":3}),
            "t3",
        )
        .unwrap();
        conn.execute_batch(
            "DROP TRIGGER events_append_only_d;
             DELETE FROM events WHERE seq = 1;",
        )
        .unwrap();
        let view = ledger_event_view(&conn, &sk.verifying_key()).unwrap();
        let chain_finding = view
            .findings
            .iter()
            .find(|finding| {
                finding
                    .detail
                    .contains("per-span chain verification failed")
            })
            .expect("middle-event deletion must produce a chain finding");
        assert_eq!(chain_finding.offset, Some(3));
        assert!(chain_finding.detail.contains("seq 2"));
        assert!(view.w19_require_clean().is_err());

        let (mut conn, sk) = setup();
        let span = new_span();
        append(
            &mut conn,
            &sk,
            &span,
            None,
            "snapshot",
            serde_json::json!({"n":1}),
            "t1",
        )
        .unwrap();
        append(
            &mut conn,
            &sk,
            &span,
            None,
            "snapshot",
            serde_json::json!({"n":2}),
            "t2",
        )
        .unwrap();
        conn.execute_batch(
            "DROP TRIGGER events_append_only_u;
             UPDATE events SET offset = -1 WHERE seq = 0;
             UPDATE events SET offset = 1 WHERE seq = 1;
             UPDATE events SET offset = 2 WHERE offset = -1;",
        )
        .unwrap();
        assert_finding(
            &conn,
            &sk,
            "unsigned offset order contradicts signed per-span sequence",
        );

        let (conn, sk) = setup();
        insert_signed_row(&conn, &sk, "made_up", serde_json::json!({}));
        assert_finding(&conn, &sk, "signed event has unknown kind");

        let (conn, sk) = setup();
        insert_signed_row(&conn, &sk, "snapshot", Value::String("not-an-object".into()));
        assert_finding(&conn, &sk, "signed event body is not an object");
    }

    #[test]
    fn w12_duplicate_names_are_rejected_at_stored_fabric_ingresses() {
        let (mut conn, sk) = setup();
        let span = new_span();
        append(
            &mut conn,
            &sk,
            &span,
            None,
            "snapshot",
            serde_json::json!({"n":1}),
            "t",
        )
        .unwrap();

        // A storage attacker can write raw JSON after bypassing the trigger.
        // Even a same-value duplicate must be rejected before Value parsing;
        // otherwise parser policy, rather than signed bytes, chooses meaning.
        conn.execute_batch(
            "DROP TRIGGER events_append_only_u;
             UPDATE events SET raw = replace(raw, '\"seq\":0', '\"seq\":0,\"seq\":0');",
        )
        .unwrap();
        assert!(
            verify_span(&conn, &sk.verifying_key(), &span).is_err(),
            "duplicate-bearing event must not enter the verified trace"
        );

        conn.execute(
            "INSERT INTO objects (id, kind, created_at, raw) VALUES (?1, ?2, ?3, ?4)",
            params!["test:duplicate", "test", "t", r#"{"x":1,"x":1}"#],
        )
        .unwrap();
        assert!(
            get_object(&conn, "test:duplicate").is_err(),
            "duplicate-bearing stored object must produce no accepted fabric value"
        );
    }

    #[test]
    fn verified_snapshot_head_stays_with_the_rows_it_observed() {
        let (mut conn, sk) = setup();
        let span = new_span();
        let first = append(
            &mut conn,
            &sk,
            &span,
            None,
            "snapshot",
            serde_json::json!({"n":1}),
            "t1",
        )
        .unwrap();

        // Capture the exact rows a SQLite SELECT observed, then model a
        // concurrent commit before authority asks for its decision offset.
        let observed_rows = all_events(&conn).unwrap();
        let later = append(
            &mut conn,
            &sk,
            &span,
            None,
            "snapshot",
            serde_json::json!({"n":2}),
            "t2",
        )
        .unwrap();
        let snapshot = verified_snapshot_from_rows(observed_rows, &sk.verifying_key()).unwrap();

        assert_eq!(snapshot.head, first.offset);
        assert_eq!(snapshot.events.len(), 1);
        assert!(
            snapshot.head < later.offset,
            "a later commit must not race the observed head ahead of its event set"
        );
    }

    #[test]
    fn deleting_a_middle_event_breaks_the_chain() {
        let (mut conn, sk) = setup();
        let s = new_span();
        for i in 0..3 {
            append(&mut conn, &sk, &s, None, "snapshot", serde_json::json!({"i":i}), "t").unwrap();
        }
        conn.execute_batch("DROP TRIGGER events_append_only_d; DELETE FROM events WHERE seq = 1;")
            .unwrap();
        assert!(verify_span(&conn, &sk.verifying_key(), &s).is_err());
    }

    #[test]
    fn w14_verified_object_accepts_each_expected_signed_type() {
        let (conn, sk) = setup();
        for (prefix, kind) in [
            ("man", "manifest"),
            ("tool", "tool"),
            ("cap", "capability"),
            ("chan", "channel"),
        ] {
            let mut body = Map::new();
            body.insert("marker".into(), Value::String(kind.into()));
            let sealed = canon::seal(prefix, body, &sk).unwrap();
            let id = put_object(&conn, kind, &sealed, "t").unwrap();
            let loaded = load_verified_object(
                &conn,
                &id,
                prefix,
                kind,
                &sk.verifying_key(),
            )
            .unwrap();
            assert_eq!(loaded["id"], id);
            assert_eq!(loaded["marker"], kind);
        }
    }

    #[test]
    fn w14_verified_object_rejects_wrong_id_prefix_kind_signature_or_key() {
        let (conn, sk) = setup();
        let mut manifest_body = Map::new();
        manifest_body.insert("marker".into(), Value::String("manifest".into()));
        let manifest = canon::seal("man", manifest_body, &sk).unwrap();
        let manifest_id = put_object(&conn, "manifest", &manifest, "t").unwrap();

        assert!(matches!(
            load_verified_object(
                &conn,
                &manifest_id,
                "cap",
                "manifest",
                &sk.verifying_key(),
            ),
            Err(TraceError::ObjectPrefixMismatch { .. })
        ));

        conn.execute(
            "UPDATE objects SET kind = 'capability' WHERE id = ?1",
            [&manifest_id],
        )
        .unwrap();
        assert!(matches!(
            load_verified_object(
                &conn,
                &manifest_id,
                "man",
                "manifest",
                &sk.verifying_key(),
            ),
            Err(TraceError::ObjectKindMismatch { .. })
        ));
        conn.execute(
            "UPDATE objects SET kind = 'manifest' WHERE id = ?1",
            [&manifest_id],
        )
        .unwrap();

        let other_key = SigningKey::generate(&mut OsRng);
        assert!(load_verified_object(
            &conn,
            &manifest_id,
            "man",
            "manifest",
            &other_key.verifying_key(),
        )
        .is_err());

        let mut capability_body = Map::new();
        capability_body.insert("marker".into(), Value::String("capability".into()));
        let capability = canon::seal("cap", capability_body, &sk).unwrap();
        let capability_id = put_object(&conn, "capability", &capability, "t").unwrap();
        let capability_raw = serde_json::to_string(&Value::Object(capability)).unwrap();
        conn.execute(
            "UPDATE objects SET raw = ?2 WHERE id = ?1",
            params![manifest_id, capability_raw],
        )
        .unwrap();
        assert!(matches!(
            load_verified_object(
                &conn,
                &manifest_id,
                "man",
                "manifest",
                &sk.verifying_key(),
            ),
            Err(TraceError::ObjectIdMismatch { requested, signed })
                if requested == manifest_id && signed == capability_id
        ));

        conn.execute(
            "UPDATE objects SET raw = replace(raw, 'capability', 'widened') WHERE id = ?1",
            [&capability_id],
        )
        .unwrap();
        assert!(load_verified_object(
            &conn,
            &capability_id,
            "cap",
            "capability",
            &sk.verifying_key(),
        )
        .is_err());
    }
}
