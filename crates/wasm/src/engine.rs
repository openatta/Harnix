//! The process-wide wasmtime engine: its configuration, its fingerprint, and
//! loading components.
//!
//! One `Engine` per process, shared by every application. It owns the code
//! of every loaded component and the epoch every deadline is measured in;
//! isolation comes from the per-instance `Store`, not from the engine, so a
//! second engine would buy nothing.
//!
//! # The fingerprint
//!
//! A precompiled artifact (`.cwasm`) is machine code for one wasmtime
//! version, one target and one engine configuration. The store compiles each
//! application once per fingerprint and the installer picks the artifact by
//! it, so the fingerprint has to be identical in the build that compiles
//! (with Cranelift) and the build that loads (without). It is therefore
//! derived from what both builds share — the wasmtime version, the target
//! triple, and the engine configuration minus the compiler's own settings —
//! and never from wasmtime's compatibility hash, which hashes the compiler
//! too and so differs between the two builds by construction.

use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use wasmtime::component::Component;
use wasmtime::{Config, Engine, Precompiled};

/// The wasmtime this crate is built against, as pinned in its Cargo.toml.
/// Part of the fingerprint: an artifact is bound to the exact version that
/// compiled it, and a patch release that fixes the compiler should not keep
/// running code the broken one produced.
pub const WASMTIME_VERSION: &str = "47.0.3";

/// How often the engine's epoch advances. A running guest yields at every
/// tick, so this bounds how late a deadline or a cancellation can be noticed.
pub const EPOCH_TICK: Duration = Duration::from_millis(10);

/// wasmtime has its own error type; flatten it before `anyhow`'s context
/// combinators apply.
pub(crate) fn wasm_err(e: wasmtime::Error) -> anyhow::Error {
    anyhow!("{e:#}")
}

/// The shared wasmtime engine. Cheap to clone.
#[derive(Clone)]
pub struct WasmEngine {
    engine: Engine,
    fingerprint: Arc<str>,
    /// Keeps the epoch ticker alive while any clone exists. Without it the
    /// epoch never advances, guests never reach a yield point, and a spinning
    /// component holds its thread with no way out.
    _ticker: Arc<EpochTicker>,
}

impl WasmEngine {
    /// The engine configuration, and the only place it is written: the
    /// fingerprint is derived from it, and two copies would be two chances
    /// for the compiling and the loading build to disagree silently.
    fn config() -> Config {
        let mut config = Config::new();
        config.wasm_component_model(true);
        // Deadlines are wall-clock questions ("has this call outlived its
        // budget"), so interruption is by epoch, not by fuel: fuel counts
        // instructions, which answers a different question.
        config.epoch_interruption(true);
        config
    }

    pub fn new() -> Result<Self> {
        let config = Self::config();
        let fingerprint = fingerprint_of(&config);
        let engine = Engine::new(&config)
            .map_err(wasm_err)
            .context("constructing the wasmtime engine")?;
        let ticker = Arc::new(EpochTicker::start(&engine)?);
        Ok(Self {
            engine,
            fingerprint: fingerprint.into(),
            _ticker: ticker,
        })
    }

    /// What an artifact must have been compiled for to load here:
    /// `wasmtime-<version>+<target triple>+<configuration digest>`. Stable
    /// for a given build; reported in `harnix.hello` so the store can serve
    /// the matching artifact.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// Load a precompiled component.
    ///
    /// `expected_fingerprint` is what the artifact claims to have been
    /// compiled for (the `<engine>` in `wasm/<name>.<engine>.cwasm`). A
    /// mismatch is refused before the bytes are looked at; wasmtime then
    /// checks its own header and configuration record as well, so an
    /// artifact that is mislabelled is refused too.
    ///
    /// # Safety
    ///
    /// The bytes are machine code, and wasmtime's checks establish only that
    /// they look like its own output, not that they are. Pass only an
    /// artifact this process compiled itself, or one whose signature (the
    /// store's countersignature over its sha256) was verified first. A
    /// tampered artifact that passes the header checks executes as native
    /// code with this process's privileges.
    pub unsafe fn load(&self, artifact: &[u8], expected_fingerprint: &str) -> Result<AppComponent> {
        if expected_fingerprint != &*self.fingerprint {
            bail!(
                "the artifact was compiled for engine `{expected_fingerprint}`, and this is \
                 `{}`: it needs an artifact built for this Harnix release",
                self.fingerprint
            );
        }
        if Engine::detect_precompiled(artifact) != Some(Precompiled::Component) {
            bail!("not a precompiled component");
        }
        // SAFETY: the caller vouches for the artifact's provenance (see
        // above); the fingerprint and the header checks only keep an honest
        // artifact for another engine from being misread.
        let component = unsafe { Component::deserialize(&self.engine, artifact) }
            .map_err(wasm_err)
            .context("loading the precompiled component")?;
        AppComponent::new(&self.engine, component)
    }

