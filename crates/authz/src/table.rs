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
}

impl std::fmt::Display for TableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TableError::Taken(name) => write!(f, "`{name}` is already in the authorization table"),
            TableError::HubOwned(name) => {
                write!(f, "`{name}` belongs to the hub and cannot be granted to a module")
            }
            TableError::Unknown(name) => write!(f, "`{name}` is not a method this process knows"),
        }
    }
}

impl MethodTable {
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
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
            self.entries.remove(name);
        }
    }

    /// Open one existing entry to one module, beyond what a device reaches.
    ///
    /// For what a module's manifest says it needs — the settings module
    /// writing a provider, which no device may do directly. Hub-owned
    /// entries cannot be granted at all.
    pub fn grant(&mut self, method: &str, module: &str) -> Result<(), TableError> {
        let Some(rule) = self.entries.get_mut(method) else {
            return Err(TableError::Unknown(method.to_string()));
        };
        if rule.hub_owned {
            return Err(TableError::HubOwned(method.to_string()));
        }
        rule.modules.insert(module.to_string());
        Ok(())
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
}
