//! Harnix's own contributions.
//!
//! Workspaces, session titles, view preferences, cross-session search, the
//! directory picker, uploads, the settings page and the request envelope are
//! **not kernel**. None of them is the hub, the transport, authorization or
//! assembly; every one is a judgement about how this product behaves, and
//! §2.1 draws the line exactly there.
//!
//! So they live here and register through the public contribution points —
//! the same `hub.method` a plugin uses, with the same names in the same
//! registry. If the built-in features could not be built out of the extension
//! surface, that surface would not be worth believing (§5.5), and this crate
//! is what makes the claim checkable: delete it and the kernel still runs,
//! with fewer methods and no special cases.
//!
//! State lives in Harnix's own directory, never mixed into the engine's root —
//! see [`store`]. Deleting it loses grouping and titles and nothing else.

pub mod files;
pub mod packages;
pub mod plugins;
pub mod recordings;
pub mod search;
pub mod settings;
pub mod store;
pub mod workspaces;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use harnix_contract::{RpcError, Subject};
use harnix_contrib::registry::Owner;
use harnix_contrib::{HostMethod, Registry};
use harnix_hub::Hub;
use serde_json::Value;
use tokio::sync::Mutex;

use store::Store;

/// Told what is installed now. Held by the app, which owns the static face
/// and the hello payload; this crate only knows when to say so.
type PackagesChanged = Box<dyn Fn(Vec<packages::Contributions>) + Send + Sync>;

/// Everything Harnix contributes, and the state behind it.
pub struct Builtin {
    hub: Arc<Hub>,
    store: Mutex<Store>,
    /// One-shot upload grants: token → where the bytes go and the ceiling
    /// the issuing method advertised.
    uploads: Mutex<HashMap<String, (PathBuf, usize)>>,
    upload_dir: PathBuf,
    /// Where projects live: what the picker opens on, and what "new project"
    /// creates into. A default, not a boundary — the fence is `$HOME` or an
    /// explicitly configured root.
    projects_root: PathBuf,
    max_upload_bytes: usize,
    /// Told when the installed set changes, so the static face and the hello
    /// payload can follow without a restart.
    on_packages_changed: Mutex<Option<PackagesChanged>>,
}

/// Every method this crate contributes. Listed once, so authorization and the
/// generated catalog can both read it without tracing dispatch.
pub const METHODS: &[&str] = &[
    "harnix.workspaces.list",
    "harnix.workspaces.create",
    "harnix.workspaces.update",
    "harnix.workspaces.reorder",
    "harnix.workspaces.remove",
    "harnix.sessions.rename",
    "harnix.sessions.archive",
    "harnix.prefs.set",
    "harnix.search",
    "harnix.requestHeaders",
    "harnix.settings.describe",
    "harnix.settings.set",
    "harnix.settings.setProvider",
    "harnix.listDirectory",
    "harnix.recentProjects",
    "harnix.files",
    "harnix.projects.create",
    "harnix.upload.begin",
    // The one thing Agix's installer cannot do for itself: receive a file.
    "harnix.plugins.upload",
    // Lifecycle. The engine does the work; these exist because changing what
    // is installed changes what this host serves — see `plugins`.
    "harnix.plugins.install",
    "harnix.plugins.uninstall",
    "harnix.plugins.enable",
    "harnix.plugins.disable",
    "harnix.plugins.reload",
    "harnix.plugins.list",
];

impl Builtin {
    pub fn new(
        hub: Arc<Hub>,
        state_root: PathBuf,
        projects_root: PathBuf,
    ) -> anyhow::Result<Arc<Self>> {
        let store = Store::open(state_root)?;
        let upload_dir = store
            .root()
            .join("uploads")
            .join(std::process::id().to_string());
        std::fs::create_dir_all(&upload_dir)?;
        Ok(Arc::new(Self {
            hub,
            store: Mutex::new(store),
            uploads: Mutex::new(HashMap::new()),
            upload_dir,
            projects_root,
            max_upload_bytes: harnix_contract::MAX_UPLOAD_BYTES,
            on_packages_changed: Mutex::new(None),
        }))
    }

