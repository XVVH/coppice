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

The original audit found mechanism failures around the primitives. W-10 through
W-13 and W-19 have since closed the bundled-SQLite, unsigned-selector, JCS input,
missing-key continuity, and operator-ledger defects. The remaining release
blockers are an unauthenticated global trace order/head, an unbound/non-atomic
payload envelope, an unspecified production key lifecycle/custody boundary,
and asserted rather than authenticated identity/credential surfaces. W-14's
typed-object and verified-restore repair passed its independent-context
re-review (APPROVE WITH NON-BLOCKING FOLLOW-UPS, 2026-07-13) and merged in
PR #43, closing RF-19/RF-20/RF-29–RF-32 and P16 with two non-blocking
follow-ups filed (RF-33/RF-34). The merged repairs do not weaken the
remaining production gates.

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
- **W-12 / RF-17 (PR #39):** seal and verify now enforce ASF's integer-only
  `|n| < 2^53` JCS input domain recursively, and raw persisted fabric JSON
  rejects duplicate names before `Value` construction. Language-neutral
  boundary/collision vectors and the targeted lane cover the written
  predicates; SI-26/RF-6 type/domain transcript binding remains open.
- **W-13 / RF-18 (PR #40):** explicit initialization is separate from reopen;
  existing and partial homes never regenerate or repair required fabric,
  user-root, or owner-KEK material. Independent review found three ordering and
  path blockers, then approved the corrected implementation. SI-27/W-17 and
  RF-14 retain lifecycle and custody scope.
- **W-19 / RF-28 (PR #41):** operator accounting consumes only a clean
  verified retained-row view and reports decodable anomalies instead of
  trusting or hiding them. Within-span signed-sequence/unsigned-offset
  inversions fail loudly; SI-25 retains cross-span order, completeness,
  rollback, and freshness.

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
| operator ledger trusts or silently omits unverified rows | RF-28, G10, W-19 | fixed in PR #41; SI-25 global-order/rollback residual retained |
| JCS semantic collision | RF-17, P24, G2, W-12 | fixed in PR #39; SI-26/RF-6 separate |
| missing-key silent regeneration / identity split | RF-18, SI-27, G3, W-13 | fixed in PR #40; lifecycle/custody design follows |
| unverified manifest application | RF-19, G10, W-14 | fixed in PR #43; independent re-review approved |
| restore preflight validates presence, not bytes | RF-20, G3, W-14 | fixed in PR #43; independent re-review approved |
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

## W-14 G9 conformance sweep (§0, §3, §4, §5, §5.3, §5.4, §8.1, §8.3)

W-14 changes authority-bearing object consumers and coherent restore
preparation. The typed boundary deliberately validates only the universal
signed-object envelope and caller-expected type; it does not invent the
per-kind schema work filed as G11/W-6.

| Normative edge | Enforcing line/function | Negative evidence or filing |
|---|---|---|
| object ids recompute over canonical signed content and cross-references resolve by id (§0) | `trace::load_verified_object` performs strict parse and `canon::verify`, then requires the signed id to equal the requested row id | `w14_verified_object_rejects_wrong_id_prefix_kind_signature_or_key`; lineage/branch/revert protected-effect negative |
| object namespace and materialized type agree with the caller's expected artifact (§0 schemas) | `w14_object_prefix_matches`, `w14_object_kind_matches`, and the single loader boundary | four-type positive matrix plus mistyped manifest/capability/tool protected-effect negatives; per-kind body schemas remain G11/W-6 |
| manifest roots and parent lineage are fabric-signed, content-addressed authority inputs (§3, §8.1, §8.3) | `Fabric::load_manifest`; `step_boundary_with_mode`, `create_branch`, and `revert_to` consume it | `w14_unverified_manifest_cannot_extend_lineage_or_mutate_state` proves no new object/event, branch substitution, or live-store restore |
| promotion is the only live mutation and uses the manifest's signed base (§5.3) | `gate_replay_check`, `promote_manifest`, `gate_trace_check`, and `approve_promotion` call `Fabric::load_manifest` before applying manifest fields | `w14_mistyped_manifest_cannot_promote_branch_state` proves no trunk file or promotion event |
| coherent revert restores every manifest root together (§5.3) | `revert_to` routes through `commit_state_change`: forward/rollback preparation, signed event + expected roots in one DB transaction, authenticated journal, all-store fsync, then DB commit; reopen chooses rollback/forward from the exact linked event | corrupt/unverified prepare negatives; later-store ordinary rollback; forced event failure for revert and promotion; reopen without/with committed event proves exact all-root rollback/roll-forward |
| broker-minted capability identity and ancestry verify fail-closed (§5 F1, §5.4.4, §8.1) | `Broker::load_capability`; decision, attenuation, revoke, ancestry, gate replay, and escalation resolution use the typed boundary | mistyped capability cannot dispatch, escalate, or receive approval authority; existing F1/tamper and A22 ancestry contracts remain green |
| approval strength cannot be hidden by unsigned object classification (§5.1, §5.4 materialized-view rule) | `check_min_auth_for_manifest` discovers ids from verified signed grants and typed-loads every referenced capability | direct `w14_mistyped_capability_cannot_weaken_promotion_approval_strength` proves no promotion/approval/trunk mutation; the escalation-resolution sibling remains covered separately |
| meters and approvals cannot arise from unsigned state (§5.1, §5.4) | `w14_decision_authority`, `w14_reserve_signed_checks`, `w14_pending_escalation`; approval event and cache share one transaction | injected/reset meter and exemption rows cannot dispatch; forced approval append failure leaves no cache, signed authority, ticket, or sentinel |
| parked human approval binds the exact candidate (§5.3 promotion/approval) | `w14_promotion_binding`, signed promotion escalation fields, `w14_verify_promotion_candidate`; approval repeats the digest and shares the promotion DB transaction | swapping two valid pending rows fails before drift or mutation; honest destructive candidate still approves and applies |
| a callable tool requires the verified registered tool object (§4, §5.4 liveness prerequisite) | `registered_tools` combines the W-11 signed placement predicate with `load_verified_object(..., "tool", "tool", ...)` | `w14_mistyped_registered_tool_cannot_dispatch` proves the protected tool effect is absent |
| advertised tools require current capability authority (§4, §5.4 liveness prerequisite) | `current_advertisable_capability` + `filter_tools_result` require typed cap, current M2, expiry, grant, ancestry, and no revoke before static caveat filtering | direct tools/list negatives after signed revoke and without a verified grant expose no tools; process positive advertises the live allowed set |
| channel registrations use the `chan` namespace and fabric signer (§8.1) | the shared loader's four-type positive/rejection matrix pins the channel boundary | SI-23/W-18 remains explicit: no current approval/identity consumer authenticates holder, sender, or user presence, so W-14 claims no channel-proof enforcement |
| interim SHA-256 state roots resolve only to bytes matching their address (F2 direction) | `Cas::get`; filesystem and SQLite plans retain verified content; `write_atomic_verified` uses an exclusive random no-follow sibling, rehashes through the retained handle, verifies the pathname still names that exact inode immediately before rename, and fsyncs publication | missing/present-corrupt negatives, FS/SQLite post-prepare substitution positives, and regular-file/symlink inode substitution rejection; CID/DAG-CBOR transition remains W-6 |
| global order, completeness, rollback, home/epoch, and freshness are authenticated | not implemented by typed object reads or CAS rehashing | SI-25/RF-13/P15/W-15; W-14 does not narrow or claim that boundary |

The first independent-context review returned REQUEST CHANGES and invalidated
the original narrow evidence claim. The corrected registry spans 24 contracts,
131 contracted tests, and 93 frozen legacy tests; required CI passed all 224
workspace tests and both acceptance demos. The expanded W-14 lane covered 152
mutants across typed objects, signed runtime decision state, immutable
FS/SQLite plans, exact parked-candidate binding, recoverable multi-root commit,
direct promotion-strength aggregation, and current-authority advertisement:
140 caught, 12 compiler-unviable, zero survivors or timeouts. The deep release
lane passed 4,096 authority cases and 512 model histories. The
independent-context re-review returned APPROVE WITH NON-BLOCKING FOLLOW-UPS
(2026-07-13) and the change merged in PR #43.

Re-review residual carried forward (not a merge blocker): decision-time
authority reconstruction (`w14_decision_authority`) performs a full
`verified_events` scan per `propose_call`, compounding the per-decision
full-ledger-scan cost ADR 0006 already flags for W-15. W-15's incremental
verified-prefix caching (verify once from a trusted checkpoint, cache keyed by
terminal event) is the intended fix; W-14 does not regress correctness, only
adds to the scan W-15 must make incremental. The crash-restart budget-durability
consequence of the same event-sourced reconstruction is RF-33; the operator
listing edge is RF-34.

## W-19 G9 inverse conformance (brief §3/§5.1; spec §6)

W-19 changes the operator diagnostic consumer, not authority reconstruction.
The deliberate difference is load-bearing: unsigned state remains inert in
the authority view, but it must be visible as doubt in the ledger that tells a
human whether history is explainable.

| Normative edge | Enforcing line/function | Negative evidence or filing |
|---|---|---|
| the trace is append-only, signed, and hash-chained per span (§6) | existing SQLite triggers; `verified_event_from_row`; `verify_verified_chain`; W-19 invokes both for every diagnostic view and rejects storage order that contradicts signed per-span `seq` | malformed raw, invalid signature, selector mismatch, middle-delete, and signed-sequence-reorder cells in the W-19 unit/process matrices; existing TRACE-CHAIN tests |
| event envelope fields and the closed event-kind vocabulary are signed (§6) | strict `parse_fabric_json`; `verified_event_from_row` checks all seven materialized fields; `w19_event_is_known_kind`; `w19_event_has_object_body` | W-19 unit matrix includes signed unknown kind and non-object body; selector matrix remains seven-dimensional |
| the trace feeds audit/forensic replay and the ledger cannot silently lie (brief §3/§5.1) | `ledger_event_view` represents every retained row as either a verified event or an explicit `TraceIntegrityFinding`; `Fabric::explain` calls `w19_require_clean` before folding roots | process negative proves invalid, mismatched, malformed, and broken-chain rows fail loudly rather than produce “explained” accounting |
| diagnostic doubt already present in the observed view creates no protected effect | CLI calls `ledger_event_view`/`w19_require_clean` before `check_drift`; `Fabric::explain` gates before `snapshot::capture` | `ledger_integrity_failure_is_loud_and_preserves_home` compares the complete home byte-for-byte after both listing and raw-detail attempts; a concurrent post-view replacement remains SI-25 freshness, not a transactionality claim |
| decodable retained records remain inspectable (brief neutrality/exportability; PR #35 ledger contract) | `LedgerEventView.records` carries exact raw text from the same SQLite statement snapshot; clean JSON renders normally, malformed text renders escaped; process exits non-zero on any finding | each process-negative cell proves non-empty detail output plus failing status and unchanged home; incompatible SQLite storage types reject the view before accounting but may prevent raw-detail rendering |
| register/intent/grant/revoke placement and authority semantics (§6) | unchanged W-11 `VerifiedEvent` consumers and placement predicates | W-11 conformance table and authority protected-effect matrix; W-19 neither grants nor closes authority |
| kind-specific body semantics beyond the signed envelope (§6) | current emitters and use-specific consumers validate the fields they require; W-19 adds only the universal object-body floor | G11/W-6: no universal per-kind schema validator or language-neutral negative corpus exists yet; no stronger conformance claim |
| structural retention and shred semantics (§6) | unchanged payload/shred implementation | SI-28/SI-29/P13 remain the explicit envelope, atomicity, generation, and forensic-erasure filings |
| global cross-span offset order, expected head, completeness, rollback, and freshness | not claimed; W-19 labels offset/span as forensic locations, but rejects an offset order that contradicts already-signed within-span `seq` | SI-25/RF-13/P15/W-15 |

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
- W-19's final `./scripts/ci required` run passed outside the socket-restricted
  sandbox: strict Clippy, 18 contracts with 95 registered tests and 95 frozen
  legacy tests, all 190 workspace tests, and both demos. Its deep release lane
  passed with 4,096 authority cases and 512 model histories.
- W-19's targeted diagnostic-view mutation lane tested 17 mutants: 14 caught,
  three compiler-unviable, zero survivors or timeouts. Iterative runs exposed
  and corrected a missing middle-deletion location assertion and an unsigned
  offset/signed-sequence accounting gap before publication.
- W-14's first submitted lane passed with 20 contracts / 105 registered tests /
  94 frozen tests, 199 workspace tests, both demos, 17 caught mutants plus one
  compiler-unviable, and the deep generated suites. Independent review proved
  those claims too narrow and returned REQUEST CHANGES. The corrected run
  replaces—not adds to—that evidence: 24 contracts / 131 registered / 93
  frozen, 224 workspace tests plus both demos, 140 caught mutants plus 12
  compiler-unviable with zero survivors/timeouts, and green 4,096-case / 512-
  history deep suites. The independent re-review returned APPROVE WITH
  NON-BLOCKING FOLLOW-UPS (RF-33/RF-34) and W-14 merged in PR #43.
- W-11 merged in PR #36, W-10 in PR #37, W-12 in PR #39, W-13 in PR #40,
  and W-19 in PR #41. The remaining findings and spec decisions stay queued in
  the canonical RF/SI/P/G/W trackers rather than being implied complete here.

## Review limits

This was source review, dependency/source inspection, standards comparison,
adversarial local reproduction, and existing-lane execution. It was not a
formal proof, hardware side-channel assessment, forensic-media experiment,
FIPS validation, or independent third-party audit. W-11 completed the
repository's independent-context review discipline. Per `AGENTS.md`, every
authority-surface change requires independent-context review before merge;
trace, key, and envelope mechanism changes should receive the same treatment,
and become mandatory whenever they alter an authority surface.
