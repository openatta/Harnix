//! A tool provider written against this crate alone: `harnix::engine` is what a module gives the
//! engine, with nothing of Agix's named.

use std::sync::Arc;

use harnix::engine::{SessionFacts, Tool, ToolProvider};
use harnix::EngineContrib;

struct Quiet;

impl ToolProvider for Quiet {
    fn name(&self) -> &str {
        "quiet"
    }

    fn tools(&self, _: &SessionFacts) -> Vec<Arc<dyn Tool>> {
        Vec::new()
    }
}

#[test]
fn a_tool_provider_is_written_and_given_through_this_crate() {
    let mut contrib = EngineContrib::new();
    contrib.provider(Arc::new(Quiet));
    assert!(format!("{contrib:?}").contains("quiet"), "{contrib:?}");
}
