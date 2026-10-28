//! The command line, and the profile it adjusts.

use std::ffi::OsString;
use std::net::IpAddr;
use std::path::PathBuf;

use clap::{CommandFactory, FromArgMatches, Parser};
use harnix_instance::Profile;

#[derive(clap::Subcommand, Debug)]
pub(crate) enum Command {
    /// Work with the interface this binary carries.
    #[command(subcommand)]
    Ui(UiCommand),
    /// Serve one Agix engine on a unix socket, as a daemon instance.
    ///
    /// What a `spawn` instance runs: this binary links Agix already, so the
    /// daemon is the same build and speaks the same protocol. Not for
    /// starting by hand, though nothing stops it.
    #[command(hide = true)]
    Daemon(DaemonCommand),
}

#[derive(clap::Args, Debug)]
pub(crate) struct DaemonCommand {
    #[arg(long)]
    pub data_root: PathBuf,
    #[arg(long)]
    pub socket: PathBuf,
    #[arg(long, default_value = "coding")]
    pub scene: String,
    #[arg(long, value_delimiter = ',')]
    pub scenes: Vec<String>,
    #[arg(long, default_value = "claude-sonnet-4-6")]
    pub model: String,
    #[arg(long, default_value = "2000")]
    pub max_tokens: u32,
    #[arg(long, default_value = "32")]
    pub session_cap: usize,
    #[arg(long, default_value = "3600")]
    pub session_idle_timeout: u64,
    #[arg(long, default_value = "300")]
    pub permission_prompt_timeout: u64,
    /// Exit when stdin closes: how a spawned daemon follows its parent out.
    #[arg(long)]
    pub exit_with_stdin: bool,
    /// The instance this daemon carries: hold its lock on the data root, so a previous daemon of
    /// the same instance has finished before this one builds its engine.
    #[arg(long)]
    pub instance: Option<String>,
    /// How long to wait for that lock, in seconds.
    #[arg(long, default_value = "10")]
    pub lock_wait: u64,
}

impl DaemonCommand {
    pub fn args(&self) -> harnix_instance::DaemonArgs {
        harnix_instance::DaemonArgs {
            data_root: self.data_root.clone(),
            socket: self.socket.clone(),
            engine: harnix_instance::EngineProfile {
                scene: self.scene.clone(),
                scenes: self.scenes.clone(),
                model: self.model.clone(),
                max_tokens: self.max_tokens,
                session_cap: self.session_cap,
                session_idle_timeout_secs: self.session_idle_timeout,
                permission_prompt_timeout_secs: self.permission_prompt_timeout,
            },
            exit_with_stdin: self.exit_with_stdin,
            instance: self.instance.clone(),
            lock_wait: std::time::Duration::from_secs(self.lock_wait),
        }
    }
}

#[derive(clap::Subcommand, Debug)]
pub(crate) enum UiCommand {
    /// Write the interface out, for something else to serve.
    ///
    /// For the deployment where a CDN or a proxy has its own static root.
    /// The binary still carries it, so there is one artifact to install and
    /// the files are available when they are wanted — rather than a second
    /// download to keep in step with this one.
    Export {
        /// Where to write it.
        dir: PathBuf,
    },
}

