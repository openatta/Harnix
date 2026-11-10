//! Pairing, listing and revoking devices.
//!
//! What a paired device is — its key, its admission, the channels it holds — is the kernel's;
//! this is the interface for managing who is paired: the methods the settings page's device list
//! calls, through what `devices.manage` hands an extension.

use harnix_ext::{caps, Access, Extension, Manifest, Registrar, Rpc, RpcError};
use serde_json::{json, Value};

#[derive(Default)]
pub struct Devices;

#[async_trait::async_trait]
impl Extension for Devices {
    type Config = ();

    fn manifest(&self) -> Manifest {
        Manifest::new("devices", env!("CARGO_PKG_VERSION"))
            .title("Devices")
            .disclosure("配对、列出、吊销设备；吊销时断开它的所有通道")
            .requires([caps::DEVICES_MANAGE])
    }

    fn register(&self, r: &mut Registrar<'_, ()>) {
        r.method("list")
            .access(Access::kernel().with_device())
            .describe("the paired devices")
            .raw(|call, _| async move { Ok(call.ext().devices()?.list()) });
        r.method("pair.begin")
            .access(Access::kernel().with_device())
            .describe("a pairing code for a new device, shown once")
            .raw(|call, params| async move {
                let label = params.get("label").and_then(Value::as_str).unwrap_or("device");
                Ok(call.ext().devices()?.pair_begin(label))
            });
        r.method("pair.complete")
            .access(Access::kernel().with_device())
            .describe("finish pairing with the code and the device's public key")
            .raw(|call, params| async move {
                call.ext().devices()?.pair_complete(&require(&params, "code")?, &require(&params, "public_key")?)
            });
        r.method("revoke")
            .access(Access::kernel().with_device())
            .describe("forget a device and end every channel it holds")
            .raw(|call, params| async move {
                let revoked = call.ext().devices()?.revoke(&require(&params, "device_id")?);
                Ok(json!({"revoked": revoked}))
            });
    }
}

fn require(params: &Value, key: &str) -> Rpc<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| RpcError::invalid_params(format!("missing {key}")))
}
