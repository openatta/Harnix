//! The dependency direction, asserted rather than described.
//!
//! concept_and_architecture.md §3 says each kernel part is ignorant of the
//! one above it. A paragraph saying so goes stale the first time someone
//! reaches for a type that is convenient and wrong; this fails the build
//! instead.
//!
//! It also asserts the thing §3.3.3's fifth invariant rests on — the hub and
//! the authorizer cannot name a topology, a connection or an HTTP type — and
//! the constraint that keeps Harnix a thin layer: no interpreter is linked
//! here, because extension execution is Agix's.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/harnix is two levels below the root")
        .to_path_buf()
}

/// Every workspace crate's declared dependencies, as cargo itself reads the manifests:
/// `cargo metadata --no-deps`, which resolves nothing and so needs no network. Read off the
/// manifest's text instead, a dependency written as `[dependencies.x]`, or under
/// `[target.'cfg(unix)'.dependencies]`, went past every rule here.
fn metadata() -> &'static serde_json::Value {
    static METADATA: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
    METADATA.get_or_init(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let out = std::process::Command::new(cargo)
            .args(["metadata", "--no-deps", "--offline", "--format-version", "1"])
            .current_dir(workspace_root())
            .output()
            .expect("cargo metadata runs");
        assert!(out.status.success(), "cargo metadata: {}", String::from_utf8_lossy(&out.stderr));
        serde_json::from_slice(&out.stdout).expect("cargo metadata's JSON")
    })
}

/// One dependency as a manifest declares it: the name the crate knows it by, its kind
/// (`None` for a normal one, `dev`, `build`), and whether it is optional.
struct Dependency {
    name: String,
    kind: Option<String>,
    optional: bool,
}

/// What `crates/<crate_dir>/Cargo.toml` declares, in every section and for every target.
fn declared(crate_dir: &str) -> Vec<Dependency> {
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let manifest = canonical(&workspace_root().join("crates").join(crate_dir).join("Cargo.toml"));
    let package = metadata()["packages"]
        .as_array()
        .expect("packages")
        .iter()
        .find(|p| p["manifest_path"].as_str().map(|m| canonical(Path::new(m))) == Some(manifest.clone()))
        .unwrap_or_else(|| panic!("{} is not a workspace member", manifest.display()));
    package["dependencies"]
        .as_array()
        .expect("dependencies")
        .iter()
        .map(|d| Dependency {
            // A renamed dependency is known by its key (`foo = { package = "bar" }` is `foo`).
            name: d["rename"].as_str().or(d["name"].as_str()).unwrap_or_default().to_string(),
            kind: d["kind"].as_str().map(str::to_string),
            optional: d["optional"].as_bool().unwrap_or(false),
        })
        .collect()
}

/// Direct dependencies of one Harnix crate, by name: normal and dev, for every target.
fn dependencies(crate_dir: &str) -> BTreeSet<String> {
    declared(crate_dir)
        .into_iter()
        .filter(|d| d.kind.as_deref() != Some("build"))
        .map(|d| d.name)
        .collect()
}

fn assert_ignorant_of(crate_dir: &str, forbidden: &[&str], because: &str) {
    let deps = dependencies(crate_dir);
    for name in forbidden {
        assert!(
            !deps.contains(*name),
            "`{crate_dir}` depends on `{name}`, and it must not: {because}"
        );
    }
}

/// Transport moves frames. It does not know what a session is.
#[test]
fn transport_does_not_know_what_a_session_is() {
    assert_ignorant_of(
        "transport",
        &[
            "harnix-instance",
            "harnix-module",
            "harnix-assembly",
            "daemon",
            "base",
        ],
        "it carries channel semantics and nothing about what they contain",
    );
}

/// Authorization decides subject × method. It does not know what a frame
/// looks like or which channel carried one — otherwise the decision starts
/// depending on the topology, and a subject with several connections gets
/// several answers to one question.
#[test]
fn authorization_does_not_know_what_a_frame_looks_like() {
    assert_ignorant_of(
        "authz",
        &[
            "axum",
            "harnix-transport",
            "harnix-hub",
            "harnix-instance",
            "daemon",
        ],
        "it sees an authenticated subject and a method name, and nothing else",
    );
}

/// The hub owns sessions. It does not know about HTTP, TLS, connections or
/// topology — which is what lets a second topology be added without touching
/// it (concept §3.3.3, fifth invariant).
#[test]
fn the_hub_does_not_know_about_connections() {
    assert_ignorant_of(
        "hub",
        &[
            "axum",
            "harnix-transport",
            "harnix-authz",
            "bytes",
            "futures",
        ],
        "it speaks to `dyn FrameSink`, which is all it may learn about a downstream",
    );
    // Nor does it name the engine's crate: it reaches an instance through an `EngineLink`, and
    // the engine's error codes it tells apart come with the link's interface
    // (`harnix_engine_api::ops::codes`).
    assert_ignorant_of(
        "hub",
        &["daemon"],
        "an instance is an `EngineLink` to the hub, whichever carrier is behind it",
    );
}

/// The hub reaches an instance through an `EngineLink` and nothing else: it names no carrier,
/// and it never opens an instance's data root (concept_and_architecture.md §3.1.1). What a carrier
/// does differently — a spawned daemon's MCP servers, a template's inherited providers written into
/// its settings — is the instance layer's. The dependency rules cannot say this, since the hub
/// depends on that layer and `Carrier` is one of its types; the hub's code can.
#[test]
fn the_hub_names_no_carrier_and_opens_no_data_root() {
    let forbidden = ["Carrier::", "DaemonLink", "InProcessLink", "data_root"];
    let dir = workspace_root().join("crates/hub/src");
    let mut offenders = Vec::new();
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let code = code_of(&std::fs::read_to_string(&path).unwrap_or_default());
        for needle in forbidden {
            if code.contains(needle) {
                offenders.push(format!("{} names `{needle}`", path.display()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "the hub reached past `EngineLink` into a carrier or a data root:\n{}",
        offenders.join("\n")
    );
}

/// Rust source with its comments and the contents of its string literals blanked: what the code
/// names, not what it says. A view the hub sends may well have a `"data_root"` key; the hub
/// reading one may not.
fn code_of(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '"' => {
                out.push('"');
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            chars.next();
                        }
                        '"' => break,
                        _ => {}
                    }
                }
                out.push('"');
            }
            // A char literal: `'"'` must not open a string. A lifetime (`'a`) has no closing quote
            // two characters on, and is copied as it is.
            '\'' => {
                let rest: String = chars.clone().take(3).collect();
                let mut it = rest.chars();
                match (it.next(), it.next(), it.next()) {
                    (Some('\\'), Some(_), Some('\'')) => {
                        chars.nth(2);
                        out.push_str("' '");
                    }
                    (Some(_), Some('\''), _) => {
                        chars.nth(1);
                        out.push_str("' '");
                    }
                    _ => out.push('\''),
                }
            }
            _ => out.push(c),
        }
    }
    out
}

#[test]
fn code_of_sees_code_and_not_what_it_says() {
    let code = code_of(
        "let a = \"data_root\"; // DaemonLink\nlet b = facts.data_root; let q = '\"'; let e = \"\\\"Carrier::\";\nfn f<'a>(x: &'a str) {}",
    );
    assert!(code.contains("facts.data_root"), "{code}");
    assert!(!code.contains("DaemonLink") && !code.contains("Carrier::") && !code.contains("\"data_root\""), "{code}");
    assert!(code.contains("fn f<'a>(x: &'a str)"), "{code}");
}

/// The instance layer builds engines and links to them. It does not know
/// the client-facing protocol exists.
#[test]
fn the_instance_layer_does_not_know_the_client_protocol_exists() {
    assert_ignorant_of(
        "instance",
        &["harnix-transport", "harnix-authz", "harnix-hub", "axum"],
        "it stands up an engine; who talks to it is not its business",
    );
}

/// A module never gets the hub or an engine link (concept §3.1.1, §4.2). The
/// contract it is written against cannot hand it one, because the crate that
/// defines the contract cannot name either.
#[test]
fn the_module_contract_cannot_reach_the_hub() {
    assert_ignorant_of(
        "module",
        &["harnix-hub", "harnix-transport", "harnix-assembly", "daemon"],
        "a module calls through the admission point like everyone else",
    );
    // It does depend on the instance layer, for templates and native
    // contributions — and that crate also holds the links. The contract may
    // name the first and never the second: a module holding a link could run
    // a turn the hub never subscribed to.
    let dir = workspace_root().join("crates/module/src");
    let mut offenders = Vec::new();
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let text = std::fs::read_to_string(entry.path()).unwrap_or_default();
        for needle in [
            "EngineLink",
            "InProcessLink",
            "DaemonLink",
            "DownLink",
            "Factory",
            "build_engine",
        ] {
            if text.contains(needle) {
                offenders.push(format!("{} names `{needle}`", entry.path().display()));
            }
        }
    }
    assert!(offenders.is_empty(), "{}", offenders.join("\n"));
}

