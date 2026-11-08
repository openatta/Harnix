//! What an application is allowed, and what it is given.
//!
//! An [`AppSpec`] is the manifest's `[capabilities]` and `[[component]]`
//! limits after the person installing the application agreed to them. It is
//! resolved once, when the VM starts — paths expanded and checked, hosts
//! normalized — so no decision at call time depends on anything a component
//! could influence between the check and the use.
//!
//! # Nothing is granted by omission
//!
//! [`Capabilities::default`] grants nothing: no network, no environment, no
//! storage, no files. A component that declares nothing can compute and call
//! the hub methods the hub's own admission lets it call, and no more.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

/// Everything the VM needs to know about one application.
#[derive(Debug, Clone)]
pub struct AppSpec {
    /// `<publisher>.<name>`; how the host's services are told which
    /// application is asking.
    pub id: String,
    pub capabilities: Capabilities,
    pub limits: Limits,
    pub dirs: AppDirs,
    /// Handed to the backend's `init`.
    pub config: serde_json::Value,
}

/// The manifest's `[capabilities]`, as agreed to at install.
///
/// `hub` (which hub methods and topics the application may use) is not here:
/// it is the hub's admission that enforces it, against the subject
/// `module:<app id>`, exactly as for a compiled-in module's grants.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    /// Hosts `http-request` may reach, matched exactly and case-insensitively.
    /// No suffix rules: an application that wants a subdomain names it.
    pub net: Vec<String>,
    /// Environment variables `secret` may read, matched exactly.
    pub env: Vec<String>,
    /// Ceiling on the key-value store, keys and values both counted.
    pub storage_bytes: u64,
    /// Directories preopened through WASI.
    pub fs: Vec<FsGrant>,
}

/// One preopened directory.
///
/// `path` must start at an anchor — `${data}` (the application's own data
/// directory) or `${project}` (read-only) — and may not contain `..`. An
/// unanchored path is refused rather than taken literally: `/` would read as
/// a grant of the whole machine, which is exactly the line a reviewer skims.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsGrant {
    pub path: String,
    #[serde(default)]
    pub writable: bool,
}

/// Where the anchors of [`FsGrant`] point for this application.
#[derive(Debug, Clone, Default)]
pub struct AppDirs {
    /// `${data}`. Created if it does not exist. Keep the key-value store
    /// ([`crate::FileKv`]) outside it: anything under `${data}` is the
    /// component's to rewrite when it is granted writable.
    pub data: PathBuf,
    /// `${project}`, if the host has one to offer; a grant of it is refused
    /// otherwise.
    pub project: Option<PathBuf>,
}

/// The manifest's `[[component]]` limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Linear memory for the whole application: the resident backend and
    /// every tool call running at the same moment, together.
    pub memory_bytes: u64,
    /// How long one call (a method, a tool, an event, `init`, `stop`) may
    /// take, waits on the host included.
    pub timeout: Duration,
}

impl Default for Limits {
    /// The manifest's defaults: 64 MiB, 30 s.
    fn default() -> Self {
        Self {
            memory_bytes: 64 * 1024 * 1024,
            timeout: Duration::from_secs(30),
        }
    }
}

/// Capabilities resolved against this application's directories.
#[derive(Debug, Clone, Default)]
pub(crate) struct Resolved {
    net: HashSet<String>,
    env: HashSet<String>,
    pub storage_bytes: u64,
    pub preopens: Vec<Preopen>,
}

#[derive(Debug, Clone)]
pub(crate) struct Preopen {
    pub host: PathBuf,
    /// Where the component sees it: `/data/…` or `/project/…`. Never the
    /// host's own path — where the application's files live on this machine
    /// is none of its business, and a stable name is what its code can be
    /// written against.
    pub guest: String,
    pub writable: bool,
}

impl Resolved {
    pub fn resolve(caps: &Capabilities, dirs: &AppDirs) -> Result<Self> {
        let mut preopens = Vec::new();
        for grant in &caps.fs {
            preopens.push(resolve_grant(grant, dirs)?);
        }
        Ok(Self {
            net: caps.net.iter().map(|h| h.to_ascii_lowercase()).collect(),
            env: caps.env.iter().cloned().collect(),
            storage_bytes: caps.storage_bytes,
            preopens,
        })
    }

    pub fn allows_url(&self, url: &str) -> bool {
        host_of(url).is_some_and(|host| self.net.contains(&host))
    }

    pub fn allows_env(&self, key: &str) -> bool {
        self.env.contains(key)
    }
}

fn resolve_grant(grant: &FsGrant, dirs: &AppDirs) -> Result<Preopen> {
    let raw = grant.path.as_str();
    let (anchor, rest, guest_root) = if let Some(rest) = raw.strip_prefix("${data}") {
        (dirs.data.clone(), rest, "/data")
    } else if let Some(rest) = raw.strip_prefix("${project}") {
        if grant.writable {
            bail!("`{raw}`: the project is offered read-only");
        }
        let Some(project) = &dirs.project else {
            bail!("`{raw}`: there is no project to offer this application");
        };
        (project.clone(), rest, "/project")
    } else {
        bail!(
            "capability path `{raw}` must start with ${{data}} or ${{project}} — an unanchored \
             path would grant more than a reviewer can judge"
        );
    };
    if !rest.is_empty() && !rest.starts_with('/') {
        bail!("capability path `{raw}`: expected a `/` after the anchor");
    }
    let rest = rest.trim_start_matches('/');
    if Path::new(rest)
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        bail!("capability path `{raw}` may not contain `..`, `.` or a root");
    }
    let host = anchor.join(rest);
    if guest_root == "/data" {
        std::fs::create_dir_all(&host)
            .with_context(|| format!("creating {} for `{raw}`", host.display()))?;
    } else if !host.is_dir() {
        bail!("`{raw}` is not a directory ({})", host.display());
    }
    let guest = if rest.is_empty() {
        guest_root.to_string()
    } else {
        format!("{guest_root}/{rest}")
    };
    Ok(Preopen {
        host,
        guest,
        writable: grant.writable,
    })
}

