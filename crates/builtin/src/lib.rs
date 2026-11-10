//! Harnix's own contributions.
//!
//! Workspaces, session titles, view preferences, cross-session search, the
//! directory picker and uploads are **not kernel**. None of them is the hub,
//! the transport, authorization or assembly; every one is a judgement about
//! how this product behaves, and concept §2.1 draws the line exactly there.
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
pub mod search;
pub mod store;
mod system;

pub use system::SYSTEM_SPACE;
pub mod workspaces;

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, RwLock};

use harnix_contract::sync::{read, write};
use harnix_contract::RpcError;
use harnix_module::{
    caps, CallContext, Capability, Handler, InstanceHandle, Manifest, MethodRegistry, Module, ModuleContext,
    Principal, SpaceDecl, SpaceRegistry,
};
use serde_json::Value;
use tokio::sync::Mutex;

use store::Store;

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
    /// Each built-in module's context, by id, set when it starts. A call a
    /// built-in makes goes out as that module, so the audit names it and a
    /// grant (settings writing a provider) applies to it alone.
    contexts: RwLock<HashMap<&'static str, ModuleContext>>,
}

/// The built-in modules: id, methods (full names), the capabilities each requires, disclosure.
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
        requires: &[caps::ENGINE_EXPORT],
    },
];

/// The system space's backend: registered only when the profile keeps it on, so turning it off
/// takes its methods out of the authorization table rather than only out of sight (concept §5.5).
const SYSTEM: Part = Part {
    id: "system",
    disclosure: "系统 space：看任何实例的会话与任务、读上下文、任务的输出；给在跑的子 Agent 发话；设定、暂停、继续、编辑、清除会话的目标；压缩、清空、换模型、改名、改写或删除消息、停止任务。每次操作写审计",
    methods: &[
        mc("harnix.system.context", |b, cx, p| Box::pin(async move { b.system_call(&cx, "harnix.system.context", p).await })),
        mc("harnix.system.tasks.stop", |b, cx, p| Box::pin(async move { b.system_call(&cx, "harnix.system.tasks.stop", p).await })),
        mc("harnix.system.tasks.message", |b, cx, p| Box::pin(async move { b.system_call(&cx, "harnix.system.tasks.message", p).await })),
        mc("harnix.system.tasks.output", |b, cx, p| Box::pin(async move { b.system_call(&cx, "harnix.system.tasks.output", p).await })),
        mc("harnix.system.messages.drop", |b, cx, p| Box::pin(async move { b.system_call(&cx, "harnix.system.messages.drop", p).await })),
        mc("harnix.system.messages.edit", |b, cx, p| Box::pin(async move { b.system_call(&cx, "harnix.system.messages.edit", p).await })),
        mc("harnix.system.compact", |b, cx, p| Box::pin(async move { b.system_call(&cx, "harnix.system.compact", p).await })),
        mc("harnix.system.clear", |b, cx, p| Box::pin(async move { b.system_call(&cx, "harnix.system.clear", p).await })),
        mc("harnix.system.model", |b, cx, p| Box::pin(async move { b.system_call(&cx, "harnix.system.model", p).await })),
        mc("harnix.system.rename", |b, cx, p| Box::pin(async move { b.system_call(&cx, "harnix.system.rename", p).await })),
        mc("harnix.system.goal", |b, cx, p| Box::pin(async move { b.system_call(&cx, "harnix.system.goal", p).await })),
        mc("harnix.system.goal.set", |b, cx, p| Box::pin(async move { b.system_call(&cx, "harnix.system.goal.set", p).await })),
        mc("harnix.system.goal.update", |b, cx, p| Box::pin(async move { b.system_call(&cx, "harnix.system.goal.update", p).await })),
        // Stopping, restarting and building any instance, for the system space's instance pane:
        // the hub lets an extension holding `instances.admin` do that, and a device none of it.
        mc("harnix.system.instances.stop", |b, cx, p| Box::pin(async move { b.system_call(&cx, "harnix.system.instances.stop", p).await })),
        mc("harnix.system.instances.restart", |b, cx, p| Box::pin(async move { b.system_call(&cx, "harnix.system.instances.restart", p).await })),
        mc("harnix.system.instances.create", |b, cx, p| Box::pin(async move { b.system_call(&cx, "harnix.system.instances.create", p).await })),
        mc("harnix.system.instances.templates", |b, cx, p| Box::pin(async move { b.system_call(&cx, "harnix.system.instances.templates", p).await })),
    ],
    requires: &[caps::INSTANCES_ADMIN, caps::ENGINE_INSPECT, caps::ENGINE_CONTROL],
};

