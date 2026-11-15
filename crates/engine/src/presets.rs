//! What a scene layer still means while the engine takes no scenes of Harnix's: the built-in scene
//! it is served on, and what its sessions are created with.
//!
//! The engine serves only its own scenes. A layer's id is therefore mapped to the scene it is built
//! on, wherever it is named — the profile's scenes, a session's creation, a scene's activation — and
//! what the layer adds that a session's options can say is said there: its prompt, appended, and
//! whether the session is named. Its tools, its tool list and its session facts are not carried.
//!
//! Every session is attended: someone answers its asks — the person at the interface, or an
//! extension that answers them.

use std::collections::HashMap;
use std::sync::Arc;

use harnix_engine_api::{BaseKind, SceneLayer};
use serde_json::{json, Map, Value};

use crate::translate;

/// The scene layers an engine serves, by the id each is served as.
#[derive(Debug, Default)]
pub struct Presets {
    /// The scene a session that names none is created in, as the profile names it.
    default: String,
    layers: HashMap<String, Preset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Preset {
    /// The engine's own scene the layer is served on.
    base: String,
    /// What it adds after the base's system prompt.
    prompt: Option<String>,
    auto_name: Option<bool>,
}

impl Presets {
    /// `layers` resolved over their bases, in order: a layer may build on one before it. `default`
    /// is the profile's scene.
    pub fn new(layers: &[SceneLayer], default: &str) -> Result<Arc<Self>, String> {
        let mut resolved: HashMap<String, Preset> = HashMap::new();
        for layer in layers {
            let parts = layer.parts();
            let (inherited, base_id) = match layer.base().kind() {
                BaseKind::Builtin { id, args } => {
                    if args.chosen_interaction().is_some() {
                        tracing::debug!(scene = layer.id(), "the base's interaction is the session's `attended` now; left out");
                    }
                    (Preset { base: id.to_string(), prompt: None, auto_name: None }, id.to_string())
                }
                // A layer before this one, or else one of the engine's own scenes: the engine says
                // which it has when a session is created in it.
                BaseKind::Named(id) => match resolved.get(id) {
                    Some(before) => (before.clone(), id.to_string()),
                    None => (Preset { base: id.to_string(), prompt: None, auto_name: None }, id.to_string()),
                },
                _ => return Err(format!("the scene `{}` has a base this engine does not know", layer.id())),
            };
            let served = layer.id().replace("{base}", &base_id);
            if resolved.contains_key(&served) {
                return Err(format!("two scene layers are served as `{served}`"));
            }
            let own: Vec<&str> = parts.prompts.iter().map(|p| p.text()).filter(|t| !t.trim().is_empty()).collect();
            let prompt = match (inherited.prompt, own.is_empty()) {
                (before, true) => before,
                (Some(before), false) => Some(format!("{before}\n\n{}", own.join("\n\n"))),
                (None, false) => Some(own.join("\n\n")),
            };
            let auto_name = if parts.auto_name == Some(false) || parts.name_prompt == Some(None) {
                Some(false)
            } else {
                parts.auto_name.or(inherited.auto_name)
            };
            resolved.insert(served, Preset { base: inherited.base, prompt, auto_name });
        }
        Ok(Arc::new(Self { default: default.to_string(), layers: resolved }))
    }

    /// No layers: every session is attended, and nothing else changes.
    pub fn none() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// The engine's own scene `id` is served on: `id` itself when it names no layer.
    pub fn base_of(&self, id: &str) -> String {
        self.layers.get(id).map_or_else(|| id.to_string(), |p| p.base.clone())
    }

    /// A profile's default scene and the others, as the engine is started with them: each layer
    /// as its base, and the others once each without the default.
    pub fn engine_scenes(&self, scene: &str, scenes: &[String]) -> (String, Vec<String>) {
        let scene = self.base_of(scene);
        let mut others: Vec<String> = Vec::new();
        for extra in scenes {
            let base = self.base_of(extra);
            if base != scene && !others.contains(&base) {
                others.push(base);
            }
        }
        (scene, others)
    }

