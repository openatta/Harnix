//! A named service of the host is handed out by what the host checks, not by what the asking
//! extension says: the type it asks through is its own to write, so a capability that type claims
//! decides nothing. `harnix.host` takes `extensions.manage`, whichever type asks for it.

use harnix_assembly::testing::Kernel;
use harnix_assembly::App;
use harnix_contract::codes;
use harnix_contract::s1::caps;
use harnix_ext::{Extension, Manifest, Registrar, Service};
use harnix_host_api::{HostApi, HostServices};
use serde_json::json;

/// An extension that may be granted `extensions.manage` or `process.observe`, and declares nothing
/// else.
struct Plain(&'static str);

#[async_trait::async_trait]
impl Extension for Plain {
    type Config = ();

    fn manifest(&self) -> Manifest {
        Manifest::new(self.0, "0.0.1").optional([caps::EXTENSIONS_MANAGE, caps::PROCESS_OBSERVE])
    }

    fn register(&self, _r: &mut Registrar<'_, ()>) {}
}

/// The host's pipes under their own name, declared by an extension that wants them: whatever
/// capability it would claim for them (one it holds, say), the host asks for its own.
struct Forged;

impl Service for Forged {
    const NAME: &'static str = HostServices::NAME;
    type Handle = dyn HostApi;
}

#[tokio::test]
async fn the_host_s_pipes_go_only_to_an_extension_granted_what_they_take() {
    let dir = tempfile::tempdir().unwrap();
    // An engine keeps its project data in the working directory: this test's own.
    std::env::set_current_dir(dir.path()).unwrap();
    let app = App::builder()
        .extension(Plain("forger"))
        .grant([caps::PROCESS_OBSERVE])
        .extension(Plain("manager"))
        .grant([caps::EXTENSIONS_MANAGE]);
    let kernel = Kernel::start(app, dir.path()).await.expect("the process starts");

    let forger = kernel.context("forger").expect("started");
    let refused = json!({"reason": "capability", "capability": caps::EXTENSIONS_MANAGE.as_str()});
    for e in [forger.service::<Forged>().err(), forger.service::<HostServices>().err()] {
        let e = e.expect("refused");
        assert_eq!(e.code, codes::REFUSED, "{e:?}");
        assert_eq!(e.data, Some(refused.clone()), "{e:?}");
    }

    let manager = kernel.context("manager").expect("started");
    assert!(manager.service::<HostServices>().is_ok());
    assert!(manager.service::<Forged>().is_ok(), "the same service, by whatever type names it");

    kernel.shutdown().await;
}
