//! Privilege is capabilities, granted where the application adds an extension, and checked at start:
//! held to the assembly's registration step, with the kernel's own rows in the table.
//!
//! Nothing behind the kernel's rows is up here (no hub, no engine). A call the router lets through
//! answers `INTERNAL_ERROR` from the row, an extension's method `METHOD_NOT_FOUND` (it never
//! started); what these tests look for is whether it was `REFUSED`, and why.

use std::sync::Arc;

use harnix_assembly::testing::{register, Linked as Modules, Registered};
use harnix_contract::s1::{caps, Capability, CapabilitySpec, ExtId, PackageCarrier, Trust};
use harnix_contract::{codes, RpcError, Subject};
use harnix_kernel::{Access, CallContext, Dispatch, GrantBook, Handler, Rpc};
use harnix_ext::{Extension, Registrar};
use harnix_ext_devices::Devices;
use harnix_ext_harnix_app::HarnixApp;
use harnix_ext_observability::Observability;
use harnix_ext_plugins::Plugins;
use harnix_ext_settings::Settings;
use harnix_ext::SpaceDecl;
use harnix_ext::Manifest;
use harnix_router::{Row, TableError};
use serde_json::{json, Value};

/// What a debugger does to another space's session, method by method: each the engine's, each
/// needing `engine.inspect` or `engine.control`.
const DEBUGGER_ACTS: &[&str] = &[
    "engine.session.inspectContext",
    "engine.session.taskOutput",
    "engine.session.goal",
    "engine.session.stopTask",
    "engine.session.messageTask",
    "engine.session.dropMessages",
    "engine.session.editMessage",
    "engine.session.compact",
    "engine.session.clear",
    "engine.session.setModel",
    "engine.session.rename",
    "engine.session.setGoal",
    "engine.session.updateGoal",
];

/// A debugger's four capabilities: the instances, a session's inside, acting on it, the process.
fn debugger_caps() -> [Capability; 4] {
    [caps::INSTANCES_ADMIN, caps::ENGINE_INSPECT, caps::ENGINE_CONTROL, caps::PROCESS_OBSERVE]
}

fn id(s: &str) -> ExtId {
    ExtId::native(s).or_else(|_| ExtId::package(s)).unwrap()
}

/// A debugger an application writes itself: the powers to look into and act on any session, as
/// optional capabilities, and a space of its own that reaches across spaces.
struct Debugger;

#[async_trait::async_trait]
impl Extension for Debugger {
    type Config = ();

    fn manifest(&self) -> harnix_ext::Manifest {
        harnix_ext::Manifest::new("debugger", "0.0.1").optional(debugger_caps())
    }

    fn register(&self, r: &mut Registrar<'_, ()>) {
        r.space(SpaceDecl::new("debugger").cross_space());
    }
}

