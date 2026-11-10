//! An extension's seat in the process: what the extension host hands the extension interface
//! when it binds one extension's declarations, and what the interface's context reaches the
//! process through.
//!
//! The host fills a [`Seat`] with the kernel's services, each a trait here, and hands it to the
//! extension's descriptor through [`Seated`]. Nothing here is the extension interface's own: an
//! extension never sees a seat, only the context made from it, and capabilities are checked by
//! that context against [`GrantBook`] before a service is reached.
//!
//! Values the kernel cannot name — the engine interface's readers, a host component's services —
//! travel as `Arc<dyn Any + Send + Sync>`, each documented with what it holds; the extension
//! interface is the one that knows their types.

use std::any::Any;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use harnix_contract::s1::{ExtId, InstanceTrust};
use serde_json::Value;
use tokio::sync::watch;

use crate::{ClientRegistry, Dispatch, GrantBook, Rpc, TopicChanged};

/// Where this process accepts connections, once it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listening {
    /// Every address it accepts on, with the port the system chose when asked for any.
    pub addrs: Vec<SocketAddr>,
    /// The one to give a person or a tool: `http(s)://<first addr>/`. It carries no token.
    pub url: String,
}

/// Publishing whole values on a topic, and saying a computed one may have changed.
#[async_trait::async_trait]
pub trait TopicPublisher: TopicChanged {
    async fn publish(&self, topic: &str, params: Value, value: Value);
}

/// What an extension may know of one instance: never its link.
#[derive(Clone)]
pub struct InstanceView {
    pub id: String,
    pub active_scenes: Vec<String>,
    pub model: String,
    pub trust: InstanceTrust,
    /// Where its engine looks for skills.
    pub skills_dir: PathBuf,
    /// Where its engine's global and scene settings tiers are written.
    pub settings_root: PathBuf,
    /// An `Arc<dyn harnix_engine_api::readers::Recordings>`, when the instance keeps recordings.
    pub recordings: Option<Arc<dyn Any + Send + Sync>>,
    /// An `Arc<dyn harnix_engine_api::readers::Transcripts>`, when its transcripts can be read here.
    pub transcripts: Option<Arc<dyn Any + Send + Sync>>,
}

/// The instances, as facts.
pub trait InstanceService: Send + Sync {
    fn view(&self, id: &str) -> Option<InstanceView>;
}

/// This process: versions, uptime, memory, threads, who is connected.
pub trait ProcessInfo: Send + Sync {
    fn process(&self) -> Value;
}

/// One-shot upload grants: a token the upload route takes once, and where the bytes land.
pub trait BulkGrants: Send + Sync {
    /// A grant for `ext`, for at most `max_bytes`: its token and the file the bytes will be in.
    fn issue(&self, ext: &ExtId, max_bytes: usize) -> Rpc<(String, PathBuf)>;
}

/// Interface entries an extension publishes while it runs, besides what it declared.
pub trait UiBoard: Send + Sync {
    /// Replace what `ext` publishes with `entries`.
    fn publish(&self, ext: &ExtId, entries: Vec<Value>);
}

/// Paired devices: listing, pairing, revoking. Who may is decided before this is reached.
pub trait DeviceAdmin: Send + Sync {
    fn list(&self) -> Value;
    /// Start pairing a device called `label`: its handle and the code, shown once.
    fn pair_begin(&self, label: &str) -> Value;
    /// Finish pairing with the code the device was shown, and its public key.
    fn pair_complete(&self, code: &str, public_key: &str) -> Rpc<Value>;
    /// Forget a device and end every channel it holds. Whether it was paired.
    fn revoke(&self, device: &str) -> bool;
}

/// Where a profile's secret references are resolved.
pub trait CredentialLookup: Send + Sync {
    fn secret(&self, reference: &str) -> Result<String, String>;
}

/// Named services beyond the kernel's own, by the extension asking: each an
/// `Arc<Handle>` of the type the service is declared with, behind `Any`.
pub trait ServiceRegistry: Send + Sync {
    fn service(&self, ext: &ExtId, name: &str) -> Option<Arc<dyn Any + Send + Sync>>;
}

/// Everything one extension's context reaches the process through.
#[derive(Clone)]
pub struct Seat {
    pub id: ExtId,
    /// Its own section of the profile, as written: `Null` when there is none.
    pub config: Value,
    /// Its own directory, created before it migrates.
    pub data_dir: PathBuf,
    pub dispatch: Arc<dyn Dispatch>,
    pub grants: Arc<dyn GrantBook>,
    pub clients: Arc<dyn ClientRegistry>,
    pub topics: Arc<dyn TopicPublisher>,
    pub instances: Arc<dyn InstanceService>,
    pub process: Arc<dyn ProcessInfo>,
    pub bulk: Arc<dyn BulkGrants>,
    pub ui: Arc<dyn UiBoard>,
    pub devices: Arc<dyn DeviceAdmin>,
    pub credentials: Arc<dyn CredentialLookup>,
    pub services: Arc<dyn ServiceRegistry>,
    /// `None` in a process that never listens.
    pub listen: Option<watch::Receiver<Option<Listening>>>,
}

/// What the extension interface implements for an extension's declarations: take the seat, after
/// which its rows answer and its context exists. A second seat is ignored.
pub trait Seated {
    fn seat(&self, seat: Seat);
}
