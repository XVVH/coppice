//! Generated state-machine coverage over the real broker, filesystem branch,
//! trace, promotion gate, approval path, and M8 drift attribution.
//!
//! The oracle is deliberately independent and path-local: agent-only changes
//! land, human-only changes remain, identical double-changes coalesce, and a
//! divergent double-change is a conflict whose trunk value wins. `proptest`
//! shrinks a failing operation history to the smallest reproducible sequence.

use asf_kernel::broker::{Broker, Decision, PromotionOutcome};
use asf_kernel::kernel::Fabric;
use asf_kernel::snapshot::{StoreKind, StoreSpec};
use asf_kernel::trace;
use proptest::prelude::*;
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const PATHS: &[&str] = &["a.md", "b.md", "nested/c.md"];
const CONTENTS: &[&str] = &["base", "red", "green", "blue"];

#[derive(Clone, Debug)]
enum Step {
    AgentWrite { path: usize, content: usize },
    HumanWrite { path: usize, content: usize },
}

fn histories() -> impl Strategy<Value = Vec<Step>> {
    prop::collection::vec(
        (any::<bool>(), 0usize..PATHS.len(), 0usize..CONTENTS.len()).prop_map(
            |(agent, path, content)| {
                if agent {
                    Step::AgentWrite { path, content }
                } else {
                    Step::HumanWrite { path, content }
                }
            },
        ),
        0..18,
    )
}

