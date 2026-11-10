//! Harnix's own contributions.
//!
//! Workspaces, session titles, view preferences, cross-session search, the
//! directory picker, uploads, the settings page and the request envelope are
//! **not kernel**. None of them is the hub, the transport, authorization or
//! assembly; every one is a judgement about how this product behaves, and
//! concept §2.1 draws the line exactly there.
//!
//! So they live here, as compile-time modules registered through the same
//! contract an application's modules use (concept §4.2). This crate depends on
//! `harnix-module`'s public API and on nothing inside the hub; a test in the
//! app holds it to that.
//!
//! State lives in Harnix's own directory, never mixed into the engine's root —
//! see [`store`]. Deleting it loses grouping and titles and nothing else.

pub mod apps;
mod export;
pub mod files;
pub mod packages;
pub mod plugins;
pub mod recordings;
pub mod search;
pub mod settings;
pub mod store;
mod system;
mod usage;
pub mod workspaces;

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, RwLock};

use harnix_contract::sync::{read, write};
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
    /// `harnix.system.usage`'s summaries, by recording, until the recording changes.
    usage: Arc<std::sync::Mutex<usage::UsageCache>>,
    /// The system space's traces, by recording, until the recording changes.
    traces: Arc<recordings::Traces>,
}

/// The built-in modules: id, methods (full names), grants, disclosure.
///
/// Every one registers through the public module contract, the same one an
/// application's module uses. If the built-ins could not be built out of it,
/// the contract would not be worth believing (concept §3); delete this crate and
/// the kernel still runs, with fewer methods and no special cases.
const PARTS: &[Part] = &[
    Part {
        id: "sessions",
        disclosure: "工作区、会话标题与归档、视图偏好、跨会话搜索、目录选择、文件引用、上传",
        methods: &[
            m("harnix.workspaces.list", |b, _| Box::pin(async move { b.workspaces_list().await })),
            m("harnix.workspaces.create", |b, p| Box::pin(async move { b.workspaces_create(p).await })),
            m("harnix.workspaces.update", |b, p| Box::pin(async move { b.workspaces_update(p).await })),
            m("harnix.workspaces.reorder", |b, p| Box::pin(async move { b.workspaces_reorder(p).await })),
            m("harnix.workspaces.remove", |b, p| Box::pin(async move { b.workspaces_remove(p).await })),
            m("harnix.sessions.rename", |b, p| Box::pin(async move { b.session_rename(p).await })),
            m("harnix.sessions.archive", |b, p| Box::pin(async move { b.session_archive(p).await })),
            m("harnix.sessions.export", |b, p| Box::pin(async move { b.session_export(p).await })),
            m("harnix.prefs.set", |b, p| Box::pin(async move { b.prefs_set(p).await })),
            m("harnix.search", |b, p| Box::pin(async move { b.search(p).await })),
            m("harnix.listDirectory", |b, p| Box::pin(async move { b.list_directory(p).await })),
            m("harnix.recentProjects", |b, _| Box::pin(async move { b.recent_projects().await })),
            m("harnix.files", |b, p| Box::pin(async move { b.files(p).await })),
            m("harnix.projects.create", |b, p| Box::pin(async move { b.projects_create(p).await })),
            m("harnix.upload.begin", |b, p| Box::pin(async move { b.upload_begin(p).await })),
        ],
        grants: &["session.export"],
    },
    Part {
        id: "settings",
        disclosure: "读写引擎的设置三层；写 provider 的凭据与端点",
        methods: &[
            m("harnix.settings.describe", |b, p| Box::pin(async move { b.settings_describe(p).await })),
            m("harnix.settings.set", |b, p| Box::pin(async move { b.settings_set(p).await })),
            m("harnix.settings.setProvider", |b, p| Box::pin(async move { b.settings_set_provider(p).await })),
        ],
        // `harnix.instances.inheritProviders` is the hub's, not the engine's: after a provider
        // changes, the instances whose template inherits `system`'s get it too (ipdrun-11).
        grants: &[
            "config.get",
            "config.getProvider",
            "config.reload",
            "harnix.instances.inheritProviders",
        ],
    },
    Part {
        id: "packages",
        disclosure: "接收插件包文件，经 Agix 安装、卸载、启停；读包里的界面段",
        methods: &[
            // The one thing Agix's installer cannot do for itself: receive a file.
            m("harnix.plugins.upload", |b, p| Box::pin(async move { b.plugins_upload(p).await })),
            // Lifecycle. The engine does the work; these exist because
            // changing what is installed changes what this host serves.
            m("harnix.plugins.install", |b, p| Box::pin(async move { b.plugins_install(p).await })),
            m("harnix.plugins.uninstall", |b, p| Box::pin(async move { b.plugins_manage("plugin.uninstall", p).await })),
            m("harnix.plugins.enable", |b, p| Box::pin(async move { b.plugins_manage("plugin.enable", p).await })),
            m("harnix.plugins.disable", |b, p| Box::pin(async move { b.plugins_manage("plugin.disable", p).await })),
            m("harnix.plugins.reload", |b, p| Box::pin(async move { b.plugins_manage("plugin.reload", p).await })),
            m("harnix.plugins.list", |b, p| Box::pin(async move { b.plugins_list(p).await })),
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
        methods: &[
            m("harnix.requestHeaders", |b, p| Box::pin(async move { b.request_headers(p).await })),
        ],
        grants: &[],
    },
];

/// The system space's backend: registered only when the profile keeps it on, so turning it off
/// takes its methods out of the authorization table rather than only out of sight (concept §5.5).
const SYSTEM: Part = Part {
    id: "system",
    disclosure: "系统 space：看任何实例的会话与任务、读调用信封与上下文、任务的输出、会话的用量；给在跑的子 Agent 发话；设定、暂停、继续、编辑、清除会话的目标；压缩、清空、换模型、改名、改写或删除消息、停止任务。每次操作写审计",
    methods: &[
        m("harnix.system.context", |b, p| Box::pin(async move { b.system_call("harnix.system.context", p).await })),
        m("harnix.system.calls", |b, p| Box::pin(async move { b.system_call("harnix.system.calls", p).await })),
        m("harnix.system.trace", |b, p| Box::pin(async move { b.system_call("harnix.system.trace", p).await })),
        m("harnix.system.trace.call", |b, p| Box::pin(async move { b.system_call("harnix.system.trace.call", p).await })),
        m("harnix.system.turns", |b, p| Box::pin(async move { b.system_call("harnix.system.turns", p).await })),
        m("harnix.system.usage", |b, p| Box::pin(async move { b.system_call("harnix.system.usage", p).await })),
        m("harnix.system.tasks.stop", |b, p| Box::pin(async move { b.system_call("harnix.system.tasks.stop", p).await })),
        m("harnix.system.tasks.message", |b, p| Box::pin(async move { b.system_call("harnix.system.tasks.message", p).await })),
        m("harnix.system.tasks.output", |b, p| Box::pin(async move { b.system_call("harnix.system.tasks.output", p).await })),
        m("harnix.system.messages.drop", |b, p| Box::pin(async move { b.system_call("harnix.system.messages.drop", p).await })),
        m("harnix.system.messages.edit", |b, p| Box::pin(async move { b.system_call("harnix.system.messages.edit", p).await })),
        m("harnix.system.compact", |b, p| Box::pin(async move { b.system_call("harnix.system.compact", p).await })),
        m("harnix.system.clear", |b, p| Box::pin(async move { b.system_call("harnix.system.clear", p).await })),
        m("harnix.system.model", |b, p| Box::pin(async move { b.system_call("harnix.system.model", p).await })),
        m("harnix.system.rename", |b, p| Box::pin(async move { b.system_call("harnix.system.rename", p).await })),
        m("harnix.system.goal", |b, p| Box::pin(async move { b.system_call("harnix.system.goal", p).await })),
        m("harnix.system.goal.set", |b, p| Box::pin(async move { b.system_call("harnix.system.goal.set", p).await })),
        m("harnix.system.goal.update", |b, p| Box::pin(async move { b.system_call("harnix.system.goal.update", p).await })),
    ],
    grants: &[
        "session.inspectContext",
        "session.stopTask",
        "session.messageTask",
        "session.taskOutput",
        "session.dropMessages",
        "session.editMessage",
        "session.compact",
        "session.clear",
        "session.setModel",
        "session.rename",
        "session.goal",
        "session.setGoal",
        "session.updateGoal",
    ],
};

/// Changing a session's model from the composer when the system space is gone: registered only
/// with `[shell] model_switch = true`, and scoped to the space the caller claims (ipdrun-12).
const MODEL: Part = Part {
    id: "model",
    disclosure: "换会话的模型：只在调用方声称的 space 自己的会话上（[shell] model_switch）",
    methods: &[m("harnix.sessions.setModel", |b, p| Box::pin(async move { b.session_set_model(p).await }))],
    grants: &["session.setModel"],
};

/// Every method the built-ins contribute, the system space's included.
pub fn methods() -> Vec<&'static str> {
    PARTS
        .iter()
        .chain([&SYSTEM, &MODEL])
        .flat_map(|p| p.methods.iter().map(|m| m.name))
        .collect()
}

