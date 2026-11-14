//! The package author's tool, and the store's.
//!
//!     harnix-pack new <dir> --id <publisher.name> [--title <title>]
//!     harnix-pack build <app dir> [-o <dir>]
//!     harnix-pack pack <app dir> [--wasm <component.wasm>] [--build] [-o <out.hxext>]
//!     harnix-pack keygen <secret key file>
//!     harnix-pack sign <package.hxext> --key <secret key file> [-o <out.hxext>]
//!     harnix-pack verify <package.hxext> [--store-key <hex>]… [--trust <publisher>=<hex>]…
//!     harnix-pack fingerprint
//!
//! and, on the store's side,
//!
//!     harnix-pack countersign <package.hxext> --key <store key file> [--artifact <x.cwasm>]… [-o <out>]
//!     harnix-pack index <store dir> --key <store key file> [--name <name>] [--days <n>]
//!     harnix-pack sign-index <index.json> --key <store key file> [--days <n> | --expires <time>]
//!
//! `new` lays out a package: its manifest, a component with one tool, the WIT it builds
//! against. `build` builds the component (`cargo build --release --target wasm32-wasip2` in
//! `<app dir>/component`) and compiles it for this engine — for the target's baseline CPU, so the
//! artifact loads on any machine of this target — into `<app dir>/build/` (or `-o`). `pack` zips
//! the manifest, `ui/`, `agent/` and both forms of the component into a `.hxext` (`--build`
//! builds first; `--wasm` takes a component already built); it prints the package's path and the
//! engine fingerprint it was compiled for: a package installs on a Harnix whose engine reports the
//! same one.
//!
//! `keygen` makes an ed25519 key (the file holds its secret; the public key is printed). `sign`
//! signs every file of a package as its publisher; a Harnix whose profile trusts that key for that
//! publisher (`[ext.packages.trusted_publishers]`) installs it without dev mode. `countersign` is the
//! store's: it checks the publisher's signature, adds the components it compiled, and signs the
//! lot; `index` lists `<store dir>/packages/*.hxext` as `<store dir>/index.json`, the catalog a
//! Harnix reads (`[ext.packages] store`), signed with the store's key and believed for `--days` (14
//! unless said). `sign-index` signs a catalog again after the store edited it — a version
//! `revoked`, a package's `min_version` raised — or to republish it before it expires;
//! `--expires` (`YYYY-MM-DDTHH:MM:SSZ`) says when instead.
//!
//! An unsigned package installs only with `[ext.packages] dev = true`.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use harnix_package::manifest::{Manifest, MANIFEST};
use harnix_package::signature::{self, Keys, PublicKey, SecretKey};

const USAGE: &str =
    "usage: harnix-pack new <dir> --id <publisher.name> | build <app dir> [-o <dir>] \
                     | pack <app dir> [--wasm <component.wasm>] [--build] [-o <out.hxext>] \
                     | keygen <key file> | sign <package> --key <key file> [-o <out>] \
                     | verify <package> [--store-key <hex>]… [--trust <publisher>=<hex>]… \
                     | countersign <package> --key <key file> [--artifact <x.cwasm>]… [-o <out>] \
                     | index <store dir> --key <key file> [--name <name>] [--days <n>] \
                     | sign-index <index.json> --key <key file> [--days <n> | --expires <time>] \
                     | fingerprint";

/// The WIT a component builds against, written beside it by `new`.
const WIT: &str = include_str!("../../../wasm/wit/ext.wit");

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or_default();
    match args.first().map(String::as_str) {
        Some("fingerprint") => {
            println!("{}", harnix_wasm::WasmEngine::new()?.fingerprint());
            Ok(())
        }
        Some("new") => new(rest),
        Some("build") => build(rest),
        Some("pack") => pack(rest),
        Some("keygen") => keygen(rest),
        Some("sign") => sign(rest),
        Some("verify") => verify(rest),
        Some("countersign") => countersign(rest),
        Some("index") => index(rest),
        Some("sign-index") => sign_index(rest),
        _ => bail!(USAGE),
    }
}

/// `<positional> [--flag value]…`: the positional, and each flag's values in order.
struct Args {
    positional: Option<PathBuf>,
    flags: Vec<(String, String)>,
    switches: Vec<String>,
}

