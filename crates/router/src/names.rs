//! How a name is built, in one place: a topic's and a route's row names, what an application id
//! may be, and which names are inside an application's own namespace. Used when rows are
//! installed and decided — never to find what answers a call: that is the row's handler.

/// The row-name prefix of a route.
pub(crate) const ROUTE: &str = "route:";
/// The row-name prefix of a topic.
pub(crate) const TOPIC: &str = "topic:";

/// The methods decided by what they name rather than by themselves: subscribing to a topic is
/// decided as `topic:<name>`, so a runtime principal reaches these whatever it listed, and the
/// topic decides.
pub(crate) const DECIDED_BY_TOPIC: &[&str] = &["harnix.subscribe", "harnix.unsubscribe"];

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

/// Whether two application ids would share names: equal, or one a dot-prefix of the other
/// (`acme` owns `app.acme.notes.x` as much as `acme.notes` does).
pub fn apps_overlap(a: &str, b: &str) -> bool {
    let nested =
        |outer: &str, inner: &str| inner.len() > outer.len() && inner.starts_with(outer) && inner.as_bytes()[outer.len()] == b'.';
    a == b || nested(a, b) || nested(b, a)
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
        assert!(apps_overlap("acme", "acme"));
        assert!(apps_overlap("acme", "acme.notes"));
        assert!(apps_overlap("acme.notes", "acme"));
        assert!(!apps_overlap("acme", "acmex"));
        assert!(!apps_overlap("acme.notes", "acme.todo"));
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
