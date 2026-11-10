//! The route table: every name a subject may call, subscribe to or fetch, one row each.
//!
//! The table is data, in one place, so "what can a client call" is answered by reading it
//! rather than by tracing dispatch, and "what answers it" by the same row. Everything absent is
//! refused — a new method is unreachable until someone installs its row.
//!
//! Three kinds of row share it, because they are one question asked about three surfaces: a
//! method by its name, a module route as `route:<id>`, a host topic as `topic:<name>`. Only
//! methods are listed by [`RouteTable::reachable`]; the other two are answered by
//! [`RouteTable::decide`] like everything else.
//!
//! Rows go in and come out by owner, all of one owner's at once ([`RouteTable::install`],
//! [`RouteTable::withdraw`]).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use harnix_authz::{decide, Decision};
use harnix_contract::Subject;
use harnix_kernel::{Access, Handler, Owner};
use serde_json::Value;

use crate::names::{self, DECIDED_BY_TOPIC};

/// What a row names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RowKind {
    Method,
    Topic,
    Route,
}

impl RowKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RowKind::Method => "method",
            RowKind::Topic => "topic",
            RowKind::Route => "route",
        }
    }
}

/// One row: its name, how far it is open, and what answers it.
#[derive(Clone)]
pub struct Row {
    /// The table's name: the method's, `topic:<name>` or `route:<id>`.
    name: String,
    kind: RowKind,
    access: Access,
    /// What answers a method. A topic or a route has none: the hub answers subscribing, the
    /// transport serves the route.
    handler: Option<Arc<dyn Handler>>,
    schema: Option<Value>,
    description: String,
}

impl Row {
    pub fn method(name: impl Into<String>, access: Access, handler: Arc<dyn Handler>) -> Self {
        Self {
            name: name.into(),
            kind: RowKind::Method,
            access,
            handler: Some(handler),
            schema: None,
            description: String::new(),
        }
    }

    /// A method row that is refused to everyone it is not granted to and answered by nothing
    /// here: what the hub owns, which no grant opens.
    pub fn closed(name: impl Into<String>, access: Access) -> Self {
        Self {
            name: name.into(),
            kind: RowKind::Method,
            access,
            handler: None,
            schema: None,
            description: String::new(),
        }
    }

    /// Subscribing to the topic `name`.
    pub fn topic(name: &str, access: Access) -> Self {
        Self {
            name: names::topic(name),
            kind: RowKind::Topic,
            access,
            handler: None,
            schema: None,
            description: String::new(),
        }
    }

    /// The routes `module` serves.
    pub fn route(module: &str, access: Access) -> Self {
        Self {
            name: names::route(module),
            kind: RowKind::Route,
            access,
            handler: None,
            schema: None,
            description: String::new(),
        }
    }

    pub fn schema(mut self, schema: Option<Value>) -> Self {
        self.schema = schema;
        self
    }

