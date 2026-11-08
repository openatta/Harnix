//! The application author's packing tool.
//!
//!     harnix-app pack <app dir> [--wasm <component.wasm>] [--build] [-o <out.hxapp>]
//!     harnix-app fingerprint
//!
//! `pack` reads the directory's `harnix-app.toml`, takes the component (`--wasm`, or with
//! `--build` the one `cargo build --release --target wasm32-wasip2` makes of `<app dir>/component`),
//! compiles it for this engine, and zips the manifest, `ui/`, `agent/` and both forms of the
//! component into a `.hxapp`. It prints the package's path and the engine fingerprint it was
//! compiled for: a package installs on a Harnix whose engine reports the same one.
//!
//! A package packed here is unsigned; a Harnix installs it only with `[apps] dev = true`.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("fingerprint") => {
            println!("{}", harnix_wasm::WasmEngine::new()?.fingerprint());
            Ok(())
        }
        Some("pack") => pack(&args[1..]),
        _ => bail!(
            "usage: harnix-app pack <app dir> [--wasm <component.wasm>] [--build] [-o <out.hxapp>] \
             | harnix-app fingerprint"
        ),
    }
}

fn pack(args: &[String]) -> Result<()> {
    let mut dir = None;
    let mut wasm = None;
    let mut out = None;
    let mut build = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--wasm" => wasm = Some(PathBuf::from(it.next().context("--wasm takes a path")?)),
            "-o" | "--out" => out = Some(PathBuf::from(it.next().context("-o takes a path")?)),
            "--build" => build = true,
            other if dir.is_none() => dir = Some(PathBuf::from(other)),
            other => bail!("unexpected `{other}`"),
        }
    }
    let dir = dir.context("which application directory?")?;
    let manifest = harnix_apps::manifest::Manifest::parse(
        &std::fs::read_to_string(dir.join(harnix_apps::manifest::MANIFEST)).with_context(|| {
            format!(
                "reading {}",
                dir.join(harnix_apps::manifest::MANIFEST).display()
            )
        })?,
    )
    .map_err(anyhow::Error::msg)?;
    if build && wasm.is_none() {
        wasm = Some(build_component(&dir.join("component"))?);
    }
    let engine = harnix_wasm::WasmEngine::new()?;
    let out = out.unwrap_or_else(|| {
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
        .map(|cwasm| harnix_apps::package::Artifact {
            fingerprint: engine.fingerprint(),
            cwasm,
            wasm: portable.as_deref(),
        });
    harnix_apps::package::pack(&dir, artifact, &out).map_err(anyhow::Error::msg)?;
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
