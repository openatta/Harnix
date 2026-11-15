//! The Harnix app: the session space and what this product keeps about its sessions.
//!
//! Workspaces, archived sessions, view preferences, cross-session search, the project picker,
//! the composer's file menu and exports are not the kernel's: none of them is the hub, the
//! transport, authorization or the assembly; every one is a judgement about how this product
//! behaves. So they are an extension of their own, built from the extension interface alone, with
//! its state in its own directory — deleting it loses grouping and nothing else. A session's title
//! is not here: it is the engine's name for the session (`harnix.session.rename`).
//!
//! It declares the session space, the default: where a call that claims no space is checked, and
//! where a session nobody placed belongs. The projects view lists the same sessions by project and
//! needs no space of its own.

mod export;
mod files;
mod store;
mod workspaces;

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use harnix_ext::{
    caps, Access, Call, Extension, ExtensionContext, Manifest, Registrar, RowScope, Rpc, RpcError, SpaceDecl,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use store::Store;

/// The extension's id: its methods are `harnix-app.*`.
pub const ID: &str = "harnix-app";

/// The session space: the conversations, and the projects they run in.
pub const SESSIONS_SPACE: &str = "sessions";

/// `[ext.harnix-app]`: nothing to say yet, and a key that is not here is a mistake.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {}

/// The Harnix app's backend.
#[derive(Debug, Default)]
pub struct HarnixApp;

/// What its handlers answer from, made when it starts.
pub(crate) struct App {
    store: Mutex<Store>,
    /// Where projects live: what the picker opens on, and what "new project" creates into. A
    /// default, not a boundary — the fence is `$HOME` or this root.
    projects_root: PathBuf,
}

#[async_trait::async_trait]
impl Extension for HarnixApp {
    type Config = Config;

    fn manifest(&self) -> Manifest {
        Manifest::new(ID, env!("CARGO_PKG_VERSION"))
            .title("Harnix")
            .disclosure("会话 space：工作区、会话的归档、视图偏好、跨会话搜索、目录选择、文件引用、导出")
            .requires([caps::TRANSCRIPTS_READ, caps::ENGINE_EXPORT])
    }

    fn register(&self, r: &mut Registrar<'_, Config>) {
        r.space(SpaceDecl::new(SESSIONS_SPACE).default_space());
        let open = || Access::device();
        let session = || Access::device().scoped(RowScope::Session);
        r.method("workspaces.list").access(open()).raw(|c, _| with(c, |a, _| async move { a.workspaces_list().await }));
        r.method("workspaces.create").access(open()).raw(|c, p| with(c, |a, _| async move { a.workspaces_create(p).await }));
        r.method("workspaces.update").access(open()).raw(|c, p| with(c, |a, _| async move { a.workspaces_update(p).await }));
        r.method("workspaces.reorder").access(open()).raw(|c, p| with(c, |a, _| async move { a.workspaces_reorder(p).await }));
        r.method("workspaces.remove").access(open()).raw(|c, p| with(c, |a, _| async move { a.workspaces_remove(p).await }));
        r.method("sessions.archive").access(session()).raw(|c, p| with(c, |a, c| async move { a.session_archive(&c, p).await }));
        r.method("sessions.export").access(session()).raw(export::session_export);
        r.method("prefs.set").access(open()).raw(|c, p| with(c, |a, _| async move { a.prefs_set(p).await }));
        r.method("search").access(open()).raw(|c, p| with(c, |a, c| async move { a.search(&c, p).await }));
        r.method("files").access(open()).raw(|c, p| with(c, |a, _| async move { a.files(p).await }));
        r.method("listDirectory").access(open()).raw(|c, p| with(c, |a, _| async move { a.list_directory(p).await }));
        r.method("recentProjects").access(open()).raw(|c, _| with(c, |a, c| async move { a.recent_projects(&c).await }));
        r.method("projects.create").access(open()).raw(|c, p| with(c, |a, _| async move { a.projects_create(p).await }));
        r.method("describe")
            .access(open())
            .describe("where this product's projects live")
            .raw(|c: Call, _| async move { Ok(json!({"projects_dir": c.ext().projects_dir()})) });
    }

    async fn start(&self, cx: ExtensionContext) -> anyhow::Result<()> {
        let store = Store::open(cx.data_dir().to_path_buf())?;
        cx.provide(App { store: Mutex::new(store), projects_root: cx.projects_dir().to_path_buf() });
        Ok(())
    }
}

/// One method answered from the extension's state, with the call it answers.
async fn with<F, Fut>(call: Call, f: F) -> Rpc<Value>
where
    F: FnOnce(Arc<App>, Call) -> Fut,
    Fut: Future<Output = Rpc<Value>>,
{
    let app = call.state::<App>()?;
    f(app, call).await
}

pub(crate) fn require_str(params: &Value, key: &str) -> Result<String, RpcError> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| RpcError::invalid_params(format!("missing {key}")))
}

pub(crate) fn require_session_id(params: &Value) -> Result<String, RpcError> {
    require_str(params, "session_id")
}

pub(crate) fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// A workspace's default label: the last path segment, or the no-project name.
pub(crate) fn default_title(path: Option<&str>) -> String {
    match path {
        Some(path) => path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or(path)
            .to_string(),
        None => "无项目".to_string(),
    }
}
