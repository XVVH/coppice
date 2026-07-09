//! Promotion: diffs, operation classes, three-way merge. Spec §5.3
//! (A11, A13); brief §5.4's "only mutation in the system".
//!
//! Pure tree logic — trees in, merge decisions out. The broker owns the
//! gate orchestration (trace-vs-capability check, policy, approval,
//! apply); this module owns the semantics:
//!
//! - Diffs speak `add | modify | delete | move | rename` (A13). Rename
//!   detection pairs a deleted path with an added path of identical
//!   content hash, so a reorganization never renders as mass deletion.
//!   Exact-hash pairing only (similarity-based detection is a fidelity
//!   upgrade for later); same-directory pairs are `rename`, cross-
//!   directory pairs are `move` (SI-17 interpretation).
//! - Three-way merge (A11): base = the manifest's state root; branch and
//!   trunk reconcile against it. Non-conflicting changes from either side
//!   land. **Conflicts never auto-resolve in the agent's favor** — where
//!   both sides changed the same path differently, trunk (the human) wins
//!   by default and a conflict card records the branch version, which
//!   stays reachable in the CAS (snapshot ancestry survives the merge).

use serde_json::{json, Value};
use std::collections::BTreeMap;

/// path → content hash. Mode is deliberately out of merge identity for
/// Stage 3 (content decides conflicts; the chosen side's mode rides along).
pub type Tree = BTreeMap<String, String>;

/// Parse a stored fs_tree object into a Tree.
pub fn tree_from_object(tree_obj: &Value) -> Tree {
    tree_obj["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            Some((
                e["path"].as_str()?.to_string(),
                e["hash"].as_str()?.to_string(),
            ))
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Add { path: String },
    Modify { path: String },
    Delete { path: String },
    /// Same content, same parent directory, new name (SI-17).
    Rename { from: String, to: String },
    /// Same content, different parent directory (SI-17).
    Move { from: String, to: String },
}

impl Op {
    /// The A13 operation class name — the vocabulary promotion rules use.
    pub fn class(&self) -> &'static str {
        match self {
            Op::Add { .. } => "add",
            Op::Modify { .. } => "modify",
            Op::Delete { .. } => "delete",
            Op::Rename { .. } => "rename",
            Op::Move { .. } => "move",
        }
    }

    pub fn to_json(&self) -> Value {
        match self {
            Op::Add { path } => json!({"op": "add", "path": path}),
            Op::Modify { path } => json!({"op": "modify", "path": path}),
            Op::Delete { path } => json!({"op": "delete", "path": path}),
            Op::Rename { from, to } => json!({"op": "rename", "from": from, "to": to}),
            Op::Move { from, to } => json!({"op": "move", "from": from, "to": to}),
        }
    }
}

fn parent_dir(path: &str) -> &str {
    path.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
}

/// Diff `base → side` as operation classes with rename/move detection.
pub fn diff(base: &Tree, side: &Tree) -> Vec<Op> {
    let mut adds: Vec<(&String, &String)> = Vec::new();
    let mut deletes: Vec<(&String, &String)> = Vec::new();
    let mut ops = Vec::new();

    for (path, hash) in side {
        match base.get(path) {
            None => adds.push((path, hash)),
            Some(b) if b != hash => ops.push(Op::Modify { path: path.clone() }),
            Some(_) => {}
        }
    }
    for (path, hash) in base {
        if !side.contains_key(path) {
            deletes.push((path, hash));
        }
    }

    // Rename detection: pair deletes with adds of identical hash. Only
    // unambiguous pairs (hash appears exactly once on each side) — a
    // conservative rule that never mislabels, at the cost of leaving
    // duplicate-content shuffles as add+delete.
    let count = |list: &[(&String, &String)], h: &str| {
        list.iter().filter(|(_, hash)| hash.as_str() == h).count()
    };
    let mut paired_adds = Vec::new();
    let mut paired_dels = Vec::new();
    for (dpath, dhash) in &deletes {
        if count(&deletes, dhash) == 1 && count(&adds, dhash) == 1 {
            let (apath, _) = adds.iter().find(|(_, h)| h == dhash).expect("counted");
            let op = if parent_dir(apath) == parent_dir(dpath) {
                Op::Rename { from: (*dpath).clone(), to: (*apath).clone() }
            } else {
                Op::Move { from: (*dpath).clone(), to: (*apath).clone() }
            };
            ops.push(op);
            paired_adds.push((*apath).clone());
            paired_dels.push((*dpath).clone());
        }
    }
    for (path, _) in adds {
        if !paired_adds.contains(path) {
            ops.push(Op::Add { path: path.clone() });
        }
    }
    for (path, _) in deletes {
        if !paired_dels.contains(path) {
            ops.push(Op::Delete { path: path.clone() });
        }
    }
    ops.sort_by_key(|o| o.to_json().to_string());
    ops
}

