C7 and C8 are refuted as written. The central failure is hardlink aliasing: family 3 works only if the retained generation is immutable, and the proposed recipe does not produce an immutable generation.

## Strongest findings

1. **High — hardlinks erase the publication boundary.** The [staging recipe](/Users/josh/dev/Coppice/docs/substrate-theory.md:151) gives the outgoing and incoming trees names for the same inode.

   - A pre-swap in-place edit to an “unchanged” file mutates both trees, so it cannot be localized to the retained generation.
   - A post-swap edit through the new live path mutates the retained evidence.
   - Comparing retained against live finds equality; comparing retained against the prepared root can detect something changed only by scanning a path that was supposedly unchanged, and cannot tell which side of the swap the edit occurred on.
   - Editors using temp-file-plus-rename break the alias on one side, while in-place writers preserve it. Correctness therefore depends on application save style.

   This is not the acknowledged descriptor tax: it affects an ordinary pathname opened after publication. The invariant must be “no writable inode is shared across generations.” That requires reflinked independent inodes, a real CoW snapshot, or full copies—not hardlinks.

2. **High — `O(changed)` requires precisely the write-boundary mechanism C8 claims to avoid.** On raw POSIX:

   - Constructing the hardlink tree already requires enumerating and linking approximately `N` entries.
   - Discovering edits at unknown paths in the retained tree requires either an `O(N)` walk or a complete change journal/watcher history. The latter is family-2 detection under another name.
   - A one-operation directory rename containing `M` descendants requires `O(M)` namespace work because directories cannot be hardlinked.
   - File↔directory transitions require examining the affected path-state union. Calling that `O(changed)` is true only if “changed” counts every affected descendant, not logical operations.

   Thus the bound can hold for a Merkle-indexed/native-snapshot substrate with a reliable mutation log. It does not hold for the claimed portable POSIX floor.

