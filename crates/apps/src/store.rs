//! The store: a static catalog (`index.json`) of applications, their versions, and a package per
//! engine for each version — and fetching one of them.
//!
//! ```json
//! {
//!   "format": "harnix-store-index@1",
//!   "name": "…",
//!   "apps": [{
//!     "id": "acme.notes", "title": "Notes", "publisher": "acme", "description": "…",
//!     "versions": [{
//!       "version": "1.2.0",
//!       "harnix_api": 1,
//!       "capabilities": {"net": ["api.acme.com"], …},
//!       "signatures": {"publisher": "<key>", "store": "<key>"},
//!       "artifacts": {"<engine fingerprint>": {"url": "packages/acme.notes-1.2.0.<engine>.hxapp", "sha256": "…", "size": 12345}}
//!     }]
//!   }]
//! }
//! ```
//!
//! An artifact is a whole package, built for one engine (`"*"`: an application with no component,
//! for every engine); its URL is relative to the index's, or absolute. The index is a list of
//! offers and nothing it says is trusted: what is downloaded is checked against the sha256 it
//! lists, and then against the package's own `SIGNATURES` (`signature.rs`) — the capabilities and
//! keys here are only what a catalog page shows before anybody downloads anything.
//!
//! The store is `https://`, or `file://` (a directory on this machine: tests, a mirror). The
//! process reads the index afresh for every question; there is nothing to go stale.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::manifest::HARNIX_API;
use crate::signature::{sha256, PublicKey};

pub const INDEX_FORMAT: &str = "harnix-store-index@1";

/// Where the catalog is unless the profile says otherwise (`[apps] store`).
pub const DEFAULT_STORE: &str = "https://store.harnix.dev/index.json";

/// The store's key, built in: a package countersigned with it is `store` trust. A profile cannot
/// add to this list (it names trusted publishers instead); a test application can replace it
/// (`harnix::AppBuilder::store_keys`, hidden).
pub const STORE_KEYS: &[&str] =
    &["c7a9ed9597ad6f8b445e6303281a68ce4f8666de3f75cc2cc859899ef5ade505"];

/// The artifact of an application with no component: the same package for every engine.
pub const ANY_ENGINE: &str = "*";

/// What one index or package may weigh.
const MAX_INDEX: u64 = 16 * 1024 * 1024;
const MAX_PACKAGE: u64 = crate::package::MAX_UNPACKED;

pub fn store_keys() -> Vec<PublicKey> {
    STORE_KEYS
        .iter()
        .map(|k| PublicKey::parse(k).expect("the built-in store key is a key"))
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Index {
    pub format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub apps: Vec<CatalogApp>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogApp {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub publisher: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub versions: Vec<CatalogVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogVersion {
    pub version: String,
    pub harnix_api: u32,
    /// What the version's manifest asks for, as the catalog shows it before a download.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub capabilities: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub signatures: Value,
    /// By engine fingerprint (or [`ANY_ENGINE`]).
    #[serde(default)]
    pub artifacts: BTreeMap<String, Artifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub url: String,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

impl Index {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let index: Index = serde_json::from_slice(bytes)
            .map_err(|e| format!("the store's index does not read: {e}"))?;
        if index.format != INDEX_FORMAT {
            return Err(format!(
                "the store's index is `{}`, and this Harnix reads `{INDEX_FORMAT}`",
                index.format
            ));
        }
        Ok(index)
    }
}

/// `x.y.z[-pre]`, compared as numbers, a pre-release before its release.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let split = |v: &str| -> (Vec<u64>, Option<String>) {
        let (core, pre) = match v.split_once('-') {
            Some((core, pre)) => (core, Some(pre.to_string())),
            None => (v, None),
        };
        (
            core.split('.').map(|n| n.parse().unwrap_or(0)).collect(),
            pre,
        )
    };
    let (ca, pa) = split(a);
    let (cb, pb) = split(b);
    ca.cmp(&cb).then_with(|| match (pa, pb) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => x.cmp(&y),
    })
}

