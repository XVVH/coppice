# Cryptographic mechanism safety audit — 2026-07-12

Scope: commit `eb493748b7ff3cdfe470a546b79923c296e2f5f0`, reviewed
against `docs/asf-schema-spec.md` v0.7 and the declared
`SU/COOP/LOCAL/NOACT/1SESS/1HUMAN/1TEN/DEBUG` posture. This is the evidence
record; live status belongs in `review-findings.md` (RF),
`spec-issues.md` (SI), `posture-assumptions.md` (P),
`testing-theory.md` (G), and `roadmap.md` (W).

## Bottom line

Ed25519 (`ed25519-dalek` strict verification), SHA-256, AES-256-GCM, and
OS randomness are suitable primitive choices. The current composition is
suitable only for cooperative local dogfooding. It is not yet a production
security boundary, published signed format, live-egress authority system, or
forensic-erasure mechanism.

The release blockers are mechanism failures around the primitives: an
unauthenticated global trace order/head, an unchecked JCS input domain,
missing key-identity continuity, incomplete typed verification before state
application, an unbound/non-atomic payload envelope, and asserted rather than
authenticated identity/credential surfaces. W-10 and W-11 merged the immediate
bundled-SQLite and unsigned-index repairs; neither weakens the remaining
production gates.

## Remediation merged from this audit

