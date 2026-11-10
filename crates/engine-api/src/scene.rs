//! A scene as a layer over another: only what changes is said, the rest is the base's.
//!
//! A layer names its base ([`SceneBase`]) and what it changes — its id and name, a system block
//! added after the base's, tools added to the scene and to its tool list, names let into a tool
//! list, a tool taken off the disallowed list, whether someone answers, whether a project is
//! needed, whether the session is named by a model call. Whatever it does not say is answered by
//! the base, method by method.
//!
//! An id, a name or a description may say `{base}`, which is replaced by the base's own:
//! `structured-{base}`, `"{base} (flow)"`.

use std::any::Any;
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
    admitted: Vec<String>,
    undisallowed: Vec<String>,
    interactive: Option<bool>,
    requires_project: Option<bool>,
    auto_name: Option<bool>,
    name_prompt: Option<Option<String>>,
    session_context: Option<SessionContext>,
}

type SessionContext = Arc<dyn Fn(&SceneFacts) -> String + Send + Sync>;

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
            admitted: Vec::new(),
            undisallowed: Vec::new(),
            interactive: None,
            requires_project: None,
            auto_name: None,
            name_prompt: None,
            session_context: None,
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

    /// A system block after the base's.
    pub fn prompt(mut self, text: PromptText) -> Self {
        self.prompts.push(text);
        self
    }

    /// A tool of the scene's own: registered in its sessions, and let through its tool list.
    pub fn tool(mut self, tool: Arc<dyn Tool>) -> Self {
        self.tools.push(tool);
        self
    }

    /// Names let through the base's tool list (`prefix*` or exact). A base that lists no tools
    /// lets every tool through already, and is left so.
    pub fn admit(mut self, pattern: &str) -> Self {
        if !self.admitted.iter().any(|p| p == pattern) {
            self.admitted.push(pattern.to_string());
        }
        self
    }

    /// Takes `tool` off the base's disallowed list, and lets it through.
    pub fn undisallow(mut self, tool: &str) -> Self {
        self.undisallowed.push(tool.to_string());
        self
    }

    /// Whether someone answers the session's questions and permission asks. An unattended
    /// scene's asks are refused before anyone sees them.
    pub fn interactive(mut self, on: bool) -> Self {
        self.interactive = Some(on);
        self
    }

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

    /// The prompt that names a session (`{message}` is its first message); `None`: none.
    pub fn name_prompt(mut self, prompt: Option<&str>) -> Self {
        self.name_prompt = Some(prompt.map(str::to_string));
        self
    }

    /// Text added to the facts the base sends once at the start of a session.
    pub fn session_context(mut self, f: impl Fn(&SceneFacts) -> String + Send + Sync + 'static) -> Self {
        self.session_context = Some(Arc::new(f));
        self
    }

    /// The id this layer declares, `{base}` unreplaced.
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn base(&self) -> &SceneBase {
        &self.base
    }

    /// Everything the layer says, for the engine to build it from.
    pub fn parts(&self) -> LayerParts<'_> {
        LayerParts {
            id: &self.id,
            base: &self.base,
            name: self.name.as_deref(),
            description: self.description.as_deref(),
            prompts: &self.prompts,
            tools: &self.tools,
            admitted: &self.admitted,
            undisallowed: &self.undisallowed,
            interactive: self.interactive,
            requires_project: self.requires_project,
            auto_name: self.auto_name,
            name_prompt: self.name_prompt.as_ref().map(Option::as_deref),
            session_context: self.session_context.as_ref(),
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
    pub admitted: &'a [String],
    pub undisallowed: &'a [String],
    pub interactive: Option<bool>,
    pub requires_project: Option<bool>,
    pub auto_name: Option<bool>,
    /// `Some(None)`: no naming prompt, whatever the base's.
    pub name_prompt: Option<Option<&'a str>>,
    pub session_context: Option<&'a Arc<dyn Fn(&SceneFacts) -> String + Send + Sync>>,
}

/// The scene a layer changes.
#[non_exhaustive]
#[derive(Clone)]
pub struct SceneBase(Base);

#[derive(Clone)]
enum Base {
    Builtin { id: String, args: BuiltinArgs },
    Named(String),
    Foreign { id: String, scene: Arc<dyn Any + Send + Sync> },
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

    /// Whatever the instance serves as `id` when the layer is built — a scene contributed before
    /// this one, or the engine's own. One it does not have stops the instance from starting.
    pub fn named(id: &str) -> Self {
        Self(Base::Named(id.to_string()))
    }

    /// A scene in the engine's own terms, which only the engine adapter can read: what
    /// `harnix::agix`'s `SceneBase::raw` makes (not stable). A value the adapter does not
    /// recognise stops the instance from starting.
    pub fn foreign(id: &str, scene: Arc<dyn Any + Send + Sync>) -> Self {
        Self(Base::Foreign { id: id.to_string(), scene })
    }

    pub fn id(&self) -> &str {
        match &self.0 {
            Base::Builtin { id, .. } | Base::Named(id) | Base::Foreign { id, .. } => id,
        }
    }

    pub fn kind(&self) -> BaseKind<'_> {
        match &self.0 {
            Base::Builtin { id, args } => BaseKind::Builtin { id, args },
            Base::Named(id) => BaseKind::Named(id),
            Base::Foreign { scene, .. } => BaseKind::Foreign(scene),
        }
    }
}

/// Which scene a [`SceneBase`] is.
#[non_exhaustive]
pub enum BaseKind<'a> {
    Builtin { id: &'a str, args: &'a BuiltinArgs },
    Named(&'a str),
    Foreign(&'a Arc<dyn Any + Send + Sync>),
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

    /// `code`'s: whether its prompt counts on someone being there.
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

/// A system block a layer adds.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptText {
    text: String,
    cached: bool,
}

impl PromptText {
    /// A block the provider may cache: the same on every call.
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

/// What a session is started with, for [`SceneLayer::session_context`].
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct SceneFacts {
    cwd: String,
    os: String,
    shell: String,
    date: String,
    model: String,
}

impl SceneFacts {
    pub fn new(cwd: &str, os: &str, shell: &str, date: &str, model: &str) -> Self {
        Self {
            cwd: cwd.to_string(),
            os: os.to_string(),
            shell: shell.to_string(),
            date: date.to_string(),
            model: model.to_string(),
        }
    }

    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    pub fn os(&self) -> &str {
        &self.os
    }

    pub fn shell(&self) -> &str {
        &self.shell
    }

    pub fn date(&self) -> &str {
        &self.date
    }

    pub fn model(&self) -> &str {
        &self.model
    }
}