/// The built-ins are modules, written against the public module contract and
/// nothing else (concept §3). If they could not be built out of
/// it, the contract would not be worth believing.
#[test]
fn the_builtins_use_only_the_public_module_api() {
    assert_ignorant_of(
        "builtin",
        &[
            "harnix-hub",
            "harnix-authz",
            "harnix-instance",
            "harnix-assembly",
            "harnix-transport",
            "daemon",
        ],
        "a built-in is a module, and a module reaches the process only through its context",
    );
    // And not through the doors the application itself uses to drive a
    // module: those are public for the application's sake, hidden from docs,
    // and a built-in using one would be a built-in with a private channel.
    let dir = workspace_root().join("crates/builtin/src");
    let mut offenders = Vec::new();
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let text = std::fs::read_to_string(entry.path()).unwrap_or_default();
        for needle in [
            "ModuleContext::new",
            "CallContext::new",
            "MethodRegistry::new",
            "EventRegistry::new",
            "RouteRegistry::new",
            "ErasedModule",
            "into_decls",
            "Tap::new",
            "Tap::channel",
            "TapParts",
        ] {
            if text.contains(needle) {
                offenders.push(format!("{} uses `{needle}`", entry.path().display()));
            }
        }
    }
    assert!(offenders.is_empty(), "{}", offenders.join("\n"));
}

/// The flow module (xops-01) is an optional module an application registers, written against the
/// public module contract like a built-in: Harnix's own module API and vocabulary, the engine
/// interface for the scenes it wraps and the tools it adds, and third-party crates. Never a layer
/// below the contract, never Agix, and never the doors the application uses to drive a module.
#[test]
fn the_flow_module_uses_only_the_public_module_api() {
    for name in dependencies("ext/flow") {
        if name.starts_with("harnix") {
            assert!(
                ["harnix-module", "harnix-contract", "harnix-engine-api"].contains(&name.as_str()),
                "`flow` depends on `{name}`; a module reaches the process only through its context"
            );
        }
        assert!(
            !["daemon", "base", "scene", "runtime", "model", "history", "telemetry", "axum"]
                .contains(&name.as_str()),
            "`flow` depends on `{name}`; it reaches the engine through `harnix-engine-api` alone"
        );
    }
    let mut offenders = Vec::new();
    let mut stack = vec![workspace_root().join("crates/ext/flow/src")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            for needle in [
                "ModuleContext::new",
                "CallContext::new",
                "MethodRegistry::new",
                "EventRegistry::new",
                "ErasedModule",
                "into_decls",
                "Tap::new",
                "Tap::channel",
                "TapParts",
            ] {
                if text.contains(needle) {
                    offenders.push(format!("{} uses `{needle}`", path.display()));
                }
            }
        }
    }
    assert!(offenders.is_empty(), "{}", offenders.join("\n"));
}

/// The Harnix app registers the flow module (xops-02 §6), and an application that
/// depends on the `harnix` library must not be made to compile it: the `harnix` binary is its own
/// crate (`crates/cli`), and the library names no flow. (The engine's built-in scenes are another
/// matter: the instance layer builds on them anyway, and the library re-exports them as
/// `harnix::scenes`.)
#[test]
fn the_library_does_not_carry_the_empty_applications_flow_module() {
    assert_ignorant_of(
        "harnix",
        &["harnix-ext-flow"],
        "a library user registers flows if it wants them; the Harnix app is `harnix-cli`",
    );
    let manifest =
        std::fs::read_to_string(workspace_root().join("crates/harnix/Cargo.toml")).unwrap();
    assert!(
        !manifest.contains("[[bin]]"),
        "`crates/harnix` builds no binary: the Harnix app's `harnix` is `crates/cli`"
    );
    let cli = dependencies("cli");
    assert!(
        cli.contains("harnix") && cli.contains("harnix-ext-flow"),
        "{cli:?}"
    );
    let cli_manifest =
        std::fs::read_to_string(workspace_root().join("crates/cli/Cargo.toml")).unwrap();
    assert!(
        cli_manifest.contains("name = \"harnix\""),
        "the binary is still called `harnix`"
    );
}

/// The contract crate is the shared vocabulary. If it grew a dependency on a
/// layer, the layers would be able to reach each other through it and every
/// rule above would be enforceable only on paper.
#[test]
fn the_contract_depends_on_no_layer() {
    let deps = dependencies("contract");
    for name in &deps {
        assert!(
            !name.starts_with("harnix-"),
            "`contract` depends on `{name}`; it is the waist of the hourglass and must stay empty"
        );
    }
}

/// The light client is what a process's own command line, tests and desktop shell link to talk
/// to it. Reaching any layer of the server would make every one of them build the server.
#[test]
fn the_client_knows_only_the_contract() {
    for name in dependencies("client") {
        assert!(
            !name.starts_with("harnix-") || name == "harnix-contract",
            "`client` depends on `{name}`; it may know the contract and nothing of the server"
        );
        assert!(
            !["daemon", "base", "runtime", "axum", "hyper", "reqwest"].contains(&name.as_str()),
            "`client` depends on `{name}`; it is meant to cost its users almost nothing"
        );
    }
}

