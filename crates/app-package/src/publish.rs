//! The publisher's and the store's side of a package: signing it, countersigning it, and listing
//! a directory of packages as a store's `index.json`, signed. What `harnix-app sign`,
//! `countersign`, `index` and `sign-index` do; nothing in a running Harnix calls these.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::path::Path;

use serde_json::{json, Value};

use crate::manifest::{Manifest, MANIFEST};
use crate::signature::{self, sha256, SecretKey, Signatures};
use crate::store::{
    compare_versions, format_utc, sign_catalog, Artifact, CatalogApp, CatalogVersion, Index,
    ANY_ENGINE, INDEX_FORMAT,
};

/// Every entry of a zip, in its order.
fn entries(path: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("{} is not a package (a zip): {e}", path.display()))?;
    let mut out = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("reading the package: {e}"))?;
        if entry.is_dir() {
            continue;
        }
        let (name, declared) = (entry.name().to_string(), entry.size());
        let bytes = crate::package::read_entry(&mut entry, declared, &name)?;
        out.push((name, bytes));
    }
    Ok(out)
}

/// Write `entries` as a zip at `out`, through a temporary file beside it (`out` may be the zip
/// they were read from).
fn write(out: &Path, entries: &[(String, Vec<u8>)]) -> Result<(), String> {
    let tmp = out.with_extension("hxapp.tmp");
    {
        let file = File::create(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in entries {
            zip.start_file(name.as_str(), options)
                .map_err(|e| e.to_string())?;
            zip.write_all(bytes).map_err(|e| e.to_string())?;
        }
        zip.finish().map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, out).map_err(|e| format!("{}: {e}", out.display()))
}

fn manifest_of(entries: &[(String, Vec<u8>)]) -> Result<Manifest, String> {
    let (_, bytes) = entries
        .iter()
        .find(|(n, _)| n == MANIFEST)
        .ok_or_else(|| format!("the package has no `{MANIFEST}`"))?;
    Manifest::parse(&String::from_utf8_lossy(bytes))
}

fn hashes(entries: &[(String, Vec<u8>)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .filter(|(n, _)| n != signature::FILE)
        .map(|(n, b)| (n.clone(), sha256(b)))
        .collect()
}

/// The publisher signs every file of the package at `package`, writing the signed package to
/// `out`. A `SIGNATURES` already there is replaced.
pub fn sign(package: &Path, key: &SecretKey, out: &Path) -> Result<Signatures, String> {
    let mut entries = entries(package)?;
    let manifest = manifest_of(&entries)?;
    let app = (manifest.app.id.as_str(), manifest.app.version.as_str());
    let signatures = Signatures::sign(app, &manifest.app.publisher, key, hashes(&entries));
    entries.retain(|(n, _)| n != signature::FILE);
    entries.push((signature::FILE.into(), signatures.to_json().into_bytes()));
    write(out, &entries)?;
    Ok(signatures)
}

/// The store adds the compiled components it built (`artifacts`: each placed at
/// `wasm/<its file name>`) and countersigns the package. The publisher's signature must verify
/// first.
pub fn countersign(
    package: &Path,
    key: &SecretKey,
    artifacts: &[&Path],
    out: &Path,
) -> Result<Signatures, String> {
    let mut entries = entries(package)?;
    let manifest = manifest_of(&entries)?;
    let app = (manifest.app.id.as_str(), manifest.app.version.as_str());
    // The store's policy: what it vouches for reaches the internet, not the machine it is
    // installed on. An application that needs a service there is a trusted publisher's to ship.
    if let Some(host) = manifest
        .capabilities
        .net
        .iter()
        .find(|h| crate::manifest::local_host(h))
    {
        return Err(format!(
            "the store does not countersign `{}`: its `net` lists `{host}`, which is this machine \
             or its local network",
            manifest.app.id
        ));
    }
    let (_, existing) = entries
        .iter()
        .find(|(n, _)| n == signature::FILE)
        .ok_or("the package is not signed by its publisher; `harnix-app sign` it first")?;
    let existing = Signatures::parse(&String::from_utf8_lossy(existing))?;
    entries.retain(|(n, _)| n != signature::FILE);
    for artifact in artifacts {
        let name = artifact
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| format!("{} has no file name", artifact.display()))?;
        let bytes = std::fs::read(artifact).map_err(|e| format!("{}: {e}", artifact.display()))?;
        let path = format!("wasm/{name}");
        entries.retain(|(n, _)| *n != path);
        entries.push((path, bytes));
    }
    let countersigned = existing.countersign(app, key, hashes(&entries))?;
    entries.push((signature::FILE.into(), countersigned.to_json().into_bytes()));
    write(out, &entries)?;
    Ok(countersigned)
}

/// The engines a package has a compiled component for (by the artifacts' file names), or
/// [`ANY_ENGINE`] when it declares no component.
fn engines(manifest: &Manifest, entries: &[(String, Vec<u8>)]) -> Vec<String> {
    let Some(component) = &manifest.component else {
        return vec![ANY_ENGINE.to_string()];
    };
    let prefix = format!("wasm/{}.", component.name);
    entries
        .iter()
        .filter_map(|(n, _)| n.strip_prefix(&prefix)?.strip_suffix(".cwasm"))
        .map(str::to_string)
        .collect()
}

/// How long a catalog is believed when its store does not say: long enough that a store published
/// by hand is not refused next week, short enough that an old copy does not outlive a revocation
/// for long.
pub const CATALOG_VALID: std::time::Duration = std::time::Duration::from_secs(14 * 24 * 3600);

/// List every `.hxapp` under `store/packages/` as the store's index, signed with `key` and valid
/// for `valid` from now, written to `store/index.json`. Each package is listed under every engine
/// it has a component for; the URLs are relative to the index.
pub fn index(
    store: &Path,
    name: Option<&str>,
    key: &SecretKey,
    valid: std::time::Duration,
) -> Result<Index, String> {
    let packages = store.join("packages");
    let mut files: Vec<_> = std::fs::read_dir(&packages)
        .map_err(|e| format!("{}: {e}", packages.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "hxapp"))
        .collect();
    files.sort();
    let mut apps: BTreeMap<String, CatalogApp> = BTreeMap::new();
    for path in files {
        let file_name = path.file_name().unwrap().to_string_lossy().to_string();
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let entries = entries(&path)?;
        let manifest = manifest_of(&entries).map_err(|e| format!("{file_name}: {e}"))?;
        let signatures: Value = entries
            .iter()
            .find(|(n, _)| n == signature::FILE)
            .map(|(_, b)| Signatures::parse(&String::from_utf8_lossy(b)))
            .transpose()
            .map_err(|e| format!("{file_name}: {e}"))?
            .map(|s| json!({"publisher": s.publisher.key, "store": s.store.map(|s| s.key)}))
            .unwrap_or(Value::Null);
        let app = apps
            .entry(manifest.app.id.clone())
            .or_insert_with(|| CatalogApp {
                id: manifest.app.id.clone(),
                title: None,
                publisher: manifest.app.publisher.clone(),
                description: None,
                min_version: None,
                versions: Vec::new(),
            });
        // The newest version's title and description are the application's.
        let newest = app
            .versions
            .iter()
            .all(|v| compare_versions(&manifest.app.version, &v.version).is_ge());
        if newest {
            app.title = manifest.app.title.clone();
            app.description = manifest.app.description.clone();
        }
        let position = app
            .versions
            .iter()
            .position(|v| v.version == manifest.app.version);
        let version = match position {
            Some(i) => &mut app.versions[i],
            None => {
                let caps = &manifest.capabilities;
                app.versions.push(CatalogVersion {
                    version: manifest.app.version.clone(),
                    harnix_api: manifest.app.harnix_api,
                    capabilities: json!({
                        "net": caps.net, "env": caps.env, "storage_mb": caps.storage_mb, "hub": caps.hub, "fs": caps.fs,
                    }),
                    signatures: signatures.clone(),
                    revoked: None,
                    artifacts: BTreeMap::new(),
                });
                app.versions.last_mut().unwrap()
            }
        };
        for engine in engines(&manifest, &entries) {
            version.artifacts.insert(
                engine,
                Artifact {
                    url: format!("packages/{file_name}"),
                    sha256: sha256(&bytes),
                    size: Some(bytes.len() as u64),
                },
            );
        }
    }
    let mut apps: Vec<CatalogApp> = apps.into_values().collect();
    for app in &mut apps {
        app.versions
            .sort_by(|a, b| compare_versions(&a.version, &b.version));
    }
    let index = Index {
        format: INDEX_FORMAT.into(),
        name: name.map(str::to_string),
        expires: String::new(),
        apps,
        signature: None,
    };
    let out = store.join("index.json");
    std::fs::write(&out, serde_json::to_string_pretty(&index).unwrap())
        .map_err(|e| format!("{}: {e}", out.display()))?;
    sign_index(&out, key, std::time::SystemTime::now() + valid)
}

/// Sign the catalog at `path` as the store, valid until `expires`, in place: what a store runs
/// after it edits its catalog (a version revoked, a `min_version` raised) and each time it
/// republishes it before it expires. Whatever else is in it is kept as it is.
pub fn sign_index(
    path: &Path,
    key: &SecretKey,
    expires: std::time::SystemTime,
) -> Result<Index, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Index::parse(&bytes)?;
    let mut catalog: Value = serde_json::from_slice(&bytes)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let secs = expires
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "an expiry before 1970".to_string())?
        .as_secs();
    catalog["expires"] = json!(format_utc(secs));
    sign_catalog(&mut catalog, key)?;
    let text = serde_json::to_string_pretty(&catalog).expect("plain data serializes");
    std::fs::write(path, &text).map_err(|e| format!("{}: {e}", path.display()))?;
    Index::parse(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::tests::{zip_of, FP, MANIFEST_TEXT};
    use crate::signature::{verify, Keys, Trust};

    fn key(byte: u8) -> SecretKey {
        SecretKey::parse(&hex::encode([byte; 32])).unwrap()
    }

    fn package(dir: &Path) -> std::path::PathBuf {
        zip_of(
            dir,
            "pkg",
            &[
                (MANIFEST, MANIFEST_TEXT.as_bytes()),
                ("wasm/tools.wasm", b"portable"),
                ("ui/card.js", b"export function register() {}"),
            ],
        )
    }

    #[test]
    fn signed_countersigned_and_listed() {
        let dir = tempfile::tempdir().unwrap();
        let (publisher, store) = (key(1), key(2));
        let pkg = package(dir.path());
        sign(&pkg, &publisher, &pkg).unwrap();
        let (files, signatures) = signature::of_zip(&pkg).unwrap();
        let only_publisher = Keys {
            trusted: [("acme".to_string(), vec![publisher.public()])].into(),
            ..Keys::default()
        };
        let verified = verify(
            signatures.as_ref(),
            &files,
            ("acme.probe", "1.0.0"),
            "acme",
            &only_publisher,
        )
        .unwrap();
        assert_eq!(verified.trust, Trust::Trusted);

        // The store compiles the component for an engine and countersigns.
        let cwasm = dir.path().join(format!("tools.{FP}.cwasm"));
        std::fs::write(&cwasm, b"compiled").unwrap();
        let store_dir = dir.path().join("store");
        std::fs::create_dir_all(store_dir.join("packages")).unwrap();
        let out = store_dir.join("packages/acme.probe-1.0.0.hxapp");
        countersign(&pkg, &store, &[cwasm.as_path()], &out).unwrap();
        let (files, signatures) = signature::of_zip(&out).unwrap();
        assert!(files.contains_key(&format!("wasm/tools.{FP}.cwasm")));
        let store_keys = Keys {
            store: vec![store.public()],
            ..Keys::default()
        };
        let verified = verify(
            signatures.as_ref(),
            &files,
            ("acme.probe", "1.0.0"),
            "acme",
            &store_keys,
        )
        .unwrap();
        assert_eq!(verified.trust, Trust::Store);
        // Signed by the publisher alone, the compiled component is nobody's.
        assert!(verify(
            signatures.as_ref(),
            &files,
            ("acme.probe", "1.0.0"),
            "acme",
            &only_publisher
        )
        .is_err());

        let index = index(&store_dir, Some("test"), &store, CATALOG_VALID).unwrap();
        let version = &index.apps[0].versions[0];
        assert_eq!(index.apps[0].id, "acme.probe");
        assert_eq!(version.artifacts.keys().collect::<Vec<_>>(), [FP]);
        assert_eq!(
            version.artifacts[FP].sha256,
            sha256(&std::fs::read(&out).unwrap())
        );
        assert_eq!(version.signatures["store"], json!(store.public().hex()));
        // Signed by the store, and believed until it expires.
        let bytes = std::fs::read(store_dir.join("index.json")).unwrap();
        let now = std::time::SystemTime::now();
        let written = Index::verify(&bytes, &[store.public()], now).unwrap();
        assert!(Index::verify(&bytes, &[store.public()], now + CATALOG_VALID).is_err());
        assert!(Index::verify(&bytes, &[publisher.public()], now).is_err());
        assert_eq!(
            written.apps[0].versions[0].artifacts[FP].url,
            "packages/acme.probe-1.0.0.hxapp"
        );
    }

    #[test]
    fn a_package_that_reaches_this_machine_is_not_countersigned() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = format!("{MANIFEST_TEXT}\n[capabilities]\nnet = [\"127.0.0.1\"]\n");
        let pkg = zip_of(
            dir.path(),
            "local",
            &[
                (MANIFEST, manifest.as_bytes()),
                ("wasm/tools.wasm", b"portable"),
                ("ui/card.js", b"export function register() {}"),
            ],
        );
        sign(&pkg, &key(1), &pkg).unwrap();
        let err = countersign(&pkg, &key(2), &[], &dir.path().join("out.hxapp")).unwrap_err();
        assert!(err.contains("`127.0.0.1`") && err.contains("this machine"), "{err}");
    }

    #[test]
    fn a_package_its_publisher_did_not_sign_is_not_countersigned() {
        let dir = tempfile::tempdir().unwrap();
        let pkg = package(dir.path());
        let out = dir.path().join("out.hxapp");
        assert!(countersign(&pkg, &key(2), &[], &out)
            .unwrap_err()
            .contains("not signed by its publisher"));
    }
}
