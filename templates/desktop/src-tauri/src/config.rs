//! `desktop.toml`: which server this window shows, and how to reach it.
//!
//! Looked for in this order, first found wins:
//!
//! 1. the file `HARNIX_DESKTOP_CONFIG` names (it must exist);
//! 2. `desktop.toml` in the user's config directory, under the app's
//!    identifier — what a user writes for themselves;
//! 3. `desktop.toml` next to the executable — what the app ships;
//! 4. nothing: local mode with the bundled sidecar.
//!
//! Then every `HARNIX_DESKTOP_*` variable that is set replaces its setting.
//! Relative paths in a file are relative to that file, so a config can travel
//! with the binary it names.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use tauri::Url;

/// The prefix of every environment variable the shell reads. An app renames
/// it with everything else (README: 改成你的应用).
const ENV_PREFIX: &str = "HARNIX_DESKTOP";

const FILE: &str = "desktop.toml";

/// The app binary when `[local] program` names none: the sidecar Tauri puts
/// next to this executable (`bundle.externalBin`, target triple removed).
const SIDECAR: &str = "harnix";

/// The app's `[brand] product_id`: its state directory is `~/.<PRODUCT_ID>` when
/// neither `HARNIX_STATE_DIR` nor a profile given to the shell says otherwise. An
/// app whose profile is built into its binary renames it with everything else
/// (README: 改成你的应用), or the shell looks for the token in `~/.harnix`.
pub const PRODUCT_ID: &str = "harnix";

/// What the shell was told to show.
#[derive(Debug)]
pub enum Config {
    Local(Local),
    Remote(Remote),
}

#[derive(Debug)]
pub struct Local {
    pub program: PathBuf,
    pub profile: Option<PathBuf>,
    /// Passed through after the shell's own flags. Not `--host`, `--port` or
    /// `--launch-link`: the shell sets those, and the app refuses a flag
    /// given twice.
    pub args: Vec<String>,
    /// `None` picks a free loopback port at every start.
    pub port: Option<u16>,
}

#[derive(Debug)]
pub struct Remote {
    pub url: Url,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Mode {
    #[default]
    Local,
    Remote,
}

// `deny_unknown_fields` throughout: a misspelt key is a setting silently not
// applied, and the person who wrote it will be looking at the app, not here.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    mode: Mode,
    #[serde(default)]
    local: LocalFile,
    #[serde(default)]
    remote: RemoteFile,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalFile {
    program: Option<PathBuf>,
    profile: Option<PathBuf>,
    #[serde(default)]
    args: Vec<String>,
    port: Option<u16>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoteFile {
    url: Option<String>,
}

/// Read the configuration. An error is a sentence for the error page.
pub fn load(identifier: &str) -> Result<Config, String> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_default();

    let (mut file, base) = match find(identifier, &exe_dir)? {
        Some(path) => {
            let text = std::fs::read_to_string(&path)
                .map_err(|e| format!("读不了 {}：{e}", path.display()))?;
            let file: File =
                toml::from_str(&text).map_err(|e| format!("{} 有误：{e}", path.display()))?;
            let base = path.parent().map(Path::to_path_buf).unwrap_or_default();
            (file, base)
        }
        None => (File::default(), exe_dir.clone()),
    };

    // Every path leaves here absolute: the child runs in the home directory,
    // not wherever the shell happened to start. A file's paths are relative
    // to the file; the environment's to the current directory, like any
    // other command-line path.
    let in_file = |p: PathBuf| absolute(&base.join(p));
    file.local.program = file.local.program.map(in_file);
    file.local.profile = file.local.profile.map(in_file);
    if let Some(mode) = env("MODE") {
        file.mode = match mode.as_str() {
            "local" => Mode::Local,
            "remote" => Mode::Remote,
            other => return Err(format!("{ENV_PREFIX}_MODE={other}：只能是 local 或 remote")),
        };
    }
    if let Some(program) = env("PROGRAM") {
        file.local.program = Some(absolute(Path::new(&program)));
    }
    if let Some(profile) = env("PROFILE") {
        file.local.profile = Some(absolute(Path::new(&profile)));
    }
    if let Some(port) = env("PORT") {
        let port = port
            .parse()
            .map_err(|_| format!("{ENV_PREFIX}_PORT={port}：不是端口号"))?;
        file.local.port = Some(port);
    }
    if let Some(url) = env("URL") {
        file.remote.url = Some(url);
    }

    match file.mode {
        Mode::Local => Ok(Config::Local(Local {
            program: file.local.program.unwrap_or_else(|| {
                exe_dir.join(format!("{SIDECAR}{}", std::env::consts::EXE_SUFFIX))
            }),
            profile: file.local.profile,
            args: file.local.args,
            port: file.local.port,
        })),
        Mode::Remote => {
            let url = file
                .remote
                .url
                .ok_or("远程模式要在 [remote] url 写上服务端的地址")?;
            Ok(Config::Remote(Remote {
                url: remote_url(&url)?,
            }))
        }
    }
}

fn find(identifier: &str, exe_dir: &Path) -> Result<Option<PathBuf>, String> {
    if let Some(path) = env("CONFIG") {
        let path = PathBuf::from(path);
        return if path.is_file() {
            Ok(Some(path))
        } else {
            Err(format!(
                "{ENV_PREFIX}_CONFIG 指向的 {} 不存在",
                path.display()
            ))
        };
    }
    let user = dirs::config_dir().map(|d| d.join(identifier).join(FILE));
    let shipped = Some(exe_dir.join(FILE));
    Ok([user, shipped].into_iter().flatten().find(|p| p.is_file()))
}

/// A remote Harnix is reached over TLS: it refuses to listen anywhere but
/// loopback without it (design §6.3), so an `http://` address of another host
/// is a mistake to report now rather than a connection that cannot work.
fn remote_url(text: &str) -> Result<Url, String> {
    let url = Url::parse(text).map_err(|e| format!("[remote] url「{text}」不是地址：{e}"))?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    match url.scheme() {
        "https" => Ok(url),
        "http" if loopback => Ok(url),
        _ => Err(format!("[remote] url「{text}」要用 https://")),
    }
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

fn env(name: &str) -> Option<String> {
    std::env::var(format!("{ENV_PREFIX}_{name}"))
        .ok()
        .filter(|v| !v.is_empty())
}