fn model_cases() -> u32 {
    std::env::var("ASF_MODEL_CASES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(64)
}

struct World {
    _tmp: tempfile::TempDir,
    vault: PathBuf,
    broker: Broker,
    manifest: String,
    branch: BTreeMap<String, PathBuf>,
    cap: String,
    channel: String,
}

fn setup() -> World {
    let tmp = tempfile::tempdir().unwrap();
    let vault = tmp.path().join("vault");
    fs::create_dir_all(vault.join("nested")).unwrap();
    for path in PATHS {
        fs::write(vault.join(path), CONTENTS[0]).unwrap();
    }

    let stores = vec![StoreSpec {
        store: "fs:vault".into(),
        tier: 1,
        kind: StoreKind::Fs,
        path: vault.clone(),
    }];
    let mut fabric = Fabric::open(tmp.path().join("fabric"), stores).unwrap();
    let human = fabric
        .register_principal("human", "model-human", "01", None)
        .unwrap();
    let agent = fabric
        .register_principal("agent", "model-agent", "02", Some(&human))
        .unwrap();
    let channel = fabric
        .register_channel(&human, "local_session", b"model", "local_session")
        .unwrap();
    let intent = fabric
        .capture_intent(
            &human,
            &channel,
            "local_session",
            "generated vault maintenance",
            json!({}),
            None,
        )
        .unwrap();
    let step = fabric
        .step_boundary(
            &human,
            &agent,
            &intent,
            json!({"bundle":"sha256:model","skills":[]}),
        )
        .unwrap();
    let branch = fabric.create_branch(&step.manifest).unwrap();
    let mut broker = Broker::new(fabric).unwrap();
    broker
        .register_tool(
            "tool:model-vault@1",
            json!([{
                "name": "note.write",
                "side_effect": "local",
                "surface": "fixed",
                "reversibility": "reversible",
                "domain": "files.vault",
                "class": "write",
                "store": "fs:vault",
                "path_args": ["path"],
            }]),
        )
        .unwrap();
    let cap = broker
        .mint(
            &step.manifest,
            &agent,
            vec![
                json!({"dim":"action.allow","tools":["tool:model-vault@1"],
                       "actions":["note.write"]}),
                json!({"dim":"reversibility.max","max":"reversible"}),
                json!({"dim":"paths.write","globs":["**"]}),
                json!({"dim":"budget.count","action_class":"write",
                       "max":1000,"window":"run"}),
                json!({"dim":"approval.min_auth","min":"local_session"}),
            ],
            vec![],
            "2027-01-01T00:00:00Z",
        )
        .unwrap();

    World {
        _tmp: tmp,
        vault,
        broker,
        manifest: step.manifest,
        branch,
        cap,
        channel,
    }
}

fn write_agent(world: &mut World, path: &str, content: &str) {
    let decision = world
        .broker
        .propose_call(
            &world.cap,
            "tool:model-vault@1",
            "note.write",
            &json!({"path": path, "content": content}),
        )
        .unwrap();
    let Decision::Allowed { ticket, .. } = decision else {
        panic!("generated in-scope write was not allowed: {decision:?}");
    };
    let target = world.branch["fs:vault"].join(path);
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(target, content).unwrap();
    world.broker.record_result(ticket, b"{}").unwrap();
}

fn read_state(root: &Path) -> BTreeMap<String, String> {
    PATHS
        .iter()
        .map(|path| {
            (
                (*path).to_string(),
                fs::read_to_string(root.join(path)).unwrap(),
            )
        })
        .collect()
}

fn expected_merge(
    base: &BTreeMap<String, String>,
    agent: &BTreeMap<String, String>,
    human: &BTreeMap<String, String>,
) -> (BTreeMap<String, String>, bool) {
    let mut merged = BTreeMap::new();
    let mut conflict = false;
    for path in PATHS {
        let key = (*path).to_string();
        let (base_v, agent_v, human_v) = (&base[&key], &agent[&key], &human[&key]);
        let value = if agent_v == base_v {
            human_v
        } else if human_v == base_v || human_v == agent_v {
            agent_v
        } else {
            conflict = true;
            human_v
        };
        merged.insert(key, value.clone());
    }
    (merged, conflict)
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: model_cases(),
        max_shrink_iters: 2_000,
        ..ProptestConfig::default()
    })]

    #[test]
    fn generated_histories_preserve_promotion_and_attribution_invariants(history in histories()) {
        let mut world = setup();
        let base = read_state(&world.vault);
        let mut agent = base.clone();
        let mut human = base.clone();

        for step in &history {
            match *step {
                Step::AgentWrite { path, content } => {
                    write_agent(&mut world, PATHS[path], CONTENTS[content]);
                    agent.insert(PATHS[path].to_string(), CONTENTS[content].to_string());
                }
                Step::HumanWrite { path, content } => {
                    fs::write(world.vault.join(PATHS[path]), CONTENTS[content]).unwrap();
                    human.insert(PATHS[path].to_string(), CONTENTS[content].to_string());
                }
            }
        }

        let (merged, has_conflict) = expected_merge(&base, &agent, &human);
        let branch = world.branch.clone();
        let outcome = world
            .broker
            .promote_manifest(&world.manifest, &branch)
            .unwrap();

        match (has_conflict, outcome) {
            (false, PromotionOutcome::Applied { .. }) => {}
            (true, PromotionOutcome::Parked { promotion }) => {
                // Parking never mutates trunk. Approval applies the same pinned
                // agent candidate, with the independently modeled trunk wins.
                prop_assert_eq!(
                    read_state(&world.vault),
                    human.clone(),
                    "history={:?}",
                    history,
                );
                world
                    .broker
                    .approve_promotion(promotion, &world.channel, "local_session")
                    .unwrap();
            }
            (expected, observed) => prop_assert!(
                false,
                "policy outcome disagreed with model: expected_conflict={expected}, observed={observed:?}, history={history:?}"
            ),
        }

        prop_assert_eq!(
            read_state(&world.vault),
            merged,
            "history={:?}",
            history,
        );
        prop_assert!(
            world.broker.fabric.check_drift().unwrap().is_empty(),
            "promotion left unexplained live divergence: history={history:?}"
        );
        let explanation = world.broker.fabric.explain().unwrap();
        prop_assert!(
            explanation.unexplained.is_empty(),
            "ledger could not explain generated history {history:?}: {:?}",
            explanation.unexplained,
        );

        let human_diverged = human != base;
        let drift_events = trace::all_events(&world.broker.fabric.conn)
            .unwrap()
            .into_iter()
            .filter(|event| event.kind == "drift")
            .count();
        prop_assert_eq!(
            drift_events,
            usize::from(human_diverged),
            "one M8 event per net divergence window: history={:?}",
            history,
        );
    }
}