/// An extension that asks for something, and nothing else.
struct Asks(&'static str, Vec<Capability>, Vec<Capability>);

#[async_trait::async_trait]
impl Extension for Asks {
    type Config = ();

    fn manifest(&self) -> Manifest {
        Manifest::new(self.0, "0.0.1").requires(self.1.clone()).optional(self.2.clone())
    }

    fn register(&self, _r: &mut Registrar<'_, ()>) {}
}

/// A product's own capability: `billing` defines `billing.charge` and requires it on one method.
struct Billing;

const CHARGE: Capability = Capability::new("billing.charge");

#[async_trait::async_trait]
impl Extension for Billing {
    type Config = ();

    fn manifest(&self) -> Manifest {
        Manifest::new("billing", "0.0.1").defines(CapabilitySpec::new("billing.charge", "charge the customer's card"))
    }

    fn register(&self, r: &mut Registrar<'_, ()>) {
        r.method("charge").access(Access::capability(CHARGE)).raw(|_cx, p| async move { Ok::<_, RpcError>(p) });
    }
}

/// Harnix's own extensions, each granted what Harnix grants it, but the one called `without`.
fn first_party(without: &str) -> Modules {
    let mut modules = Modules::default();
    if without != "harnix-app" {
        modules = modules.extension(HarnixApp, [caps::TRANSCRIPTS_READ, caps::ENGINE_EXPORT, caps::ENGINE_CONTROL]);
    }
    if without != "observability" {
        modules = modules.extension(Observability, [caps::RECORDINGS_READ, caps::ENGINE_INSPECT, caps::PROCESS_OBSERVE]);
    }
    if without != "settings" {
        modules = modules.extension(Settings, [caps::ENGINE_CONFIG_READ, caps::ENGINE_CONFIG_WRITE, caps::ENGINE_MCP, caps::ENGINE_INSPECT]);
    }
    if without != "plugins" {
        modules = modules.extension(Plugins::default(), [caps::ENGINE_PLUGINS]);
    }
    if without != "devices" {
        modules = modules.extension(Devices, [caps::DEVICES_MANAGE]);
    }
    if without != "flow" {
        modules = modules.extension(harnix_ext_flow::Flow::builder().build(), [caps::ENGINE_CONTROL, caps::ENGINE_INSPECT]);
    }
    modules
}

/// What was registered, with the state directory it lives on for as long as the test does.
struct Fixture {
    registered: Registered,
    _state: tempfile::TempDir,
}

impl std::ops::Deref for Fixture {
    type Target = Registered;
    fn deref(&self) -> &Registered {
        &self.registered
    }
}

fn registered(modules: Modules) -> anyhow::Result<Fixture> {
    let state = tempfile::tempdir().unwrap();
    let registered = register(modules, state.path())?;
    Ok(Fixture { registered, _state: state })
}

async fn call(r: &Registered, subject: Subject, method: &str, params: Value) -> Rpc<Value> {
    r.router.call(CallContext::root(subject, None), method, params).await
}

/// Refused, by the router, naming the capability missing.
fn refused_for(e: &RpcError, cap: &Capability) -> bool {
    e.code == codes::REFUSED && e.data == Some(json!({"capability": cap.as_str()}))
}

mod capabilities {
    use super::*;

    #[test]
    fn granting_less_than_a_module_requires_refuses_the_start() {
        let modules = Modules::default().extension(Asks("fleet", vec![caps::ENGINE_INSPECT, caps::ENGINE_CONFIG_READ], vec![]), [caps::ENGINE_INSPECT]);
        let e = registered(modules).err().expect("started").to_string();
        assert!(e.contains("`fleet` requires `engine.config.read`"), "{e}");
    }

    #[test]
    fn granting_more_than_a_module_asks_for_refuses_the_start() {
        let modules = Modules::default().extension(Asks("fleet", vec![caps::ENGINE_INSPECT], vec![]), [caps::ENGINE_INSPECT, caps::ENGINE_EXPORT]);
        let e = registered(modules).err().expect("started").to_string();
        assert!(e.contains("`fleet` was granted `engine.export`, which it neither requires nor lists as optional"), "{e}");
    }

    #[test]
    fn what_only_a_package_may_hold_or_nobody_defined_is_not_granted() {
        let package_only = Modules::default().extension(Asks("fleet", vec![], vec![caps::EVENTS_ALL_TOOLS]), [caps::EVENTS_ALL_TOOLS]);
        let e = registered(package_only).err().expect("started").to_string();
        assert!(e.contains("only a package may hold"), "{e}");
        let nobody = Capability::new("nobody.defines");
        let undefined = Modules::default().extension(Asks("fleet", vec![nobody.clone()], vec![]), [nobody]);
        let e = registered(undefined).err().expect("started").to_string();
        assert!(e.contains("nothing here defines"), "{e}");
    }

    #[test]
    fn an_optional_capability_may_go_ungranted() {
        let r = registered(Modules::default().extension(Debugger, [])).unwrap();
        assert!(r.extensions[0].granted.is_empty());
        assert_eq!(r.extensions[0].optional.len(), 4);
    }

    #[tokio::test]
    async fn an_extensions_own_capability_is_defined_granted_and_required() {
        let r = registered(
            Modules::default()
                .extension(Billing, [])
                .extension(Asks("shop", vec![CHARGE], vec![]), [CHARGE])
                .extension(Asks("other", vec![], vec![]), []),
        )
        .unwrap();
        // Let through to the method (which answers nothing: it never started).
        let e = call(&r, Subject::extension(id("shop")), "billing.charge", json!({})).await.unwrap_err();
        assert_eq!(e.code, codes::METHOD_NOT_FOUND, "{e}");
        let e = call(&r, Subject::extension(id("other")), "billing.charge", json!({})).await.unwrap_err();
        assert!(refused_for(&e, &CHARGE), "{e:?}");
        let e = call(&r, Subject::device("d"), "billing.charge", json!({})).await.unwrap_err();
        assert_eq!(e.code, codes::REFUSED);
        assert_eq!(r.extensions[0].defines, ["billing.charge"]);
    }

    #[test]
    fn a_capability_is_defined_under_the_definers_id_only() {
        struct Squatter;
        #[async_trait::async_trait]
        impl Extension for Squatter {
            type Config = ();
            fn manifest(&self) -> Manifest {
                Manifest::new("squatter", "0.0.1").defines(CapabilitySpec::new("billing.charge", "not its own"))
            }
            fn register(&self, _r: &mut Registrar<'_, ()>) {}
        }
        let e = registered(Modules::default().extension(Squatter, [])).err().expect("started").to_string();
        assert!(e.contains("extension `squatter` defines"), "{e}");
    }

    #[test]
    fn the_built_ins_hold_what_they_require_and_nothing_else() {
        let r = registered(first_party("")).unwrap();
        let observability = r.extensions.iter().find(|m| m.id == "observability").unwrap();
        assert_eq!(observability.granted, ["recordings.read", "engine.inspect", "process.observe"]);
        // What each requires, and what it lists as optional and Harnix grants it too: the Harnix
        // app reads and changes a project's trust with `engine.control`; the settings page writes a
        // project's MCP servers and signs in to them with `engine.mcp`, and reads a session's MCP
        // context with `engine.inspect`.
        for status in &r.extensions {
            let held = [status.requires.clone(), status.optional.clone()].concat();
            assert_eq!(status.granted, held, "{}", status.id);
        }
        let app = r.extensions.iter().find(|m| m.id == "harnix-app").unwrap();
        assert_eq!(app.optional, ["engine.control"]);
        assert!(r.grants.holds(&id("settings"), &caps::ENGINE_CONFIG_WRITE));
        let settings = r.extensions.iter().find(|m| m.id == "settings").unwrap();
        assert_eq!(settings.optional, ["engine.mcp", "engine.inspect"]);
        assert!(!r.grants.holds(&id("settings"), &caps::ENGINE_CONTROL));
    }
}

mod debugger {
    use super::*;

    /// Granted the capabilities, a debugger the application wrote looks into and acts on another
    /// space's sessions: nothing about it is first-party.
    #[tokio::test]
    async fn granted_the_capabilities_a_debugger_acts_on_any_sessions() {
        let r = registered(Modules::default().extension(Debugger, debugger_caps())).unwrap();
        for method in DEBUGGER_ACTS {
            let e = call(&r, Subject::extension(id("debugger")), method, json!({"session_id": "s"})).await.unwrap_err();
            assert_eq!(e.code, codes::INTERNAL_ERROR, "{method} was not let through: {e}");
        }
        assert_eq!(r.extensions[0].spaces, ["debugger"]);
    }

    #[tokio::test]
    async fn ungranted_every_one_of_them_is_refused_and_says_what_is_missing() {
        let r = registered(Modules::default().extension(Debugger, [])).unwrap();
        for method in DEBUGGER_ACTS {
            let e = call(&r, Subject::extension(id("debugger")), method, json!({"session_id": "s"})).await.unwrap_err();
            let cap = r.router.table().row(method).and_then(|row| row.access().requirement().cloned()).unwrap();
            assert!(refused_for(&e, &cap), "{method}: {e:?}");
        }
        // Nor does a device reach any of them, whatever it claims.
        for method in DEBUGGER_ACTS {
            let e = call(&r, Subject::device("d"), method, json!({"space": "debugger"})).await.unwrap_err();
            assert_eq!(e.code, codes::REFUSED, "{method}");
        }
    }
}

mod unlinked {
    use super::*;

    /// The table's rows, as text, one each.
    fn table(modules: Modules) -> Vec<Value> {
        let r = registered(modules).unwrap();
        r.router.snapshot()["rows"].as_array().cloned().unwrap_or_default()
    }

    /// Not linking an extension changes the kernel by nothing: what is missing is its own rows.
    #[test]
    fn without_an_extension_only_its_rows_differ() {
        for ext in ["harnix-app", "observability", "settings", "plugins", "devices", "flow"] {
            let all_rows = table(first_party(""));
            let rows = table(first_party(ext));
            let owner = format!("ext:{ext}");
            let missing: Vec<&Value> = all_rows.iter().filter(|row| !rows.contains(row)).collect();
            assert!(!missing.is_empty(), "{ext} has no rows");
            for row in &missing {
                assert!(row["owner"] == owner, "without {ext}, {row} is missing");
            }
            assert!(rows.iter().all(|row| all_rows.contains(row)), "without {ext}, a row appeared");
        }
    }
}

mod scope {
    use super::*;

    struct Answer;

    #[async_trait::async_trait]
    impl Handler for Answer {
        async fn call(&self, _: CallContext, p: Value) -> Rpc<Value> {
            Ok(p)
        }
    }

    /// No extension, and a package `acme.notes` registered as the packages' table registers one:
    /// its consent, then its rows.
    fn with_package(frame: Option<&[&str]>) -> Fixture {
        let r = registered(Modules::default()).unwrap();
        let consent = ["engine.session.get", "harnix.sessions.list", "harnix.session.create"].map(String::from);
        r.grants.set(id("acme.notes"), [], Trust::consent(consent, PackageCarrier::Wasm));
        // As a package's own are installed: open to it as well.
        let own = Row::method("acme.notes.list", Access::device().with_extension(id("acme.notes")), Arc::new(Answer));
        r.router.install_package(&Subject::Kernel, "acme.notes", vec![own]).unwrap();
        if let Some(calls) = frame {
            r.grants.set_frame(&id("acme.notes"), Some(calls.iter().map(|c| c.to_string()).collect()));
        }
        r
    }

    fn refused_outside(e: &RpcError) -> bool {
        e.code == codes::REFUSED && e.message == harnix_authz::OUTSIDE_OWN_SPACE
    }

    #[tokio::test]
    async fn a_package_reaches_its_own_space_and_its_sessions_only() {
        let r = with_package(None);
        let me = || Subject::extension(id("acme.notes"));
        // Its own space, or one under it: let through (to a hub that is not up).
        for space in ["acme.notes", "acme.notes.board"] {
            let e = call(&r, me(), "engine.session.get", json!({"session_id": "s", "space": space})).await.unwrap_err();
            assert_eq!(e.code, codes::INTERNAL_ERROR, "{space}: {e}");
        }
        // Another space's session, a space that only starts like its own, or no claim at all.
        for params in [json!({"session_id": "s", "space": "sessions"}), json!({"session_id": "s", "space": "acme.notesy"}), json!({"session_id": "s"})] {
            let e = call(&r, me(), "engine.session.get", params.clone()).await.unwrap_err();
            assert!(refused_outside(&e), "{params}: {e:?}");
        }
        // Listing and creating are about a space: not claiming one is refused.
        assert!(refused_outside(&call(&r, me(), "harnix.sessions.list", json!({})).await.unwrap_err()));
        assert!(refused_outside(&call(&r, me(), "harnix.session.create", json!({"space": "flow"})).await.unwrap_err()));
        // Reading another session's inside is no package's: no capability opens it to one.
        let e = call(&r, me(), "engine.session.inspectContext", json!({"space": "acme.notes"})).await.unwrap_err();
        assert_eq!(e.code, codes::REFUSED);
        // Its own names are its own, wherever.
        assert_eq!(call(&r, me(), "acme.notes.list", json!({"x": 1})).await.unwrap(), json!({"x": 1}));
    }

    /// What a frame may list of the hub's session rows and topics reaches its own space alone:
    /// each is scoped to a session or a space, so a page claiming another space, or none, is
    /// refused before the hub is asked. The session space's own rows it does not reach at all.
    #[tokio::test]
    async fn the_session_rows_and_topics_a_frame_may_list_are_its_own_spaces() {
        let listed = [
            "harnix-app.sessions.archive",
            "harnix-app.sessions.export",
            "harnix.session.rename",
            "harnix.topics.subscribe",
            "topic:harnix.session.tasks",
            "topic:harnix.sessions.status",
            "topic:harnix.instances.sessions",
        ];
        let r = registered(first_party("")).unwrap();
        // As a package's own are installed: open to it as well.
        let own = Row::method("acme.notes.list", Access::device().with_extension(id("acme.notes")), Arc::new(Answer));
        r.router.install_package(&Subject::Kernel, "acme.notes", vec![own]).unwrap();
        r.grants.set(id("acme.notes"), [], Trust::consent([], PackageCarrier::Wasm));
        r.grants.set_frame(&id("acme.notes"), Some(listed.iter().map(|c| c.to_string()).collect()));
        let page = || {
            let mut cx = CallContext::root(Subject::device("d"), None);
            cx.on_behalf_of = Some(id("acme.notes"));
            cx
        };
        let sub = |topic: &str, space: Option<&str>| {
            let mut params = json!({"instance": "main", "session_id": "s"});
            if let Some(space) = space {
                params["space"] = json!(space);
            }
            json!({"topic": topic, "params": params})
        };
        // The session space's own rows are a first-party power: listed or not, a frame has none of
        // them.
        for method in ["harnix-app.sessions.archive", "harnix-app.sessions.export"] {
            let e = r.router.call(page(), method, json!({"session_id": "s", "space": "acme.notes"})).await.unwrap_err();
            assert_eq!((e.code, e.message.as_str()), (codes::REFUSED, harnix_router::NOT_FOR_PACKAGES), "{method}");
        }
        for method in ["harnix.session.rename"] {
            for space in [None, Some("sessions")] {
                let mut params = json!({"session_id": "s"});
                if let Some(space) = space {
                    params["space"] = json!(space);
                }
                let e = r.router.call(page(), method, params).await.unwrap_err();
                assert!(refused_outside(&e), "{method} {space:?}: {e:?}");
            }
        }
        for topic in ["harnix.session.tasks", "harnix.sessions.status", "harnix.instances.sessions"] {
            for space in [None, Some("sessions")] {
                let e = r.router.call(page(), "harnix.topics.subscribe", sub(topic, space)).await.unwrap_err();
                assert!(refused_outside(&e), "{topic} {space:?}: {e:?}");
            }
            // Its own space's: through to the hub, which is not up here.
            let e = r.router.call(page(), "harnix.topics.subscribe", sub(topic, Some("acme.notes"))).await.unwrap_err();
            assert!(!refused_outside(&e), "{topic}: {e:?}");
        }
    }

    #[tokio::test]
    async fn a_page_in_a_frame_has_the_devices_power_and_its_interfaces_list_both() {
        let page = |on: &str| {
            let mut cx = CallContext::root(Subject::device("d"), None);
            cx.on_behalf_of = Some(id(on));
            cx
        };
        // Without a framed interface it calls nothing.
        let r = with_package(None);
        let e = r.router.call(page("acme.notes"), "harnix.sessions.list", json!({"space": "acme.notes"})).await.unwrap_err();
        assert_eq!((e.code, e.message.as_str()), (codes::REFUSED, harnix_router::NO_FRAME));

        let r = with_package(Some(&["harnix.sessions.list", "engine.session.get", "engine.session.inspectContext"]));
        // Listed, in its own space: let through. Its own names: let through.
        let e = r.router.call(page("acme.notes"), "harnix.sessions.list", json!({"space": "acme.notes"})).await.unwrap_err();
        assert_eq!(e.code, codes::INTERNAL_ERROR, "{e}");
        assert!(r.router.call(page("acme.notes"), "acme.notes.list", json!({})).await.is_ok());
        // Listed, another space: narrowed. Not listed, though a device may: refused.
        let e = r.router.call(page("acme.notes"), "engine.session.get", json!({"space": "sessions"})).await.unwrap_err();
        assert!(refused_outside(&e), "{e:?}");
        let e = r.router.call(page("acme.notes"), "harnix.spaces.list", json!({})).await.unwrap_err();
        assert_eq!((e.code, e.message.as_str()), (codes::REFUSED, harnix_router::OUTSIDE_FRAME));
        // Listed, but not the device's to do: the intersection.
        let e = r.router.call(page("acme.notes"), "engine.session.inspectContext", json!({"space": "acme.notes"})).await.unwrap_err();
        assert_eq!(e.code, codes::REFUSED);
        // And the record says whose page it was.
        let entry = &r.router.audit().recent(1)[0];
        assert_eq!(entry.via.as_deref(), Some("ext:acme.notes"));
    }
}

mod names {
    use super::*;

    fn package(r: &Registered, app: &str) -> Result<(), TableError> {
        let row = Row::method(format!("{app}.x"), Access::device(), Arc::new(scope_answer()));
        r.router.install_package(&Subject::Kernel, app, vec![row])
    }

    fn scope_answer() -> impl Handler {
        struct Nothing;
        #[async_trait::async_trait]
        impl Handler for Nothing {
            async fn call(&self, _: CallContext, _: Value) -> Rpc<Value> {
                Ok(Value::Null)
            }
        }
        Nothing
    }

    fn taken_by(result: Result<(), TableError>) -> Option<String> {
        match result {
            Err(TableError::NamespaceTaken { by, .. }) => Some(by),
            _ => None,
        }
    }

    #[test]
    fn a_native_extension_keeps_its_namespace_from_a_package() {
        let r = registered(first_party("").extension(Asks("acme", vec![], vec![]), [])).unwrap();
        // Under a native id, by whole segments: refused, saying whose it is.
        assert_eq!(taken_by(package(&r, "acme.notes")).as_deref(), Some("acme"));
        assert!(package(&r, "acme").is_err(), "one segment is a native id, not a package's");
        // A built-in's id is a native id like any other.
        assert_eq!(taken_by(package(&r, "settings.extra")).as_deref(), Some("settings"));
        // Only by whole segments.
        assert!(package(&r, "acmeish.notes").is_ok());
    }

    #[test]
    fn a_packages_first_segment_is_not_a_reserved_word() {
        let r = registered(Modules::default()).unwrap();
        for word in harnix_contract::s1::RESERVED {
            assert!(package(&r, &format!("{word}.notes")).is_err(), "{word}");
        }
        assert!(package(&r, "example.showcase").is_ok());
    }

    #[test]
    fn packages_do_not_overlap_each_other_and_a_name_is_the_longest_ids() {
        let r = registered(Modules::default()).unwrap();
        package(&r, "acme.notes").unwrap();
        assert_eq!(taken_by(package(&r, "acme.notes")).as_deref(), Some("acme.notes"));
        assert!(package(&r, "acme.notes.pro").is_err(), "a package's id has two segments");
        assert!(package(&r, "acme.todo").is_ok(), "a sibling shares no names");
        // Withdrawn, its id is free again.
        r.router.withdraw(&Subject::Kernel, &harnix_kernel::Owner::extension(id("acme.notes")), "a test");
        assert!(package(&r, "acme.notes").is_ok());
    }
}

mod privileged {
    use harnix_ext::{DescriptorBuilder, ExtensionDescriptor};

    use super::*;

    /// A package's descriptor whose consent lists `calls`.
    fn package(calls: &[&str]) -> ExtensionDescriptor {
        let trust = Trust::consent(calls.iter().map(|c| c.to_string()), PackageCarrier::Wasm);
        DescriptorBuilder::new(id("acme.notes"), Manifest::new("acme.notes", "1.0.0"), trust).build()
    }

    /// A first-party row a person reaches — a session's context, the settings' — is not one a
    /// package's consent may list: listing one refuses the package at registration, saying why,
    /// and what it may list of the hub's is still listed.
    #[tokio::test]
    async fn a_package_listing_a_first_party_power_is_refused_at_registration() {
        let r = registered(first_party("")).unwrap();
        for name in ["observability.context", "observability.tasks.output", "settings.setProvider", "plugins.install", "devices.revoke"] {
            assert!(r.router.table().row(name).is_some(), "{name} is a row");
            let e = r.check_package(&package(&[name]), &[]).unwrap_err();
            assert_eq!(e.code, codes::REFUSED, "{name}");
            assert!(e.message.contains(name) && e.message.contains(harnix_router::NOT_FOR_PACKAGES), "{name}: {}", e.message);
        }
        r.check_package(&package(&["harnix.session.create", "topic:harnix.sessions.status"]), &[]).unwrap();
        // A device reaches the same row; the package as itself does not, listed or not.
        let me = Subject::extension(id("acme.notes"));
        r.grants.set(id("acme.notes"), [], Trust::consent(["observability.context".to_string()], PackageCarrier::Wasm));
        let e = call(&r, me, "observability.context", json!({"space": "acme.notes"})).await.unwrap_err();
        assert_eq!(e.code, codes::REFUSED);
    }
    struct Nothing;

    #[async_trait::async_trait]
    impl harnix_ext::http::RouteHandler for Nothing {
        async fn handle(
            &self,
            _cx: harnix_ext::http::RouteContext,
            _req: harnix_ext::http::Request<harnix_ext::http::Body>,
        ) -> harnix_ext::http::Response<harnix_ext::http::Body> {
            harnix_ext::http::status(harnix_ext::http::StatusCode::NOT_FOUND, "nothing")
        }
    }

    /// A package's routes are a directory of its files and its carrier's interface files; a tool
    /// endpoint is not one of them any more: a spawned daemon reaches its tools over its socket.
    #[test]
    fn a_packages_carrier_serves_its_interface_and_nothing_else() {
        let r = registered(first_party("")).unwrap();
        let with = |prefix: &str| {
            let trust = Trust::consent(Vec::<String>::new(), PackageCarrier::Wasm);
            let mut d = DescriptorBuilder::new(id("acme.notes"), Manifest::new("acme.notes", "1.0.0"), trust);
            d.route(prefix, Access::device(), Nothing);
            d.build()
        };
        r.check_package(&with("ui"), &[]).unwrap();
        let e = r.check_package(&with("mcp"), &[]).unwrap_err();
        assert_eq!(e.code, codes::REFUSED);
        assert!(e.message.contains("(`ui`)"), "{}", e.message);
    }
}
