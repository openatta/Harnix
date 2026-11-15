//! A scene as a layer over another: only what changes is said, the rest is the base's.
//!
//! A layer names its base ([`SceneBase`]) and what it changes — its id and name, a block added at
//! the end of the system prompt, tools of its own, whether someone answers, whether a project is
//! needed, whether the session is named by a model call. Whatever it does not say is the base's.
//!
//! The engine serves only its own scenes. A layer is not built into it: the hub expands the one a
//! session is created in ([`SceneLayers`]) into the engine's scene it runs on and what the
//! session is created with ([`ExpandedScene`]), and the session's own tools are answered on the
//! link that created it.
//!
//! An id, a name or a description may say `{base}`, which is replaced by the base's own:
//! `flow-{base}`, `"{base} (flow)"`.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use crate::tool::Tool;

/// The most tools one session may have of its own.
const MAX_TOOLS: usize = 64;

/// A scene, as a change to another one.
#[non_exhaustive]
#[derive(Clone)]
pub struct SceneLayer {
    id: String,
    base: SceneBase,
    name: Option<String>,
    description: Option<String>,
    prompts: Vec<PromptText>,
    tools: Vec<Arc<dyn Tool>>,
    interactive: Option<bool>,
    requires_project: Option<bool>,
    auto_name: Option<bool>,
}

impl SceneLayer {
    /// A scene served as `id` (`{base}` is the base's id), and the base's in everything else.
    pub fn new(id: &str, base: SceneBase) -> Self {
        Self {
            id: id.to_string(),
            base,
            name: None,
            description: None,
            prompts: Vec::new(),
            tools: Vec::new(),
            interactive: None,
            requires_project: None,
            auto_name: None,
        }
    }

    pub fn name(mut self, name: &str) -> Self {
        self.name = Some(name.to_string());
        self
    }

    pub fn description(mut self, description: &str) -> Self {
        self.description = Some(description.to_string());
        self
    }

    /// A block at the end of the system prompt, after the base layer's. A sub-agent of the
    /// session does not have it.
    pub fn prompt(mut self, text: PromptText) -> Self {
        self.prompts.push(text);
        self
    }

    /// A tool of the scene's own, in its sessions alone. Its name is `^[A-Za-z][A-Za-z0-9_]{0,63}$`,
    /// not `ext__…` or `mcp__…`, and one session has at most 64.
    ///
    /// The link answers a call by the tool's name, without asking which scene the session is in:
    /// a tool by the same name in two layers an instance has — of one extension or of two — must
    /// be the same object (one `Arc`, shared), or the instance does not start.
    pub fn tool(mut self, tool: Arc<dyn Tool>) -> Self {
        self.tools.push(tool);
        self
    }

    /// Whether someone answers the session's questions and permission asks. An unattended
    /// scene's asks are refused before anyone sees them, so its own tools run unasked.
    pub fn interactive(mut self, on: bool) -> Self {
        self.interactive = Some(on);
        self
    }

    /// Whether a session needs a project. `true` refuses a session created with none; `false`
    /// cannot loosen a base of the engine's that needs one.
    pub fn requires_project(mut self, on: bool) -> Self {
        self.requires_project = Some(on);
        self
    }

    /// Whether a session is named by a model call after its first turn. Under replay that call
    /// was never recorded, so a scene that must not reach a model outside its turns says no.
    pub fn auto_name(mut self, on: bool) -> Self {
        self.auto_name = Some(on);
        self
    }

    /// The id this layer declares, `{base}` unreplaced.
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn base(&self) -> &SceneBase {
        &self.base
    }

    /// Everything the layer says.
    pub fn parts(&self) -> LayerParts<'_> {
        LayerParts {
            id: &self.id,
            base: &self.base,
            name: self.name.as_deref(),
            description: self.description.as_deref(),
            prompts: &self.prompts,
            tools: &self.tools,
            interactive: self.interactive,
            requires_project: self.requires_project,
            auto_name: self.auto_name,
        }
    }
}

