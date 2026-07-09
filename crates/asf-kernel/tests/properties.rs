//! Property tests (testing-theory G1). Deterministic: a seeded xorshift
//! generator (no external dep, fully reproducible) plus one exhaustive
//! enumeration. Shrinking-quality proptest adoption remains open — these
//! establish the properties; a counterexample here prints its seed/case.
//!
//! Properties:
//! - P1 merge identities: branch==base → merged==trunk; trunk==base →
//!   merged==branch.
//! - P2 conflict soundness: a conflict card appears exactly where both
//!   sides changed a path differently, and trunk's version won it (A11).
//! - P3 non-conflicting agent changes always land.
//! - P4 glob soundness (exhaustive): glob_covers(p, c) implies every path
//!   matched by c is matched by p (SI-12's rule must never over-approve).
//! - P5 attenuation semantic subset: if verify_attenuation accepts a
//!   child, then ANY call the child allows, the parent allows — a
//!   counterexample is a privilege escalation through §5.2.

use asf_kernel::capability::{glob_covers, glob_matches, verify_attenuation};
use asf_kernel::evaluate::{evaluate, CallCtx, Outcome};
use asf_kernel::promote::{three_way, Tree};
use serde_json::{json, Value};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[(self.next() % xs.len() as u64) as usize]
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

const PATHS: &[&str] = &[
    "inbox/a.md", "inbox/b.md", "inbox/sub/c.md", "MOCs/m.md", "root.md", "notes/x.md",
];
const HASHES: &[&str] = &["h1", "h2", "h3", "h4"];

fn random_tree(rng: &mut Rng) -> Tree {
    let mut t = Tree::new();
    for p in PATHS {
        if rng.chance(60) {
            t.insert(p.to_string(), rng.pick(HASHES).to_string());
        }
    }
    t
}

/// Derive a plausible sibling of `base`: some paths kept, some edited,
/// some deleted, some added.
fn mutate(rng: &mut Rng, base: &Tree) -> Tree {
    let mut t = Tree::new();
    for (p, h) in base {
        if rng.chance(75) {
            let h = if rng.chance(30) { rng.pick(HASHES).to_string() } else { h.clone() };
            t.insert(p.clone(), h);
        } // else deleted
    }
    for p in PATHS {
        if !base.contains_key(*p) && rng.chance(25) {
            t.insert(p.to_string(), rng.pick(HASHES).to_string());
        }
    }
    t
}

#[test]
fn p1_p2_p3_merge_properties() {
    let mut rng = Rng(0x5EED_0001);
    for case in 0..2000 {
        let base = random_tree(&mut rng);
        let branch = mutate(&mut rng, &base);
        let trunk = mutate(&mut rng, &base);

        // P1a: agent did nothing → trunk state is the merge.
        let r = three_way(&base, &base, &trunk);
        assert_eq!(r.merged, trunk, "P1a case {case}");
        assert!(r.conflicts.is_empty(), "P1a case {case}");

        // P1b: human did nothing → agent's branch is the merge.
        let r = three_way(&base, &branch, &base);
        assert_eq!(r.merged, branch, "P1b case {case}");
        assert!(r.conflicts.is_empty(), "P1b case {case}");

        // General merge: P2 + P3, path by path.
        let r = three_way(&base, &branch, &trunk);
        let mut all: Vec<&String> = base.keys().chain(branch.keys()).chain(trunk.keys()).collect();
        all.sort();
        all.dedup();
        for p in all {
            let (b, br, tr) = (base.get(p), branch.get(p), trunk.get(p));
            let both_changed_differently = br != b && tr != b && br != tr;
            let has_conflict = r.conflicts.iter().any(|c| &c.path == p);
            assert_eq!(
                both_changed_differently, has_conflict,
                "P2 conflict iff divergent double-change: case {case} path {p} (b={b:?} br={br:?} tr={tr:?})"
            );
            if both_changed_differently {
                // Trunk wins, including trunk-deleted (absent from merged).
                assert_eq!(r.merged.get(p), tr, "P2 trunk-wins: case {case} path {p}");
            }
            if br != b && tr == b {
                assert_eq!(r.merged.get(p), br, "P3 agent change lands: case {case} path {p}");
            }
        }
    }
}