    pub fn describe(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> RowKind {
        self.kind
    }

    pub fn access(&self) -> &Access {
        &self.access
    }

    pub fn handler(&self) -> Option<&Arc<dyn Handler>> {
        self.handler.as_ref()
    }
}

/// One installed row and who installed it.
#[derive(Clone)]
pub(crate) struct Entry {
    pub row: Row,
    pub owner: Owner,
}

pub struct RouteTable {
    rows: BTreeMap<String, Entry>,
    /// Old names that answer as the row of a new one.
    aliases: BTreeMap<String, String>,
    /// The runtime applications whose rows are installed, by id ([`RouteTable::install_app`]).
    runtime: BTreeSet<String>,
    /// Runtime principals: the applications whose subject (`module:<id>`) reaches only what is
    /// listed here — its `hub` capability — and its own names ([`RouteTable::reach_only`]). An
    /// application that was withdrawn stays one, listing nothing.
    reach: BTreeMap<String, BTreeSet<String>>,
}

/// Why a runtime application, acting as itself, is refused a name it did not list.
pub const OUTSIDE_HUB: &str = "not in the application's `hub` capability";

/// Why a grant, an installation or an alias did not take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableError {
    /// The name is already in the table. Two owners of one name is a question with no right
    /// answer.
    Taken(String),
    /// The hub owns it; nothing may be granted it.
    HubOwned(String),
    /// Nothing by that name exists to be granted, or aliased.
    Unknown(String),
    /// A runtime application's name that is not in its own namespace (`app.<id>.<name>`,
    /// `topic:app.<id>.<name>`), or an id that cannot be one.
    Namespace(String),
    /// A runtime application whose id overlaps a name owner already in the table — the same id,
    /// or one nested in the other (`acme` and `acme.notes` would share `app.acme.notes.*`).
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

impl RouteTable {
    pub fn new() -> Self {
        Self {
            rows: BTreeMap::new(),
            aliases: BTreeMap::new(),
            runtime: BTreeSet::new(),
            reach: BTreeMap::new(),
        }
    }

    /// One owner's rows, all or none. Refuses a name already in the table (or an alias), and one
    /// named twice; on a refusal nothing was written.
    pub fn install(&mut self, owner: &Owner, rows: Vec<Row>) -> Result<(), TableError> {
        let mut seen = BTreeSet::new();
        for row in &rows {
            if self.taken(&row.name) || !seen.insert(row.name.as_str()) {
                return Err(TableError::Taken(row.name.clone()));
            }
        }
        for row in rows {
            self.rows.insert(row.name.clone(), Entry { row, owner: owner.clone() });
        }
        Ok(())
    }

    fn taken(&self, name: &str) -> bool {
        self.rows.contains_key(name) || self.aliases.contains_key(name)
    }

    /// Install one runtime application's rows, all or none.
    ///
    /// The only way anything enters the table after assembly. Conservative on purpose, because
    /// a row here is what a client may reach:
    ///
    /// - every name is in the application's own namespace — a method `app.<app>.<name>` or a
    ///   topic `topic:app.<app>.<name>`, nothing else (no `harnix.*`, no routes, no engine
    ///   method);
    /// - the id may not overlap any owner already in the table — a compiled-in module or another
    ///   runtime application — so one cannot add names inside another's namespace;
    /// - a name already present is refused (compiled-in names keep precedence: they were there
    ///   first);
    /// - nothing it installs can be granted to anyone later: it is only as wide as it said.
    ///
    /// On any refusal nothing was written.
    pub fn install_app(&mut self, app: &str, rows: Vec<Row>) -> Result<(), TableError> {
        self.check_app(app, rows.iter().map(Row::name))?;
        let id = harnix_contract::ExtId::parse(app).map_err(|_| TableError::Namespace(app.to_string()))?;
        self.install(&Owner::extension(id), rows)?;
        self.runtime.insert(app.to_string());
        Ok(())
    }

    /// What [`RouteTable::install_app`] would refuse, without writing anything: for a caller
    /// that has more to set up before the rows go in.
    pub fn check_app<'a>(&self, app: &str, names: impl IntoIterator<Item = &'a str>) -> Result<(), TableError> {
        if !names::valid_app_id(app) {
            return Err(TableError::Namespace(app.to_string()));
        }
        if let Some(with) = self.owners().find(|owner| names::apps_overlap(app, owner)) {
            return Err(TableError::Overlap {
                app: app.to_string(),
                with: with.to_string(),
            });
        }
        let mut seen = BTreeSet::new();
        for name in names {
            if names::short_app_name(app, name).is_none() {
                return Err(TableError::Namespace(name.to_string()));
            }
            if self.taken(name) || !seen.insert(name) {
                return Err(TableError::Taken(name.to_string()));
            }
        }
        Ok(())
    }