/// Lowercased host of an absolute http(s) URL; `None` means denied.
///
/// Public because a refusal must be able to name what was refused without
/// echoing the URL: a component may have built it from a secret, and the
/// refusal often travels on into a tool result and the transcript.
///
/// Deliberately narrow: anything that is not plainly `http(s)://host…`
/// yields `None`. A parser that is clever about malformed input is one an
/// attacker gets to negotiate with.
pub fn host_of(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    // Userinfo is the classic disguise (`https://allowed.example@evil.example/`).
    let authority = authority.rsplit('@').next()?;
    let host = authority.split(':').next()?;
    if host.is_empty() {
        return None;
    }
    Some(host.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve(caps: Capabilities, dirs: &AppDirs) -> Result<Resolved> {
        Resolved::resolve(&caps, dirs)
    }

    fn dirs() -> (tempfile::TempDir, AppDirs) {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("project");
        std::fs::create_dir_all(project.join("src")).unwrap();
        let dirs = AppDirs {
            data: tmp.path().join("data"),
            project: Some(project),
        };
        (tmp, dirs)
    }

    #[test]
    fn nothing_declared_grants_nothing() {
        let r = resolve(Capabilities::default(), &AppDirs::default()).unwrap();
        assert!(!r.allows_url("https://example.com/"));
        assert!(!r.allows_env("PATH"));
        assert!(r.preopens.is_empty());
        assert_eq!(r.storage_bytes, 0);
    }

    #[test]
    fn net_matches_the_host_exactly() {
        let r = resolve(
            Capabilities {
                net: vec!["API.example.com".into()],
                ..Default::default()
            },
            &AppDirs::default(),
        )
        .unwrap();
        assert!(r.allows_url("https://api.example.com/x"));
        assert!(r.allows_url("http://api.EXAMPLE.com:8080/x"));
        assert!(!r.allows_url("https://example.com/x"), "not a suffix rule");
        assert!(!r.allows_url("https://api.example.com.evil.test/x"));
        assert!(!r.allows_url("https://api.example.com@evil.test/x"));
        assert!(r.allows_url("https://user:pw@api.example.com/x"));
        for url in [
            "file:///etc/passwd",
            "ftp://api.example.com/",
            "api.example.com/x",
            "https://",
        ] {
            assert!(!r.allows_url(url), "{url}");
        }
    }

    #[test]
    fn env_is_exact_and_case_sensitive() {
        let r = resolve(
            Capabilities {
                env: vec!["TOKEN".into()],
                ..Default::default()
            },
            &AppDirs::default(),
        )
        .unwrap();
        assert!(r.allows_env("TOKEN"));
        assert!(!r.allows_env("token"));
        assert!(!r.allows_env("TOKEN_2"));
    }

    #[test]
    fn grants_resolve_under_their_anchor_and_hide_the_host_path() {
        let (_tmp, dirs) = dirs();
        let r = resolve(
            Capabilities {
                fs: vec![
                    FsGrant {
                        path: "${data}".into(),
                        writable: true,
                    },
                    FsGrant {
                        path: "${data}/cache".into(),
                        writable: true,
                    },
                    FsGrant {
                        path: "${project}/src".into(),
                        writable: false,
                    },
                ],
                ..Default::default()
            },
            &dirs,
        )
        .unwrap();
        let guests: Vec<&str> = r.preopens.iter().map(|p| p.guest.as_str()).collect();
        assert_eq!(guests, ["/data", "/data/cache", "/project/src"]);
        assert!(
            dirs.data.join("cache").is_dir(),
            "the data directory is created"
        );
        assert_eq!(r.preopens[2].host, dirs.project.unwrap().join("src"));
    }

    #[test]
    fn unanchored_traversing_and_writable_project_grants_are_refused() {
        let (_tmp, dirs) = dirs();
        for (path, writable, says) in [
            ("/", false, "must start with"),
            ("${data}/../..", true, "may not contain"),
            ("${data}/./x", true, "may not contain"),
            ("${data}x", true, "expected a `/`"),
            ("${project}", true, "read-only"),
            ("${project}/missing", false, "not a directory"),
        ] {
            let err = resolve(
                Capabilities {
                    fs: vec![FsGrant {
                        path: path.into(),
                        writable,
                    }],
                    ..Default::default()
                },
                &dirs,
            )
            .err()
            .unwrap_or_else(|| panic!("`{path}` should be refused"))
            .to_string();
            assert!(err.contains(says), "`{path}`: {err}");
        }
    }

    #[test]
    fn a_project_grant_without_a_project_is_refused() {
        let err = resolve(
            Capabilities {
                fs: vec![FsGrant {
                    path: "${project}".into(),
                    writable: false,
                }],
                ..Default::default()
            },
            &AppDirs::default(),
        )
        .err()
        .unwrap()
        .to_string();
        assert!(err.contains("no project"), "{err}");
    }
}
