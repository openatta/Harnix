//! Which subjects reach which methods, routes and topics.
//!
//! The table is data, in one place, so "what can a client call" is answered
//! by reading it rather than by tracing dispatch. Everything absent is
//! refused — a new method is unreachable until someone writes it down.
//!
//! Three kinds of entry share it, because they are one question asked about
//! three surfaces: a method by its name, a module route as `route:<id>`, a
//! host topic as `topic:<name>`. Only methods are listed by
//! [`MethodTable::reachable`]; the other two are answered by
//! [`MethodTable::decide`] like everything else.

use std::collections::{BTreeMap, BTreeSet};

use harnix_contract::Subject;

use crate::audit::Decision;

/// How far a method is open — the shapes the kernel's own entries come in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// This binary only. Never reachable from outside the process.
    Kernel,
    /// Any paired device, any module, and the kernel.
    Device,
    /// It exists and is refused, with the reason a client can show. A module
    /// may still be granted it by name — see [`MethodTable::grant`].
    Refused(&'static str),
    /// Refused, and **not grantable**: the hub owns it, and a module reaching
    /// it would be a way around the hub (the hub subscribes before anyone;
    /// turns are started by the hub). Kernel included.
    HubOwned(&'static str),
}

/// One kind of caller a module method or route is declared for.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Principal {
    /// A paired device: a person.
    Device,
    /// One module, by id.
    Module(String),
    /// Any module.
    AnyModule,
    /// This binary.
    Kernel,
}

/// Everything the table knows about one entry.
#[derive(Debug, Clone, Default)]
pub struct Rule {
    pub device: bool,
    pub kernel: bool,
    /// Module ids allowed; `*` is any module.
    pub modules: BTreeSet<String>,
    /// Why it is refused to whoever is not allowed, when that is on purpose.
    pub refused: Option<&'static str>,
    /// Refused to everyone, grants included.
    pub hub_owned: bool,
}

impl Rule {
    fn from_reach(reach: Reach) -> Self {
        match reach {
            Reach::Kernel => Rule {
                kernel: true,
                ..Rule::default()
            },
            Reach::Device => Rule {
                device: true,
                kernel: true,
                modules: BTreeSet::from(["*".to_string()]),
                ..Rule::default()
            },
            Reach::Refused(reason) => Rule {
                refused: Some(reason),
                ..Rule::default()
            },
            Reach::HubOwned(reason) => Rule {
                refused: Some(reason),
                hub_owned: true,
                ..Rule::default()
            },
        }
    }

    /// A rule built from a module's declaration.
    pub fn for_principals(principals: &[Principal]) -> Self {
        let mut rule = Rule::default();
        for p in principals {
            match p {
                Principal::Device => rule.device = true,
                Principal::Kernel => rule.kernel = true,
                Principal::AnyModule => {
                    rule.modules.insert("*".into());
                }
                Principal::Module(id) => {
                    rule.modules.insert(id.clone());
                }
            }
        }
        rule
    }

    fn allows_module(&self, id: &str) -> bool {
        self.modules.contains("*") || self.modules.contains(id)
    }
}

pub struct MethodTable {
    entries: BTreeMap<String, Rule>,
    /// The entries a runtime application declared after assembly, by name,
    /// with the application that owns each. Everything not in here was put in
    /// the table at assembly and is never touched by [`MethodTable::withdraw_app`].
    runtime: BTreeMap<String, String>,
}

/// Why a grant or a declaration did not take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableError {
    /// The name is already in the table. Two owners of one name is a
    /// question with no right answer.
    Taken(String),
    /// The hub owns it; nothing may be granted it.
    HubOwned(String),
    /// Nothing by that name exists to be granted.
    Unknown(String),
    /// A runtime application's name that is not in its own namespace
    /// (`app.<id>.<name>`, `topic:app.<id>.<name>`), or an id that cannot be one.
    Namespace(String),
    /// A runtime application whose id overlaps a name owner already in the
    /// table — the same id, or one nested in the other (`acme` and
    /// `acme.notes` would share `app.acme.notes.*`).
    Overlap { app: String, with: String },
}

