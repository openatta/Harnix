//! What answers the product's `ctl.*`: the host, once there is one, and the word that the product
//! asked this process to go.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use harnix_package::acquire::refused;
use harnix_wasm_port::socket::{Answer, ControlAnswer, Inbound};
use harnix_wasm_port::{codes, Control, RpcError};
use serde_json::{json, Value};
use tokio::sync::Notify;

/// The wire is made before the host (the host's kernel is the wire's client end) and the host
/// answers the wire: so the wire starts with this, and the host is put in it once it is made.
/// Nothing reaches it before `proc.hello` is answered, which comes after.
pub(crate) struct Answering {
    control: OnceLock<ControlAnswer>,
    /// Told once `ctl.shutdown` has been answered by the host.
    pub(crate) shutdown: Notify,
    /// The package and run directory of its command line: what a start runs from.
    pkg: Option<PathBuf>,
    run: Option<PathBuf>,
    /// The extension this process is (`--ext`): what a start must name.
    ext: String,
    /// A `ctl.start` has come: one does, and a process that failed to start is replaced, not
    /// started again.
    started: AtomicBool,
    /// Why the process cannot go on, once a start showed it: [`crate::run`] ends on it.
    fatal: Mutex<Option<String>>,
    fatal_told: Notify,
}

impl Answering {
    pub(crate) fn new(ext: String, pkg: Option<PathBuf>, run: Option<PathBuf>) -> Self {
        Self {
            control: OnceLock::new(),
            shutdown: Notify::new(),
            pkg,
            run,
            ext,
            started: AtomicBool::new(false),
            fatal: Mutex::new(None),
            fatal_told: Notify::new(),
        }
    }

    /// Resolves with why the process must end, once a start showed it.
    pub(crate) async fn fatal(&self) -> String {
        loop {
            if let Some(why) = self.fatal.lock().unwrap_or_else(|e| e.into_inner()).clone() {
                return why;
            }
            // One waiter, and a permit stored if it comes first.
            self.fatal_told.notified().await;
        }
    }

    fn give_up(&self, why: String) {
        *self.fatal.lock().unwrap_or_else(|e| e.into_inner()) = Some(why);
        self.fatal_told.notify_one();
    }

    /// What a `ctl.start` may be: the only one, for this extension, and without a package or run
    /// directory of its own — they are on the command line, given with the sandbox that was made
    /// from them before there was a wire.
    fn check_start(&self, start: &Value) -> Result<(), RpcError> {
        if self.started.swap(true, Ordering::SeqCst) {
            return Err(refused("this extension process was started already: `ctl.start` comes once"));
        }
        if start.get("app").and_then(Value::as_str) != Some(self.ext.as_str()) {
            let why = format!("`ctl.start` names {}, and this process is `{}`", start.get("app").unwrap_or(&Value::Null), self.ext);
            self.give_up(why.clone());
            return Err(refused(why));
        }
        for key in ["pkg", "run"] {
            if start.get(key).is_some_and(|v| !v.is_null()) {
                return Err(refused(format!("`ctl.start` carries `{key}`: a process takes it from its command line")));
            }
        }
        Ok(())
    }

    pub(crate) fn answer_with(&self, control: Arc<dyn Control>) {
        let _ = self.control.set(ControlAnswer(control));
    }
}

#[async_trait]
impl Answer for Answering {
    async fn call(&self, call: Inbound, mut args: Value) -> Result<Value, RpcError> {
        let Some(control) = self.control.get() else {
            return Err(RpcError {
                code: codes::EXTENSION_UNAVAILABLE,
                message: "the extension process is still starting".into(),
                data: Some(json!({"failure": "starting"})),
            });
        };
        // Its own directories, from its command line.
        let start = call.method == "ctl.start";
        if start {
            self.check_start(&args)?;
            if let Some(start) = args.as_object_mut() {
                start.insert("pkg".into(), json!(self.pkg));
                start.insert("run".into(), json!(self.run));
            }
        }
        let shutdown = call.method == "ctl.shutdown";
        let answer = control.call(call, args).await;
        // A component that is not the one the product verified: this is not a process to keep.
        if let (true, Err(e)) = (start, &answer) {
            if let Some(setup) = e.data.as_ref().and_then(|d| d.get("setup")).and_then(Value::as_str) {
                self.give_up(format!("{}: {setup}", e.message));
            }
        }
        if shutdown {
            self.shutdown.notify_one();
        }
        answer
    }

    fn notify(&self, method: &str, params: Value) {
        if let Some(control) = self.control.get() {
            control.notify(method, params);
        }
    }
}
