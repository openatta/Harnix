//! One request read and one answer written, the same way on both topologies.
//!
//! The socket hands over a text message and the split topology a POST body; from the bytes on it
//! is this, so a frame over the ceiling, one that is not JSON and one with no method are refused
//! in the same words under the same codes whichever topology carried them (concept §3.3), and an error
//! reaches the client whole, `data` and all.

use harnix_contract::{codes, RpcError};
use harnix_hub::ClientId;
use serde_json::{json, Value};

/// One JSON-RPC request, read.
#[derive(Debug)]
pub(crate) struct Request {
    /// What the answer is sent under; null when the frame had none.
    pub id: Value,
    /// It had no `id`: nothing is answered.
    pub is_notification: bool,
    pub method: String,
    /// `{}` when the frame had none.
    pub params: Value,
}

impl Request {
    /// Say which client this is, from the credential the handshake produced — not from the
    /// connection that carried the request. In a topology where the call and the frames it
    /// registers a watcher for travel on different connections, connection identity is the wrong
    /// answer, and quietly; taking it from the credential makes both topologies mean the same
    /// thing here (concept §3.3.3, item 2).
    pub fn say_client(&mut self, client: ClientId) {
        if let Some(params) = self.params.as_object_mut() {
            params.insert("__client".into(), json!(client.0));
        }
    }
}

/// A frame refused before it is a request, with the id it is answered under (null when there was
/// none to read).
#[derive(Debug)]
pub(crate) struct Malformed {
    pub id: Value,
    pub error: RpcError,
}

/// Read one request.
///
/// One over the ceiling is refused with the ceiling in the message: letting it through would mean
/// `hello` publishes a limit that nothing checks, and the client is told which frame it was by its
/// id, when one can be read.
pub(crate) fn parse_request(bytes: &[u8]) -> Result<Request, Malformed> {
    if bytes.len() > harnix_contract::MAX_FRAME_BYTES {
        let id = serde_json::from_slice::<Value>(bytes)
            .ok()
            .and_then(|v| v.get("id").cloned())
            .unwrap_or(Value::Null);
        return Err(Malformed {
            id,
            error: RpcError::new(
                codes::INVALID_REQUEST,
                format!(
                    "frame is {} bytes; the ceiling is {} — use the bulk channel",
                    bytes.len(),
                    harnix_contract::MAX_FRAME_BYTES
                ),
            ),
        });
    }
    let Ok(mut value) = serde_json::from_slice::<Value>(bytes) else {
        return Err(Malformed {
            id: Value::Null,
            error: RpcError::new(codes::PARSE_ERROR, "invalid JSON"),
        });
    };
    let is_notification = value.get("id").is_none();
    let id = value.get("id").cloned().unwrap_or(Value::Null);
    let Some(method) = value.get("method").and_then(Value::as_str).map(str::to_string) else {
        return Err(Malformed {
            id,
            error: RpcError::new(codes::INVALID_REQUEST, "missing method"),
        });
    };
    let params = value
        .get_mut("params")
        .map(Value::take)
        .unwrap_or_else(|| json!({}));
    Ok(Request {
        id,
        is_notification,
        method,
        params,
    })
}

/// A result.
pub(crate) fn ok(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// An error, whole: `data` says what a client can act on (the space a session belongs to, the
/// turn that is busy, the ceiling it went over).
pub(crate) fn error(id: Value, error: &RpcError) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": error})
}

/// A frame as the socket sends it.
pub(crate) fn text(frame: &Value) -> String {
    serde_json::to_string(frame).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_read_with_its_id_method_and_params() {
        let request =
            parse_request(br#"{"jsonrpc":"2.0","id":7,"method":"harnix.ping","params":{"a":1}}"#).unwrap();
        assert_eq!(request.id, json!(7));
        assert!(!request.is_notification);
        assert_eq!(request.method, "harnix.ping");
        assert_eq!(request.params, json!({"a": 1}));

        let notification = parse_request(br#"{"jsonrpc":"2.0","method":"harnix.ping"}"#).unwrap();
        assert!(notification.is_notification);
        assert_eq!(notification.id, Value::Null);
        assert_eq!(notification.params, json!({}), "no params is empty params");
    }

    /// What is refused before it is a request, and under which id.
    #[test]
    fn a_malformed_frame_is_refused_under_its_id() {
        let e = parse_request(b"{not json").unwrap_err();
        assert_eq!((e.id, e.error.code), (Value::Null, codes::PARSE_ERROR));

        let e = parse_request(br#"{"jsonrpc":"2.0","id":"x"}"#).unwrap_err();
        assert_eq!((e.id, e.error.code), (json!("x"), codes::INVALID_REQUEST));
        assert_eq!(e.error.message, "missing method");

        let big = format!(
            r#"{{"jsonrpc":"2.0","id":3,"method":"harnix.send","params":{{"m":"{}"}}}}"#,
            "x".repeat(harnix_contract::MAX_FRAME_BYTES)
        );
        let e = parse_request(big.as_bytes()).unwrap_err();
        assert_eq!((e.id, e.error.code), (json!(3), codes::INVALID_REQUEST));
        assert!(e.error.message.contains(&harnix_contract::MAX_FRAME_BYTES.to_string()), "{}", e.error.message);
    }

    /// The client is said in the params, over whatever the client put there itself.
    #[test]
    fn the_client_is_said_from_the_credential() {
        let mut request =
            parse_request(br#"{"jsonrpc":"2.0","id":1,"method":"harnix.attach","params":{"__client":99}}"#).unwrap();
        request.say_client(ClientId(4));
        assert_eq!(request.params["__client"], json!(4));
    }

    /// An error's `data` goes out with it; one without is just code and message.
    #[test]
    fn an_error_frame_carries_its_data() {
        let mut e = RpcError::new(codes::BUSY, "busy");
        e.data = Some(json!({"max_in_flight": 4}));
        let frame = error(json!(1), &e);
        assert_eq!(frame["error"]["data"]["max_in_flight"], 4);
        let frame = error(Value::Null, &RpcError::new(codes::PARSE_ERROR, "invalid JSON"));
        assert!(frame["error"].get("data").is_none(), "{frame}");
        let sent: Value = serde_json::from_str(&text(&ok(json!(2), json!({})))).unwrap();
        assert_eq!(sent, json!({"jsonrpc": "2.0", "id": 2, "result": {}}));
    }
}
