//! An application's isolated interface: `[interface]` and its `[[surface]]`s.
//!
//! A surface is a region of the shell an application's page fills, in a sandboxed frame of its
//! own (`/apps/<id>/ui/<page>`): an opaque origin, no network (`connect-src 'none'`), and only the
//! shell's bridge to talk through (`harnix-ui-bridge@1`, `@harnix/app-sdk`). The shell owns the
//! frame around it — its title, loading, failure and reload — and everything the page asks for is
//! done by the shell, filtered by what is written here.
//!
//! ```toml
//! [interface]                    # optional
//! isolate = true                 # a trusted publisher's interface in frames too (see below)
//! hub = ["harnix.sessions", "harnix.send"]
//!                                # what a frame may call and subscribe to beyond its own
//!                                # app.<id>.*: names of FRAME_HUB, `topic:` for topics
//! links = true                   # may ask the shell to open a web page (confirmed each time)
//!
//! [[surface]]
//! point = "space"                # see POINTS
//! space = "acme.notes"           # the interface space: the app's id or `<id>.<name>`
//! page = "frame/board.html"      # under ui/
//! title = { en = "Notes", zh-CN = "笔记" }   # or one string
//! icon = "file"                  # the space's entry; a shell glyph
//! order = 60
//! center = "hidden"              # hidden | minimized | chat (no page: the conversation)
//! ```
//!
//! # How the interface runs
//!
//! A store-signed or unsigned application's interface runs in its frames, and its `[[ui]]` modules
//! (which would run in the page, with the page's rights) do not run at all. A trusted publisher's
//! runs in the page, as its `[[ui]]` modules, unless `[interface] isolate = true` or it has no
//! `[[ui]]` module: then in frames too. [`runs_in_frames`] is that rule, in one place.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::manifest::{safe_relative, valid_dotted, valid_name, Error, MANIFEST};
use crate::signature::Trust;

/// Where a surface goes: the regions of the shell a frame may fill.
///
/// | point | the shell's | made | gone |
/// |---|---|---|---|
/// | `space` | a space: its entry, and its centre (`overview`) | first entered | when the app stops (kept, hidden, while away) |
/// | `space.sidebar` | that space's left column, under the space list | first entered | when the app stops |
/// | `space.tab` | a tab of its right column | first selected | when the app stops |
/// | `space.secondary` | its second side panel | opened (`secondary.open`) | closed |
/// | `details.panel` | the right column's details, for items of `kinds` | an item is shown | another is |
/// | `settings.section` | a section of Settings | Settings opens on it | Settings closes |
/// | `home.card` | a card under a new conversation's composer (auto height) | the page shows it | it is left |
/// | `modal` | a dialog the shell draws (`openModal`) | opened | closed |
pub const POINTS: &[&str] = &[
    "space",
    "space.sidebar",
    "space.tab",
    "space.secondary",
    "details.panel",
    "settings.section",
    "home.card",
    "modal",
];

/// How the shell narrows a call a frame makes through a name its `[interface] hub` lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Says nothing of any session or application: passed on as asked.
    Open,
    /// About the sessions of a space: only of the application's own (`[[space]]`). The shell
    /// fills `space` in when it is left out, and refuses another.
    Space,
    /// About one session (`session_id`, and `instance`): only one in a space of the
    /// application's own. The shell looks it up before passing the call on.
    Session,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Space => "space",
            Self::Session => "session",
        }
    }
}

/// Everything `[interface] hub` may list — anything else is refused when the manifest is read —
/// and how the shell narrows each. An allowlist: the page's connection reaches a great deal (it
/// is a device's), and a frame is the application's, which is not the person's. The shell has the
/// same table (`FRAME_HUB` in `@harnix/app-sdk/protocol`); a test checks they agree.
pub const FRAME_HUB: &[(&str, Scope)] = &[
    ("harnix.sessions", Scope::Space),
    ("harnix.send", Scope::Session),
    ("harnix.sessions.tasks", Scope::Session),
    ("session.get", Scope::Session),
    ("session.history", Scope::Session),
    ("session.entries", Scope::Session),
    ("session.interrupt", Scope::Session),
    ("harnix.spaces.list", Scope::Open),
    ("topic:harnix.sessions.tasks", Scope::Session),
    ("topic:harnix.spaces", Scope::Open),
];

