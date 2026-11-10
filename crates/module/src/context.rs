//! What a module is handed: at migration, at start, and on every call.
//!
//! [`ModuleContext`] is the whole of a module's reach into the process: its
//! configuration section, its data directory, its credentials, its topics,
//! calls through the admission point, and read-only facts about instances.
//! There is no hub and no engine link in it — a module that wants a session
//! asks for one through a method, like everyone else.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use harnix_contract::sync::{read, write};
use harnix_contract::{ExtId, RpcError, Subject};
use serde::de::DeserializeOwned;
use serde_json::Value;

use harnix_ext::Credentials;
use crate::events::{Events, TopicDecl};

/// The process side of a module's context: implemented by the application,
/// not by modules.
#[async_trait::async_trait]
pub trait Kernel: Send + Sync {
    /// One call through the admission point, as `subject`.
    async fn call(&self, subject: &Subject, method: &str, params: Value) -> Result<Value, RpcError>;
    /// One call through the admission point, made as `cx` says: its subject, and the call it was
    /// made while handling. A kernel that keeps no such record answers it as [`Kernel::call`].
    async fn call_in(&self, cx: harnix_kernel::CallContext, method: &str, params: Value) -> Result<Value, RpcError> {
        self.call(&cx.subject, method, params).await
    }
    /// Publish a whole-value snapshot on a topic.
    async fn publish(&self, topic: &str, params: Value, value: Value);
    /// Facts about one instance.
    fn instance(&self, id: &str) -> Option<InstanceHandle>;
    /// Watch a session from the server side, as `subject`.
    async fn tap(
        &self,
        subject: &Subject,
        session: &crate::SessionRef,
        capacity: usize,
    ) -> Result<crate::Tap, RpcError>;
    /// Where this process listens: `None` inside until the listener is bound. A kernel that
    /// never listens returns no receiver at all.
    fn listen(&self) -> Option<tokio::sync::watch::Receiver<Option<Listen>>> {
        None
    }
    /// The runtime-application host, for the module `module` — `None` unless
    /// this process hosts applications (`[apps] enabled`) **and** `module` is
    /// the one that hosts them: the `apps` built-in. What enters the
    /// authorization table at run time comes through one door, and a
    /// compiled-in module that wants names of its own declares them at
    /// registration like every other.
    fn apps(&self, module: &str) -> Option<Arc<dyn crate::apps::AppRuntime>> {
        let _ = module;
        None
    }
}

/// Where this process accepts connections (ipdrun-14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listen {
    /// Every address this process accepts on — both loopback families when bound to loopback —
    /// with the port the system chose when `--port 0` asked it to.
    pub addrs: Vec<std::net::SocketAddr>,
    /// The one to give a person or a tool: `http(s)://<first addr>/`. It carries no token.
    pub url: String,
}

/// Read-only facts about one instance: where its data is, what it runs. Not
/// a way to call it — calls go through [`ModuleContext::call`].
#[derive(Clone)]
pub struct InstanceHandle {
    pub id: String,
    /// The engine's data root: settings tiers, transcripts, memory.
    pub data_root: PathBuf,
    /// Where each session's model-call recording is, one directory per
    /// session id.
    pub recordings_root: PathBuf,
    /// The transcript store, for readers that would otherwise page the RPC
    /// surface to get at text (search).
    pub history: Option<Arc<dyn harnix_instance::HistoryStore>>,
    pub active_scenes: Vec<String>,
    pub model: String,
}

struct Shared {
    id: String,
    ext: ExtId,
    config: Value,
    data_dir: PathBuf,
    credentials: Arc<dyn Credentials>,
    kernel: Arc<dyn Kernel>,
    events: Events,
    state: RwLock<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

/// One module's context. Cheap to clone; every clone is the same module.
#[derive(Clone)]
pub struct ModuleContext {
    shared: Arc<Shared>,
}

impl ModuleContext {
    #[doc(hidden)]
    pub fn new(
        id: &str,
        prefix: &str,
        config: Value,
        data_dir: PathBuf,
        credentials: Arc<dyn Credentials>,
        kernel: Arc<dyn Kernel>,
        topics: Vec<TopicDecl>,
    ) -> Self {
        let events = Events {
            kernel: kernel.clone(),
            prefix: prefix.to_string(),
            topics: Arc::new(topics),
        };
        Self {
            shared: Arc::new(Shared {
                id: id.to_string(),
                ext: ExtId::parse(id).expect("a module is started only once its id was checked"),
                config,
                data_dir,
                credentials,
                kernel,
                events,
                state: RwLock::new(HashMap::new()),
            }),
        }
    }

    pub fn id(&self) -> &str {
        &self.shared.id
    }

    /// `[ext.<id>]`, deserialized. An absent section is an empty table,
    /// so a config type whose fields all have defaults always reads.
    pub fn config<T: DeserializeOwned>(&self) -> anyhow::Result<T> {
        let value = match &self.shared.config {
            Value::Null => Value::Object(Default::default()),
            other => other.clone(),
        };
        serde_json::from_value(value)
            .map_err(|e| anyhow::anyhow!("[ext.{}]: {e}", self.shared.id))
    }

