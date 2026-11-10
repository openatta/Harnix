//! `harnix-app` — a process assembled from modules (concept §3.1).
//!
//! ```no_run
//! fn main() -> anyhow::Result<()> {
//!     harnix_assembly::App::builder()
//!         .name("helm", env!("CARGO_PKG_VERSION"))
//!         .with_builtins()
//!         // .module(watchlist::Watchlist::default())
//!         .run()
//! }
//! ```
//!
//! The `harnix` binary is exactly that with no modules of its own — the
//! Harnix app. An application's binary is the same call with its
//! modules added; nothing about the kernel changes between the two.
//!
//! What is fixed and what is chosen: the start order (profile → register →
//! migrate → instances → start → serve), the admission point, the hub, the
//! transport. What an application chooses: its name and version, its
//! modules and their order, and where secrets come from.

mod apps;
mod audit_log;
mod cli;
mod devices;
mod gate;
mod kernel_rows;
mod modules;
mod paths;
mod process;
mod run;
mod state_lock;

use std::ffi::OsString;
use std::sync::Arc;

use harnix_module::{Capability, Credentials, ErasedModule, Module};

pub use devices::DevicesModule;
pub use modules::{MethodStatus, ModuleStatus, Namespace};

/// The version of Harnix this binary was built with.
pub const HARNIX_VERSION: &str = env!("CARGO_PKG_VERSION");

/// How long the shutdown after SIGINT, SIGTERM or a closed stdin (`--exit-with-stdin`) may take:
/// every module's `stop`, then the instances. Past it the process logs that and exits anyway, as
/// it does on a second signal (ipdrun-02).
pub const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(10);

pub struct App;

impl App {
    pub fn builder() -> AppBuilder {
        AppBuilder {
            name: "harnix".into(),
            version: HARNIX_VERSION.into(),
            builtins: false,
            modules: Vec::new(),
            granted: Vec::new(),
            credentials: None,
            args: None,
            app_hosts: Vec::new(),
            store_keys: None,
            app_idle: None,
            child_entries: Vec::new(),
        }
    }
}

/// The registration step on its own, with nothing served: what `tests/grants.rs` holds the
/// grants, the capabilities and the name rules to. Not for a product.
#[doc(hidden)]
pub mod testing {
    use std::sync::Arc;

    use harnix_module::{Capability, ErasedModule, Module};

    pub use harnix_kernel::GrantTable;
    pub use harnix_router::Router;

    /// The modules registered, and the router over the kernel's rows and theirs. Nothing behind
    /// the kernel's rows is up: a call the router lets through to one answers `INTERNAL_ERROR`, and
    /// one to a module's method `METHOD_NOT_FOUND` (the module never started) — either way not
    /// `REFUSED`, which is what these tests tell apart.
    pub struct Registered {
        pub router: Arc<Router>,
        pub grants: Arc<GrantTable>,
        pub modules: Vec<crate::ModuleStatus>,
    }

    /// Modules to register, each with what it is granted.
    #[derive(Default)]
    pub struct Modules(Vec<(Box<dyn ErasedModule>, Vec<Capability>)>);

    impl Modules {
        pub fn with<M: Module>(mut self, module: M, granted: impl IntoIterator<Item = Capability>) -> Self {
            harnix_module::assert_api::<M>();
            self.0.push((Box::new(module), granted.into_iter().collect()));
            self
        }
    }

    /// Register Harnix's built-ins (when `builtins`) and `modules` under a default profile, with
    /// state in `state`. An error is the start refused.
    pub fn register(builtins: bool, modules: Modules, state: &std::path::Path) -> anyhow::Result<Registered> {
        let (table, grants, modules) = crate::run::register_only(builtins, modules.0, state)?;
        let router = Arc::new(Router::new(table, grants.clone(), Arc::new(harnix_authz::Audit::default())));
        Ok(Registered { router, grants, modules })
    }
}

/// What a subcommand this binary is re-executed as runs instead of the product: its arguments,
/// those after its name.
pub type ChildEntry = fn(Vec<OsString>) -> anyhow::Result<()>;

