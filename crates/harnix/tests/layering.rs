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

/// Direct dependencies of one Harnix crate, by name.
fn dependencies(crate_dir: &str) -> BTreeSet<String> {
    let manifest = workspace_root()
        .join("crates")
        .join(crate_dir)
        .join("Cargo.toml");
    let text = std::fs::read_to_string(&manifest)
        .unwrap_or_else(|e| panic!("{}: {e}", manifest.display()));
    let mut names = BTreeSet::new();
    let mut in_deps = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_deps = line.starts_with("[dependencies]") || line.starts_with("[dev-dependencies]");
            continue;
        }
        if !in_deps || line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some((name, _)) = line.split_once('=') {
            names.insert(name.trim().to_string());
        }
    }
    names
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
            "harnix-app",
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
/// it (§3.3.3, fifth invariant).
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

/// A module never gets the hub or an engine link (design §5.3, §14). The
/// contract it is written against cannot hand it one, because the crate that
/// defines the contract cannot name either.
#[test]
fn the_module_contract_cannot_reach_the_hub() {
    assert_ignorant_of(
        "module",
        &["harnix-hub", "harnix-transport", "harnix-app", "daemon"],
        "a module calls through the admission point like everyone else",
    );
    // It does depend on the instance layer, for templates and native
    // contributions — and that crate also holds the links. The contract may
    // name the first and never the second: a module holding a link could run
    // a turn the hub never subscribed to (design §5.3).
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
/// nothing else (design §3.1, roadmap M2-3). If they could not be built out of
/// it, the contract would not be worth believing.
#[test]
fn the_builtins_use_only_the_public_module_api() {
    assert_ignorant_of(
        "builtin",
        &[
            "harnix-hub",
            "harnix-authz",
            "harnix-instance",
            "harnix-app",
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
/// public module contract like a built-in: Harnix's own module API and vocabulary, Agix's scene
/// and tool interfaces for the scenes it wraps, and third-party crates. Never a layer below the
/// contract, and never the doors the application uses to drive a module.
#[test]
fn the_flow_module_uses_only_the_public_module_api() {
    for name in dependencies("flow") {
        if name.starts_with("harnix") {
            assert!(
                ["harnix-module", "harnix-contract"].contains(&name.as_str()),
                "`flow` depends on `{name}`; a module reaches the process only through its context"
            );
        }
        assert!(
            !["daemon", "runtime", "model", "history", "telemetry", "axum"]
                .contains(&name.as_str()),
            "`flow` depends on `{name}`; of Agix it may name `base` and `scene` and nothing else"
        );
    }
    let mut offenders = Vec::new();
    let mut stack = vec![workspace_root().join("crates/flow/src")];
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

/// The empty application registers the flow module (xops-02 §6), and an application that
/// depends on the `harnix` library must not be made to compile it: the `harnix` binary is its own
/// crate (`crates/cli`), and the library names no flow. (The engine's built-in scenes are another
/// matter: the instance layer builds on them anyway, and the library re-exports them as
/// `harnix::scenes`.)
#[test]
fn the_library_does_not_carry_the_empty_applications_flow_module() {
    assert_ignorant_of(
        "harnix",
        &["harnix-flow"],
        "a library user registers flows if it wants them; the empty application is `harnix-cli`",
    );
    let manifest =
        std::fs::read_to_string(workspace_root().join("crates/harnix/Cargo.toml")).unwrap();
    assert!(
        !manifest.contains("[[bin]]"),
        "`crates/harnix` builds no binary: the empty application's `harnix` is `crates/cli`"
    );
    let cli = dependencies("cli");
    assert!(
        cli.contains("harnix") && cli.contains("harnix-flow"),
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
/// This is §3.3.3's fifth invariant, in the only form that survives: "adding
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

/// One crate's `[dependencies]` (not its dev-dependencies), by name: whether each is optional.
fn normal_dependencies(crate_dir: &str) -> std::collections::BTreeMap<String, bool> {
    let manifest = workspace_root()
        .join("crates")
        .join(crate_dir)
        .join("Cargo.toml");
    let text = std::fs::read_to_string(&manifest)
        .unwrap_or_else(|e| panic!("{}: {e}", manifest.display()));
    let mut deps = std::collections::BTreeMap::new();
    let mut in_deps = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_deps = line.starts_with("[dependencies]");
            continue;
        }
        if !in_deps || line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some((name, rest)) = line.split_once('=') {
            deps.insert(name.trim().to_string(), rest.contains("optional = true"));
        }
    }
    deps
}

/// Agix's crates, by the names Harnix's crates know them by.
const AGIX: &[&str] = &["daemon", "base", "model", "history", "scene", "telemetry", "runtime", "plugin"];

/// Runtime applications are four layers, and each knows only those below it:
///
/// | crate | holds | may not reach |
/// |---|---|---|
/// | `harnix-app-package` | manifest, package, signatures, store, disclosure, the `harnix-app` tool | a VM, the kernel, Agix, an HTTP framework |
/// | `harnix-app-port` | `Platform` and `Control`, and their serde types | anything but the contract |
/// | `harnix-app-host` | lifecycle, registry, health, dispatch to the VM, idling | the kernel, the hub, Agix, an HTTP framework |
/// | `harnix-apps` | the embedded glue: the port over the kernel, the Agix plugin, `/apps/<id>/…` | — |
///
/// So the package and the host are the same code in a product's process and in a process of an
/// application's own, and the port between the host and whatever it runs in is all either side
/// sees of the other.
#[test]
fn the_application_layers_reach_only_down() {
    let forbidden_above = ["harnix-module", "harnix-hub", "harnix-builtin", "harnix-instance", "harnix-authz", "harnix-transport", "harnix-app", "harnix-apps", "axum", "wasmtime"];
    for (dir, allowed) in [
        ("app-package", &["harnix-contract", "harnix-wasm"][..]),
        ("app-port", &["harnix-contract"][..]),
        ("app-host", &["harnix-app-package", "harnix-app-port", "harnix-wasm"][..]),
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
            if dir == "app-package" && name == "harnix-wasm" {
                assert!(optional, "`app-package` links `harnix-wasm` only for the packing tool");
            }
        }
    }
    let port = normal_dependencies("app-port");
    for name in port.keys() {
        assert!(
            ["harnix-contract", "async-trait", "serde", "serde_json"].contains(&name.as_str()),
            "`app-port` depends on `{name}`: the boundary is serde types and async traits, nothing else"
        );
    }
    let package = std::fs::read_to_string(workspace_root().join("crates/app-package/Cargo.toml")).unwrap();
    assert!(
        package.contains("\"dep:harnix-wasm\"") && package.contains("required-features = [\"compile\"]"),
        "the packing tool, and only it, brings `harnix-wasm` into `app-package`"
    );
}

/// The application runtime is reached through one door. `harnix-wasm` (the engine) is named by
/// the application host alone (and by the packing tool, behind its own feature), the host by the
/// embedded glue alone, and the glue (`harnix-apps`) by the application layer alone, as an optional
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
                dir == "app-host" || (dir == "app-package" && *optional),
                "`{dir}` depends on `harnix-wasm`; only `harnix-app-host` may"
            );
        }
        if deps.contains_key("harnix-app-host") {
            assert_eq!(dir, "apps", "`{dir}` depends on `harnix-app-host`; only `harnix-apps` may");
        }
        if deps.contains_key("harnix-apps") {
            assert_eq!(dir, "app", "`{dir}` depends on `harnix-apps`; only the application layer may");
        }
        for layer in ["harnix-app-package", "harnix-app-port"] {
            if deps.contains_key(layer) {
                assert!(
                    ["app-host", "apps"].contains(&dir.as_str()),
                    "`{dir}` depends on `{layer}`; only the host and the glue may"
                );
            }
        }
    }
    let manifest = std::fs::read_to_string(root.join("app/Cargo.toml")).unwrap();
    let line = manifest
        .lines()
        .find(|l| l.starts_with("harnix-apps"))
        .expect("harnix-app names harnix-apps");
    assert!(line.contains("optional = true"), "`{line}`: harnix-apps must be optional");
    assert!(
        manifest.contains("apps = [\"dep:harnix-apps\"]"),
        "harnix-app's `apps` feature is what brings the runtime in"
    );
}

/// Harnix reads exactly one section of a package manifest.
///
/// This is §2.2 in the only form that survives a rewrite. "Harnix has no plugin
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
    let path = workspace_root().join("crates/builtin/src/packages.rs");
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

/// Every crate that depends on a Harnix crate depends on one that exists, and
/// nothing depends on itself. Cheap, and it catches a rename that only half
/// happened.
#[test]
fn every_harnix_dependency_resolves() {
    let crates = [
        "app",
        "instance",
        "authz",
        "builtin",
        "cli",
        "client",
        "contract",
        "flow",
        "harnix",
        "hub",
        "module",
        "testapp",
        "transport",
        "wasm",
        "app-package",
        "app-port",
        "app-host",
        "apps",
    ];
    let known: BTreeSet<String> = crates
        .iter()
        .map(|c| {
            if *c == "harnix" {
                "harnix".to_string()
            } else {
                format!("harnix-{c}")
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
