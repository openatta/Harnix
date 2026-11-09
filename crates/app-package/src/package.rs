//! The `.hxapp`: a zip, read before anything of it is written, and unpacked whole.
//!
//! ```text
//! harnix-app.toml                     the manifest (manifest.rs)
//! ui/…                                ES modules, as a package's [[ui]]
//! agent/plugin.toml, agent/…          an Agix package; agent/skills/<name>/SKILL.md beside it
//! wasm/<name>.wasm                    the component, portable (optional; the product cannot run it)
//! wasm/<name>.<fingerprint>.cwasm     the component compiled for one engine — one per engine
//! ```
//!
//! What is refused before a byte is unpacked: an entry that is absolute, climbs out (`..`), is a
//! link, or is not one of the parts above; a package that is larger than [`MAX_UNPACKED`] or has
//! more than [`MAX_ENTRIES`] entries; a manifest that does not read; a component with no artifact
//! for this engine (named, with the engines the package does have one for); an interface module
//! the manifest names and the package does not have; an `agent/` without its `plugin.toml`.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::manifest::{safe_relative, Manifest, MANIFEST};

/// Everything unpacked, together.
pub const MAX_UNPACKED: u64 = 256 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 10_000;

/// The top level of a package.
const PARTS: &[&str] = &["ui", "agent", "wasm"];
/// Beside the manifest, and read by nobody: a package's own description of itself.
const NOTES: &[&str] = &["README.md", "LICENSE", "SIGNATURES"];

/// A package that read, and what of it this engine would run.
#[derive(Debug, Clone)]
pub struct Inspected {
    pub manifest: Manifest,
    /// The artifact for this engine: its path in the package.
    pub artifact: Option<String>,
    /// `agent/`'s Agix package: its `[plugin]` name and version.
    pub agent: Option<AgentPackage>,
    /// `agent/skills/<name>/` directories.
    pub skills: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentPackage {
    pub name: String,
    pub version: String,
}

/// An entry, read whole, and refused if it inflates to more than the size it declares. What
/// [`inspect`] adds up to bound a package is the sizes entries declare, and an inflater stops
/// only where the compressed data does: so every read of an entry stops one byte past what it
/// declared, and a package whose few kilobytes inflate to gigabytes is refused at that byte rather
/// than held in memory or written to disk.
pub(crate) fn read_entry(entry: &mut impl Read, declared: u64, name: &str) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::with_capacity(declared.min(1 << 20) as usize);
    entry
        .take(declared.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| format!("`{name}`: {e}"))?;
    if bytes.len() as u64 > declared {
        return Err(oversized(name, declared));
    }
    Ok(bytes)
}

fn oversized(name: &str, declared: u64) -> String {
    format!("the package's `{name}` inflates to more than the {declared} bytes it declares")
}

