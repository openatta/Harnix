//! How a name is built and whose it is, in one place: a topic's and a route's row names, what an
//! application id may be, which names are inside an extension's namespace, and who owns one.
//! Used when ids are registered and rows installed — never to find what answers a call: that is
//! the row's handler.
//!
//! **Who owns a name** is the registered extension id it starts with by whole segments, the
//! longest one ([`owner_of`]). So no two registered ids may overlap ([`overlap`]): a native
//! `example` and a package `example.showcase` would both own `example.showcase.*`, and the native
//! one, registered first, keeps it; the package fails `namespace_taken`. A package's first
//! segment is never a reserved word ([`RESERVED`]) nor a native extension's id.

pub use harnix_contract::s1::RESERVED;
use harnix_contract::ExtId;

/// The row-name prefix of a route.
pub(crate) const ROUTE: &str = "route:";
/// The row-name prefix of a topic.
pub(crate) const TOPIC: &str = "topic:";

/// The methods decided by what they name rather than by themselves: subscribing to a topic is
/// decided as `topic:<name>`, so a runtime principal reaches these whatever it listed, and the
/// topic decides.
pub(crate) const DECIDED_BY_TOPIC: &[&str] = &["harnix.topics.subscribe", "harnix.topics.unsubscribe"];

/// The table's name for a module's routes, `/apps/<id>/`.
pub fn route(module: &str) -> String {
    format!("{ROUTE}{module}")
}

/// The table's name for subscribing to a host topic.
pub fn topic(name: &str) -> String {
    format!("{TOPIC}{name}")
}

/// The topic a row name stands for, when it is one.
pub(crate) fn topic_of(name: &str) -> Option<&str> {
    name.strip_prefix(TOPIC)
}

/// The module a route row is for, when it is one.
pub(crate) fn route_of(name: &str) -> Option<&str> {
    name.strip_prefix(ROUTE)
}

pub(crate) fn is_surface(name: &str) -> bool {
    topic_of(name).is_some() || route_of(name).is_some()
}

/// Whether an id can name a runtime application: one or more dot-separated segments of lowercase
/// letters, digits and `-`, each starting with a letter (`fake`, `acme.notes`); at most 64
/// characters. It becomes a method prefix (`app.<id>.`), a directory name and a space id.
pub fn valid_app_id(id: &str) -> bool {
    id.len() <= 64
        && id.split('.').all(|segment| {
            let mut chars = segment.chars();
            matches!(chars.next(), Some('a'..='z')) && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '-'))
        })
}

/// Whether `prefix` begins `name` by whole segments: `acme` begins `acme.notes`, not `acmeish`.
pub fn whole_prefix(prefix: &str, name: &str) -> bool {
    name == prefix || name.strip_prefix(prefix).is_some_and(|rest| rest.starts_with('.'))
}

/// Whether two extension ids would share names: equal, or one a whole-segment prefix of the
/// other (`acme` owns `acme.notes.x` as much as `acme.notes` does).
pub fn overlap(a: &str, b: &str) -> bool {
    whole_prefix(a, b) || whole_prefix(b, a)
}

/// Who owns `name` among `ids`: the longest that begins it by whole segments.
pub fn owner_of<'a>(name: &str, ids: impl IntoIterator<Item = &'a ExtId>) -> Option<&'a ExtId> {
    ids.into_iter()
        .filter(|id| whole_prefix(id.as_str(), name))
        .max_by_key(|id| id.as_str().len())
}

/// The part of a row name an extension id is matched against: what follows `app.` in a method
/// or a topic, while extensions' names are `app.<id>.<name>`.
pub(crate) fn extension_part(name: &str) -> Option<&str> {
    let name = topic_of(name).unwrap_or(name);
    name.strip_prefix("app.")
}

/// The full name of an application's method or topic: `app.<id>.<name>`.
pub fn app_name(app: &str, name: &str) -> String {
    format!("app.{app}.{name}")
}

/// Whether a name is inside an application's namespace: `app.<id>.<name>`, or that topic, by
/// whole segments — `app.acmeish.x` is not `acme`'s.
pub fn in_app_namespace(id: &str, name: &str) -> bool {
    let own = |name: &str| {
        name.strip_prefix("app.")
            .and_then(|n| n.strip_prefix(id))
            .is_some_and(|rest| rest.starts_with('.'))
    };
    own(name) || topic_of(name).is_some_and(own)
}

/// The part of `name` after `app.<app>.` (or `topic:app.<app>.`), when it is a well-formed name
/// inside the application's own namespace.
pub(crate) fn short_app_name<'a>(app: &str, name: &'a str) -> Option<&'a str> {
    let method_prefix = format!("app.{app}.");
    let topic_prefix = format!("{TOPIC}app.{app}.");
    name.strip_prefix(&method_prefix)
        .or_else(|| name.strip_prefix(&topic_prefix))
        .filter(|short| valid_short_name(short))
}

/// Whether a name in an application's `hub` belongs to an application, and so is that
/// application's to admit rather than a grant to check.
pub fn names_an_application(name: &str) -> bool {
    name.starts_with("app.") || topic_of(name).is_some_and(|t| t.starts_with("app."))
}

/// A method or topic name inside an application's namespace: dot-separated segments of ASCII
/// letters, digits, `_` and `-`; at most 128 characters.
fn valid_short_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .split('.')
            .all(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_is_by_whole_segments() {
        assert!(overlap("acme", "acme"));
        assert!(overlap("acme", "acme.notes"));
        assert!(overlap("acme.notes", "acme"));
        assert!(!overlap("acme", "acmex"));
        assert!(!overlap("acme.notes", "acme.todo"));
    }

    #[test]
    fn a_name_is_the_longest_whole_segment_prefixs() {
        let ids: Vec<ExtId> = ["acme", "acme.notes", "flow"].iter().map(|s| ExtId::parse(s).unwrap()).collect();
        assert_eq!(owner_of("acme.notes.list", &ids).map(ExtId::as_str), Some("acme.notes"));
        assert_eq!(owner_of("acme.todo.list", &ids).map(ExtId::as_str), Some("acme"));
        assert_eq!(owner_of("acmeish.x", &ids), None);
        assert_eq!(owner_of("flow", &ids).map(ExtId::as_str), Some("flow"));
        assert_eq!(extension_part("topic:flow.runs"), Some("flow.runs"));
        assert_eq!(extension_part("harnix.hello"), None);
    }

    #[test]
    fn an_applications_namespace_is_by_whole_segments() {
        assert!(in_app_namespace("acme", "app.acme.x"));
        assert!(in_app_namespace("acme", "topic:app.acme.ticks"));
        assert!(!in_app_namespace("acme", "app.acmeish.x"));
        assert!(!in_app_namespace("acme", "app.acme"));
        assert!(!in_app_namespace("acme", "harnix.acme.x"));
        assert_eq!(short_app_name("acme", "app.acme.notes.list"), Some("notes.list"));
        assert_eq!(short_app_name("acme", "app.acme..x"), None);
    }
}