/// What a [`SceneLayer`] says. `None` leaves it to the base.
#[non_exhaustive]
pub struct LayerParts<'a> {
    pub id: &'a str,
    pub base: &'a SceneBase,
    pub name: Option<&'a str>,
    pub description: Option<&'a str>,
    pub prompts: &'a [PromptText],
    pub tools: &'a [Arc<dyn Tool>],
    pub interactive: Option<bool>,
    pub requires_project: Option<bool>,
    pub auto_name: Option<bool>,
}

/// The scene a layer changes.
#[non_exhaustive]
#[derive(Clone)]
pub struct SceneBase(Base);

#[derive(Clone)]
enum Base {
    Builtin { id: String, args: BuiltinArgs },
    Named(String),
}

impl SceneBase {
    /// One of the engine's own scenes, as the engine has it: `harnix`, `code`, `mini`.
    pub fn builtin(id: &str) -> Self {
        Self(Base::Builtin { id: id.to_string(), args: BuiltinArgs::default() })
    }

    /// One of the engine's own scenes, made with `args`.
    pub fn builtin_with(id: &str, args: BuiltinArgs) -> Self {
        Self(Base::Builtin { id: id.to_string(), args })
    }

    /// Whatever the instance serves as `id` — a layer contributed before this one, or else one of
    /// the engine's own scenes.
    pub fn named(id: &str) -> Self {
        Self(Base::Named(id.to_string()))
    }

    pub fn id(&self) -> &str {
        match &self.0 {
            Base::Builtin { id, .. } | Base::Named(id) => id,
        }
    }

    pub fn kind(&self) -> BaseKind<'_> {
        match &self.0 {
            Base::Builtin { id, args } => BaseKind::Builtin { id, args },
            Base::Named(id) => BaseKind::Named(id),
        }
    }
}

/// Which scene a [`SceneBase`] is.
#[non_exhaustive]
pub enum BaseKind<'a> {
    Builtin { id: &'a str, args: &'a BuiltinArgs },
    Named(&'a str),
}

/// How a built-in scene is made.
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct BuiltinArgs {
    interaction: Option<Interaction>,
}

impl BuiltinArgs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether someone answers its sessions — what [`SceneLayer::interactive`] says when the
    /// layers over it do not; `code`'s prompt is chosen by it too.
    pub fn interaction(mut self, interaction: Interaction) -> Self {
        self.interaction = Some(interaction);
        self
    }

    pub fn chosen_interaction(&self) -> Option<Interaction> {
        self.interaction
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interaction {
    Interactive,
    Unattended,
}

/// A block a layer adds to the system prompt.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptText {
    text: String,
    cached: bool,
}

impl PromptText {
    /// A block the same on every call. From 0.4.4 the mark does nothing: a session's added text
    /// carries no cache mark of its own.
    pub fn cached(text: impl Into<String>) -> Self {
        Self { text: text.into(), cached: true }
    }

    pub fn plain(text: impl Into<String>) -> Self {
        Self { text: text.into(), cached: false }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_cached(&self) -> bool {
        self.cached
    }
}

/// One scene layer as a session is set up from it: the engine's own scene it runs on, and what it
/// adds — each part merged over the layers below it.
#[non_exhaustive]
#[derive(Clone)]
pub struct ExpandedScene {
    /// The id it is served as.
    pub id: String,
    /// The engine's own scene it runs on.
    pub base: String,
    /// May still say `{base}` when the base is the engine's own: the engine's name fills it.
    pub name: Option<String>,
    pub description: Option<String>,
    /// Base-most first.
    pub prompts: Vec<PromptText>,
    /// Base-most first.
    pub tools: Vec<Arc<dyn Tool>>,
    pub attended: Option<bool>,
    pub requires_project: Option<bool>,
    pub auto_name: Option<bool>,
}

impl ExpandedScene {
    /// What it adds at the end of the system prompt, its blocks one after another; `None` when it
    /// adds nothing.
    pub fn prompt_append(&self) -> Option<String> {
        let blocks: Vec<&str> = self.prompts.iter().map(PromptText::text).filter(|t| !t.trim().is_empty()).collect();
        (!blocks.is_empty()).then(|| blocks.join("\n\n"))
    }
}

impl std::fmt::Debug for ExpandedScene {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExpandedScene")
            .field("id", &self.id)
            .field("base", &self.base)
            .field("tools", &self.tools.iter().map(|t| t.spec().name().to_string()).collect::<Vec<_>>())
            .field("attended", &self.attended)
            .field("requires_project", &self.requires_project)
            .field("auto_name", &self.auto_name)
            .finish_non_exhaustive()
    }
}

