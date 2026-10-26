//! `harnix-app` — a process assembled from modules (design §7).
//!
//! ```no_run
//! fn main() -> anyhow::Result<()> {
//!     harnix_app::App::builder()
//!         .name("helm", env!("CARGO_PKG_VERSION"))
//!         .with_builtins()
//!         // .module(watchlist::Watchlist::default())
//!         .run()
//! }
//! ```
//!
//! The `harnix` binary is exactly that with no modules of its own — the
//! empty application. An application's binary is the same call with its
//! modules added; nothing about the kernel changes between the two.
//!
//! What is fixed and what is chosen: the start order (profile → register →
//! migrate → instances → start → serve), the admission point, the hub, the
//! transport. What an application chooses: its name and version, its
//! modules and their order, and where secrets come from.

mod audit_log;
mod cli;
mod devices;
mod gate;
mod methods;
mod modules;
mod paths;
mod run;

use std::ffi::OsString;
use std::sync::Arc;

use harnix_module::{Credentials, ErasedModule, Module};

pub use devices::DevicesModule;
pub use modules::{MethodStatus, ModuleStatus, Namespace};

/// The version of Harnix this binary was built with.
pub const HARNIX_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct App;

impl App {
    pub fn builder() -> AppBuilder {
        AppBuilder {
            name: "harnix".into(),
            version: HARNIX_VERSION.into(),
            builtins: false,
            modules: Vec::new(),
            credentials: None,
            args: None,
        }
    }
}

pub struct AppBuilder {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) builtins: bool,
    pub(crate) modules: Vec<Box<dyn ErasedModule>>,
    pub(crate) credentials: Option<Arc<dyn Credentials>>,
    pub(crate) args: Option<Vec<OsString>>,
}

impl AppBuilder {
    /// The application's name and version, for `--version` and the
    /// diagnostics page.
    pub fn name(mut self, name: impl Into<String>, version: impl Into<String>) -> Self {
        self.name = name.into();
        self.version = version.into();
        self
    }

    /// Read the profile and flags from the command line. The default; here
    /// so a `main.rs` can say so.
    pub fn profile_from_args(self) -> Self {
        self
    }

    /// Command-line arguments to use instead of the process's own (the
    /// program name first).
    pub fn args<I, T>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString>,
    {
        self.args = Some(args.into_iter().map(Into::into).collect());
        self
    }

    /// Harnix's own modules: the session space's backend, settings, packages,
    /// recordings, devices. Registered before the application's.
    pub fn with_builtins(mut self) -> Self {
        self.builtins = true;
        self
    }

    /// Add a module. The order modules are added is the order they register,
    /// migrate and start. A module written against another
    /// `MODULE_API_VERSION` does not compile.
    pub fn module<M: Module>(mut self, module: M) -> Self {
        harnix_module::assert_api::<M>();
        self.modules.push(Box::new(module));
        self
    }

    /// Where `{ credential = "…" }` references in `[modules.<id>]` are
    /// resolved. The environment by default.
    pub fn credentials(mut self, source: impl Credentials + 'static) -> Self {
        self.credentials = Some(Arc::new(source));
        self
    }

    /// Start and serve until interrupted.
    pub fn run(self) -> anyhow::Result<()> {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?
            .block_on(run::run(self))
    }
}
