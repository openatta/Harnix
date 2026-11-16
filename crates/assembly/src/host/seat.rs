//! The kernel's services, as an extension's seat gets them: each a trait of the kernel's seat,
//! implemented over the part of the process that answers it.

use std::any::Any;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use harnix_contract::s1::ExtId;
use harnix_contract::sync::lock;
use harnix_contract::RpcError;
use harnix_ext::Service;
use harnix_host_api::{HostApi, HostServices, SettingsFiles, SettingsTiers};
use harnix_hub::Hub;
use harnix_instance::InstanceRegistry;
use harnix_kernel::seat::{
    BulkGrants, CredentialLookup, DeviceAdmin, InstanceService, InstanceView, Listening, ProcessInfo, Seat,
    ServiceRegistry, TopicPublisher, UiBoard, Unserved,
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
/// engine writes down — the instance's own, kept with it, so what a reader read is kept too.
pub(crate) struct InstanceViews {
    instances: Arc<InstanceRegistry>,
}

impl InstanceViews {
    pub fn new(instances: Arc<InstanceRegistry>) -> Self {
        Self { instances }
    }
}

impl InstanceService for InstanceViews {
    fn view(&self, id: &str) -> Option<InstanceView> {
        let listed = self.instances.facts(id)?;
        let (facts, trust) = (listed.engine, listed.trust);
        let readers = self.instances.readers(id);
        let recordings = readers.as_ref().map(|r| Arc::new(r.recordings.clone()) as Arc<dyn Any + Send + Sync>);
        let transcripts = readers.map(|r| Arc::new(r.transcripts) as Arc<dyn Any + Send + Sync>);
        Some(InstanceView {
            id: id.to_string(),
            active_scenes: facts.active_scenes.clone(),
            model: facts.model.clone().unwrap_or_default(),
            trust,
            recordings,
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
        let path = dir.join(format!("{token}-{}", safe_name(name)));
        lock(&self.grants).insert(token.clone(), (path.clone(), max_bytes));
        Ok((token, path))
    }
}

/// A client's name for what it uploads, as the end of a file name: nothing that leaves the
/// directory or means anything to a shell.
fn safe_name(name: &str) -> String {
    name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).collect()
}

impl BulkGrants for HostUploads {
    fn issue(&self, ext: &ExtId, name: Option<&str>, max_bytes: usize) -> Rpc<(String, PathBuf)> {
        let dir = self.dir.join("ext").join(ext.as_str());
        std::fs::create_dir_all(&dir)
            .map_err(|e| RpcError::new(harnix_contract::codes::INTERNAL_ERROR, format!("{}: {e}", dir.display())))?;
        let token = uuid::Uuid::new_v4().simple().to_string();
        let path = match name {
            Some(name) => dir.join(format!("{token}-{}", safe_name(name))),
            None => dir.join(&token),
        };
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

/// The host's named services: the host components' pipes, and where an instance's engine reads
/// its settings tiers. Each is made for an extension that holds what it takes, and the capability
/// is this side's, by the name asked for: the type an extension asks through is its own to write,
/// and says nothing.
pub(crate) struct HostServicesRegistry {
    grants: Arc<dyn GrantBook>,
    host: OnceLock<Weak<super::dynamic::DynamicHost>>,
    instances: Arc<InstanceRegistry>,
}

impl HostServicesRegistry {
    pub fn new(grants: Arc<dyn GrantBook>, instances: Arc<InstanceRegistry>) -> Self {
        Self { grants, host: OnceLock::new(), instances }
    }

    pub fn set(&self, host: &Arc<super::dynamic::DynamicHost>) {
        let _ = self.host.set(Arc::downgrade(host));
    }
}

impl ServiceRegistry for HostServicesRegistry {
    fn service(&self, ext: &ExtId, name: &str) -> Result<Arc<dyn Any + Send + Sync>, Unserved> {
        let capability = match name {
            HostServices::NAME => HostServices::CAPABILITY,
            SettingsTiers::NAME => SettingsTiers::CAPABILITY,
            _ => return Err(Unserved::Unknown),
        };
        if !self.grants.holds(ext, &capability) {
            return Err(Unserved::Lacks(capability));
        }
        if name == HostServices::NAME {
            // None before the host is made, and in a process that runs no extension host.
            let host = self.host.get().and_then(Weak::upgrade).ok_or(Unserved::Unknown)?;
            let api: Arc<dyn HostApi> = Arc::new(super::dynamic::HostFor::new(host, ext.clone()));
            return Ok(Arc::new(api));
        }
        let files: Arc<dyn SettingsFiles> = Arc::new(TiersOf(self.instances.clone()));
        Ok(Arc::new(files))
    }
}

/// The settings tiers of each instance's engine, under its data root: the engine's own layout.
struct TiersOf(Arc<InstanceRegistry>);

impl SettingsFiles for TiersOf {
    fn global(&self, instance: &str) -> Option<PathBuf> {
        Some(self.0.facts(instance)?.engine.data_root.join("settings.json"))
    }

    fn scene(&self, instance: &str, scene: &str) -> Option<PathBuf> {
        Some(self.0.facts(instance)?.engine.data_root.join("scenes").join(scene).join("settings.json"))
    }
}