/// Engine methods a built-in calls that no device may: granted to the one module that needs
/// each.
pub fn engine_methods() -> Vec<&'static str> {
    let mut all: Vec<&'static str> = PARTS
        .iter()
        .chain([&SYSTEM, &MODEL])
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
            usage: Default::default(),
            traces: Default::default(),
        }))
    }

    /// The built-in modules, in the order they register; the system space's last, when it is on.
    pub fn modules(self: &Arc<Self>, system_space: bool) -> Vec<BuiltinModule> {
        self.modules_with(system_space, false)
    }

    /// [`Builtin::modules`], and `harnix.sessions.setModel` with `model_switch`
    /// (`[shell] model_switch`).
    pub fn modules_with(self: &Arc<Self>, system_space: bool, model_switch: bool) -> Vec<BuiltinModule> {
        PARTS
            .iter()
            .chain(system_space.then_some(&SYSTEM))
            .chain(model_switch.then_some(&MODEL))
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
        read(&self.contexts)
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
        read(&self.contexts)
            .values()
            .find_map(|cx| cx.instance(id))
    }

    /// The `system` instance's facts: where its data is, what it runs.
    ///
    /// An error rather than a panic before any built-in module has started: methods are bound
    /// before `start`, so a module ordered ahead of the built-ins that calls one while it starts
    /// reaches here first, and a panic there took its whole call with it.
    pub(crate) fn system(&self) -> Result<InstanceHandle, RpcError> {
        read(&self.contexts)
            .values()
            .find_map(|cx| cx.instance(harnix_contract::SYSTEM_INSTANCE))
            .ok_or_else(|| {
                RpcError::new(
                    harnix_contract::codes::INTERNAL_ERROR,
                    "the built-in modules have not started yet",
                )
            })
    }
}

