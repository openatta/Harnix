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

use harnix_contract::{RpcError, Subject};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::credentials::Credentials;
use crate::events::{Events, TopicDecl};

/// The process side of a module's context: implemented by the application,
/// not by modules.
#[async_trait::async_trait]
pub trait Kernel: Send + Sync {
    /// One call through the admission point, as `subject`.
    async fn call(&self, subject: &Subject, method: &str, params: Value) -> Result<Value, RpcError>;
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
    pub history: Option<Arc<dyn history::store::HistoryStore>>,
    pub active_scenes: Vec<String>,
    pub model: String,
}

struct Shared {
    id: String,
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

    /// `[modules.<id>]`, deserialized. An absent section is an empty table,
    /// so a config type whose fields all have defaults always reads.
    pub fn config<T: DeserializeOwned>(&self) -> anyhow::Result<T> {
        let value = match &self.shared.config {
            Value::Null => Value::Object(Default::default()),
            other => other.clone(),
        };
        serde_json::from_value(value)
            .map_err(|e| anyhow::anyhow!("[modules.{}]: {e}", self.shared.id))
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

    /// Call any method this module may reach, as `Subject::Module(id)`. The
    /// same admission point a device goes through, and the same audit.
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        self.shared.kernel.call(&self.subject(), method, params).await
    }

    /// Drive sessions: create, send, interrupt, close, tap (design §6.1).
    pub fn sessions(&self) -> crate::Sessions<'_> {
        crate::Sessions { cx: self }
    }

    /// Build, ensure and stop instances from templates (design §5.2).
    pub fn instances(&self) -> crate::Instances<'_> {
        crate::Instances { cx: self }
    }

    pub(crate) fn subject(&self) -> Subject {
        Subject::Module {
            id: self.shared.id.clone(),
        }
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

    /// Keep a value for this module's handlers: `cx.state::<T>()`.
    pub fn provide<T: Send + Sync + 'static>(&self, value: T) {
        self.provide_arc(Arc::new(value));
    }

    pub fn provide_arc<T: Send + Sync + 'static>(&self, value: Arc<T>) {
        self.shared
            .state
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(TypeId::of::<T>(), value);
    }

    pub fn state<T: Send + Sync + 'static>(&self) -> Result<Arc<T>, RpcError> {
        self.shared
            .state
            .read()
            .unwrap_or_else(|e| e.into_inner())
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

/// What a handler is handed: who is calling, and its module's context.
#[derive(Clone)]
pub struct CallContext {
    subject: Subject,
    module: ModuleContext,
}

impl CallContext {
    #[doc(hidden)]
    pub fn new(subject: Subject, module: ModuleContext) -> Self {
        Self { subject, module }
    }

    pub fn subject(&self) -> &Subject {
        &self.subject
    }

    /// The paired device behind this call, when it came from one.
    pub fn device(&self) -> Option<&str> {
        match &self.subject {
            Subject::Device { id } => Some(id),
            _ => None,
        }
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
