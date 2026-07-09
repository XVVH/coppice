# Review findings — implementation issues

Tracked defects and hardening items found in code review (distinct from
`spec-issues.md`, which tracks *spec* ambiguities). IDs are `RF-n`. Source:
the self-review pass over commit `1eec9e3` (Stage 1–3, 2026-07-09), with a
second adversarial pass that re-ranked by *failure direction* and corrected
two severities (RF-1 upgraded, RF-4 downgraded, RF-6 zeroize claim verified).

No GitHub remote / issue tracker is configured; this file is the tracker.
Ordering below is **action priority** (fail-open before fail-closed before
hygiene), not ID order. `(#n)` cross-references the original review numbering.

Status legend: **open** · **in-progress** · **fixed** (cite commit) ·
**wontfix** (cite rationale) · **accepted** (documented property, no code change).

Severity × direction: a *fail-open* bug grants authority it shouldn't;
a *fail-closed* bug denies authority it should grant (safe, but breaks
workflows). Fail-open ranks above fail-closed at equal blast radius.

---

## RF-1 — time bounds compared as strings; fail-open at the boundary (#7) — fixed

**Fixed** in the RF-1 timestamp PR. Added `asf_kernel::parse_instant`
(RFC 3339 → `OffsetDateTime`); expiry, the `time` caveat, and the
attenuation `time` subset now compare instants, with unparseable timestamps
failing closed. Tests: `time_caveat_no_longer_fails_open_at_subsecond_boundary`
and `unparseable_time_bound_fails_closed` (evaluate), plus property `p6`
(instant compare always matches chronology; crafted same-second pairs prove
lexical comparison does not — guarding against a regression to string compare).

**Severity: medium. Direction: FAIL-OPEN.**
`evaluate.rs` compares RFC 3339 timestamps lexically:
[expires_at](crates/asf-kernel/src/evaluate.rs:76),
[not_before / not_after](crates/asf-kernel/src/evaluate.rs:190). The `time`
crate emits *variable-width* subseconds (`…00Z` vs `…00.5Z`), and lexical
order disagrees with chronological order because `'Z'` (0x5A) > `'.'` (0x2E).
Verified empirically: `"…00Z" <= "…00.5Z"` returns `false` though the
whole-second instant is earlier.

**Failure scenario:** capability with `not_after: "2026-07-09T12:00:00Z"`
(whole second — the natural human/config value). A call at
`now = "2026-07-09T12:00:00.3Z"` evaluates `now <= not_after` as
`".3Z" < "Z"` → **true** → the call ~0.3 s past the deadline is admitted.
Same root cause admits calls ~1 s past `expires_at`, and lets an attenuation
`time` subset ([capability.rs:222](crates/asf-kernel/src/capability.rs:222))
verify a child window that slightly exceeds its parent. Blast radius is
bounded to a <1 s window at each boundary, hence medium not high — but it is
the only *fail-open* finding.

**Fix:** parse both sides to `OffsetDateTime` and compare instants; never
compare RFC 3339 as strings. Audit every `now <=`/`>=`/`<`/`>` on timestamps
(evaluate.rs, capability.rs `time` dim). Add a property test: for random
instant pairs, string-compare and instant-compare must agree — it currently
fails.

## RF-2 — approval exemption consumed even when the call is denied (#1) — fixed

**Fixed** in the RF-2/RF-3 broker-consumption PR. `evaluate` is now
side-effect-free — the `exempt` closure *peeks* (no `UPDATE`) and reports
`consumed_exemptions`; the broker decrements only in the `Outcome::Allow`
arm, inside one transaction with the meter bump. Regression test:
`exemption_survives_a_denied_call` (a call denied on a non-escalatable
caveat leaves an approved exemption intact for a later legitimate call).

**Severity: medium. Direction: fail-closed (agent loses granted authority).**
In `propose_call` the `exempt` closure runs the
[`UPDATE exemptions SET remaining = remaining - 1`](crates/asf-kernel/src/broker.rs:287)
*inside* `evaluate()`, per failing escalatable check, committed immediately —
before the aggregate `Allow/Deny/Escalate` outcome is known and with no
rollback.

**Failure scenario:** agent holds 1 approved budget exemption; it issues a
write that is both over budget (escalatable, exemption applies) *and*
out-of-scope path (hard deny). Outcome is correctly `Deny`, but the exemption
is already spent, so the agent's next *legitimate* over-budget write parks
again. Griefable; a human-granted approval is silently drained by a call that
never executed. The meter-write error path leaks identically.

**Fix:** make `evaluate` *report* which exemptions it would consume (data,
not side effect); commit the decrement only on `Outcome::Allow`, in the same
transaction as the meter bump (see RF-3).

## RF-3 — meters/exemptions consumed at decision time, not execution time (#2) — fixed (as accepted residual)

**Severity: medium-low. Direction: fail-closed (agent loses budget).**
Meter bump and exemption decrement happen in `propose_call`; the `tool_call`
ledger event is written only in `record_result`. If a call is `Allowed` but
never recorded (downstream error, agent disconnect, crash between the two),
budget is spent with no corresponding trace event — runtime meter and signed
ledger diverge. Enforcement stays correct because the promotion gate recounts
budgets from trace events, not the meter.