    /// Take every row `owner` installed back out, and say which. Another owner's rows are never
    /// touched, whatever they are called. From the moment this returns, a call to one of them is
    /// decided as unknown — the same answer as a name nobody ever installed. A runtime
    /// application withdrawn this way reaches nothing as itself any more.
    pub fn withdraw(&mut self, owner: &Owner) -> Vec<String> {
        let names: Vec<String> = self
            .rows
            .iter()
            .filter(|(_, entry)| &entry.owner == owner)
            .map(|(name, _)| name.clone())
            .collect();
        for name in &names {
            self.rows.remove(name);
        }
        if let Some(app) = owner.extension_id() {
            if self.runtime.remove(app.as_str()) {
                if let Some(listed) = self.reach.get_mut(app.as_str()) {
                    listed.clear();
                }
            }
        }
        names
    }

    /// Make `app`'s subject a runtime principal: `module:<app>` reaches the names in `listed`
    /// (its `hub` capability: methods, and topics as `topic:<name>`) and its own (`app.<app>.*`,
    /// `topic:app.<app>.*`), and nothing else, whatever a row opens to every module. Replaces
    /// what it listed before.
    ///
    /// What is listed is still decided by its row, as for any module: a name a device may call
    /// is open to it, one refused to devices only if it was granted it ([`RouteTable::grant`]),
    /// another application's only if that application declared it for modules. So the table
    /// answers in one place what an application may do as itself.
    pub fn reach_only(&mut self, app: &str, listed: impl IntoIterator<Item = String>) {
        self.reach.insert(app.to_string(), listed.into_iter().collect());
    }

    /// Whether `module:<id>` may even ask for `name`: `false` only for a runtime principal and a
    /// name outside what it listed and its own.
    fn within_reach(&self, id: &str, name: &str) -> bool {
        let Some(listed) = self.reach.get(id) else {
            return true;
        };
        listed.contains(name) || names::in_app_namespace(id, name) || DECIDED_BY_TOPIC.contains(&name)
    }

    /// The runtime application that installed this row, if one did.
    pub fn runtime_owner(&self, name: &str) -> Option<&str> {
        self.rows
            .get(name)
            .and_then(|entry| entry.owner.extension_id())
            .map(|id| id.as_str())
            .filter(|id| self.runtime.contains(*id))
    }

    /// Who owns a row.
    pub fn owner(&self, name: &str) -> Option<&Owner> {
        self.rows.get(name).map(|entry| &entry.owner)
    }

