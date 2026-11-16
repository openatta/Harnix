//! `harnix.ui`: what the page draws for whom — each extension's interface entry (what it declared,
//! and whether it is enabled), and the interface modules and frames the extensions publish (the
//! engine packages' are `plugins`', the packages' the component that runs them). A method, and a topic of the same value pushed when
//! any of it changes: the shell activates a UI package for an extension that comes up, and takes
//! back everything one that went down registered, without a reload.
//!
//! The assembly's to answer: it is the one layer that knows every extension and every source of
//! modules.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

use harnix_contract::sync::lock;
use harnix_contract::wire::{self, ExtensionEntry, UiView};
use harnix_kernel::{Rpc, TopicChanged, TopicSource};
use serde_json::{json, Value};

use crate::gate::Diagnostics;

/// The topic, and the method of the same value.
pub(crate) const UI: &str = "harnix.ui";

/// The interface entries a page is told of: what each extension publishes while it runs. Sources
/// that change apart, told as one list. Replaced at assembly and again after an install: a package
/// that needed a reload to appear is a package nobody will believe installed.
pub(crate) struct UiModules {
    lists: Mutex<BTreeMap<String, Vec<Value>>>,
    /// Every extension and what it declared of its interface: set once registration is done.
    diagnostics: Arc<OnceLock<Arc<Diagnostics>>>,
    /// Who fans `harnix.ui` out: told whenever any of it may have changed.
    changed: OnceLock<Arc<dyn TopicChanged>>,
}

impl UiModules {
    pub fn new(diagnostics: Arc<OnceLock<Arc<Diagnostics>>>) -> Self {
        Self {
            lists: Mutex::new(Default::default()),
            diagnostics,
            changed: OnceLock::new(),
        }
    }

    /// From now on a change is pushed to whoever watches `harnix.ui`.
    pub fn notify(&self, to: Arc<dyn TopicChanged>) {
        let _ = self.changed.set(to);
    }

    /// What one extension publishes, in place of what it published before.
    pub fn set_extension(&self, ext: &str, modules: Vec<Value>) {
        {
            let mut lists = lock(&self.lists);
            match modules.is_empty() {
                true => lists.remove(ext),
                false => lists.insert(ext.to_string(), modules),
            };
        }
        self.changed();
    }

    /// Something besides the modules changed: an extension enabled, disabled, or failed to start.
    pub fn changed(&self) {
        if let Some(to) = self.changed.get() {
            to.changed(UI, json!({}));
        }
    }

    /// `hello.extensions`: every extension, enabled or not — what the shell activates its
    /// compiled-in UI packages by, before the first draw. Of its interface only `bundle`, the UI
    /// package it is drawn by when that is not its own id.
    pub fn extensions(&self) -> Vec<ExtensionEntry> {
        self.entries()
            .into_iter()
            .map(|mut e| {
                e.bundle = e.ui.take().and_then(|ui| ui.get("bundle").and_then(Value::as_str).map(str::to_string));
                e
            })
            .collect()
    }

    /// The whole value: each extension's entry with its interface, and the modules.
    pub fn view(&self) -> Value {
        wire::params(&UiView { extensions: self.entries(), contributions: self.modules() })
    }

    fn entries(&self) -> Vec<ExtensionEntry> {
        self.diagnostics.get().map(|d| d.ui_entries()).unwrap_or_default()
    }

    fn modules(&self) -> Vec<Value> {
        let lists = lock(&self.lists);
        lists.values().flatten().cloned().collect()
    }
}

/// `harnix.ui` as a topic: computed afresh whenever the modules say it changed.
pub(crate) struct UiTopic(pub Arc<UiModules>);

#[async_trait::async_trait]
impl TopicSource for UiTopic {
    async fn compute(&self, _params: &Value) -> Rpc<Value> {
        Ok(self.0.view())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[derive(Default)]
    struct Counted(AtomicUsize);

    impl TopicChanged for Counted {
        fn changed(&self, topic: &str, _params: Value) {
            assert_eq!(topic, UI);
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Every source that changes is pushed, and the modules are told as one list.
    #[test]
    fn each_change_is_pushed_and_the_modules_are_one_list() {
        let ui = UiModules::new(Arc::default());
        ui.set_extension("packages", vec![json!("before")]);
        let counted = Arc::new(Counted::default());
        ui.notify(counted.clone());
        ui.set_extension("packages", vec![json!("app")]);
        ui.set_extension("plugins", vec![json!("pkg")]);
        ui.set_extension("acme", vec![json!("ext")]);
        ui.set_extension("acme", Vec::new());
        ui.changed();
        assert_eq!(counted.0.load(Ordering::SeqCst), 5);
        assert_eq!(ui.view(), json!({"extensions": [], "contributions": ["app", "pkg"]}));
        assert!(ui.extensions().is_empty());
    }
}
