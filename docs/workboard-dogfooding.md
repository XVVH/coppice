# Coppice Workboard — structured-local-state dogfood profile

The workboard is Coppice's second real dogfooding workflow. It tracks the
dogfooding program itself while exercising a different state shape from vault
maintenance:

- `workboard.db` is an opaque SQLite root containing work items, optimistic
  revisions, dependencies, evidence links, and activity history.
- `evidence/` is a filesystem root containing Markdown observations,
  reproductions, decisions, and session notes.
- One proxy session snapshots, branches, traces, promotes, and reverts both
  roots together.

This is intentionally a trusted compile-time profile, not a generic plugin or
downstream-supplied registration mechanism. Its tool metadata, default grant,
store topology, and tool ref (`tool:workboard@1.0`) live in the proxy.

## What the agent can do

| Tool | Behavior |
| --- | --- |
| `work.list` / `work.get` | Read compact ordered summaries, then one item's details, dependencies, evidence links, and activity. |
| `work.create` | Create a task at priority 0–4. |
| `work.update` | Change title, details, status, or priority if `expected_revision` is current. |
| `work.add_dependency` | Add a dependency if the revision is current and the edge would remain acyclic. |
| `work.link_evidence` | Link an existing safe relative evidence path and advance the task revision. |
| `work.close` | Mark a task done if the revision is current and every dependency is done. |
| `evidence.list` / `evidence.read` | Read Markdown evidence inside the fixed root. |
| `evidence.create` | Create without overwriting an existing file. |
| `evidence.edit` | Replace exactly one matching string. |

There is no delete action in v1. Evidence mutation and SQLite linking are two
calls because the current trusted action registration binds each action to one
mutated store. The session still captures their combined result as one
two-root promotion/revert unit.

SQLite writes have no invented `paths.write` value: a row id is not a
filesystem path and agent-supplied target metadata is not a trustworthy guard.
The grant instead has a fixed action allowlist, a 20-write run budget, a two-hour
expiry, and `local_session` approval. Evidence paths are confined by the tool
server to the fixed root; absolute paths, dot components, parent traversal, and
symlink traversal are rejected.

## One-time setup

Use a separate fabric home from the vault profile. A home is pinned to its
exact profile and root paths; changing either is rejected because it would
change the coordinated store topology or recover a branch into the wrong roots.
Keep the agent workspace separate from both state roots, just as with the vault
profile.

```sh
cargo build --release

ASF=/ABS/PATH/TO/Coppice/target/release/asf
STATE=/ABS/PATH/TO/coppice-workboard-state
mkdir -p "$STATE/evidence"

# Seed the first pieces of real work before the fabric takes its first snapshot.
"$ASF" workboard create --db "$STATE/workboard.db" --evidence "$STATE/evidence" \
  --title "Run the structured-state daily-triage scenario" --priority 3
"$ASF" workboard create --db "$STATE/workboard.db" --evidence "$STATE/evidence" \
  --title "Exercise stale-revision handling" --priority 2
"$ASF" workboard create --db "$STATE/workboard.db" --evidence "$STATE/evidence" \
  --title "Exercise concurrent human SQLite edit and parked promotion" --priority 2
"$ASF" workboard create --db "$STATE/workboard.db" --evidence "$STATE/evidence" \
  --title "Exercise coherent workboard revert" --priority 3
```

Point the MCP client in the dedicated dogfood workspace at:

```json
{
  "mcpServers": {
    "coppice-workboard": {
      "command": "/ABS/PATH/TO/Coppice/target/release/asf",
      "args": [
        "proxy",
        "--home", "/ABS/PATH/TO/coppice-workboard-state/asf-home",
        "--profile", "workboard",
        "--db", "/ABS/PATH/TO/coppice-workboard-state/workboard.db",
        "--evidence", "/ABS/PATH/TO/coppice-workboard-state/evidence",
        "--downstream", "/ABS/PATH/TO/Coppice/target/release/asf",
        "workboard-server",
        "--db", "/ABS/PATH/TO/coppice-workboard-state/workboard.db",
        "--evidence", "/ABS/PATH/TO/coppice-workboard-state/evidence"
      ]
    }
  }
}
```

