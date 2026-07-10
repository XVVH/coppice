//! Property tests (testing-theory G1). Deterministic: a seeded xorshift
//! generator (fully reproducible), one exhaustive enumeration, and shrinkable
//! `proptest` cases over the complete Stage-2 caveat vocabulary.
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

use asf_kernel::capability::{glob_covers, glob_matches, verify_attenuation, KNOWN_DIMS};
use asf_kernel::evaluate::{evaluate, CallCtx, Outcome};
use asf_kernel::promote::{three_way, Tree};
use proptest::prelude::*;
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

/// P6 (RF-1 guard) — instant comparison agrees with true chronology, and
/// lexical string comparison provably does NOT. A regression to string
/// comparison of timestamps would flip `string_disagreements` checks and be
/// caught; the instant path must never disagree.
#[test]
fn p6_instant_compare_agrees_with_chronology_strings_do_not() {
    use time::format_description::well_known::Rfc3339;
    use time::{Duration, OffsetDateTime};

    let base = OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap();
    let fmt = |t: OffsetDateTime| t.format(&Rfc3339).unwrap();

    // Crafted SAME-second, mixed-precision pairs — the exact RF-1 hazard.
    // Lexical order diverges from chronology only within one second, so this
    // must be constructed, not hoped for from random spread.
    let crafted = [
        (base, base + Duration::milliseconds(500)),
        (base + Duration::milliseconds(1), base + Duration::milliseconds(999)),
        (base, base + Duration::nanoseconds(1)),
    ];
    let mut string_disagreements = 0;
    for (ti, tj) in crafted {
        let (si, sj) = (fmt(ti), fmt(tj));
        // Instant path (what the kernel now uses) must match chronology.
        assert_eq!(
            asf_kernel::parse_instant(&si).unwrap() <= asf_kernel::parse_instant(&sj).unwrap(),
            ti <= tj,
            "instant compare wrong: {si} vs {sj}"
        );
        // Lexical path (the RF-1 bug) disagrees on these.
        if (si <= sj) != (ti <= tj) {
            string_disagreements += 1;
        }
    }
    assert!(
        string_disagreements > 0,
        "crafted same-second pairs should expose the lexical hazard — if 0, the test lost its teeth"
    );

    // Random pairs (whole/sub-second mix): the instant path must ALWAYS
    // agree with chronology, spread across the whole range.
    let mut rng = Rng(0x5EED_0006);
    let mk = |rng: &mut Rng| -> OffsetDateTime {
        let secs = (rng.next() % 200_000_000) as i64;
        let nanos = if rng.chance(50) { 0 } else { (rng.next() % 1_000_000_000) as i64 };
        base + Duration::seconds(secs) + Duration::nanoseconds(nanos)
    };
    for _ in 0..4000 {
        let (ti, tj) = (mk(&mut rng), mk(&mut rng));
        let (si, sj) = (fmt(ti), fmt(tj));
        assert_eq!(
            asf_kernel::parse_instant(&si).unwrap() <= asf_kernel::parse_instant(&sj).unwrap(),
            ti <= tj,
            "instant compare disagreed with chronology: {si} vs {sj}"
        );
    }
}

// ---- P7: shrinkable full-vocabulary attenuation semantics ------------

fn actions_from_mask(mask: u8) -> Vec<&'static str> {
    ACTIONS
        .iter()
        .enumerate()
        .filter_map(|(i, action)| (mask & (1 << i) != 0).then_some(*action))
        .collect()
}

