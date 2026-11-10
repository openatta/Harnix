//! The kernel's services, as an extension's seat gets them: each a trait of the kernel's seat,
//! implemented over the part of the process that answers it.

use std::any::Any;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use harnix_contract::s1::ExtId;
use harnix_contract::sync::lock;
use harnix_contract::RpcError;
use harnix_engine::readers::{RecordingsDir, TranscriptStore};
use harnix_engine_api::readers::{Recordings, Transcripts};
use harnix_ext::Service;
use harnix_host_api::{HostApi, HostServices};
use harnix_hub::Hub;
use harnix_instance::InstanceRegistry;
use harnix_kernel::seat::{
    BulkGrants, CredentialLookup, DeviceAdmin, InstanceService, InstanceView, Listening, ProcessInfo, Seat,
    ServiceRegistry, TopicPublisher, UiBoard,
};
use harnix_kernel::{ClientRegistry, Dispatch, GrantBook, Rpc, TopicChanged};
use serde_json::Value;
use tokio::sync::watch;

use crate::kernel_rows::UiModules;

/// What every extension's seat is made of: one of each service, shared.
pub(crate) struct Seats {
    pub dispatch: Arc<dyn Dispatch>,
    pub grants: Arc<dyn GrantBook>,
    pub clients: Arc<dyn ClientRegistry>,
    pub topics: Arc<dyn TopicPublisher>,
    pub instances: Arc<dyn InstanceService>,
    pub process: Arc<dyn ProcessInfo>,
    pub bulk: Arc<HostUploads>,
    pub ui: Arc<dyn UiBoard>,
    pub devices: Arc<dyn DeviceAdmin>,
    pub credentials: Arc<dyn CredentialLookup>,
    pub services: Arc<HostServicesRegistry>,
    pub listen: watch::Receiver<Option<Listening>>,
    pub projects_dir: PathBuf,
}

impl Seats {
    pub fn seat(&self, id: &ExtId, config: &Value, data_dir: &Path) -> Seat {
        Seat {
            id: id.clone(),
            config: config.clone(),
            data_dir: data_dir.to_path_buf(),
            projects_dir: self.projects_dir.clone(),
            dispatch: self.dispatch.clone(),
            grants: self.grants.clone(),
            clients: self.clients.clone(),
            topics: self.topics.clone(),
            instances: self.instances.clone(),
            process: self.process.clone(),
            bulk: self.bulk.clone(),
            ui: self.ui.clone(),
            devices: self.devices.clone(),
            credentials: self.credentials.clone(),
            services: self.services.clone(),
            listen: Some(self.listen.clone()),
        }
    }
}

/// Topics, published and recomputed in the hub.
pub(crate) struct HubTopics(pub Arc<Hub>);

impl TopicChanged for HubTopics {
    fn changed(&self, topic: &str, params: Value) {
        TopicChanged::changed(&*self.0, topic, params)
    }
}

#[async_trait::async_trait]
impl TopicPublisher for HubTopics {
    async fn publish(&self, topic: &str, params: Value, value: Value) {
        self.0.publish(topic, params, value).await
    }
}

/// Instances as facts: what the instance registry knows of each, and the readers of what its
/// engine writes down.
pub(crate) struct InstanceViews {
    instances: Arc<InstanceRegistry>,
    /// One recordings reader per root, kept: what a reader folded (the usage figures) is kept
    /// until a recording changes, and a reader made for every view would keep nothing.
    readers: Mutex<HashMap<PathBuf, Arc<RecordingsDir>>>,
}

impl InstanceViews {
    pub fn new(instances: Arc<InstanceRegistry>) -> Self {
        Self { instances, readers: Mutex::new(HashMap::new()) }
    }
}

impl InstanceService for InstanceViews {
    fn view(&self, id: &str) -> Option<InstanceView> {
        let listed = self.instances.facts(id)?;
        let (facts, trust) = (listed.engine, listed.trust);
        let root = facts.recordings_root.clone().unwrap_or_else(|| facts.data_root.join("recordings"));
        let recordings: Arc<dyn Recordings> =
            lock(&self.readers).entry(root.clone()).or_insert_with(|| Arc::new(RecordingsDir::new(root))).clone();
        let transcripts = facts
            .history
            .clone()
            .map(|store| Arc::new(Arc::new(TranscriptStore::new(store)) as Arc<dyn Transcripts>) as Arc<dyn Any + Send + Sync>);
        Some(InstanceView {
            id: id.to_string(),
            active_scenes: facts.active_scenes.clone(),
            model: facts.model.clone().unwrap_or_default(),
            trust,
            skills_dir: facts.data_root.join("skills"),
            settings_root: facts.data_root.clone(),
            recordings: Some(Arc::new(recordings)),
            transcripts,
        })
    }
}