impl Args {
    fn parse(args: &[String], with_value: &[&str], switches: &[&str]) -> Result<Self> {
        let mut out = Args {
            positional: None,
            flags: Vec::new(),
            switches: Vec::new(),
        };
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            if with_value.contains(&arg.as_str()) {
                let value = it.next().with_context(|| format!("{arg} takes a value"))?;
                out.flags.push((arg.clone(), value.clone()));
            } else if switches.contains(&arg.as_str()) {
                out.switches.push(arg.clone());
            } else if out.positional.is_none() && !arg.starts_with('-') {
                out.positional = Some(PathBuf::from(arg));
            } else {
                bail!("unexpected `{arg}`\n{USAGE}");
            }
        }
        Ok(out)
    }

    fn one(&self, names: &[&str]) -> Option<&str> {
        self.flags
            .iter()
            .rev()
            .find(|(n, _)| names.contains(&n.as_str()))
            .map(|(_, v)| v.as_str())
    }

    fn all(&self, name: &str) -> Vec<&str> {
        self.flags
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    fn positional(&self, what: &str) -> Result<&Path> {
        self.positional
            .as_deref()
            .with_context(|| format!("which {what}?\n{USAGE}"))
    }
}

fn read_manifest(dir: &Path) -> Result<Manifest> {
    let path = dir.join(MANIFEST);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    Manifest::parse(&text).map_err(anyhow::Error::msg)
}

fn read_key(path: &str) -> Result<SecretKey> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?;
    SecretKey::parse(&text).map_err(|e| anyhow::anyhow!("{path}: {e}"))
}

/* ── a package ───────────────────────────────────────────────────── */

fn new(args: &[String]) -> Result<()> {
    let args = Args::parse(args, &["--id", "--title"], &[])?;
    let dir = args.positional("directory")?;
    let id = args
        .one(&["--id"])
        .context("--id <publisher.name> names the package")?;
    let (publisher, name) = id
        .split_once('.')
        .context("an application id is <publisher>.<name>")?;
    let title = args.one(&["--title"]).unwrap_or(name);
    if dir.exists() && std::fs::read_dir(dir)?.next().is_some() {
        bail!("{} exists and is not empty", dir.display());
    }
    let manifest = format!(
        r#"[app]
id = "{id}"
version = "0.1.0"
harnix_api = {api}
title = "{title}"
publisher = "{publisher}"

# Nothing is granted that is not written here, and the installer shows each line.
[capabilities]

[[component]]
name = "tools"
world = "harnix:ext/tools-ext@1.0.0"
memory_mb = 32
timeout_ms = 20000

# Every tool the component lists, and only those: `harnix-pack build` checks it does.
[[tool]]
name = "hello"
description = "Say hello to someone"
read_only = true
concurrency_safe = true
[tool.input_schema]
type = "object"
properties.name = {{ type = "string" }}
"#,
        api = harnix_package::manifest::HARNIX_API,
    );
    // Validated as it would be installed, so a scaffold that would be refused is never written.
    Manifest::parse(&manifest).map_err(anyhow::Error::msg)?;
    let crate_name = format!("{}-tools", name.replace('.', "-"));
    let cargo = format!(
        r#"[package]
name = "{crate_name}"
version = "0.1.0"
edition = "2021"
publish = false

# Its own workspace: it builds for wasm32-wasip2 (`rustup target add wasm32-wasip2`).
[workspace]

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.48"
serde_json = "1"

[profile.release]
opt-level = "s"
strip = true
lto = true
"#
    );
    let lib = r#"//! The package's tools: a `tools-ext` component (`harnix:ext`, wit/ext.wit).
//!
//! To also see, and veto, the calls of these tools, build `tools-ext-events` instead and implement
//! `exports::harnix::ext::events::Guest` (`on_event` answering `HookDecision::Proceed`,
//! `Block(reason)` or `AddContext(note)`).

wit_bindgen::generate!({
    path: "wit",
    world: "tools-ext",
});

use exports::harnix::ext::tools::{Guest as Tools, ToolDef, ToolOutput};
use serde_json::{json, Value};

struct App;

impl Tools for App {
    fn list_tools() -> Vec<ToolDef> {
        vec![ToolDef {
            name: "hello".into(),
            description: "Say hello to someone".into(),
            doc: None,
            input_schema: json!({"type": "object", "properties": {"name": {"type": "string"}}}).to_string(),
            read_only: true,
            concurrency_safe: true,
        }]
    }

    fn call_tool(name: String, input_json: String, _call_id: String) -> ToolOutput {
        let input: Value = serde_json::from_str(&input_json).unwrap_or(Value::Null);
        match name.as_str() {
            "hello" => ToolOutput {
                content: format!("hello, {}", input["name"].as_str().unwrap_or("world")),
                structured: None,
                is_error: false,
            },
            other => ToolOutput {
                content: format!("no tool `{other}`"),
                structured: None,
                is_error: true,
            },
        }
    }
}

export!(App);
"#;
    let readme = format!(
        "# {title}\n\n`{id}`: an extension package.\n\n```sh\nharnix-pack build .          # the component, compiled for this engine\nharnix-pack pack . --build   # {id}-0.1.0.hxext\nharnix-pack keygen publisher.key\nharnix-pack sign {id}-0.1.0.hxext --key publisher.key\n```\n"
    );
    let files: [(&str, &str); 5] = [
        (MANIFEST, &manifest),
        ("component/Cargo.toml", &cargo),
        ("component/src/lib.rs", lib),
        ("component/wit/ext.wit", WIT),
        ("README.md", &readme),
    ];
    for (path, text) in files {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    }
    println!("{}", dir.display());
    Ok(())
}

