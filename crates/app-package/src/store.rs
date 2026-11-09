//! The store: a catalog (`index.json`) of applications, their versions, and a package per engine
//! for each version — signed by the store, and fetching one of them.
//!
//! ```json
//! {
//!   "format": "harnix-store-index@1",
//!   "name": "…",
//!   "expires": "2026-12-01T00:00:00Z",
//!   "apps": [{
//!     "id": "acme.notes", "title": "Notes", "publisher": "acme", "description": "…",
//!     "min_version": "1.1.0",
//!     "versions": [{
//!       "version": "1.2.0",
//!       "harnix_api": 1,
//!       "capabilities": {"net": ["api.acme.com"], …},
//!       "signatures": {"publisher": "<key>", "store": "<key>"},
//!       "revoked": "why, when the store withdrew it",
//!       "artifacts": {"<engine fingerprint>": {"url": "packages/acme.notes-1.2.0.<engine>.hxapp", "sha256": "…", "size": 12345}}
//!     }]
//!   }],
//!   "signature": {"key": "<store key>", "signature": "<hex>"}
//! }
//! ```
//!
//! An artifact is a whole package, built for one engine (`"*"`: an application with no component,
//! for every engine); its URL is relative to the index's, or absolute. What is downloaded is
//! checked against the sha256 the index lists, and then against the package's own `SIGNATURES`
//! (`signature.rs`): the capabilities and keys here are only what a catalog page shows before
//! anybody downloads anything.
//!
//! The index itself is believed only as the store signed it ([`Index::verify`]): with one of the
//! store keys this Harnix carries, over everything in it but the signature, and only until it
//! `expires`. So whoever serves the catalog — a CDN, a mirror, someone in between — cannot serve
//! an old copy in which a version the store has since revoked is still the newest, for longer
//! than the store chose; nor drop a revocation (`revoked`, or an application's `min_version`: the
//! store supports it from that version on), which a version installed here is flagged by.
//!
//! The store is `https://`, or `file://` (a directory on this machine: tests, a mirror). Over
//! `https://` a redirect is followed only to `https://`. The process reads the index afresh for
//! every question; there is nothing to go stale here.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::manifest::HARNIX_API;
use crate::signature::{sha256, PublicKey, SecretKey};

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

/// Redirects followed for one download, every one of them to `https://`.
const MAX_REDIRECTS: usize = 5;

/// What the store's signature over its catalog is over: this line, then the catalog without its
/// `signature`, as [`canonical`] writes it.
const CATALOG_PAYLOAD: &str = "harnix-store-index@1 catalog\n";

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
    /// Until when the catalog is believed: `YYYY-MM-DDTHH:MM:SSZ`, UTC ([`parse_utc`]).
    #[serde(default)]
    pub expires: String,
    #[serde(default)]
    pub apps: Vec<CatalogApp>,
    /// The store's, over everything else in it ([`sign_catalog`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<CatalogSignature>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogSignature {
    pub key: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogApp {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub publisher: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The oldest version the store still supports: one before it is withdrawn, as a revoked one
    /// is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_version: Option<String>,
    #[serde(default)]
    pub versions: Vec<CatalogVersion>,
}

impl CatalogApp {
    /// Why the store no longer offers `version` of it, if it does not: revoked, or older than its
    /// `min_version`. A version the catalog does not list at all is held to `min_version` alone.
    pub fn withdrawn(&self, version: &str) -> Option<String> {
        let revoked = self
            .versions
            .iter()
            .find(|v| v.version == version)
            .and_then(|v| v.revoked.as_ref());
        if let Some(reason) = revoked {
            return Some(format!("the store revoked it: {reason}"));
        }
        match &self.min_version {
            Some(min) if compare_versions(version, min) == Ordering::Less => Some(format!(
                "the store supports `{}` from {min} on",
                self.id
            )),
            _ => None,
        }
    }
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
    /// Why the store withdrew this version: it is not installed from here again, and where it is
    /// installed it is flagged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked: Option<String>,
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
    /// Read a catalog without believing it: what the store's own tools do with the one they are
    /// making. A catalog a store served is read with [`Index::verify`].
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