/// An instance's scene layers, each expanded, and their tools by name.
#[derive(Clone, Default)]
pub struct SceneLayers(Arc<Inner>);

#[derive(Default)]
struct Inner {
    by_id: BTreeMap<String, ExpandedScene>,
    tools: BTreeMap<String, Arc<dyn Tool>>,
}

impl SceneLayers {
    /// `layers` expanded in the order given: a layer may build on one before it. Refused when a
    /// tool's name is not one the engine takes for a session's own, when one layer has more than
    /// 64 tools or two by one name, when two layers have different tools by one name, or when one
    /// tool is in an unattended layer and an attended one. A second layer served as one id
    /// replaces the first.
    pub fn new(layers: &[SceneLayer]) -> Result<Self, String> {
        let mut by_id: BTreeMap<String, ExpandedScene> = BTreeMap::new();
        for layer in layers {
            let expanded = expand(layer, &by_id)?;
            if by_id.contains_key(&expanded.id) {
                tracing::warn!(scene = %expanded.id, "a second scene layer served as this id replaces the first");
            }
            by_id.insert(expanded.id.clone(), expanded);
        }
        let mut tools: BTreeMap<String, Arc<dyn Tool>> = BTreeMap::new();
        let mut unattended: HashMap<String, bool> = HashMap::new();
        for scene in by_id.values() {
            for tool in &scene.tools {
                let name = tool.spec().name();
                match tools.get(name) {
                    Some(had) if !same(had, tool) => {
                        return Err(format!(
                            "two scene layers have different tools named `{name}`: a tool by one name is one object, shared by every layer that has it"
                        ))
                    }
                    Some(_) => {}
                    None => {
                        tools.insert(name.to_string(), tool.clone());
                    }
                }
                let here = scene.attended == Some(false);
                if *unattended.entry(name.to_string()).or_insert(here) != here {
                    return Err(format!("the tool `{name}` is in an unattended scene layer and in an attended one"));
                }
            }
        }
        Ok(Self(Arc::new(Inner { by_id, tools })))
    }

    pub fn get(&self, id: &str) -> Option<&ExpandedScene> {
        self.0.by_id.get(id)
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.0.by_id.keys().map(String::as_str)
    }

    /// The layers' tool by this name.
    pub fn tool(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.0.tools.get(name)
    }

    pub fn is_empty(&self) -> bool {
        self.0.by_id.is_empty()
    }
}

impl std::fmt::Debug for SceneLayers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.ids()).finish()
    }
}

fn same(a: &Arc<dyn Tool>, b: &Arc<dyn Tool>) -> bool {
    std::ptr::eq(Arc::as_ptr(a) as *const (), Arc::as_ptr(b) as *const ())
}