3. **High — root exchange is hostile to watchers.** Linux inotify watches objects, not future occupants of a pathname. After exchanging the watched root, recursive watches remain associated with the outgoing directory objects; the newly staged directories are unwatched until the consumer rebuilds its watch set. Linux also documents rename pairing as inherently racy and recursive watch construction as expensive for large trees. [inotify documentation](https://man7.org/linux/man-pages/man7/inotify.7.html)

   On macOS, Apple explicitly requires a full-tree rescan when a watched root is moved or renamed. An exchange-rename therefore makes the external observation cost `O(N)` per publication for a conforming WatchRoot consumer. [Apple FSEvents guide](https://developer.apple.com/library/archive/documentation/Darwin/Conceptual/FSEvents_ProgGuide/UsingtheFSEventsFramework/UsingtheFSEventsFramework.html)

   Syncthing’s documented rescan checks every existing entry’s metadata and rehashes entries whose metadata changed. It also retains periodic full scans because watcher delivery is not complete. [Syncthing synchronization documentation](https://docs.syncthing.net/users/syncing.html)

   I cannot substantiate a universal Syncthing/Dropbox delete-and-recreate conflict storm without black-box tests. The defensible finding is the dichotomy: rebuild/rescan at `O(N)`, or risk missing live changes.

4. **High — “NFS-safe” conflates server atomicity with client coherence.** NFS RENAME is atomic at the server, but client LOOKUP and directory caches may retain the old symlink binding. Linux defaults permit directory attributes to remain cached for up to 60 seconds; NFS itself explicitly provides weaker cache coherence than a cluster filesystem. [Linux NFS documentation](https://man7.org/linux/man-pages/man5/nfs.5.html)

   NFSv4.1 directory delegations can provide synchronous recall, but they are optional and not always granted. Without one, different clients can resolve old and new targets concurrently, and a stale client can continue pathname-based writes into the retained generation after the nominal swap. [RFC 8881](https://www.rfc-editor.org/rfc/rfc8881.html)

   “NFS-safe” is defensible only as “server-atomic, no missing-name interval.” It is not a single globally visible transition. The profile needs cache/delegation requirements, a stale-resolution bound, and a retention/reconciliation grace period.

## Claim verdicts

| Item | Verdict | Reason |
|---|---|---|
| C6 | **Refute as worded** | WALs are an orthogonal crash-recovery mechanism; leases, delegations, and oplocks are revocable exclusion/coherence. Neither is a clean fourth family. More importantly, mature systems compose families: Git combines expected-old validation, locking, and atomic rename; MVCC combines version retention with locks or validation; LSM systems combine WAL, serialization, and manifest indirection. The examples do not establish an exhaustive or disjoint taxonomy. |
| C7 | **Refute** | Retention is sound only when the retained generation cannot change. Hardlinks, stale NFS resolution, and open handles violate that prerequisite and blur the claimed temporal cut. |
| C8 | **Refute as written** | The property-first framing is good, but the hardlink recipe, “NFS-safe” binding, “nearly free” cost, and unconditional `O(changed)` bound fail. |
| C9 | **Refine** | This is not one ladder. CoW improves retention cost; namespace primitives determine publication atomicity; leases/watchers determine observer coherence; mediation determines attribution. A CoW filesystem does not automatically retarget mounted paths, open handles, or watchers, nor make a multi-store swap atomic. This should be a profile matrix, not a monotonic substrate ladder. |
| F3 | **Valid but non-operational** | It needs explicit variables and thresholds. Under root-watcher rescans, the crossover arrives at surprisingly small sparse-change ratios. |

## F3 crossover

Let:

- `N` = total entries
- `K` = entries targeted per publication
- `p` = publications per unit time
- `s` = full-scan throughput in entries/second
- `d` = family-2 validation cost per targeted entry
- `q` = fraction of publications causing a full watcher/sync rescan

For native CoW, giving family 3 every advantage:

```
family 2: T₂ ≈ p K d
family 3: T₃ ≥ p q N / s
```

Family 3 loses when:

```
N / K > s d / q
```

Illustrative—not measured—values of `s = 50,000 entries/s` and `d = 100 µs` give:

- If every root swap causes a scan (`q = 1`), family 3 loses when `N > 5K`, meaning less than 20% of the tree changes.
- If only 1% cause a scan, it loses when `N > 500K`.

For `N = 1,000,000`, `K = 100`, and 12 publications/hour:

```
family 2: 10 ms/publication ≈ 0.12 s/hour
family 3: 20 s/publication ≈ 240 s/hour
```

That lower bound excludes staging, reconciliation, hashing, retained-byte growth, network metadata, and conflict copies. On the portable hardlink-tree implementation, add `O(N)` link/tree-construction work regardless of watcher behavior.

F3 should therefore measure full scans, metadata operations, bytes rehashed, conflict copies, network traffic, retained unique bytes, and stale-client duration across inotify, FSEvents, NFS mount profiles, Syncthing, and Dropbox. Until that matrix exists, “nearly free” is unsupported rather than merely vulnerable to future falsification.

::code-comment{title="[P1] Hardlinks invalidate retained-state evidence" body="Unchanged entries in the two generations name the same inode. In-place writes before or after the swap mutate both generations, so the outgoing tree is neither immutable evidence nor a stable temporal boundary. This also makes reconciliation dependent on writer save style. Family 3 requires independent writable inodes—reflinks, snapshots, or copies—not hardlinks." file="/Users/josh/dev/Coppice/docs/substrate-theory.md" start=151 end=154 priority=1}

::code-comment{title="[P1] O(changed) assumes a complete mutation oracle" body="A raw retained POSIX tree cannot be proven unchanged at unknown paths without an O(tree) walk or a complete mutation log. Watchers can overflow and root exchange invalidates their object/path mapping; directory renames and topology changes can also touch an entire subtree despite being one logical operation. State the bound only for profiles supplying an immutable indexed generation plus reliable change enumeration." file="/Users/josh/dev/Coppice/docs/substrate-theory.md" start=155 end=157 priority=1}

::code-comment{title="[P1] NFS atomicity is not client coherence" body="Rename-over-symlink is atomic at the NFS server, but cached LOOKUP and directory state can leave clients resolving the old target for tens of seconds under ordinary mount settings. Directory delegations can improve this but are optional. Label this server-atomic/eventually-visible and specify the cache/delegation and retention-grace profile instead of calling it NFS-safe." file="/Users/josh/dev/Coppice/docs/substrate-theory.md" start=147 end=150 priority=1}