pub struct AppBuilder {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) builtins: bool,
    pub(crate) modules: Vec<Box<dyn ErasedModule>>,
    /// What each of `modules` was granted, in the same order.
    pub(crate) granted: Vec<Vec<Capability>>,
    pub(crate) credentials: Option<Arc<dyn Credentials>>,
    pub(crate) args: Option<Vec<OsString>>,
    pub(crate) app_hosts: Vec<String>,
    pub(crate) store_keys: Option<Vec<String>>,
    pub(crate) app_idle: Option<std::time::Duration>,
    pub(crate) child_entries: Vec<(String, ChildEntry)>,
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
        self.granted.push(Vec::new());
        self
    }

    /// Grant the module added last these capabilities: `.module(Fleet).grant([caps::ENGINE_INSPECT])`.
    ///
    /// Checked when the process starts, which refuses to start, naming the module and the
    /// capability, when a module is not granted all it `requires`, or is granted what it neither
    /// requires nor lists as `optional`, or what nobody defines for native extensions. Without a
    /// module before it, this panics: the assembly is the program, and that is a mistake in it.
    pub fn grant(mut self, caps: impl IntoIterator<Item = Capability>) -> Self {
        let last = self.granted.last_mut().expect("`grant` follows the `module` it grants to");
        last.extend(caps);
        self
    }

    /// Where `{ credential = "…" }` references in `[modules.<id>]` are
    /// resolved. The environment by default.
    pub fn credentials(mut self, source: impl Credentials + 'static) -> Self {
        self.credentials = Some(Arc::new(source));
        self
    }

    /// Hand the runtime-application host to this module as well as to the `apps` built-in.
    ///
    /// For a test application whose module fakes an application host (`harnix-testapp`'s
    /// `fakehost`); a product has no reason to. Without `[apps] enabled` there is no host to hand.
    #[doc(hidden)]
    pub fn app_host(mut self, module: impl Into<String>) -> Self {
        self.app_hosts.push(module.into());
        self
    }

    /// Verify store-countersigned applications with these keys (hex ed25519) instead of the
    /// store's own, built in.
    ///
    /// For a test application that installs from a mock store (`harnix-testapp`); a product has no
    /// reason to, and a profile cannot. Without the `apps` feature there is nothing to verify.
    #[doc(hidden)]
    pub fn store_keys<I, T>(mut self, keys: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<String>,
    {
        self.store_keys = Some(keys.into_iter().map(Into::into).collect());
        self
    }

    /// Hibernate every on-demand runtime application's backend after this much idle, in place of
    /// its manifest's `idle_secs` (never under a minute).
    ///
    /// For a test application that needs to see one hibernate (`harnix-testapp`); a product has no
    /// reason to, and a profile cannot. Without the `apps` feature there is nothing to idle.
    #[doc(hidden)]
    pub fn app_idle(mut self, idle: std::time::Duration) -> Self {
        self.app_idle = Some(idle);
        self
    }

    /// Run `entry` instead of the product when this binary is started as `<program> <name> …`.
    ///
    /// How a process the product starts is the product's own build: an extension process is
    /// `<product> ext-proc`. The subcommand is dispatched before anything else — no profile is
    /// read, no state lock taken, nothing assembled, no runtime built — and `entry` gets the
    /// arguments after the name. A name the command line already has (`daemon`, `ui`) panics.
    pub fn child_entry(mut self, name: impl Into<String>, entry: ChildEntry) -> Self {
        let name = name.into();
        assert!(!cli::is_subcommand(&name), "`{name}` is a subcommand of the product itself");
        self.child_entries.retain(|(n, _)| *n != name);
        self.child_entries.push((name, entry));
        self
    }

    /// Start and serve until interrupted.
    pub fn run(self) -> anyhow::Result<()> {
        if let Some(done) = cli::child_entry(&self) {
            return done;
        }
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?
            .block_on(run::run(self))
    }
}