    /// Read a catalog a store served, and believe it only if one of `keys` signed it and it has
    /// not expired at `now`.
    pub fn verify(bytes: &[u8], keys: &[PublicKey], now: SystemTime) -> Result<Self, String> {
        let mut value: Value = serde_json::from_slice(bytes)
            .map_err(|e| format!("the store's index does not read: {e}"))?;
        let signature = value
            .as_object_mut()
            .and_then(|o| o.remove("signature"))
            .ok_or("the store's catalog is not signed, and a catalog is believed only as its store signed it")?;
        let signature: CatalogSignature = serde_json::from_value(signature)
            .map_err(|e| format!("the store's catalog signature does not read: {e}"))?;
        let key = PublicKey::parse(&signature.key)
            .map_err(|e| format!("the store's catalog signature: {e}"))?;
        if !keys.contains(&key) {
            return Err(format!(
                "the store's catalog is signed by a key this Harnix does not carry ({})",
                signature.key
            ));
        }
        if !key.verifies(&catalog_payload(&value), &signature.signature) {
            return Err(
                "the store's catalog signature does not verify: it is not the catalog the store signed"
                    .into(),
            );
        }
        let index = Index::parse(bytes)?;
        let expires = parse_utc(&index.expires).map_err(|e| {
            format!("the store's catalog has no expiry it can be held to (`expires`: {e})")
        })?;
        if now >= UNIX_EPOCH + Duration::from_secs(expires) {
            return Err(format!(
                "the store's catalog expired at {}: the store has not published a fresh one, or this \
                 is an old copy of it",
                index.expires
            ));
        }
        Ok(index)
    }
}

/// Sign `catalog` (an index, as JSON) as the store, with `key`: whatever signature it had is
/// replaced. Its `expires` is the caller's to set first: it is signed with the rest.
pub fn sign_catalog(catalog: &mut Value, key: &SecretKey) -> Result<(), String> {
    let object = catalog
        .as_object_mut()
        .ok_or("a catalog is a JSON object")?;
    object.remove("signature");
    let signature = CatalogSignature {
        key: key.public().hex(),
        signature: key.sign(&catalog_payload(catalog)),
    };
    catalog["signature"] = serde_json::to_value(signature).expect("plain data serializes");
    Ok(())
}

fn catalog_payload(unsigned: &Value) -> Vec<u8> {
    let mut text = CATALOG_PAYLOAD.to_string();
    canonical(unsigned, &mut text);
    text.into_bytes()
}

/// One way to write a JSON value: no whitespace, every object's keys sorted. What the store signs
/// and a Harnix verifies are then the same bytes however either side's JSON library orders keys.
fn canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(key.clone()).to_string());
                out.push(':');
                canonical(&map[key], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// `YYYY-MM-DDTHH:MM:SSZ` as seconds since the epoch: the one form a catalog's `expires` is
/// written in.
pub fn parse_utc(text: &str) -> Result<u64, String> {
    let bad = || format!("`{text}` is not a time written as YYYY-MM-DDTHH:MM:SSZ");
    let b = text.as_bytes();
    if b.len() != 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' || b[19] != b'Z' {
        return Err(bad());
    }
    let n = |range: std::ops::Range<usize>| -> Result<i64, String> {
        text[range].parse::<u32>().map(i64::from).map_err(|_| bad())
    };
    let (y, m, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
    let (hh, mm, ss) = (n(11..13)?, n(14..16)?, n(17..19)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || hh > 23 || mm > 59 || ss > 59 || y < 1970 {
        return Err(bad());
    }
    let secs = (days_from_civil(y, m, d) * 86_400 + hh * 3600 + mm * 60 + ss) as u64;
    // A day the month does not have (February 30) comes back as another date.
    if format_utc(secs) != text {
        return Err(bad());
    }
    Ok(secs)
}

/// Seconds since the epoch as [`parse_utc`] reads them.
pub fn format_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}

/// Days since 1970-01-01 of a date in the proleptic Gregorian calendar (H. Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { yoe + era * 400 + 1 } else { yoe + era * 400 }, m, d)
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
    /// What its catalog must be signed with: the store keys this Harnix carries.
    keys: Vec<PublicKey>,
}