**Correction to the original fix idea.** The first writeup proposed
"commit consumption in `record_result`." On implementation that is **wrong**:
deferring consumption past the decision lets two calls proposed before either
records both read the same pre-consumption meter and both pass the same
budget — turning a fail-*closed* bug into a fail-*open* one under pipelined
proposes. Rejected.

**What was done instead.** Consumption stays at decision time but is now
(a) committed only on `Outcome::Allow` and (b) atomic (meter bump + exemption
decrement in one transaction) — see RF-2. This keeps the meter monotonic and
never over-grants. The residual — an Allowed-but-never-recorded call
over-counts budget by one — is the **fail-safe** direction and is reconciled
by the gate's ledger-based recount. Accepted as designed; no further change.

## RF-4 — Ed25519 verified non-strict (`verify`, not `verify_strict`) (#3) — open

**Severity: low (hygiene). Direction: n/a in current design.**
[canon.rs:136](crates/asf-kernel/src/canon.rs:136) uses `vk.verify`, which
admits signature malleability (non-canonical S, small-order components).
**No exploit in the current design:** the signature is excluded from the
object `id`, and records are keyed by id, so a malleated signature yields the
same id and cannot forge a distinct record. Worth fixing for cross-boundary
verification (ledger export, multi-actor) and as defense-in-depth.

**Fix:** `verify_strict`. One-line change; add a malleability test vector.

## RF-5 — keystore dir perms, chmod-after-write race, no zeroization (#5) — open

**Severity: low. Direction: exposure, not authority.**
Three sub-items in `keys.rs`:
1. [`create_dir_all`](crates/asf-kernel/src/keys.rs:59) uses umask-default
   dir mode (~0755): the keystore *directory* is world-traversable, leaking
   filenames/sizes/existence of `secrets.json` even though file contents are
   0600. → create the dir `0700`.
2. [`write_private`](crates/asf-kernel/src/keys.rs:125) writes then chmods
   0600 — a window where the private key exists at umask-default perms. →
   O_EXCL create at mode 0600, or 0600 temp + rename.
3. No zeroization of key material. Verified: dalek 2.2.0 gates zeroize behind
   `#[cfg(feature = "zeroize")]` and
   [Cargo.toml](Cargo.toml:13) enables only `rand_core`, so `SigningKey` does
   **not** zeroize on drop; our own buffers (`Vec<u8>`/`[u8;32]` in
   `load_or_create_raw`, `kek`, `new_dek`, `WrappedDek.dek`) never do. →
   enable the dalek `zeroize` feature; wrap our buffers in `Zeroizing`.

## RF-6 — object id type-prefix is not covered by the signature (#4) — open

**Severity: low (theoretical). Direction: type confusion, currently defanged.**
The id is `<prefix>:<hash(body)>` but only body bytes are signed, and
[`verify` derives the prefix from the claimed id itself](crates/asf-kernel/src/canon.rs:103).
The same signed body presented under a different prefix (`man:` vs `cap:`)
still verifies. Practically defanged: objects are looked up by full id and
downstream code reads type-specific fields, so a manifest-as-capability has
no `caveats`/`expires_at` and hits the structural fail-closed deny. No known
exploit; a latent seam.

**Fix:** include the type/prefix inside the signed body (e.g. a `type` field),
or sign `prefix ‖ body`. Bundle with RF-4 as a canon.rs hardening pass.

## RF-7 — plaintext-hash addressing is a confirmation oracle surviving shred (#6) — accepted

**Severity: informational. Spec-sanctioned property, no code change.**
Payloads are keyed by [`sha256(plaintext)`](crates/asf-kernel/src/payload.rs:74)
and the hash persists after crypto-shredding (§1: structure persists,
substance doesn't). Anyone with read access to `payloads`/`tombstones` can
confirm a *guessable* payload's past presence by hashing a candidate, even
post-shred. Inherent to plaintext content-addressing.

**Action:** document the limit explicitly in the spec/threat model —
"shredding destroys content, not the ability to confirm a known plaintext was
once present." A salted/keyed address would close the oracle at the cost of
cross-object dedup; revisit only for a high-sensitivity payload class.

## RF-8 — `materialize_fs` does not re-validate entry paths (#8) — open

**Severity: low. Direction: defense-in-depth (local trust boundary).**
[materialize_fs](crates/asf-kernel/src/snapshot.rs:203) joins each tree
entry's `path` onto the destination
([dest.join(rel)](crates/asf-kernel/src/snapshot.rs:214)) without rejecting
absolute or `..` components. `capture_fs` never produces such paths, so this
only bites on a corrupted or hand-crafted CAS tree object — inside the local
trust boundary — but a crafted tree could write outside the branch/staging
dir during materialize/restore.

**Fix:** validate each `rel` (reject leading `/` and any `..` segment) before
join, mirroring `asf-cli`'s `vault_server::safe_join`.

---

## Verified sound during review (recorded so they aren't re-litigated)

- Per-payload DEKs each perform exactly one encryption → no GCM nonce reuse
  on payload data.
- Credential injection stores pre-injection args (`args_raw`); the secret
  reaches only the forwarded child-stdin copy, never the payload store.
- The promotion gate recounts budgets from signed trace events, not the
  runtime meter — so RF-2/RF-3 meter drift cannot defeat enforcement.
- Trace append-only is enforced by triggers *and* by signature recomputation
  on verify — trigger-bypass tampering is caught.
- Attenuation subset logic + fail-closed unknown dimensions are solid and
  property-tested (P5: no privilege escalation across generated pairs).
