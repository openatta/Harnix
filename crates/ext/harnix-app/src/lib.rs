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
//! needs no space of its own. A project's trust is the engine's; the projects space shows and
//! changes it through this extension (`trust`).
//!
//! It declares the home space too: the page a person lands on, with the figures and lists other
//! extensions put there (`harnix-app.home.panels`). It holds no conversation; it is declared so that
//! what the page asks claiming it is answered for a space that exists, and lists nothing — a session
//! is never created there (`SpaceDecl::no_sessions`).

mod export;
mod files;
mod import;
mod project;
mod store;
mod trust;
mod workspaces;

use std::collections::BTreeMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use harnix_ext::{
    catalog, caps, Access, Call, Extension, ExtensionContext, Manifest, Registrar, RowScope, Rpc, RpcError, SpaceDecl,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use store::Store;

/// The extension's id: its methods are `harnix-app.*`.
pub const ID: &str = "harnix-app";

/// The session space: the conversations, and the projects they run in.
pub const SESSIONS_SPACE: &str = "sessions";

/// The home space: where the interface lands, an overview with no conversation of its own.
pub const HOME_SPACE: &str = "home";

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
    /// Each project's trust as last read or written here, by its root: the topic's value.
    trust: Mutex<BTreeMap<String, Value>>,
    /// Where projects live: what the picker opens on, and what "new project" creates into. A
    /// default, not a boundary — the fence is `$HOME` or this root.
    projects_root: PathBuf,
    /// Bundle uploads granted and not yet imported.
    grants: import::Grants,
}

#[async_trait::async_trait]
impl Extension for HarnixApp {
    type Config = Config;

    fn manifest(&self) -> Manifest {
        Manifest::new(ID, env!("CARGO_PKG_VERSION"))
            .title("Harnix")
            .disclosure("会话 space：工作区、会话的归档、视图偏好、跨会话搜索、目录选择、文件引用、会话项目里的文件（只读）、导出与导入；替设备读、改项目的信任")
            .requires([caps::TRANSCRIPTS_READ, caps::ENGINE_EXPORT])
            // Reading and changing a project's trust (`engine.project.*`). Without it the projects
            // space shows no trust and offers none.
            .optional([caps::ENGINE_CONTROL])
    }

    fn register(&self, r: &mut Registrar<'_, Config>) {
        r.space(SpaceDecl::new(SESSIONS_SPACE).default_space());
        r.space(SpaceDecl::new(HOME_SPACE).no_sessions());
        let open = || Access::device();
        let session = || Access::device().scoped(RowScope::Session);
        r.method("workspaces.list").access(open()).raw(|c, _| with(c, |a, _| async move { a.workspaces_list().await }));
        r.method("workspaces.create").access(open()).raw(|c, p| with(c, |a, _| async move { a.workspaces_create(p).await }));
        r.method("workspaces.update").access(open()).raw(|c, p| with(c, |a, _| async move { a.workspaces_update(p).await }));
        r.method("workspaces.reorder").access(open()).raw(|c, p| with(c, |a, _| async move { a.workspaces_reorder(p).await }));
        r.method("workspaces.remove").access(open()).raw(|c, p| with(c, |a, _| async move { a.workspaces_remove(p).await }));
        r.method("sessions.archive").access(session()).raw(|c, p| with(c, |a, c| async move { a.session_archive(&c, p).await }));
        r.method("sessions.export").access(session()).raw(export::session_export);
        let space = || Access::device().scoped(RowScope::Space);
        r.method("sessions.importUpload")
            .access(space())
            .describe("a one-shot URL to send a session bundle's .zip to")
            .raw(|c, p| with(c, |a, c| async move { a.import_upload(&c, &p) }));
        r.method("sessions.import")
            .access(space())
            .describe("install the uploaded session bundle as new sessions in the space")
            .raw(|c, p| with(c, |a, c| async move { a.import(&c, p).await }));
        r.method("prefs.set").access(open()).raw(|c, p| with(c, |a, _| async move { a.prefs_set(p).await }));
        r.method("search").access(open()).raw(|c, p| with(c, |a, c| async move { a.search(&c, p).await }));
        r.method("files")
            .access(session())
            .describe("files for the `@` menu {session_id | project_root, query?, limit?}: a session's project, or a project here")
            .raw(|c, p| with(c, |a, c| async move { a.files(&c, p).await }));
        r.method("files.list")
            .access(session())
            .describe("one directory of a session's project {session_id, path?, all?}: entries, the hidden count")
            .raw(|c, p| with(c, |a, c| async move { a.files_list(&c, p).await }));
        r.method("files.read")
            .access(session())
            .describe("one file of a session's project {session_id, path}: text, an image, or why neither")
            .raw(|c, p| with(c, |a, c| async move { a.files_read(&c, p).await }));
        r.method("listDirectory").access(open()).raw(|c, p| with(c, |a, _| async move { a.list_directory(p).await }));
        r.method("recentProjects").access(open()).raw(|c, _| with(c, |a, c| async move { a.recent_projects(&c).await }));
        r.method("projects.create").access(open()).raw(|c, p| with(c, |a, _| async move { a.projects_create(p).await }));
        r.method("projects.status")
            .access(open())
            .describe("a project's trust: what waits on it, what is withheld now; only a project added here or a listed session's")
            .raw(|c, p| with(c, |a, c| async move { a.project_status(&c, p).await }));
        r.method("projects.trust")
            .access(open())
            .describe("trust a project, or no longer: its own hooks, MCP servers and looser settings start or stop applying")
            .raw(|c, p| with(c, |a, c| async move { a.project_trust(&c, p).await }));
        r.method("sessions.projectStatus")
            .access(session())
            .describe("the trust of the project a session runs in, and what of it is withheld")
            .raw(|c, p| with(c, |a, c| async move { a.session_project_status(&c, p).await }));
        r.topic(trust::TOPIC);
        r.method("describe")
            .access(open())
            .describe("where this product's projects live")
            .raw(|c: Call, _| async move { Ok(json!({"projects_dir": c.ext().projects_dir()})) });
        // Its UI package defines what else a project tree lists, what a project's overview and the
        // home page show, for others to fill.
        for point in catalog::points_of(ID) {
            r.ui().point(point.decl());
        }
    }

    async fn start(&self, cx: ExtensionContext) -> anyhow::Result<()> {
        let store = Store::open(cx.data_dir().to_path_buf())?;
        cx.provide(App {
            store: Mutex::new(store),
            trust: Mutex::new(BTreeMap::new()),
            projects_root: cx.projects_dir().to_path_buf(),
            grants: Default::default(),
        });
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
