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
//! [`RouteTable::withdraw`]). What an extension holds is not here: a call is decided by its row
//! and by a [`GrantBook`], read at the time.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use harnix_authz::{decide, Decision};
use harnix_contract::s1::{caps, Capability, CapabilitySpec, ExtId};
use harnix_contract::Subject;
use harnix_kernel::{Access, GrantBook, Handler, Owner};
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

/// Whether an extension id is a native extension's or a package's: they may not overlap, and
/// the native one keeps its names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdKind {
    Native,
    Package,
}

pub struct RouteTable {
    rows: BTreeMap<String, Entry>,
    /// Old names that answer as the row of a new one.
    aliases: BTreeMap<String, String>,
    /// Every extension id registered, rows or none: a native one whether it is enabled or not, a
    /// package's until it is withdrawn. Who owns a name is decided among these.
    extensions: BTreeMap<ExtId, IdKind>,
    /// Every capability a row may require: the kernel's, and what the extensions defined.
    defined: BTreeMap<String, CapabilitySpec>,
}

/// Why a package, acting as itself, is refused a name its consent does not list.
pub const OUTSIDE_HUB: &str = "not in the application's `hub` capability";

/// Why an extension nothing registered is refused everything: one withdrawn, or never there.
pub const UNREGISTERED: &str = "not a registered extension";

/// Why a grant, an installation or an alias did not take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableError {
    /// The name is already in the table. Two owners of one name is a question with no right
    /// answer.
    Taken(String),
    /// The hub owns it: no extension may declare it, nor list it.
    HubOwned(String),
    /// Nothing by that name exists to be listed, or aliased.
    Unknown(String),
    /// A runtime application's name that is not in its own namespace (`app.<id>.<name>`,
    /// `topic:app.<id>.<name>`), or an id that cannot be one.
    Namespace(String),
    /// The id overlaps one registered already — the same id, one nested in the other, or a
    /// package whose first segment is a native extension's or a reserved word. `by` is who has it.
    NamespaceTaken { id: String, by: String },
    /// The name belongs to another extension: the longest registered id it starts with by whole
    /// segments is `owner`'s.
    NotOwner { name: String, owner: String },
    /// A row requires a capability nobody defined, or an extension defines one outside its id.
    Capability(String),
}

impl std::fmt::Display for TableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TableError::Taken(name) => write!(f, "`{name}` is already in the authorization table"),
            TableError::HubOwned(name) => {
                write!(f, "`{name}` belongs to the hub and no extension may declare or reach it")
            }
            TableError::Unknown(name) => write!(f, "`{name}` is not a method this process knows"),
            TableError::Namespace(name) => write!(
                f,
                "`{name}` is not a name a runtime application may declare: only `app.<its id>.<name>` and its topics"
            ),
            TableError::NamespaceTaken { id, by } => {
                write!(f, "the id `{id}` overlaps `{by}`, which is registered already (namespace_taken)")
            }
            TableError::NotOwner { name, owner } => write!(f, "`{name}` is in `{owner}`'s namespace"),
            TableError::Capability(name) => write!(f, "`{name}` is not a capability defined here"),
        }
    }
}

impl RouteTable {
    pub fn new() -> Self {
        Self {
            rows: BTreeMap::new(),
            aliases: BTreeMap::new(),
            extensions: BTreeMap::new(),
            defined: caps::KERNEL.iter().map(|spec| (spec.name().to_string(), spec.clone())).collect(),
        }
    }

    /// Register an extension id, before its rows. Refuses one that overlaps an id registered
    /// already ([`names::overlap`]), and a package's whose first segment is reserved or a native
    /// extension's: natives register at assembly, so the native one keeps its names.
    pub fn register(&mut self, id: &ExtId, kind: IdKind) -> Result<(), TableError> {
        let taken = |by: &str| TableError::NamespaceTaken { id: id.to_string(), by: by.to_string() };
        if kind == IdKind::Package && names::RESERVED.contains(&id.first_segment()) {
            return Err(taken(id.first_segment()));
        }
        for (other, other_kind) in &self.extensions {
            let first = kind == IdKind::Package && *other_kind == IdKind::Native && other.as_str() == id.first_segment();
            if first || names::overlap(id.as_str(), other.as_str()) {
                return Err(taken(other.as_str()));
            }
        }
        self.extensions.insert(id.clone(), kind);
        Ok(())
    }