/// How a frame's call through `name` (`topic:<name>` for a topic) is narrowed; `None` when no
/// frame may be given it.
pub fn scope_of(name: &str) -> Option<Scope> {
    FRAME_HUB.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
}

/// Shell-side methods a frame is never given, whatever its `hub` says: installing and configuring
/// this Harnix, its devices and its other instances, uploading (the bridge proxies uploads), and
/// the page's own subscriptions (the bridge's `subscribe` is the way).
const NEVER: &[&str] = &[
    "harnix.apps.",
    "harnix.plugins.",
    "harnix.settings.",
    "harnix.devices.",
    "harnix.instances.",
    "harnix.system.",
    "harnix.upload.",
    "harnix.prompts.",
    "harnix.prefs.",
    "harnix.subscribe",
    "harnix.unsubscribe",
    "harnix.attach",
    "harnix.detach",
    "harnix.requestHeaders",
];

/// `[interface]`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Interface {
    /// A trusted publisher's interface in frames rather than in the page.
    #[serde(default)]
    pub isolate: bool,
    /// What a frame may call and subscribe to (`topic:<name>`) beyond the application's own
    /// `app.<id>.*`: names of [`FRAME_HUB`]. Admitted as the page — a device — by the hub, after
    /// the shell has checked the name against this list and narrowed the call ([`Scope`]).
    #[serde(default)]
    pub hub: Vec<String>,
    /// The bridge's `openLink`: the shell asks the person, then opens the page outside.
    #[serde(default)]
    pub links: bool,
}

/// A surface's title: one string, or one per locale (`en`, `zh-CN`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Title {
    One(String),
    ByLocale(BTreeMap<String, String>),
}

/// One `[[surface]]`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Surface {
    pub point: String,
    /// Unique among the application's surfaces of the same point (and space). A tab's, a
    /// settings section's and a modal's are required; the others default to the point.
    #[serde(default)]
    pub id: Option<String>,
    /// The page, under `ui/`: an `.html` file. Absent only for a space whose centre is the
    /// conversation (`center = "chat"`).
    #[serde(default)]
    pub page: Option<String>,
    #[serde(default)]
    pub title: Option<Title>,
    /// The interface space the `space*` surfaces belong to.
    #[serde(default)]
    pub space: Option<String>,
    /// A space's entry glyph, the shell's (`file`, `chat`, …).
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub order: Option<i64>,
    /// A space's centre: its page alone (`hidden`, the default), its page over the conversation's
    /// bar (`minimized`), or the conversation (`chat`).
    #[serde(default)]
    pub center: Option<String>,
    /// A `details.panel`'s: the item kinds it draws, each `<id>.<name>`.
    #[serde(default)]
    pub kinds: Vec<String>,
    /// A modal's: the shell's wide dialog.
    #[serde(default)]
    pub wide: bool,
}

impl Surface {
    /// Its id: as written, or the point's.
    pub fn id(&self) -> &str {
        self.id.as_deref().unwrap_or(&self.point)
    }
}

/// Whether an application's interface runs in frames (true) or in the page as its `[[ui]]`
/// modules (false), by how far its package is trusted.
pub fn runs_in_frames(trust: Trust, interface: &Interface, has_modules: bool) -> bool {
    trust != Trust::Trusted || interface.isolate || !has_modules
}

/// Whether a frame of an application may call `method` (or, with `topic`, subscribe to it): its
/// own `app.<id>.*`, or a name its `[interface] hub` lists.
pub fn admits(id: &str, interface: &Interface, name: &str, topic: bool) -> bool {
    if name.starts_with(&format!("app.{id}.")) {
        return true;
    }
    let wanted = if topic {
        format!("topic:{name}")
    } else {
        name.to_string()
    };
    interface.hub.contains(&wanted)
}