/// This process's figures, once they are assembled.
pub(crate) struct ProcessView(pub Arc<OnceLock<Arc<crate::process::Process>>>);

impl ProcessInfo for ProcessView {
    fn process(&self) -> Value {
        self.0.get().map(|p| p.view()).unwrap_or(Value::Null)
    }
}

/// The extensions' upload grants: one use each, claimed by the upload route.
pub(crate) struct HostUploads {
    dir: PathBuf,
    grants: Mutex<HashMap<String, (PathBuf, usize)>>,
}

impl HostUploads {
    /// Grants landing under `dir`, made when the first one is issued.
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, grants: Mutex::new(HashMap::new()) }
    }

    pub fn claim(&self, token: &str) -> Option<(PathBuf, usize)> {
        lock(&self.grants).remove(token)
    }

    /// A grant a device asked for (`harnix.bulk.upload`), as `<token>-<name>`, the name made safe.
    pub fn issue_named(&self, name: &str, max_bytes: usize) -> Rpc<(String, PathBuf)> {
        let dir = self.dir.clone();
        std::fs::create_dir_all(&dir)
            .map_err(|e| RpcError::new(harnix_contract::codes::INTERNAL_ERROR, format!("{}: {e}", dir.display())))?;
        let token = format!("u-{}", uuid::Uuid::new_v4().simple());
        let safe: String =
            name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).collect();
        let path = dir.join(format!("{token}-{safe}"));
        lock(&self.grants).insert(token.clone(), (path.clone(), max_bytes));
        Ok((token, path))
    }
}

impl BulkGrants for HostUploads {
    fn issue(&self, ext: &ExtId, max_bytes: usize) -> Rpc<(String, PathBuf)> {
        let dir = self.dir.join("ext").join(ext.as_str());
        std::fs::create_dir_all(&dir)
            .map_err(|e| RpcError::new(harnix_contract::codes::INTERNAL_ERROR, format!("{}: {e}", dir.display())))?;
        let token = uuid::Uuid::new_v4().simple().to_string();
        let path = dir.join(&token);
        lock(&self.grants).insert(token.clone(), (path.clone(), max_bytes));
        Ok((token, path))
    }
}

/// What the upload route claims from.
#[async_trait::async_trait]
impl harnix_transport::BulkStore for HostUploads {
    async fn claim(&self, token: &str) -> Option<(PathBuf, usize)> {
        HostUploads::claim(self, token)
    }
}

/// The interface entries an extension publishes, into what `hello` tells the page.
pub(crate) struct UiEntries(pub Arc<UiModules>);

impl UiBoard for UiEntries {
    fn publish(&self, ext: &ExtId, entries: Vec<Value>) {
        self.0.set_extension(ext.as_str(), entries);
    }
}

/// Where the product's credentials are looked up.
pub(crate) struct Secrets(pub Arc<dyn harnix_ext::Credentials>);

impl CredentialLookup for Secrets {
    fn secret(&self, reference: &str) -> Result<String, String> {
        self.0.get(reference).map(|s| s.expose().to_string())
    }
}

/// The host's named services: one, the host components' pipes, made for whoever asks.
#[derive(Default)]
pub(crate) struct HostServicesRegistry {
    host: OnceLock<Weak<super::dynamic::DynamicHost>>,
}

impl HostServicesRegistry {
    pub fn set(&self, host: &Arc<super::dynamic::DynamicHost>) {
        let _ = self.host.set(Arc::downgrade(host));
    }
}

impl ServiceRegistry for HostServicesRegistry {
    fn service(&self, ext: &ExtId, name: &str) -> Option<Arc<dyn Any + Send + Sync>> {
        if name != HostServices::NAME {
            return None;
        }
        let host = self.host.get()?.upgrade()?;
        let api: Arc<dyn HostApi> = Arc::new(super::dynamic::HostFor::new(host, ext.clone()));
        Some(Arc::new(api))
    }
}
