//! What answers the product's `ctl.*`: the host, once there is one, and the word that the product
//! asked this process to go.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use harnix_wasm_port::socket::{Answer, ControlAnswer, Inbound};
use harnix_wasm_port::{codes, Control, RpcError};
use serde_json::{json, Value};
use tokio::sync::Notify;

/// The wire is made before the host (the host's kernel is the wire's client end) and the host
/// answers the wire: so the wire starts with this, and the host is put in it once it is made.
/// Nothing reaches it before `proc.hello` is answered, which comes after.
#[derive(Default)]
pub(crate) struct Answering {
    control: OnceLock<ControlAnswer>,
    /// Told once `ctl.shutdown` has been answered by the host.
    pub(crate) shutdown: Notify,
}

impl Answering {
    pub(crate) fn answer_with(&self, control: Arc<dyn Control>) {
        let _ = self.control.set(ControlAnswer(control));
    }
}

#[async_trait]
impl Answer for Answering {
    async fn call(&self, call: Inbound, args: Value) -> Result<Value, RpcError> {
        let Some(control) = self.control.get() else {
            return Err(RpcError {
                code: codes::EXTENSION_UNAVAILABLE,
                message: "the extension process is still starting".into(),
                data: Some(json!({"failure": "starting"})),
            });
        };
        let shutdown = call.method == "ctl.shutdown";
        let answer = control.call(call, args).await;
        if shutdown {
            self.shutdown.notify_one();
        }
        answer
    }
}
