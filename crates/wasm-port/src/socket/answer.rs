//! The port's two carried traits as [`Answer`]s: [`KernelAnswer`] on the product's end,
//! [`ControlAnswer`] on the process's.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::{Answer, Inbound, CTL, PROC};
use crate::rpc::{serve_control, serve_kernel, told_control, told_kernel};
use crate::{Control, Kernel, RpcError};

fn unknown(method: &str) -> RpcError {
    RpcError::not_found(format!("`{method}` is not a method of this end of the extension wire"))
}

/// Answers `proc.*` with a [`Kernel`]: the product's end.
pub struct KernelAnswer(pub Arc<dyn Kernel>);

/// A call's metadata is the wire's to work out (from the parent this end sent), never the
/// process's, and the kernel is handed it as the wire made it.
#[async_trait]
impl Answer for KernelAnswer {
    async fn call(&self, call: Inbound, args: Value) -> Result<Value, RpcError> {
        let Some(name) = call.method.strip_prefix(PROC) else {
            return Err(unknown(&call.method));
        };
        serve_kernel(self.0.as_ref(), name, call.meta, args).await
    }

    fn notify(&self, method: &str, params: Value) {
        if let Some(name) = method.strip_prefix(PROC) {
            // Nobody to tell that a notification was not one of the port's.
            let _ = told_kernel(self.0.as_ref(), name, params);
        }
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
        let meta = call.meta.clone();
        let answer = serve_control(self.0.as_ref(), name, call.meta, args);
        let Some(own) = own else {
            return answer.await;
        };
        tokio::pin!(answer);
        tokio::select! {
            answered = &mut answer => answered,
            _ = call.cancel.cancelled() => {
                let _ = self.0.cancel(meta, own).await;
                answer.await
            }
        }
    }

    fn notify(&self, method: &str, params: Value) {
        if let Some(name) = method.strip_prefix(CTL) {
            // Nobody to tell that a notification was not one of the port's.
            let _ = told_control(self.0.as_ref(), name, params);
        }
    }
}
