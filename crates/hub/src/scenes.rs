//! Scene layers, as the hub sets a session up from one.
//!
//! The engine serves only its own scenes. An instance's scene layers come with its link
//! ([`EngineControl::scene_layers`](harnix_engine_link::EngineControl::scene_layers)): a session
//! created in one is created in the engine's scene the layer runs on, set up with what the layer
//! adds, and the ledger records which layer it was (`spaces.rs`). The engine reports the session's
//! scene as its own; the hub adds `scene_layer` wherever it describes the session.
//!
//! A client lists, describes and activates a layer as it would one of the engine's scenes: a
//! layer is described as its base is, with what it says over it, and activating one activates its
//! base.

use harnix_contract::RpcError;
use harnix_engine_link::ops::codes;
use harnix_engine_link::{ExpandedScene, SceneLayers};
use serde_json::{json, Map, Value};

/// The layer a session is created in, when `params` name one; refused when it needs a project
/// and the caller said none.
pub(crate) fn layer_for(layers: &SceneLayers, params: &Value) -> Result<Option<ExpandedScene>, RpcError> {
    let Some(layer) = params.get("scene").and_then(Value::as_str).and_then(|id| layers.get(id)) else {
        return Ok(None);
    };
    // An omitted project is the engine's default project; only one given as none has none.
    let none = params.get("project_root").is_some_and(Value::is_null);
    if layer.requires_project == Some(true) && none {
        return Err(RpcError::with_data(
            codes::PROJECT_REQUIRED,
            format!("the scene `{}` needs a project: give `project_root`", layer.id),
            json!({"scene": layer.id}),
        ));
    }
    Ok(Some(layer.clone()))
}

/// A call about one scene, as the engine is to read it: a layer named by its base.
pub(crate) fn about_one(layers: &SceneLayers, mut params: Value) -> (Value, Option<ExpandedScene>) {
    let layer = params.get("scene").and_then(Value::as_str).and_then(|id| layers.get(id)).cloned();
    if let Some(layer) = &layer {
        params["scene"] = json!(layer.base);
    }
    (params, layer)
}

/// The engine's answer about the base, as the layer's.
pub(crate) fn one_described(layer: &ExpandedScene, mut answer: Value) -> Value {
    let Some(obj) = answer.as_object_mut() else { return answer };
    let base = (obj.get("name").cloned(), obj.get("description").cloned());
    over_base(layer, obj, base);
    if layer.requires_project == Some(true) {
        if let Some(Value::Object(caps)) = obj.get_mut("capabilities") {
            caps.insert("requires_project".into(), json!(true));
        }
    }
    answer
}

/// The engine's scene list with a row for each layer whose base is in it: the base's row, as the
/// layer.
pub(crate) fn listed(layers: &SceneLayers, mut answer: Value) -> Value {
    let Some(Value::Array(rows)) = answer.get_mut("scenes") else { return answer };
    let mut added = Vec::new();
    for id in layers.ids() {
        let Some(layer) = layers.get(id) else { continue };
        let Some(Value::Object(base)) = rows.iter().find(|r| r.get("scene").and_then(Value::as_str) == Some(layer.base.as_str())) else {
            continue;
        };
        let mut row = base.clone();
        // How many sessions run in the base is not how many run in the layer, which the engine
        // does not count.
        row.insert("sessions".into(), Value::Null);
        let names = (row.get("name").cloned(), row.get("description").cloned());
        over_base(layer, &mut row, names);
        added.push(Value::Object(row));
    }
    rows.extend(added);
    answer
}

