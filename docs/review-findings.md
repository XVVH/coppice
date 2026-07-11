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

## RF-4 — Ed25519 verified non-strict (`verify`, not `verify_strict`) (#3) — fixed

**Fixed in `5350010`.** Canonical object verification now uses
`VerifyingKey::verify_strict`; the existing round-trip and tamper tests exercise
the common verification boundary.

**Severity: low (hygiene). Direction: n/a in current design.**
[canon.rs:136](crates/asf-kernel/src/canon.rs:136) uses `vk.verify`, which
admits signature malleability (non-canonical S, small-order components).
**No exploit in the current design:** the signature is excluded from the
object `id`, and records are keyed by id, so a malleated signature yields the
same id and cannot forge a distinct record. Worth fixing for cross-boundary
verification (ledger export, multi-actor) and as defense-in-depth.

**Fix:** `verify_strict`. One-line change; add a malleability test vector.

## RF-5 — keystore dir perms, chmod-after-write race, no zeroization (#5) — fixed

**Fixed in `5350010`.** Keystore directories are forced to `0700`; private
files are fully written at `0600` before atomic no-overwrite publication;
concurrent first starts converge on one key; dalek zeroization is enabled; and
local KEK/DEK/raw-key buffers use `Zeroizing`. The same pass makes the fabric
home and CAS private and restricts the approval socket to `0600`.

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

## RF-8 — `materialize_fs` does not re-validate entry paths (#8) — fixed

**Fixed in `5350010`.** Loaded tree objects now require safe relative paths,
valid entry shapes and modes, and no duplicate paths before materialization or
composition. A crafted `../` tree regression test proves the destination
cannot be escaped.

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

## RF-9 — EOF-triggered promotion is unreachable under real MCP clients — fixed

**Fixed** in the RF-9 recovery PR: sessions write a `session_live:<manifest>`
meta marker at bootstrap, cleared only when the gate runs; bootstrap (and the
new `asf recover --home … --vault … [man:… …]`) gates any marker whose pid is
dead — SIGKILL/power-loss safe. A signal handler (SIGTERM/SIGINT/SIGHUP)
additionally runs the gate before exit so the common shutdown lands work
immediately. Tests: `sigkilled_session_is_recovered_by_next_bootstrap`,
`sigterm_runs_the_gate_before_exit` (proxy_smoke).

**Severity: high. Direction: FAIL-CLOSED (work stranded, not lost).**
Found by dogfooding, first real-client session (2026-07-09). The promotion
gate ran only after the proxy's stdin read-loop returned EOF
(`proxy.rs`), but real MCP clients don't grant a graceful EOF: Claude Code
(via the MCP TypeScript SDK stdio transport) kills the server process on
shutdown. Result: every real session's branch was stranded — writes reported
as successful, never promoted, no ledger record of the non-promotion, and no
CLI to gate an orphaned branch after the fact. The scripted smoke test
passed because piped stdin closes cleanly — a client-fidelity gap in the
test harness. Residual hardening in `5350010`: promotion now requires every
candidate branch root to equal the final signed
`tool_call.state_root_after` (or the manifest base when no call was recorded).
A call that mutates after result recording is cut off therefore cannot smuggle
that untraced state into trunk. PID-reuse against the liveness check reads a
reused pid as dead only if the new process isn't an `asf` invocation —
recovery of a genuinely live session is prevented by the args match.

---

## RF-10 — promotion rewrites the whole store; every mtime clobbered — fixed

**Fixed** in the dogfooding-round-2 PR: fs restores now apply IN PLACE —
only files whose content differs are written (tmp+rename per file),
deletions pruned, emptied dirs removed; unchanged files keep mtimes and
inodes. The prepare-all-then-commit-all contract survives: prepare
validates tree parse + CAS blob presence for every store before any store
is touched. Sqlite keeps the staging swap (single file). Test:
`fs_restore_leaves_unchanged_files_untouched`.

**Severity: medium. Direction: fidelity/legibility (no data loss).**
Found by dogfooding DF-P1 (2026-07-09): after a session promoted, every
file in the vault carried the same fresh mtime. `commit_restore` rebuilt
the entire store from CAS into staging and renamed it into place — sound
for crash-safety, but it made every promotion read as a full-vault rewrite
to humans, sync clients, backup tools, and mtime-sorted note UIs.
Residual: a crash mid-apply leaves a mixed-but-valid tree; the apply is
idempotent and drift detection attributes leftovers, but the multi-file
atomic swap property is traded away for fidelity.