    /// The section as it was written.
    pub fn config_value(&self) -> &Value {
        &self.shared.config
    }

    /// This module's own directory, created before `migrate`. Separate from
    /// Harnix's `.harnix/` ledger and from every other module's.
    pub fn data_dir(&self) -> &Path {
        &self.shared.data_dir
    }

    pub fn credentials(&self) -> &dyn Credentials {
        self.shared.credentials.as_ref()
    }

    pub fn events(&self) -> &Events {
        &self.shared.events
    }

    /// Call any method this module may reach, as `Subject::Extension(id)`. The
    /// same admission point a device goes through, and the same audit. A call
    /// made while answering one is [`CallContext::call`], which says so.
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        let cx = harnix_kernel::CallContext::root(self.subject(), None);
        self.shared.kernel.call_in(cx, method, params).await
    }

    /// Drive sessions: create, send, interrupt, close, tap (concept §3.2).
    pub fn sessions(&self) -> crate::Sessions<'_> {
        crate::Sessions { cx: self }
    }

    /// Build, ensure and stop instances from templates (concept §3.1.1).
    pub fn instances(&self) -> crate::Instances<'_> {
        crate::Instances { cx: self }
    }

    pub(crate) fn subject(&self) -> Subject {
        Subject::extension(self.shared.ext.clone())
    }

    pub(crate) fn kernel(&self) -> &Arc<dyn Kernel> {
        &self.shared.kernel
    }

    pub fn instance(&self, id: &str) -> Option<InstanceHandle> {
        self.shared.kernel.instance(id)
    }

    /// Where this process listens, once it does. Modules start **before** the listener is
    /// bound, so this is `None` throughout `start`: spawn a task on [`ModuleContext::listening`]
    /// to write a run record or a link.
    pub fn listen(&self) -> Option<Listen> {
        self.shared.kernel.listen().and_then(|rx| rx.borrow().clone())
    }

    /// [`ModuleContext::listen`], waiting until the listener is bound. Never returns in a process
    /// that does not serve.
    pub async fn listening(&self) -> Listen {
        if let Some(mut rx) = self.shared.kernel.listen() {
            loop {
                if let Some(listen) = rx.borrow_and_update().clone() {
                    return listen;
                }
                if rx.changed().await.is_err() {
                    break;
                }
            }
        }
        std::future::pending().await
    }

    /// The runtime-application host (see [`crate::AppHost`]): registering an
    /// application's methods, topics and spaces while the process runs.
    /// `None` unless the profile says `[apps] enabled = true`, and `None` for
    /// every module but the one that hosts applications (the `apps`
    /// built-in). What it registers through it is audited as its own.
    pub fn apps(&self) -> Option<crate::AppHost> {
        self.shared
            .kernel
            .apps(&self.shared.id)
            .map(|runtime| crate::AppHost::new(runtime, &self.shared.id))
    }

    /// Keep a value for this module's handlers: `cx.state::<T>()`.
    pub fn provide<T: Send + Sync + 'static>(&self, value: T) {
        self.provide_arc(Arc::new(value));
    }

    pub fn provide_arc<T: Send + Sync + 'static>(&self, value: Arc<T>) {
        write(&self.shared.state).insert(TypeId::of::<T>(), value);
    }

    pub fn state<T: Send + Sync + 'static>(&self) -> Result<Arc<T>, RpcError> {
        read(&self.shared.state)
            .get(&TypeId::of::<T>())
            .cloned()
            .and_then(|v| v.downcast::<T>().ok())
            .ok_or_else(|| {
                RpcError::new(
                    harnix_contract::codes::INTERNAL_ERROR,
                    format!(
                        "module `{}` has not provided a {}",
                        self.shared.id,
                        std::any::type_name::<T>()
                    ),
                )
            })
    }
}

/// What a handler is handed: the call — who is making it — and its module's context.
#[derive(Clone)]
pub struct CallContext {
    call: harnix_ext::Call,
    module: ModuleContext,
}

impl CallContext {
    #[doc(hidden)]
    pub fn new(call: harnix_ext::Call, module: ModuleContext) -> Self {
        Self { call, module }
    }

    pub fn subject(&self) -> &Subject {
        self.call.subject()
    }

    /// The paired device behind this call, when it came from one.
    pub fn device(&self) -> Option<&str> {
        self.call.device()
    }

    /// Call a method as this module while answering this call: decided and audited like
    /// [`ModuleContext::call`], and on the record as made within this call.
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        self.call.call(method, params).await
    }

    pub fn module(&self) -> &ModuleContext {
        &self.module
    }

    pub fn state<T: Send + Sync + 'static>(&self) -> Result<Arc<T>, RpcError> {
        self.module.state::<T>()
    }
}

/// What `migrate` is handed.
pub struct MigrateContext {
    pub id: String,
    pub data_dir: PathBuf,
    pub config: Value,
}

/// What a migration did, for the diagnostics page.
#[derive(Debug, Clone, Default)]
pub struct MigrationReport {
    pub applied: Vec<String>,
}

impl MigrationReport {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn applied(steps: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            applied: steps.into_iter().map(Into::into).collect(),
        }
    }
}
