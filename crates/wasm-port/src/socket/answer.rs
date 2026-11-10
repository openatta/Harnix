//! The port's two carried traits as [`Answer`]s: [`KernelAnswer`] on the product's end,
//! [`ControlAnswer`] on the process's.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::{Answer, Inbound, CTL, PROC};
use crate::rpc::{serve_control, serve_kernel};
use crate::{Control, Kernel, RpcError};

fn unknown(method: &str) -> RpcError {
    RpcError::not_found(format!("`{method}` is not a method of this end of the extension wire"))
}

/// Answers `proc.*` with a [`Kernel`]: the product's end.
pub struct KernelAnswer(pub Arc<dyn Kernel>);

#[async_trait]
impl Answer for KernelAnswer {
    async fn call(&self, call: Inbound, mut args: Value) -> Result<Value, RpcError> {
        let Some(name) = call.method.strip_prefix(PROC) else {
            return Err(unknown(&call.method));
        };
        // Who waits on a `host.call` is what this end worked out from the call's parent; the chain
        // the process wrote into the value would let it make another extension refuse a call.
        if name == "call" {
            if let Some(args) = args.as_object_mut() {
                let chain: Vec<&str> = call.meta.chain.iter().map(|link| link.ext.as_str()).collect();
                args.insert("chain".into(), chain.into());
            }
        }
        serve_kernel(self.0.as_ref(), name, args).await
    }
}

/// Answers `ctl.*` with a [`Control`]: the process's end.
pub struct ControlAnswer(pub Arc<dyn Control>);

#[async_trait]
impl Answer for ControlAnswer {
    async fn call(&self, call: Inbound, args: Value) -> Result<Value, RpcError> {
        let Some(name) = call.method.strip_prefix(CTL) else {
            return Err(unknown(&call.method));
        };
        // A tool call or a hook has an id of its own the host cancels it by; a `$/cancel` (or its
        // deadline) for one is that. The call still answers, cancelled or not.
        let own = match name {
            "call_tool" | "hook" => args.get("call_id").and_then(Value::as_str).map(str::to_owned),
            _ => None,
        };
        let answer = serve_control(self.0.as_ref(), name, args);
        let Some(own) = own else {
            return answer.await;
        };
        tokio::pin!(answer);
        tokio::select! {
            answered = &mut answer => answered,
            _ = call.cancel.cancelled() => {
                let _ = self.0.cancel(own).await;
                answer.await
            }
        }
    }
}