    /// What `id` is registered as, if it is.
    pub fn registered(&self, id: &ExtId) -> Option<IdKind> {
        self.extensions.get(id).copied()
    }

    /// A capability `by` defines: named `<its id>.<name>`.
    pub fn define(&mut self, by: &ExtId, spec: CapabilitySpec) -> Result<(), TableError> {
        let name = spec.name();
        let inside = name.strip_prefix(by.as_str()).is_some_and(|rest| rest.len() > 1 && rest.starts_with('.'));
        if !inside || self.defined.contains_key(name) {
            return Err(TableError::Capability(name.to_string()));
        }
        self.defined.insert(name.to_string(), spec);
        Ok(())
    }

    /// How `cap` was defined, when it was.
    pub fn defined(&self, cap: &Capability) -> Option<&CapabilitySpec> {
        self.defined.get(cap.as_str())
    }

    /// One owner's rows, all or none. Refuses a name already in the table (or an alias), and one
    /// named twice; an extension's row in another's namespace, one the hub would own, and one
    /// requiring a capability nobody defined. On a refusal nothing was written. An extension not
    /// registered yet is registered as a native one.
    pub fn install(&mut self, owner: &Owner, rows: Vec<Row>) -> Result<(), TableError> {
        let mut seen = BTreeSet::new();
        for row in &rows {
            if self.taken(&row.name) || !seen.insert(row.name.as_str()) {
                return Err(TableError::Taken(row.name.clone()));
            }
            if let Some(cap) = row.access.requirement() {
                if !self.defined.contains_key(cap.as_str()) {
                    return Err(TableError::Capability(cap.to_string()));
                }
            }
        }
        if let Owner::Extension(id) = owner {
            if self.registered(id).is_none() {
                self.register(id, IdKind::Native)?;
            }
            for row in &rows {
                self.check_owner(id, row)?;
            }
        }
        for row in rows {
            self.rows.insert(row.name.clone(), Entry { row, owner: owner.clone() });
        }
        Ok(())
    }

    /// Whether `id` may own `row`: the hub's are nobody's to declare, a route is under its own id,
    /// and a name in an extension namespace is the longest registered id's it starts with. A
    /// native extension's names outside one (`harnix.*`) are the assembly's to vouch for until
    /// every name is under its owner's id.
    fn check_owner(&self, id: &ExtId, row: &Row) -> Result<(), TableError> {
        if row.access.is_hub_owned() {
            return Err(TableError::HubOwned(row.name.clone()));
        }
        if let Some(routed) = names::route_of(&row.name) {
            return match routed == id.as_str() {
                true => Ok(()),
                false => Err(TableError::NotOwner { name: row.name.clone(), owner: routed.to_string() }),
            };
        }
        match names::extension_part(&row.name) {
            Some(part) => match names::owner_of(part, self.extensions.keys()) {
                Some(owner) if owner == id => Ok(()),
                other => Err(TableError::NotOwner {
                    name: row.name.clone(),
                    owner: other.map_or_else(|| "nobody".to_string(), ToString::to_string),
                }),
            },
            None if self.registered(id) == Some(IdKind::Package) => Err(TableError::Namespace(row.name.clone())),
            None => Ok(()),
        }
    }

    fn taken(&self, name: &str) -> bool {
        self.rows.contains_key(name) || self.aliases.contains_key(name)
    }

    /// Install one runtime application's rows, all or none, registering its id as a package's.
    ///
    /// The only way anything enters the table after assembly. Conservative on purpose, because
    /// a row here is what a client may reach:
    ///
    /// - every name is in the application's own namespace — a method `app.<app>.<name>` or a
    ///   topic `topic:app.<app>.<name>`, nothing else (no `harnix.*`, no routes, no engine
    ///   method);
    /// - the id may not overlap any id registered — a compiled-in module, enabled or not, or
    ///   another runtime application — so one cannot add names inside another's namespace;
    /// - a name already present is refused (compiled-in names keep precedence: they were there
    ///   first).
    ///
    /// On any refusal nothing was written.
    pub fn install_app(&mut self, app: &str, rows: Vec<Row>) -> Result<(), TableError> {
        self.check_app(app, rows.iter().map(Row::name))?;
        let id = ExtId::parse(app).map_err(|_| TableError::Namespace(app.to_string()))?;
        self.register(&id, IdKind::Package)?;
        if let Err(e) = self.install(&Owner::extension(id.clone()), rows) {
            self.extensions.remove(&id);
            return Err(e);
        }
        Ok(())
    }

