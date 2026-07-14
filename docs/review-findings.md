# Review findings — implementation issues

Tracked defects and hardening items found in code review (distinct from
`spec-issues.md`, which tracks *spec* ambiguities). IDs are `RF-n`. Source:
the self-review pass over commit `1eec9e3` (Stage 1–3, 2026-07-09), with a
second adversarial pass that re-ranked by *failure direction* and corrected
two severities (RF-1 upgraded, RF-4 downgraded, RF-6 zeroize claim verified),
plus the 2026-07-12 cryptographic mechanism audit and its first
independent-context W-11 review (RF-16…RF-27).

This file is the canonical tracker. GitHub issues/PRs may mirror an RF id, but
must link back rather than becoming a second source of truth; no external issue
was created in this audit session. Ordering below is **action priority**
(fail-open before fail-closed before hygiene), not ID order. `(#n)`
cross-references the original review numbering.

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
   calls, stored as plaintext JSON ([keys.rs:107-128](../crates/asf-kernel/src/keys.rs#L107-L128),
   `secret_set`/`secret_get`). Unlike payloads, these are **never**
   KEK-wrapped. The whole "agent never holds the real key" property
   protects the *agent context*; it does not protect the *disk*.
2. **`owner.kek`** — the owner KEK as a raw 32-byte cleartext file
   ([keys.rs:131-141](../crates/asf-kernel/src/keys.rs#L131-L141)), sitting beside
   the ciphertext and DB it protects. Consequence: the payload AES-256-GCM
   encryption we *did* build ([payload.rs:130-155](../crates/asf-kernel/src/payload.rs#L130-L155))
   provides **zero** at-rest confidentiality against any reader who can
   copy both `fabric.db` and `keys/`. This is distinct from logical shred:
   shred deletes the live wrapped DEK and clears the live ciphertext
   ([payload.rs:172-197](../crates/asf-kernel/src/payload.rs#L172-L197)), so
   the KEK alone cannot reconstruct a random destroyed DEK. The forensic
   gap is conditional but real: if WAL/freelist residue, snapshots, or
   backups retain both an old ciphertext and its wrapped DEK, the persistent
   KEK makes that recovered pair decryptable.

**Failure scenario:** any actor that bypasses or legitimately holds the
filesystem boundary — a same-uid compromised process, privileged host
compromise, an unprotected offline disk, or an unencrypted backup containing
both the database and keys — reads live credentials and decrypts live
payloads. A shredded payload is recoverable only where that actor also finds
residual or historical copies of both its ciphertext and wrapped DEK. A
separate unprivileged Unix uid is blocked by the enforced 0700/0600 modes;
multi-user deployment does not by itself bypass that boundary.

**Why accepted for current dogfooding:** same-Unix-user + single-tenant +
debug posture explicitly excludes hostile same-uid processes and offline
storage/backup compromise. The finding bites when those threats enter scope,
or when tenants share one uid or fabric home; isolated per-uid homes retain
the present filesystem boundary.

**Prior tracking:** UNTRACKED as a confidentiality finding. RF-5 covered
only the *directory perms leaking `secrets.json`'s filename*; the audit
mentioned `keys.rs:114` only as *forensic residue* under crypto-shredding.
Neither files the plaintext storage of the secret bodies or the KEK, nor
the fact that the KEK-beside-ciphertext nullifies live-payload encryption
against a copied fabric home.

**Fix directions** (post-dogfooding graduation gate, coordinated with
forensic crypto-shredding because key custody controls whether recovered
wrapped DEKs remain useful): OS keychain / secure
enclave custody for the KEK and secrets; or a passphrase/hardware-derived
KEK never written in cleartext; or remote key custody (the scalability
analysis's unbuilt fleet machinery). Tracked in the posture-assumptions
ledger under the multi-user / multi-tenant / production graduation gates.

## RF-15 — fabric-home state at rest (CAS, branches, `fabric.db`) is plaintext; confidentiality rests solely on filesystem permissions — open

**Severity: medium. Direction: EXPOSURE.** Surfaced by the
posture-assumptions sweep (2026-07-11); this is the RF-5 *expansion the
audit explicitly asked for and that was never filed*
([security audit](security-correctness-audit-2026-07-09.md), "RF-5
should be expanded to cover plaintext fabric home, CAS, branches"). CAS
blobs ([snapshot.rs:91-100](../crates/asf-kernel/src/snapshot.rs#L91-L100),
[:176-247](../crates/asf-kernel/src/snapshot.rs#L176-L247)), materialized session
branches, and `fabric.db` (events, signed objects, and payload ciphertext)
are stored in the clear; confidentiality rests entirely on the forced
0700/0600 perms on the home. Distinct from RF-14 (that is keys/secrets —
the crown jewels; this is the user's own content and trace).

**Failure scenario:** a raw disk/backup copy, privileged or same-uid
compromise, or tenants deliberately co-located inside one uid/home expose the
full vault content, branch working state, and trace. The enforced home modes
protect against an ordinary second uid, and separate per-uid fabric homes
preserve that boundary. Note the payload ciphertext in `fabric.db` is only as
protected against an offline copy as RF-14's colocated KEK.

**Prior tracking:** the *perms hardening* was done (audit resolved 0700/0600
across the home); the *plaintext-at-rest* residual was flagged for an RF-5
expansion that never happened. NOTED-NOT-FILED until now.

**Fix directions:** at-rest encryption of the fabric home (envelope
encryption under RF-14's custody fix), or full-disk/OS-level encryption as
the offline-storage deployment floor. Shared-home tenancy additionally needs
tenant-scoped authorization and storage isolation; encryption alone does not
provide it. Separate homes under separate Unix identities remain a valid
interim multi-user boundary.

---

## RF-16 — unsigned event indexes can conceal signed revocation and alter replay clocks — fixed (452c9bc, W-11)

**Severity: high under the current no-actuation posture; critical before live
egress. Direction: FAIL-OPEN.** Found and reproduced in the 2026-07-12
cryptographic mechanism audit. `events_of_kinds` filters on the unsigned
SQLite `kind` column before verification; the A22 view branches on that row
value while verifying a separate signed `raw` object. Decision-time liveness
does not first establish a fully verified event set. Gate replay additionally
uses the unsigned row `at`, which `verify_span` does not cross-check.

**Reproduction:** append a valid signed revoke, bypass the append-only trigger,
then change only its row `kind` to `grant` and `span` to another span. Signed
`raw` is unchanged and no signing key is used. `Broker::propose_call` returns
`Allowed` after the revoke. Moving a tail row also leaves the shortened
substrate chain internally valid because no expected head is anchored.

**Fix:** decode a `VerifiedEvent` from signed `raw`; require every materialized
column to agree before any filtering; derive authority and replay time only
from verified fields; evaluate against one verified, transactionally
consistent view. Add the full index-column adversary matrix and targeted A22
mutations. SI-25/RF-13 remain the larger global-order and rollback fix.

**Remediation:** `452c9bc` derives authority inputs through
`VerifiedEvent`: signed raw is verified first, all materialized columns
except the explicitly unsigned global `offset` must agree, and authority,
tool registration, ledger explanation, and gate branch-tip selection consume
the verified view. The combined concealment reproduction now denies
structurally without creating a dispatch ticket or escalation; the seven-field
matrix and targeted constructor mutation lane are green.

The first independent-context review correctly returned REQUEST CHANGES on two
additional RF-16 edges. The final implementation derives the decision head
from the same SQLite statement snapshot as its event set, so a concurrent
commit cannot race `O` ahead of the reconstructed view; and every within-span
consumer touched by W-11 replays/selects by signed `seq`, including branch-tip
selection, so swapping unsigned offsets cannot promote an older attested root.
Protected-effect regressions cover both boundaries and the W-11 consumer
mutation lane is green. SI-25 still owns cross-span offset authenticity,
completeness, signature-invalid row erasure, rollback, and freshness.
Independent-context re-review returned APPROVE WITH NON-BLOCKING FOLLOW-UPS.
The authority-review gate is satisfied; fixed by `452c9bc` and merged in PR
#36.

## RF-17 — JCS accepts signature-preserving numeric semantic collisions — fixed by W-12 (PR #39)

**Severity: high at the signed-format boundary. Direction: AUTHENTICITY.** The
spec requires integer `|n| < 2^53` and forbids floats, but `jcs_bytes`, `seal`,
and `verify` perform no recursive domain validation. The locked canonicalizer
casts `u64` to `f64`.

**Reproduction:** an ASF object sealed with integer `9007199254740992` was
mutated to exact `serde_json::Value` integer `9007199254740993`; canonical body
bytes remained equal and `canon::verify` returned `Ok(())`. This upgrades P24
from an interoperability note to a signature-authenticity defect for accepted
out-of-spec input.

**Current remediation:** `canon` recursively rejects floats and integers with
`|n| >= 2^53` before canonicalization at both seal and verify. A strict raw
fabric-object parser rejects duplicate names before constructing a
`serde_json::Value`; persisted event rows, the common object loader, and the
two broker capability scans that bypass it all use that parser. The original
adjacent-u64 collision is reproduced and denied, exact boundary positives
prove valid signed bytes unchanged, and the language-neutral G2 fixture
includes bounds, nested floats, collision inputs, and duplicates. A protected
tool-dispatch negative covers both duplicate-bearing object and event rows.
Type/domain transcript binding remains the separate RF-6/SI-26 decision.

## RF-18 — existing homes silently regenerate missing identity and KEK files — fixed by W-13 (PR #40)

**W-13 remediation:** `Fabric::initialize` and
`Fabric::open_existing` are now distinct public operations. Initialization
publishes the complete fabric/user-root/owner-KEK set as one fsynced directory
entry; reopen validates all three before opening SQLite and never creates key
material. Existing and partial homes refuse re-initialization; fabric-home,
database, CAS, and proxy runtime-store paths must already have the expected
non-symlink shape, and reopen never repairs missing state. Malformed
`secrets.json` syntax, shape, or non-string values fail without overwrite.
The `KEY-CONTINUITY` contract exercises loss, malformed material, database-only,
keys-only, mixed partial-home, missing-CAS/runtime, and path-substitution cases;
the targeted `w13_*` mutation lane caught all 35 viable mutants (five
compiler-unviable, zero survivors). SI-27/W-17 and RF-14 remain open.
The first independent-context review found three ordering/path blockers; the
corrected implementation was independently re-reviewed and approved, then
merged in PR #40.

**Severity: high. Direction: IDENTITY SPLIT / IRRECOVERABLE DATA LOSS.** The
original `load_or_create_raw` path could not distinguish first initialization
from key loss.
Removing `fabric.ed25519`, `user_root.ed25519`, or `owner.kek` from an existing
home silently creates new material. Old events then fail under a new fabric
identity and old payloads become unreadable under a new KEK. No trusted home
identity, historical key registry, rotation chain, or recovery ceremony exists;
before W-13, parent-directory entries were not explicitly fsynced after
publication.

**Required boundary (implemented by W-13):** split explicit initialization from
reopen, fail closed when any required key is absent from initialized state,
fsync directory publication, and refuse malformed secret storage rather than
treating it as empty.
SI-27 owns external trust anchoring, rotation, recovery, and historical
verification; RF-14 remains the separate cleartext-custody finding.

## RF-19 — manifests are applied without universal signature/type verification — fixed by W-14 (PR #43)

**Severity: high. Direction: INTEGRITY / UNAUTHORIZED STATE APPLICATION.**
Promotion verifies manifests, but `create_branch`, `revert_to`, and some parent
reads consume mutable object rows without recomputing the id or signature. A
database writer without the signing key can replace raw root references; CAS
hashing proves only that the attacker-selected bytes match their address, not
that the root was authorized.

**Fix:** one typed `load_verified_object` boundary taking expected prefix,
stored kind, and verifying key; every authority-bearing object read goes
through it. Negative tests must leave all live stores byte-for-byte unchanged.

**W-14 remediation:** `trace::load_verified_object` verifies strict stored raw,
signature/id recomputation, exact requested signed id, id prefix, and stored
kind before returning fields. Manifest branch, parent-lineage, revert, and
promotion consumers; capability decision, ancestry, approval, and replay
consumers; and registered-tool lookup share that boundary. Approval-strength
aggregation discovers capability ids from verified signed grants rather than
the unsigned `objects.kind` selector. The `TYPED-OBJECT` contract covers all
four prefixes (manifest/tool/capability/channel) and protected-effect negatives
for lineage, branch/revert, promotion, dispatch/escalation, approval, and tool
lookup. Required CI and the targeted mutation lane are green; the
independent-context authority re-review returned APPROVE WITH NON-BLOCKING
FOLLOW-UPS (RF-33/RF-34) and the change merged in PR #43.

## RF-20 — filesystem restore preparation checks CAS presence, not integrity — fixed by W-14 (PR #43)

**Severity: medium-high. Direction: PARTIAL DESTRUCTIVE FAILURE.** For
filesystem restores, `prepare_restore` checks only `cas.has`. Commit deletes
live files absent from the desired tree before the first `cas.get` rehash. A
corrupt referenced blob can therefore pass prepare, trigger live deletions,
and fail only during the later write pass.

**Fix:** read and hash-verify every referenced blob during prepare, preferably
staging immutable verified bytes for commit. The negative contract asserts no
live mutation when any required CAS object is missing or corrupt.

**W-14 remediation:** filesystem preparation now loads the tree and calls the
rehashing CAS read for every referenced blob, retaining the verified bytes in
the prepared plan. Commit never re-reads attacker-controlled CAS content. The
`RESTORE-INTEGRITY` contract proves prepared commits consume the staged bytes
and missing or present-but-corrupt dependencies abort coherent revert before
either the filesystem or SQLite live store is restored. The first independent
review found the SQLite sibling still retained a predictable mutable staging
pathname. The corrected SQLite plan retains the verified image bytes and CAS
address, creates an exclusive randomized sibling only at commit, rehashes it
through its retained handle, verifies the pathname still names that exact
non-symlink inode immediately before atomic rename, and fsyncs file plus
parent. A direct
post-prepare CAS and legacy-staging substitution test proves only the retained
verified image reaches the live database; a separate regular-file/symlink
substitution negative proves the live target remains unchanged. The
independent-context re-review returned APPROVE WITH NON-BLOCKING FOLLOW-UPS;
merged in PR #43.

## RF-21 — bundled SQLite 3.46.0 is affected by the WAL-reset corruption race — fixed (fa9defd, W-10)

**Severity: high for foundational integrity; low-probability occurrence, not a
remote exploit.** `rusqlite 0.32.1` with `bundled` selects
`libsqlite3-sys 0.30.1`, whose bundled header is SQLite 3.46.0. ASF explicitly
enables WAL and can open multiple process/thread connections and checkpoints.
SQLite reports the corruption race across 3.7.0 through 3.51.2, fixed in
3.51.3 and later, with separately published fixed backports at 3.44.6 and
3.50.7; ASF's 3.46.0 is not one of them. RustSec is green because this is
bundled C source rather than a RustSec advisory. Primary source:
[SQLite's WAL-reset analysis](https://www.sqlite.org/wal.html#the_wal_reset_bug).

**Fix:** upgrade to a binding that bundles a patched SQLite and enforce the
minimum runtime library version in code/CI so a dependency regression cannot
silently reintroduce the affected engine.

**Remediation:** `fa9defd` pins `rusqlite 0.40.1` with
`libsqlite3-sys 0.38.1` (bundled SQLite 3.53.2) and refuses to initialize or
open a fabric below the conservative 3.51.3 floor. The version-only guard
deliberately rejects older fixed backports because it cannot attest their patch
provenance. The two-sided `SQLITE-ENGINE` contract is green.
The stable runtime-floor mutation lane catches all 4 mutants. Merged in PR #37.

## RF-22 — payload envelope metadata is unauthenticated and put/shred is non-atomic — open

**Severity: high at the production/storage boundary.** Neither DEK wrapping
nor payload encryption binds associated data. Stored `alg` and `kek_id` are
ignored on unwrap; resolution accepts only the plaintext hash and ignores the
signed size, media type, and DEK id. DEK/payload insertion, destructive shred
steps, and the signed shred event cross separate transaction boundaries. The
long-lived KEK uses random GCM nonces with no invocation cap or rotation.

**Fix:** SI-28 defines a versioned canonical AAD/envelope and wrap lifecycle;
SI-29 resolves post-shred generations. Implementation then decrypts against a
complete expected `PayloadRef`, fails closed on unknown algorithms, and makes
put/shred crash-safe. P13/P19 and forensic storage remain release gates.

## RF-23 — a downstream tool can reflect an injected credential to the agent — open

**Severity: high before real credentials or third-party tools. Direction:
EXPOSURE.** The broker correctly excludes injected credentials from traced
arguments, but the downstream process receives the secret in its tool-call
arguments and its result is forwarded byte-for-byte to the agent. A malicious
or merely echoing adapter therefore defeats the absolute claim that the agent
never sees the real credential.

**Fix:** make the trusted credential adapter an explicit boundary; prefer
scoped/one-use downstream credentials and injection below the agent-visible
protocol. Response scrubbing can catch accidents but cannot contain an
adversarial encoder. P27 and the third-party/live-egress gates own the interim
prohibition.

## RF-24 — holder, channel, and auth-strength claims lack cryptographic proof — open

**Severity: high before actuation or multi-actor authority. Direction:
IMPERSONATION / SELF-APPROVAL.** Capability `holder` is not authenticated or
checked. Principal public keys have no proof of possession. `capture_intent`
uses the locally held user-root key to sign caller-supplied principal, channel,
and auth strength; the approval socket equates same-uid connection with
`local_session` user presence.

**Fix:** SI-23 and W-4/W-18 own authenticated user presence, holder
proof/attestation, and channel-device binding. No actuation registration or
multi-actor authority claim may precede that work.

## RF-25 — signed grant parent can disagree with capability ancestry — fixed (452c9bc, W-11)

**Severity: medium; latent FAIL-OPEN at the signer/conformance boundary.**
The first W-11 independent-context review found that activation keyed grants by
`body.capability` plus event manifest while ignoring the normative §6
`body.parent`. A signature-valid malformed child grant could therefore
activate against a different ancestry reconstructed from the capability object.
The storage-only attacker cannot mint such an event, but §5.4 requires doubtful
activation to grant nothing and G9 forbids relying on honest emission.

**Current remediation:** A22 grant bindings now include the signed parent, and
`a22_state_at` requires it to match the next capability in the verified
ancestry exactly. Missing, null-for-child, and wrong-parent grants all deny
before dispatch in `a22_malformed_grant_parent_cannot_activate_capability`.
The existing targeted `a22_*` lane catches all 29 current mutants.
Independent re-review approved the correction; fixed by `452c9bc` and merged
in PR #36.

## RF-26 — signed tool registration placement was not enforced — fixed (452c9bc, W-11)

**Severity: medium; latent FAIL-OPEN at the signer/conformance boundary.**
`registered_tools` previously accepted any signature-verified `register`
event naming a tool. Section 6 instead requires registrations to live on the
fabric-lifetime span with `manifest:null`; a malformed signed event on a
session span or under a manifest could make its tool callable.

**Current remediation:** live tool registration now requires both placement
dimensions explicitly. The two-sided TOOL-REGISTRATION contract independently
tests wrong-span and non-null-manifest cases and proves no protected dispatch
occurs. The W-11 consumer mutation lane covers the placement predicate.
Independent re-review approved the correction; fixed by `452c9bc` and merged
in PR #36.

## RF-27 — recovery preselection trusts unsigned promotion selectors — open

**Severity: low. Direction: FAIL-CLOSED AVAILABILITY.** The W-11 independent
re-review found that explicit `asf recover` preselection queries the
materialized event `kind` and `manifest` columns directly when deciding
whether a manifest already faced promotion. A storage attacker can insert or
mutate a row to look like a promotion and make recovery skip stranded work.
The later gate is not bypassed and no call or merge is authorized; an attacker
with database write access already has broader denial-of-service options.

**Fix:** derive the promotion-event half of the precheck from
`VerifiedEvent` while retaining the broker-owned promotions-table half, with
a process-level recovery negative proving the stranded branch is not skipped.
This is a non-blocking dogfooding hardening item outside W-11's authority
surface; schedule it with the next RF-9 recovery pass.

## RF-28 — operator ledger trusts or hides unverified retained rows — fixed by W-19 (PR #41)

**Severity: medium. Direction: FAIL-OPEN OPERATOR DECEPTION.**
Before W-19, `Fabric::explain` folded raw `EventRow` values into its narrative
and root accounting without checking the event signature,
signed/materialized selector agreement, or per-span chain. A storage writer
could therefore make an unsigned or invalidly signed row claim that live state
had an explaining cause, or alter the displayed kind, span, or time of a
genuine signed event. The one-line W-11-era candidate replacement with
`verified_events` was also insufficient: that authority view deliberately
omits wholly unsigned or foreign-signed rows, which would make an operator
integrity surface silently hide the anomaly.

The edit was never part of a merged PR. PR #37 explicitly excluded it as an
unrelated W-11 reporting-line change; the docs-only PR #38 excluded it again,
but the local candidate was not removed or tracked. A review of every PR
merged, opened, or updated in the preceding 24 hours (#30–#40) confirmed that
no patch or review owned it.

**Remediation:** W-19 gives reporting its own integrity view. Verified events may feed
the narrative and root accounting only when the retained view is free of
signature, selector, parse, kind, and chain findings. It also rejects unsigned
storage order that contradicts the already-signed within-span sequence,
preventing a W-11-shaped stale-tip lie. Every decodable anomalous row is
reported with enough location to inspect it, and doubt already present in the
observed view makes the CLI exit non-zero before drift attribution or CAS
capture can create a protected effect. SQLite type errors also fail before
accounting, although they may prevent raw-detail rendering. The two-sided
ledger contract and a stable targeted mutation lane cover this boundary.
SI-25/RF-13 remain explicit for unsigned cross-span offset order, deletion,
rollback, expected-head completeness, and freshness, including replacement
after a diagnostic view has already been observed.
Merged in PR #41 after the required, deep, and targeted mutation lanes passed;
GitHub's Linux, macOS, static, and audit checks also passed.

## RF-29 — unsigned broker meter/exemption caches authorize dispatch — fixed by W-14 (PR #43)

**Severity: high. Direction: FAIL-OPEN AUTHORITY.** Decision time read
`broker_meters` and `exemptions` directly. A storage writer could reset a
consumed meter or inject an exemption and receive an Allowed dispatch ticket;
gate replay was too late to prevent credential injection or the downstream
effect. Approval also inserted the exemption before its signed event, so an
append failure left widening state behind.

**Remediation:** decision meters and exemption headroom reconstruct from
verified signed tool-call, escalation, and exact-matching approval events plus
the broker's in-memory pending dispatch reservations. The two tables remain
compatibility caches and are never read for authorization. Approval event and
cache update now share one SQLite transaction, and approval without its exact
prior signed escalation grants nothing. Protected-effect negatives reproduce
both table writes and forced approval-append failure without dispatch. The
corrected G9 matrix also rejects signer-anomalous capability, caveat, manifest,
auth-strength, zero-use, duplicate-binding, and cross-capability accounting
edges.

## RF-30 — promotion/revert can partially commit or mutate without a signed event — fixed by W-14 (PR #43)

**Severity: high. Direction: CROSS-STORE INTEGRITY / UNRECORDED MUTATION.**
Both paths previously restored stores sequentially, then appended their event
and expected roots. A later-store failure left roots disagreeing; an event
failure left changed live state without its ledger cause.

**Remediation:** one internal state-change transaction prepares both forward
and rollback plans for every root, stages the primary signed event, expected
roots, promotion status, and companion approval in one uncommitted SQLite
transaction, then publishes a fabric-signed recovery journal. Stores are
applied and fsynced before the database commit. Ordinary failure restores and
fsyncs every before-root and rolls back the database; a retained journal makes
reopen deterministically roll back when the linked verified event is absent or
roll forward when it is present and its root tuple matches exactly. The journal
is an implementation-private recovery record, not a new public §6 event kind.
P16 becomes closed when W-14 merges; SI-25 still owns ledger deletion,
database rollback, and external freshness.
Recovery negatives additionally reject mistyped, wrong-version, symlink,
directory, and event/manifest-misbound journals before restore; a forced store
sync failure rolls every root and the event transaction back.

## RF-31 — parked promotion approval trusts unsigned candidate selectors — fixed by W-14 (PR #43)

**Severity: high. Direction: HUMAN-APPROVAL MISBINDING.** The mutable
`promotions` row supplied manifest and preview while its signed escalation
named only a numeric promotion id. Swapping two valid pending rows redirected
approval from the reviewed branch to another valid branch.

**Remediation:** the escalation signs promotion id, manifest, canonical digest
of the exact preview, digest of the branch-root tuple, and versioned policy
context. Approval recomputes and verifies that binding before auth-strength,
drift, or merge work; table-only and swapped rows are inert. The signed
approval repeats the candidate digest and commits atomically with promotion.

## RF-32 — tools/list advertises revoked, expired, stale, or ungranted capability surfaces — fixed by W-14 (PR #43)

**Severity: low. Direction: STALE AUTHORITY SURFACE / FAIL-CLOSED CONFUSION.**
Advertisement previously verified only the capability object. Dispatch still
denied, but a revoked or orphaned capability kept exposing tools to the agent.

**Remediation:** advertisement shares dispatch's current-authority prerequisite:
typed object, current-manifest M2 binding, mandatory live expiry, exact signed
grant and ancestry, and absence of leaf/ancestor revoke. Direct tools/list
tests cover signed revoke and capability-without-verified-grant.

Corrected W-14 assurance is green: strict Clippy; 24 contracts with 131
contracted tests and 93 frozen legacy tests; all 224 workspace tests and both
acceptance demos; 152 targeted mutants (140 caught, 12 compiler-unviable, zero
survivors/timeouts); and the deep 4,096-authority-case / 512-model-history
release lane. The independent-context re-review (fresh context, not the
authoring session) returned APPROVE WITH NON-BLOCKING FOLLOW-UPS on
2026-07-13; RF-19/RF-20/RF-29–RF-32 and P16 are closed by the merge in
PR #43. The re-review confirmed the implementation conforms to its
oracle filings SI-31 (owned-state transition/journal), SI-33 (event-derived
consumable authority), and SI-34 (approval candidate binding), and verified
against source — not the diff alone — that the Allow arm still reserves a
pending call with its checks (the RF-3 pipelined-propose fail-open stays
closed), the promotion event body carries `merged` so a real crashed
promotion rolls forward, and the recovery journal's commit point is exact
(linked event committed iff its SQLite transaction committed). Two
non-blocking follow-ups were filed (RF-33/RF-34) plus a P22 line-reference
correction and a per-decision-scan performance note carried to W-15.

## RF-33 — decision budget consumption is no longer durable across a crash restart — accepted (posture residual)

**Severity: medium. Direction: FAIL-OPEN across a process restart, bounded
by the LOCAL/DEBUG posture.** Surfaced by the W-14 independent re-review
(2026-07-13). Under W-14, decision-time budget and exemption headroom are
reconstructed from verified signed `tool_call`/`escalation`/`approval` events
plus the broker's in-memory pending dispatch reservations (the SI-33
doctrine); `broker_meters` is no longer written or read, and `exemptions` is
written but never read for authorization. This intentionally drops the
incidental durability the old decision-time `broker_meters` write provided.

**Residual:** a call that is Allowed (ticket issued, possibly dispatched
downstream) but whose result is never recorded — a crash between dispatch and
`record_result` — loses its in-memory reservation on restart. No signed
`tool_call` event exists for it, so its budget is not counted and the agent
may re-propose, over-granting by the lost reservation. This is the same
in-flight-reservation residual already tracked as **P22 / RF-3**, now
uniformly event-sourced rather than incidentally persisted.

**Why bounded under the current posture:** local effects land only on the
branch; the promotion gate recounts budget from signed events; and RF-9
strands any branch mutation whose root diverges from the last signed
`tool_call.state_root_after`, so over-granted extra writes cannot reach trunk
and revert covers them. **Must close before first live egress** with the
durable external-effect protocol (P22): a remote effect that already crossed
the boundary cannot be un-sent, so the reservation must become durable
(signed dispatch/reservation record) before the effect is dispatched.
Accepted for dogfooding; no code change.

## RF-34 — operator promotion listing fails closed on a single malformed parked row — open (low)

**Severity: low. Direction: FAIL-CLOSED (operator availability/legibility).**
Surfaced by the W-14 independent re-review (2026-07-13). `list_promotions`
now verifies every pending row against its signed candidate binding
(`w14_verify_promotion_candidate`, the RF-31 fix) and returns `Err` if any
one row fails. A storage writer who corrupts or injects a single
`promotions` row therefore breaks the listing of *all* pending promotions
rather than surfacing just the bad one. No unauthorized effect occurs and a
database writer already holds broader denial options, so this is fail-closed;
but it is an operator surface, and per the G9 operator-command discipline it
should report per-row findings (which row failed, and why) with a bounded
non-zero exit — the W-19 ledger-integrity pattern — rather than a single
all-or-nothing error. Non-blocking hardening; schedule with the next
operator-surface / RF-9 recovery pass.

## RF-35 — journal recovery lacks the ratified freshness predicate, pre-restore capture/attribution, and home/epoch binding — open (medium, posture-bounded)

**Severity: medium. Direction: UNRECORDED MUTATION / DATA LOSS on the
reopen-recovery path, measured against clauses ratified after the merge.**
Filed by the A24 ratification (W-20, 2026-07-14, ADR 0007 D31-2/D31-4/
D31-6): the merged W-14 recovery conforms to its oracle as reviewed, and
the ratification then *adjusted* the protocol in three places the
implementation does not yet meet.

1. **Freshness (D31-4):** `recover_pending_state_change` validates the
   journal against itself (linked event presence; kind/manifest/tuple
   agreement with the journal's own fields), never against the substrate's
   current-roots view **V**. A stale retained journal — a backup restore,
   a copied home directory, a replant — whose linked event is committed
   rolls live stores back to a historical tuple; one whose event never
   committed rolls them to its `before` images. Unrecorded,
   kernel-executed state mutation, surfacing only as later unattributed
   drift.
2. **Attribution (D31-6, the serious half):** recovery restores without
   capturing live roots to CAS or recording drift, and the fs apply's
   first pass deletes live files the target does not contain. Human edits
   made between crash and reopen — an unbounded window, in the store
   humans actually edit — are deleted or overwritten with **no CAS copy
   and no ledger trace** (M8's clause, missing at the recovery consumer).
3. **Binding (D31-2/H3/R9):** the journal carries no home/epoch; a
   pre-migration journal would be honored post-migration. Unimplementable
   before W-15 introduces epochs.

**Why bounded under the current posture:** the adversarial replant is moot
while RF-14 leaves the fabric key cleartext in the home (a same-uid
attacker forges rather than replants), so the live exposure is
*accidental* staleness plus the downtime-edit data-loss window, which
requires a crash landing exactly between journal publication and journal
removal followed by pre-reopen edits to affected roots. Dogfooding's
failure mode remains "revert and shrug" — except for item 2's lost
downtime edits, which is why this is medium, not low. **Carried by W-15**,
which already rewrites the same function for S4 prefix-bounding; the
adversarial value of item 1 activates with W-17 key custody. Founding
determinations and the V-derivation are ADR 0007's — **as adjusted through review rounds 1–2 (R3/R4, R8/R9/R11/R13)**: total
V now includes unbranched `tool_call.state_root_after` (round 2 showed
its omission bricks a genuine revert-crash recovery) and is projected
onto the journal's store set; freshness is positional (per-store
prior-attestation positions in the journal — value-only equality passes
same-epoch ABA replays); the **recovery capture record** (fabric-signed
write-ahead of the captured roots between capture and restore —
first-write-wins, journal validation family, removed before the journal;
without it a crash between restore and emission erases the downtime-edit
evidence); per-store window-drift + `fabric_recovery` closing pairs
carrying the journal id, pair-or-neither, with the pinned orderings
(closing-record idempotency check before the freshness predicate);
per-arm epoch guard. Round 3 (R14/R15): capture-record validation is
exact-set (a partial record cannot prove non-divergence), and retries
run the element-wise explanation check — live state explained by neither
the capture record nor the restore target fails closed in place, so a
divergence window opened during recovery's own downtime is preserved by
refusal (automated multi-window preservation is SI-39). Recovery never
moves V (normative invariant). The enforcing tests land with W-15's
contract lanes, including the G13(e)/(f)/(g) journal-validation
negatives round 3 split out.

## RF-36 — gate replay grants approval headroom without binding, order, or double-resolution checks — open (high, posture-bounded)

**Severity: high. Direction: FAIL-OPEN AUTHORITY at the gate, the
authoritative recount.** Found by round 1 of the PR #48 independent
review (finding 1); verified at source. `gate_trace_check` pre-aggregates
every signed `approved` event into `(capability, caveat) → uses` headroom
before replaying any tool call — checking none of: a matching prior
signed escalation; approval-after-escalation or approval-before-effect
order (a later approval retro-funds an earlier call at replay);
manifest/M2 agreement; auth strength; conflicting bindings; double
resolution. Decision time (`w14_decision_authority`) enforces all of
these, so the gate — which A25/§5.4 make **authoritative for what becomes
durable** — is strictly weaker than the advisory check, inverting the
authority hierarchy.

**Why bounded under the current posture:** locally the broker is the sole
producer of approval events and emits them correctly bound and ordered,
so the lax replay is unreachable from the agent. The exposure is
foreign/replayed substrate — exactly the W-9 corpus surface — and any
future path where trace rows are ingested rather than produced.
**Clause:** A25 §5.5 "Decision and gate" as adjusted by ADR 0007 R1 (the
recount applies the same exact-match binding predicate at each effect's
durable authorization offset — the W-2 shared-evaluator discipline
extended to consumption). Carried by **W-22**; its contract lane also
supplies the double-resolution negative G13 records as outstanding.

## RF-37 — approval-time re-merge applies un-previewed outcomes without comparison or re-park — open (medium)

**Severity: medium. Direction: HUMAN-APPROVAL MISBINDING (consent scope),
no authority widening beyond the digest-pinned branch content.** Found by
round 1 of the PR #48 independent review (finding 2), which refuted the
draft's narrowing rationale with a concrete counterexample: preview
against trunk `H` shows the branch edit conflicting (trunk-wins, nothing
applied); trunk returns to base before approval; the re-merge sees no
conflict and installs the full branch edit the human was shown *not*
landing. `approve_promotion` re-merges pinned branch roots against live
trunk and applies the result with no comparison to the previewed outcome.

**Clause:** A26 §6 re-merge boundary as adjusted by ADR 0007 R2,
re-quantified by R12 (round 2), and given its comparison basis by R16
(round 3) — the re-merge MUST reproduce the previewed outcome exactly:
per previewed op, over the op's full touched-path set, the re-merged
result equals the previewed merged result (the referent tree is part of
the signed candidate) with conflict status unchanged; never the
whole-store merged root, whose equality would re-park on every unrelated
human trunk edit; any difference re-parks as a fresh candidate with a
fresh escalation and digest. Bounded meanwhile: the
applied content is still the digest-pinned branch content, the gate
re-verifies trace-vs-capability, drift is attributed first (M8), and
Tier-1 promotion remains revertible. Carried by **W-22**; the
outcome-equality negative (G13) lands with its contract lane.

## RF-38 — a poisoned escalation chain denies all capabilities with no operator recovery path — open (low)

**Severity: low. Direction: FAIL-CLOSED availability (self-DoS), no
authorization bypass.** Found by round 1 of the PR #48 independent review
(finding 8). `w14_decision_authority` scans every escalation/approval
anomaly before filtering to the requested capability, so one duplicate
resolution or conflicting binding — for any capability — fails the
consumption view for **all** capabilities in the home, permanently
(the event stream is append-only). Ratified as intentional scope (ADR
0007 R7: a corrupted authority chain is a home-level integrity incident;
per-capability scoping would let a poisoned chain keep granting
elsewhere). What is missing is the operator recovery path — today nothing
short of an epoch action clears it. If the remedy needs a new record kind
(a ratified supersession/quarantine record), it graduates to an SI per
the triage rule before implementation. Schedule with the next
operator-surface pass (alongside RF-34).

## RF-39 — exemption approval does not bind the presented escalation version; the C2 listing renders unsigned rows — open (high, posture-bounded)

**Severity: high. Direction: HUMAN-APPROVAL MISBINDING on the exemption
surface — the RF-31 defect class, found alive on the second approval
surface.** Found by round 2 of the PR #48 independent review (finding 3);
verified at source. Three legs: (1) A9 batching appends a **new signed
escalation event per violation under one numeric id** with evolving
`count`/`sample` (`enqueue_escalation`), so "the signed escalation" is
version-ambiguous per id and nothing identifies which version the human
reviewed; (2) the approval event body names only the numeric id,
capability, and caveat — no escalation event id, digest, count, or
sample — so the approval binds no specific presented candidate; (3)
`list_escalations`, the C2 display surface, renders id, capability,
manifest, caveat, count, and samples directly from the mutable
`escalations` table with no verification against the signed chain — a
storage write can present benign samples for a hostile batch.

**Clause:** A26 §6 exemption candidate identity as adjusted by ADR 0007
R10 — the approval body carries `escalation_event` (the exact signed
version presented, verified at resolution); newer batch versions never
widen the grant and remain visible; C2 listings render from, or verify
against, signed escalation events (the RF-34/W-19 pattern). **Why
bounded:** approval surfaces are broker-owned (C2) and local-only; the
authority tuple (capability, caveat, manifest, auth-strength, uses) is
already exact-match verified from signed events, so the misbinding scope
is *which batch instance* the human believed they approved, not which
capability widens; and a table writer already needs home access (RF-14
posture). Carried by **W-22**; its lane supplies the version-swap and
tampered-listing negatives.

## Verified sound during review (recorded so they aren't re-litigated)

- Per-payload DEKs each perform exactly one encryption → no GCM nonce reuse
  on payload data.
- Credential injection stores pre-injection args (`args_raw`); the secret is
  absent from the request payload store. RF-23 separately tracks downstream
  reflection in the response path.
- The promotion gate recounts budgets from signed trace events, not the
  runtime meter — so RF-2/RF-3 meter drift cannot defeat enforcement.
- Trace append-only triggers and signature recomputation catch signed-raw and
  middle-chain tampering at explicit verification boundaries. RF-16/RF-13
  delimit the unsigned-index, completeness, ordering, and rollback residuals.
- Attenuation subset logic + fail-closed unknown dimensions are solid and
  property-tested (P5: no privilege escalation across generated pairs).