/// `layer` over its base: a layer expanded before it, or the engine's own scene.
fn expand(layer: &SceneLayer, before: &BTreeMap<String, ExpandedScene>) -> Result<ExpandedScene, String> {
    let p = layer.parts();
    let (below, base_id) = match layer.base().kind() {
        BaseKind::Builtin { id, args } => (builtin(id, args), id),
        BaseKind::Named(id) => match before.get(id) {
            Some(below) => (below.clone(), id),
            None => (builtin(id, &BuiltinArgs::default()), id),
        },
    };
    let layered = below.id != below.base;
    let fill = |text: &str, theirs: &Option<String>| -> String {
        match (layered, theirs) {
            (true, Some(theirs)) => text.replace("{base}", theirs),
            (true, None) => text.replace("{base}", base_id),
            // The engine's own: its name is the engine's to say.
            (false, _) => text.to_string(),
        }
    };
    let id = layer.id().replace("{base}", base_id);
    let mut prompts = below.prompts.clone();
    prompts.extend(p.prompts.iter().cloned());
    let mut tools = below.tools.clone();
    for tool in p.tools {
        let name = tool.spec().name();
        if !session_tool_name(name) {
            return Err(format!(
                "the scene `{id}` has a tool named `{name}`: a scene's tool is named ^[A-Za-z][A-Za-z0-9_]{{0,63}}$, and not ext__… or mcp__…"
            ));
        }
        if tools.iter().any(|t| t.spec().name() == name) {
            return Err(format!("the scene `{id}` has two tools named `{name}`"));
        }
        tools.push(tool.clone());
    }
    if tools.len() > MAX_TOOLS {
        return Err(format!("the scene `{id}` has {} tools of its own; a session has at most {MAX_TOOLS}", tools.len()));
    }
    Ok(ExpandedScene {
        name: p.name.map(|n| fill(n, &below.name)).or(below.name.clone()),
        description: p.description.map(|d| fill(d, &below.description)).or(below.description.clone()),
        id,
        base: below.base,
        prompts,
        tools,
        attended: p.interactive.or(below.attended),
        requires_project: p.requires_project.or(below.requires_project),
        auto_name: p.auto_name.or(below.auto_name),
    })
}

/// One of the engine's own scenes, adding nothing.
fn builtin(id: &str, args: &BuiltinArgs) -> ExpandedScene {
    ExpandedScene {
        id: id.to_string(),
        base: id.to_string(),
        name: None,
        description: None,
        prompts: Vec::new(),
        tools: Vec::new(),
        attended: args.chosen_interaction().map(|i| i == Interaction::Interactive),
        requires_project: None,
        auto_name: None,
    }
}