impl std::fmt::Display for TableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TableError::Taken(name) => write!(f, "`{name}` is already in the authorization table"),
            TableError::HubOwned(name) => {
                write!(f, "`{name}` belongs to the hub and cannot be granted to a module")
            }
            TableError::Unknown(name) => write!(f, "`{name}` is not a method this process knows"),
            TableError::Namespace(name) => write!(
                f,
                "`{name}` is not a name a runtime application may declare: only `app.<its id>.<name>` and its topics"
            ),
            TableError::Overlap { app, with } => write!(
                f,
                "the application id `{app}` overlaps `{with}`, which already has names in the authorization table"
            ),
        }
    }
}

impl MethodTable {
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            runtime: BTreeMap::new(),
        }
    }

    pub fn allow(mut self, method: &str, reach: Reach) -> Self {
        self.entries.insert(method.to_string(), Rule::from_reach(reach));
        self
    }

    pub fn allow_all(mut self, methods: &[&str], reach: Reach) -> Self {
        for m in methods {
            self.entries.insert((*m).to_string(), Rule::from_reach(reach));
        }
        self
    }

    /// Add an entry a module declared. Refuses a name already present.
    pub fn declare(&mut self, name: &str, rule: Rule) -> Result<(), TableError> {
        if self.entries.contains_key(name) {
            return Err(TableError::Taken(name.to_string()));
        }
        self.entries.insert(name.to_string(), rule);
        Ok(())
    }

    /// Take a module's entries out again — it was disabled after declaring
    /// them (a failed migration, a failed start). A method that stays in the
    /// table with nothing behind it answers "unknown" from the wrong layer.
    pub fn retract(&mut self, names: &[String]) {
        for name in names {
            if !self.runtime.contains_key(name) {
                self.entries.remove(name);
            }
        }
    }

    /// Put one runtime application's names in the table, all or none.
    ///
    /// The only way anything enters the table after assembly. Conservative on
    /// purpose, because a row here is what a client may reach:
    ///
    /// - every name is in the application's own namespace — a method
    ///   `app.<app>.<name>` or a topic `topic:app.<app>.<name>`, nothing else
    ///   (no `harnix.*`, no routes, no engine method);
    /// - the rule is built here from the principals, so it can never be a
    ///   refusal, a hub-owned entry or anything a grant could widen later;
    /// - the id may not overlap any owner already in the table — a
    ///   compiled-in module (`app.<module>.*`) or another runtime application
    ///   — so one cannot add names inside another's namespace;
    /// - a name already present is refused (compiled-in names keep
    ///   precedence: they were there first).
    ///
    /// On any refusal nothing was written.
    pub fn declare_app(&mut self, app: &str, rows: &[(String, Vec<Principal>)]) -> Result<(), TableError> {
        self.check_app(app, rows)?;
        for (name, principals) in rows {
            self.entries.insert(name.clone(), Rule::for_principals(principals));
            self.runtime.insert(name.clone(), app.to_string());
        }
        Ok(())
    }

    /// What [`MethodTable::declare_app`] would refuse, without writing
    /// anything: for a caller that has more to set up before the names go in.
    pub fn check_app(&self, app: &str, rows: &[(String, Vec<Principal>)]) -> Result<(), TableError> {
        if !valid_app_id(app) {
            return Err(TableError::Namespace(app.to_string()));
        }
        if let Some(with) = self.owners().find(|owner| apps_overlap(app, owner)) {
            return Err(TableError::Overlap {
                app: app.to_string(),
                with: with.to_string(),
            });
        }
        let method_prefix = format!("app.{app}.");
        let topic_prefix = format!("{TOPIC}app.{app}.");
        let mut seen = BTreeSet::new();
        for (name, _) in rows {
            let short = name
                .strip_prefix(&method_prefix)
                .or_else(|| name.strip_prefix(&topic_prefix));
            if !short.is_some_and(valid_short_name) {
                return Err(TableError::Namespace(name.clone()));
            }
            if self.entries.contains_key(name) || !seen.insert(name.as_str()) {
                return Err(TableError::Taken(name.clone()));
            }
        }
        Ok(())
    }

    /// Take every name one runtime application declared back out, and say
    /// which. Names put in at assembly are never touched, whatever they are
    /// called. From the moment this returns, a call to one of them is decided
    /// as unknown — the same answer as a name nobody ever declared.
    pub fn withdraw_app(&mut self, app: &str) -> Vec<String> {
        let names: Vec<String> = self
            .runtime
            .iter()
            .filter(|(_, owner)| owner.as_str() == app)
            .map(|(name, _)| name.clone())
            .collect();
        for name in &names {
            self.runtime.remove(name);
            self.entries.remove(name);
        }
        names
    }

    /// The runtime application that declared this name, if one did.
    pub fn runtime_owner(&self, name: &str) -> Option<&str> {
        self.runtime.get(name).map(String::as_str)
    }

    /// Who owns names under `app.*` now: every compiled-in module with a
    /// method, topic or route in the table, and every runtime application.
    fn owners(&self) -> impl Iterator<Item = &str> + '_ {
        let assembled = self.entries.keys().filter(|n| !self.runtime.contains_key(*n)).filter_map(|name| {
            let rest = name
                .strip_prefix("app.")
                .or_else(|| name.strip_prefix(TOPIC).and_then(|t| t.strip_prefix("app.")));
            match rest {
                // A compiled-in module's id has no dots, so its namespace is the first segment.
                Some(rest) => rest.split('.').next(),
                None => name.strip_prefix(ROUTE),
            }
        });
        assembled.chain(self.runtime.values().map(String::as_str))
    }

    /// Open one existing entry to one module, beyond what a device reaches.
    ///
    /// For what a module's manifest says it needs — the settings module
    /// writing a provider, which no device may do directly. Hub-owned
    /// entries cannot be granted at all.
    pub fn grant(&mut self, method: &str, module: &str) -> Result<(), TableError> {
        // What a runtime application declared is its own, and only as wide as it said.
        if self.runtime.contains_key(method) {
            return Err(TableError::Unknown(method.to_string()));
        }
        let Some(rule) = self.entries.get_mut(method) else {
            return Err(TableError::Unknown(method.to_string()));
        };
        if rule.hub_owned {
            return Err(TableError::HubOwned(method.to_string()));
        }
        rule.modules.insert(module.to_string());
        Ok(())
    }

    /// Whether [`MethodTable::grant`] would take for `module`, without
    /// writing: `Ok(true)` when it would open the entry, `Ok(false)` when the
    /// entry already lets that module in (say, it is open to every module).
    pub fn check_grant(&self, method: &str, module: &str) -> Result<bool, TableError> {
        if self.runtime.contains_key(method) {
            return Err(TableError::Unknown(method.to_string()));
        }
        let Some(rule) = self.entries.get(method) else {
            return Err(TableError::Unknown(method.to_string()));
        };
        if rule.hub_owned {
            return Err(TableError::HubOwned(method.to_string()));
        }
        Ok(!rule.allows_module(module))
    }

    /// Take back what [`MethodTable::grant`] gave `module`: a runtime
    /// application's `hub` capability, when it is withdrawn. Touches nothing
    /// but that one id on that one entry, so an entry open to every module
    /// stays open.
    pub fn revoke(&mut self, method: &str, module: &str) {
        if let Some(rule) = self.entries.get_mut(method) {
            rule.modules.remove(module);
        }
    }

    pub fn decide(&self, subject: &Subject, method: &str) -> Decision {
        let Some(rule) = self.entries.get(method) else {
            return Decision::Unknown;
        };
        let allowed = match subject {
            Subject::Kernel => rule.kernel,
            Subject::Device { .. } => rule.device,
            Subject::Module { id } => !rule.hub_owned && rule.allows_module(id),
        };
        if allowed {
            return Decision::Allow;
        }
        Decision::Refuse(rule.refused.unwrap_or(match subject {
            Subject::Kernel => "this is not the kernel's to call",
            Subject::Device { .. } => {
                "this method is not reachable by a device; it was declared for the kernel or for modules"
            }
            Subject::Module { .. } => "this method was not declared for, or granted to, this module",
        }))
    }

    /// Methods this subject may call — not routes, not topics.
    pub fn reachable(&self, subject: &Subject) -> Vec<&str> {
        self.entries
            .keys()
            .filter(|m| !is_surface(m))
            .filter(|m| matches!(self.decide(subject, m), Decision::Allow))
            .map(String::as_str)
            .collect()
    }

    /// Topics this subject may subscribe to.
    pub fn reachable_topics(&self, subject: &Subject) -> Vec<&str> {
        self.entries
            .keys()
            .filter_map(|m| m.strip_prefix(TOPIC))
            .filter(|t| matches!(self.decide(subject, &topic(t)), Decision::Allow))
            .collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every entry and its rule — the source the generated method catalog
    /// and the diagnostics page read.
    pub fn rows(&self) -> impl Iterator<Item = (&str, &Rule)> + '_ {
        self.entries.iter().map(|(m, r)| (m.as_str(), r))
    }
}