#[derive(Parser, Debug)]
#[command(about = "Agix, assembled into a product")]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// A profile: which scenes, which providers, which modules' settings,
    /// which interface, which transport topology. Flags below override what
    /// it says.
    #[arg(long)]
    pub profile: Option<PathBuf>,

    #[arg(long)]
    pub port: Option<u16>,

    /// Bind address. A non-loopback bind needs TLS and paired devices.
    #[arg(long)]
    pub host: Option<IpAddr>,

    /// Default scene for new sessions, and the state root under
    /// `<engine-dir>/scenes/<scene>/`.
    #[arg(long)]
    pub scene: Option<String>,

    /// Extra scenes to activate, comma-separated (e.g. `chat,research`).
    #[arg(long, value_delimiter = ',')]
    pub scenes: Vec<String>,

    /// Model for new sessions. Settings tiers still win over this.
    #[arg(long)]
    pub model: Option<String>,

    /// Serve the interface from this directory instead of the one compiled
    /// in. For working on the front end, and for running a different
    /// interface without rebuilding the backend.
    #[arg(long)]
    pub ui_dir: Option<PathBuf>,

    /// Serve no interface at all — a pure RPC node.
    #[arg(long)]
    pub headless: bool,

    /// Play recordings back from this directory instead of calling a model.
    ///
    /// A whole-process decision, and only an operator's to make: a client
    /// cannot ask for it. What it is for is tests — an agent's behaviour
    /// (tool calls, permission asks, sub-agents) becomes deterministic and
    /// free, instead of depending on what a provider felt like doing.
    #[arg(long)]
    pub replay_dir: Option<PathBuf>,

    /// TLS certificate chain (PEM). Required to bind anything but loopback.
    #[arg(long)]
    pub tls_cert: Option<PathBuf>,

    /// TLS private key (PEM).
    #[arg(long)]
    pub tls_key: Option<PathBuf>,

    /// The engine's directory: settings tiers, transcripts, memory, skills,
    /// recordings. Defaults to `$AGIX_CONFIG_HOME`, else `~/.agix`.
    #[arg(long)]
    pub agix_dir: Option<PathBuf>,

    /// The projects directory: where the picker starts and where a new project
    /// is created. Defaults to `$HARNIX_DATA_DIR`, else `~/Documents`. A
    /// starting point, not a fence.
    #[arg(long)]
    pub data_dir: Option<PathBuf>,

    /// Keep the token out of the page: a browser is admitted by a one-time
    /// launch link exchanged for a session cookie. One link is printed at
    /// startup; the token holder mints more at `POST /auth/link`.
    #[arg(long)]
    pub launch_link: bool,

    /// Stop when stdin closes: how a desktop shell that started this process takes it along
    /// when it goes, even when it is killed outright and cannot say so.
    #[arg(long, hide = true)]
    pub exit_with_stdin: bool,
}

/// What `--version` prints: every version a bug report needs.
pub(crate) fn versions(_app: &str, app_version: &str) -> (String, String) {
    let agix = daemon::discovery::identity();
    let agix = agix.get("version").and_then(|v| v.as_str()).unwrap_or("?").to_string();
    let short = app_version.to_string();
    let long = format!(
        "{app_version}\n\
         harnix {}\n\
         agix {agix}\n\
         module API {}\n\
         contribution API {}\n\
         protocol {}",
        crate::HARNIX_VERSION,
        harnix_module::MODULE_API_VERSION,
        harnix_contract::CONTRIB_API_VERSION,
        harnix_contract::PROTOCOL_VERSION,
    );
    (short, long)
}

pub(crate) fn parse(app: &str, app_version: &str, args: Option<Vec<OsString>>) -> Cli {
    let (short, long) = versions(app, app_version);
    let command = Cli::command()
        .name(app.to_string())
        .version(short)
        .long_version(long);
    let matches = match args {
        Some(args) => command.get_matches_from(args),
        None => command.get_matches(),
    };
    Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit())
}

/// The profile, then the flags on top of it.
///
/// A flag overriding a profile line is the ordinary case — try a different
/// scene without editing a file. A flag is only applied when it was actually
/// given, so an unmentioned flag does not quietly reset a profile line to its
/// own default.
pub(crate) fn resolve_profile(cli: &Cli) -> anyhow::Result<Profile> {
    let mut profile = match &cli.profile {
        Some(path) => Profile::load(path)?,
        None => Profile::default(),
    };
    if let Some(port) = cli.port {
        profile.transport.port = port;
    }
    if let Some(host) = cli.host {
        profile.transport.host = host.to_string();
    }
    if let Some(scene) = &cli.scene {
        profile.engine.scene = scene.clone();
    }
    if !cli.scenes.is_empty() {
        profile.engine.scenes = cli.scenes.clone();
    }
    if let Some(model) = &cli.model {
        profile.engine.model = model.clone();
    }
    if let Some(dir) = &cli.ui_dir {
        profile.ui.dir = Some(dir.clone());
    }
    if cli.launch_link {
        profile.transport.launch_link = true;
    }
    Ok(profile)
}