/// The component of `dir`: built, and compiled for this engine. Both files' paths.
fn build_and_compile(dir: &Path, out: &Path) -> Result<(PathBuf, PathBuf, String)> {
    let manifest = read_manifest(dir)?;
    let component = manifest
        .component
        .as_ref()
        .context("the manifest declares no [[component]]: there is nothing to build")?;
    let wasm = build_component(&dir.join("component"))?;
    let engine = harnix_wasm::WasmEngine::new()?;
    let bytes = std::fs::read(&wasm).with_context(|| format!("reading {}", wasm.display()))?;
    check_tools(&engine, &bytes, &manifest)?;
    let compiled = engine.compile(&bytes)?;
    std::fs::create_dir_all(out)?;
    let portable = out.join(format!("{}.wasm", component.name));
    let artifact = out.join(format!("{}.{}.cwasm", component.name, engine.fingerprint()));
    std::fs::write(&portable, &bytes)?;
    std::fs::write(&artifact, compiled)?;
    Ok((portable, artifact, engine.fingerprint().to_string()))
}

fn build(args: &[String]) -> Result<()> {
    let args = Args::parse(args, &["-o", "--out"], &[])?;
    let dir = args.positional("application directory")?;
    let out = args
        .one(&["-o", "--out"])
        .map(PathBuf::from)
        .unwrap_or_else(|| dir.join("build"));
    let (portable, artifact, fingerprint) = build_and_compile(dir, &out)?;
    println!("{}", portable.display());
    println!("{}", artifact.display());
    println!("{fingerprint}");
    Ok(())
}

fn pack(args: &[String]) -> Result<()> {
    let args = Args::parse(args, &["--wasm", "-o", "--out"], &["--build"])?;
    let dir = args.positional("application directory")?;
    let manifest = read_manifest(dir)?;
    let mut wasm = args.one(&["--wasm"]).map(PathBuf::from);
    if args.switches.iter().any(|s| s == "--build") && wasm.is_none() {
        wasm = Some(build_component(&dir.join("component"))?);
    }
    let engine = harnix_wasm::WasmEngine::new()?;
    let out = args
        .one(&["-o", "--out"])
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(format!(
                "{}-{}.hxext",
                manifest.app.id, manifest.app.version
            ))
        });
    let portable = match &wasm {
        Some(path) => {
            Some(std::fs::read(path).with_context(|| format!("reading {}", path.display()))?)
        }
        None => None,
    };
    let compiled = match &portable {
        Some(bytes) => {
            check_tools(&engine, bytes, &manifest)?;
            Some(engine.compile(bytes)?)
        }
        None => None,
    };
    let artifact = compiled
        .as_ref()
        .map(|cwasm| harnix_package::package::Artifact {
            fingerprint: engine.fingerprint(),
            cwasm,
            wasm: portable.as_deref(),
        });
    harnix_package::package::pack(dir, artifact, &out).map_err(anyhow::Error::msg)?;
    println!("{}", out.display());
    println!("{}", engine.fingerprint());
    Ok(())
}