impl Store {
    pub fn new(url: &str, keys: Vec<PublicKey>) -> Result<Self, String> {
        let url = Url::parse(url).map_err(|e| format!("the store `{url}` is not a URL: {e}"))?;
        check_scheme(&url)?;
        Ok(Self { url, keys })
    }

    pub fn url(&self) -> &str {
        self.url.as_str()
    }

    /// The catalog, as the store signed it ([`Index::verify`]).
    pub async fn index(&self) -> Result<Index, String> {
        Index::verify(&get(&self.url, MAX_INDEX).await?, &self.keys, SystemTime::now())
    }

    /// The version to install: `version`, or the newest one of this Harnix's `harnix_api` that the
    /// store has not withdrawn ([`CatalogApp::withdrawn`]) — and its artifact for the engine
    /// `fingerprint`, or why there is none. A withdrawn version is not installed by name either.
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
                if let Some(why) = app.withdrawn(wanted) {
                    return Err(format!("`{id}` {wanted} is not installed from this store: {why}"));
                }
                v
            }
            None => app
                .versions
                .iter()
                .filter(|v| v.harnix_api == HARNIX_API && app.withdrawn(&v.version).is_none())
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
/// engine has a build of each, and the newest it could install; with `installed` (id → version,
/// what is installed here), the version installed of each, and why the store withdrew it if it
/// did (`installed_revoked`).
pub fn view(index: &Index, fingerprint: &str, installed: &BTreeMap<String, String>) -> Value {
    let apps: Vec<Value> = index
        .apps
        .iter()
        .map(|app| {
            let mut versions: Vec<&CatalogVersion> = app.versions.iter().collect();
            versions.sort_by(|a, b| compare_versions(&b.version, &a.version));
            let offered = |v: &CatalogVersion| v.harnix_api == HARNIX_API && app.withdrawn(&v.version).is_none();
            let installable = |v: &CatalogVersion| {
                offered(v)
                    && (v.artifacts.contains_key(fingerprint)
                        || v.artifacts.contains_key(ANY_ENGINE))
            };
            let latest = versions.iter().find(|v| offered(v));
            let mut entry = json!({
                "id": app.id,
                "title": app.title,
                "publisher": app.publisher,
                "description": app.description,
                "min_version": app.min_version,
                "latest": latest.map(|v| v.version.clone()),
                "installable": latest.is_some_and(|v| installable(v)),
                "capabilities": latest.map(|v| v.capabilities.clone()),
                "versions": versions.iter().map(|v| json!({
                    "version": v.version,
                    "harnix_api": v.harnix_api,
                    "installable": installable(v),
                    "revoked": app.withdrawn(&v.version),
                    "engines": v.artifacts.keys().collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
            });
            if let Some(version) = installed.get(&app.id) {
                entry["installed"] = json!(version);
                entry["installed_revoked"] = json!(app.withdrawn(version));
            }
            entry
        })
        .collect();
    json!({"name": index.name, "expires": index.expires, "apps": apps})
}

fn check_scheme(url: &Url) -> Result<(), String> {
    match url.scheme() {
        "https" | "file" => Ok(()),
        other => Err(format!(
            "`{url}` is {other}://, and a store is https:// (or file:// on this machine)"
        )),
    }
}

/// The client a store is read with: `https://` only, a redirect included. reqwest's own policy
/// follows a redirect from `https://` to `http://`, which would hand the download to anyone on
/// the path; so it is replaced, not left to its default.
fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .https_only(true)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            match follows(attempt.url(), attempt.previous().len()) {
                Ok(()) => attempt.follow(),
                Err(why) => attempt.error(why),
            }
        }))
        .build()
        .map_err(|e| e.to_string())
}