    /// Compile a component to an artifact `load` accepts on an engine with
    /// the same fingerprint.
    #[cfg(feature = "compile")]
    pub fn compile(&self, wasm: &[u8]) -> Result<Vec<u8>> {
        let component = Component::new(&self.engine, wasm)
            .map_err(wasm_err)
            .context("compiling the component")?;
        // Refuse here what `load` would refuse, so a build tool cannot
        // produce an artifact no Harnix will run.
        AppComponent::new(&self.engine, component.clone())?;
        component
            .serialize()
            .map_err(wasm_err)
            .context("serializing the compiled component")
    }

    /// Compile and load in one step, for development: the code never leaves
    /// this process, so there is nothing to verify and no `unsafe`.
    #[cfg(feature = "compile")]
    pub fn load_wasm(&self, wasm: &[u8]) -> Result<AppComponent> {
        let component = Component::new(&self.engine, wasm)
            .map_err(wasm_err)
            .context("compiling the component")?;
        AppComponent::new(&self.engine, component)
    }

    pub(crate) fn inner(&self) -> &Engine {
        &self.engine
    }
}

/// A loaded component, ready to be linked into an [`crate::AppVm`]. Cheap to
/// clone.
#[derive(Clone)]
pub struct AppComponent {
    pub(crate) component: Component,
    world: crate::World,
}

impl AppComponent {
    fn new(engine: &Engine, component: Component) -> Result<Self> {
        let world = world_of(engine, &component)?;
        Ok(Self { component, world })
    }

    /// Which `harnix:app` world the component implements. The installer
    /// compares it with the manifest's claim.
    pub fn world(&self) -> crate::World {
        self.world
    }
}

/// The world a component implements, from the interfaces it exports.
///
/// Read from the exports rather than trusted from the manifest: a manifest
/// can be edited, the component's exports cannot without rebuilding it.
fn world_of(engine: &Engine, component: &Component) -> Result<crate::World> {
    let ty = component.component_type();
    let exports: Vec<String> = ty
        .exports(engine)
        .map(|(name, _)| name.to_string())
        .collect();
    let exports_interface = |interface: &str| {
        let prefix = format!("harnix:app/{interface}@0.1.");
        exports.iter().any(|e| e.starts_with(&prefix))
    };
    if exports_interface("backend") {
        Ok(crate::World::Backend)
    } else if exports_interface("tools") {
        Ok(crate::World::Tools)
    } else {
        bail!(
            "the component implements neither `harnix:app/backend` nor `harnix:app/tools` \
             at 0.1 (it exports: {})",
            if exports.is_empty() {
                "nothing".to_string()
            } else {
                exports.join(", ")
            }
        )
    }
}

/// The fingerprint of an engine built from `config`.
fn fingerprint_of(config: &Config) -> String {
    let described = without_field(&format!("{config:?}"), "compiler_config");
    let digest = hex::encode(Sha256::digest(described.as_bytes()));
    format!(
        "wasmtime-{WASMTIME_VERSION}+{}+{}",
        env!("HARNIX_WASM_TARGET"),
        &digest[..12]
    )
}