/// Read and check a package against an engine whose fingerprint is `fingerprint`.
pub fn inspect(path: &Path, fingerprint: &str) -> Result<Inspected, String> {
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("{} is not a package (a zip): {e}", path.display()))?;
    if archive.len() > MAX_ENTRIES {
        return Err(format!(
            "the package has {} entries, more than {MAX_ENTRIES}",
            archive.len()
        ));
    }
    let mut names = BTreeSet::new();
    let mut total = 0u64;
    for i in 0..archive.len() {
        let entry = archive
            .by_index_raw(i)
            .map_err(|e| format!("reading the package: {e}"))?;
        let name = entry.name().trim_end_matches('/').to_string();
        if name.is_empty() {
            continue;
        }
        if !safe_relative(&name) {
            return Err(format!(
                "the package's entry `{}` leaves the package",
                entry.name()
            ));
        }
        if entry.is_symlink() {
            return Err(format!(
                "the package's entry `{name}` is a link; a package holds files"
            ));
        }
        let top = name.split('/').next().unwrap_or_default();
        let known = if name.contains('/') || entry.is_dir() {
            PARTS.contains(&top)
        } else {
            name == MANIFEST || NOTES.contains(&top)
        };
        if !known {
            return Err(format!(
                "the package's entry `{name}` is not part of an application package (it has \
                 {MANIFEST}, ui/, agent/, wasm/)"
            ));
        }
        total = total.saturating_add(entry.size());
        if total > MAX_UNPACKED {
            return Err(format!(
                "the package unpacks to more than {} MiB",
                MAX_UNPACKED >> 20
            ));
        }
        if !entry.is_dir() {
            names.insert(name);
        }
    }

    let read_text = |archive: &mut zip::ZipArchive<File>, name: &str| -> Result<String, String> {
        let mut entry = archive
            .by_name(name)
            .map_err(|_| format!("the package has no `{name}`"))?;
        let declared = entry.size();
        String::from_utf8(read_entry(&mut entry, declared, name)?)
            .map_err(|_| format!("`{name}` is not UTF-8 text"))
    };
    let manifest = Manifest::parse(&read_text(&mut archive, MANIFEST)?)?;

    let artifact = match &manifest.component {
        None => {
            if let Some(stray) = names.iter().find(|n| n.starts_with("wasm/")) {
                return Err(format!(
                    "the package carries `{stray}`, and its manifest declares no [[component]]"
                ));
            }
            None
        }
        Some(component) => {
            let prefix = format!("wasm/{}.", component.name);
            let wanted = format!("{prefix}{fingerprint}.cwasm");
            if names.contains(&wanted) {
                Some(wanted)
            } else {
                let offered: Vec<&str> = names
                    .iter()
                    .filter_map(|n| n.strip_prefix(&prefix)?.strip_suffix(".cwasm"))
                    .collect();
                return Err(if offered.is_empty() {
                    format!(
                        "the package has no compiled `{}` component (`{wanted}`); compile it for this \
                         engine, `{fingerprint}`",
                        component.name
                    )
                } else {
                    format!(
                        "the package's `{}` component was compiled for {} and this Harnix's engine is \
                         `{fingerprint}`: it needs an artifact built for this Harnix release",
                        component.name,
                        offered.iter().map(|o| format!("`{o}`")).collect::<Vec<_>>().join(", ")
                    )
                });
            }
        }
    };

    for entry in &manifest.ui {
        let file = format!("ui/{}", entry.module);
        if !names.contains(&file) {
            return Err(format!(
                "the manifest names the interface module `{}`, and the package has no `{file}`",
                entry.module
            ));
        }
    }

    for surface in &manifest.surfaces {
        let Some(page) = &surface.page else { continue };
        let file = format!("ui/{page}");
        if !names.contains(&file) {
            return Err(format!(
                "the manifest's {} surface `{}` is the page `{page}`, and the package has no `{file}`",
                surface.point,
                surface.id()
            ));
        }
    }

    let has_agent = names.iter().any(|n| n.starts_with("agent/"));
    let agent = if has_agent {
        let text = read_text(&mut archive, "agent/plugin.toml").map_err(|_| {
            "the package's agent/ has no plugin.toml: it is an Agix package".to_string()
        })?;
        Some(agent_package(&text)?)
    } else {
        None
    };
    let skills: Vec<String> = names
        .iter()
        .filter_map(|n| {
            let rest = n.strip_prefix("agent/skills/")?;
            let (skill, file) = rest.split_once('/')?;
            (file == "SKILL.md").then(|| skill.to_string())
        })
        .collect();
    for skill in &skills {
        if !crate::manifest::valid_name(skill) {
            return Err(format!(
                "the skill `{skill}` (agent/skills/) is lowercase letters, digits and `-`"
            ));
        }
    }

    Ok(Inspected {
        manifest,
        artifact,
        agent,
        skills,
    })
}

/// Only the manifest of the package at `path`: for a tool that looks at a package compiled for
/// another engine, which [`inspect`] would refuse.
pub fn manifest_of(path: &Path) -> Result<Manifest, String> {
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("{} is not a package (a zip): {e}", path.display()))?;
    let mut entry = archive
        .by_name(MANIFEST)
        .map_err(|_| format!("the package has no `{MANIFEST}`"))?;
    let declared = entry.size();
    let text = String::from_utf8(read_entry(&mut entry, declared, MANIFEST)?)
        .map_err(|_| format!("`{MANIFEST}` is not UTF-8 text"))?;
    Manifest::parse(&text)
}

