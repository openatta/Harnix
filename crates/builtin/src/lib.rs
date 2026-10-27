//! Harnix's own contributions.
//!
//! Workspaces, session titles, view preferences, cross-session search, the
//! directory picker, uploads, the settings page and the request envelope are
//! **not kernel**. None of them is the hub, the transport, authorization or
//! assembly; every one is a judgement about how this product behaves, and
//! §2.1 draws the line exactly there.
//!
//! So they live here, as compile-time modules registered through the same
//! contract an application's modules use (design §7). This crate depends on
//! `harnix-module`'s public API and on nothing inside the hub; a test in the
//! app holds it to that.
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
mod system;
pub mod workspaces;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use harnix_contract::RpcError;
use harnix_module::{
    CallContext, Handler, InstanceHandle, Manifest, MethodRegistry, Module, ModuleContext,
    Principal,
};
use serde_json::Value;
use tokio::sync::Mutex;

use store::Store;

/// Told what is installed now. Held by the app, which owns the static face
/// and the hello payload; this crate only knows when to say so.
type PackagesChanged = Box<dyn Fn(Vec<packages::Contributions>) + Send + Sync>;

/// The state behind Harnix's built-in modules, shared by all of them.
pub struct Builtin {
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
    /// Each built-in module's context, by id, set when it starts. A call a
    /// built-in makes goes out as that module, so the audit names it and a
    /// grant (settings writing a provider) applies to it alone.
    contexts: RwLock<HashMap<&'static str, ModuleContext>>,
}

/// The built-in modules: id, methods (full names), grants, disclosure.
///
/// Every one registers through the public module contract, the same one an
/// application's module uses. If the built-ins could not be built out of it,
/// the contract would not be worth believing (§5.5); delete this crate and
/// the kernel still runs, with fewer methods and no special cases.
const PARTS: &[Part] = &[
    Part {
        id: "sessions",
        disclosure: "工作区、会话标题与归档、视图偏好、跨会话搜索、目录选择、文件引用、上传",
        methods: &[
            "harnix.workspaces.list",
            "harnix.workspaces.create",
            "harnix.workspaces.update",
            "harnix.workspaces.reorder",
            "harnix.workspaces.remove",
            "harnix.sessions.rename",
            "harnix.sessions.archive",
            "harnix.prefs.set",
            "harnix.search",
            "harnix.listDirectory",
            "harnix.recentProjects",
            "harnix.files",
            "harnix.projects.create",
            "harnix.upload.begin",
        ],
        grants: &[],
    },
    Part {
        id: "settings",
        disclosure: "读写引擎的设置三层；写 provider 的凭据与端点",
        methods: &[
            "harnix.settings.describe",
            "harnix.settings.set",
            "harnix.settings.setProvider",
        ],
        grants: &["config.get", "config.getProvider", "config.reload"],
    },
    Part {
        id: "packages",
        disclosure: "接收插件包文件，经 Agix 安装、卸载、启停；读包里的界面段",
        methods: &[
            // The one thing Agix's installer cannot do for itself: receive a file.
            "harnix.plugins.upload",
            // Lifecycle. The engine does the work; these exist because
            // changing what is installed changes what this host serves.
            "harnix.plugins.install",
            "harnix.plugins.uninstall",
            "harnix.plugins.enable",
            "harnix.plugins.disable",
            "harnix.plugins.reload",
            "harnix.plugins.list",
        ],
        grants: &[
            "plugin.list",
            "plugin.install",
            "plugin.uninstall",
            "plugin.enable",
            "plugin.disable",
            "plugin.reload",
        ],
    },
    Part {
        id: "recordings",
        disclosure: "读本机的模型调用录像（请求信封）",
        methods: &["harnix.requestHeaders"],
        grants: &[],
    },
];

/// The system space's backend: registered only when the profile keeps it on, so turning it off
/// takes its methods out of the authorization table rather than only out of sight (design §9.1).
const SYSTEM: Part = Part {
    id: "system",
    disclosure: "系统 space：看任何实例的会话与任务、读调用信封与上下文、任务的输出；压缩、清空、换模型、改名、改写或删除消息、停止任务。每次操作写审计",
    methods: &[
        "harnix.system.context",
        "harnix.system.calls",
        "harnix.system.trace",
        "harnix.system.trace.call",
        "harnix.system.tasks.stop",
        "harnix.system.tasks.output",
        "harnix.system.messages.drop",
        "harnix.system.messages.edit",
        "harnix.system.compact",
        "harnix.system.clear",
        "harnix.system.model",
        "harnix.system.rename",
    ],
    grants: &[
        "session.inspectContext",
        "session.stopTask",
        "session.taskOutput",
        "session.dropMessages",
        "session.editMessage",
        "session.compact",
        "session.clear",
        "session.setModel",
        "session.rename",
    ],
};

/// Every method the built-ins contribute, the system space's included.
pub fn methods() -> Vec<&'static str> {
    PARTS
        .iter()
        .chain([&SYSTEM])
        .flat_map(|p| p.methods.iter().copied())
        .collect()
}

/// Engine methods a built-in calls that no device may: in the table as the
/// kernel's, and granted to the one module that needs each.
pub fn engine_methods() -> Vec<&'static str> {
    let mut all: Vec<&'static str> = PARTS
        .iter()
        .chain([&SYSTEM])
        .flat_map(|p| p.grants.iter().copied())
        .collect();
    all.sort();
    all.dedup();
    all
}

