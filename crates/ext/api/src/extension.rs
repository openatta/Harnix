//! An extension, and a product's list of them.

use std::path::{Path, PathBuf};

use harnix_contract::s1::{Capability, ExtId};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::context::{ConfigError, ExtensionContext};
use crate::descriptor::{DescriptorBuilder, ExtensionDescriptor};
use crate::manifest::Manifest;
use crate::registrar::Registrar;

/// One native extension: Rust the product links in and grants capabilities to.
///
/// Order of calls: `manifest` and `register`, once each, in the order the product listed its
/// extensions; then `migrate`; then — once the instances exist — `start`; `stop` on the way
/// out, in reverse. An extension whose registration conflicts, whose configuration does not
/// read, or whose `migrate` or `start` fails is disabled with everything it declared; the rest
/// start. Why is on the diagnostics page.
#[async_trait::async_trait]
pub trait Extension: Send + Sync + 'static {
    /// Its own section of the profile, `[ext.<id>]`; `()` for none. An absent section is the
    /// default. Which keys it refuses is this type's to say: Harnix's own deny unknown ones.
    type Config: DeserializeOwned + Default + Send + Sync + 'static;

    fn manifest(&self) -> Manifest;

    /// Everything it provides: methods, topics, spaces, tools, routes, interface, engine
    /// contributions, instance templates.
    fn register(&self, r: &mut Registrar<'_, Self::Config>);

    /// Bring its own data up to date — its own, never Harnix's. When and what happens on failure
    /// are Harnix's; how is the extension's.
    async fn migrate(&self, _cx: &MigrateContext) -> anyhow::Result<MigrationReport> {
        Ok(MigrationReport::none())
    }

    async fn start(&self, _cx: ExtensionContext) -> anyhow::Result<()> {
        Ok(())
    }

    async fn stop(&self) {}
}

mod sealed {
    pub trait Sealed {}
}

/// [`Extension`] without its configuration type, so a product can hold different ones in one
/// list. Implemented for every `Extension`, and for nothing else.
#[async_trait::async_trait]
pub trait ErasedExtension: sealed::Sealed + Send + Sync + 'static {
    fn manifest(&self) -> Manifest;

    /// Its configuration read from `config` (its section as written, `Null` for none), then its
    /// declarations, under `id`.
    fn register(&self, id: &ExtId, config: &Value) -> Result<ExtensionDescriptor, ConfigError>;

    async fn migrate(&self, cx: &MigrateContext) -> anyhow::Result<MigrationReport>;
    async fn start(&self, cx: ExtensionContext) -> anyhow::Result<()>;
    async fn stop(&self);
}

impl<E: Extension> sealed::Sealed for E {}

#[async_trait::async_trait]
impl<E: Extension> ErasedExtension for E {
    fn manifest(&self) -> Manifest {
        Extension::manifest(self)
    }

    fn register(&self, id: &ExtId, config: &Value) -> Result<ExtensionDescriptor, ConfigError> {
        let typed: E::Config = match config {
            Value::Null => E::Config::default(),
            written => crate::context::read_config(id, written)?,
        };
        let mut builder = DescriptorBuilder::native(id.clone(), Extension::manifest(self));
        let mut registrar = Registrar::new(id, &typed, &mut builder);
        Extension::register(self, &mut registrar);
        Ok(builder.build())
    }

    async fn migrate(&self, cx: &MigrateContext) -> anyhow::Result<MigrationReport> {
        Extension::migrate(self, cx).await
    }

    async fn start(&self, cx: ExtensionContext) -> anyhow::Result<()> {
        Extension::start(self, cx).await
    }

    async fn stop(&self) {
        Extension::stop(self).await
    }
}

/// A product's native extensions, each with what it is granted: the source is the grant list.
#[derive(Default)]
pub struct ExtensionSet {
    entries: Vec<(Box<dyn ErasedExtension>, Vec<Capability>)>,
}

impl ExtensionSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add `e`, granted `grant`. The order added is the order they register, migrate and start.
    pub fn with<E: Extension>(mut self, e: E, grant: impl IntoIterator<Item = Capability>) -> Self {
        self.entries.push((Box::new(e), grant.into_iter().collect()));
        self
    }

    /// Another set's, after these.
    pub fn and(mut self, other: ExtensionSet) -> Self {
        self.entries.extend(other.entries);
        self
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Each extension and what it is granted, in order.
    pub fn into_parts(self) -> Vec<(Box<dyn ErasedExtension>, Vec<Capability>)> {
        self.entries
    }
}

/// What `migrate` is handed: its id, its directory (created already) and its section.
pub struct MigrateContext {
    id: ExtId,
    data_dir: PathBuf,
    config: Value,
}

impl MigrateContext {
    pub fn new(id: ExtId, data_dir: PathBuf, config: Value) -> Self {
        Self { id, data_dir, config }
    }

    pub fn id(&self) -> &ExtId {
        &self.id
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Its section, read as `C`; an absent one reads as an empty table.
    pub fn config<C: DeserializeOwned>(&self) -> Result<C, ConfigError> {
        crate::context::read_config(&self.id, &self.config)
    }
}

/// What a migration did, for the diagnostics page.
#[derive(Debug, Clone, Default)]
pub struct MigrationReport {
    applied: Vec<String>,
}

impl MigrationReport {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn applied(steps: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self { applied: steps.into_iter().map(Into::into).collect() }
    }

    /// The steps it applied, in order.
    pub fn steps(&self) -> &[String] {
        &self.applied
    }
}