/// Uploads are a module's business, not the kernel's, so transport asks
/// through a trait rather than knowing whose they are.
#[async_trait::async_trait]
impl harnix_module::BulkStore for Builtin {
    async fn claim(&self, token: &str) -> Option<(PathBuf, usize)> {
        self.claim_upload(token).await
    }
}

struct Part {
    id: &'static str,
    disclosure: &'static str,
    methods: &'static [Method],
    grants: &'static [&'static str],
}

/// What answers one built-in method: a function of the built-ins' state and the params.
type Answer = fn(Arc<Builtin>, Value) -> Pin<Box<dyn Future<Output = Result<Value, RpcError>> + Send>>;

/// One built-in method: its full name, and what answers it.
struct Method {
    name: &'static str,
    answer: Answer,
}

const fn m(name: &'static str, answer: Answer) -> Method {
    Method { name, answer }
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
        for method in self.part.methods {
            let short = method.name.strip_prefix("harnix.").unwrap_or(method.name);
            registry
                .method(short, Principal::Device)
                .allow(Principal::AnyModule)
                .allow(Principal::Kernel)
                .with(Arc::new(Bound {
                    builtin: self.builtin.clone(),
                    answer: method.answer,
                }));
        }
    }

    async fn start(&self, cx: ModuleContext) -> anyhow::Result<()> {
        write(&self.builtin.contexts)
            .insert(self.part.id, cx);
        Ok(())
    }
}

/// One registered method, with the state it answers from.
struct Bound {
    builtin: Arc<Builtin>,
    answer: Answer,
}

#[async_trait::async_trait]
impl Handler for Bound {
    async fn call(&self, _cx: CallContext, params: Value) -> Result<Value, RpcError> {
        (self.answer)(self.builtin.clone(), params).await
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    /// A built-in method reached before any built-in module started answers an error; it used to
    /// panic, taking the caller's whole call with it.
    #[tokio::test]
    async fn a_method_before_start_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let builtin = super::Builtin::new(dir.path().join("state"), dir.path().join("projects")).unwrap();
        assert!(builtin.system().is_err());
        let e = builtin.settings_describe(json!({})).await.unwrap_err();
        assert_eq!(e.code, harnix_contract::codes::INTERNAL_ERROR);
        assert!(builtin.settings_set(json!({"tier": "global", "key": "model.max_tokens", "value": 4000})).await.is_err());
    }
}