impl Default for MethodTable {
    fn default() -> Self {
        Self::new()
    }
}

const ROUTE: &str = "route:";
const TOPIC: &str = "topic:";

/// The table's name for a module's routes, `/apps/<id>/`.
pub fn route(module: &str) -> String {
    format!("{ROUTE}{module}")
}

/// The table's name for subscribing to a host topic.
pub fn topic(name: &str) -> String {
    format!("{TOPIC}{name}")
}

fn is_surface(name: &str) -> bool {
    name.starts_with(ROUTE) || name.starts_with(TOPIC)
}

/// Whether an id can name a runtime application: one or more dot-separated
/// segments of lowercase letters, digits and `-`, each starting with a letter
/// (`fake`, `acme.notes`); at most 64 characters. It becomes a method prefix
/// (`app.<id>.`), a directory name and a space id.
pub fn valid_app_id(id: &str) -> bool {
    id.len() <= 64
        && id.split('.').all(|segment| {
            let mut chars = segment.chars();
            matches!(chars.next(), Some('a'..='z')) && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '-'))
        })
}

/// Whether two application ids would share names: equal, or one a
/// dot-prefix of the other (`acme` owns `app.acme.notes.x` as much as
/// `acme.notes` does).
pub fn apps_overlap(a: &str, b: &str) -> bool {
    let nested = |outer: &str, inner: &str| inner.len() > outer.len() && inner.starts_with(outer) && inner.as_bytes()[outer.len()] == b'.';
    a == b || nested(a, b) || nested(b, a)
}