    /// Every extension that owns a row now: no runtime application may take its id, or nest
    /// under it.
    fn owners(&self) -> impl Iterator<Item = &str> + '_ {
        let ids: BTreeSet<&str> = self.rows.values().filter_map(|entry| entry.owner.extension_id().map(|id| id.as_str())).collect();
        ids.into_iter()
    }

    /// Open one existing row to one module, beyond what a device reaches.
    ///
    /// For what a module's manifest says it needs — the settings module writing a provider,
    /// which no device may do directly. What the hub owns cannot be granted at all, and what a
    /// runtime application installed is its own, only as wide as it said.
    pub fn grant(&mut self, name: &str, module: &str) -> Result<(), TableError> {
        self.check_grant(name, module)?;
        if let Some(entry) = self.rows.get_mut(name) {
            entry.row.access.grant(module);
        }
        Ok(())
    }

    /// Whether [`RouteTable::grant`] would take for `module`, without writing: `Ok(true)` when it
    /// would open the row, `Ok(false)` when the row already lets that module in (say, it is open
    /// to every module).
    pub fn check_grant(&self, name: &str, module: &str) -> Result<bool, TableError> {
        if self.runtime_owner(name).is_some() {
            return Err(TableError::Unknown(name.to_string()));
        }
        let Some(entry) = self.rows.get(name) else {
            return Err(TableError::Unknown(name.to_string()));
        };
        if entry.row.access.is_hub_owned() {
            return Err(TableError::HubOwned(name.to_string()));
        }
        Ok(!entry.row.access.opens_extension(module))
    }

    /// Take back what [`RouteTable::grant`] gave `module`: a runtime application's `hub`
    /// capability, when it is withdrawn. Touches nothing but that one grant on that one row, so
    /// a row open to every module stays open.
    pub fn revoke(&mut self, name: &str, module: &str) {
        if let Some(entry) = self.rows.get_mut(name) {
            entry.row.access.revoke(module);
        }
    }

    /// Let `old` answer as the row `new`: decided, reached and answered as `new` is. For a name
    /// that is moving, while its callers move with it.
    pub fn alias(&mut self, old: &str, new: &str) -> Result<(), TableError> {
        if self.taken(old) {
            return Err(TableError::Taken(old.to_string()));
        }
        if !self.rows.contains_key(new) {
            return Err(TableError::Unknown(new.to_string()));
        }
        self.aliases.insert(old.to_string(), new.to_string());
        Ok(())
    }

    /// The name a call means: its own, or what it is an alias of.
    pub fn resolve<'a>(&'a self, name: &'a str) -> &'a str {
        self.aliases.get(name).map_or(name, String::as_str)
    }

    /// The row a name means, through an alias.
    pub fn row(&self, name: &str) -> Option<&Row> {
        self.rows.get(self.resolve(name)).map(|entry| &entry.row)
    }

    pub fn decide(&self, subject: &Subject, name: &str) -> Decision {
        let name = self.resolve(name);
        // Before whether it exists: an application asking for something it did not list is told
        // that, not whether the name is there.
        if let Subject::Extension { id } = subject {
            if !self.within_reach(id.as_str(), name) {
                return Decision::Refuse(OUTSIDE_HUB);
            }
        }
        match self.rows.get(name) {
            Some(entry) => decide(subject, &entry.row.access),
            None => Decision::Unknown,
        }
    }

    /// Methods this subject may call — not routes, not topics.
    pub fn reachable(&self, subject: &Subject) -> Vec<&str> {
        self.rows
            .keys()
            .filter(|m| !names::is_surface(m))
            .filter(|m| matches!(self.decide(subject, m), Decision::Allow))
            .map(String::as_str)
            .collect()
    }

    /// Topics this subject may subscribe to.
    pub fn reachable_topics(&self, subject: &Subject) -> Vec<&str> {
        self.rows
            .keys()
            .filter_map(|m| names::topic_of(m))
            .filter(|t| matches!(self.decide(subject, &names::topic(t)), Decision::Allow))
            .collect()
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub(crate) fn entries(&self) -> impl Iterator<Item = &Entry> + '_ {
        self.rows.values()
    }

    pub(crate) fn aliases(&self) -> impl Iterator<Item = (&str, &str)> + '_ {
        self.aliases.iter().map(|(old, new)| (old.as_str(), new.as_str()))
    }
}

