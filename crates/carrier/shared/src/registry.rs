//! What a running package is, and the roster of them: read by the tool provider (`tools.rs`), the
//! routes (`routes.rs`) and the interface entries (`frames.rs`) of either carrier. How a call
//! reaches a package is the carrier's, kept in its own registry around [`Roster`].

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use harnix_ext::engine::ToolsChanged;
use harnix_package::manifest::{Interface, Surface, UiEntry};
use harnix_package::signature::Trust;
use harnix_wasm_port::ToolDef;
use serde_json::Value;

use crate::sync::{lock, read, write};
use crate::tools::EventScope;

pub use harnix_package::tool_name;

/// How long one call into a package is given when its manifest has no component to say.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// One package that is running.
#[derive(Clone)]
pub struct Loaded {
    pub id: String,
    pub version: String,
    pub title: Option<String>,
    /// The unpacked package.
    pub root: PathBuf,
    /// It has a component: its tools are offered.
    pub component: bool,
    /// Which tool calls its `events` is offered ([`crate::tools::offered`]).
    pub events: EventScope,
    /// What its manifest declares it provides (in process, as its component agreed when it
    /// started; in a process of its own, what was consented to, whatever the process says).
    pub tools: Vec<ToolDef>,
    pub ui: Vec<UiEntry>,
    /// Its isolated interface: `[interface]` and the `[[surface]]`s its frames fill.
    pub interface: Interface,
    pub surfaces: Vec<Surface>,
    /// How far its package was trusted when it started: whether its interface runs in the page
    /// (its `[[ui]]` modules) or in frames (its surfaces) is decided from this.
    pub trust: Trust,
    /// Its `[[space]]` ids, in order: where a frame's new sessions land.
    pub backend_spaces: Vec<String>,
    /// `[inline]`, as the shell draws it (`packages.list` → `runtime.inline`).
    pub inline: Value,
    /// How long one of its calls is given (its component's `timeout_ms`). Only a call over a
    /// wire has a deadline made from it.
    pub timeout: Duration,
}

impl Loaded {
    /// The name a session sees one of its tools by ([`tool_name`]).
    pub fn tool_name(&self, tool: &str) -> String {
        tool_name(&self.id, tool)
    }
}

/// Where a running tool call's progress is told: `{app, tool, session_id, tool_use_id, text}`.
pub type OnProgress = Arc<dyn Fn(Value) + Send + Sync>;

/// Where one tool call's progress goes, by the call's id.
pub type Listener = Arc<dyn Fn(&str) + Send + Sync>;

/// The running packages, and who hears a tool call's progress.
#[derive(Default)]
pub struct Roster {
    loaded: RwLock<BTreeMap<String, Arc<Loaded>>>,
    progress: RwLock<Option<OnProgress>>,
    listeners: Mutex<HashMap<String, Listener>>,
    changed: ToolsChanged,
}

impl Roster {
    /// Fired when a package is inserted or removed: what the tool provider answers is then another set.
    pub fn changed(&self) -> ToolsChanged {
        self.changed.clone()
    }

    /// Tell whoever listens about a tool call's progress.
    pub fn progress(&self, value: Value) {
        let f = read(&self.progress).clone();
        if let Some(f) = f {
            f(value);
        }
    }

    pub fn on_progress(&self, f: OnProgress) {
        *write(&self.progress) = Some(f);
    }

    pub fn loaded(&self) -> Vec<Arc<Loaded>> {
        read(&self.loaded).values().cloned().collect()
    }

    pub fn get(&self, id: &str) -> Option<Arc<Loaded>> {
        read(&self.loaded).get(id).cloned()
    }

    pub fn insert(&self, loaded: Arc<Loaded>) {
        write(&self.loaded).insert(loaded.id.clone(), loaded);
        self.changed.notify();
    }

    pub fn remove(&self, id: &str) -> Option<Arc<Loaded>> {
        let removed = write(&self.loaded).remove(id);
        self.changed.notify();
        removed
    }

    /// Hear one tool call's progress, until [`Self::unlisten`].
    pub fn listen(&self, call_id: &str, listener: Listener) {
        lock(&self.listeners).insert(call_id.to_string(), listener);
    }

    pub fn unlisten(&self, call_id: &str) {
        lock(&self.listeners).remove(call_id);
    }

    /// A tool call's progress was told.
    pub fn told(&self, call_id: &str, text: &str) {
        let listener = lock(&self.listeners).get(call_id).cloned();
        if let Some(listener) = listener {
            listener(text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loaded(id: &str) -> Arc<Loaded> {
        Arc::new(Loaded {
            id: id.into(),
            version: "1.0.0".into(),
            title: None,
            root: PathBuf::new(),
            component: true,
            events: EventScope::Own,
            tools: Vec::new(),
            ui: Vec::new(),
            interface: Interface::default(),
            surfaces: Vec::new(),
            trust: Trust::parse("unsigned"),
            backend_spaces: Vec::new(),
            inline: Value::Null,
            timeout: DEFAULT_TIMEOUT,
        })
    }

    #[tokio::test]
    async fn the_roster_says_when_a_package_starts_and_stops() {
        let roster = Roster::default();
        let mut heard = roster.changed().watch();
        roster.insert(loaded("acme.a"));
        assert!(heard.changed().await);
        roster.remove("acme.a");
        assert!(heard.changed().await);
    }
}
