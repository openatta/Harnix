//! What a package serves over HTTP, under `/ext/<id>/` on the process's own listener: the two
//! routes of its descriptor ([`UiFiles`], [`McpEndpoint`]) are the carriers' shared ones. What is
//! this carrier's is how a daemon's `tools/call` reaches the package: over its wire, to its own
//! process.

use std::sync::Arc;

use harnix_carrier_shared::routes::{mcp_call, mcp_result};
use harnix_carrier_shared::{Hosted, Loaded};
use harnix_wasm_port::socket::now_ms;
use harnix_wasm_port::CallMeta;
use serde_json::Value;

use crate::registry::Registry;

#[async_trait::async_trait]
impl Hosted for Registry {
    fn running(&self, id: &str) -> Option<Arc<Loaded>> {
        self.get(id)
    }

    async fn call_tool(&self, app: &Loaded, params: &Value) -> Result<Value, (i64, String)> {
        let _held = self.hold(&app.id);
        let control = match self.awake(&app.id).await {
            Some(control) if app.component => control,
            _ => return Err((-32601, format!("`{}` has no tools", app.id))),
        };
        let call = mcp_call(app, params)?;
        // A client that goes away drops this future, and the wire takes the call back.
        let meta = CallMeta::default().with_deadline(Some(now_ms() + app.timeout.as_millis() as u64));
        mcp_result(app, control.call_tool(meta, call).await)
    }
}