impl Default for RouteTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use harnix_authz::{access_for, Principal};
    use harnix_kernel::{CallContext, KernelPart, Rpc};

    use super::*;
    use crate::names::{route, topic};

    struct Echo;

    #[async_trait::async_trait]
    impl Handler for Echo {
        async fn call(&self, _: CallContext, params: Value) -> Rpc<Value> {
            Ok(params)
        }
    }

    fn device() -> Subject {
        Subject::Device { id: "d".into() }
    }

    fn module(id: &str) -> Subject {
        Subject::extension(harnix_contract::ExtId::parse(id).unwrap())
    }

    fn kernel() -> Owner {
        Owner::Kernel(KernelPart::Hub)
    }

    fn method(name: &str, access: Access) -> Row {
        Row::method(name, access, Arc::new(Echo))
    }

    fn with(rows: Vec<Row>) -> RouteTable {
        let mut t = RouteTable::new();
        t.install(&kernel(), rows).unwrap();
        t
    }

    #[test]
    fn a_device_method_reaches_devices_modules_and_the_kernel() {
        let t = with(vec![method("m", Access::device())]);
        assert_eq!(t.decide(&device(), "m"), Decision::Allow);
        assert_eq!(t.decide(&module("x"), "m"), Decision::Allow);
        assert_eq!(t.decide(&Subject::Kernel, "m"), Decision::Allow);
    }

    #[test]
    fn a_refusal_can_be_granted_to_one_module_and_no_other() {
        let mut t = with(vec![method("config.setProvider", Access::refused("no"))]);
        t.grant("config.setProvider", "settings").unwrap();
        assert_eq!(t.decide(&module("settings"), "config.setProvider"), Decision::Allow);
        assert_eq!(t.decide(&module("other"), "config.setProvider"), Decision::Refuse("no"));
        assert_eq!(t.decide(&device(), "config.setProvider"), Decision::Refuse("no"));
    }

    #[test]
    fn what_the_hub_owns_is_never_granted() {
        let mut t = with(vec![Row::closed("session.run_turn", Access::hub_owned("hub"))]);
        assert_eq!(
            t.grant("session.run_turn", "x"),
            Err(TableError::HubOwned("session.run_turn".into()))
        );
        assert!(matches!(t.decide(&Subject::Kernel, "session.run_turn"), Decision::Refuse(_)));
    }

    #[test]
    fn a_module_declaration_admits_exactly_who_it_names() {
        let mut t = RouteTable::new();
        t.install(
            &Owner::extension(harnix_contract::ExtId::parse("w").unwrap()),
            vec![
                method("app.w.list", access_for(&[Principal::Device, Principal::Module("trade".into())])),
                method("app.w.internal", access_for(&[Principal::Kernel])),
            ],
        )
        .unwrap();
        assert_eq!(t.decide(&device(), "app.w.list"), Decision::Allow);
        assert_eq!(t.decide(&module("trade"), "app.w.list"), Decision::Allow);
        assert!(matches!(t.decide(&module("other"), "app.w.list"), Decision::Refuse(_)));
        assert!(matches!(t.decide(&device(), "app.w.internal"), Decision::Refuse(_)));
        assert_eq!(
            t.install(&Owner::extension(harnix_contract::ExtId::parse("w").unwrap()), vec![method("app.w.list", Access::nobody())]),
            Err(TableError::Taken("app.w.list".into()))
        );
    }

    #[test]
    fn routes_and_topics_are_decided_but_not_listed_as_methods() {
        let mut t = RouteTable::new();
        t.install(
            &Owner::extension(harnix_contract::ExtId::parse("w").unwrap()),
            vec![
                Row::route("w", access_for(&[Principal::Device])),
                Row::topic("app.w.groups", access_for(&[Principal::Device])),
            ],
        )
        .unwrap();
        assert!(t.reachable(&device()).is_empty());
        assert_eq!(t.reachable_topics(&device()), vec!["app.w.groups"]);
        assert_eq!(t.decide(&device(), &route("w")), Decision::Allow);
    }

    fn rows(names: &[&str]) -> Vec<Row> {
        names
            .iter()
            .map(|n| match n.strip_prefix("topic:") {
                Some(t) => Row::topic(t, access_for(&[Principal::Device])),
                None => method(n, access_for(&[Principal::Device])),
            })
            .collect()
    }

    /// A compiled-in module, as assembly leaves it: one method, one topic, a route.
    fn assembled() -> RouteTable {
        let mut t = with(vec![method("harnix.hello", Access::device())]);
        t.install(
            &Owner::extension(harnix_contract::ExtId::parse("probe").unwrap()),
            vec![
                method("app.probe.echo", access_for(&[Principal::Device])),
                Row::topic("app.probe.counter", access_for(&[Principal::Device])),
            ],
        )
        .unwrap();
        t.install(&Owner::extension(harnix_contract::ExtId::parse("routed").unwrap()), vec![Row::route("routed", access_for(&[Principal::Device]))])
            .unwrap();
        t
    }

    #[test]
    fn a_runtime_application_declares_and_withdraws_its_own_names() {
        let mut t = assembled();
        t.install_app("fake", rows(&["app.fake.echo", "topic:app.fake.ticks"])).unwrap();
        assert_eq!(t.decide(&device(), "app.fake.echo"), Decision::Allow);
        assert_eq!(t.reachable_topics(&device()), vec!["app.fake.ticks", "app.probe.counter"]);
        assert_eq!(t.runtime_owner("app.fake.echo"), Some("fake"));
        assert_eq!(t.runtime_owner("app.probe.echo"), None);

        let mut withdrawn = t.withdraw(&Owner::extension(harnix_contract::ExtId::parse("fake").unwrap()));
        withdrawn.sort();
        assert_eq!(withdrawn, vec!["app.fake.echo", "topic:app.fake.ticks"]);
        // Refused afterwards exactly like a name nobody declared.
        assert_eq!(t.decide(&device(), "app.fake.echo"), Decision::Unknown);
        assert_eq!(t.decide(&device(), &topic("app.fake.ticks")), Decision::Unknown);
        // What assembly put in is untouched.
        assert_eq!(t.decide(&device(), "app.probe.echo"), Decision::Allow);
        assert!(t.withdraw(&Owner::extension(harnix_contract::ExtId::parse("fake").unwrap())).is_empty());
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
                matches!(t.install_app("fake", rows(&[name])), Err(TableError::Namespace(_))),
                "{name} was accepted"
            );
        }
        assert!(
            matches!(t.install_app("fake", vec![Row::route("fake", Access::device())]), Err(TableError::Namespace(_))),
            "a route was accepted"
        );
        for id in ["", "Fake", "1fake", "fake.", ".fake", "fa_ke", "fake..x"] {
            assert!(matches!(t.install_app(id, rows(&[])), Err(TableError::Namespace(_))), "`{id}` accepted");
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
            t.install_app("probe", rows(&["app.probe.echo"])),
            Err(TableError::Overlap { app: "probe".into(), with: "probe".into() })
        );
        assert!(matches!(t.install_app("probe", rows(&["app.probe.new"])), Err(TableError::Overlap { .. })));
        assert!(matches!(t.install_app("probe.sub", rows(&["app.probe.sub.x"])), Err(TableError::Overlap { .. })));
        // A module known only by its route counts too.
        assert!(matches!(t.install_app("routed", rows(&["app.routed.x"])), Err(TableError::Overlap { .. })));
        // The compiled-in method is still the compiled-in one.
        assert_eq!(t.decide(&device(), "app.probe.echo"), Decision::Allow);
        assert_eq!(t.runtime_owner("app.probe.echo"), None);
        // And withdrawing a compiled-in module, which only assembly does, never takes an
        // application's names.
        t.install_app("fake", rows(&["app.fake.echo"])).unwrap();
        t.withdraw(&Owner::extension(harnix_contract::ExtId::parse("probe").unwrap()));
        assert_eq!(t.decide(&device(), "app.fake.echo"), Decision::Allow);
        assert_eq!(t.decide(&device(), "app.probe.echo"), Decision::Unknown);
    }

    #[test]
    fn runtime_applications_do_not_overlap_each_other() {
        let mut t = assembled();
        t.install_app("acme.notes", rows(&["app.acme.notes.list"])).unwrap();
        assert!(matches!(t.install_app("acme.notes", rows(&["app.acme.notes.other"])), Err(TableError::Overlap { .. })));
        assert!(matches!(t.install_app("acme", rows(&["app.acme.x"])), Err(TableError::Overlap { .. })));
        assert!(matches!(
            t.install_app("acme.notes.pro", rows(&["app.acme.notes.pro.x"])),
            Err(TableError::Overlap { .. })
        ));
        // A sibling shares no names.
        t.install_app("acme.todo", rows(&["app.acme.todo.list"])).unwrap();
        // Once withdrawn, the id is free again.
        t.withdraw(&Owner::extension(harnix_contract::ExtId::parse("acme.notes").unwrap()));
        t.install_app("acme.notes", rows(&["app.acme.notes.list"])).unwrap();
    }

    #[test]
    fn a_declaration_is_all_or_nothing() {
        let mut t = assembled();
        assert_eq!(
            t.install_app("fake", rows(&["app.fake.a", "app.fake.a"])),
            Err(TableError::Taken("app.fake.a".into()))
        );
        assert_eq!(
            t.install_app("fake", rows(&["app.fake.a", "harnix.hello"])),
            Err(TableError::Namespace("harnix.hello".into()))
        );
        assert_eq!(t.decide(&device(), "app.fake.a"), Decision::Unknown);
        assert!(t.runtime.is_empty());
        // Nor does one owner's batch take half its names.
        assert_eq!(
            t.install(&kernel(), rows(&["harnix.new", "harnix.hello"])),
            Err(TableError::Taken("harnix.hello".into()))
        );
        assert_eq!(t.decide(&device(), "harnix.new"), Decision::Unknown);
    }

    #[test]
    fn a_runtime_rule_is_what_its_principals_say_and_cannot_be_widened() {
        let mut t = assembled();
        t.install_app(
            "fake",
            vec![
                method("app.fake.mine", access_for(&[Principal::Kernel])),
                method("app.fake.peer", access_for(&[Principal::Module("peer".into())])),
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
        let mut t = assembled();
        t.install(
            &kernel(),
            vec![
                method("settings.write", Access::refused("not for devices")),
                Row::closed("harnix.turn", Access::hub_owned("the hub's")),
            ],
        )
        .unwrap();
        t.install_app("fake", rows(&["app.fake.echo"])).unwrap();
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
    fn a_runtime_principal_reaches_what_it_listed_and_its_own_and_nothing_more() {
        let mut t = assembled();
        t.install(
            &kernel(),
            vec![
                method("settings.write", Access::refused("not for devices")),
                method("session.create", Access::device()),
                method("harnix.subscribe", Access::device()),
                Row::topic("harnix.sessions.status", Access::device()),
                Row::topic("harnix.process", Access::device()),
            ],
        )
        .unwrap();
        let own = access_for(&[Principal::Device, Principal::Module("acme.notes".into())]);
        t.install_app(
            "acme.notes",
            vec![method("app.acme.notes.echo", own.clone()), Row::topic("app.acme.notes.ticks", own)],
        )
        .unwrap();
        t.install_app("acme.todo", vec![method("app.acme.todo.list", access_for(&[Principal::AnyModule]))]).unwrap();
        t.reach_only(
            "acme.notes",
            ["session.create", "settings.write", "topic:harnix.sessions.status", "app.acme.todo.list"].map(String::from),
        );
        let me = module("acme.notes");
        // Open to every module, and listed.
        assert_eq!(t.decide(&me, "session.create"), Decision::Allow);
        // Open to every module, not listed: what the table alone used to let it reach.
        assert_eq!(t.decide(&me, "harnix.hello"), Decision::Refuse(OUTSIDE_HUB));
        assert_eq!(t.decide(&me, "nothing.here"), Decision::Refuse(OUTSIDE_HUB), "told whether it exists");
        // Listed, refused to devices: only once it is granted.
        assert!(matches!(t.decide(&me, "settings.write"), Decision::Refuse(r) if r != OUTSIDE_HUB));
        t.grant("settings.write", "acme.notes").unwrap();
        assert_eq!(t.decide(&me, "settings.write"), Decision::Allow);
        // Its own names, unlisted; one it did not declare is unknown like any other.
        assert_eq!(t.decide(&me, "app.acme.notes.echo"), Decision::Allow);
        assert_eq!(t.decide(&me, "app.acme.notes.nope"), Decision::Unknown);
        assert_eq!(t.decide(&me, &topic("app.acme.notes.ticks")), Decision::Allow);
        // Another application's, listed, as that one declared it.
        assert_eq!(t.decide(&me, "app.acme.todo.list"), Decision::Allow);
        // Topics: subscribing is decided by the topic.
        assert_eq!(t.decide(&me, "harnix.subscribe"), Decision::Allow);
        assert_eq!(t.decide(&me, &topic("harnix.sessions.status")), Decision::Allow);
        assert_eq!(t.decide(&me, &topic("harnix.process")), Decision::Refuse(OUTSIDE_HUB));
        // So what it is told it reaches is what it reaches.
        assert_eq!(
            t.reachable(&me),
            ["app.acme.notes.echo", "app.acme.todo.list", "harnix.subscribe", "session.create", "settings.write"]
        );
        assert_eq!(t.reachable_topics(&me), ["app.acme.notes.ticks", "harnix.sessions.status"]);
        // A compiled-in module is not narrowed by any of this.
        assert_eq!(t.decide(&module("probe"), "harnix.hello"), Decision::Allow);

        // Withdrawn, it stays a runtime principal and reaches nothing at all.
        t.withdraw(&Owner::extension(harnix_contract::ExtId::parse("acme.notes").unwrap()));
        assert_eq!(t.decide(&me, "session.create"), Decision::Refuse(OUTSIDE_HUB));
        assert!(t.reachable(&me).iter().all(|m| DECIDED_BY_TOPIC.contains(m)), "{:?}", t.reachable(&me));
    }

    #[test]
    fn an_alias_answers_as_its_row_and_is_not_a_row_of_its_own() {
        let mut t = assembled();
        t.install(&kernel(), vec![method("settings.write", Access::refused("not for devices"))]).unwrap();
        t.alias("harnix.hi", "harnix.hello").unwrap();
        t.alias("old.write", "settings.write").unwrap();
        assert_eq!(t.decide(&device(), "harnix.hi"), Decision::Allow);
        assert_eq!(t.decide(&device(), "old.write"), Decision::Refuse("not for devices"));
        assert_eq!(t.row("harnix.hi").map(Row::name), Some("harnix.hello"));
        assert!(!t.reachable(&device()).contains(&"harnix.hi"), "listed by its new name only");
        // Neither name can be taken twice, and an alias needs something to point at.
        assert_eq!(t.alias("harnix.hello", "app.probe.echo"), Err(TableError::Taken("harnix.hello".into())));
        assert_eq!(t.alias("harnix.hi", "app.probe.echo"), Err(TableError::Taken("harnix.hi".into())));
        assert_eq!(t.alias("x", "nothing.here"), Err(TableError::Unknown("nothing.here".into())));
        assert_eq!(t.install(&kernel(), rows(&["harnix.hi"])), Err(TableError::Taken("harnix.hi".into())));
    }

    #[test]
    fn rows_are_installed_and_withdrawn_by_owner() {
        let mut t = assembled();
        assert_eq!(t.owner("app.probe.echo"), Some(&Owner::extension(harnix_contract::ExtId::parse("probe").unwrap())));
        assert_eq!(t.owner("harnix.hello"), Some(&kernel()));
        let mut gone = t.withdraw(&Owner::extension(harnix_contract::ExtId::parse("probe").unwrap()));
        gone.sort();
        assert_eq!(gone, ["app.probe.echo", "topic:app.probe.counter"]);
        assert_eq!(t.owner("route:routed"), Some(&Owner::extension(harnix_contract::ExtId::parse("routed").unwrap())));
        assert_eq!(t.decide(&device(), "harnix.hello"), Decision::Allow);
    }
}