    /// What [`RouteTable::install_app`] would refuse, without writing anything: for a caller
    /// that has more to set up before the rows go in.
    pub fn check_app<'a>(&self, app: &str, names: impl IntoIterator<Item = &'a str>) -> Result<(), TableError> {
        let Ok(id) = ExtId::parse(app) else {
            return Err(TableError::Namespace(app.to_string()));
        };
        let mut scratch = RouteTable {
            rows: BTreeMap::new(),
            aliases: BTreeMap::new(),
            extensions: self.extensions.clone(),
            defined: BTreeMap::new(),
        };
        scratch.register(&id, IdKind::Package)?;
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
    /// application's id is free again.
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
        if let Some(id) = owner.extension_id() {
            if self.registered(id) == Some(IdKind::Package) {
                self.extensions.remove(id);
            }
        }
        names
    }

    /// Whether a package may list `name` in its consent: a method or a topic (`topic:<name>`)
    /// that exists, and not the hub's own.
    pub fn check_listed(&self, name: &str) -> Result<(), TableError> {
        let Some(entry) = self.rows.get(self.resolve(name)) else {
            return Err(TableError::Unknown(name.to_string()));
        };
        if entry.row.access.is_hub_owned() {
            return Err(TableError::HubOwned(name.to_string()));
        }
        Ok(())
    }

    /// Whether `ext` owns the row `name` stands for.
    pub fn owns(&self, ext: &ExtId, name: &str) -> bool {
        self.rows
            .get(self.resolve(name))
            .and_then(|entry| entry.owner.extension_id())
            .is_some_and(|owner| owner == ext)
    }

    /// Who owns a row.
    pub fn owner(&self, name: &str) -> Option<&Owner> {
        self.rows.get(name).map(|entry| &entry.owner)
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

    /// Whether `subject` may call `name`, by its row and what `grants` say it holds. A package is
    /// held to what its consent lists and its own names first: asking for anything else it is
    /// told that, not whether the name is there.
    pub fn decide(&self, subject: &Subject, grants: &dyn GrantBook, name: &str) -> Decision {
        let name = self.resolve(name);
        if let Subject::Extension { id } = subject {
            match grants.trust(id) {
                None => return Decision::Refuse(UNREGISTERED),
                Some(trust) => {
                    if let Some(calls) = trust.calls() {
                        let own = self.owns(id, name) || names::in_app_namespace(id.as_str(), name);
                        let listed = own || calls.contains(name) || DECIDED_BY_TOPIC.contains(&name);
                        if !listed {
                            return Decision::Refuse(OUTSIDE_HUB);
                        }
                    }
                }
            }
        }
        match self.rows.get(name) {
            Some(entry) => decide(subject, grants, &entry.row.access),
            None => Decision::Unknown,
        }
    }

    /// Methods this subject may call — not routes, not topics.
    pub fn reachable(&self, subject: &Subject, grants: &dyn GrantBook) -> Vec<&str> {
        self.rows
            .keys()
            .filter(|m| !names::is_surface(m))
            .filter(|m| matches!(self.decide(subject, grants, m), Decision::Allow))
            .map(String::as_str)
            .collect()
    }

    /// Topics this subject may subscribe to.
    pub fn reachable_topics(&self, subject: &Subject, grants: &dyn GrantBook) -> Vec<&str> {
        self.rows
            .keys()
            .filter_map(|m| names::topic_of(m))
            .filter(|t| matches!(self.decide(subject, grants, &names::topic(t)), Decision::Allow))
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
    use harnix_contract::s1::{PackageCarrier, Trust};
    use harnix_kernel::{CallContext, GrantTable, KernelPart, Rpc};

    use super::*;
    use crate::names::{route, topic};

    struct Echo;

    #[async_trait::async_trait]
    impl Handler for Echo {
        async fn call(&self, _: CallContext, params: Value) -> Rpc<Value> {
            Ok(params)
        }
    }

    fn id(s: &str) -> ExtId {
        ExtId::parse(s).unwrap()
    }

    fn device() -> Subject {
        Subject::device("d")
    }

    fn module(s: &str) -> Subject {
        Subject::extension(id(s))
    }

    fn kernel() -> Owner {
        Owner::Kernel(KernelPart::Hub)
    }

    fn ext(s: &str) -> Owner {
        Owner::extension(id(s))
    }

    fn method(name: &str, access: Access) -> Row {
        Row::method(name, access, Arc::new(Echo))
    }

    fn with(rows: Vec<Row>) -> RouteTable {
        let mut t = RouteTable::new();
        t.install(&kernel(), rows).unwrap();
        t
    }

    /// Every native module of these tests, holding nothing, and the packages with their consent.
    fn grants(natives: &[&str], packages: &[(&str, &[&str])]) -> GrantTable {
        let g = GrantTable::new();
        for n in natives {
            g.set(id(n), [], Trust::Origin);
        }
        for (p, calls) in packages {
            g.set(id(p), [], Trust::consent(calls.iter().map(|c| c.to_string()), PackageCarrier::Wasm));
        }
        g
    }

    fn natives() -> GrantTable {
        grants(&["x", "trade", "other", "probe", "settings", "peer"], &[])
    }

    #[test]
    fn a_device_method_reaches_devices_modules_and_the_kernel() {
        let t = with(vec![method("m", Access::device())]);
        let g = natives();
        assert_eq!(t.decide(&device(), &g, "m"), Decision::Allow);
        assert_eq!(t.decide(&module("x"), &g, "m"), Decision::Allow);
        assert_eq!(t.decide(&Subject::Kernel, &g, "m"), Decision::Allow);
        // An extension nothing registered reaches nothing.
        assert_eq!(t.decide(&module("stranger"), &g, "m"), Decision::Refuse(UNREGISTERED));
    }

    #[test]
    fn a_capability_row_opens_to_whoever_holds_it_and_no_other() {
        let t = with(vec![method("config.setProvider", Access::capability(caps::ENGINE_CONFIG_WRITE).refusing("no"))]);
        let g = natives();
        g.set(id("settings"), [caps::ENGINE_CONFIG_WRITE], Trust::Origin);
        assert_eq!(t.decide(&module("settings"), &g, "config.setProvider"), Decision::Allow);
        assert_eq!(t.decide(&module("other"), &g, "config.setProvider"), Decision::Refuse("no"));
        assert_eq!(t.decide(&device(), &g, "config.setProvider"), Decision::Refuse("no"));
    }

    #[test]
    fn what_the_hub_owns_reaches_nobody_and_no_extension_declares_it() {
        let mut t = with(vec![Row::closed("session.run_turn", Access::hub_owned("hub"))]);
        let g = natives();
        assert!(matches!(t.decide(&Subject::Kernel, &g, "session.run_turn"), Decision::Refuse(_)));
        assert_eq!(t.check_listed("session.run_turn"), Err(TableError::HubOwned("session.run_turn".into())));
        assert_eq!(
            t.install(&ext("x"), vec![Row::closed("app.x.turn", Access::hub_owned("mine"))]),
            Err(TableError::HubOwned("app.x.turn".into()))
        );
    }

    #[test]
    fn a_row_requires_only_what_was_defined() {
        let mut t = RouteTable::new();
        let billing = id("billing");
        t.register(&billing, IdKind::Native).unwrap();
        let charge = Capability::new("billing.charge");
        assert_eq!(
            t.install(&ext("billing"), vec![method("app.billing.charge", Access::capability(charge.clone()))]),
            Err(TableError::Capability("billing.charge".into()))
        );
        t.define(&billing, CapabilitySpec::new("billing.charge", "charge a card")).unwrap();
        t.install(&ext("billing"), vec![method("app.billing.charge", Access::capability(charge.clone()))]).unwrap();
        // Only under its own id, and once.
        assert!(t.define(&billing, CapabilitySpec::new("billing.charge", "again")).is_err());
        assert!(t.define(&billing, CapabilitySpec::new("engine.billing", "not its")).is_err());
        assert!(t.define(&billing, CapabilitySpec::new("billing", "no name")).is_err());

        let g = natives();
        g.set(id("shop"), [charge], Trust::Origin);
        assert_eq!(t.decide(&module("shop"), &g, "app.billing.charge"), Decision::Allow);
        assert!(matches!(t.decide(&module("other"), &g, "app.billing.charge"), Decision::Refuse(_)));
    }

    #[test]
    fn a_module_declaration_admits_exactly_who_it_names() {
        let mut t = RouteTable::new();
        t.install(
            &ext("w"),
            vec![
                method("app.w.list", access_for(&[Principal::Device, Principal::Module("trade".into())])),
                method("app.w.internal", access_for(&[Principal::Kernel])),
            ],
        )
        .unwrap();
        let g = natives();
        assert_eq!(t.decide(&device(), &g, "app.w.list"), Decision::Allow);
        assert_eq!(t.decide(&module("trade"), &g, "app.w.list"), Decision::Allow);
        assert!(matches!(t.decide(&module("other"), &g, "app.w.list"), Decision::Refuse(_)));
        assert!(matches!(t.decide(&device(), &g, "app.w.internal"), Decision::Refuse(_)));
        assert_eq!(
            t.install(&ext("w"), vec![method("app.w.list", Access::kernel())]),
            Err(TableError::Taken("app.w.list".into()))
        );
    }

    #[test]
    fn routes_and_topics_are_decided_but_not_listed_as_methods() {
        let mut t = RouteTable::new();
        t.install(
            &ext("w"),
            vec![
                Row::route("w", access_for(&[Principal::Device])),
                Row::topic("app.w.groups", access_for(&[Principal::Device])),
            ],
        )
        .unwrap();
        let g = natives();
        assert!(t.reachable(&device(), &g).is_empty());
        assert_eq!(t.reachable_topics(&device(), &g), vec!["app.w.groups"]);
        assert_eq!(t.decide(&device(), &g, &route("w")), Decision::Allow);
        // A route under another's id is not its to serve.
        assert!(matches!(t.install(&ext("v"), vec![Row::route("w2", Access::device())]), Err(TableError::NotOwner { .. })));
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
            &ext("probe"),
            vec![
                method("app.probe.echo", access_for(&[Principal::Device])),
                Row::topic("app.probe.counter", access_for(&[Principal::Device])),
            ],
        )
        .unwrap();
        t.install(&ext("routed"), vec![Row::route("routed", access_for(&[Principal::Device]))]).unwrap();
        t
    }

    #[test]
    fn a_runtime_application_declares_and_withdraws_its_own_names() {
        let mut t = assembled();
        let g = natives();
        t.install_app("fake", rows(&["app.fake.echo", "topic:app.fake.ticks"])).unwrap();
        assert_eq!(t.decide(&device(), &g, "app.fake.echo"), Decision::Allow);
        assert_eq!(t.reachable_topics(&device(), &g), vec!["app.fake.ticks", "app.probe.counter"]);
        assert_eq!(t.registered(&id("fake")), Some(IdKind::Package));
        assert!(t.owns(&id("fake"), "app.fake.echo"));
        assert!(!t.owns(&id("fake"), "app.probe.echo"));

        let mut withdrawn = t.withdraw(&ext("fake"));
        withdrawn.sort();
        assert_eq!(withdrawn, vec!["app.fake.echo", "topic:app.fake.ticks"]);
        // Refused afterwards exactly like a name nobody declared.
        assert_eq!(t.decide(&device(), &g, "app.fake.echo"), Decision::Unknown);
        assert_eq!(t.decide(&device(), &g, &topic("app.fake.ticks")), Decision::Unknown);
        assert_eq!(t.registered(&id("fake")), None, "its id is free again");
        // What assembly put in is untouched.
        assert_eq!(t.decide(&device(), &g, "app.probe.echo"), Decision::Allow);
        assert!(t.withdraw(&ext("fake")).is_empty());
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
        assert_eq!(t.decide(&device(), &natives(), "harnix.hello"), Decision::Allow);
        assert_eq!(t.registered(&super::ExtId::parse("fake").unwrap()), None);
    }

    #[test]
    fn a_native_id_keeps_its_namespace_and_a_packages_first_segment_is_not_one() {
        let mut t = assembled();
        let taken = |by: &str| TableError::NamespaceTaken { id: String::new(), by: by.into() };
        let by = |e: TableError| match e {
            TableError::NamespaceTaken { by, .. } => taken(&by),
            other => other,
        };
        // The same id as a compiled-in module, one nested in it, one known only by its route.
        assert_eq!(t.install_app("probe", rows(&["app.probe.new"])).map_err(by), Err(taken("probe")));
        assert_eq!(t.install_app("probe.sub", rows(&["app.probe.sub.x"])).map_err(by), Err(taken("probe")));
        assert_eq!(t.install_app("routed", rows(&["app.routed.x"])).map_err(by), Err(taken("routed")));
        // A native id registered with no rows at all — a disabled module — counts as much.
        t.register(&id("quiet"), IdKind::Native).unwrap();
        assert_eq!(t.install_app("quiet.notes", rows(&["app.quiet.notes.x"])).map_err(by), Err(taken("quiet")));
        // Reserved first segments.
        for word in RESERVED_WORDS {
            let pkg = format!("{word}.notes");
            assert!(matches!(t.register(&id(&pkg), IdKind::Package), Err(TableError::NamespaceTaken { .. })), "{pkg}");
        }
        // The compiled-in method is still the compiled-in one.
        assert_eq!(t.decide(&device(), &natives(), "app.probe.echo"), Decision::Allow);
        // And withdrawing a compiled-in module, which only assembly does, never takes an
        // application's names, nor frees the module's id.
        t.install_app("fake", rows(&["app.fake.echo"])).unwrap();
        t.withdraw(&ext("probe"));
        assert_eq!(t.decide(&device(), &natives(), "app.fake.echo"), Decision::Allow);
        assert_eq!(t.decide(&device(), &natives(), "app.probe.echo"), Decision::Unknown);
        assert_eq!(t.registered(&id("probe")), Some(IdKind::Native));
    }

    const RESERVED_WORDS: &[&str] = names::RESERVED;

    #[test]
    fn a_name_is_owned_by_the_longest_id_it_starts_with() {
        let mut t = RouteTable::new();
        t.register(&id("acme"), IdKind::Native).unwrap();
        // `acme` owns what is under it; nobody owns what is under no one.
        t.install(&ext("acme"), rows(&["app.acme.list"])).unwrap();
        assert_eq!(
            t.install(&ext("other"), rows(&["app.acme.more"])),
            Err(TableError::NotOwner { name: "app.acme.more".into(), owner: "acme".into() })
        );
        assert!(matches!(t.install(&ext("other"), rows(&["app.nobody.x"])), Err(TableError::NotOwner { .. })));
    }

    #[test]
    fn runtime_applications_do_not_overlap_each_other() {
        let mut t = assembled();
        t.install_app("acme.notes", rows(&["app.acme.notes.list"])).unwrap();
        assert!(matches!(t.install_app("acme.notes", rows(&["app.acme.notes.other"])), Err(TableError::NamespaceTaken { .. })));
        assert!(matches!(t.install_app("acme", rows(&["app.acme.x"])), Err(TableError::NamespaceTaken { .. })));
        assert!(matches!(
            t.install_app("acme.notes.pro", rows(&["app.acme.notes.pro.x"])),
            Err(TableError::NamespaceTaken { .. })
        ));
        // A sibling shares no names.
        t.install_app("acme.todo", rows(&["app.acme.todo.list"])).unwrap();
        // Once withdrawn, the id is free again.
        t.withdraw(&ext("acme.notes"));
        t.install_app("acme.notes", rows(&["app.acme.notes.list"])).unwrap();
    }

    #[test]
    fn a_declaration_is_all_or_nothing() {
        let mut t = assembled();
        let g = natives();
        assert_eq!(
            t.install_app("fake", rows(&["app.fake.a", "app.fake.a"])),
            Err(TableError::Taken("app.fake.a".into()))
        );
        assert_eq!(
            t.install_app("fake", rows(&["app.fake.a", "harnix.hello"])),
            Err(TableError::Namespace("harnix.hello".into()))
        );
        assert_eq!(t.decide(&device(), &g, "app.fake.a"), Decision::Unknown);
        assert_eq!(t.registered(&id("fake")), None);
        // Nor does one owner's batch take half its names.
        assert_eq!(
            t.install(&kernel(), rows(&["harnix.new", "harnix.hello"])),
            Err(TableError::Taken("harnix.hello".into()))
        );
        assert_eq!(t.decide(&device(), &g, "harnix.new"), Decision::Unknown);
    }

    #[test]
    fn a_package_reaches_what_its_consent_lists_and_its_own_and_nothing_more() {
        let mut t = assembled();
        t.install(
            &kernel(),
            vec![
                method("settings.write", Access::capability(caps::ENGINE_CONFIG_WRITE)),
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
        let g = grants(
            &["probe"],
            &[("acme.notes", &["session.create", "settings.write", "topic:harnix.sessions.status", "app.acme.todo.list"])],
        );
        let me = module("acme.notes");
        // Open to every extension, and listed.
        assert_eq!(t.decide(&me, &g, "session.create"), Decision::Allow);
        // Open to every extension, not listed.
        assert_eq!(t.decide(&me, &g, "harnix.hello"), Decision::Refuse(OUTSIDE_HUB));
        assert_eq!(t.decide(&me, &g, "nothing.here"), Decision::Refuse(OUTSIDE_HUB), "told whether it exists");
        // Listed, and requiring a capability: a package holds none of the kernel's.
        assert!(matches!(t.decide(&me, &g, "settings.write"), Decision::Refuse(r) if r != OUTSIDE_HUB));
        // Its own names, unlisted; one it did not declare is unknown like any other.
        assert_eq!(t.decide(&me, &g, "app.acme.notes.echo"), Decision::Allow);
        assert_eq!(t.decide(&me, &g, "app.acme.notes.nope"), Decision::Unknown);
        assert_eq!(t.decide(&me, &g, &topic("app.acme.notes.ticks")), Decision::Allow);
        // Another application's, listed, as that one declared it.
        assert_eq!(t.decide(&me, &g, "app.acme.todo.list"), Decision::Allow);
        // Topics: subscribing is decided by the topic.
        assert_eq!(t.decide(&me, &g, "harnix.subscribe"), Decision::Allow);
        assert_eq!(t.decide(&me, &g, &topic("harnix.sessions.status")), Decision::Allow);
        assert_eq!(t.decide(&me, &g, &topic("harnix.process")), Decision::Refuse(OUTSIDE_HUB));
        // So what it is told it reaches is what it reaches.
        assert_eq!(
            t.reachable(&me, &g),
            ["app.acme.notes.echo", "app.acme.todo.list", "harnix.subscribe", "session.create"]
        );
        assert_eq!(t.reachable_topics(&me, &g), ["app.acme.notes.ticks", "harnix.sessions.status"]);
        // A compiled-in module is not narrowed by any of this.
        assert_eq!(t.decide(&module("probe"), &g, "harnix.hello"), Decision::Allow);
        // Listing: what exists and the hub does not own.
        assert_eq!(t.check_listed("session.create"), Ok(()));
        assert_eq!(t.check_listed("topic:harnix.process"), Ok(()));
        assert_eq!(t.check_listed("nothing.here"), Err(TableError::Unknown("nothing.here".into())));
    }

    #[test]
    fn an_alias_answers_as_its_row_and_is_not_a_row_of_its_own() {
        let mut t = assembled();
        let g = natives();
        t.install(&kernel(), vec![method("settings.write", Access::refused("not for devices"))]).unwrap();
        t.alias("harnix.hi", "harnix.hello").unwrap();
        t.alias("old.write", "settings.write").unwrap();
        assert_eq!(t.decide(&device(), &g, "harnix.hi"), Decision::Allow);
        assert_eq!(t.decide(&device(), &g, "old.write"), Decision::Refuse("not for devices"));
        assert_eq!(t.row("harnix.hi").map(Row::name), Some("harnix.hello"));
        assert!(!t.reachable(&device(), &g).contains(&"harnix.hi"), "listed by its new name only");
        // Neither name can be taken twice, and an alias needs something to point at.
        assert_eq!(t.alias("harnix.hello", "app.probe.echo"), Err(TableError::Taken("harnix.hello".into())));
        assert_eq!(t.alias("harnix.hi", "app.probe.echo"), Err(TableError::Taken("harnix.hi".into())));
        assert_eq!(t.alias("x", "nothing.here"), Err(TableError::Unknown("nothing.here".into())));
        assert_eq!(t.install(&kernel(), rows(&["harnix.hi"])), Err(TableError::Taken("harnix.hi".into())));
    }

    #[test]
    fn rows_are_installed_and_withdrawn_by_owner() {
        let mut t = assembled();
        assert_eq!(t.owner("app.probe.echo"), Some(&ext("probe")));
        assert_eq!(t.owner("harnix.hello"), Some(&kernel()));
        let mut gone = t.withdraw(&ext("probe"));
        gone.sort();
        assert_eq!(gone, ["app.probe.echo", "topic:app.probe.counter"]);
        assert_eq!(t.owner("route:routed"), Some(&ext("routed")));
        assert_eq!(t.decide(&device(), &natives(), "harnix.hello"), Decision::Allow);
    }
}