    pub fn upload_dir(&self) -> &Path {
        &self.upload_dir
    }

    pub fn max_upload_bytes(&self) -> usize {
        self.max_upload_bytes
    }

    pub fn hub(&self) -> &Arc<Hub> {
        &self.hub
    }

    pub async fn on_packages_changed(
        &self,
        f: impl Fn(Vec<packages::Contributions>) + Send + Sync + 'static,
    ) {
        *self.on_packages_changed.lock().await = Some(Box::new(f));
    }

    /// What every installed package contributes to this side.
    pub async fn contributions(&self) -> Vec<packages::Contributions> {
        packages::discover(&self.hub).await
    }

    /// Claim an upload grant. `None` means the token was never issued or has
    /// already been spent.
    pub async fn claim_upload(&self, token: &str) -> Option<(PathBuf, usize)> {
        self.uploads.lock().await.remove(token)
    }

    /// Register every method through the public door.
    pub async fn register(self: &Arc<Self>, registry: &mut Registry) {
        for name in METHODS {
            let entry = Arc::new(Method {
                owner: self.clone(),
                name,
            });
            if let Err(reason) = registry.method(*name, Owner::Builtin, entry) {
                tracing::error!(method = name, %reason, "built-in method not registered");
            }
        }
    }

    async fn dispatch(&self, name: &str, params: Value) -> Result<Value, RpcError> {
        match name {
            "harnix.workspaces.list" => self.workspaces_list().await,
            "harnix.workspaces.create" => self.workspaces_create(params).await,
            "harnix.workspaces.update" => self.workspaces_update(params).await,
            "harnix.workspaces.reorder" => self.workspaces_reorder(params).await,
            "harnix.workspaces.remove" => self.workspaces_remove(params).await,
            "harnix.sessions.rename" => self.session_rename(params).await,
            "harnix.sessions.archive" => self.session_archive(params).await,
            "harnix.prefs.set" => self.prefs_set(params).await,
            "harnix.search" => self.search(params).await,
            "harnix.requestHeaders" => self.request_headers(params).await,
            "harnix.settings.describe" => self.settings_describe(params).await,
            "harnix.settings.set" => self.settings_set(params).await,
            "harnix.settings.setProvider" => self.settings_set_provider(params).await,
            "harnix.listDirectory" => self.list_directory(params).await,
            "harnix.recentProjects" => self.recent_projects().await,
            "harnix.files" => self.files(params).await,
            "harnix.projects.create" => self.projects_create(params).await,
            "harnix.upload.begin" => self.upload_begin(params).await,
            "harnix.plugins.upload" => self.plugins_upload(params).await,
            "harnix.plugins.install" => self.plugins_install(params).await,
            "harnix.plugins.uninstall" => self.plugins_manage("plugin.uninstall", params).await,
            "harnix.plugins.enable" => self.plugins_manage("plugin.enable", params).await,
            "harnix.plugins.disable" => self.plugins_manage("plugin.disable", params).await,
            "harnix.plugins.reload" => self.plugins_manage("plugin.reload", params).await,
            "harnix.plugins.list" => self.plugins_list(params).await,
            other => Err(RpcError::not_found(format!(
                "`{other}` is not a built-in method"
            ))),
        }
    }
}

/// Uploads are a contribution's business, not the kernel's, so transport
/// asks through a trait rather than knowing whose they are.
#[async_trait::async_trait]
impl harnix_transport::BulkStore for Builtin {
    async fn claim(&self, token: &str) -> Option<(PathBuf, usize)> {
        self.claim_upload(token).await
    }
}

/// One registered name. A thin adapter so the registry holds a uniform entry
/// rather than eighteen closures.
struct Method {
    owner: Arc<Builtin>,
    name: &'static str,
}

#[async_trait::async_trait]
impl HostMethod for Method {
    async fn call(&self, _subject: &Subject, params: Value) -> Result<Value, RpcError> {
        self.owner.dispatch(self.name, params).await
    }
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

pub(crate) fn new_id(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4().simple())
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