The same operator surfaces work with this home:

```sh
"$ASF" approve --home "$STATE/asf-home" promotions
"$ASF" ledger --home "$STATE/asf-home"
"$ASF" stats --home "$STATE/asf-home"
"$ASF" recover --home "$STATE/asf-home" --profile workboard \
  --db "$STATE/workboard.db" --evidence "$STATE/evidence"
```

The human CLI writes trunk directly. That is useful for seeding and deliberate
between-session edits; after tracking begins, the next boundary should record
those changes as human-attributed drift.

## Use it as the dogfooding record

Create one work item for each run, finding, or follow-up. Put long-form notes in
an evidence file, then link it using the current task revision. A normal agent
sequence is:

1. `work.list` for triage.
2. `work.create` or `work.update` to establish the structured item.
3. `evidence.create` for the session log or reproduction.
4. `work.link_evidence` with the task's latest revision.
5. `work.add_dependency` for follow-up ordering.
6. `work.close` only after the linked work and dependencies are complete.

Keep the rubric id in the title (`DF-W1`, `DF-P4`, and so on) and put ledger
event ids, approval latency, false-positive judgment, and reproduction steps in
the evidence note. This replaces a single ever-growing vault log as the primary
record; existing vault-log observations remain valid historical evidence.

## Structured-state scenarios

Run these in addition to the vault rubric. They are not synthetic tests: use
the board for current Coppice work while staging only the edge named by each
scenario.

| ID | Scenario | Expected result |
| --- | --- | --- |
| DF-W1 | Daily triage: list, reprioritize, start, and close ordinary work. | Reads and valid revisioned writes succeed; clean session promotes. |
| DF-W2 | Create a finding, a Markdown reproduction, and link them. | Both roots remain branch-only until one coherent promotion. |
| DF-W3 | Build a three-item dependency chain and attempt a cycle. | Valid edges land; the cycle is rejected; unfinished prerequisites prevent close. |
| DF-W4 | Reuse an old `expected_revision`. | Tool returns a legible stale-revision error and preserves the newer row. |
| DF-W5 | Edit trunk SQLite with the human CLI while the agent changes its branch DB. | Opaque-store conflict parks; trunk wins pending operator decision (SI-18). |
| DF-W6 | Hand-edit an unrelated evidence file mid-session. | M8 attributes the human edit and preserves it through the merge. |
| DF-W7 | Promote changes to both roots, then revert to the base manifest. | Task rows/revisions and evidence contents restore together. |
| DF-W8 | Kill the proxy after calls but before normal client shutdown. | Existing stranded-session recovery gates both roots once. |

Track two profile-specific tripwires: SQLite conflict incidence and stale
revision frequency. Frequent SQLite conflicts are evidence that whole-store
merge is too coarse for this workload; they are not permission to invent a
row-level merge before its authority and conflict semantics are ratified.

## Deliberate limits

- SQLite is a whole-store merge unit (SI-18). Concurrent trunk and branch
  changes park and trunk wins; there is no row merge.
- The workboard uses SQLite's rollback journal, not a persistent WAL, because
  the current snapshot root is the database image itself.
- The cooperative same-Unix-user boundary from `dogfooding.md` still applies.
  Client deny rules must cover the workboard DB, evidence root, fabric home,
  operator CLI, and shell.
- This adds no egress, credentials, standing rules, multi-actor visibility, or
  generic tool registration. Those remain separate milestones.
- `work.tracking` and `work.evidence` are provisional first-party domain labels,
  not a ratified fabric taxonomy; SI-22 must settle governance before they feed
  portable trust or third-party registrations.