- **W-10 / RF-21 (PR #37):** `rusqlite 0.40.1` now bundles SQLite 3.53.2, above the
  conservative 3.51.3 floor. `Fabric::open` checks the linked runtime before
  creating or opening protected state. The paired contract proves both safe
  startup and zero-state refusal below the floor; the targeted mutation lane
  catches all 4 runtime-floor mutants. SQLite documents the affected conditions
  and fixed releases in its
  [WAL-reset analysis](https://www.sqlite.org/wal.html#the_wal_reset_bug).
- **W-11 / RF-16 (PR #36):** authority consumers now accept a `VerifiedEvent` derived
  from signature-verified raw and exact agreement with every denormalized
  selector (`id`, `span`, `seq`, `prev`, `manifest`, `at`, `kind`).
  The original combined `kind`+`span` revocation concealment now fails
  structurally before any ticket/effect/escalation, and a per-field matrix plus
  targeted mutation lane pins the boundary.

W-11 is an authority-surface change. Its first independent-context review
returned REQUEST CHANGES with five findings; all five were corrected, and the
independent re-review returned APPROVE WITH NON-BLOCKING FOLLOW-UPS. The
authority-review gate is satisfied. SI-25/RF-13 still own authenticated
cross-span `offset`, completeness, expected head, deletion/signature
invalidation, and rollback;
the merged fix does not claim those properties.

## Adversarial reproductions

### A22 revocation concealment

After a valid signed revoke was appended, changing only the SQLite row's
unsigned `kind` to `grant` and `span` to another span (leaving signed `raw`
unchanged and using no signing key) made `Broker::propose_call` return
`Allowed`. Decision-time liveness filters and branches on the denormalized row
before it establishes a fully verified event view. Filed as RF-16.

### JCS semantic collision

The locked `serde_json_canonicalizer 0.3.2` converts `u64` to `f64`. Against
ASF's actual seal/verify boundary, changing a signed integer from
`9007199254740992` to `9007199254740993` preserved the canonical bytes and
`canon::verify` returned `Ok(())`. `serde_json::Value` retained the mutated
exact integer. The spec forbids both values, but seal and verify do not enforce
that domain. Filed as RF-17; this is authenticity, not only interoperability.

## Primitive assessment

| Primitive | Result | Boundary |
|---|---|---|
| Ed25519 / `ed25519-dalek 2.2.0` | suitable | key custody, rotation, type/domain binding remain open |
| SHA-256 | suitable | plaintext addressing leaks equality by design; RF-7 |
| AES-256-GCM / `aes-gcm 0.10.3` | suitable | AAD, algorithm dispatch, wrapping nonce lifetime, and storage protocol are incomplete |
| `OsRng` | suitable | fail-stop on unavailable OS entropy is correct |
| JCS / RFC 8785 | conditionally suitable | I-JSON, duplicate-name, and numeric-domain validation must precede signing and verification |
| SQLite | initial lock unsuitable; W-10 remediation suitable for RF-21 | bundled 3.46.0 was affected; merged bundled 3.53.2 plus a conservative runtime floor |

Algorithm conformance does not establish FIPS 140 module validation. The
current Rust crates are not evidence of a validated cryptographic module.

## Finding map

| Area | Canonical tracker | Disposition |
|---|---|---|
| event index columns drive authority and replay clock | RF-16, P15, G10, W-11 | fixed in PR #36; independent re-review approved |
| signed grant parent not bound to capability ancestry | RF-25, G9, W-11 | fixed in PR #36 |
| signed tool-registration placement not enforced | RF-26, G9, W-11 | fixed in PR #36 |
| recovery preselection trusts unsigned promotion selectors | RF-27, G7 | non-blocking fail-closed hardening |
| JCS semantic collision | RF-17, P24, G2, W-12 | fix now |
| missing-key silent regeneration / identity split | RF-18, SI-27, G3, W-13 | implementation in progress; lifecycle design follows |
| unverified manifest application | RF-19, G10, W-14 | fix now |
| restore preflight validates presence, not bytes | RF-20, G3, W-14 | fix now |
| bundled SQLite WAL-reset defect | RF-21, W-10 | fixed in PR #37; 4/4 floor mutants caught |
| payload AAD/dispatch/atomicity/shred generation | RF-22, P13/P19, SI-28/SI-29 | design now; versioned implementation |
| credential reflection through downstream result | RF-23, P27, SI-23, W-18 | block credentials/third-party tools |
| holder/channel/auth claims lack proof | RF-24, P3/P4/P5/P8, SI-23, W-4/W-18 | before actuation/live identity claims |
| unsigned global order / rollback / head | RF-13, SI-25, P15, G10, W-15 | production and standing-authority gate |
| signature type/domain transcript | RF-6, SI-26, P23, G2, W-6 | publication gate |
| cleartext/co-resident key custody | RF-14, SI-27, P1/P2/P7, W-17 | before real credentials/production |
| cleartext fabric state | RF-15, P17 | production/offline-storage gate |
| redaction commitment construction | SI-30 | specification only; not implemented |

Known open problems remain open: cross-run memory taint, compensation fidelity,
multi-actor visibility policy, domain taxonomy governance, and F2
canonicalization. This audit does not silently resolve them.

## W-11 G9 conformance sweep (§5.4, §6)

This maps every normative edge W-11 touches. The first independent-context
review found four missing enforcing edges and one evidence-classification gap;
the table includes their merged corrections. Cross-span offset/head clauses
that cannot be enforced without choosing a protocol remain filed as
SI-25/RF-13 rather than silently interpreted.

| Normative edge | Enforcing line/function | Negative evidence or filing |
|---|---|---|
| validity derives from signed objects plus verified substrate prefix (§5.4) | `trace::verified_event_from_row`, `trace::verified_event_snapshot` | `w11_each_signed_event_selector_mismatch_prevents_dispatch`; constructor matrix is supporting; SI-25 for completeness/freshness |
| verified grant at `G < O` (§5.4.1) | `a22_grant_bindings`, `a22_state_at`, `m7_verify_grant` consume `VerifiedEvent`; A22 binding includes signed `parent` | `a22_malformed_grant_parent_cannot_activate_capability`; existing placement/order negatives |
| no verified revoke of leaf/ancestor at `R < O` (§5.4.2) | `a22_revoke_offsets`, `a22_ancestry`, `a22_state_at` consume `VerifiedEvent` | `a22_signed_revoke_cannot_be_concealed_by_unsigned_indexes`; existing direct/ancestor negatives |
| caveats use signed event `at` (§5.4.3, Two clocks) | `VerifiedEvent.at` is parsed from signed raw after row agreement; gate `CallCtx.now = ev.at` | every-field mismatch test includes `at`; existing `si22_gate_clock_is_the_events_at_not_gate_time` |
| ancestry verifies fail-closed and grants are well ordered (§5.4.4) | `a22_ancestry`, `a22_state_at` | existing unresolved ancestry and out-of-order grant negatives |
| decision and gate share the same reconstruction (§5.4 durable offset/structural rule) | both call the same `a22_*` helpers; decision event set and observed head come from one `VerifiedEventSnapshot` statement view | `verified_snapshot_head_stays_with_the_rows_it_observed`; decision concealment regression; existing gate matrix |
| revokes resolve across the entire verified substrate (§5.4 cascade) | `trace::verified_events` global scan, then `a22_revoke_offsets` with no manifest/span filter | existing cross-manifest ancestor revoke negative |
| doubt never widens: unsigned state inert; signature-verified anomalies close (§5.4) | invalid signatures do not enter `verified_events`; signature-valid index disagreement fails structurally; `a22_revoke_offsets` ignores placement | existing unsigned-row and anomaly tests; new concealment negative |
| per-span signed chain and materialized event fields (§6) | `verify_span`, `verified_event_from_row`, `verify_verified_chain`; gate replay and branch-tip selection order one span by signed `seq` | middle-delete/raw-tamper tests; every-selector protected-effect matrix; `w11_unsigned_offset_cannot_select_older_signed_branch_tip` |
| grant/revoke bodies and placement (§6) | signed `VerifiedEvent.kind/span/manifest`; exact grant parent; `a22_*` edge-specific placement | malformed-parent matrix; grant-on-session, unexpected-span revoke, and malformed provenance negatives |
| registrations live on fabric-lifetime span with `manifest:null` (§6) | `w11_live_tool_registration` | `signed_misplaced_tool_registration_cannot_dispatch` checks each placement dimension |
| substrate offset is global order (§5.4/§6) | current `VerifiedEvent.offset` remains explicitly unsigned; signed `seq` is used only within one span | SI-25/RF-13/P15 for cross-span order, completeness, and freshness |

## W-13 G9 inverse conformance (§8.1, §8.2)

The spec assigns key roles but contains no lifecycle protocol; SI-27 is the
explicit filing for that absence. W-13 implements only the conservative local
continuity boundary RF-18 permits and does not reinterpret missing material as
rotation or recovery.

| Normative edge | Enforcing line/function | Negative evidence or filing |
|---|---|---|
| user-root and fabric signing roles remain distinct (§8.1) | `Role`, `Keystore::signing_key`; `Fabric` loads both from required existing files | `missing_or_malformed_required_key_never_regenerates` covers both roles; positive reopen compares both identities |
| fabric/broker key continues to sign fabric objects (§8.1) | existing `Fabric::substrate_event`, manifest/channel/non-human sealing call sites are unchanged; W-13 only changes key acquisition | existing wrong-key/tamper negatives; SI-26 remains the separate type/domain transcript question |
| agent-instance keys sign runtime attestations (§8.1) | no M4 runtime attestation implementation exists | G3/M4 open gap; W-13 neither creates nor claims an agent-instance lifecycle |
| org mode adds an org root (§8.1) | org mode is absent | SI-27/W-17 lifecycle design; no implementation claim |
| payload DEKs are wrapped to the owner KEK (§8.2) | existing `Kek::new_dek`/`unwrap_dek`; W-13 requires `owner.kek` continuity on reopen | `existing_fabric_key_loss_fails_without_home_mutation` and unit loss/malformed matrix; existing DEK wrap/shred primitive test |
| multi-actor visibility is key distribution (§8.2) | single-owner Stage-1 KEK only | SI-27/W-17 and the spec's deferred policy; RF-14 remains the cleartext/co-resident custody blocker |
| reopening never treats loss as initialization (SI-27/RF-18 immediate boundary) | distinct `Fabric::initialize`/`open_existing`, `Keystore::initialize`/`open_existing`; `w13_prepare_new_home`, `w13_validate_existing_fabric_home`, `w13_load_required_key`, `Cas::open_existing` | database-only, keys-only, mixed partial-home, missing/malformed key, database/CAS substitution, and no-repair matrices prove no replacement/home mutation |
| proxy runtime state is created only on explicit first initialization | `w13_open_fabric_before_runtime_state`, separate fabric/runtime entry predicates, and existing-runtime validation | missing identity creates no `memory.db`; missing existing runtime is not repaired; runtime-init failure creates no fabric identity; outer/fabric/runtime symlink targets remain unchanged |
| credential storage parse doubt never authorizes overwrite (RF-18) | `w13_load_secret_store` + `w13_parse_secret_store` shared by get/set; atomic write parent-fsync | malformed syntax, top-level shape, non-string value, and dangling-symlink matrix preserves the original entry/bytes |

Crash scope is precise: W-13 fsyncs each key, the unpublished keystore
directory, and its parent after directory publication. The suite proves
complete steady-state publication and fail-closed partial reopen; it does not
yet inject hard process exit at each syscall. That remaining crash-injection
evidence is recorded in G3 rather than claimed here.

## Verification evidence

- `./scripts/ci required` passed after the independent-review corrections:
  strict Clippy, 14 two-sided contracts with 70 registered tests, all 168
  workspace tests (including process/socket surfaces), and both acceptance
  demos.
- `ASF_PROPTEST_CASES=4096 ASF_MODEL_CASES=512 ./scripts/ci deep` passed in
  release mode.
- The targeted `verified_event_from_row` mutation lane tested eight mutants:
  seven caught, one unviable, zero survivors.
- The new W-11 consumer lane tested 17 mutants: 15 caught, two unviable, zero
  survivors. The expanded A22 lane caught all 29 mutants, including exact
  signed-parent binding.
- RustSec loaded 1,160 advisories and scanned 116 locked dependencies with no
  reported Rust advisory. RustSec does not cover the bundled SQLite C defect,
  so W-10 additionally pins the amalgamation version and runtime floor.
- W-10's targeted runtime-floor lane caught 4/4 mutants after its first run
  exposed and corrected a missing exact-3.51.3 positive boundary.
- W-13's targeted proxy/initialization/key/CAS/secret-store lane tested 40
  mutants: 35 caught, five compiler-unviable whole-function replacements, zero
  survivors or timeouts. Its iterative runs exposed and corrected missing
  empty-home, ordering, non-repair, path-substitution, and independently masked
  predicate boundaries.
- W-13's `./scripts/ci required` run passed outside the socket-restricted
  sandbox: strict Clippy, 17 contracts with 93 registered tests and 95 frozen
  legacy tests, all 188 workspace tests, and both demos. The deep release lane
  passed with 4,096 authority cases and 512 model histories.
- W-11 merged in PR #36 and W-10 merged in PR #37; the broader audit trackers
  remain local work in progress for the queued remediations.

## Review limits

This was source review, dependency/source inspection, standards comparison,
adversarial local reproduction, and existing-lane execution. It was not a
formal proof, hardware side-channel assessment, forensic-media experiment,
FIPS validation, or independent third-party audit. W-11 completed the
repository's independent-context review discipline; future authority, trace,
key, and envelope changes must do the same before merge.