---

## RF-11 — the vault's `.git` was inside the store boundary — fixed

**Fixed** in the dogfooding-round-2 PR: `EXCLUDED_DIRS = [".git"]` —
excluded from `capture_fs` (so git activity no longer moves the state
root), invisible on branches (the downstream never sees the backstop),
and never deleted or rewritten by restore. Test:
`git_dir_is_outside_the_store_boundary`.

**Severity: medium. Direction: entanglement (backstop inside the system it
backstops).** Found while diagnosing DF-P1: `capture_fs` walked everything,
so the git-backed corpus's `.git` was captured in every snapshot, its
churn moved state roots (operator `git commit` = spurious drift), and
promotions rewrote git's internals from the branch copy. The out-of-band
undo of last resort must live outside the fabric's byte-boundary.
Note: on existing fabric homes the first post-upgrade check reports one
quiet root change (root recomputed without `.git`) — expected, once.

---

## RF-12 — parked promotions are illegible: "ESCALATE #null", stderr-only — fixed

**Fixed** in the dogfooding-round-2 PR: promotion-policy escalations now
render as `PARKED promotion #N — ops […], N conflict(s); resolve via
asf approve … promotions`. Test: ledger assertion in
`si20_midsession_edit_to_branch_touched_path_parks_as_conflict`.

**Severity: low code / high UX. Direction: legibility.** Found by
dogfooding DF-P3 (2026-07-09): a correctly-parked move promotion read as
"FAIL: promotion did not move the file". Two causes: the ledger renderer
read `body["escalation"]` where promotion escalations carry
`body["promotion"]` (hence `#null`), and showed none of the ops preview —
so the operator could not see that classification had in fact produced a
correct `move` op, nor that the park was policy (§5.3 default: auto-apply
only add/modify), nor how to resolve it. The park itself was RIGHT; the
system just failed to say so. Block-nothing-**loudly** requires the loud
part.

---

## RF-13 — substrate offsets are index metadata, not signature-bound; cross-span ordering claims rest on them — open

**Severity: low (defanged by the current trust boundary). Direction:
FAIL-OPEN at future boundaries (export, backups, multi-actor); fail-closed
griefing possible locally.** Found during the A21 adversarial-coverage
review (2026-07-10). Event `offset` is the sqlite rowid: it appears in no
signed body and — unlike `span`/`seq`/`kind`/`id` — is not cross-checked
against the signed object by `verify_span`
([trace.rs:266](crates/asf-kernel/src/trace.rs:266));
`events_in_span` iterates `ORDER BY seq`, so within-span rowid order is
never compared to chain order either. Every cross-span ordering claim
therefore rests on unattested storage metadata: M7's grant-before-effect
check (A21), drift `between` windows (SI-8), intent `captured_before`
anchors (SI-10 — the countersigning event's offset is the proof's anchor),
and the gate's approval-headroom reads.

**Failure scenario:** an adversary with database write access who first
drops the append-only triggers (the trigger-bypass scenario the suite
already exercises for content tamper) renumbers rowids without touching a
signed byte: no chain breaks, no signature fails. A grant renumbered
"before" a call it actually followed launders an M7 ordering violation
(fail-open); renumbered "after", it makes an honest run unpromotable
(fail-closed griefing); drift windows and `captured_before` anchors shift
arbitrarily.

**Why not currently exploitable:** on a single-user fabric home, the access
that writes `fabric.db` also reads `keys/` (same account) — that adversary
holds the fabric key and can forge signed events outright, which no
ordering hardening prevents. The same-user boundary is the audit's accepted
dogfooding limitation. This finding matters exactly where that assumption
breaks: exported ledgers verified by third parties, backups/replicas
rewritable offline, and multi-actor deployments where DB access and key
custody separate.

**Fix directions** (bundle with the deferred trace-head-anchoring release
gate — A21 makes M7 a new consumer of it):
1. Cheap now: `verify_span` additionally checks within-span offset
   monotonicity against seq. Catches renumbering that inverts a span's
   internal order; cross-span-only reorders that preserve each span's
   internal order still escape.
