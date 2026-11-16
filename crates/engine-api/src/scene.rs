//! A scene as a layer over another: only what changes is said, the rest is the base's.
//!
//! A layer names its base ([`SceneBase`]) and what it changes — its id and name, a block added at
//! the end of the system prompt, tools of its own, whether someone answers, whether a project is
//! needed, whether the session is named by a model call. Whatever it does not say is the base's.
//!
//! The engine serves only its own scenes. A layer is not built into it: the hub expands the one a
//! session is created in into the engine's scene it runs on and what the session is created
//! with (`harnix-engine-link`), and the session's own tools are answered on the link that created
//! it. A layer's own tools are named `^[A-Za-z][A-Za-z0-9_-]{0,63}$`, not `ext__…` or `mcp__…`, at
//! most 64 to a session, and a tool by one name is one object shared by every layer that has it:
//! the expansion refuses what breaks that.
//!
//! An id, a name or a description may say `{base}`, which is replaced by the base's own:
//! `flow-{base}`, `"{base} (flow)"`.

use std::sync::Arc;

use crate::tool::Tool;

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

    /// A tool of the scene's own, in its sessions alone. Its name is `^[A-Za-z][A-Za-z0-9_-]{0,63}$`,
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