#[test]
fn p4_glob_cover_soundness_exhaustive() {
    // Every glob and path over a small alphabet, bounded depth. If
    // glob_covers claims coverage, matching must agree on EVERY path.
    let segs = ["a", "b", "ab", "*", "**", "a*"];
    let path_segs = ["a", "b", "ab", "ba"];
    let mut globs: Vec<String> = Vec::new();
    let mut paths: Vec<String> = Vec::new();
    for d1 in segs {
        globs.push(d1.to_string());
        for d2 in segs {
            globs.push(format!("{d1}/{d2}"));
            for d3 in segs {
                globs.push(format!("{d1}/{d2}/{d3}"));
            }
        }
    }
    for d1 in path_segs {
        paths.push(d1.to_string());
        for d2 in path_segs {
            paths.push(format!("{d1}/{d2}"));
            for d3 in path_segs {
                paths.push(format!("{d1}/{d2}/{d3}"));
            }
        }
    }
    let mut covered_pairs = 0;
    for parent in &globs {
        for child in &globs {
            if !glob_covers(parent, child) {
                continue;
            }
            covered_pairs += 1;
            for path in &paths {
                if glob_matches(child, path) {
                    assert!(
                        glob_matches(parent, path),
                        "SOUNDNESS HOLE: glob_covers({parent}, {child}) but {path} matches child only"
                    );
                }
            }
        }
    }
    assert!(covered_pairs > 100, "the conservative rule should accept some real coverage");
}

// ---- P5: attenuation semantic subset ---------------------------------

const ACTIONS: &[&str] = &["note.read", "note.write", "note.move", "note.delete"];
const GLOBSETS: &[&[&str]] = &[
    &["**"],
    &["inbox/**"],
    &["inbox/**", "MOCs/**"],
    &["inbox/drafts/**"],
];
const REVS: &[&str] = &["reversible", "compensable", "irreversible"];

fn random_cap(rng: &mut Rng, id: &str, parent: Option<&str>) -> Value {
    let n_actions = 1 + (rng.next() % ACTIONS.len() as u64) as usize;
    let mut actions: Vec<&str> = Vec::new();
    while actions.len() < n_actions {
        let a = rng.pick(ACTIONS);
        if !actions.contains(a) {
            actions.push(a);
        }
    }
    json!({
        "id": id, "parent": parent, "holder": "prin:agent",
        "bound_manifest": "man:x",
        "issued_at": "2026-07-08T00:00:00Z",
        "expires_at": "2027-01-01T00:00:00Z",
        "caveats": [
            {"dim": "action.allow", "tools": ["tool:vault@1.0"], "actions": actions},
            {"dim": "reversibility.max", "max": rng.pick(REVS)},
            {"dim": "paths.write", "globs": rng.pick(GLOBSETS)},
            {"dim": "budget.count", "action_class": "write",
             "max": rng.next() % 5, "window": "run"},
        ],
        "on_violation": { "default": "deny", "escalatable": [] },
    })
}

#[test]
fn p5_attenuation_implies_semantic_subset() {
    let mut rng = Rng(0x5EED_0002);
    let write_path_pool: &[&[&str]] = &[
        &["inbox/a.md"],
        &["inbox/drafts/d.md"],
        &["MOCs/m.md"],
        &["inbox/a.md", "MOCs/m.md"],
        &["elsewhere/x.md"],
    ];
    let mut verified_pairs = 0;
    let mut allows_checked = 0;
    for _ in 0..4000 {
        let parent = random_cap(&mut rng, "cap:parent", None);
        let child = random_cap(&mut rng, "cap:child", Some("cap:parent"));
        if verify_attenuation(&parent, &child).is_err() {
            continue; // invalid attenuations are P5-irrelevant (and separately tested)
        }
        verified_pairs += 1;
        for _ in 0..40 {
            let action = *rng.pick(ACTIONS);
            let class = if action == "note.delete" { "delete" } else if action == "note.move" { "move" } else if action == "note.read" { "read" } else { "write" };
            let rev = *rng.pick(REVS);
            let paths: Option<Vec<String>> = if class == "read" {
                None
            } else {
                Some(rng.pick(write_path_pool).iter().map(|s| s.to_string()).collect())
            };
            let ctx = CallCtx {
                tool: "tool:vault@1.0",
                action,
                reversibility: rev,
                side_effect: "local",
                action_class: class,
                write_paths: paths,
                now: "2026-07-08T06:00:00Z",
                current_manifest: "man:x",
            };
            let used = rng.next() % 5;
            let child_eval = evaluate(&child, &ctx, &mut |_| used, &mut |_| None);
            if child_eval.outcome == Outcome::Allow {
                allows_checked += 1;
                let parent_eval = evaluate(&parent, &ctx, &mut |_| used, &mut |_| None);
                assert_eq!(
                    parent_eval.outcome,
                    Outcome::Allow,
                    "PRIVILEGE ESCALATION: child allows {action} (rev={rev}, used={used}, \
                     paths={:?}) but parent does not.\nparent={parent}\nchild={child}",
                    ctx.write_paths
                );
            }
        }
    }
    assert!(verified_pairs > 50, "generator should produce valid attenuations ({verified_pairs})");
    assert!(allows_checked > 500, "and child-allowed calls to check ({allows_checked})");
}