/// Neither the hub nor the authorizer may name a topology.
///
/// This is concept §3.3.3's fifth invariant, in the only form that survives: "adding
/// a topology does not touch these two layers" is a statement about a diff,
/// and a diff needs a baseline nobody has six months from now. What does not
/// expire is that these layers cannot *say* any of these words — the hub
/// faces "a downstream that can take frames", the authorizer faces "an
/// authenticated subject and a method name", and if either ever needed to
/// know which connection carried something, it would have to name one of
/// these to do it.
///
/// The dependency assertions above stop them reaching a topology through a
/// type. This stops them reaching one through a string.
#[test]
fn neither_the_hub_nor_the_authorizer_names_a_topology() {
    let forbidden = [
        "single_duplex",
        "split_streams",
        "request_only",
        "SingleDuplex",
        "SplitStreams",
        "RequestOnly",
        "Topology",
        "WebSocket",
        "websocket",
        "/events/",
        "/rpc",
        "Sse",
        "http",
        "HTTP",
    ];
    let mut offenders = Vec::new();
    for layer in ["hub", "authz"] {
        let dir = workspace_root().join("crates").join(layer).join("src");
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            // Code only. The prose in these files *says* "knows nothing about
            // HTTP", and a check that cannot tell a comment from a call would
            // fail on the sentence describing the rule it is enforcing.
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let code: String = text
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n");
            for needle in forbidden {
                if code.contains(needle) {
                    offenders.push(format!("{} names `{needle}`", path.display()));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "the topology leaked into a layer that must not perceive one:\n{}",
        offenders.join("\n")
    );
}

/// Harnix links no interpreter, and runs WebAssembly in one place only.
///
/// Extending the *agent* is Agix's: its carriers, its sandbox, its
/// capability gate. Harnix may read a manifest, but it must never run a
/// script a package brought — that is where a second answer to "what may an
/// extension do" would come from — so QuickJS is named nowhere here.
///
/// An application's own backend and tools are Harnix's, and they run in
/// WebAssembly, in `harnix-wasm` and nowhere else (crates/wasm/src/lib.rs).
/// Everything outside that crate reaches a component through its API, which
/// names no wasmtime type, so this is a check on the boundary rather than on
/// a convention.
#[test]
fn harnix_constructs_no_interpreter_and_runs_wasm_only_in_harnix_wasm() {
    let root = workspace_root().join("crates");
    let wasm_crate = root.join("wasm");
    let mut offenders = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs" && e != "toml") {
                continue;
            }
            // This file names them in order to forbid them.
            if path.ends_with("layering.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let mut forbidden = vec!["QuickJsEngine", "rquickjs"];
            if !path.starts_with(&wasm_crate) {
                forbidden.push("wasmtime");
            }
            for needle in forbidden {
                if text.contains(needle) {
                    offenders.push(format!("{} names `{needle}`", path.display()));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "QuickJS is Agix's, and wasmtime is `harnix-wasm`'s alone:\n{}",
        offenders.join("\n")
    );
}

/// `harnix-wasm` is the boundary between wasmtime and Harnix, and it is a
/// leaf: it defines the traits the host implements (`AppHost`, `KvStore`,
/// `HttpFetcher`) instead of depending on the crates that will implement
/// them, and it knows nothing of the engine. Anything else would let a
/// component's runtime reach a layer through a type.
#[test]
fn the_wasm_runtime_depends_on_no_harnix_crate_and_nothing_of_agix() {
    for name in dependencies("wasm") {
        assert!(
            !name.starts_with("harnix"),
            "`wasm` depends on `{name}`; it is a leaf the host plugs services into"
        );
        assert!(
            !["daemon", "base", "model", "history", "scene", "telemetry", "runtime", "plugin"]
                .contains(&name.as_str()),
            "`wasm` depends on `{name}`; an application's VM knows nothing of the engine"
        );
    }
}

/// One crate's normal dependencies (not its dev- or build-dependencies), for every target, by
/// name: whether each is optional.
fn normal_dependencies(crate_dir: &str) -> std::collections::BTreeMap<String, bool> {
    declared(crate_dir)
        .into_iter()
        .filter(|d| d.kind.is_none())
        .map(|d| (d.name, d.optional))
        .collect()
}

/// Agix's crates, by the names Harnix's crates know them by.
const AGIX: &[&str] = &["daemon", "base", "model", "history", "scene", "telemetry", "runtime", "plugin"];

/// Runtime applications are four layers, and each knows only those below it:
///
/// | crate | holds | may not reach |
/// |---|---|---|
/// | `harnix-package` | manifest, package, signatures, store, disclosure, the `harnix-app` tool | a VM, the kernel, Agix, an HTTP framework |
/// | `harnix-wasm-port` | `Platform` and `Control`, and their serde types | anything but the contract |
/// | `harnix-wasm-host` | lifecycle, registry, health, dispatch to the VM, idling | the kernel, the hub, Agix, an HTTP framework |
/// | `harnix-apps` | the embedded glue: the port over the kernel, the Agix plugin, `/apps/<id>/…` | — |
///
/// So the package and the host are the same code in a product's process and in a process of an
/// application's own, and the port between the host and whatever it runs in is all either side
/// sees of the other.
#[test]
fn the_application_layers_reach_only_down() {
    let forbidden_above = ["harnix-module", "harnix-hub", "harnix-builtin", "harnix-instance", "harnix-authz", "harnix-transport", "harnix-assembly", "harnix-apps", "axum", "wasmtime"];
    for (dir, allowed) in [
        ("package", &["harnix-contract", "harnix-wasm"][..]),
        ("wasm-port", &["harnix-contract"][..]),
        ("wasm-host", &["harnix-contract", "harnix-package", "harnix-wasm-port", "harnix-wasm"][..]),
    ] {
        for (name, optional) in normal_dependencies(dir) {
            assert!(
                !forbidden_above.contains(&name.as_str()) && !AGIX.contains(&name.as_str()),
                "`{dir}` depends on `{name}`; it is below the kernel, Agix and HTTP"
            );
            if name.starts_with("harnix") {
                assert!(allowed.contains(&name.as_str()), "`{dir}` depends on `{name}`; it may name {allowed:?}");
            }
            // The packing tool compiles components: only it, behind `compile`, links the engine.
            if dir == "package" && name == "harnix-wasm" {
                assert!(optional, "`package` links `harnix-wasm` only for the packing tool");
            }
        }
    }
    // What else `wasm-port` may name, the runtime only behind `socket`, is `l20_wasm_port`.
    let package = std::fs::read_to_string(workspace_root().join("crates/package/Cargo.toml")).unwrap();
    assert!(
        package.contains("\"dep:harnix-wasm\"") && package.contains("required-features = [\"compile\"]"),
        "the packing tool, and only it, brings `harnix-wasm` into `package`"
    );
}

/// The application runtime is reached through one door. `harnix-wasm` (the engine) is named by
/// the application host alone (and by the packing tool, behind its own feature), the host by the
/// embedded glue and the extension process (`harnix-proc`, which only a product built with `proc`
/// links), and the glue (`harnix-apps`) by the application layer alone, as an optional
/// dependency behind its `apps` feature: a build that does not ask for applications links none of
/// them.
#[test]
fn only_the_apps_feature_links_the_application_runtime() {
    let root = workspace_root().join("crates");
    for entry in std::fs::read_dir(&root).into_iter().flatten().flatten() {
        let dir = entry.file_name().to_string_lossy().to_string();
        if !entry.path().join("Cargo.toml").is_file() {
            continue;
        }
        let deps = normal_dependencies(&dir);
        if let Some(optional) = deps.get("harnix-wasm") {
            assert!(
                dir == "wasm-host" || (dir == "package" && *optional),
                "`{dir}` depends on `harnix-wasm`; only `harnix-wasm-host` may"
            );
        }
        if deps.contains_key("harnix-wasm-host") {
            assert!(
                ["apps", "proc"].contains(&dir.as_str()),
                "`{dir}` depends on `harnix-wasm-host`; only `harnix-apps` and `harnix-proc` may"
            );
        }
        if deps.contains_key("harnix-apps") {
            assert_eq!(dir, "assembly", "`{dir}` depends on `harnix-apps`; only the application layer may");
        }
        for layer in ["harnix-package", "harnix-wasm-port"] {
            if deps.contains_key(layer) {
                assert!(
                    ["wasm-host", "apps", "proc"].contains(&dir.as_str()),
                    "`{dir}` depends on `{layer}`; only the host, the glue and the extension process may"
                );
            }
        }
    }
    let manifest = std::fs::read_to_string(root.join("assembly/Cargo.toml")).unwrap();
    let line = manifest
        .lines()
        .find(|l| l.starts_with("harnix-apps"))
        .expect("harnix-assembly names harnix-apps");
    assert!(line.contains("optional = true"), "`{line}`: harnix-apps must be optional");
    assert!(
        manifest.contains("apps = [\"dep:harnix-apps\"]"),
        "harnix-assembly's `apps` feature is what brings the runtime in"
    );
}

/// Harnix reads exactly one section of a package manifest.
///
/// This is concept §2.2 in the only form that survives a rewrite. "Harnix has no plugin
/// system" is a sentence; what makes it true is that nothing here parses the
/// engine's half of a manifest — no payloads, no capabilities, no scene, no
/// agent types — and nothing runs what a package brought.
///
/// It was two sections for a while, the second being a projection Harnix would
/// execute. A projection has no capabilities, so it introduced no second
/// answer to "what may an extension do" — but it made the rule need a
/// carve-out, and a rule with a carve-out is a rule somebody widens. One
/// section needs no exception.
#[test]
fn harnix_reads_only_its_own_manifest_section() {
    let path = workspace_root().join("crates/ext/plugins/src/packages.rs");
    let text = std::fs::read_to_string(&path).expect("packages.rs");
    let start = text
        .find("struct Sections {")
        .expect("the manifest reader's struct");
    let body = &text[start..start + text[start..].find('}').expect("a closing brace")];

    let fields: BTreeSet<&str> = body
        .lines()
        .filter_map(|line| line.trim().strip_suffix(','))
        .filter_map(|line| line.split(':').next())
        .map(str::trim)
        .filter(|name| !name.is_empty() && !name.starts_with('#'))
        .collect();

    assert_eq!(
        fields,
        BTreeSet::from(["ui"]),
        "the manifest reader grew a section; everything but `ui` is the engine's"
    );

    // And nothing else in the file names one of the engine's. Code only —
    // the prose here *says* "no capabilities", and a check that cannot tell a
    // comment from a field fails on the sentence describing the rule.
    let code: String = text
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    for needle in [
        "WasmPayload",
        "McpPayload",
        "SceneSection",
        "AgentDef",
        "capabilities",
    ] {
        assert!(
            !code.contains(needle),
            "packages.rs names `{needle}`, which is the engine's half of the manifest"
        );
    }
}

/// The same rule's second reader. A plugin application's `agent/` is an Agix package, installed
/// by `plugin.install` under the name and version its `plugin.toml` gives; `harnix-package`
/// reads those two keys of `[plugin]` and nothing else of the engine's manifest.
#[test]
fn a_plugin_applications_agent_package_is_read_for_its_name_and_version_only() {
    let path = workspace_root().join("crates/package/src/package.rs");
    let text = std::fs::read_to_string(&path).expect("package.rs");
    // Code only, and not the tests: they write whole manifests to read them back.
    let code: String = text[..text.find("#[cfg(test)]").unwrap_or(text.len())]
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let start = code.find("fn agent_package(").expect("the agent package's reader");
    let end = start + code[start..].find("\n}").expect("its closing brace");
    let reader = &code[start..end];
    let quoted = |within: &str, call: &str| -> BTreeSet<String> {
        within
            .match_indices(call)
            .filter_map(|(at, _)| {
                let rest = &within[at + call.len()..];
                rest.strip_prefix('"')?.split('"').next().map(str::to_string)
            })
            .collect()
    };
    assert_eq!(quoted(reader, ".get("), BTreeSet::from(["plugin".to_string()]), "a table besides `[plugin]` is read");
    assert_eq!(
        quoted(reader, "field("),
        BTreeSet::from(["name".to_string(), "version".to_string()]),
        "a key of `[plugin]` besides name and version is read"
    );
    // And that reader is the only place the file parses TOML: anything else reading the
    // manifest would be a second truth about what an extension is.
    let outside = format!("{}{}", &code[..start], &code[end..]);
    assert!(
        !outside.contains("toml::"),
        "package.rs parses TOML outside `agent_package`; the engine's manifest is the engine's"
    );
}

/// The rules read what cargo reads: kind and optionality come from cargo, not from which
/// section a line happened to sit under.
#[test]
fn dependencies_are_read_as_cargo_reads_them() {
    let flow = declared("ext/flow");
    let module = flow.iter().find(|d| d.name == "harnix-module").expect("flow names harnix-module");
    assert!(module.optional && module.kind.is_none(), "an optional normal dependency");
    assert!(flow.iter().any(|d| d.name == "tempfile" && d.kind.as_deref() == Some("dev")));
    assert!(normal_dependencies("ext/flow").get("harnix-module") == Some(&true));
    assert!(!normal_dependencies("ext/flow").contains_key("tempfile"), "a dev-dependency is not normal");
}

/// Every crate that depends on a Harnix crate depends on one that exists, and
/// nothing depends on itself. Cheap, and it catches a rename that only half
/// happened.
#[test]
fn every_harnix_dependency_resolves() {
    let crates = [
        "assembly",
        "instance",
        "authz",
        "builtin",
        "cli",
        "client",
        "contract",
        "engine",
        "engine-api",
        "ext/api",
        "ext/devices",
        "ext/flow",
        "ext/observability",
        "ext/plugins",
        "ext/settings",
        "harnix",
        "host-api",
        "hub",
        "kernel",
        "module",
        "router",
        "testapp",
        "transport",
        "wasm",
        "package",
        "wasm-port",
        "wasm-host",
        "apps",
        "supervise",
        "proc",
    ];
    let known: BTreeSet<String> = crates
        .iter()
        .map(|c| {
            if *c == "harnix" {
                "harnix".to_string()
            } else if *c == "ext/api" {
                "harnix-ext".to_string()
            } else {
                format!("harnix-{}", c.replace('/', "-"))
            }
        })
        .collect();
    for dir in crates {
        for dep in dependencies(dir) {
            if dep.starts_with("harnix-") {
                assert!(
                    known.contains(&dep),
                    "`{dir}` depends on unknown crate `{dep}`"
                );
                assert_ne!(dep, format!("harnix-{dir}"), "`{dir}` depends on itself");
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The layering rules of 0.5.0.
//
// Each rule is a test of its own: L (what a crate may depend on), C (what its source may say),
// and the global ones. A rule the code does not meet yet carries an ignore naming the stage that
// meets it, so what is left to do is what `cargo test -p harnix --test layering -- --ignored`
// still fails, and a stage that finishes removes the ignores it earned. A rule over a crate that
// does not exist yet fails by saying so, which is why it is ignored until the crate does.
// ─────────────────────────────────────────────────────────────────────────────────────────────
mod rules {
    use super::*;
    use serde_json::Value;

    /// The crates of the layering, by package name: the one table a rename touches.
    mod krate {
        pub const CONTRACT: &str = "harnix-contract";
        pub const KERNEL: &str = "harnix-kernel";
        pub const AUTHZ: &str = "harnix-authz";
        pub const ROUTER: &str = "harnix-router";
        pub const TRANSPORT: &str = "harnix-transport";
        pub const HUB: &str = "harnix-hub";
        pub const INSTANCE: &str = "harnix-instance";
        pub const ENGINE_API: &str = "harnix-engine-api";
        pub const ENGINE: &str = "harnix-engine";
        pub const SUPERVISE: &str = "harnix-supervise";
        pub const ASSEMBLY: &str = "harnix-assembly";
        pub const EXT: &str = "harnix-ext";
        pub const HOST_API: &str = "harnix-host-api";
        pub const PACKAGES: &str = "harnix-ext-packages";
        pub const CARRIER_WASM: &str = "harnix-carrier-wasm";
        pub const CARRIER_PROC: &str = "harnix-carrier-proc";
        pub const PROC: &str = "harnix-proc";
        pub const PACKAGE: &str = "harnix-package";
        pub const WASM_PORT: &str = "harnix-wasm-port";
        pub const WASM_HOST: &str = "harnix-wasm-host";
        pub const WASM: &str = "harnix-wasm";
        pub const CLIENT: &str = "harnix-client";
        pub const FACADE: &str = "harnix";
        pub const CLI: &str = "harnix-cli";
        pub const TESTAPP: &str = "harnix-testapp";
        /// The first-party extensions: `harnix-ext-<id>`, each a crate of its own.
        pub const FIRST_PARTY_PREFIX: &str = "harnix-ext-";
        pub const FIRST_PARTY: &[&str] = &[
            "harnix-ext-system",
            "harnix-ext-harnix-app",
            "harnix-ext-settings",
            "harnix-ext-plugins",
            "harnix-ext-observability",
            "harnix-ext-devices",
            "harnix-ext-flow",
        ];
        /// Every crate the workspace may have (C12).
        pub const ALL: &[&str] = &[
            CONTRACT, KERNEL, AUTHZ, ROUTER, TRANSPORT, HUB, INSTANCE, ENGINE_API, ENGINE, SUPERVISE,
            ASSEMBLY, EXT, HOST_API, PACKAGES, CARRIER_WASM, CARRIER_PROC, PROC, PACKAGE, WASM_PORT,
            WASM_HOST, WASM, CLIENT, FACADE, CLI, TESTAPP,
            "harnix-ext-system",
            "harnix-ext-harnix-app",
            "harnix-ext-settings",
            "harnix-ext-plugins",
            "harnix-ext-observability",
            "harnix-ext-devices",
            "harnix-ext-flow",
        ];
        /// The skeleton: what no first-party extension, carrier or extension process is part of.
        pub const SKELETON: &[&str] = &[
            CONTRACT, KERNEL, AUTHZ, ROUTER, TRANSPORT, HUB, INSTANCE, ENGINE_API, ENGINE, SUPERVISE,
            ASSEMBLY,
        ];
    }
    use krate::*;

    fn is_harnix(name: &str) -> bool {
        name == FACADE || name.starts_with("harnix-")
    }

    // ── reading the workspace ──────────────────────────────────────────────────────────────

    fn packages() -> &'static Vec<Value> {
        metadata()["packages"].as_array().expect("packages")
    }

    fn member_names() -> Vec<String> {
        packages().iter().filter_map(|p| p["name"].as_str().map(str::to_string)).collect()
    }

    fn package(name: &str) -> &'static Value {
        packages()
            .iter()
            .find(|p| p["name"].as_str() == Some(name))
            .unwrap_or_else(|| panic!("`{name}` is not a workspace member (yet)"))
    }

    /// A package's normal dependencies: the name it knows each by, and whether it is optional.
    fn normal(name: &str) -> Vec<(String, bool)> {
        package(name)["dependencies"]
            .as_array()
            .expect("dependencies")
            .iter()
            .filter(|d| d["kind"].is_null())
            .map(|d| {
                (
                    d["rename"].as_str().or(d["name"].as_str()).unwrap_or_default().to_string(),
                    d["optional"].as_bool().unwrap_or(false),
                )
            })
            .collect()
    }

    fn crate_dir(name: &str) -> PathBuf {
        Path::new(package(name)["manifest_path"].as_str().expect("manifest_path"))
            .parent()
            .expect("a manifest has a directory")
            .to_path_buf()
    }

    /// Every file under `dir` with one of these extensions, build output and dependencies left out.
    fn files_under(dir: &Path, extensions: &[&str]) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if path.is_dir() {
                    if !["target", "node_modules", "dist"].contains(&name.as_str()) {
                        stack.push(path);
                    }
                } else if path.extension().is_some_and(|e| extensions.iter().any(|x| e == *x)) {
                    out.push(path);
                }
            }
        }
        out.sort();
        out
    }

    /// A crate's production source: `src/**/*.rs`, each file up to its `#[cfg(test)]`.
    fn sources(name: &str) -> Vec<(PathBuf, String)> {
        files_under(&crate_dir(name).join("src"), &["rs"])
            .into_iter()
            .map(|path| {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                let end = text.find("#[cfg(test)]").unwrap_or(text.len());
                (path, text[..end].to_string())
            })
            .collect()
    }

    fn rel(path: &Path) -> String {
        path.strip_prefix(workspace_root()).unwrap_or(path).display().to_string()
    }

    /// Rust source without its comments, and with the contents of its string literals either kept
    /// or blanked: the rules that name a string literal read the first, the rest the second.
    fn lex(text: &str, keep_strings: bool) -> String {
        let chars: Vec<char> = text.chars().collect();
        let mut out = String::with_capacity(text.len());
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            let next = chars.get(i + 1).copied();
            if c == '/' && next == Some('/') {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            } else if c == '/' && next == Some('*') {
                let mut depth = 0;
                while i < chars.len() {
                    if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                        depth += 1;
                        i += 2;
                    } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                        depth -= 1;
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        if chars[i] == '\n' {
                            out.push('\n');
                        }
                        i += 1;
                    }
                }
            } else if c == 'r' && matches!(next, Some('"') | Some('#')) && {
                let mut j = i + 1;
                while chars.get(j) == Some(&'#') {
                    j += 1;
                }
                chars.get(j) == Some(&'"')
            } {
                // A raw string: r"…", r#"…"#.
                let mut hashes = 0;
                i += 1;
                while chars[i] == '#' {
                    hashes += 1;
                    i += 1;
                }
                i += 1;
                out.push('"');
                loop {
                    if i >= chars.len() {
                        break;
                    }
                    if chars[i] == '"' && (0..hashes).all(|k| chars.get(i + 1 + k) == Some(&'#')) {
                        i += 1 + hashes;
                        break;
                    }
                    if keep_strings {
                        out.push(chars[i]);
                    }
                    i += 1;
                }
                out.push('"');
            } else if c == '"' {
                out.push('"');
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' {
                        if keep_strings {
                            out.push(chars[i]);
                        }
                        i += 1;
                    }
                    if i < chars.len() {
                        if keep_strings {
                            out.push(chars[i]);
                        }
                        i += 1;
                    }
                }
                out.push('"');
                i += 1;
            } else if c == '\'' {
                // A char literal (`'"'`, `'\n'`) must not open a string; a lifetime has no closing
                // quote two characters on and is copied as it is.
                let a = chars.get(i + 1).copied();
                let b = chars.get(i + 2).copied();
                let d = chars.get(i + 3).copied();
                if a == Some('\\') && d == Some('\'') {
                    out.push_str("' '");
                    i += 4;
                } else if a.is_some() && b == Some('\'') {
                    out.push_str("' '");
                    i += 3;
                } else {
                    out.push('\'');
                    i += 1;
                }
            } else {
                out.push(c);
                i += 1;
            }
        }
        out
    }

    /// TypeScript or JavaScript source without its comments (a `//` after a colon is a URL).
    fn strip_ts_comments(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut chars = text.chars().peekable();
        let mut prev = ' ';
        while let Some(c) = chars.next() {
            if c == '/' && chars.peek() == Some(&'/') && prev != ':' {
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            } else if c == '/' && chars.peek() == Some(&'*') {
                chars.next();
                let mut last = ' ';
                for c in chars.by_ref() {
                    if last == '*' && c == '/' {
                        break;
                    }
                    last = c;
                }
            } else {
                out.push(c);
            }
            prev = c;
        }
        out
    }

    // ── dependency rules ───────────────────────────────────────────────────────────────────

    fn agix_and(more: &[&'static str]) -> Vec<&'static str> {
        AGIX.iter().copied().chain(more.iter().copied()).collect()
    }

    /// `krate`'s normal dependencies on Harnix crates are among `allowed`, it depends on none of
    /// `forbidden`, and the ones in `optional_only` are optional wherever it has them.
    fn depends(krate: &str, allowed: &[&str], forbidden: &[&str], optional_only: &[&str]) {
        for (dep, optional) in normal(krate) {
            assert!(
                !is_harnix(&dep) || allowed.contains(&dep.as_str()),
                "`{krate}` depends on `{dep}`; of Harnix it may name {allowed:?}"
            );
            assert!(!forbidden.contains(&dep.as_str()), "`{krate}` depends on `{dep}`, and it must not");
            assert!(
                optional || !optional_only.contains(&dep.as_str()),
                "`{krate}` depends on `{dep}`, and only optionally"
            );
        }
    }

    /// Who depends on `target` (normally), among all workspace members: name and optionality.
    fn dependents(target: &str) -> Vec<(String, bool)> {
        member_names()
            .into_iter()
            .flat_map(|name| {
                normal(&name).into_iter().filter(|(dep, _)| dep == target).map(move |(_, optional)| (name.clone(), optional))
            })
            .collect()
    }

    /// `target` is depended on by `allowed` crates alone (`optional` ones only optionally).
    fn only_used_by(target: &str, allowed: &[&str], optional: &[&str]) {
        for (user, is_optional) in dependents(target) {
            assert!(allowed.contains(&user.as_str()), "`{user}` depends on `{target}`; only {allowed:?} may");
            assert!(
                is_optional || !optional.contains(&user.as_str()),
                "`{user}` depends on `{target}`, and only optionally"
            );
        }
    }

    /// The first-party extensions that exist, and nothing else of the `harnix-ext-` family but
    /// the host component that manages packages.
    fn first_party_members() -> Vec<String> {
        member_names()
            .into_iter()
            .filter(|n| n.starts_with(FIRST_PARTY_PREFIX) && n != PACKAGES)
            .collect()
    }

    #[test]
    fn l01_contract() {
        depends(CONTRACT, &[], &agix_and(&["tokio"]), &[]);
    }

    #[test]
    fn l02_kernel() {
        depends(KERNEL, &[CONTRACT], &agix_and(&["axum", "http"]), &[]);
    }

    #[test]
    fn l03_authz() {
        depends(AUTHZ, &[CONTRACT, KERNEL], &agix_and(&["axum"]), &[]);
    }

    #[test]
    fn l04_router() {
        depends(ROUTER, &[CONTRACT, KERNEL, AUTHZ], &agix_and(&["axum"]), &[]);
    }

    #[test]
    fn l05_transport() {
        let forbidden = agix_and(&[ROUTER, HUB, INSTANCE, ENGINE, ENGINE_API]);
        depends(TRANSPORT, &[CONTRACT, KERNEL, AUTHZ], &forbidden, &[]);
    }

    #[test]
    fn l06_hub() {
        let forbidden = agix_and(&[INSTANCE, ENGINE, "axum", "bytes", "futures"]);
        depends(HUB, &[CONTRACT, KERNEL, ENGINE_API], &forbidden, &[]);
    }

    #[test]
    fn l07_instance() {
        depends(INSTANCE, &[CONTRACT, KERNEL, ENGINE_API, ENGINE, SUPERVISE], &agix_and(&["axum"]), &[]);
    }

    #[test]
    fn l08_engine_api() {
        depends(ENGINE_API, &[CONTRACT], &agix_and(&["axum"]), &[]);
    }

    #[test]
    fn l09_engine() {
        depends(ENGINE, &[CONTRACT, ENGINE_API], &["axum"], &[]);
    }

    #[test]
    fn l10_supervise() {
        depends(SUPERVISE, &[], &agix_and(&[]), &[]);
    }

    #[test]
    #[ignore = "0.5.0：P15"]
    fn l11_assembly() {
        let allowed = [
            CONTRACT, KERNEL, AUTHZ, ROUTER, TRANSPORT, HUB, INSTANCE, ENGINE_API, ENGINE, EXT, HOST_API, SUPERVISE,
        ];
        depends(ASSEMBLY, &allowed, &agix_and(&[]), &[]);
    }

    #[test]
    fn l12_ext() {
        depends(EXT, &[CONTRACT, KERNEL, ENGINE_API], &agix_and(&[ENGINE, "wasmtime"]), &["axum"]);
    }

    #[test]
    fn l13_host_api() {
        depends(HOST_API, &[CONTRACT, EXT, ENGINE_API], &agix_and(&[KERNEL, ENGINE]), &[]);
    }

    #[test]
    #[ignore = "0.5.0：P12"]
    fn l14_first_party_extensions() {
        let members = first_party_members();
        assert!(!members.is_empty(), "there is no first-party extension crate (`harnix-ext-<id>`) yet");
        for name in members {
            depends(&name, &[EXT], &agix_and(&[]), &["axum"]);
        }
    }

    #[test]
    #[ignore = "0.5.0：P12"]
    fn l15_packages() {
        let forbidden = agix_and(&[CARRIER_WASM, CARRIER_PROC, WASM, WASM_HOST]);
        depends(PACKAGES, &[EXT, HOST_API, PACKAGE], &forbidden, &[]);
    }

    #[test]
    #[ignore = "0.5.0：P15"]
    fn l16_carrier_wasm() {
        depends(CARRIER_WASM, &[EXT, HOST_API, WASM_HOST, WASM_PORT, PACKAGE], &agix_and(&[]), &[]);
    }

    #[test]
    #[ignore = "0.5.0：P19"]
    fn l17_carrier_proc() {
        let forbidden = agix_and(&[WASM, WASM_HOST, PROC, ENGINE]);
        depends(CARRIER_PROC, &[EXT, HOST_API, WASM_PORT, PACKAGE], &forbidden, &[]);
    }

    #[test]
    fn l18_proc() {
        depends(PROC, &[WASM_HOST, WASM, WASM_PORT, PACKAGE, ENGINE_API, ENGINE], &[], &[]);
    }

    #[test]
    fn l19_package() {
        depends(PACKAGE, &[CONTRACT, WASM], &agix_and(&[KERNEL, EXT]), &[WASM]);
    }

    #[test]
    fn l20_wasm_port() {
        depends(WASM_PORT, &[CONTRACT], &[], &[]);
        for (dep, optional) in normal(WASM_PORT) {
            if is_harnix(&dep) {
                continue;
            }
            assert!(
                ["async-trait", "serde", "serde_json", "tokio", "tokio-util"].contains(&dep.as_str()),
                "`{WASM_PORT}` depends on `{dep}`: the boundary is serde types and async traits"
            );
            assert!(
                optional || !["tokio", "tokio-util"].contains(&dep.as_str()),
                "`{WASM_PORT}` depends on `{dep}`, and only behind the `socket` feature"
            );
        }
    }

    #[test]
    fn l21_wasm_host() {
        depends(WASM_HOST, &[CONTRACT, PACKAGE, WASM_PORT, WASM], &agix_and(&[KERNEL, EXT]), &[]);
    }

    #[test]
    fn l22_wasm() {
        depends(WASM, &[], &agix_and(&[]), &[]);
    }

    #[test]
    fn l23_client() {
        depends(CLIENT, &[CONTRACT], &agix_and(&["axum", "hyper", "reqwest"]), &[]);
    }

    #[test]
    #[ignore = "0.5.0：P20"]
    fn l24_facade() {
        // Always: the assembly, the extension interface and the engine's. Optionally: the
        // engine, every first-party extension and host component, both carriers, the process.
        let always = [ASSEMBLY, EXT, ENGINE_API];
        for (dep, optional) in normal(FACADE) {
            if !is_harnix(&dep) || always.contains(&dep.as_str()) {
                continue;
            }
            let may_be_optional =
                dep == ENGINE || dep.starts_with(FIRST_PARTY_PREFIX) || dep.starts_with("harnix-carrier-") || dep == PROC;
            assert!(may_be_optional, "`{FACADE}` depends on `{dep}`; of Harnix it names {always:?} and optional extras");
            assert!(optional, "`{FACADE}` depends on `{dep}`, and only optionally");
        }
        let _ = package(ASSEMBLY);
    }

    #[test]
    fn l25_agix_is_the_engines_alone() {
        assert!(!member_names().iter().all(|n| n != ENGINE), "`{ENGINE}` does not exist (yet)");
        for name in member_names() {
            for (dep, _) in normal(&name) {
                if AGIX.contains(&dep.as_str()) {
                    assert_eq!(name, ENGINE, "`{name}` depends on Agix's `{dep}`; only `{ENGINE}` does");
                }
            }
        }
    }

    #[test]
    #[ignore = "0.5.0：P15"]
    fn l25_the_wasm_runtime_is_reached_through_the_host() {
        only_used_by(WASM, &[WASM_HOST, PROC, PACKAGE], &[PACKAGE]);
        only_used_by(WASM_HOST, &[CARRIER_WASM, PROC], &[]);
        assert!(member_names().iter().any(|n| n == WASM_HOST), "`{WASM_HOST}` does not exist (yet)");
    }

    #[test]
    #[ignore = "0.5.0：P20"]
    fn l25_the_process_library_is_the_facades_alone() {
        only_used_by(PROC, &[FACADE], &[FACADE]);
        assert!(member_names().iter().any(|n| n == PROC), "`{PROC}` does not exist (yet)");
    }

    #[test]
    fn l25_supervision_is_the_instance_layers_and_the_assemblys() {
        only_used_by(SUPERVISE, &[INSTANCE, ASSEMBLY], &[]);
        assert!(member_names().iter().any(|n| n == SUPERVISE), "`{SUPERVISE}` does not exist (yet)");
    }

    #[test]
    fn l25_the_host_api_is_the_assemblys_and_the_host_components() {
        only_used_by(HOST_API, &[ASSEMBLY, PACKAGES, CARRIER_WASM, CARRIER_PROC], &[]);
        assert!(member_names().iter().any(|n| n == HOST_API), "`{HOST_API}` does not exist (yet)");
    }

    #[test]
    fn l25_the_router_is_the_assemblys_alone() {
        only_used_by(ROUTER, &[ASSEMBLY], &[]);
        assert!(member_names().iter().any(|n| n == ROUTER), "`{ROUTER}` does not exist (yet)");
    }

    // ── source rules ───────────────────────────────────────────────────────────────────────

    /// Needles that must not appear in the production source of `scope`, read with strings kept
    /// (`true`) or blanked (`false`), `exempt` files left out. A scope crate that is not there
    /// fails the rule: a rule over nothing proves nothing.
    fn says_nothing_of(scope: &[&str], needles: &[&str], keep_strings: bool, exempt: &dyn Fn(&Path) -> bool) {
        let mut offenders = Vec::new();
        for krate in scope {
            for (path, text) in sources(krate) {
                if exempt(&path) {
                    continue;
                }
                let code = lex(&text, keep_strings);
                for needle in needles {
                    if code.contains(needle) {
                        offenders.push(format!("{} says `{needle}`", rel(&path)));
                    }
                }
            }
        }
        assert!(offenders.is_empty(), "\n{}", offenders.join("\n"));
    }

    fn nobody(_: &Path) -> bool {
        false
    }

    /// The identifiers of the first-party extensions and their spaces.
    const FIRST_PARTY_IDS: &[&str] = &[
        "system", "sessions", "projects", "flow", "harnix-app", "settings", "plugins", "packages", "devices",
        "observability",
    ];

    fn first_party_literals() -> Vec<String> {
        FIRST_PARTY_IDS.iter().map(|id| format!("\"{id}\"")).collect()
    }

    fn assert_knows_no_first_party(scope: &[&str]) {
        let literals = first_party_literals();
        let needles: Vec<&str> = literals.iter().map(String::as_str).collect();
        // The development-period alias table names them by necessity; it goes with the aliases.
        says_nothing_of(scope, &needles, true, &|p| p.ends_with("assembly/src/aliases.rs") || p.ends_with("src/aliases.rs"));
    }

    #[test]
    #[ignore = "0.5.0：P4"]
    fn c01_the_skeleton_knows_no_first_party_extension_but_the_assembly() {
        let scope: Vec<&str> = SKELETON.iter().copied().filter(|c| *c != ASSEMBLY).collect();
        assert_knows_no_first_party(&scope);
        assert!(member_names().iter().any(|n| n == ASSEMBLY), "`{ASSEMBLY}` does not exist (yet)");
    }

    #[test]
    #[ignore = "0.5.0：P13"]
    fn c01_the_assembly_knows_none_either() {
        assert_knows_no_first_party(&[ASSEMBLY]);
    }

    #[test]
    fn c02_nothing_dispatches_on_a_name_prefix() {
        let needles = [
            "starts_with(\"app.\")",
            "starts_with(\"harnix.\")",
            "starts_with(\"engine.\")",
            "strip_prefix(\"harnix.\")",
        ];
        // The one place that knows how a name is built.
        says_nothing_of(&[ROUTER, HUB, ASSEMBLY, INSTANCE, TRANSPORT], &needles, true, &|p| p.ends_with("router/src/names.rs"));
    }

    #[test]
    fn c03_the_private_call_channels_are_gone() {
        let needles = ["__client", "__chain"];
        let mut offenders = Vec::new();
        for name in member_names() {
            for (path, text) in sources(&name) {
                if needles.iter().any(|n| lex(&text, true).contains(n)) {
                    offenders.push(rel(&path));
                }
            }
        }
        let mut dirs = vec![workspace_root().join("tests").join("live")];
        for entry in std::fs::read_dir(workspace_root().join("ui/packages")).into_iter().flatten().flatten() {
            dirs.push(entry.path().join("src"));
        }
        for dir in dirs {
            for path in files_under(&dir, &["ts", "tsx", "mjs"]) {
                let text = strip_ts_comments(&std::fs::read_to_string(&path).unwrap_or_default());
                if needles.iter().any(|n| text.contains(n)) {
                    offenders.push(rel(&path));
                }
            }
        }
        assert!(offenders.is_empty(), "\n{}", offenders.join("\n"));
    }

    /// The names Agix's methods and events go by, read from the engine's translation table: the
    /// string constants of `translate.rs`.
    fn agix_names() -> Vec<String> {
        let path = crate_dir(ENGINE).join("src/translate.rs");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let names: Vec<String> = lex(&text, true)
            .lines()
            .filter(|l| l.trim_start().starts_with("pub const ") || l.trim_start().starts_with("const "))
            .filter_map(|l| l.split('"').nth(1).map(str::to_string))
            .filter(|n| !n.is_empty())
            .collect();
        assert!(!names.is_empty(), "{} has no name constants", path.display());
        names
    }

    #[test]
    #[ignore = "0.5.0：P13"]
    fn c04_the_hub_knows_no_carrier_no_data_root_and_no_agix_name() {
        says_nothing_of(
            &[HUB],
            &["Carrier", "DaemonLink", "InProcessLink", "data_root", "InstanceRegistry::", "EngineContribution::"],
            false,
            &nobody,
        );
        let quoted: Vec<String> = agix_names().into_iter().map(|n| format!("\"{n}\"")).collect();
        let needles: Vec<&str> = quoted.iter().map(String::as_str).collect();
        says_nothing_of(&[HUB], &needles, true, &nobody);
    }

    #[test]
    fn c05_no_topology_is_named_by_the_hub_the_router_or_the_authorizer() {
        let needles = [
            "single_duplex", "split_streams", "request_only", "SingleDuplex", "SplitStreams", "RequestOnly",
            "Topology", "WebSocket", "websocket", "/events/", "/rpc", "Sse", "http", "HTTP",
        ];
        says_nothing_of(&[HUB, ROUTER, AUTHZ], &needles, true, &nobody);
    }

    /// `doc(hidden)` and `agix::` in the extension interface's crates, and `axum` outside the one
    /// file that has it, behind its feature.
    fn assert_interface_crate_is_clean(name: &str) {
        says_nothing_of(&[name], &["doc(hidden)", "agix::"], false, &nobody);
        let mut offenders = Vec::new();
        for (path, text) in sources(name) {
            let in_the_one_file = path.ends_with("src/http/axum.rs");
            let code = lex(&text, true);
            let lines: Vec<&str> = code.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                if !line.contains("axum") || line.contains("feature = \"axum\"") {
                    continue;
                }
                let gated = i > 0 && lines[i - 1].contains("feature = \"axum\"");
                if !(in_the_one_file || (gated && line.contains("mod axum"))) {
                    offenders.push(format!("{}:{} names axum outside `http/axum.rs` behind its feature", rel(&path), i + 1));
                }
            }
        }
        assert!(offenders.is_empty(), "\n{}", offenders.join("\n"));
    }

    #[test]
    fn c06_the_engine_interface_hides_nothing_and_names_no_agix() {
        assert_interface_crate_is_clean(ENGINE_API);
    }

    #[test]
    fn c06_the_extension_interface_hides_nothing_and_names_no_agix() {
        assert_interface_crate_is_clean(EXT);
    }

    fn host_components_and_first_party() -> Vec<String> {
        let mut scope = first_party_members();
        scope.extend([PACKAGES, CARRIER_WASM, CARRIER_PROC].iter().map(|s| s.to_string()));
        scope
    }

    #[test]
    #[ignore = "0.5.0：P12"]
    fn c07_extensions_and_host_components_reach_neither_agix_nor_the_kernel() {
        let owned = host_components_and_first_party();
        let scope: Vec<&str> = owned.iter().map(String::as_str).collect();
        assert!(first_party_members().len() >= FIRST_PARTY.len(), "the first-party extensions are not all crates yet");
        says_nothing_of(&scope, &["agix::", "harnix_kernel", "harnix_engine::"], false, &nobody);
    }

    #[test]
    fn c08_the_agix_escape_hatch_is_in_two_places_behind_one_feature() {
        let gated = |text: &str| {
            let code = lex(text, true);
            let lines: Vec<&str> = code.lines().collect();
            lines.iter().enumerate().filter(|(_, l)| l.contains("pub mod agix")).map(|(i, _)| {
                i > 0 && lines[i - 1].replace(' ', "").contains("cfg(feature=\"agix\")")
            }).collect::<Vec<bool>>()
        };
        let mut seen = Vec::new();
        for name in member_names() {
            for (path, text) in sources(&name) {
                for ok in gated(&text) {
                    assert!(ok, "{} has `pub mod agix` outside `cfg(feature = \"agix\")`", rel(&path));
                    seen.push(name.clone());
                }
            }
        }
        seen.sort();
        let mut expected = vec![ENGINE.to_string(), FACADE.to_string()];
        expected.sort();
        assert_eq!(seen, expected, "`pub mod agix` is in the engine and the facade, and nowhere else");
    }

    /// The engine interface's two unstable doors — an engine's own scene as a layer's base, an
    /// engine's own hook — are opened by the `agix` escape hatch alone; the extension host passes
    /// on hooks it was given, switched off with their extension, and opens nothing.
    #[test]
    fn c08_the_engine_interfaces_unstable_doors_are_the_escape_hatchs() {
        let allowed = ["crates/engine/src/agix.rs", "crates/assembly/src/host/contribution.rs"];
        let mut offenders = Vec::new();
        for name in member_names() {
            for (path, text) in sources(&name) {
                let code = lex(&text, false).replace(' ', "");
                let opens = ["SceneBase::foreign(", ".foreign_hook(", ".foreign_hook_while("].iter().any(|n| code.contains(n));
                if opens && !allowed.iter().any(|a| rel(&path) == *a) {
                    offenders.push(format!("{} opens an unstable door of the engine interface", rel(&path)));
                }
            }
        }
        assert!(offenders.is_empty(), "\n{}", offenders.join("\n"));
    }

    #[test]
    #[ignore = "0.5.0：P15"]
    fn c09_only_the_facade_and_the_cli_switch_on_a_carrier() {
        let mut scope: Vec<String> = SKELETON.iter().map(|s| s.to_string()).collect();
        scope.extend([EXT, HOST_API].iter().map(|s| s.to_string()));
        scope.extend(host_components_and_first_party());
        let mut offenders = Vec::new();
        for name in &scope {
            for (path, text) in sources(name) {
                let code = lex(&text, true).replace(' ', "");
                for needle in ["feature=\"wasm\"", "feature=\"proc\""] {
                    if code.contains(needle) {
                        offenders.push(format!("{} says `{needle}`", rel(&path)));
                    }
                }
            }
        }
        assert!(offenders.is_empty(), "\n{}", offenders.join("\n"));
    }

    #[test]
    fn c10_no_interpreter_is_built_and_wasmtime_is_the_wasm_crates_alone() {
        let mut offenders = Vec::new();
        for name in member_names() {
            let dir = crate_dir(&name);
            for path in files_under(&dir, &["rs", "toml"]) {
                if path.ends_with("layering.rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                let mut forbidden = vec!["QuickJsEngine", "rquickjs"];
                if name != WASM {
                    forbidden.push("wasmtime");
                }
                for needle in forbidden {
                    if text.contains(needle) {
                        offenders.push(format!("{} names `{needle}`", rel(&path)));
                    }
                }
            }
        }
        assert!(offenders.is_empty(), "\n{}", offenders.join("\n"));
    }

    /// Harnix reads exactly the `[[ui]]` section of an Agix package's manifest.
    fn assert_reads_only_ui(path: &Path) {
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let start = text.find("struct Sections {").expect("the manifest reader's struct");
        let body = &text[start..start + text[start..].find('}').expect("a closing brace")];
        let fields: BTreeSet<&str> = body
            .lines()
            .filter_map(|line| line.trim().strip_suffix(','))
            .filter_map(|line| line.split(':').next())
            .map(str::trim)
            .filter(|name| !name.is_empty() && !name.starts_with('#'))
            .collect();
        assert_eq!(fields, BTreeSet::from(["ui"]), "the manifest reader grew a section; everything but `ui` is the engine's");
        let code = lex(&text, false);
        for needle in ["WasmPayload", "McpPayload", "SceneSection", "AgentDef", "capabilities"] {
            assert!(!code.contains(needle), "{} names `{needle}`, the engine's half of the manifest", path.display());
        }
    }

    /// The other reader: `agent/`'s `[plugin]`, for its name and version.
    fn assert_reads_only_plugin_name_and_version(path: &Path) {
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let code = lex(&text[..text.find("#[cfg(test)]").unwrap_or(text.len())], true);
        let start = code.find("fn agent_package(").expect("the agent package's reader");
        let end = start + code[start..].find("\n}").expect("its closing brace");
        let reader = &code[start..end];
        let quoted = |call: &str| -> BTreeSet<String> {
            reader
                .match_indices(call)
                .filter_map(|(at, _)| reader[at + call.len()..].strip_prefix('"')?.split('"').next().map(str::to_string))
                .collect()
        };
        assert_eq!(quoted(".get("), BTreeSet::from(["plugin".to_string()]), "a table besides `[plugin]` is read");
        assert_eq!(quoted("field("), BTreeSet::from(["name".to_string(), "version".to_string()]), "a key besides name and version is read");
        let outside = format!("{}{}", &code[..start], &code[end..]);
        assert!(!outside.contains("toml::"), "{} parses TOML outside `agent_package`", path.display());
    }

    #[test]
    fn c11_the_plugins_extension_reads_only_the_ui_section() {
        assert_reads_only_ui(&crate_dir("harnix-ext-plugins").join("src/packages.rs"));
    }

    #[test]
    fn c11_the_package_crate_reads_only_an_agent_plugins_name_and_version() {
        assert_reads_only_plugin_name_and_version(&crate_dir(PACKAGE).join("src/package.rs"));
    }

    #[test]
    #[ignore = "0.5.0：P15"]
    fn c12_every_workspace_crate_is_known_and_every_dependency_resolves() {
        let members = member_names();
        for name in &members {
            assert!(ALL.contains(&name.as_str()), "`{name}` is a workspace crate nobody listed");
            for (dep, _) in normal(name) {
                assert!(!is_harnix(&dep) || members.contains(&dep), "`{name}` depends on `{dep}`, which is not a workspace crate");
                assert_ne!(&dep, name, "`{name}` depends on itself");
            }
        }
    }

    #[test]
    fn c13_the_kernel_is_named_only_inside_the_extension_interfaces_internal_module() {
        let dir = crate_dir(EXT).join("src");
        assert!(dir.join("internal").is_dir() || dir.join("internal.rs").is_file(), "`internal` does not exist (yet)");
        let mut offenders = Vec::new();
        for (path, text) in sources(EXT) {
            let code = lex(&text, true);
            let inside = path.starts_with(dir.join("internal"));
            if !inside && code.contains("harnix_kernel") {
                offenders.push(format!("{} names `harnix_kernel` outside `internal`", rel(&path)));
            }
            if code.replace(' ', "").contains("pubuseharnix_kernel") {
                offenders.push(format!("{} re-exports `harnix_kernel`", rel(&path)));
            }
        }
        let lib = lex(&std::fs::read_to_string(dir.join("lib.rs")).unwrap_or_default(), true);
        assert!(!lib.contains("pub mod internal"), "`internal` is public");
        assert!(offenders.is_empty(), "\n{}", offenders.join("\n"));
    }

    /// A public struct or enum in `text` that the rule asks `#[non_exhaustive]` of and lacks it:
    /// every public enum, and every struct with a public field (a unit struct, or one whose
    /// fields are all private, cannot be built outside its crate and needs nothing).
    fn lacking_non_exhaustive(text: &str) -> Vec<String> {
        let code = lex(text, false);
        let lines: Vec<&str> = code.lines().collect();
        let mut missing = Vec::new();
        let mut attrs: Vec<&str> = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            let t = line.trim();
            if t.starts_with("#[") || t.starts_with("#![") {
                attrs.push(t);
                continue;
            }
            if t.is_empty() {
                continue;
            }
            let declared = t
                .strip_prefix("pub struct ")
                .map(|r| ("struct", r))
                .or_else(|| t.strip_prefix("pub enum ").map(|r| ("enum", r)));
            if let Some((kind, rest)) = declared {
                let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                let marked = attrs.iter().any(|a| a.contains("non_exhaustive"));
                let needs = if kind == "enum" {
                    true
                } else {
                    // From the declaration to its closing brace or semicolon: a public field in it?
                    let tail: String = lines[i..].join("\n");
                    let open = tail.find(['{', '(', ';']).unwrap_or(0);
                    match tail.as_bytes().get(open) {
                        Some(b';') | None => false,
                        Some(&b) => {
                            let (o, c) = if b == b'{' { ('{', '}') } else { ('(', ')') };
                            let mut depth = 0;
                            let mut body = String::new();
                            for ch in tail[open..].chars() {
                                if ch == o {
                                    depth += 1;
                                } else if ch == c {
                                    depth -= 1;
                                }
                                body.push(ch);
                                if depth == 0 {
                                    break;
                                }
                            }
                            body.contains("pub ")
                        }
                    }
                };
                if needs && !marked {
                    missing.push(format!("pub {kind} {name}"));
                }
            }
            attrs.clear();
        }
        missing
    }

    /// `cfg(feature` on a field or variant: inside the braces of a public struct or enum.
    fn feature_gated_members(text: &str) -> Vec<String> {
        let code = lex(text, true);
        let mut found = Vec::new();
        for (at, _) in code.match_indices("pub struct ").chain(code.match_indices("pub enum ")) {
            let Some(open) = code[at..].find(['{', ';']).map(|o| at + o) else { continue };
            if code.as_bytes()[open] == b';' {
                continue;
            }
            let mut depth = 0;
            let mut end = open;
            for (o, ch) in code[open..].char_indices() {
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = open + o;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if code[open..end].contains("cfg(feature") {
                found.push(code[at..open].trim().to_string());
            }
        }
        found
    }

    fn s1_sources() -> Vec<(PathBuf, String)> {
        files_under(&crate_dir(CONTRACT).join("src/s1"), &["rs"])
            .into_iter()
            .map(|p| {
                let text = std::fs::read_to_string(&p).unwrap_or_default();
                (p, text)
            })
            .collect()
    }

    fn assert_non_exhaustive(files: Vec<(PathBuf, String)>) {
        assert!(!files.is_empty(), "there is nothing to check here (yet)");
        let mut offenders = Vec::new();
        for (path, text) in files {
            let end = text.find("#[cfg(test)]").unwrap_or(text.len());
            for item in lacking_non_exhaustive(&text[..end]) {
                offenders.push(format!("{}: {item} lacks `#[non_exhaustive]`", rel(&path)));
            }
        }
        assert!(offenders.is_empty(), "\n{}", offenders.join("\n"));
    }

    fn assert_no_feature_gated_members(files: Vec<(PathBuf, String)>) {
        assert!(!files.is_empty(), "there is nothing to check here (yet)");
        let mut offenders = Vec::new();
        for (path, text) in files {
            let end = text.find("#[cfg(test)]").unwrap_or(text.len());
            for item in feature_gated_members(&text[..end]) {
                offenders.push(format!("{}: `{item}` has a feature-gated field or variant", rel(&path)));
            }
        }
        assert!(offenders.is_empty(), "\n{}", offenders.join("\n"));
    }

    #[test]
    fn c14_the_engine_interface_is_non_exhaustive() {
        assert_non_exhaustive(sources(ENGINE_API));
    }

    #[test]
    fn c14_the_extension_interface_and_the_host_api_are_non_exhaustive() {
        assert_non_exhaustive(sources(EXT));
        assert_non_exhaustive(sources(HOST_API));
    }

    #[test]
    fn c14_the_contracts_s1_types_are_non_exhaustive() {
        assert_non_exhaustive(s1_sources());
    }

    #[test]
    fn c15_the_engine_interface_has_no_feature_gated_members() {
        assert_no_feature_gated_members(sources(ENGINE_API));
    }

    #[test]
    fn c15_the_extension_interface_has_no_feature_gated_members() {
        assert_no_feature_gated_members(sources(EXT));
    }

    #[test]
    fn c15_the_contracts_s1_types_have_no_feature_gated_members() {
        assert_no_feature_gated_members(s1_sources());
    }

    #[test]
    fn c16_events_are_taken_by_the_hub_and_instances_are_not_linked_from_outside() {
        let mut offenders = Vec::new();
        for name in member_names() {
            for (path, text) in sources(&name) {
                let code = lex(&text, false);
                // A call; the trait and the links that implement it define one.
                if name != HUB && code.contains(".take_events(") {
                    offenders.push(format!("{} calls `take_events` outside the hub", rel(&path)));
                }
            }
        }
        assert!(sources(HUB).iter().any(|(_, t)| t.contains(".take_events(")), "the hub takes no events");
        let registry: Vec<_> = sources(INSTANCE).into_iter().filter(|(_, t)| t.contains("struct InstanceRegistry")).collect();
        assert!(!registry.is_empty(), "`InstanceRegistry` does not exist (yet)");
        for (path, text) in sources(INSTANCE) {
            if lex(&text, false).contains("pub fn link(") {
                offenders.push(format!("{} has a public `link`", rel(&path)));
            }
        }
        assert!(offenders.is_empty(), "\n{}", offenders.join("\n"));
    }

    #[test]
    #[ignore = "0.5.0：P13"]
    fn c17_no_shim_and_no_alias_table_is_left() {
        let members = member_names();
        assert!(!members.iter().any(|n| n == "harnix-module"), "the `harnix-module` shim is still a crate");
        assert!(
            !files_under(&workspace_root().join("crates"), &["rs"]).iter().any(|p| p.ends_with("src/aliases.rs")),
            "an `aliases.rs` is left"
        );
        assert!(members.iter().any(|n| n == ASSEMBLY), "`{ASSEMBLY}` does not exist (yet)");
    }

    #[test]
    #[ignore = "0.5.0：P23"]
    fn c18_nothing_in_this_file_is_ignored() {
        let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/layering.rs")).unwrap();
        // The attribute, spelled so that this test does not count itself.
        let attribute = format!("#[{}", "ignore");
        assert_eq!(text.matches(&attribute).count(), 0, "a rule is still waiting for its stage");
    }

    // ── the readers that the rules stand on ────────────────────────────────────────────────

    #[test]
    fn lex_drops_comments_and_keeps_or_blanks_strings() {
        let text = "let a = \"system\"; // starts_with(\"app.\")\n/* \"flow\" /* nested */ */ let b = r#\"plugins\"#; let c = '\"'; fn f<'a>() {}";
        let kept = lex(text, true);
        assert!(kept.contains("\"system\"") && kept.contains("\"plugins\""), "{kept}");
        assert!(!kept.contains("app.") && !kept.contains("flow") && !kept.contains("nested"), "{kept}");
        assert!(kept.contains("fn f<'a>()"), "{kept}");
        let blank = lex(text, false);
        assert!(!blank.contains("system") && !blank.contains("plugins") && blank.contains("\"\""), "{blank}");
    }

    #[test]
    fn strip_ts_comments_keeps_urls() {
        let text = "const u = \"https://x.dev\"; // __client\n/* __chain */ const k = 1;";
        let code = strip_ts_comments(text);
        assert!(code.contains("https://x.dev") && !code.contains("__client") && !code.contains("__chain"), "{code}");
    }

    #[test]
    fn non_exhaustive_is_asked_of_enums_and_structs_with_public_fields_only() {
        let text = "
#[derive(Debug)]
#[non_exhaustive]
pub struct Marked { pub a: u8 }

/// Docs between the attribute and the item do not matter.
#[non_exhaustive]
/// Like this.
pub enum MarkedEnum { A, B }

pub struct Unit;
pub struct Private { a: u8, b: String }
pub struct Tuple(u8);
pub(crate) struct Crate { pub a: u8 }

pub struct Open { pub a: u8 }
pub enum OpenEnum { A }
pub struct OpenTuple(pub u8);
";
        assert_eq!(lacking_non_exhaustive(text), vec!["pub struct Open", "pub enum OpenEnum", "pub struct OpenTuple"]);
    }

    #[test]
    fn a_feature_gate_on_a_member_is_found() {
        let text = "pub struct A { pub a: u8, #[cfg(feature = \"x\")] pub b: u8 }\npub enum B { C, D }\n#[cfg(feature = \"x\")]\npub struct Fine { pub a: u8 }";
        assert_eq!(feature_gated_members(text), vec!["pub struct A"]);
    }

    #[test]
    fn the_rules_name_only_crates_that_are_listed() {
        for name in [CONTRACT, KERNEL, AUTHZ, ROUTER, TRANSPORT, HUB, INSTANCE, ENGINE_API, ENGINE, SUPERVISE, ASSEMBLY, EXT, HOST_API,
            PACKAGES, CARRIER_WASM, CARRIER_PROC, PROC, PACKAGE, WASM_PORT, WASM_HOST, WASM, CLIENT, FACADE, CLI, TESTAPP] {
            assert!(ALL.contains(&name), "`{name}` is in a rule and not in the list of crates");
        }
        for name in FIRST_PARTY {
            assert!(ALL.contains(name), "`{name}` is not in the list of crates");
        }
    }
}