#[derive(Debug, Clone)]
pub struct Conflict {
    pub path: String,
    pub base: Option<String>,
    pub branch: Option<String>,
    pub trunk: Option<String>,
    /// Always "trunk_wins" in Stage 3 — recorded, not configurable.
    pub resolution: &'static str,
}

impl Conflict {
    pub fn to_json(&self) -> Value {
        json!({
            "path": self.path, "base": self.base,
            "branch": self.branch, "trunk": self.trunk,
            "resolution": self.resolution,
        })
    }
}

#[derive(Debug)]
pub struct MergeResult {
    pub merged: Tree,
    /// What the agent did (branch relative to base), as operation classes.
    pub ops: Vec<Op>,
    /// Paths where both sides changed differently. Trunk won each one;
    /// the branch version's hash stays resolvable in the CAS.
    pub conflicts: Vec<Conflict>,
}

/// Three-way merge per A11. For each path: untouched-by-agent follows
/// trunk; untouched-by-human follows branch; identical changes coincide;
/// divergent changes conflict and trunk wins.
pub fn three_way(base: &Tree, branch: &Tree, trunk: &Tree) -> MergeResult {
    let mut paths: Vec<&String> = base.keys().chain(branch.keys()).chain(trunk.keys()).collect();
    paths.sort();
    paths.dedup();

    let mut merged = Tree::new();
    let mut conflicts = Vec::new();
    for path in paths {
        let (b, br, tr) = (base.get(path), branch.get(path), trunk.get(path));
        let winner = if br == b {
            tr // agent untouched → trunk state stands (incl. deletion)
        } else if tr == b {
            br // human untouched → agent's change lands
        } else if br == tr {
            tr // both made the identical change
        } else {
            conflicts.push(Conflict {
                path: path.clone(),
                base: b.cloned(),
                branch: br.cloned(),
                trunk: tr.cloned(),
                resolution: "trunk_wins",
            });
            tr // conflicts never auto-resolve in the agent's favor
        };
        if let Some(hash) = winner {
            merged.insert(path.clone(), hash.clone());
        }
    }
    MergeResult { merged, ops: diff(base, branch), conflicts }
}

