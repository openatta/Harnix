//! The interface half of an extension, declared: the shell draws it.
//!
//! A native extension's interface is usually a UI package compiled into the shell, which the
//! shell activates for it ([`UiDecl::bundle`]). A package's is frames ([`SurfaceDecl`]) and the
//! declarative points it fills ([`InlineDecl`]). Either may load ES modules into the page
//! ([`ModuleDecl`]: that takes `ui.page`), and a native one may define contribution points of its
//! own ([`PointDecl`]) for other extensions to fill. All of it goes into `harnix.ui` and the
//! diagnostics.

use serde::Serialize;
use serde_json::Value;

use crate::catalog::{Frequency, PointKind};
use crate::context::ExtensionContext;

/// What one extension declares of its interface.
#[derive(Debug, Clone, Default, Serialize)]
pub struct UiDecl {
    #[serde(skip_serializing_if = "Option::is_none")]
    bundle: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    surfaces: Vec<SurfaceDecl>,
    #[serde(skip_serializing_if = "InlineDecl::is_empty")]
    inline: InlineDecl,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    modules: Vec<ModuleDecl>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    points: Vec<PointDecl>,
}

impl UiDecl {
    /// The UI package compiled into the shell that is this extension's interface, by the id it
    /// registers under; the extension's own id when that is the same. The shell calls the
    /// package's `register` only while this extension is enabled.
    pub fn bundle(&mut self, id: &str) -> &mut Self {
        self.bundle = Some(id.to_string());
        self
    }

    /// A frame.
    pub fn surface(&mut self, surface: SurfaceDecl) -> &mut Self {
        self.surfaces.push(surface);
        self
    }

    /// The declarative points it fills.
    pub fn inline(&mut self) -> &mut InlineDecl {
        &mut self.inline
    }

    /// An ES module loaded into the page.
    pub fn module(&mut self, module: ModuleDecl) -> &mut Self {
        self.modules.push(module);
        self
    }

    /// A contribution point of its own, `<its id>.<name>`.
    pub fn point(&mut self, point: PointDecl) -> &mut Self {
        self.points.push(point);
        self
    }

    pub fn bundle_id(&self) -> Option<&str> {
        self.bundle.as_deref()
    }

    pub fn surfaces(&self) -> &[SurfaceDecl] {
        &self.surfaces
    }

    pub fn inline_entries(&self) -> &[InlineEntry] {
        &self.inline.entries
    }

    pub fn modules(&self) -> &[ModuleDecl] {
        &self.modules
    }

    pub fn points(&self) -> &[PointDecl] {
        &self.points
    }

    pub fn is_empty(&self) -> bool {
        self.bundle.is_none()
            && self.surfaces.is_empty()
            && self.inline.is_empty()
            && self.modules.is_empty()
            && self.points.is_empty()
    }
}

/// A frame of the extension's own pages: `entry` under its static files.
#[derive(Debug, Clone, Serialize)]
pub struct SurfaceDecl {
    id: String,
    entry: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    title: String,
}

impl SurfaceDecl {
    pub fn new(id: &str, entry: &str) -> Self {
        Self { id: id.to_string(), entry: entry.to_string(), title: String::new() }
    }

    pub fn title(mut self, title: &str) -> Self {
        self.title = title.to_string();
        self
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn entry(&self) -> &str {
        &self.entry
    }
}

/// Which declarative point an inline entry fills.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InlineKind {
    ToolRow,
    FlowBlock,
    Command,
    SidebarGroup,
}

/// One declarative entry: the point, and what it says there.
#[derive(Debug, Clone, Serialize)]
pub struct InlineEntry {
    kind: InlineKind,
    spec: Value,
}

impl InlineEntry {
    pub fn kind(&self) -> InlineKind {
        self.kind
    }

    pub fn spec(&self) -> &Value {
        &self.spec
    }
}

/// The declarative points an extension fills: no code of its own runs for them.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(transparent)]
pub struct InlineDecl {
    entries: Vec<InlineEntry>,
}