/// One version of one application, chosen, and its artifact for this engine.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub app: CatalogApp,
    pub version: CatalogVersion,
    pub artifact: Artifact,
    pub url: Url,
}

#[derive(Debug)]
pub struct Store {
    url: Url,
}

impl Store {
    pub fn new(url: &str) -> Result<Self, String> {
        let url = Url::parse(url).map_err(|e| format!("the store `{url}` is not a URL: {e}"))?;
        check_scheme(&url)?;
        Ok(Self { url })
    }

    pub fn url(&self) -> &str {
        self.url.as_str()
    }

    pub async fn index(&self) -> Result<Index, String> {
        Index::parse(&get(&self.url, MAX_INDEX).await?)
    }

    /// The version to install: `version`, or the newest one of this Harnix's `harnix_api` — and
    /// its artifact for the engine `fingerprint`, or why there is none.
    pub fn resolve(
        &self,
        index: &Index,
        id: &str,
        version: Option<&str>,
        fingerprint: &str,
    ) -> Result<Resolved, String> {
        let app = index
            .apps
            .iter()
            .find(|a| a.id == id)
            .ok_or_else(|| format!("the store has no application `{id}`"))?;
        let chosen = match version {
            Some(wanted) => {
                let v = app
                    .versions
                    .iter()
                    .find(|v| v.version == wanted)
                    .ok_or_else(|| {
                        format!(
                            "the store has no version {wanted} of `{id}` (it has {})",
                            app.versions
                                .iter()
                                .map(|v| v.version.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    })?;
                if v.harnix_api != HARNIX_API {
                    return Err(format!(
                        "`{id}` {wanted} is written for application interface {} and this Harnix \
                         implements {HARNIX_API}",
                        v.harnix_api
                    ));
                }
                v
            }
            None => app
                .versions
                .iter()
                .filter(|v| v.harnix_api == HARNIX_API)
                .max_by(|a, b| compare_versions(&a.version, &b.version))
                .ok_or_else(|| {
                    format!(
                        "the store has no version of `{id}` for application interface {HARNIX_API}"
                    )
                })?,
        };
        let artifact = chosen
            .artifacts
            .get(fingerprint)
            .or_else(|| chosen.artifacts.get(ANY_ENGINE))
            .ok_or_else(|| {
                let offered: Vec<String> =
                    chosen.artifacts.keys().map(|k| format!("`{k}`")).collect();
                format!(
                    "`{id}` {} has no build for this Harnix's engine (`{fingerprint}`){}: upgrade \
                     Harnix, or wait for the store to build it for this release",
                    chosen.version,
                    if offered.is_empty() {
                        String::new()
                    } else {
                        format!("; the store has it for {}", offered.join(", "))
                    }
                )
            })?;
        let url = self
            .url
            .join(&artifact.url)
            .map_err(|e| format!("the artifact's URL `{}`: {e}", artifact.url))?;
        check_scheme(&url)?;
        Ok(Resolved {
            app: app.clone(),
            version: chosen.clone(),
            artifact: artifact.clone(),
            url,
        })
    }

    /// Download it to `to`, and check it is the package the index lists.
    pub async fn download(&self, resolved: &Resolved, to: &Path) -> Result<(), String> {
        let bytes = get(&resolved.url, MAX_PACKAGE).await?;
        let actual = sha256(&bytes);
        if !actual.eq_ignore_ascii_case(&resolved.artifact.sha256) {
            return Err(format!(
                "the package downloaded from {} is not the one the store lists: its sha256 is \
                 {actual}, and the index says {}",
                resolved.url, resolved.artifact.sha256
            ));
        }
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(to, bytes).map_err(|e| format!("{}: {e}", to.display()))
    }
}

/// What the catalog is shown as: each application, its versions for this Harnix, whether this
/// engine has a build of each, and the newest it could install.
pub fn view(index: &Index, fingerprint: &str) -> Value {
    let apps: Vec<Value> = index
        .apps
        .iter()
        .map(|app| {
            let mut versions: Vec<&CatalogVersion> = app.versions.iter().collect();
            versions.sort_by(|a, b| compare_versions(&b.version, &a.version));
            let installable = |v: &CatalogVersion| {
                v.harnix_api == HARNIX_API
                    && (v.artifacts.contains_key(fingerprint)
                        || v.artifacts.contains_key(ANY_ENGINE))
            };
            let latest = versions.iter().find(|v| v.harnix_api == HARNIX_API);
            json!({
                "id": app.id,
                "title": app.title,
                "publisher": app.publisher,
                "description": app.description,
                "latest": latest.map(|v| v.version.clone()),
                "installable": latest.is_some_and(|v| installable(v)),
                "capabilities": latest.map(|v| v.capabilities.clone()),
                "versions": versions.iter().map(|v| json!({
                    "version": v.version,
                    "harnix_api": v.harnix_api,
                    "installable": installable(v),
                    "engines": v.artifacts.keys().collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({"name": index.name, "apps": apps})
}

fn check_scheme(url: &Url) -> Result<(), String> {
    match url.scheme() {
        "https" | "file" => Ok(()),
        other => Err(format!(
            "`{url}` is {other}://, and a store is https:// (or file:// on this machine)"
        )),
    }
}

async fn get(url: &Url, limit: u64) -> Result<Vec<u8>, String> {
    if url.scheme() == "file" {
        let path = url
            .to_file_path()
            .map_err(|_| format!("`{url}` is not a path on this machine"))?;
        let meta = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if meta.len() > limit {
            return Err(format!(
                "{} is larger than {} MiB",
                path.display(),
                limit >> 20
            ));
        }
        return std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()));
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;
    let response = client
        .get(url.clone())
        .send()
        .await
        .map_err(|e| format!("the store did not answer ({url}): {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "the store answered {} for {url}",
            response.status()
        ));
    }
    if response.content_length().is_some_and(|n| n > limit) {
        return Err(format!("{url} is larger than {} MiB", limit >> 20));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("reading {url}: {e}"))?;
    if bytes.len() as u64 > limit {
        return Err(format!("{url} is larger than {} MiB", limit >> 20));
    }
    Ok(bytes.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(fingerprint: &str) -> Index {
        Index::parse(
            json!({
                "format": INDEX_FORMAT,
                "apps": [{
                    "id": "acme.notes", "publisher": "acme",
                    "versions": [
                        {"version": "1.0.0", "harnix_api": 1, "artifacts": {fingerprint: {"url": "p/1.0.0.hxapp", "sha256": "aa"}}},
                        {"version": "1.10.0", "harnix_api": 1, "artifacts": {"engine-0+elsewhere+0": {"url": "p/1.10.0.hxapp", "sha256": "bb"}}},
                        {"version": "1.2.0", "harnix_api": 1, "artifacts": {fingerprint: {"url": "https://cdn.example.com/1.2.0.hxapp", "sha256": "cc"}}},
                        {"version": "2.0.0", "harnix_api": 2, "artifacts": {fingerprint: {"url": "p/2.0.0.hxapp", "sha256": "dd"}}},
                        {"version": "1.3.0-rc.1", "harnix_api": 1, "artifacts": {"*": {"url": "p/any.hxapp", "sha256": "ee"}}}
                    ]
                }]
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap()
    }

    #[test]
    fn versions_compare_as_numbers() {
        assert_eq!(compare_versions("1.10.0", "1.2.0"), Ordering::Greater);
        assert_eq!(compare_versions("1.2.0-rc.1", "1.2.0"), Ordering::Less);
        assert_eq!(compare_versions("1.2.0", "1.2.0"), Ordering::Equal);
        assert_eq!(compare_versions("0.2.0", "0.10.0"), Ordering::Less);
    }

    #[test]
    fn the_newest_version_for_this_interface_is_chosen_and_its_engine_must_match() {
        let store = Store::new("file:///srv/store/index.json").unwrap();
        let fp = "engine-47+here+123";
        let index = index(fp);
        // 1.10.0 is the newest of interface 1, and has no build for this engine: said so, with
        // the engines it has one for — not quietly swapped for an older version.
        let err = store.resolve(&index, "acme.notes", None, fp).unwrap_err();
        assert!(
            err.contains("no build for this Harnix's engine (`engine-47+here+123`)"),
            "{err}"
        );
        assert!(err.contains("`engine-0+elsewhere+0`"), "{err}");
        assert!(err.contains("upgrade Harnix"), "{err}");

        let r = store
            .resolve(&index, "acme.notes", Some("1.0.0"), fp)
            .unwrap();
        assert_eq!(r.url.as_str(), "file:///srv/store/p/1.0.0.hxapp");
        let r = store
            .resolve(&index, "acme.notes", Some("1.2.0"), fp)
            .unwrap();
        assert_eq!(r.url.as_str(), "https://cdn.example.com/1.2.0.hxapp");
        // An application with no component has one package for every engine.
        let r = store
            .resolve(&index, "acme.notes", Some("1.3.0-rc.1"), fp)
            .unwrap();
        assert_eq!(r.artifact.sha256, "ee");

        let err = store
            .resolve(&index, "acme.notes", Some("2.0.0"), fp)
            .unwrap_err();
        assert!(err.contains("application interface 2"), "{err}");
        assert!(store
            .resolve(&index, "acme.notes", Some("9.9.9"), fp)
            .unwrap_err()
            .contains("no version 9.9.9"));
        assert!(store
            .resolve(&index, "acme.nobody", None, fp)
            .unwrap_err()
            .contains("no application"));
    }

    #[test]
    fn the_view_says_what_this_engine_can_install() {
        let fp = "engine-47+here+123";
        let view = view(&index(fp), fp);
        let app = &view["apps"][0];
        assert_eq!(app["latest"], json!("1.10.0"));
        assert_eq!(app["installable"], json!(false));
        let installable: Vec<&str> = app["versions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["installable"] == json!(true))
            .map(|v| v["version"].as_str().unwrap())
            .collect();
        assert_eq!(installable, ["1.3.0-rc.1", "1.2.0", "1.0.0"]);
    }

    #[test]
    fn a_store_is_https_or_on_this_machine() {
        assert!(Store::new("http://store.example.com/index.json")
            .unwrap_err()
            .contains("https://"));
        assert!(Store::new("https://store.example.com/index.json").is_ok());
        let store = Store::new("file:///srv/store/index.json").unwrap();
        let mut index = index("fp");
        index.apps[0].versions[0]
            .artifacts
            .get_mut("fp")
            .unwrap()
            .url = "http://plain.example.com/x.hxapp".into();
        assert!(store
            .resolve(&index, "acme.notes", Some("1.0.0"), "fp")
            .unwrap_err()
            .contains("https://"));
        assert!(Index::parse(br#"{"format": "other@9", "apps": []}"#)
            .unwrap_err()
            .contains("harnix-store-index@1"));
    }

    #[tokio::test]
    async fn a_download_is_the_package_the_index_lists_or_nothing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pkg.hxapp"), b"the package").unwrap();
        let index_url = Url::from_file_path(dir.path().join("index.json")).unwrap();
        let store = Store::new(index_url.as_str()).unwrap();
        let resolved = |sha: &str| Resolved {
            app: index("fp").apps[0].clone(),
            version: index("fp").apps[0].versions[0].clone(),
            artifact: Artifact {
                url: "pkg.hxapp".into(),
                sha256: sha.into(),
                size: None,
            },
            url: index_url.join("pkg.hxapp").unwrap(),
        };
        let to = dir.path().join("staging/pkg.hxapp");
        store
            .download(&resolved(&sha256(b"the package")), &to)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), b"the package");
        let other = dir.path().join("staging/other.hxapp");
        let err = store
            .download(&resolved(&sha256(b"something else")), &other)
            .await
            .unwrap_err();
        assert!(err.contains("not the one the store lists"), "{err}");
        assert!(!other.exists());
    }
}