/// The zero-authorship default promotion policy (§0, A13): auto-promote
/// iff there are no conflicts and every branch operation is `add` or
/// `modify` — the classes whose worst case is revertible clutter. Any
/// delete/move/rename, or any conflict, parks for human approval. Caveat-
/// shaped on purpose: when StandingRules arrive (Stage 3 trust loop),
/// ratified promotion rules replace this constant in the same vocabulary.
pub fn default_policy_allows(ops: &[Op], conflicts: &[Conflict]) -> bool {
    conflicts.is_empty() && ops.iter().all(|o| matches!(o.class(), "add" | "modify"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(entries: &[(&str, &str)]) -> Tree {
        entries.iter().map(|(p, h)| (p.to_string(), h.to_string())).collect()
    }

    #[test]
    fn rename_never_renders_as_mass_deletion() {
        // A13's motivating case: reorganize 3 files into a subdirectory.
        let base = tree(&[("a.md", "h1"), ("b.md", "h2"), ("c.md", "h3")]);
        let side = tree(&[("notes/a.md", "h1"), ("notes/b.md", "h2"), ("notes/c.md", "h3")]);
        let ops = diff(&base, &side);
        assert_eq!(ops.len(), 3);
        assert!(ops.iter().all(|o| o.class() == "move"), "{ops:?}");
    }

    #[test]
    fn rename_vs_move_distinction() {
        let base = tree(&[("dir/a.md", "h1"), ("dir/b.md", "h2")]);
        let side = tree(&[("dir/a2.md", "h1"), ("other/b.md", "h2")]);
        let ops = diff(&base, &side);
        assert!(ops.contains(&Op::Rename { from: "dir/a.md".into(), to: "dir/a2.md".into() }));
        assert!(ops.contains(&Op::Move { from: "dir/b.md".into(), to: "other/b.md".into() }));
    }

    #[test]
    fn ambiguous_duplicate_content_stays_add_delete() {
        // Two identical files deleted, one identical file added: pairing
        // would be a guess, so we don't.
        let base = tree(&[("x.md", "same"), ("y.md", "same")]);
        let side = tree(&[("z.md", "same")]);
        let ops = diff(&base, &side);
        let classes: Vec<&str> = ops.iter().map(Op::class).collect();
        assert!(classes.contains(&"add") && classes.contains(&"delete"));
        assert!(!classes.contains(&"rename") && !classes.contains(&"move"));
    }

    #[test]
    fn non_conflicting_changes_from_both_sides_land() {
        let base = tree(&[("keep.md", "h0"), ("agent.md", "h0"), ("human.md", "h0")]);
        let branch = tree(&[("keep.md", "h0"), ("agent.md", "AGENT"), ("human.md", "h0"), ("new.md", "N")]);
        let trunk = tree(&[("keep.md", "h0"), ("agent.md", "h0"), ("human.md", "HUMAN")]);
        let r = three_way(&base, &branch, &trunk);
        assert!(r.conflicts.is_empty());
        assert_eq!(r.merged["agent.md"], "AGENT");
        assert_eq!(r.merged["human.md"], "HUMAN");
        assert_eq!(r.merged["new.md"], "N");
        assert_eq!(r.merged["keep.md"], "h0");
    }

    #[test]
    fn conflicts_trunk_wins_and_branch_version_recorded() {
        let base = tree(&[("both.md", "h0"), ("del-vs-mod.md", "h0"), ("mod-vs-del.md", "h0")]);
        let branch = tree(&[("both.md", "AGENT"), ("mod-vs-del.md", "AGENT")]); // deleted del-vs-mod
        let trunk = tree(&[("both.md", "HUMAN"), ("del-vs-mod.md", "HUMAN")]);  // deleted mod-vs-del
        let r = three_way(&base, &branch, &trunk);
        assert_eq!(r.conflicts.len(), 3);
        // Divergent edit: trunk content wins.
        assert_eq!(r.merged["both.md"], "HUMAN");
        // Agent deleted, human modified: file survives with human content.
        assert_eq!(r.merged["del-vs-mod.md"], "HUMAN");
        // Agent modified, human deleted: stays deleted.
        assert!(!r.merged.contains_key("mod-vs-del.md"));
        // Every conflict card carries the losing branch version.
        for c in &r.conflicts {
            assert_eq!(c.resolution, "trunk_wins");
        }
    }

    #[test]
    fn identical_changes_do_not_conflict() {
        let base = tree(&[("f.md", "h0")]);
        let branch = tree(&[("f.md", "SAME")]);
        let trunk = tree(&[("f.md", "SAME")]);
        let r = three_way(&base, &branch, &trunk);
        assert!(r.conflicts.is_empty());
        assert_eq!(r.merged["f.md"], "SAME");
    }

    #[test]
    fn default_policy_gates_destructive_classes() {
        let base = tree(&[("a.md", "h1")]);
        let adds = three_way(&base, &tree(&[("a.md", "h1"), ("b.md", "h2")]), &base);
        assert!(default_policy_allows(&adds.ops, &adds.conflicts));

        let deletes = three_way(&base, &tree(&[]), &base);
        assert!(!default_policy_allows(&deletes.ops, &deletes.conflicts));

        let conflicted = three_way(&base, &tree(&[("a.md", "X")]), &tree(&[("a.md", "Y")]));
        assert!(!default_policy_allows(&conflicted.ops, &conflicted.conflicts));
    }
}