/// `debug` (a `{:?}` rendering of a struct) without the field `field`.
///
/// `Config`'s `Debug` lists every wasm feature and every tunable that was
/// set, which is exactly the configuration an artifact depends on — plus,
/// in a build with a compiler, the compiler's settings, which is exactly what
/// must not distinguish the compiling build from the loading one. Removing
/// that one field is less fragile than restating the rest by hand.
fn without_field(debug: &str, field: &str) -> String {
    let needle = format!(", {field}: ");
    let Some(start) = debug.find(&needle) else {
        return debug.to_string();
    };
    let bytes = debug.as_bytes();
    let (mut depth, mut in_str, mut escaped) = (0usize, false, false);
    let mut end = debug.len();
    for (i, &b) in bytes.iter().enumerate().skip(start + needle.len()) {
        if in_str {
            match b {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' | b'[' | b'(' => depth += 1,
            // The parent's closing bracket: keep the space before it.
            b'}' | b']' | b')' if depth == 0 => {
                end = debug[..i].trim_end().len();
                break;
            }
            b'}' | b']' | b')' => depth -= 1,
            b',' if depth == 0 => {
                end = i;
                break;
            }
            _ => {}
        }
    }
    format!("{}{}", &debug[..start], &debug[end..])
}

/// Advances the engine's epoch on a fixed interval.
///
/// A thread rather than a timer task: it has to keep running when every
/// async worker is occupied, and "every worker is occupied" is precisely what
/// a runaway guest causes.
struct EpochTicker {
    stop: Arc<AtomicBool>,
}

impl EpochTicker {
    fn start(engine: &Engine) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let engine = engine.weak();
        let flag = stop.clone();
        std::thread::Builder::new()
            .name("harnix-wasm-epoch".into())
            .spawn(move || {
                while !flag.load(Ordering::Relaxed) {
                    std::thread::sleep(EPOCH_TICK);
                    match engine.upgrade() {
                        Some(engine) => engine.increment_epoch(),
                        None => break,
                    }
                }
            })
            // An engine whose epoch never advances cannot enforce a single
            // deadline, so it is not an engine to hand out.
            .context("starting the epoch ticker")?;
        Ok(Self { stop })
    }
}

impl Drop for EpochTicker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_engine_configuration_is_self_consistent() {
        WasmEngine::new().expect("Engine::new rejects a Config whose options disagree");
    }

    #[test]
    fn the_fingerprint_names_the_version_and_target_and_is_stable() {
        let a = WasmEngine::new().unwrap();
        let b = WasmEngine::new().unwrap();
        assert_eq!(a.fingerprint(), b.fingerprint());
        let fp = a.fingerprint();
        assert!(
            fp.starts_with(&format!("wasmtime-{WASMTIME_VERSION}+")),
            "{fp}"
        );
        assert!(fp.contains(env!("HARNIX_WASM_TARGET")), "{fp}");
        // It ends up in a file name (`<name>.<engine>.cwasm`).
        assert!(!fp.contains(['/', '\\', ' ', ':']), "{fp}");
    }

    /// The compiler's settings must not reach the fingerprint, or the store's
    /// compiler and the product would never agree.
    #[test]
    fn the_fingerprint_ignores_the_compiler() {
        let described = without_field(&format!("{:?}", WasmEngine::config()), "compiler_config");
        assert!(!described.contains("compiler_config"), "{described}");
        assert!(described.contains("epoch_interruption"), "{described}");
    }

    #[test]
    fn a_field_is_removed_with_whatever_it_nests() {
        let s = r#"S { a: 1, c: C { x: [1, 2], y: "}," }, d: true }"#;
        assert_eq!(without_field(s, "c"), "S { a: 1, d: true }");
        assert_eq!(without_field("S { a: 1, c: 2 }", "c"), "S { a: 1 }");
        assert_eq!(without_field("S { a: 1 }", "c"), "S { a: 1 }");
    }

    /// The pin in Cargo.toml and the constant above are the same decision
    /// written twice; this keeps them from drifting.
    #[test]
    fn the_stated_wasmtime_version_is_the_pinned_one() {
        let manifest = include_str!("../Cargo.toml");
        for dep in ["wasmtime ", "wasmtime-wasi "] {
            let line = manifest
                .lines()
                .find(|l| l.starts_with(dep))
                .unwrap_or_else(|| panic!("no `{dep}` dependency"));
            assert!(
                line.contains(&format!("\"={WASMTIME_VERSION}\"")),
                "`{line}` does not pin {WASMTIME_VERSION}"
            );
        }
    }

    #[test]
    fn bytes_that_are_not_an_artifact_are_refused() {
        let engine = WasmEngine::new().unwrap();
        let fp = engine.fingerprint().to_string();
        // SAFETY: refused before deserialization; nothing is executed.
        let err = unsafe { engine.load(b"definitely not an artifact", &fp) }
            .err()
            .unwrap();
        assert!(
            err.to_string().contains("not a precompiled component"),
            "{err}"
        );
    }

    #[test]
    fn a_mismatched_fingerprint_is_refused_before_the_bytes_are_read() {
        let engine = WasmEngine::new().unwrap();
        // SAFETY: refused before deserialization; nothing is executed.
        let err = unsafe { engine.load(b"", "wasmtime-1.0.0+elsewhere+000000000000") }
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("wasmtime-1.0.0+elsewhere"), "{err}");
        assert!(err.contains(engine.fingerprint()), "{err}");
    }
}
