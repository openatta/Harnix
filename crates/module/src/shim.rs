//! A module, as an extension's descriptor.
//!
//! The extension host knows one shape, `harnix_ext::ExtensionDescriptor`. A module's registration
//! functions are read into one here: its methods and topics under the names it always had (its
//! prefix, `harnix.` for a built-in, `app.<id>.` for an application's), its route as an `axum`
//! route, its spaces, its engine contribution and its templates. The module's own context is made
//! when it starts and set where its handlers find it; until then a method answers that it is not
//! served yet.

use std::sync::{Arc, OnceLock};

use harnix_authz::access_for;
use harnix_contract::s1::ExtId;
use harnix_contract::RpcError;
use harnix_ext::{Call, DescriptorBuilder, ExtensionDescriptor, MethodHandler};
use serde_json::Value;

use crate::{
    CallContext, EngineContrib, ErasedModule, EventRegistry, Handler, InstanceTemplates, MethodRegistry,
    ModuleContext, RouteRegistry, SpaceRegistry,
};

/// Where a module's context is, once it is started.
pub type ModuleSlot = Arc<OnceLock<ModuleContext>>;

/// What a module declares, as a descriptor under `id`, its names under `prefix`. Its handlers
/// answer with the context set in `slot`.
pub fn describe(module: &dyn ErasedModule, id: &ExtId, prefix: &str, slot: &ModuleSlot) -> ExtensionDescriptor {
    let mut d = DescriptorBuilder::native(id.clone(), module.manifest().to_ext());

    let mut methods = MethodRegistry::new(prefix);
    module.methods(&mut methods);
    for decl in methods.into_decls() {
        let access = match decl.requires {
            Some(cap) => access_for(&decl.principals).requiring(cap),
            None => access_for(&decl.principals),
        };
        let mut method = d.method(&decl.name).access(access).describe(&decl.description);
        if let Some(schema) = decl.schema {
            method = method.schema(schema);
        }
        method.with(Arc::new(Legacy { handler: decl.handler, slot: slot.clone(), name: decl.name }));
    }

    let mut events = EventRegistry::new(prefix);
    module.events(&mut events);
    for topic in events.into_decls() {
        let mut declared = d.topic(&topic.name).access(access_for(&topic.principals));
        for param in &topic.params {
            declared = declared.param(param);
        }
    }

    let mut routes = RouteRegistry::new();
    module.routes(&mut routes);
    if let Some(route) = routes.into_decl() {
        d.route("", access_for(&route.principals), harnix_ext::http::axum::router(route.router));
    }

    let mut spaces = SpaceRegistry::default();
    module.spaces(&mut spaces);
    for space in spaces.into_list() {
        d.space(space);
    }

    let mut contrib = EngineContrib::new();
    module.engine(&mut contrib);
    if !contrib.scenes().is_empty()
        || !contrib.providers().is_empty()
        || !contrib.prompt_gates().is_empty()
        || contrib.credential_source().is_some()
        || !contrib.admitted().is_empty()
        || !contrib.foreign_hooks().is_empty()
    {
        d.engine().extend(contrib);
    }

    let mut templates = InstanceTemplates::default();
    module.templates(&mut templates);
    for mut template in templates.into_list() {
        template.id = format!("{id}.{}", template.id);
        d.template(template);
    }
    d.build()
}

/// A module's method, answered with its context once it is started.
struct Legacy {
    handler: Arc<dyn Handler>,
    slot: ModuleSlot,
    name: String,
}

#[async_trait::async_trait]
impl MethodHandler for Legacy {
    async fn call(&self, call: Call, params: Value) -> Result<Value, RpcError> {
        let Some(module) = self.slot.get().cloned() else {
            return Err(RpcError::not_found(format!("method `{}` is not served here (yet)", self.name)));
        };
        self.handler.call(CallContext::new(call, module), params).await
    }
}