2. Real fix: attest global order — a single per-home substrate chain
   interleaving all spans (offset becomes a chained seq), or periodic
   signed anchor events committing `(span, seq) → offset` mappings (the
   §6 analog of SI-10's countersign pattern).
3. Export rule regardless: exported evidence bundles carry chain order,
   never rowids, as the ordering authority.

---

## RF-14 — key and credential material is stored in cleartext at rest; confidentiality rests solely on filesystem permissions — open

**Severity: medium-high. Direction: EXPOSURE (not authority) — but the
exposure is the crown jewels.** Surfaced by the posture-assumptions sweep
(2026-07-11). Two cleartext files under `keys/`, protected only by 0600 in
a 0700 dir, no encryption:

1. **`secrets.json`** — the real credentials the broker injects into tool
   calls, stored as plaintext JSON ([keys.rs:107-119](crates/asf-kernel/src/keys.rs:107),
   `secret_set`/`secret_get`). Unlike payloads, these are **never**
   KEK-wrapped. The whole "agent never holds the real key" property
   protects the *agent context*; it does not protect the *disk*.
2. **`owner.kek`** — the owner KEK as a raw 32-byte cleartext file
   ([keys.rs:133-141](crates/asf-kernel/src/keys.rs:133)), sitting beside
   the ciphertext and DB it protects. Consequence: the payload AES-256-GCM
   encryption we *did* build ([payload.rs:148](crates/asf-kernel/src/payload.rs:148))
   provides **zero** at-rest confidentiality against any reader who can
   open `fabric.db` — because that same reader holds `keys/` — and
   crypto-shredding is defeated by KEK survival (the DEKs are wrapped to a
   key that never leaves the disk).

**Failure scenario:** any of — a second local uid, a stolen disk or
laptop, an unencrypted backup, a multi-tenant host, or a same-uid
compromised process with a shell — reads live credentials and the master
key directly. Every payload ever encrypted, and every future one, is
readable; every shred is reversible.

**Why not currently exploitable:** same-Unix-user + single-tenant + debug
posture (the accepted dogfooding trust boundary). It bites the moment any
of those relaxes.

**Prior tracking:** UNTRACKED as a confidentiality finding. RF-5 covered
only the *directory perms leaking `secrets.json`'s filename*; the audit
mentioned `keys.rs:114` only as *forensic residue* under crypto-shredding.
Neither files the plaintext storage of the secret bodies or the KEK, nor
the fact that the KEK-beside-ciphertext nullifies the payload encryption.

**Fix directions** (post-dogfooding graduation gate — bundled with forensic
crypto-shredding, which is the same root cause): OS keychain / secure
enclave custody for the KEK and secrets; or a passphrase/hardware-derived
KEK never written in cleartext; or remote key custody (the scalability
analysis's unbuilt fleet machinery). Tracked in the posture-assumptions
ledger under the multi-user / multi-tenant / production graduation gates.

## RF-15 — fabric-home state at rest (CAS, branches, `fabric.db`) is plaintext; confidentiality rests solely on filesystem permissions — open

**Severity: medium. Direction: EXPOSURE.** Surfaced by the
posture-assumptions sweep (2026-07-11); this is the RF-5 *expansion the
audit explicitly asked for and that was never filed*
([security audit](docs/security-correctness-audit-2026-07-09.md), "RF-5
should be expanded to cover plaintext fabric home, CAS, branches"). CAS
blobs ([snapshot.rs:85](crates/asf-kernel/src/snapshot.rs:85),
[:223](crates/asf-kernel/src/snapshot.rs:223)), materialized session
branches, and `fabric.db` (events, signed objects, and payload ciphertext)
are stored in the clear; confidentiality rests entirely on the forced
0700/0600 perms on the home. Distinct from RF-14 (that is keys/secrets —
the crown jewels; this is the user's own content and trace).

**Failure scenario:** traversable parent dirs, a second local uid, disk
theft, or a multi-tenant host expose the full vault content, branch working
state, and trace. Note the payload ciphertext in `fabric.db` is only as
protected as RF-14's KEK — i.e. not, at rest.

**Prior tracking:** the *perms hardening* was done (audit resolved 0700/0600
across the home); the *plaintext-at-rest* residual was flagged for an RF-5
expansion that never happened. NOTED-NOT-FILED until now.

**Fix directions:** at-rest encryption of the fabric home (envelope
encryption under RF-14's custody fix), or full-disk/OS-level encryption as
the deployment floor. Same graduation gates as RF-14.

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
