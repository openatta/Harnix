//! `harnix-assembly` — a process assembled from extensions.
//!
//! ```no_run
//! fn main() -> anyhow::Result<()> {
//!     harnix_assembly::App::builder()
//!         .name("helm", env!("CARGO_PKG_VERSION"))
//!         .with_builtins()
//!         // .extension(watchlist::Watchlist::default()).grant([caps::ENGINE_INSPECT])
//!         .run()
//! }
//! ```
//!
//! The `harnix` binary is exactly that with no extensions of its own — the
//! Harnix app. A product's binary is the same call with its extensions
//! added; nothing about the kernel changes between the two.
//!
//! What is fixed and what is chosen: the start order (profile → register →
//! migrate → instances → start → serve), the admission point, the hub, the
//! transport, the extension host. What a product chooses: its name and version, its
//! extensions, their order and what each is granted, and where secrets come from.

mod aliases;
mod apps;
mod audit_log;
mod cli;
mod pairing;
mod gate;
mod host;
mod kernel_rows;
mod paths;
mod process;
mod profile;
mod run;
mod state_lock;

use std::ffi::OsString;
use std::sync::Arc;

use harnix_contract::s1::Capability;
use harnix_ext::{Credentials, Extension, ExtensionSet};
use harnix_module::Module;

use host::{Added, Entry};

pub use host::{ExtensionStatus, MethodStatus, Namespace};

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
            entries: Vec::new(),
            credentials: None,
            args: None,
            app_hosts: Vec::new(),
            store_keys: None,
            app_idle: None,
            child_entries: Vec::new(),
        }
    }
}

/// The process without its listener, for tests: the registration step alone
/// ([`testing::register`]), or every step up to serving ([`testing::Kernel`]). Not for a product.
#[doc(hidden)]
pub mod testing {
    use std::sync::Arc;

    use harnix_contract::s1::Capability;
    use harnix_contract::{RpcError, Subject};
    use harnix_ext::{Extension, ExtensionContext};
    use harnix_kernel::{CallContext, Dispatch};
    use harnix_module::Module;
    use serde_json::Value;

    pub use harnix_kernel::GrantTable;
    pub use harnix_router::Router;

    use crate::host::{Added, Entry, Namespace};

    /// The extensions registered, and the router over the kernel's rows and theirs. Nothing behind
    /// the kernel's rows is up: a call the router lets through to one answers `INTERNAL_ERROR`, and
    /// one to an extension's method `METHOD_NOT_FOUND` (it never started) — either way not
    /// `REFUSED`, which is what these tests tell apart.
    pub struct Registered {
        pub router: Arc<Router>,
        pub grants: Arc<GrantTable>,
        pub extensions: Vec<crate::ExtensionStatus>,
    }

    /// Extensions and modules to register, each with what it is granted.
    #[derive(Default)]
    pub struct Modules(Vec<Entry>);

    impl Modules {
        pub fn with<M: Module>(mut self, module: M, granted: impl IntoIterator<Item = Capability>) -> Self {
            harnix_module::assert_api::<M>();
            let added = Added::Module { module: Box::new(module), namespace: Namespace::App };
            self.0.push(Entry { added, granted: granted.into_iter().collect() });
            self
        }

        pub fn extension<E: Extension>(mut self, e: E, granted: impl IntoIterator<Item = Capability>) -> Self {
            self.0.push(Entry { added: Added::Extension(Box::new(e)), granted: granted.into_iter().collect() });
            self
        }

        /// Every extension of `set`, each granted what the set says.
        pub fn extensions(mut self, set: harnix_ext::ExtensionSet) -> Self {
            for (e, granted) in set.into_parts() {
                self.0.push(Entry { added: Added::Extension(e), granted });
            }
            self
        }
    }

    /// Register Harnix's built-ins (when `builtins`) and `modules` under a default profile, with
    /// state in `state`. An error is the start refused.
    pub fn register(builtins: bool, modules: Modules, state: &std::path::Path) -> anyhow::Result<Registered> {
        let (table, grants, extensions) =
            crate::run::register_only(builtins, modules.0, &crate::profile::Profile::default(), state)?;
        let router = Arc::new(Router::new(table, grants.clone(), Arc::new(harnix_authz::Audit::default())));
        Ok(Registered { router, grants, extensions })
    }

    /// The route table a product would serve, without starting anything: every row, in the shape
    /// the method-table snapshot pins.
    pub fn describe(app: crate::AppBuilder) -> anyhow::Result<Value> {
        let dir = tempfile_dir()?;
        let (table, grants, _) =
            crate::run::register_only(app.builtins, app.entries, &crate::profile::Profile::default(), &dir)?;
        let snapshot = Router::new(table, grants, Arc::new(harnix_authz::Audit::default())).snapshot();
        let _ = std::fs::remove_dir_all(&dir);
        Ok(snapshot)
    }