/// What [`inspect`] says of a package already unpacked at `root`: the manifest, the agent
/// package, the skills — to put an installed version back in place (an update that failed).
pub fn inspect_dir(root: &Path) -> Result<Inspected, String> {
    let read = |name: &str| {
        std::fs::read_to_string(root.join(name))
            .map_err(|e| format!("{}: {e}", root.join(name).display()))
    };
    let manifest = Manifest::parse(&read(MANIFEST)?)?;
    let agent = if root.join("agent").is_dir() {
        Some(agent_package(&read("agent/plugin.toml")?)?)
    } else {
        None
    };
    let mut skills: Vec<String> = std::fs::read_dir(root.join("agent").join("skills"))
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().join("SKILL.md").is_file())
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    skills.sort();
    let artifact = manifest.component.as_ref().and_then(|c| {
        let prefix = format!("{}.", c.name);
        std::fs::read_dir(root.join("wasm"))
            .ok()?
            .flatten()
            .find_map(|e| {
                let name = e.file_name().into_string().ok()?;
                (name.starts_with(&prefix) && name.ends_with(".cwasm"))
                    .then(|| format!("wasm/{name}"))
            })
    });
    Ok(Inspected {
        manifest,
        artifact,
        agent,
        skills,
    })
}

/// `[plugin] name` and `version` of an Agix manifest: what `plugin.install` is asked to install
/// under. Nothing else of it is read here; the rest is the engine's.
fn agent_package(text: &str) -> Result<AgentPackage, String> {
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| {
        format!("agent/plugin.toml is not valid TOML: {}", e.message())
    })?;
    let field = |key: &str| {
        table
            .get("plugin")
            .and_then(|p| p.get(key))
            .and_then(toml::Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| format!("agent/plugin.toml has no `[plugin] {key}`"))
    };
    Ok(AgentPackage {
        name: field("name")?,
        version: field("version")?,
    })
}

/// Unpack the whole package into `dest`, which must not exist. Checked by [`inspect`] first.
pub fn extract(path: &Path, dest: &Path) -> Result<(), String> {
    if dest.exists() {
        return Err(format!("{} exists already", dest.display()));
    }
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("reading the package: {e}"))?;
    std::fs::create_dir_all(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("reading the package: {e}"))?;
        let name = entry.name().trim_end_matches('/').to_string();
        if name.is_empty() {
            continue;
        }
        if !safe_relative(&name) || entry.is_symlink() {
            return Err(format!("the package's entry `{name}` is refused"));
        }
        let out = dest.join(&name);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let declared = entry.size();
        let mut writer = File::create(&out).map_err(|e| format!("{}: {e}", out.display()))?;
        let copied = std::io::copy(&mut (&mut entry).take(declared.saturating_add(1)), &mut writer)
            .map_err(|e| format!("{}: {e}", out.display()))?;
        if copied > declared {
            return Err(oversized(&name, declared));
        }
    }
    Ok(())
}