/// A method or topic name inside an application's namespace: dot-separated
/// segments of ASCII letters, digits, `_` and `-`; at most 128 characters.
fn valid_short_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .split('.')
            .all(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-')))
}

/// The engine methods that change credentials, repoint model endpoints, or
/// hand the caller the process itself. Refused to devices always, with the
/// reason written out — the engine trusts whoever reaches its dispatch, and
/// in this process that is the hub (§3.4). A module may be granted one by
/// name in its manifest (the settings module writes providers); the grant is
/// on the diagnostics page.
///
/// `plugin.*` is deliberately **not** in this list. Installing a plugin runs
/// code, but one process serves one user and a device is that person: the
/// question this layer really answers is "may installed code do this on their
/// behalf", not "may this person install something". The enforcement that
/// matters is the engine's — its capability gate, its sandbox, and the
/// disclosure it returns from the install itself.
///
/// `mcp.addServer` stays refused, and the difference is the point: it
/// configures a subprocess-spawning tool with no manifest, no capability
/// declaration and no disclosure at all.
pub const ENGINE_REFUSALS: &[(&str, &str)] = &[
    (
        "config.setProvider",
        "changing provider credentials or base_url would point model traffic elsewhere",
    ),
    (
        "config.set",
        "changing provider credentials or base_url would point model traffic elsewhere",
    ),
    (
        "config.update",
        "changing provider credentials or base_url would point model traffic elsewhere",
    ),
    (
        "mcp.addServer",
        "adding an MCP server is equivalent to installing a tool that runs subprocesses",
    ),
    (
        "import.run",
        "config import runs outside the disclosure this layer is for",
    ),
    (
        "import.list",
        "config import runs outside the disclosure this layer is for",
    ),
];

