//! `harnix` — the empty application: the built-ins, and the flow module (xops-02 §6).
//!
//! An application's own binary is the built-ins with its modules added (`templates/app`); an
//! application that wants flows registers `harnix_flow::Flow` itself, as this one does.
//!
//! The flow module here has no programs, gates or tools of an application — a flow that names
//! one does not validate (a dry run still goes through it: there everything is mocked). Runs are
//! kept in the module's data directory (`runs/`); flows and skills come from the example built
//! into this binary, under a writable layer in the module's data directory (`packs/`) where the
//! flow space's conversations save what they write. Session stations run in `flow-chat` or
//! `flow-coding` (Agix's chat and coding scenes, wrapped).

use std::sync::Arc;

use harnix_flow::{DirPacks, Flow, MemPacks};

/// The example flow built into the binary: a session reviews the subject, a person approves.
pub const EXAMPLE_FLOW: &str = include_str!("../packs/flows/example-review.yaml");
/// The skill its session works from.
pub const EXAMPLE_SKILL: &str = include_str!("../packs/skills/review/SKILL.md");

/// The flow module as the empty application registers it.
pub fn flow() -> Flow {
    Flow::builder()
        .packs_with(|dir| {
            DirPacks::writable(dir.join("packs")).with_builtin(
                MemPacks::new()
                    .flow(EXAMPLE_FLOW)
                    .skill("review", EXAMPLE_SKILL),
            )
        })
        .scene(Arc::new(scene::scene::chat::ChatScene))
        .scene(Arc::new(scene::scene::coding::CodingScene))
        .build()
}

fn main() -> anyhow::Result<()> {
    harnix::App::builder().with_builtins().module(flow()).run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use harnix_flow::{validate_yaml, Packs};

    /// Valid with nothing of an application's: the catalog has only the wrapped scenes.
    #[test]
    fn the_example_flow_is_valid_in_the_empty_application() {
        let packs = MemPacks::new()
            .skill("review", EXAMPLE_SKILL)
            .scene("flow-chat")
            .scene("flow-coding");
        let problems = validate_yaml(EXAMPLE_FLOW, &packs as &dyn Packs);
        assert!(problems.is_empty(), "{problems:?}");
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
        assert_eq!(packs.origin("example-review"), harnix_flow::Origin::Builtin);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
