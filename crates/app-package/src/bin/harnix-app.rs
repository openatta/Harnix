//! The application author's tool, and the store's.
//!
//!     harnix-app new <dir> --id <publisher.name> [--title <title>]
//!     harnix-app build <app dir> [-o <dir>]
//!     harnix-app pack <app dir> [--wasm <component.wasm>] [--build] [-o <out.hxapp>]
//!     harnix-app keygen <secret key file>
//!     harnix-app sign <package.hxapp> --key <secret key file> [-o <out.hxapp>]
//!     harnix-app verify <package.hxapp> [--store-key <hex>]… [--trust <publisher>=<hex>]…
//!     harnix-app fingerprint
//!
//! and, on the store's side,
//!
//!     harnix-app countersign <package.hxapp> --key <store key file> [--artifact <x.cwasm>]… [-o <out>]
//!     harnix-app index <store dir> [--name <name>]
//!
//! `new` lays out an application: its manifest, a component with one tool, the WIT it builds
//! against. `build` builds the component (`cargo build --release --target wasm32-wasip2` in
//! `<app dir>/component`) and compiles it for this engine — for the target's baseline CPU, so the
//! artifact loads on any machine of this target — into `<app dir>/build/` (or `-o`). `pack` zips
//! the manifest, `ui/`, `agent/` and both forms of the component into a `.hxapp` (`--build`
//! builds first; `--wasm` takes a component already built); it prints the package's path and the
//! engine fingerprint it was compiled for: a package installs on a Harnix whose engine reports the
//! same one.
//!
//! `keygen` makes an ed25519 key (the file holds its secret; the public key is printed). `sign`
//! signs every file of a package as its publisher; a Harnix whose profile trusts that key for that
//! publisher (`[apps.trusted_publishers]`) installs it without dev mode. `countersign` is the
//! store's: it checks the publisher's signature, adds the components it compiled, and signs the
//! lot; `index` lists `<store dir>/packages/*.hxapp` as `<store dir>/index.json`, the catalog a
//! Harnix reads (`[apps] store`).
//!
//! An unsigned package installs only with `[apps] dev = true`.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use harnix_app_package::manifest::{Manifest, MANIFEST};
use harnix_app_package::signature::{self, Keys, PublicKey, SecretKey};

const USAGE: &str =
    "usage: harnix-app new <dir> --id <publisher.name> | build <app dir> [-o <dir>] \
                     | pack <app dir> [--wasm <component.wasm>] [--build] [-o <out.hxapp>] \
                     | keygen <key file> | sign <package> --key <key file> [-o <out>] \
                     | verify <package> [--store-key <hex>]… [--trust <publisher>=<hex>]… \
                     | countersign <package> --key <key file> [--artifact <x.cwasm>]… [-o <out>] \
                     | index <store dir> [--name <name>] | fingerprint";

/// The WIT a component builds against, written beside it by `new`.
const WIT: &str = include_str!("../../../wasm/wit/app.wit");

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

/* ── an application ───────────────────────────────────────────────────── */

fn new(args: &[String]) -> Result<()> {
    let args = Args::parse(args, &["--id", "--title"], &[])?;
    let dir = args.positional("directory")?;
    let id = args
        .one(&["--id"])
        .context("--id <publisher.name> names the application")?;
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
world = "harnix:app/tools-app@0.1.0"
memory_mb = 32
timeout_ms = 20000
"#,
        api = harnix_app_package::manifest::HARNIX_API,
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
    let lib = r#"//! The application's tools: a `tools-app` component (`harnix:app`, wit/app.wit).
//!
//! To also see, and veto, the calls of these tools, build `tools-app-events` instead and implement
//! `exports::harnix::app::events::Guest` (`on_event` answering `HookDecision::Proceed`,
//! `Block(reason)` or `AddContext(note)`).

wit_bindgen::generate!({
    path: "wit",
    world: "tools-app",
});

use exports::harnix::app::tools::{Guest as Tools, ToolDef, ToolOutput};
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
        "# {title}\n\n`{id}`: a Harnix application.\n\n```sh\nharnix-app build .          # the component, compiled for this engine\nharnix-app pack . --build   # {id}-0.1.0.hxapp\nharnix-app keygen publisher.key\nharnix-app sign {id}-0.1.0.hxapp --key publisher.key\n```\n"
    );
    let files: [(&str, &str); 5] = [
        (MANIFEST, &manifest),
        ("component/Cargo.toml", &cargo),
        ("component/src/lib.rs", lib),
        ("component/wit/app.wit", WIT),
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
                "{}-{}.hxapp",
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
        Some(bytes) => Some(engine.compile(bytes)?),
        None => None,
    };
    let artifact = compiled
        .as_ref()
        .map(|cwasm| harnix_app_package::package::Artifact {
            fingerprint: engine.fingerprint(),
            cwasm,
            wasm: portable.as_deref(),
        });
    harnix_app_package::package::pack(dir, artifact, &out).map_err(anyhow::Error::msg)?;
    println!("{}", out.display());
    println!("{}", engine.fingerprint());
    Ok(())
}

/// `cargo build --release --target wasm32-wasip2` in `component`, and the one `.wasm` it made.
fn build_component(component: &Path) -> Result<PathBuf> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let target = component.join("target");
    let status = std::process::Command::new(cargo)
        .args(["build", "--release", "--target", "wasm32-wasip2"])
        .env("CARGO_TARGET_DIR", &target)
        .current_dir(component)
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
    let signed = harnix_app_package::publish::sign(package, &key, &out).map_err(anyhow::Error::msg)?;
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
    let signed = harnix_app_package::publish::countersign(package, &key, &refs, &out)
        .map_err(anyhow::Error::msg)?;
    println!("{}", out.display());
    println!("store {}", signed.store.map(|s| s.key).unwrap_or_default());
    Ok(())
}

fn verify(args: &[String]) -> Result<()> {
    let args = Args::parse(args, &["--store-key", "--trust"], &[])?;
    let package = args.positional("package")?;
    let mut keys = Keys {
        store: harnix_app_package::store::store_keys(),
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
    let manifest = harnix_app_package::package::manifest_of(package).map_err(anyhow::Error::msg)?;
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

fn index(args: &[String]) -> Result<()> {
    let args = Args::parse(args, &["--name"], &[])?;
    let store = args.positional("store directory")?;
    let index =
        harnix_app_package::publish::index(store, args.one(&["--name"])).map_err(anyhow::Error::msg)?;
    println!("{}", store.join("index.json").display());
    for app in &index.apps {
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