/// Whether a redirect to `next`, after `hops` requests, is followed.
fn follows(next: &Url, hops: usize) -> Result<(), String> {
    if next.scheme() != "https" {
        return Err(format!(
            "the store redirected to `{next}`, and a store is read over https:// only"
        ));
    }
    if hops > MAX_REDIRECTS {
        return Err(format!("the store redirected more than {MAX_REDIRECTS} times"));
    }
    Ok(())
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
    let client = client()?;
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
        let store = Store::new("file:///srv/store/index.json", vec![]).unwrap();
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
        let view = view(&index(fp), fp, &BTreeMap::new());
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
        assert!(Store::new("http://store.example.com/index.json", vec![])
            .unwrap_err()
            .contains("https://"));
        assert!(Store::new("https://store.example.com/index.json", vec![]).is_ok());
        let store = Store::new("file:///srv/store/index.json", vec![]).unwrap();
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
        let store = Store::new(index_url.as_str(), vec![]).unwrap();
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

    fn key(byte: u8) -> SecretKey {
        SecretKey::parse(&hex::encode([byte; 32])).unwrap()
    }

    /// A catalog as a store publishes it: expiring at `expires`, signed with `key`.
    fn signed(expires: &str, key: &SecretKey) -> Vec<u8> {
        let mut catalog = serde_json::to_value(index("fp")).unwrap();
        catalog["expires"] = json!(expires);
        sign_catalog(&mut catalog, key).unwrap();
        serde_json::to_vec_pretty(&catalog).unwrap()
    }

    fn at(text: &str) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(parse_utc(text).unwrap())
    }

    #[test]
    fn a_catalog_is_believed_only_signed_by_the_store_and_until_it_expires() {
        let store = key(2);
        let keys = [store.public()];
        let now = at("2026-11-09T12:00:00Z");
        let bytes = signed("2026-11-10T00:00:00Z", &store);
        let verified = Index::verify(&bytes, &keys, now).unwrap();
        assert_eq!(verified.apps[0].id, "acme.notes");

        // The same catalog with its keys in another order and other whitespace is the same catalog.
        let mut reordered: serde_json::Map<String, Value> = serde_json::Map::new();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        for (k, v) in value.as_object().unwrap().iter().rev() {
            reordered.insert(k.clone(), v.clone());
        }
        assert!(Index::verify(&serde_json::to_vec(&Value::Object(reordered)).unwrap(), &keys, now).is_ok());

        // Expired: an old copy, however well signed.
        let err = Index::verify(&bytes, &keys, at("2026-11-10T00:00:00Z")).unwrap_err();
        assert!(err.contains("expired at 2026-11-10T00:00:00Z"), "{err}");
        // Not signed at all.
        let mut unsigned: Value = serde_json::from_slice(&bytes).unwrap();
        unsigned.as_object_mut().unwrap().remove("signature");
        let err = Index::verify(&serde_json::to_vec(&unsigned).unwrap(), &keys, now).unwrap_err();
        assert!(err.contains("not signed"), "{err}");
        // Signed by a store this Harnix does not carry.
        let err = Index::verify(&signed("2026-11-10T00:00:00Z", &key(9)), &keys, now).unwrap_err();
        assert!(err.contains("a key this Harnix does not carry"), "{err}");
        // Changed after it was signed: a revocation dropped, an expiry pushed back.
        for edit in [
            |v: &mut Value| v["apps"][0]["versions"][0]["revoked"] = json!("a hole"),
            |v: &mut Value| v["expires"] = json!("2099-01-01T00:00:00Z"),
        ] {
            let mut changed: Value = serde_json::from_slice(&bytes).unwrap();
            edit(&mut changed);
            let err = Index::verify(&serde_json::to_vec(&changed).unwrap(), &keys, now).unwrap_err();
            assert!(err.contains("does not verify"), "{err}");
        }
        // No expiry is no catalog.
        let mut forever = serde_json::to_value(index("fp")).unwrap();
        sign_catalog(&mut forever, &store).unwrap();
        let err = Index::verify(&serde_json::to_vec(&forever).unwrap(), &keys, now).unwrap_err();
        assert!(err.contains("no expiry"), "{err}");
    }

    #[test]
    fn a_withdrawn_version_is_not_installed_and_is_flagged_where_it_is() {
        let fp = "fp";
        let store = Store::new("file:///srv/store/index.json", vec![]).unwrap();
        let mut index = index(fp);
        // 1.2.0 revoked; nothing before 1.0.0 supported, which withdraws an older one installed.
        index.apps[0].versions[2].revoked = Some("it leaks the token".into());
        index.apps[0].versions[1].revoked = Some("never built for this engine".into());
        index.apps[0].min_version = Some("1.0.0".into());
        let err = store.resolve(&index, "acme.notes", Some("1.2.0"), fp).unwrap_err();
        assert!(err.contains("the store revoked it: it leaks the token"), "{err}");
        // The newest is the newest not withdrawn.
        let r = store.resolve(&index, "acme.notes", None, fp).unwrap();
        assert_eq!(r.version.version, "1.3.0-rc.1");
        index.apps[0].min_version = Some("1.4.0".into());
        let err = store.resolve(&index, "acme.notes", Some("1.3.0-rc.1"), fp).unwrap_err();
        assert!(err.contains("supports `acme.notes` from 1.4.0 on"), "{err}");
        index.apps[0].min_version = Some("1.0.0".into());

        let installed = BTreeMap::from([("acme.notes".to_string(), "1.2.0".to_string())]);
        let shown = view(&index, fp, &installed);
        let app = &shown["apps"][0];
        assert_eq!(app["installed"], json!("1.2.0"));
        assert_eq!(app["installed_revoked"], json!("the store revoked it: it leaks the token"));
        let v120 = app["versions"].as_array().unwrap().iter().find(|v| v["version"] == json!("1.2.0")).unwrap();
        assert_eq!(v120["installable"], json!(false));
        let older = BTreeMap::from([("acme.notes".to_string(), "0.9.0".to_string())]);
        assert_eq!(
            view(&index, fp, &older)["apps"][0]["installed_revoked"],
            json!("the store supports `acme.notes` from 1.0.0 on")
        );
        let fine = BTreeMap::from([("acme.notes".to_string(), "1.0.0".to_string())]);
        assert_eq!(view(&index, fp, &fine)["apps"][0]["installed_revoked"], Value::Null);
    }

    #[tokio::test]
    async fn a_store_is_read_over_https_and_redirected_only_to_https() {
        assert!(follows(&Url::parse("https://cdn.example.com/x").unwrap(), 1).is_ok());
        let err = follows(&Url::parse("http://cdn.example.com/x").unwrap(), 1).unwrap_err();
        assert!(err.contains("https:// only"), "{err}");
        assert!(follows(&Url::parse("https://cdn.example.com/x").unwrap(), MAX_REDIRECTS + 1).is_err());
        // The client itself refuses plain http, before it connects to anything.
        let err = client().unwrap().get("http://127.0.0.1:9/index.json").send().await.unwrap_err();
        assert!(err.is_builder(), "{err}");
    }

    #[test]
    fn expiry_is_written_one_way() {
        assert_eq!(parse_utc("1970-01-01T00:00:00Z"), Ok(0));
        assert_eq!(parse_utc("2026-11-09T12:34:56Z").map(format_utc).as_deref(), Ok("2026-11-09T12:34:56Z"));
        assert_eq!(format_utc(parse_utc("2028-02-29T23:59:59Z").unwrap() + 1), "2028-03-01T00:00:00Z");
        for bad in ["2026-02-30T00:00:00Z", "2026-11-09 12:00:00Z", "2026-11-09T12:00:00+01:00", "", "2026-13-01T00:00:00Z"] {
            assert!(parse_utc(bad).is_err(), "{bad}");
        }
    }
}