/// What the hub owns. Refused to everyone, the kernel and every module
/// included, and never grantable: each is a way around the hub, and the hub
/// being the only holder of sessions is the invariant everything else rests
/// on.
pub const HUB_OWNED: &[(&str, &str)] = &[
    (
        "daemon.shutdown",
        "the process is stopped by whoever started it, not over its own protocol",
    ),
    (
        "session.run_turn",
        "use harnix.send — the hub owns turns, so closing a tab cannot lose one",
    ),
    (
        "session.subscribe",
        "use harnix.attach — the hub subscribes before any client does",
    ),
    (
        "session.unsubscribe",
        "use harnix.detach — the hub owns subscriptions",
    ),
    (
        "daemon.subscribeEvents",
        "host events are the hub's to fan out",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> Subject {
        Subject::Device { id: "d".into() }
    }

    fn module(id: &str) -> Subject {
        Subject::Module { id: id.into() }
    }

    #[test]
    fn a_device_method_reaches_devices_modules_and_the_kernel() {
        let t = MethodTable::new().allow("m", Reach::Device);
        assert_eq!(t.decide(&device(), "m"), Decision::Allow);
        assert_eq!(t.decide(&module("x"), "m"), Decision::Allow);
        assert_eq!(t.decide(&Subject::Kernel, "m"), Decision::Allow);
    }

    #[test]
    fn a_refusal_can_be_granted_to_one_module_and_no_other() {
        let mut t = MethodTable::new().allow("config.setProvider", Reach::Refused("no"));
        t.grant("config.setProvider", "settings").unwrap();
        assert_eq!(t.decide(&module("settings"), "config.setProvider"), Decision::Allow);
        assert_eq!(t.decide(&module("other"), "config.setProvider"), Decision::Refuse("no"));
        assert_eq!(t.decide(&device(), "config.setProvider"), Decision::Refuse("no"));
    }

    #[test]
    fn what_the_hub_owns_is_never_granted() {
        let mut t = MethodTable::new().allow("session.run_turn", Reach::HubOwned("hub"));
        assert_eq!(
            t.grant("session.run_turn", "x"),
            Err(TableError::HubOwned("session.run_turn".into()))
        );
        assert!(matches!(t.decide(&Subject::Kernel, "session.run_turn"), Decision::Refuse(_)));
    }

    #[test]
    fn a_module_declaration_admits_exactly_who_it_names() {
        let mut t = MethodTable::new();
        t.declare(
            "app.w.list",
            Rule::for_principals(&[Principal::Device, Principal::Module("trade".into())]),
        )
        .unwrap();
        t.declare("app.w.internal", Rule::for_principals(&[Principal::Kernel])).unwrap();
        assert_eq!(t.decide(&device(), "app.w.list"), Decision::Allow);
        assert_eq!(t.decide(&module("trade"), "app.w.list"), Decision::Allow);
        assert!(matches!(t.decide(&module("other"), "app.w.list"), Decision::Refuse(_)));
        assert!(matches!(t.decide(&device(), "app.w.internal"), Decision::Refuse(_)));
        assert_eq!(
            t.declare("app.w.list", Rule::default()),
            Err(TableError::Taken("app.w.list".into()))
        );
    }

    #[test]
    fn routes_and_topics_are_decided_but_not_listed_as_methods() {
        let mut t = MethodTable::new();
        t.declare(&route("w"), Rule::for_principals(&[Principal::Device])).unwrap();
        t.declare(&topic("app.w.groups"), Rule::for_principals(&[Principal::Device])).unwrap();
        assert!(t.reachable(&device()).is_empty());
        assert_eq!(t.reachable_topics(&device()), vec!["app.w.groups"]);
        assert_eq!(t.decide(&device(), &route("w")), Decision::Allow);
    }

    fn rows(names: &[&str]) -> Vec<(String, Vec<Principal>)> {
        names.iter().map(|n| (n.to_string(), vec![Principal::Device])).collect()
    }

    /// A compiled-in module, as assembly leaves it: one method, one topic, a route.
    fn assembled() -> MethodTable {
        let mut t = MethodTable::new().allow("harnix.hello", Reach::Device);
        t.declare("app.probe.echo", Rule::for_principals(&[Principal::Device])).unwrap();
        t.declare(&topic("app.probe.counter"), Rule::for_principals(&[Principal::Device])).unwrap();
        t.declare(&route("routed"), Rule::for_principals(&[Principal::Device])).unwrap();
        t
    }

    #[test]
    fn a_runtime_application_declares_and_withdraws_its_own_names() {
        let mut t = assembled();
        t.declare_app("fake", &rows(&["app.fake.echo", "topic:app.fake.ticks"])).unwrap();
        assert_eq!(t.decide(&device(), "app.fake.echo"), Decision::Allow);
        assert_eq!(t.reachable_topics(&device()), vec!["app.fake.ticks", "app.probe.counter"]);
        assert_eq!(t.runtime_owner("app.fake.echo"), Some("fake"));
        assert_eq!(t.runtime_owner("app.probe.echo"), None);

        let mut withdrawn = t.withdraw_app("fake");
        withdrawn.sort();
        assert_eq!(withdrawn, vec!["app.fake.echo", "topic:app.fake.ticks"]);
        // Refused afterwards exactly like a name nobody declared.
        assert_eq!(t.decide(&device(), "app.fake.echo"), Decision::Unknown);
        assert_eq!(t.decide(&device(), &topic("app.fake.ticks")), Decision::Unknown);
        // What assembly put in is untouched.
        assert_eq!(t.decide(&device(), "app.probe.echo"), Decision::Allow);
        assert!(t.withdraw_app("fake").is_empty());
    }

    #[test]
    fn a_runtime_application_stays_in_its_namespace() {
        let mut t = assembled();
        for name in [
            "harnix.hello",
            "harnix.apps.list",
            "session.create",
            "config.setProvider",
            "app.other.echo",
            "app.fake",
            "app.fake.",
            "app.fake..x",
            "app.fake.a b",
            "route:fake",
            "topic:harnix.instances",
            "topic:app.other.ticks",
        ] {
            assert!(
                matches!(t.declare_app("fake", &rows(&[name])), Err(TableError::Namespace(_))),
                "{name} was accepted"
            );
        }
        for id in ["", "Fake", "1fake", "fake.", ".fake", "fa_ke", "fake..x"] {
            assert!(matches!(t.declare_app(id, &rows(&[])), Err(TableError::Namespace(_))), "`{id}` accepted");
        }
        // Nothing was written by any of the refusals.
        assert!(t.decide(&device(), "harnix.hello") == Decision::Allow);
        assert!(t.runtime.is_empty());
    }

    #[test]
    fn compiled_in_names_keep_precedence() {
        let mut t = assembled();
        // The same id as a compiled-in module, or one nested in it.
        assert_eq!(
            t.declare_app("probe", &rows(&["app.probe.echo"])),
            Err(TableError::Overlap { app: "probe".into(), with: "probe".into() })
        );
        assert!(matches!(t.declare_app("probe", &rows(&["app.probe.new"])), Err(TableError::Overlap { .. })));
        assert!(matches!(t.declare_app("probe.sub", &rows(&["app.probe.sub.x"])), Err(TableError::Overlap { .. })));
        // A module known only by its route counts too.
        assert!(matches!(t.declare_app("routed", &rows(&["app.routed.x"])), Err(TableError::Overlap { .. })));
        // The compiled-in method is still the compiled-in one.
        assert_eq!(t.decide(&device(), "app.probe.echo"), Decision::Allow);
        assert_eq!(t.runtime_owner("app.probe.echo"), None);
        // And `retract`, which only assembly calls, never takes a runtime name.
        t.declare_app("fake", &rows(&["app.fake.echo"])).unwrap();
        t.retract(&["app.fake.echo".to_string()]);
        assert_eq!(t.decide(&device(), "app.fake.echo"), Decision::Allow);
    }

    #[test]
    fn runtime_applications_do_not_overlap_each_other() {
        let mut t = assembled();
        t.declare_app("acme.notes", &rows(&["app.acme.notes.list"])).unwrap();
        assert!(matches!(t.declare_app("acme.notes", &rows(&["app.acme.notes.other"])), Err(TableError::Overlap { .. })));
        assert!(matches!(t.declare_app("acme", &rows(&["app.acme.x"])), Err(TableError::Overlap { .. })));
        assert!(matches!(t.declare_app("acme.notes.pro", &rows(&["app.acme.notes.pro.x"])), Err(TableError::Overlap { .. })));
        // A sibling shares no names.
        t.declare_app("acme.todo", &rows(&["app.acme.todo.list"])).unwrap();
        // Once withdrawn, the id is free again.
        t.withdraw_app("acme.notes");
        t.declare_app("acme.notes", &rows(&["app.acme.notes.list"])).unwrap();
    }

    #[test]
    fn a_declaration_is_all_or_nothing() {
        let mut t = assembled();
        assert_eq!(
            t.declare_app("fake", &rows(&["app.fake.a", "app.fake.a"])),
            Err(TableError::Taken("app.fake.a".into()))
        );
        assert_eq!(
            t.declare_app("fake", &rows(&["app.fake.a", "harnix.hello"])),
            Err(TableError::Namespace("harnix.hello".into()))
        );
        assert_eq!(t.decide(&device(), "app.fake.a"), Decision::Unknown);
        assert!(t.runtime.is_empty());
    }

    #[test]
    fn a_runtime_rule_is_what_its_principals_say_and_cannot_be_widened() {
        let mut t = assembled();
        t.declare_app(
            "fake",
            &[
                ("app.fake.mine".into(), vec![Principal::Kernel]),
                ("app.fake.peer".into(), vec![Principal::Module("peer".into())]),
            ],
        )
        .unwrap();
        assert!(matches!(t.decide(&device(), "app.fake.mine"), Decision::Refuse(_)));
        assert_eq!(t.decide(&Subject::Kernel, "app.fake.mine"), Decision::Allow);
        assert_eq!(t.decide(&module("peer"), "app.fake.peer"), Decision::Allow);
        assert!(matches!(t.decide(&module("other"), "app.fake.peer"), Decision::Refuse(_)));
        assert_eq!(t.grant("app.fake.mine", "other"), Err(TableError::Unknown("app.fake.mine".into())));
    }

    #[test]
    fn a_grant_is_checked_like_a_modules_and_revoked_without_touching_the_rest() {
        let mut t = assembled()
            .allow("settings.write", Reach::Refused("not for devices"))
            .allow("harnix.turn", Reach::HubOwned("the hub's"));
        t.declare_app("fake", &rows(&["app.fake.echo"])).unwrap();
        // Open to every module already: nothing to grant, and revoking leaves it open.
        assert_eq!(t.check_grant("harnix.hello", "acme.notes"), Ok(false));
        assert_eq!(t.check_grant("settings.write", "acme.notes"), Ok(true));
        assert_eq!(t.check_grant("harnix.turn", "acme.notes"), Err(TableError::HubOwned("harnix.turn".into())));
        assert_eq!(t.check_grant("nothing.here", "acme.notes"), Err(TableError::Unknown("nothing.here".into())));
        assert_eq!(t.check_grant("app.fake.echo", "acme.notes"), Err(TableError::Unknown("app.fake.echo".into())));

        t.grant("settings.write", "acme.notes").unwrap();
        t.grant("settings.write", "settings").unwrap();
        assert_eq!(t.decide(&module("acme.notes"), "settings.write"), Decision::Allow);
        t.revoke("settings.write", "acme.notes");
        assert!(matches!(t.decide(&module("acme.notes"), "settings.write"), Decision::Refuse(_)));
        assert_eq!(t.decide(&module("settings"), "settings.write"), Decision::Allow, "another grant stays");
        t.revoke("harnix.hello", "acme.notes");
        assert_eq!(t.decide(&module("acme.notes"), "harnix.hello"), Decision::Allow);
    }

    #[test]
    fn overlap_is_by_whole_segments() {
        assert!(apps_overlap("acme", "acme"));
        assert!(apps_overlap("acme", "acme.notes"));
        assert!(apps_overlap("acme.notes", "acme"));
        assert!(!apps_overlap("acme", "acmex"));
        assert!(!apps_overlap("acme.notes", "acme.todo"));
    }
}