/// `obj`, a description of the layer's base, as one of the layer: its id, its name and
/// description (`{base}` filled from the base's), whether it needs a project (a layer can make a
/// base need one, not stop it needing one), which base it is on.
fn over_base(layer: &ExpandedScene, obj: &mut Map<String, Value>, (name, description): (Option<Value>, Option<Value>)) {
    let fill = |own: &Option<String>, theirs: Option<Value>| -> Option<Value> {
        let theirs = theirs.and_then(|v| v.as_str().map(str::to_string));
        match (own, theirs) {
            (Some(own), theirs) => Some(json!(own.replace("{base}", theirs.as_deref().unwrap_or(&layer.base)))),
            (None, theirs) => theirs.map(Value::from),
        }
    };
    obj.insert("scene".into(), json!(layer.id));
    for (key, own, theirs) in [("name", &layer.name, name), ("description", &layer.description, description)] {
        if let Some(value) = fill(own, theirs) {
            obj.insert(key.into(), value);
        }
    }
    if layer.requires_project == Some(true) && obj.contains_key("requires_project") {
        obj.insert("requires_project".into(), json!(true));
    }
    obj.insert("base".into(), json!(layer.base));
    obj.insert("scene_layer".into(), json!(true));
}

#[cfg(test)]
mod tests {
    use super::*;
    use harnix_engine_api::{SceneBase, SceneLayer};

    fn layers() -> SceneLayers {
        SceneLayers::new(&[
            SceneLayer::new("flow-{base}", SceneBase::builtin("code")).name("{base} (flow)").requires_project(false),
            SceneLayer::new("lost", SceneBase::builtin("nowhere")),
            SceneLayer::new("strict", SceneBase::builtin("mini")).requires_project(true),
        ])
        .unwrap()
    }

    #[test]
    fn a_layer_is_listed_as_its_base_is_with_what_it_says() {
        let answer = json!({"scenes": [
            {"scene": "code", "name": "Agix Code", "description": "codes", "active": true, "sessions": 2, "requires_project": true},
            {"scene": "mini", "name": "Mini", "active": false, "requires_project": false},
        ]});
        let rows = listed(&layers(), answer)["scenes"].as_array().cloned().unwrap();
        assert_eq!(rows.len(), 4, "a layer whose base is not listed is not: {rows:?}");
        assert_eq!(
            rows[2],
            json!({"scene": "flow-code", "name": "Agix Code (flow)", "description": "codes", "active": true, "sessions": null,
                   "requires_project": true, "base": "code", "scene_layer": true}),
            "a layer does not stop its base needing a project"
        );
        assert_eq!((&rows[3]["scene"], &rows[3]["requires_project"], &rows[3]["description"]), (&json!("strict"), &json!(true), &Value::Null));
    }

    #[test]
    fn a_layer_is_described_and_activated_as_its_base() {
        let (sent, layer) = about_one(&layers(), json!({"scene": "flow-code", "project_root": null}));
        assert_eq!(sent, json!({"scene": "code", "project_root": null}));
        let described = one_described(
            &layer.unwrap(),
            json!({"scene": "code", "name": "Agix Code", "capabilities": {"requires_project": true}}),
        );
        assert_eq!((&described["scene"], &described["base"]), (&json!("flow-code"), &json!("code")));
        assert_eq!(described["capabilities"]["requires_project"], true);
        let (sent, layer) = about_one(&layers(), json!({"scene": "mini"}));
        assert_eq!((sent, layer.is_none()), (json!({"scene": "mini"}), true));
    }

    #[test]
    fn a_layer_that_needs_a_project_refuses_a_session_given_none() {
        let layers = SceneLayers::new(&[SceneLayer::new("p", SceneBase::builtin("harnix")).requires_project(true)]).unwrap();
        let refused = layer_for(&layers, &json!({"scene": "p", "project_root": null})).unwrap_err();
        assert_eq!((refused.code, refused.data.clone()), (codes::PROJECT_REQUIRED, Some(json!({"scene": "p"}))));
        assert!(layer_for(&layers, &json!({"scene": "p"})).unwrap().is_some(), "an omitted project is the engine's default");
        assert!(layer_for(&layers, &json!({"scene": "harnix", "project_root": null})).unwrap().is_none());
    }
}