/// Zip the directory `dir` into `out` with its files at the zip's root: an Agix package from an
/// unpacked `agent/`, for `plugin.install`.
pub fn zip_dir(dir: &Path, out: &Path) -> Result<(), String> {
    let mut files = Vec::new();
    walk(dir, dir, &mut files).map_err(|e| format!("{}: {e}", dir.display()))?;
    let file = File::create(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (relative, path) in files {
        zip.start_file(relative, options)
            .map_err(|e| e.to_string())?;
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        zip.write_all(&bytes).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

/// Every file under `dir`, relative to `root` with `/` separators, in a stable order.
pub fn walk(
    root: &Path,
    dir: &Path,
    out: &mut Vec<(String, PathBuf)>,
) -> std::io::Result<()> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<_, _>>()?;
    entries.sort();
    for path in entries {
        let meta = std::fs::symlink_metadata(&path)?;
        if meta.is_dir() {
            walk(root, &path, out)?;
        } else if meta.is_file() {
            let relative = path
                .strip_prefix(root)
                .expect("under root")
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            out.push((relative, path));
        }
    }
    Ok(())
}

/// What goes into a package beside the application's directory: the component, compiled.
pub struct Artifact<'a> {
    pub fingerprint: &'a str,
    pub cwasm: &'a [u8],
    /// The portable component, shipped beside it when given.
    pub wasm: Option<&'a [u8]>,
}

/// Pack an application directory — its `harnix-app.toml`, `ui/`, `agent/` — and its compiled
/// component into a `.hxapp` at `out`. Anything else in the directory (the component's sources) is
/// left out. The manifest is checked first.
pub fn pack(
    app_dir: &Path,
    artifact: Option<Artifact<'_>>,
    out: &Path,
) -> Result<Manifest, String> {
    let manifest_text = std::fs::read_to_string(app_dir.join(MANIFEST))
        .map_err(|e| format!("{}: {e}", app_dir.join(MANIFEST).display()))?;
    let manifest = Manifest::parse(&manifest_text)?;
    let file = File::create(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut put = |name: &str, bytes: &[u8]| -> Result<(), String> {
        zip.start_file(name, options).map_err(|e| e.to_string())?;
        zip.write_all(bytes).map_err(|e| e.to_string())
    };
    put(MANIFEST, manifest_text.as_bytes())?;
    for part in ["ui", "agent"] {
        let dir = app_dir.join(part);
        if !dir.is_dir() {
            continue;
        }
        let mut files = Vec::new();
        walk(app_dir, &dir, &mut files).map_err(|e| format!("{}: {e}", dir.display()))?;
        for (relative, path) in files {
            let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            put(&relative, &bytes)?;
        }
    }
    match (&manifest.component, artifact) {
        (Some(component), Some(artifact)) => {
            put(
                &format!("wasm/{}.{}.cwasm", component.name, artifact.fingerprint),
                artifact.cwasm,
            )?;
            if let Some(wasm) = artifact.wasm {
                put(&format!("wasm/{}.wasm", component.name), wasm)?;
            }
        }
        (Some(component), None) => {
            return Err(format!(
                "the manifest declares the component `{}`, and none was given",
                component.name
            ))
        }
        (None, Some(_)) => {
            return Err("a component was given, and the manifest declares none".into())
        }
        (None, None) => {}
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(manifest)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub const FP: &str = "engine-0.0.0+test+000000000000";

    pub const MANIFEST_TEXT: &str = r#"
[app]
id = "acme.probe"
version = "1.0.0"
harnix_api = 1
publisher = "acme"

[[component]]
name = "tools"
world = "harnix:app/tools-app@0.1.0"

[[ui]]
point = "home.card"
module = "card.js"
api_version = 2
"#;

    /// A zip of `entries` (name, bytes) at `dir/name.hxapp`.
    pub fn zip_of(dir: &Path, name: &str, entries: &[(&str, &[u8])]) -> PathBuf {
        let path = dir.join(format!("{name}.hxapp"));
        let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
        for (n, bytes) in entries {
            zip.start_file(*n, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    type Entries = Vec<(String, Vec<u8>)>;

    fn good(extra: &[(&str, &[u8])]) -> Entries {
        let mut v: Vec<(String, Vec<u8>)> = vec![
            (MANIFEST.into(), MANIFEST_TEXT.as_bytes().to_vec()),
            (format!("wasm/tools.{FP}.cwasm"), b"artifact".to_vec()),
            (
                "ui/card.js".into(),
                b"export function register() {}".to_vec(),
            ),
            (
                "agent/plugin.toml".into(),
                b"[plugin]\nname = \"acme-probe\"\nversion = \"1.0.0\"\napi_version = \"0.1\"\n"
                    .to_vec(),
            ),
            (
                "agent/skills/probe-skill/SKILL.md".into(),
                b"---\nname: probe-skill\n---\n".to_vec(),
            ),
        ];
        v.extend(extra.iter().map(|(n, b)| (n.to_string(), b.to_vec())));
        v
    }

    fn package(dir: &Path, name: &str, entries: Entries) -> PathBuf {
        let refs: Vec<(&str, &[u8])> = entries
            .iter()
            .map(|(n, b)| (n.as_str(), b.as_slice()))
            .collect();
        zip_of(dir, name, &refs)
    }

    #[test]
    fn a_good_package_reads_and_unpacks() {
        let dir = tempfile::tempdir().unwrap();
        let path = package(dir.path(), "good", good(&[("README.md", b"hi")]));
        let inspected = inspect(&path, FP).unwrap();
        assert_eq!(inspected.manifest.app.id, "acme.probe");
        assert_eq!(
            inspected.artifact.as_deref(),
            Some(format!("wasm/tools.{FP}.cwasm").as_str())
        );
        assert_eq!(
            inspected.agent,
            Some(AgentPackage {
                name: "acme-probe".into(),
                version: "1.0.0".into()
            })
        );
        assert_eq!(inspected.skills, ["probe-skill"]);
        let dest = dir.path().join("out");
        extract(&path, &dest).unwrap();
        assert!(dest.join("ui/card.js").is_file());
        assert!(dest.join("agent/skills/probe-skill/SKILL.md").is_file());
        assert!(
            extract(&path, &dest).is_err(),
            "unpacked over an existing directory"
        );

        let agent_zip = dir.path().join("agent.zip");
        zip_dir(&dest.join("agent"), &agent_zip).unwrap();
        let archive = zip::ZipArchive::new(File::open(&agent_zip).unwrap()).unwrap();
        let mut inside: Vec<&str> = archive.file_names().collect();
        inside.sort();
        assert_eq!(inside, ["plugin.toml", "skills/probe-skill/SKILL.md"]);
    }

    #[test]
    fn a_bad_package_is_refused_with_why() {
        let dir = tempfile::tempdir().unwrap();
        let without = |name: &str| {
            good(&[])
                .into_iter()
                .filter(|(n, _)| n != name)
                .collect::<Vec<_>>()
        };
        let cases: Vec<(&str, Entries, String)> = vec![
            (
                "escape",
                good(&[("../evil", b"x")]),
                "leaves the package".into(),
            ),
            (
                "absolute",
                good(&[("/etc/passwd", b"x")]),
                "leaves the package".into(),
            ),
            (
                "stray",
                good(&[("bin/tool", b"x")]),
                "not part of an application package".into(),
            ),
            (
                "other-engine",
                {
                    let mut v = without(&format!("wasm/tools.{FP}.cwasm"));
                    v.push((
                        "wasm/tools.engine-9.9.9+elsewhere+111111111111.cwasm".into(),
                        b"x".to_vec(),
                    ));
                    v
                },
                "compiled for `engine-9.9.9+elsewhere+111111111111`".into(),
            ),
            (
                "no-artifact",
                without(&format!("wasm/tools.{FP}.cwasm")),
                "no compiled `tools` component".into(),
            ),
            ("no-ui", without("ui/card.js"), "has no `ui/card.js`".into()),
            (
                "no-plugin-toml",
                without("agent/plugin.toml"),
                "no plugin.toml".into(),
            ),
            ("no-manifest", without(MANIFEST), format!("no `{MANIFEST}`")),
            (
                "bad-manifest",
                {
                    let mut v = without(MANIFEST);
                    v.push((
                        MANIFEST.into(),
                        MANIFEST_TEXT
                            .replace("[[ui]]", "[capabilities]\nroot = true\n\n[[ui]]")
                            .into_bytes(),
                    ));
                    v
                },
                "unknown capability `root`".into(),
            ),
            (
                "bad-skill",
                good(&[("agent/skills/Bad_Skill/SKILL.md", b"x")]),
                "the skill `Bad_Skill`".into(),
            ),
        ];
        for (name, entries, says) in cases {
            let path = package(dir.path(), name, entries);
            let err = inspect(&path, FP)
                .err()
                .unwrap_or_else(|| panic!("{name} was accepted"));
            assert!(err.contains(&says), "{name}: wanted `{says}`, got: {err}");
        }
        let not_zip = dir.path().join("plain.hxapp");
        std::fs::write(&not_zip, "hello").unwrap();
        assert!(inspect(&not_zip, FP).unwrap_err().contains("not a package"));
    }

    #[test]
    fn packing_takes_the_parts_and_leaves_the_sources() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("app");
        std::fs::create_dir_all(app.join("ui")).unwrap();
        std::fs::create_dir_all(app.join("agent/skills/s")).unwrap();
        std::fs::create_dir_all(app.join("component/src")).unwrap();
        std::fs::write(app.join(MANIFEST), MANIFEST_TEXT).unwrap();
        std::fs::write(app.join("ui/card.js"), "x").unwrap();
        std::fs::write(
            app.join("agent/plugin.toml"),
            "[plugin]\nname = \"p\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        std::fs::write(app.join("agent/skills/s/SKILL.md"), "x").unwrap();
        std::fs::write(app.join("component/src/lib.rs"), "x").unwrap();
        let out = dir.path().join("app.hxapp");
        let artifact = Artifact {
            fingerprint: FP,
            cwasm: b"compiled",
            wasm: Some(b"portable"),
        };
        pack(&app, Some(artifact), &out).unwrap();
        let archive = zip::ZipArchive::new(File::open(&out).unwrap()).unwrap();
        let mut inside: Vec<&str> = archive.file_names().collect();
        inside.sort();
        let cwasm = format!("wasm/tools.{FP}.cwasm");
        assert_eq!(
            inside,
            [
                "agent/plugin.toml",
                "agent/skills/s/SKILL.md",
                MANIFEST,
                "ui/card.js",
                cwasm.as_str(),
                "wasm/tools.wasm"
            ]
        );
        assert!(inspect(&out, FP).is_ok());
        assert!(pack(&app, None, &out)
            .unwrap_err()
            .contains("none was given"));
    }

    /// `entries` deflated, with `liar`'s uncompressed size rewritten to `declared` in its local
    /// header and in the central directory: what a zip bomb looks like to a reader that trusts it.
    pub fn lying_zip(dir: &Path, name: &str, entries: &[(&str, &[u8])], liar: &str, declared: u32) -> PathBuf {
        let path = dir.join(format!("{name}.hxapp"));
        let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (n, bytes) in entries {
            zip.start_file(*n, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        // Each header's name follows its fixed part: 30 bytes in a local header, 46 in the
        // central directory's; the uncompressed size is at 22 and 24 in them.
        let headers = [(&[0x50u8, 0x4b, 0x03, 0x04][..], 30usize, 22usize), (&[0x50, 0x4b, 0x01, 0x02][..], 46, 24)];
        let mut patched = 0;
        for (signature, fixed, at) in headers {
            for i in 0..bytes.len().saturating_sub(fixed) {
                if &bytes[i..i + 4] != signature {
                    continue;
                }
                let name_len_at = if fixed == 30 { 26 } else { 28 };
                let name_len = u16::from_le_bytes([bytes[i + name_len_at], bytes[i + name_len_at + 1]]) as usize;
                if bytes.get(i + fixed..i + fixed + name_len) == Some(liar.as_bytes()) {
                    bytes[i + at..i + at + 4].copy_from_slice(&declared.to_le_bytes());
                    patched += 1;
                }
            }
        }
        assert_eq!(patched, 2, "both of `{liar}`'s headers");
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn an_entry_that_inflates_past_its_declared_size_is_refused_before_it_is_held() {
        let dir = tempfile::tempdir().unwrap();
        // 8 MiB of one byte deflates to a few kilobytes, and says it is 30 bytes.
        let big = vec![b' '; 8 << 20];
        let good = good(&[]);
        let mut entries: Vec<(&str, &[u8])> = good
            .iter()
            .filter(|(n, _)| n != "ui/card.js")
            .map(|(n, b)| (n.as_str(), b.as_slice()))
            .collect();
        entries.push(("ui/card.js", &big));
        let path = lying_zip(dir.path(), "bomb", &entries, "ui/card.js", 30);
        assert!(std::fs::metadata(&path).unwrap().len() < 1 << 20);
        // What the archive says adds up; nothing is inflated to read it.
        assert!(inspect(&path, FP).is_ok());
        let err = crate::signature::of_zip(&path).unwrap_err();
        assert!(err.contains("`ui/card.js` inflates to more than the 30 bytes it declares"), "{err}");
        let out = dir.path().join("out");
        let err = extract(&path, &out).unwrap_err();
        assert!(err.contains("`ui/card.js` inflates to more than the 30 bytes it declares"), "{err}");
        let written = std::fs::metadata(out.join("ui/card.js")).map(|m| m.len()).unwrap_or(0);
        assert!(written <= 31, "{written} bytes were written");

        // The manifest is read the same way.
        let mut entries: Vec<(&str, &[u8])> = good
            .iter()
            .filter(|(n, _)| n != MANIFEST)
            .map(|(n, b)| (n.as_str(), b.as_slice()))
            .collect();
        let padded = format!("{MANIFEST_TEXT}{}", " ".repeat(8 << 20));
        entries.push((MANIFEST, padded.as_bytes()));
        let path = lying_zip(dir.path(), "manifest-bomb", &entries, MANIFEST, 64);
        let err = inspect(&path, FP).unwrap_err();
        assert!(err.contains("inflates to more than the 64 bytes it declares"), "{err}");
    }
}