impl Builtin {
    pub fn new(state_root: PathBuf, projects_root: PathBuf) -> anyhow::Result<Arc<Self>> {
        let store = Store::open(state_root)?;
        let upload_dir = store
            .root()
            .join("uploads")
            .join(std::process::id().to_string());
        std::fs::create_dir_all(&upload_dir)?;
        Ok(Arc::new(Self {
            store: Mutex::new(store),
            uploads: Mutex::new(HashMap::new()),
            upload_dir,
            projects_root,
            max_upload_bytes: harnix_contract::MAX_UPLOAD_BYTES,
            on_packages_changed: Mutex::new(None),
            contexts: RwLock::new(HashMap::new()),
        }))
    }

    /// The built-in modules, in the order they register; the system space's last, when it is on.
    pub fn modules(self: &Arc<Self>, system_space: bool) -> Vec<BuiltinModule> {
        PARTS
            .iter()
            .chain(system_space.then_some(&SYSTEM))
            .map(|part| BuiltinModule {
                builtin: self.clone(),
                part,
            })
            .collect()
    }

    pub fn upload_dir(&self) -> &Path {
        &self.upload_dir
    }

    pub fn max_upload_bytes(&self) -> usize {
        self.max_upload_bytes
    }

    pub async fn on_packages_changed(
        &self,
        f: impl Fn(Vec<packages::Contributions>) + Send + Sync + 'static,
    ) {
        *self.on_packages_changed.lock().await = Some(Box::new(f));
    }

    /// What every installed package contributes to this side. Empty until
    /// the packages module has started.
    pub async fn contributions(&self) -> Vec<packages::Contributions> {
        match self.context("packages") {
            Some(cx) => packages::discover(&cx).await,
            None => Vec::new(),
        }
    }

    /// Claim an upload grant. `None` means the token was never issued or has
    /// already been spent.
    pub async fn claim_upload(&self, token: &str) -> Option<(PathBuf, usize)> {
        self.uploads.lock().await.remove(token)
    }

    fn context(&self, id: &str) -> Option<ModuleContext> {
        self.contexts
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }

    /// One call as a built-in module, through the admission point.
    pub(crate) async fn call(
        &self,
        module: &str,
        method: &str,
        params: Value,
    ) -> Result<Value, RpcError> {
        let cx = self.context(module).ok_or_else(|| {
            RpcError::new(
                harnix_contract::codes::INTERNAL_ERROR,
                format!("the `{module}` module has not started"),
            )
        })?;
        cx.call(method, params).await
    }

    /// One instance's facts.
    pub(crate) fn instance(&self, id: &str) -> Option<InstanceHandle> {
        self.contexts
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .find_map(|cx| cx.instance(id))
    }

    /// The `system` instance's facts: where its data is, what it runs.
    pub(crate) fn system(&self) -> InstanceHandle {
        self.contexts
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .find_map(|cx| cx.instance(harnix_contract::SYSTEM_INSTANCE))
            .expect("a built-in method ran before any built-in module started")
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
            name if name.starts_with("harnix.system.") => self.system_call(name, params).await,
            other => Err(RpcError::not_found(format!(
                "`{other}` is not a built-in method"
            ))),
        }
    }
}

/// Uploads are a module's business, not the kernel's, so transport asks
/// through a trait rather than knowing whose they are.
#[async_trait::async_trait]
impl harnix_transport::BulkStore for Builtin {
    async fn claim(&self, token: &str) -> Option<(PathBuf, usize)> {
        self.claim_upload(token).await
    }
}

struct Part {
    id: &'static str,
    disclosure: &'static str,
    methods: &'static [&'static str],
    grants: &'static [&'static str],
}

/// One built-in module.
pub struct BuiltinModule {
    builtin: Arc<Builtin>,
    part: &'static Part,
}

#[async_trait::async_trait]
impl Module for BuiltinModule {
    const API_VERSION: u32 = harnix_module::MODULE_API_VERSION;

    fn manifest(&self) -> Manifest {
        let mut manifest =
            Manifest::new(self.part.id, env!("CARGO_PKG_VERSION")).disclosure(self.part.disclosure);
        for grant in self.part.grants {
            manifest = manifest.grant(*grant);
        }
        manifest
    }

    fn methods(&self, registry: &mut MethodRegistry) {
        for name in self.part.methods {
            let short = name.strip_prefix("harnix.").unwrap_or(name);
            registry
                .method(short, Principal::Device)
                .allow(Principal::AnyModule)
                .allow(Principal::Kernel)
                .with(Arc::new(Method {
                    builtin: self.builtin.clone(),
                    name,
                }));
        }
    }

    async fn start(&self, cx: ModuleContext) -> anyhow::Result<()> {
        self.builtin
            .contexts
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(self.part.id, cx);
        Ok(())
    }
}

/// One registered name. A thin adapter so the registry holds a uniform entry
/// rather than twenty-five closures.
struct Method {
    builtin: Arc<Builtin>,
    name: &'static str,
}

#[async_trait::async_trait]
impl Handler for Method {
    async fn call(&self, _cx: CallContext, params: Value) -> Result<Value, RpcError> {
        self.builtin.dispatch(self.name, params).await
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