/// What the component lists as its tools against the manifest's `[[tool]]`: the same names, and the
/// same description, input schema, `read_only` and `concurrency_safe` for each. The manifest is
/// what the installer shows and the host registers, so a component that lists anything else is a
/// package that would be refused when it starts; this says so while its author can still fix it.
fn check_tools(engine: &harnix_wasm::WasmEngine, wasm: &[u8], manifest: &Manifest) -> Result<()> {
    use harnix_wasm::{VmHost, MemoryKv, NoNetwork, Services};

    /// Listing tools asks nothing of the host: whatever the component asks for is refused.
    struct Nothing;

    #[async_trait::async_trait]
    impl VmHost for Nothing {
        async fn call(&self, _: &str, _: &str, _: serde_json::Value) -> Result<serde_json::Value, String> {
            Err("not while listing tools".into())
        }
        fn publish(&self, _: &str, _: &str, _: serde_json::Value) -> Result<(), String> {
            Err("not while listing tools".into())
        }
        async fn subscribe(&self, _: &str, _: &str, _: &serde_json::Value) -> Result<(), String> {
            Err("not while listing tools".into())
        }
        async fn unsubscribe(&self, _: &str, _: &str, _: &serde_json::Value) {}
    }

    let component = engine.load_wasm(wasm)?;
    let services = Services {
        host: std::sync::Arc::new(Nothing),
        kv: std::sync::Arc::new(MemoryKv::new()),
        http: std::sync::Arc::new(NoNetwork),
    };
    let listed = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(harnix_wasm::AppVm::listed_tools(engine, &component, services))?;

    let mut problems = Vec::new();
    for tool in &manifest.tools {
        let Some(found) = listed.iter().find(|t| t.name == tool.name) else {
            problems.push(format!("the manifest declares `{}`, and the component does not list it", tool.name));
            continue;
        };
        let schema_of = |text: &str| serde_json::from_str::<serde_json::Value>(text).ok();
        if found.description != tool.description {
            problems.push(format!("`{}`: the description differs (component: {:?})", tool.name, found.description));
        }
        if found.doc != tool.doc {
            problems.push(format!("`{}`: the `doc` differs", tool.name));
        }
        if schema_of(&found.input_schema) != schema_of(&tool.input_schema_json()) {
            problems.push(format!("`{}`: the input schema differs", tool.name));
        }
        if (found.read_only, found.concurrency_safe) != (tool.read_only, tool.concurrency_safe) {
            problems.push(format!("`{}`: `read_only` or `concurrency_safe` differs", tool.name));
        }
    }
    for found in &listed {
        if !manifest.tools.iter().any(|t| t.name == found.name) {
            problems.push(format!("the component lists `{}`, and the manifest has no `[[tool]]` for it", found.name));
        }
    }
    if problems.is_empty() {
        return Ok(());
    }
    bail!("the component's tools are not the manifest's:\n  {}", problems.join("\n  "))
}

/// `cargo build --release --target wasm32-wasip2` in `component`, and the one `.wasm` it made.
fn build_component(component: &Path) -> Result<PathBuf> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    // Absolute: cargo resolves a relative target directory against `current_dir`, not ours.
    let component = std::fs::canonicalize(component)
        .with_context(|| format!("reading {}", component.display()))?;
    let target = component.join("target");
    let status = std::process::Command::new(cargo)
        .args(["build", "--release", "--target", "wasm32-wasip2"])
        .env("CARGO_TARGET_DIR", &target)
        .current_dir(&component)
        .status()
        .with_context(|| format!("running cargo in {}", component.display()))?;
    if !status.success() {
        bail!("the component did not build (is the target installed? rustup target add wasm32-wasip2)");
    }
    let release = target.join("wasm32-wasip2").join("release");
    let built: Vec<PathBuf> = std::fs::read_dir(&release)
        .with_context(|| format!("reading {}", release.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "wasm"))
        .collect();
    match built.as_slice() {
        [one] => Ok(one.clone()),
        [] => bail!("the build made no .wasm in {}", release.display()),
        many => bail!("the build made several .wasm files: {many:?}"),
    }
}

/* ── signatures ───────────────────────────────────────────────────────── */

