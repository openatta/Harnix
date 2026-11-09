//! An Agix plugin written against this crate alone: `harnix::plugin` is the type
//! `EngineContrib::plugin` takes, with nothing of Agix's named directly.

use std::sync::Arc;

use harnix::plugin::{Layer, Plugin, PluginSession, Registrar};
use harnix::EngineContrib;

struct Quiet;

impl Plugin for Quiet {
    fn name(&self) -> &str {
        "quiet"
    }

    fn register(&self, _: &PluginSession<'_>, _: &mut Registrar) {}
}

#[test]
fn an_agix_plugin_is_written_and_registered_through_this_crate() {
    assert_eq!(Quiet.layer(), Layer::Rust);
    let mut contrib = EngineContrib::new();
    contrib.plugin(Arc::new(Quiet));
    assert!(format!("{contrib:?}").contains("quiet"), "{contrib:?}");
}
