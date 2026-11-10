//! What the diagnostics page and `harnix.extensions` say about each extension: "my space does
//! nothing" needs an answer, not a guess.

use harnix_contract::s1::{Access, Capability};
use harnix_ext::ExtensionDescriptor;
use serde::Serialize;
use serde_json::{json, Value};

/// One extension, as the diagnostics page lists it.
#[derive(Debug, Clone, Serialize)]
pub struct ExtensionStatus {
    pub id: String,
    pub version: String,
    /// `origin` (linked into the product) or `consent` (a package).
    pub trust: &'static str,
    /// `native`, `wasm` or `process`.
    pub carrier: &'static str,
    pub enabled: bool,
    /// Where it was disabled: `assembly`, `registration`, `configuration`, `migration` or `start`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub disclosure: String,
    /// The capabilities it must be granted, may be granted, was granted, and defines.
    pub requires: Vec<String>,
    pub optional: Vec<String>,
    pub granted: Vec<String>,
    pub defines: Vec<String>,
    pub methods: Vec<MethodStatus>,
    pub topics: Vec<String>,
    pub routes: bool,
    pub tools: Vec<String>,
    pub scenes: Vec<String>,
    /// Instance templates, by their full ids.
    pub templates: Vec<String>,
    /// Spaces it is the backend of.
    pub spaces: Vec<String>,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub ui: Value,
    pub migrations: Vec<String>,
    /// After a start failure: what stays until the next start, because the instance was already
    /// built with it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// One method, as the diagnostics page lists it.
#[derive(Debug, Clone, Serialize)]
pub struct MethodStatus {
    pub name: String,
    /// Who may call it: `device`, `kernel`, `module:<id>`, `module:*`.
    pub principals: Vec<String>,
    /// The capability a caller must hold besides.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<Value>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
}

impl ExtensionStatus {
    /// What its manifest says, before anything of it is read.
    pub fn new(manifest: &harnix_ext::Manifest, granted: &[Capability]) -> Self {
        let names = |caps: &[Capability]| caps.iter().map(ToString::to_string).collect();
        Self {
            id: manifest.id().to_string(),
            version: manifest.version().to_string(),
            trust: "origin",
            carrier: "native",
            enabled: true,
            phase: None,
            reason: None,
            disclosure: manifest.disclosure_text().to_string(),
            requires: names(manifest.required()),
            optional: names(manifest.optionals()),
            granted: names(granted),
            defines: manifest.defined().iter().map(|d| d.name().to_string()).collect(),
            methods: Vec::new(),
            topics: Vec::new(),
            routes: false,
            tools: Vec::new(),
            scenes: Vec::new(),
            templates: Vec::new(),
            spaces: Vec::new(),
            ui: Value::Null,
            migrations: Vec::new(),
            note: None,
        }
    }

    /// What its descriptor declares.
    pub fn declared(&mut self, d: &ExtensionDescriptor) {
        self.methods = d
            .methods()
            .iter()
            .map(|m| MethodStatus {
                name: m.name().to_string(),
                principals: principals(m.access()),
                requires: m.access().requirement().map(ToString::to_string),
                schema: m.schema().cloned(),
                description: m.description().to_string(),
            })
            .collect();
        self.topics = d.topics().iter().map(|t| t.name().to_string()).collect();
        self.routes = !d.routes().is_empty();
        self.tools = d.tools().iter().map(|t| t.name().to_string()).collect();
        self.scenes = d.engine().map(super::contribution::scene_ids).unwrap_or_default();
        self.templates = d.templates().iter().map(|t| t.id.clone()).collect();
        self.spaces = d.spaces().iter().map(|s| s.id.clone()).collect();
        if !d.ui().is_empty() {
            self.ui = serde_json::to_value(d.ui()).unwrap_or(Value::Null);
        }
    }

    /// The line `harnix.extensions` gives it.
    pub fn summary(&self) -> Value {
        json!({
            "id": self.id,
            "version": self.version,
            "trust": self.trust,
            "carrier": self.carrier,
            "enabled": self.enabled,
            "granted": self.granted,
            "rows": self.methods.len() + self.topics.len() + usize::from(self.routes),
            "phase": self.phase,
            "reason": self.reason,
        })
    }
}

/// Who a row is open to, for a status line.
fn principals(access: &Access) -> Vec<String> {
    let mut out = Vec::new();
    if access.opens_devices() {
        out.push("device".to_string());
    }
    out.push("kernel".to_string());
    if access.opens_every_extension() {
        out.push("module:*".to_string());
    }
    out.extend(access.extensions_listed().into_iter().map(|id| format!("module:{id}")));
    out
}