impl InlineDecl {
    /// How a tool's call is drawn in the stream.
    pub fn tool_row(&mut self, spec: Value) -> &mut Self {
        self.add(InlineKind::ToolRow, spec)
    }

    /// How an entry it writes into a session is drawn.
    pub fn flow_block(&mut self, spec: Value) -> &mut Self {
        self.add(InlineKind::FlowBlock, spec)
    }

    /// A `/` command or `@` reference.
    pub fn command(&mut self, spec: Value) -> &mut Self {
        self.add(InlineKind::Command, spec)
    }

    /// A group in the session list.
    pub fn sidebar_group(&mut self, spec: Value) -> &mut Self {
        self.add(InlineKind::SidebarGroup, spec)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn add(&mut self, kind: InlineKind, spec: Value) -> &mut Self {
        self.entries.push(InlineEntry { kind, spec });
        self
    }
}

/// An ES module the page loads for a point. Needs `ui.page`.
#[derive(Debug, Clone, Serialize)]
pub struct ModuleDecl {
    point: String,
    entry: String,
    api_version: u32,
}

impl ModuleDecl {
    pub fn new(point: &str, entry: &str) -> Self {
        Self { point: point.to_string(), entry: entry.to_string(), api_version: crate::CONTRIB_API_VERSION }
    }

    pub fn point(&self) -> &str {
        &self.point
    }

    pub fn entry(&self) -> &str {
        &self.entry
    }
}

/// A contribution point an extension defines, for others to fill: in the generated catalog with
/// the shell's own.
#[derive(Debug, Clone)]
pub struct PointDecl {
    id: String,
    kind: PointKind,
    frequency: Frequency,
    what: String,
    when: String,
}

impl PointDecl {
    pub fn new(id: &str, kind: PointKind, frequency: Frequency) -> Self {
        Self { id: id.to_string(), kind, frequency, what: String::new(), when: String::new() }
    }

    /// What a contribution here supplies.
    pub fn what(mut self, text: &str) -> Self {
        self.what = text.to_string();
        self
    }

    /// When it is evaluated.
    pub fn when(mut self, text: &str) -> Self {
        self.when = text.to_string();
        self
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

impl Serialize for PointDecl {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        serde_json::json!({
            "id": self.id,
            "kind": self.kind.as_str(),
            "frequency": self.frequency.as_str(),
            "what": self.what,
            "when": self.when,
        })
        .serialize(s)
    }
}

/// One interface entry published while the extension runs: a module for a point.
#[derive(Debug, Clone, Serialize)]
pub struct UiEntry {
    point: String,
    module: String,
    api_version: u32,
    #[serde(rename = "plugin", skip_serializing_if = "Option::is_none")]
    owner: Option<String>,
}

impl UiEntry {
    /// `module` is the URL the page loads it from, one of the extension's own routes.
    pub fn new(point: &str, module: &str, api_version: u32) -> Self {
        Self { point: point.to_string(), module: module.to_string(), api_version, owner: None }
    }

    /// Whose module it is when that is not the extension publishing it: the package an
    /// extension that serves packages' interfaces got it from. The page signs what the module
    /// registers with this name.
    pub fn owner(mut self, name: &str) -> Self {
        self.owner = Some(name.to_string());
        self
    }
}

/// What an extension publishes of its interface while it runs, besides what it declared: the
/// entries a set of installed engine packages bring, for one. Each call replaces the last.
pub struct UiPublisher<'a> {
    cx: &'a ExtensionContext,
}

impl<'a> UiPublisher<'a> {
    pub(crate) fn new(cx: &'a ExtensionContext) -> Self {
        Self { cx }
    }

    pub fn publish(&self, entries: Vec<UiEntry>) {
        let values = entries.iter().filter_map(|e| serde_json::to_value(e).ok()).collect();
        self.cx.ports().ui(self.cx.id(), values);
    }
}