/// Whether the engine takes `name` for a session's own tool, apart from the connection's
/// (`ext__…`) and an MCP server's.
fn session_tool_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && name.len() <= 64
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.starts_with("ext__")
        && !name.starts_with("mcp__")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::{ToolCx, ToolError, ToolOutput, ToolSpec};
    use serde_json::{json, Value};

    struct Named(ToolSpec);

    #[async_trait::async_trait]
    impl Tool for Named {
        fn spec(&self) -> &ToolSpec {
            &self.0
        }
        async fn call(&self, _: ToolCx, _: Value) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput::text(""))
        }
    }

    fn tool(name: &str) -> Arc<dyn Tool> {
        Arc::new(Named(ToolSpec::new(name, "d", json!({"type": "object"}))))
    }

    fn names(scene: &ExpandedScene) -> Vec<&str> {
        scene.tools.iter().map(|t| t.spec().name()).collect()
    }

    #[test]
    fn a_layer_on_an_earlier_one_is_merged_over_it() {
        let (a, b) = (tool("a"), tool("b"));
        let flow = SceneLayer::new("flow-{base}", SceneBase::builtin("code"))
            .name("{base} (flow)")
            .prompt(PromptText::plain("flow"))
            .tool(a.clone())
            .interactive(true)
            .auto_name(false)
            .requires_project(true);
        let author = SceneLayer::new("author", SceneBase::named("flow-code"))
            .name("{base}, writing")
            .prompt(PromptText::cached("author"))
            .tool(b)
            .requires_project(false);
        let layers = SceneLayers::new(&[flow, author]).unwrap();
        let flow = layers.get("flow-code").unwrap();
        assert_eq!((flow.base.as_str(), flow.name.as_deref()), ("code", Some("{base} (flow)")), "the engine's name fills it");
        let author = layers.get("author").unwrap();
        assert_eq!(author.base, "code");
        assert_eq!(author.name.as_deref(), Some("{base} (flow), writing"));
        assert_eq!(author.prompt_append().as_deref(), Some("flow\n\nauthor"));
        assert_eq!(names(author), ["a", "b"]);
        assert_eq!((author.attended, author.auto_name, author.requires_project), (Some(true), Some(false), Some(false)));
        assert!(layers.tool("a").is_some_and(|t| same(t, &a)));
        assert_eq!(layers.ids().collect::<Vec<_>>(), ["author", "flow-code"]);
    }

    #[test]
    fn a_named_base_no_layer_has_is_the_engines() {
        let layers = SceneLayers::new(&[SceneLayer::new("x-{base}", SceneBase::named("mini")).name("{base}!")]).unwrap();
        let x = layers.get("x-mini").unwrap();
        assert_eq!((x.base.as_str(), x.name.as_deref(), x.attended), ("mini", Some("{base}!"), None));
        assert_eq!(x.prompt_append(), None);
    }

    #[test]
    fn whether_someone_answers_is_the_outermost_say_else_the_bases_interaction() {
        let unattended = || SceneBase::builtin_with("code", BuiltinArgs::new().interaction(Interaction::Unattended));
        let layers = SceneLayers::new(&[
            SceneLayer::new("quiet", unattended()),
            SceneLayer::new("loud", unattended()).interactive(true),
            SceneLayer::new("over", SceneBase::named("quiet")),
        ])
        .unwrap();
        assert_eq!(layers.get("quiet").unwrap().attended, Some(false));
        assert_eq!(layers.get("loud").unwrap().attended, Some(true));
        assert_eq!(layers.get("over").unwrap().attended, Some(false));
    }

    #[test]
    fn a_tool_by_one_name_is_one_object_across_layers() {
        let shared = tool("t");
        let ok = SceneLayers::new(&[
            SceneLayer::new("a", SceneBase::builtin("harnix")).tool(shared.clone()),
            SceneLayer::new("b", SceneBase::builtin("code")).tool(shared.clone()),
        ]);
        assert!(ok.is_ok());
        let two = SceneLayers::new(&[
            SceneLayer::new("a", SceneBase::builtin("harnix")).tool(shared.clone()),
            SceneLayer::new("b", SceneBase::builtin("code")).tool(tool("t")),
        ]);
        assert!(two.unwrap_err().contains("different tools named `t`"));
        let mixed = SceneLayers::new(&[
            SceneLayer::new("a", SceneBase::builtin("harnix")).tool(shared.clone()).interactive(false),
            SceneLayer::new("b", SceneBase::builtin("code")).tool(shared),
        ]);
        assert!(mixed.unwrap_err().contains("unattended"));
    }

    #[test]
    fn a_layers_own_tools_are_checked() {
        let one = |layer: SceneLayer| SceneLayers::new(&[layer]);
        let base = || SceneLayer::new("a", SceneBase::builtin("harnix"));
        assert!(one(base().tool(tool("t")).tool(tool("t"))).is_err(), "two by one name");
        for bad in ["ext__acme__t", "mcp__s__t", "1t", "a-b", ""] {
            assert!(one(base().tool(tool(bad))).is_err(), "{bad}");
        }
        let many = (0..65).fold(base(), |l, n| l.tool(tool(&format!("t{n}"))));
        assert!(one(many).unwrap_err().contains("at most 64"));
        let over = SceneLayers::new(&[base().tool(tool("t")), SceneLayer::new("b", SceneBase::named("a")).tool(tool("t"))]);
        assert!(over.is_err(), "the layer below has it already");
    }

    #[test]
    fn a_second_layer_by_one_id_replaces_the_first() {
        let layers = SceneLayers::new(&[
            SceneLayer::new("x", SceneBase::builtin("code")),
            SceneLayer::new("x", SceneBase::builtin("mini")),
        ])
        .unwrap();
        assert_eq!(layers.get("x").unwrap().base, "mini");
        assert!(SceneLayers::default().is_empty());
    }
}
