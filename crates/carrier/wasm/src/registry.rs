//! What is running: the packages whose tools are on offer and whose interface is served, read by
//! the tool provider (`tools.rs`), the routes (`routes.rs`) and the interface entries
//! (`frames.rs`). A package is in it exactly while it runs.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};

use crate::sync::{lock, read, write};
use harnix_package::manifest::{Interface, Surface, UiEntry};
use harnix_package::signature::Trust;
use harnix_wasm_port::{Control, Offer, ToolDef};
use serde_json::Value;

pub use harnix_package::tool_name;

/// One package that is running.
pub struct Loaded {
    pub id: String,
    pub version: String,
    pub title: Option<String>,
    /// The unpacked package.
    pub root: PathBuf,
    /// It has a component: its tools are offered, and its events run around every tool call.
    pub component: bool,
    /// What its manifest declares it provides, as its component agreed when it started.
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
pub(crate) type Listener = Arc<dyn Fn(&str) + Send + Sync>;

/// What is running, shared with the tool provider and the routes.
#[derive(Default)]
pub struct Registry {
    apps: RwLock<BTreeMap<String, Arc<Loaded>>>,
    /// What the host offered, by package: read when it is loaded.
    offered: RwLock<BTreeMap<String, Offer>>,
    progress: RwLock<Option<OnProgress>>,
    listeners: Mutex<HashMap<String, Listener>>,
    control: OnceLock<Weak<dyn Control>>,
    /// Each package's `[capabilities] net`, from the moment it starts: the hosts its HTTP may
    /// reach, decided here.
    nets: RwLock<BTreeMap<String, Vec<String>>>,
}

impl Registry {
    /// Tell whoever listens about a tool call's progress.
    pub fn progress(&self, value: Value) {
        let f = read(&self.progress).clone();
        if let Some(f) = f {
            f(value);
        }
    }

    pub(crate) fn on_progress(&self, f: OnProgress) {
        *write(&self.progress) = Some(f);
    }

    pub fn loaded(&self) -> Vec<Arc<Loaded>> {
        read(&self.apps).values().cloned().collect()
    }

    pub fn get(&self, id: &str) -> Option<Arc<Loaded>> {
        read(&self.apps).get(id).cloned()
    }

    pub(crate) fn insert(&self, loaded: Arc<Loaded>) {
        write(&self.apps).insert(loaded.id.clone(), loaded);
    }

    pub(crate) fn remove(&self, id: &str) -> Option<Arc<Loaded>> {
        write(&self.apps).remove(id)
    }

    pub(crate) fn set_net(&self, id: &str, net: Option<Vec<String>>) {
        match net {
            Some(net) => write(&self.nets).insert(id.to_string(), net),
            None => write(&self.nets).remove(id),
        };
    }

    pub(crate) fn net(&self, id: &str) -> Option<Vec<String>> {
        read(&self.nets).get(id).cloned()
    }

    /// The host that runs what is listed here: what tool calls and hooks go to.
    pub fn control(&self) -> Option<Arc<dyn Control>> {
        self.control.get().and_then(Weak::upgrade)
    }

    pub(crate) fn lend(&self, control: Weak<dyn Control>) {
        let _ = self.control.set(control);
    }

    /// The host offered a package's tools (or, with none, took them back).
    pub(crate) fn offer(&self, offer: Offer) {
        let mut offered = write(&self.offered);
        if offer.component {
            offered.insert(offer.app.clone(), offer);
        } else {
            offered.remove(&offer.app);
        }
    }

    pub(crate) fn offered(&self, id: &str) -> Option<Offer> {
        read(&self.offered).get(id).cloned()
    }

    /// Hear one tool call's progress, until [`Self::unlisten`].
    pub(crate) fn listen(&self, call_id: &str, listener: Listener) {
        lock(&self.listeners).insert(call_id.to_string(), listener);
    }

    pub(crate) fn unlisten(&self, call_id: &str) {
        lock(&self.listeners).remove(call_id);
    }

    /// The host told a tool call's progress.
    pub(crate) fn told(&self, call_id: &str, text: &str) {
        let listener = lock(&self.listeners).get(call_id).cloned();
        if let Some(listener) = listener {
            listener(text);
        }
    }
}
