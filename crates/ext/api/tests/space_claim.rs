//! A native extension using an instance a template built for a space claims that space, as a
//! device does: its sessions there are opened, read and driven with [`Sessions::in_space`], and
//! without the claim — or claiming another space — the hub answers `SPACE_MISMATCH`. Being native
//! does not exempt it.
//!
//! [`Sessions::in_space`]: harnix_ext::Sessions::in_space

use harnix_ext::engine::{EngineProfile, InstanceTemplate};
use harnix_ext::{codes, Entry, Extension, ExtensionContext, Manifest, OnBusy, Registrar, Rpc, SessionSpec, SpaceDecl};
use serde_json::json;

const ID: &str = "board";

struct Board;

#[async_trait::async_trait]
impl Extension for Board {
    type Config = ();

    fn manifest(&self) -> Manifest {
        Manifest::new(ID, "0.1.0").disclosure("a space whose sessions run on an instance of their own")
    }

    fn register(&self, r: &mut Registrar<'_, ()>) {
        r.template(InstanceTemplate::in_process("side", EngineProfile::new()));
        r.space(SpaceDecl::new(ID).instance("template:board.side"));
        r.space(SpaceDecl::new("elsewhere"));
    }

    async fn start(&self, _cx: ExtensionContext) -> anyhow::Result<()> {
        Ok(())
    }
}

fn refused<T: std::fmt::Debug>(r: Rpc<T>) -> i32 {
    r.unwrap_err().code
}

#[tokio::test]
async fn an_extension_claims_the_space_whose_instance_it_uses() {
    let dir = tempfile::tempdir().unwrap();
    // An engine keeps its project data in the working directory: this test's own.
    std::env::set_current_dir(dir.path()).unwrap();
    std::fs::write(dir.path().join("profile.toml"), "").unwrap();
    let app = harnix::App::builder().extension(Board).ungranted();
    let kernel = harnix::testing::Kernel::start(app, dir.path()).await.expect("the process starts");
    let cx = kernel.context(ID).expect("started, and seated");

    // A session of its space: on the instance the template built for it.
    let mine = cx.sessions().in_space(ID);
    let session = mine.create(SessionSpec::new()).await.expect("a session in its space");
    assert_ne!(session.instance, "main", "{session:?}");

    // Claiming its space, it reaches the session.
    mine.open(&session.instance, &session.session_id).await.unwrap();
    mine.append_entry(&session, Entry::new("note", json!({"n": 1}))).await.unwrap();
    assert_eq!(mine.entries(&session).await.unwrap().len(), 1);
    mine.send(&session, "hi", OnBusy::Queue).await.unwrap();

    // Claiming none, or another space: refused before the engine is asked.
    let unclaimed = cx.sessions();
    assert_eq!(refused(unclaimed.open(&session.instance, &session.session_id).await), codes::SPACE_MISMATCH);
    assert_eq!(refused(unclaimed.entries(&session).await), codes::SPACE_MISMATCH);
    let other = cx.sessions().in_space("elsewhere");
    assert_eq!(refused(other.open(&session.instance, &session.session_id).await), codes::SPACE_MISMATCH);
    assert_eq!(refused(other.entries(&session).await), codes::SPACE_MISMATCH);
    assert_eq!(refused(other.send(&session, "hi", OnBusy::Queue).await), codes::SPACE_MISMATCH);
    assert_eq!(refused(other.create(SessionSpec::new().instance(&session.instance)).await), codes::SPACE_MISMATCH);

    mine.close(&session).await.unwrap();
    kernel.shutdown().await;
}