    fn tempfile_dir() -> anyhow::Result<std::path::PathBuf> {
        let dir = std::env::temp_dir().join(format!("harnix-describe-{}-{}", std::process::id(), uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// A process started — its extensions registered, migrated and started, its instances built,
    /// its hub up — and not listening: the real router and hub, and an engine that replays and so
    /// never calls a model. Its state is in `dir`, which outlives it.
    pub struct Kernel {
        started: crate::run::Started,
        _basis: crate::run::Basis,
    }

    impl Kernel {
        pub async fn start(app: crate::AppBuilder, dir: &std::path::Path) -> anyhow::Result<Self> {
            let replay = dir.join("recordings");
            std::fs::create_dir_all(&replay)?;
            let args: Vec<std::ffi::OsString> = vec![
                "harnix-test".into(),
                "--headless".into(),
                "--agix-dir".into(),
                dir.join("engine").into(),
                "--data-dir".into(),
                dir.join("data").into(),
                "--replay-dir".into(),
                replay.into(),
            ];
            let cli = crate::cli::parse(&app.name, &app.version, Some(args));
            let basis = crate::run::Basis::read(&cli)?;
            let started = crate::run::start_process(app, &cli, &basis).await?;
            Ok(Self { started, _basis: basis })
        }

        /// One call through the router, as `subject`.
        pub async fn call(&self, subject: Subject, method: &str, params: Value) -> Result<Value, RpcError> {
            self.started.admission.router.call(CallContext::root(subject, None), method, params).await
        }

        /// The context of an extension it started.
        pub fn context(&self, id: &str) -> Option<ExtensionContext> {
            self.started
                .records
                .iter()
                .find(|r| r.manifest.id() == id && r.is_enabled())
                .and_then(|r| r.descriptor.as_ref())
                .and_then(|d| d.context())
        }

        /// What the diagnostics say about each extension.
        pub fn extensions(&self) -> Vec<crate::ExtensionStatus> {
            self.started.records.iter().map(|r| r.status.clone()).collect()
        }

        /// Stop it: the extensions in reverse, then the hub and the instances.
        pub async fn shutdown(self) {
            crate::host::lifecycle::stop(&self.started.records).await;
            self.started.hub.shutdown().await;
            self.started.instances.shutdown().await;
        }
    }
}

/// What a subcommand this binary is re-executed as runs instead of the product: its arguments,
/// those after its name.
pub type ChildEntry = fn(Vec<OsString>) -> anyhow::Result<()>;

pub struct AppBuilder {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) builtins: bool,
    /// The product's extensions and modules, in order, each with what it is granted.
    pub(crate) entries: Vec<Entry>,
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

    /// Add a native extension. The order extensions are added is the order they register,
    /// migrate and start. What it is granted follows: `.extension(e).grant([..])`, or
    /// `.ungranted()` for none.
    pub fn extension<E: Extension>(mut self, e: E) -> ExtensionSlot {
        self.entries.push(Entry { added: Added::Extension(Box::new(e)), granted: Vec::new() });
        ExtensionSlot { builder: self }
    }

    /// Add every extension of `set`, each granted what the set says, in its order.
    pub fn extensions(mut self, set: ExtensionSet) -> Self {
        for (e, granted) in set.into_parts() {
            self.entries.push(Entry { added: Added::Extension(e), granted });
        }
        self
    }

    /// Add a module, read as an extension through its shim, its names as they were
    /// (`app.<id>.*`). A module written against another `MODULE_API_VERSION` does not compile.
    pub fn module<M: Module>(mut self, module: M) -> Self {
        harnix_module::assert_api::<M>();
        self.entries.push(Entry {
            added: Added::Module { module: Box::new(module), namespace: host::Namespace::App },
            granted: Vec::new(),
        });
        self
    }

    /// Grant the module added last these capabilities: `.module(Fleet).grant([caps::ENGINE_INSPECT])`.
    ///
    /// Checked when the process starts, which refuses to start, naming the extension and the
    /// capability, when one is not granted all it `requires`, or is granted what it neither
    /// requires nor lists as `optional`, or what nobody defines for native extensions. Without a
    /// module before it, this panics: the assembly is the program, and that is a mistake in it.
    pub fn grant(mut self, caps: impl IntoIterator<Item = Capability>) -> Self {
        let last = self.entries.last_mut().expect("`grant` follows the `module` it grants to");
        last.granted.extend(caps);
        self
    }

    /// Where `{ credential = "…" }` references in `[ext.<id>]` are
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

/// The extension just added, waiting to be told what it is granted.
#[must_use = "an extension is added once it is granted, or said to be ungranted"]
pub struct ExtensionSlot {
    builder: AppBuilder,
}

impl ExtensionSlot {
    /// Grant it these capabilities: checked at start against what its manifest requires and lists
    /// as optional.
    pub fn grant(mut self, caps: impl IntoIterator<Item = Capability>) -> AppBuilder {
        if let Some(last) = self.builder.entries.last_mut() {
            last.granted.extend(caps);
        }
        self.builder
    }

    /// Grant it nothing: it reaches what any extension reaches.
    pub fn ungranted(self) -> AppBuilder {
        self.builder
    }
}