/// Check `[interface]` and the `[[surface]]`s of the application `id`.
pub fn check(id: &str, interface: &Interface, surfaces: &[Surface]) -> Result<(), Error> {
    for entry in &interface.hub {
        let name = entry.strip_prefix("topic:").unwrap_or(entry);
        let family = name.starts_with("harnix.") || name.starts_with("session.");
        if !valid_dotted(name) || !family {
            return Err(format!(
                "{MANIFEST}: `{entry}` in `[interface] hub` is a `harnix.*` or `session.*` method, \
                 or `topic:` and such a topic (the application's own `app.{id}.*` needs no entry)"
            ));
        }
        if let Some(never) = NEVER
            .iter()
            .find(|n| name == n.trim_end_matches('.') || name.starts_with(*n))
        {
            return Err(format!(
                "{MANIFEST}: `{entry}` in `[interface] hub` is the shell's own (`{never}…`): an \
                 application's frame is never given it"
            ));
        }
        if scope_of(entry).is_none() {
            return Err(format!(
                "{MANIFEST}: `{entry}` in `[interface] hub` is not one a frame may be given; it may \
                 list {}",
                FRAME_HUB.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
            ));
        }
    }
    let ours = |space: &str| {
        space == id
            || space
                .strip_prefix(&format!("{id}."))
                .is_some_and(valid_name)
    };
    let mut seen = BTreeSet::new();
    let mut spaces = BTreeSet::new();
    for s in surfaces {
        let at = format!("the {} surface `{}`", s.point, s.id());
        if !POINTS.contains(&s.point.as_str()) {
            return Err(format!(
                "{MANIFEST}: a surface's `point` is one of {}, not `{}`",
                POINTS.join(", "),
                s.point
            ));
        }
        if let Some(own) = &s.id {
            if !valid_name(own) {
                return Err(format!(
                    "{MANIFEST}: {at}: an id is lowercase letters, digits and `-`"
                ));
            }
        }
        if matches!(s.point.as_str(), "space.tab" | "settings.section" | "modal") && s.id.is_none()
        {
            return Err(format!("{MANIFEST}: a {} surface needs an `id`", s.point));
        }
        let in_space = s.point == "space" || s.point.starts_with("space.");
        match (&s.space, in_space) {
            (Some(space), true) if !ours(space) => {
                return Err(format!(
                    "{MANIFEST}: {at}: its `space` is `{id}` or `{id}.<name>`, not `{space}`"
                ))
            }
            (None, true) => {
                return Err(format!(
                    "{MANIFEST}: {at}: a space's surface names its `space`"
                ))
            }
            (Some(_), false) => {
                return Err(format!(
                    "{MANIFEST}: {at}: only a space's surfaces have a `space`"
                ))
            }
            _ => {}
        }
        let center = s.center.as_deref().unwrap_or("hidden");
        if s.point == "space" {
            if !["hidden", "minimized", "chat"].contains(&center) {
                return Err(format!(
                    "{MANIFEST}: {at}: `center` is hidden, minimized or chat, not `{center}`"
                ));
            }
        } else if s.center.is_some() || s.icon.is_some() {
            return Err(format!(
                "{MANIFEST}: {at}: `center` and `icon` are a space's (`point = \"space\"`)"
            ));
        }
        match (&s.page, s.point == "space" && center == "chat") {
            (Some(page), false) => {
                if !safe_relative(page) || !page.ends_with(".html") {
                    return Err(format!(
                        "{MANIFEST}: {at}: its page `{page}` is an `.html` file under ui/"
                    ));
                }
            }
            (Some(_), true) => {
                return Err(format!(
                    "{MANIFEST}: {at}: a space whose centre is the conversation has no page"
                ))
            }
            (None, false) => return Err(format!("{MANIFEST}: {at} has no `page`")),
            (None, true) => {}
        }
        if let Some(Title::ByLocale(titles)) = &s.title {
            if titles.is_empty() || titles.keys().any(|l| l != "en" && l != "zh-CN") {
                return Err(format!(
                    "{MANIFEST}: {at}: a title by locale has `en` and `zh-CN` keys"
                ));
            }
        }
        if let Some(icon) = &s.icon {
            if icon.is_empty()
                || !icon
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
            {
                return Err(format!("{MANIFEST}: {at}: `{icon}` is not a glyph name"));
            }
        }
        if s.point == "details.panel" {
            if s.kinds.is_empty() {
                return Err(format!(
                    "{MANIFEST}: {at}: a details panel lists the `kinds` it draws"
                ));
            }
            for kind in &s.kinds {
                if !kind
                    .strip_prefix(&format!("{id}."))
                    .is_some_and(valid_dotted)
                {
                    return Err(format!(
                        "{MANIFEST}: {at}: the kind `{kind}` is `{id}.<name>`: an application draws its own items"
                    ));
                }
            }
        } else if !s.kinds.is_empty() {
            return Err(format!("{MANIFEST}: {at}: `kinds` are a details panel's"));
        }
        if s.wide && s.point != "modal" {
            return Err(format!("{MANIFEST}: {at}: `wide` is a modal's"));
        }
        let scope = s.space.as_deref().unwrap_or("");
        if !seen.insert((s.point.clone(), scope.to_string(), s.id().to_string())) {
            return Err(format!("{MANIFEST}: {at} is declared twice"));
        }
        // One sidebar and one secondary panel per space.
        if matches!(s.point.as_str(), "space.sidebar" | "space.secondary")
            && !seen.insert((s.point.clone(), scope.to_string(), String::new()))
        {
            return Err(format!("{MANIFEST}: a space has one {} surface", s.point));
        }
        if s.point == "space" {
            spaces.insert(scope.to_string());
        }
    }
    for s in surfaces.iter().filter(|s| s.point.starts_with("space.")) {
        let space = s.space.as_deref().unwrap_or("");
        if !spaces.contains(space) {
            return Err(format!(
                "{MANIFEST}: the {} surface `{}` is in the space `{space}`, which has no `point = \"space\"` surface",
                s.point,
                s.id()
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface(toml: &str) -> Surface {
        toml::from_str(toml).unwrap()
    }

    const ID: &str = "acme.notes";

    #[test]
    fn who_runs_in_frames() {
        let plain = Interface::default();
        let isolated = Interface {
            isolate: true,
            ..Interface::default()
        };
        assert!(runs_in_frames(Trust::Store, &plain, true));
        assert!(runs_in_frames(Trust::Unsigned, &plain, true));
        assert!(!runs_in_frames(Trust::Trusted, &plain, true));
        assert!(runs_in_frames(Trust::Trusted, &isolated, true));
        // A trusted application with nothing to run in the page has only its frames.
        assert!(runs_in_frames(Trust::Trusted, &plain, false));
    }

    #[test]
    fn a_frame_reaches_its_own_names_and_what_its_hub_lists() {
        let interface = Interface {
            hub: vec![
                "harnix.sessions".into(),
                "topic:harnix.sessions.tasks".into(),
            ],
            ..Interface::default()
        };
        assert!(admits(ID, &interface, "app.acme.notes.list", false));
        assert!(admits(ID, &interface, "app.acme.notes.changes", true));
        assert!(admits(ID, &interface, "harnix.sessions", false));
        assert!(admits(ID, &interface, "harnix.sessions.tasks", true));
        assert!(!admits(ID, &interface, "harnix.sessions.tasks", false));
        assert!(!admits(ID, &interface, "harnix.sessions", true));
        assert!(!admits(ID, &interface, "app.acme.other.list", false));
        assert!(!admits(ID, &interface, "app.acme.notesx.list", false));
    }

    #[test]
    fn a_good_set_of_surfaces_checks() {
        let surfaces = vec![
            surface("point = \"space\"\nspace = \"acme.notes\"\npage = \"board.html\"\ntitle = { en = \"Notes\", zh-CN = \"笔记\" }"),
            surface("point = \"space.sidebar\"\nspace = \"acme.notes\"\npage = \"side.html\""),
            surface("point = \"space.tab\"\nid = \"info\"\nspace = \"acme.notes\"\npage = \"tab.html\""),
            surface("point = \"space.secondary\"\nspace = \"acme.notes\"\npage = \"more.html\""),
            surface("point = \"space\"\nspace = \"acme.notes.chat\"\ncenter = \"chat\""),
            surface("point = \"details.panel\"\npage = \"item.html\"\nkinds = [\"acme.notes.note\"]"),
            surface("point = \"settings.section\"\nid = \"notes\"\npage = \"settings.html\""),
            surface("point = \"home.card\"\npage = \"card.html\""),
            surface("point = \"modal\"\nid = \"edit\"\npage = \"edit.html\"\nwide = true"),
        ];
        check(
            ID,
            &Interface {
                hub: vec!["harnix.sessions".into()],
                ..Interface::default()
            },
            &surfaces,
        )
        .unwrap();
    }

    #[test]
    fn what_does_not_check_says_why() {
        let one = |toml: &str| check(ID, &Interface::default(), &[surface(toml)]).unwrap_err();
        assert!(one("point = \"flow.block\"\npage = \"a.html\"").contains("one of"));
        assert!(one("point = \"home.card\"\npage = \"../a.html\"").contains("under ui/"));
        assert!(one("point = \"home.card\"\npage = \"a.js\"").contains(".html"));
        assert!(one("point = \"home.card\"").contains("no `page`"));
        assert!(one("point = \"modal\"\npage = \"a.html\"").contains("needs an `id`"));
        assert!(one("point = \"space\"\npage = \"a.html\"").contains("names its `space`"));
        assert!(
            one("point = \"space\"\nspace = \"other.app\"\npage = \"a.html\"")
                .contains("`acme.notes` or")
        );
        assert!(
            one("point = \"home.card\"\nspace = \"acme.notes\"\npage = \"a.html\"")
                .contains("only a space's")
        );
        assert!(one(
            "point = \"space\"\nspace = \"acme.notes\"\npage = \"a.html\"\ncenter = \"sideways\""
        )
        .contains("hidden, minimized or chat"));
        assert!(one(
            "point = \"space\"\nspace = \"acme.notes\"\ncenter = \"chat\"\npage = \"a.html\""
        )
        .contains("has no page"));
        assert!(one(
            "point = \"space.tab\"\nid = \"t\"\nspace = \"acme.notes\"\npage = \"a.html\""
        )
        .contains("has no `point = \"space\"`"));
        assert!(one("point = \"details.panel\"\npage = \"a.html\"").contains("`kinds`"));
        assert!(
            one("point = \"details.panel\"\npage = \"a.html\"\nkinds = [\"other.kind\"]")
                .contains("its own items")
        );
        assert!(one("point = \"home.card\"\npage = \"a.html\"\nwide = true").contains("a modal's"));
        assert!(
            one("point = \"home.card\"\npage = \"a.html\"\ntitle = { fr = \"x\" }")
                .contains("`en` and `zh-CN`")
        );
        let twice = check(
            ID,
            &Interface::default(),
            &[
                surface("point = \"home.card\"\npage = \"a.html\""),
                surface("point = \"home.card\"\npage = \"b.html\""),
            ],
        )
        .unwrap_err();
        assert!(twice.contains("declared twice"), "{twice}");
        let hub = |entry: &str| {
            check(
                ID,
                &Interface {
                    hub: vec![entry.into()],
                    ..Interface::default()
                },
                &[],
            )
            .unwrap_err()
        };
        assert!(hub("app.acme.notes.list").contains("`harnix.*` or `session.*`"));
        assert!(hub("harnix.apps.uninstall").contains("the shell's own"));
        assert!(hub("harnix.subscribe").contains("the shell's own"));
        assert!(hub("topic:harnix.settings.changed").contains("the shell's own"));
        // An allowlist: what it does not list is not given, even of the families it may reach.
        for entry in ["session.create", "session.delete", "harnix.entries.append", "topic:harnix.sessions.status"] {
            assert!(hub(entry).contains("not one a frame may be given"), "{entry}");
        }
        check(
            ID,
            &Interface {
                hub: FRAME_HUB.iter().map(|(n, _)| n.to_string()).collect(),
                ..Interface::default()
            },
            &[],
        )
        .unwrap();
    }

    #[test]
    fn the_shell_narrows_frames_by_the_same_table() {
        let protocol = include_str!("../../../ui/packages/app-sdk/src/protocol.ts");
        let start = protocol.find("export const FRAME_HUB").expect("FRAME_HUB in protocol.ts");
        let table = &protocol[start..start + protocol[start..].find("};").unwrap()];
        for (name, scope) in FRAME_HUB {
            let entry = format!("\"{name}\": \"{}\"", scope.as_str());
            assert!(table.contains(&entry), "protocol.ts's FRAME_HUB lacks {entry}");
        }
        assert_eq!(table.matches("\": \"").count(), FRAME_HUB.len(), "the same names, no more");
    }
}
