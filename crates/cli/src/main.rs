//! `harnix` — the Harnix app: Harnix's own extensions, flows, schedules, and packages in this process.
//!
//! An application's own binary is Harnix's extensions with its own added (`templates/app`); an
//! application that wants flows registers `harnix_ext_flow::Flow` itself, as this one does, and
//! one that runs packages adds `harnix::first_party::packages()`.
//!
//! The flow module here has no programs, checks or tools of an application — a flow that names
//! one does not validate (a dry run still goes through it: there everything is mocked). Command
//! gates are on: a gate of the pack may run a command on this machine, in the run's workspace.
//! Runs are kept in the module's data directory (`runs/`); flows and their parts come from the
//! example built into this binary, under a writable layer in the module's data directory
//! (`packs/`) where the flow space's conversations save what they write. The flow module offers Agix's
//! built-in scenes wrapped — `flow-harnix`, `flow-code`, `flow-mini` — and a run's main session is
//! on `flow-harnix` unless its flow names another, in a directory of the run's own. Run under
//! `--replay-dir`, the flow sessions replay recordings named after their run or flow.
//!
//! Schedules (`harnix::first_party::Schedules`) are kept in their extension's data directory and run
//! while this process does: turns in sessions, new sessions, tasks of a flow in a project.
//!
//! With no profile named, a session starts in the `harnix` scene and `code` is served beside it
//! ([`default_engine`]), so the new-session dialog offers both.

#[cfg(feature = "flow")]
use harnix_ext_flow::{DirPacks, Flow, MemPacks};

// The example pack is the flow library's (`crates/flow/packs/examples`): it goes wherever the
// library is copied, and the library's golden runs play it.
#[cfg(feature = "flow")]
/// The example flow built into the binary: a session reviews the subject, a person approves.
pub const EXAMPLE_FLOW: &str = include_str!("../../flow/packs/examples/flows/example-review.yaml");
#[cfg(feature = "flow")]
/// The skill its session works from.
pub const EXAMPLE_SKILL: &str = include_str!("../../flow/packs/examples/skills/review/SKILL.md");
#[cfg(feature = "flow")]
/// The rule it keeps.
pub const EXAMPLE_RULE: &str = include_str!("../../flow/packs/examples/rules/review-honest.md");
#[cfg(feature = "flow")]
/// The policy it works under.
pub const EXAMPLE_POLICY: &str = include_str!("../../flow/packs/examples/policies/read-only.yaml");

#[cfg(feature = "flow")]
/// The example's flow and parts.
fn example() -> MemPacks {
    MemPacks::new()
        .flow(EXAMPLE_FLOW)
        .skill("review", EXAMPLE_SKILL)
        .rule("review-honest", EXAMPLE_RULE)
        .policy(EXAMPLE_POLICY)
}

#[cfg(feature = "flow")]
/// The flow module as the Harnix app registers it.
pub fn flow() -> Flow {
    Flow::builder()
        .packs_with(|dir| {
            DirPacks::writable(dir.join("packs")).with_builtin(example())
        })
        .command_gates(true)
        // Under `--replay-dir` only (outside it the names mean nothing): a run's main session
        // replays `flow-run-<run>`, a flow's conversation `flow-conversation-<flow>`, the general
        // one `flow-conversation` — what `scripts/harnix-demo.sh` makes its conversations with.
        .main_recording(|rec| Some(format!("flow-run-{}", rec.id)))
        .conversation_recording(|flow| {
            Some(if flow.is_empty() {
                "flow-conversation".to_string()
            } else {
                format!("flow-conversation-{flow}")
            })
        })
        .build()
}

/// Harnix's own extensions, as this build has them.
pub fn extensions() -> harnix::ExtensionSet {
    let set = harnix::first_party::standard();
    #[cfg(feature = "flow")]
    let set = set.with(flow(), harnix::first_party::FLOW_GRANTS);
    #[cfg(feature = "schedule")]
    let set = set.with(harnix::first_party::Schedules::new(), harnix::first_party::SCHEDULE_GRANTS);
    #[cfg(any(feature = "wasm", feature = "proc"))]
    let set = set.with(harnix::first_party::packages(), harnix::first_party::PACKAGES_GRANTS);
    set
}

/// The `[engine]` it runs with when no profile is named: sessions start in `harnix`, and `code`
/// (a coding agent in a project) is there to choose beside it.
pub fn default_engine() -> harnix::engine::EngineProfile {
    harnix::engine::EngineProfile::new().scenes(["code"])
}

fn main() -> anyhow::Result<()> {
    harnix::App::builder().extensions(extensions()).default_engine(default_engine()).run()
}

#[cfg(all(test, feature = "flow"))]
mod tests {
    use super::*;
    use harnix_ext_flow::{validate_yaml, Packs};

    /// Valid with nothing of an application's: the catalog has only the wrapped scenes.
    #[test]
    fn the_example_flow_is_valid_in_the_empty_application() {
        let packs = example().main_scene("flow-harnix");
        let problems = validate_yaml(EXAMPLE_FLOW, &packs as &dyn Packs);
        assert!(problems.is_empty(), "{problems:?}");
    }

    /// The demo's flows (`scripts/demo`, brought in by `scripts/harnix-demo.sh`) are valid here,
    /// with the demo's skills.
    #[test]
    fn the_demo_flows_are_valid_in_the_empty_application() {
        let demo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/demo");
        let catalog = MemPacks::new().main_scene("flow-harnix").command_gates();
        let packs = DirPacks::new([demo.clone()]).with_builtin(catalog);
        for kind in harnix_ext_flow::PartKind::ALL {
            for name in packs.parts(kind) {
                let (text, _) = packs.part(kind, &name).unwrap();
                let problems = harnix_ext_flow::part_problems(kind, &name, &text, &packs as &dyn Packs);
                assert!(problems.is_empty(), "{kind} {name}: {problems:?}");
            }
        }
        let mut n = 0;
        for e in std::fs::read_dir(demo.join("flows")).unwrap() {
            let yaml = std::fs::read_to_string(e.unwrap().path()).unwrap();
            let problems = validate_yaml(&yaml, &packs as &dyn Packs);
            assert!(problems.is_empty(), "{yaml}\n{problems:?}");
            n += 1;
        }
        // However many the demo has: every file there is a flow these packs read, and there is one.
        assert!(n > 0, "the demo has no flows");
        assert_eq!(n, packs.flows().len(), "{:?}", packs.flows());
    }

    #[test]
    fn saved_flows_go_over_the_built_in_one() {
        let tmp = std::env::temp_dir().join(format!("harnix-cli-{}", std::process::id()));
        let packs = DirPacks::writable(tmp.join("packs")).with_builtin(
            MemPacks::new()
                .flow(EXAMPLE_FLOW)
                .skill("review", EXAMPLE_SKILL),
        );
        assert!(packs.writable());
        assert_eq!(packs.flows(), ["example-review"]);
        assert_eq!(packs.origin("example-review"), harnix_ext_flow::Origin::Builtin);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
