//! Host topics a module publishes on.
//!
//! A topic carries **whole-value snapshots**, never deltas: a subscriber
//! that arrives late gets the current value and every later one, and a merge
//! bug that only shows in one order of arrival cannot exist. A topic may take
//! subscription parameters (`app.trade.job { job_id }`); each distinct set of
//! parameters is its own value.

use std::sync::Arc;

use harnix_authz::Principal;
use harnix_contract::RpcError;
use serde::Serialize;
use serde_json::Value;

use crate::context::Kernel;

#[derive(Debug, Clone)]
pub struct TopicDecl {
    /// The full name: `app.<id>.<name>`, or `harnix.<name>` for a built-in.
    pub name: String,
    /// The subscription parameters it takes, by name.
    pub params: Vec<String>,
    pub principals: Vec<Principal>,
}

pub struct EventRegistry {
    prefix: String,
    decls: Vec<TopicDecl>,
}

impl EventRegistry {
    #[doc(hidden)]
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            decls: Vec::new(),
        }
    }

    /// Declare `<prefix><name>`, readable by devices unless `only` says
    /// otherwise.
    pub fn topic(&mut self, name: &str) -> TopicBuilder<'_> {
        self.decls.push(TopicDecl {
            name: format!("{}{name}", self.prefix),
            params: Vec::new(),
            principals: vec![Principal::Device, Principal::AnyModule],
        });
        TopicBuilder {
            decl: self.decls.last_mut().expect("just pushed"),
        }
    }

    #[doc(hidden)]
    pub fn into_decls(self) -> Vec<TopicDecl> {
        self.decls
    }
}

pub struct TopicBuilder<'a> {
    decl: &'a mut TopicDecl,
}

impl TopicBuilder<'_> {
    pub fn param(self, name: &str) -> Self {
        self.decl.params.push(name.to_string());
        self
    }

    /// Replace who may subscribe.
    pub fn only(self, principals: &[Principal]) -> Self {
        self.decl.principals = principals.to_vec();
        self
    }
}

/// Publishing, for one module: only its own topics.
#[derive(Clone)]
pub struct Events {
    pub(crate) kernel: Arc<dyn Kernel>,
    pub(crate) prefix: String,
    pub(crate) topics: Arc<Vec<TopicDecl>>,
}

impl Events {
    /// Publish the whole current value of `<prefix><topic>` for these
    /// subscription parameters (`Value::Null` or `{}` for none).
    pub async fn publish(&self, topic: &str, params: Value, value: impl Serialize) -> Result<(), RpcError> {
        let name = format!("{}{topic}", self.prefix);
        let Some(decl) = self.topics.iter().find(|t| t.name == name) else {
            return Err(RpcError::not_found(format!("`{name}` is not a topic this module declared")));
        };
        let params = normalize(params, &decl.params)?;
        let value = serde_json::to_value(value)
            .map_err(|e| RpcError::new(harnix_contract::codes::INTERNAL_ERROR, e.to_string()))?;
        self.kernel.publish(&name, params, value).await;
        Ok(())
    }
}

/// Keep only the declared parameters, so `{job_id: 1, extra: 2}` and
/// `{job_id: 1}` are the same subscription.
fn normalize(params: Value, declared: &[String]) -> Result<Value, RpcError> {
    let mut out = serde_json::Map::new();
    let given = params.as_object().cloned().unwrap_or_default();
    for name in declared {
        match given.get(name) {
            Some(v) => {
                out.insert(name.clone(), v.clone());
            }
            None => return Err(RpcError::invalid_params(format!("missing topic parameter `{name}`"))),
        }
    }
    Ok(Value::Object(out))
}
