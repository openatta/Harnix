//! The methods a module adds, and who may call them.
//!
//! Every method declares the principals it is for. Nothing is open by
//! default: a method declared for `Device` is reachable by a paired device,
//! one declared for `Module("trade")` only by that module, and so on — the
//! declaration becomes the method's row in the authorization table.

use std::future::Future;
use std::sync::Arc;

use harnix_authz::Principal;
use harnix_contract::RpcError;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use crate::context::CallContext;

/// What answers a call. Usually built by [`MethodBuilder::handler`] from a
/// typed async function.
#[async_trait::async_trait]
pub trait Handler: Send + Sync {
    async fn call(&self, cx: CallContext, params: Value) -> Result<Value, RpcError>;
}

/// One declared method.
#[derive(Clone)]
pub struct MethodDecl {
    /// The full name: `app.<id>.<name>`, or `harnix.<name>` for a built-in.
    pub name: String,
    pub principals: Vec<Principal>,
    /// A JSON Schema for the params, when the module gives one. Listed in
    /// the catalog and on the diagnostics page; the handler's own
    /// deserialization is what enforces the shape.
    pub schema: Option<Value>,
    pub description: String,
    pub handler: Arc<dyn Handler>,
}

pub struct MethodRegistry {
    prefix: String,
    decls: Vec<MethodDecl>,
}

impl MethodRegistry {
    #[doc(hidden)]
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            decls: Vec::new(),
        }
    }

    /// Declare `<prefix><name>` for one principal; `allow` adds more. The
    /// method exists once `handler` (or `raw`) is called.
    pub fn method(&mut self, name: &str, principal: Principal) -> MethodBuilder<'_> {
        MethodBuilder {
            name: format!("{}{name}", self.prefix),
            principals: vec![principal],
            schema: None,
            description: String::new(),
            registry: self,
        }
    }

    #[doc(hidden)]
    pub fn into_decls(self) -> Vec<MethodDecl> {
        self.decls
    }
}

pub struct MethodBuilder<'a> {
    registry: &'a mut MethodRegistry,
    name: String,
    principals: Vec<Principal>,
    schema: Option<Value>,
    description: String,
}

impl MethodBuilder<'_> {
    pub fn allow(mut self, principal: Principal) -> Self {
        self.principals.push(principal);
        self
    }

    pub fn schema(mut self, schema: Value) -> Self {
        self.schema = Some(schema);
        self
    }

    pub fn describe(mut self, text: impl Into<String>) -> Self {
        self.description = text.into();
        self
    }

    /// A typed handler: params are deserialized into `P` (a mismatch is
    /// `INVALID_PARAMS`), the result serialized from `R`.
    pub fn handler<P, R, F, Fut>(self, f: F)
    where
        P: DeserializeOwned + Send + 'static,
        R: Serialize + Send + 'static,
        F: Fn(CallContext, P) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<R, RpcError>> + Send + 'static,
    {
        self.finish(Arc::new(Typed {
            f,
            _types: std::marker::PhantomData::<fn(P) -> R>,
        }))
    }

    /// A handler on raw JSON.
    pub fn raw<F, Fut>(self, f: F)
    where
        F: Fn(CallContext, Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value, RpcError>> + Send + 'static,
    {
        self.finish(Arc::new(Raw(f)))
    }

    /// A handler that is already a [`Handler`].
    pub fn with(self, handler: Arc<dyn Handler>) {
        self.finish(handler)
    }

    fn finish(self, handler: Arc<dyn Handler>) {
        self.registry.decls.push(MethodDecl {
            name: self.name,
            principals: self.principals,
            schema: self.schema,
            description: self.description,
            handler,
        });
    }
}

struct Typed<F, P, R> {
    f: F,
    _types: std::marker::PhantomData<fn(P) -> R>,
}

#[async_trait::async_trait]
impl<P, R, F, Fut> Handler for Typed<F, P, R>
where
    P: DeserializeOwned + Send + 'static,
    R: Serialize + Send + 'static,
    F: Fn(CallContext, P) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<R, RpcError>> + Send + 'static,
{
    async fn call(&self, cx: CallContext, params: Value) -> Result<Value, RpcError> {
        let params: P = serde_json::from_value(params)
            .map_err(|e| RpcError::invalid_params(e.to_string()))?;
        let result = (self.f)(cx, params).await?;
        serde_json::to_value(result).map_err(|e| RpcError::new(harnix_contract::codes::INTERNAL_ERROR, e.to_string()))
    }
}

struct Raw<F>(F);

#[async_trait::async_trait]
impl<F, Fut> Handler for Raw<F>
where
    F: Fn(CallContext, Value) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Value, RpcError>> + Send + 'static,
{
    async fn call(&self, cx: CallContext, params: Value) -> Result<Value, RpcError> {
        (self.0)(cx, params).await
    }
}