/// Changing a session's model from the composer when the system space is gone: registered only
/// with `[shell] model_switch = true`, and scoped to the space the caller claims (ipdrun-12).
const MODEL: Part = Part {
    id: "model",
    disclosure: "换会话的模型：只在调用方声称的 space 自己的会话上（[shell] model_switch）",
    methods: &[m("harnix.sessions.setModel", |b, p| Box::pin(async move { b.session_set_model(p).await }))],
    requires: &[caps::ENGINE_CONTROL],
};

/// Every method the built-ins contribute, the system space's included.
pub fn methods() -> Vec<&'static str> {
    PARTS
        .iter()
        .chain([&SYSTEM, &MODEL])
        .flat_map(|p| p.methods.iter().map(|m| m.name))
        .collect()
}

/// Every capability a built-in requires.
pub fn capabilities() -> Vec<Capability> {
    let mut all: Vec<Capability> = PARTS
        .iter()
        .chain([&SYSTEM, &MODEL])
        .flat_map(|p| p.requires.iter().cloned())
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
            contexts: RwLock::new(HashMap::new()),
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
    requires: &'static [Capability],
}

type Answering = Pin<Box<dyn Future<Output = Result<Value, RpcError>> + Send>>;

/// What answers one built-in method: a function of the built-ins' state and the params, or — for
/// what calls on while it answers, so those calls are this one's children — of the call too.
#[derive(Clone, Copy)]
enum Answer {
    Plain(fn(Arc<Builtin>, Value) -> Answering),
    Within(fn(Arc<Builtin>, CallContext, Value) -> Answering),
}

/// One built-in method: its full name, and what answers it.
struct Method {
    name: &'static str,
    answer: Answer,
}

const fn m(name: &'static str, answer: fn(Arc<Builtin>, Value) -> Answering) -> Method {
    Method { name, answer: Answer::Plain(answer) }
}

const fn mc(name: &'static str, answer: fn(Arc<Builtin>, CallContext, Value) -> Answering) -> Method {
    Method { name, answer: Answer::Within(answer) }
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
        Manifest::new(self.part.id, env!("CARGO_PKG_VERSION"))
            .disclosure(self.part.disclosure)
            .requires(self.part.requires.iter().cloned())
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

    /// The system space is the system module's: it reaches across spaces.
    fn spaces(&self, spaces: &mut SpaceRegistry) {
        if self.part.id == SYSTEM.id {
            spaces.add(SpaceDecl::new(SYSTEM_SPACE).cross_space());
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
    async fn call(&self, cx: CallContext, params: Value) -> Result<Value, RpcError> {
        match self.answer {
            Answer::Plain(f) => f(self.builtin.clone(), params).await,
            Answer::Within(f) => f(self.builtin.clone(), cx, params).await,
        }
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
    /// A built-in method reached before any built-in module started answers an error; it used to
    /// panic, taking the caller's whole call with it.
    #[tokio::test]
    async fn a_method_before_start_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let builtin = super::Builtin::new(dir.path().join("state"), dir.path().join("projects")).unwrap();
        assert_eq!(builtin.system().err().map(|e| e.code), Some(harnix_contract::codes::INTERNAL_ERROR));
    }
}