fn keygen(args: &[String]) -> Result<()> {
    let args = Args::parse(args, &[], &[])?;
    let path = args.positional("key file")?;
    if path.exists() {
        bail!("{} exists; a key is not overwritten", path.display());
    }
    let (key, seed) = SecretKey::generate().map_err(anyhow::Error::msg)?;
    std::fs::write(path, format!("{seed}\n"))
        .with_context(|| format!("writing {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    println!("{}", key.public().hex());
    Ok(())
}

fn sign(args: &[String]) -> Result<()> {
    let args = Args::parse(args, &["--key", "-o", "--out"], &[])?;
    let package = args.positional("package")?;
    let key = read_key(args.one(&["--key"]).context("--key <secret key file>")?)?;
    let out = args
        .one(&["-o", "--out"])
        .map(PathBuf::from)
        .unwrap_or_else(|| package.to_path_buf());
    let signed = harnix_package::publish::sign(package, &key, &out).map_err(anyhow::Error::msg)?;
    println!("{}", out.display());
    println!("{} {}", signed.publisher.id, signed.publisher.key);
    Ok(())
}

fn countersign(args: &[String]) -> Result<()> {
    let args = Args::parse(args, &["--key", "--artifact", "-o", "--out"], &[])?;
    let package = args.positional("package")?;
    let key = read_key(args.one(&["--key"]).context("--key <store key file>")?)?;
    let out = args
        .one(&["-o", "--out"])
        .map(PathBuf::from)
        .unwrap_or_else(|| package.to_path_buf());
    let artifacts: Vec<PathBuf> = args
        .all("--artifact")
        .into_iter()
        .map(PathBuf::from)
        .collect();
    let refs: Vec<&Path> = artifacts.iter().map(PathBuf::as_path).collect();
    let signed = harnix_package::publish::countersign(package, &key, &refs, &out)
        .map_err(anyhow::Error::msg)?;
    println!("{}", out.display());
    println!("store {}", signed.store.map(|s| s.key).unwrap_or_default());
    Ok(())
}

fn verify(args: &[String]) -> Result<()> {
    let args = Args::parse(args, &["--store-key", "--trust"], &[])?;
    let package = args.positional("package")?;
    let mut keys = Keys {
        store: harnix_package::store::store_keys(),
        ..Keys::default()
    };
    let given: Vec<&str> = args.all("--store-key");
    if !given.is_empty() {
        keys.store = given
            .into_iter()
            .map(|k| PublicKey::parse(k).map_err(anyhow::Error::msg))
            .collect::<Result<_>>()?;
    }
    for trust in args.all("--trust") {
        let (publisher, key) = trust
            .split_once('=')
            .context("--trust <publisher>=<hex key>")?;
        keys.trusted
            .entry(publisher.to_string())
            .or_default()
            .push(PublicKey::parse(key).map_err(anyhow::Error::msg)?);
    }
    let (files, signatures) = signature::of_zip(package).map_err(anyhow::Error::msg)?;
    let manifest = harnix_package::package::manifest_of(package).map_err(anyhow::Error::msg)?;
    let app = &manifest.app;
    let verified = signature::verify(
        signatures.as_ref(),
        &files,
        (&app.id, &app.version),
        &app.publisher,
        &keys,
    )
    .map_err(anyhow::Error::msg)?;
    println!("{} {} {}", app.id, app.version, verified.trust);
    Ok(())
}

/// `--days <n>`: how long a catalog signed now is believed.
fn valid_for(args: &Args) -> Result<std::time::Duration> {
    match args.one(&["--days"]) {
        None => Ok(harnix_package::publish::CATALOG_VALID),
        Some(days) => {
            let days: u64 = days.parse().with_context(|| format!("--days {days}: a number of days"))?;
            Ok(std::time::Duration::from_secs(days * 24 * 3600))
        }
    }
}

fn index(args: &[String]) -> Result<()> {
    let args = Args::parse(args, &["--name", "--key", "--days"], &[])?;
    let store = args.positional("store directory")?;
    let key = read_key(args.one(&["--key"]).context("--key <store key file>")?)?;
    let index = harnix_package::publish::index(store, args.one(&["--name"]), &key, valid_for(&args)?)
        .map_err(anyhow::Error::msg)?;
    println!("{}", store.join("index.json").display());
    for app in &index.packages {
        for version in &app.versions {
            println!(
                "{} {} {}",
                app.id,
                version.version,
                version
                    .artifacts
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }
    }
    Ok(())
}

fn sign_index(args: &[String]) -> Result<()> {
    let args = Args::parse(args, &["--key", "--days", "--expires"], &[])?;
    let path = args.positional("index.json")?;
    let key = read_key(args.one(&["--key"]).context("--key <store key file>")?)?;
    let expires = match args.one(&["--expires"]) {
        Some(at) => {
            let secs = harnix_package::store::parse_utc(at).map_err(anyhow::Error::msg)?;
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs)
        }
        None => std::time::SystemTime::now() + valid_for(&args)?,
    };
    let index = harnix_package::publish::sign_index(path, &key, expires).map_err(anyhow::Error::msg)?;
    println!("{} {}", path.display(), index.expires);
    Ok(())
}