fn ranked<'a>(rank: u8, values: &'a [&'a str]) -> &'a str {
    values[usize::from(rank)]
}

fn shrinkable_cases() -> u32 {
    std::env::var("ASF_PROPTEST_CASES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(512)
}

#[allow(clippy::too_many_arguments)]
fn full_cap(
    id: &str,
    parent: Option<&str>,
    actions: u8,
    rev: u8,
    reach: u8,
    budget: u8,
    scope: u8,
    not_before_hour: u8,
    not_after_hour: u8,
    auth: u8,
) -> Value {
    let revs = ["reversible", "compensable", "irreversible"];
    let reaches = ["none", "mocks", "live"];
    let scopes = ["**", "inbox/**", "inbox/drafts/**"];
    let strengths = ["unverified", "platform_oauth", "passkey"];
    json!({
        "id": id,
        "parent": parent,
        "holder": "prin:agent",
        "bound_manifest": "man:x",
        "issued_at": "2026-07-08T00:00:00Z",
        "expires_at": "2027-01-01T00:00:00Z",
        "caveats": [
            {"dim": "action.allow", "tools": ["tool:vault@1.0"],
             "actions": actions_from_mask(actions)},
            {"dim": "reversibility.max", "max": ranked(rev, &revs)},
            {"dim": "external_reach", "mode": ranked(reach, &reaches)},
            {"dim": "budget.count", "action_class": "write",
             "max": budget, "window": "run"},
            {"dim": "paths.write", "globs": [ranked(scope, &scopes)]},
            {"dim": "time",
             "not_before": format!("2026-07-08T{not_before_hour:02}:00:00Z"),
             "not_after": format!("2026-07-08T{not_after_hour:02}:00:00Z")},
            {"dim": "approval.min_auth", "min": ranked(auth, &strengths)},
        ],
        "on_violation": {"default": "deny", "escalatable": []},
    })
}

#[test]
fn p7_generator_tracks_the_evaluator_vocabulary() {
    let cap = full_cap("cap:coverage", None, 1, 0, 0, 0, 0, 0, 13, 0);
    let mut generated: Vec<&str> = cap["caveats"]
        .as_array()
        .expect("generated capability has caveats")
        .iter()
        .map(|caveat| caveat["dim"].as_str().expect("generated caveat has a dimension"))
        .collect();
    generated.sort_unstable();
    generated.dedup();

    let mut evaluated = KNOWN_DIMS.to_vec();
    evaluated.sort_unstable();
    evaluated.dedup();

    assert_eq!(
        generated, evaluated,
        "P7 must generate every mechanically evaluated caveat dimension; update full_cap when KNOWN_DIMS changes"
    );
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: shrinkable_cases(),
        max_shrink_iters: 10_000,
        ..ProptestConfig::default()
    })]

    /// The shrinkable form of P5 covers every dimension known to the
    /// evaluator. A failure prints a minimal authority-widening witness.
    #[test]
    fn p7_full_vocabulary_attenuation_is_semantic_subset(
        parent_action_mask in 1u8..16,
        child_action_seed in 1u8..16,
        rev_a in 0u8..3,
        rev_b in 0u8..3,
        reach_a in 0u8..3,
        reach_b in 0u8..3,
        budget_a in 0u8..8,
        budget_b in 0u8..8,
        scope_a in 0u8..3,
        scope_b in 0u8..3,
        auth_a in 0u8..3,
        auth_b in 0u8..3,
        child_start in 0u8..7,
        child_end in 7u8..13,
        call_action in 0usize..ACTIONS.len(),
        call_rev in 0u8..3,
        call_external in any::<bool>(),
        call_path in 0u8..4,
        used in 0u64..9,
        now_hour in 0u8..14,
    ) {
        let mut child_action_mask = parent_action_mask & child_action_seed;
        if child_action_mask == 0 {
            child_action_mask = parent_action_mask & parent_action_mask.wrapping_neg();
        }
        let (parent_rev, child_rev) = (rev_a.max(rev_b), rev_a.min(rev_b));
        let (parent_reach, child_reach) = (reach_a.max(reach_b), reach_a.min(reach_b));
        let (parent_budget, child_budget) = (budget_a.max(budget_b), budget_a.min(budget_b));
        // Higher scope index is narrower: ** → inbox/** → inbox/drafts/**.
        let (parent_scope, child_scope) = (scope_a.min(scope_b), scope_a.max(scope_b));
        // Higher auth rank is a stricter approval requirement.
        let (parent_auth, child_auth) = (auth_a.min(auth_b), auth_a.max(auth_b));

        let parent = full_cap(
            "cap:parent", None, parent_action_mask, parent_rev, parent_reach,
            parent_budget, parent_scope, 0, 13, parent_auth,
        );
        let child = full_cap(
            "cap:child", Some("cap:parent"), child_action_mask, child_rev,
            child_reach, child_budget, child_scope, child_start, child_end,
            child_auth,
        );
        prop_assert!(
            verify_attenuation(&parent, &child).is_ok(),
            "constructed attenuation rejected:\nparent={parent}\nchild={child}"
        );

        let action = ACTIONS[call_action];
        let class = match action {
            "note.delete" => "delete",
            "note.move" => "move",
            "note.read" => "read",
            _ => "write",
        };
        let paths = ["inbox/drafts/d.md", "inbox/a.md", "MOCs/m.md", "elsewhere/x.md"];
        let write_paths = (class != "read").then(|| vec![paths[usize::from(call_path)].to_string()]);
        let ctx = CallCtx {
            tool: "tool:vault@1.0",
            action,
            reversibility: ranked(call_rev, &["reversible", "compensable", "irreversible"]),
            side_effect: if call_external { "external" } else { "local" },
            action_class: class,
            write_paths,
            now: &format!("2026-07-08T{now_hour:02}:00:00Z"),
            current_manifest: "man:x",
        };
        let child_eval = evaluate(&child, &ctx, &mut |_| used, &mut |_| None);
        if child_eval.outcome == Outcome::Allow {
            let parent_eval = evaluate(&parent, &ctx, &mut |_| used, &mut |_| None);
            prop_assert_eq!(
                parent_eval.outcome,
                Outcome::Allow,
                "PRIVILEGE ESCALATION:\naction={} class={} external={} path={:?} used={} now={}\nparent={}\nchild={}",
                action,
                class,
                call_external,
                ctx.write_paths,
                used,
                ctx.now,
                parent,
                child,
            );
        }
    }
}
