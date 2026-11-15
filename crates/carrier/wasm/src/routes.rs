//! What a package serves over HTTP, under `/ext/<id>/`: the two routes of its descriptor
//! ([`UiFiles`], [`McpEndpoint`]) are the carriers' shared ones. What is this carrier's is how a
//! daemon's `tools/call` reaches the package: a daemon is another process and has no VM, so its
//! sessions call the tools here, and they run in this process's VM.

use std::sync::Arc;

use harnix_carrier_shared::routes::{mcp_call, mcp_result};
use harnix_carrier_shared::{Hosted, Loaded};
use harnix_wasm_port::CallMeta;
use serde_json::Value;

use crate::registry::Registry;

#[async_trait::async_trait]
impl Hosted for Registry {
    fn running(&self, id: &str) -> Option<Arc<Loaded>> {
        self.get(id)
    }

    async fn call_tool(&self, app: &Loaded, params: &Value) -> Result<Value, (i64, String)> {
        let control = match self.control() {
            Some(control) if app.component => control,
            _ => return Err((-32601, format!("`{}` has no tools", app.id))),
        };
        let call = mcp_call(app, params)?;
        // A client that goes away drops this future, and the call's instance with it.
        mcp_result(app, control.call_tool(CallMeta::default(), call).await)
    }
}