    /// A call as the engine is to see it: a session created or resumed attended, with what its
    /// layer adds; a layer's id in place of the scene it is served on.
    pub(crate) fn prepare(&self, method: &str, mut params: Value) -> Value {
        match method {
            translate::SESSION_CREATE => {
                let named = params.get("scene").and_then(Value::as_str).map(str::to_string);
                let scene = named.or_else(|| (!self.default.is_empty()).then(|| self.default.clone()));
                let preset = scene.and_then(|s| self.layers.get(&s));
                let obj = object(&mut params);
                if let Some(preset) = preset {
                    obj.insert("scene".into(), json!(preset.base));
                }
                let options = options(obj);
                options.insert("attended".into(), json!(true));
                if let Some(preset) = preset {
                    if preset.auto_name == Some(false) {
                        options.insert("auto_name".into(), json!(false));
                    }
                    if let Some(prompt) = &preset.prompt {
                        options.insert("prompt".into(), json!({"append": prompt}));
                    }
                }
                params
            }
            // A session created before it had options of its own takes these; one that has them
            // keeps its own.
            translate::SESSION_RESUME => {
                options(object(&mut params)).insert("attended".into(), json!(true));
                params
            }
            translate::SCENE_ACTIVATE => {
                let base = params.get("scene").and_then(Value::as_str).and_then(|s| self.layers.get(s)).map(|p| p.base.clone());
                if let Some(base) = base {
                    params["scene"] = json!(base);
                }
                params
            }
            _ => params,
        }
    }
}

/// The scenes an engine serves, as the profile names them: its default and the others, once each
/// and sorted. The same for one built here and one spawned with the same profile.
pub fn active_scenes(scene: &str, scenes: &[String]) -> Vec<String> {
    let mut all: Vec<String> = std::iter::once(scene.to_string()).chain(scenes.iter().cloned()).collect();
    all.sort();
    all.dedup();
    all
}

fn object(params: &mut Value) -> &mut Map<String, Value> {
    if !params.is_object() {
        *params = json!({});
    }
    params.as_object_mut().expect("an object just now")
}

fn options(params: &mut Map<String, Value>) -> &mut Map<String, Value> {
    let options = params.entry("options").or_insert_with(|| json!({}));
    if !options.is_object() {
        *options = json!({});
    }
    options.as_object_mut().expect("an object just now")
}

#[cfg(test)]
mod tests {
    use super::*;
    use harnix_engine_api::{PromptText, SceneBase};

    fn layers() -> Arc<Presets> {
        let flow = SceneLayer::new("flow-{base}", SceneBase::builtin("code")).auto_name(false).prompt(PromptText::plain("flow"));
        let author = SceneLayer::new("author", SceneBase::named("flow-code")).prompt(PromptText::cached("author"));
        let quiet = SceneLayer::new("quiet", SceneBase::builtin("mini")).name_prompt(None);
        Presets::new(&[flow, author, quiet], "author").unwrap()
    }

    #[test]
    fn a_layer_is_served_on_its_base_and_a_layer_on_a_layer_inherits_it() {
        let presets = layers();
        assert_eq!(presets.base_of("flow-code"), "code");
        assert_eq!(presets.base_of("author"), "code");
        assert_eq!(presets.base_of("quiet"), "mini");
        assert_eq!(presets.base_of("harnix"), "harnix", "a scene that is no layer is itself");
        let author = &presets.layers["author"];
        assert_eq!(author.prompt.as_deref(), Some("flow\n\nauthor"));
        assert_eq!(author.auto_name, Some(false));
        assert_eq!(presets.layers["quiet"].auto_name, Some(false), "no naming prompt is no naming");
    }

    #[test]
    fn two_layers_served_as_one_id_are_refused() {
        let a = SceneLayer::new("x", SceneBase::builtin("code"));
        let b = SceneLayer::new("x", SceneBase::builtin("mini"));
        assert!(Presets::new(&[a, b], "harnix").is_err());
    }

    #[test]
    fn a_session_is_created_on_the_base_with_what_its_layer_adds() {
        let presets = layers();
        let created = presets.prepare(translate::SESSION_CREATE, json!({"scene": "flow-code", "options": {"model": "m"}}));
        assert_eq!(created["scene"], "code");
        assert_eq!(
            created["options"],
            json!({"model": "m", "attended": true, "auto_name": false, "prompt": {"append": "flow"}})
        );
        let defaulted = presets.prepare(translate::SESSION_CREATE, json!({}));
        assert_eq!(defaulted["scene"], "code", "the default is a layer");
        assert_eq!(defaulted["options"]["prompt"]["append"], "flow\n\nauthor");
        let plain = Presets::none().prepare(translate::SESSION_CREATE, json!({"scene": "code"}));
        assert_eq!(plain, json!({"scene": "code", "options": {"attended": true}}));
        assert_eq!(Presets::none().prepare(translate::SESSION_CREATE, Value::Null), json!({"options": {"attended": true}}));
    }

    #[test]
    fn a_resumed_session_is_attended_and_an_activated_layer_is_its_base() {
        let presets = layers();
        let resumed = presets.prepare(translate::SESSION_RESUME, json!({"session_id": "s", "scene": "code"}));
        assert_eq!(resumed, json!({"session_id": "s", "scene": "code", "options": {"attended": true}}));
        let activated = presets.prepare(translate::SCENE_ACTIVATE, json!({"scene": "flow-code"}));
        assert_eq!(activated["scene"], "code");
        assert_eq!(presets.prepare(translate::SCENE_ACTIVATE, json!({"scene": "mini"}))["scene"], "mini");
        let other = json!({"session_id": "s"});
        assert_eq!(presets.prepare(translate::SESSION_GET, other.clone()), other);
    }

    #[test]
    fn the_active_scenes_are_the_default_and_the_others_once_each() {
        let others = ["mini".to_string(), "code".to_string(), "mini".to_string()];
        assert_eq!(active_scenes("harnix", &others), ["code", "harnix", "mini"]);
        assert_eq!(active_scenes("code", &["code".to_string()]), ["code"]);
        assert_eq!(active_scenes("harnix", &[]), ["harnix"]);
    }
}
