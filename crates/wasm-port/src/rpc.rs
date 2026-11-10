//! The port as names and JSON: each method of [`Kernel`], [`Agent`] and [`Control`] is a name and
//! one JSON value beside the call's [`CallMeta`], answered by one JSON value or an [`RpcError`];
//! each notification a name and a value, answered by nothing.
//!
//! [`serve_kernel`], [`serve_agent`] and [`serve_control`] answer a call so named on whatever
//! implements the trait, [`told_kernel`] and [`told_control`] take a notification;
//! [`KernelClient`], [`AgentClient`] and [`ControlClient`] implement the trait by sending each call
//! to a [`Wire`]. (An edition with a process per application carries
//! `Kernel` and `Control` over its link; its `Agent` is beside the host, and needs no wire. The
//! wire for it is here so a test can prove its values survive one too.) A link between two processes is a `Wire`; under the `loopback` feature [`Loopback`] is
//! one that goes nowhere — each call is encoded to text, decoded, answered by the target, and its
//! answer encoded and decoded again — so a test run through it proves every argument and answer
//! survives the trip.

use std::sync::Arc;

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use crate::*;

/// Where an encoded call goes, and its answer comes from.
#[async_trait]
pub trait Wire: Send + Sync {
    async fn send(&self, method: &'static str, meta: CallMeta, params: Value) -> Result<Value, RpcError>;
    /// A notification: nothing answers it, and sending it does not wait. One the wire can no
    /// longer carry is dropped.
    fn notify(&self, method: &'static str, params: Value);
}

fn encode<T: Serialize>(value: &T) -> Result<Value, RpcError> {
    serde_json::to_value(value)
        .map_err(|e| RpcError::new(harnix_contract::codes::INTERNAL_ERROR, format!("not encoded: {e}")))
}

fn decode<T: DeserializeOwned>(method: &str, value: Value) -> Result<T, RpcError> {
    serde_json::from_value(value)
        .map_err(|e| RpcError::invalid_params(format!("`{method}`: not what it takes: {e}")))
}

fn unknown(method: &str) -> RpcError {
    RpcError::not_found(format!("`{method}` is not a method of this port"))
}

/// Declares the wire side of a trait from its method lists: the client that sends each call (with
/// its metadata) and each notification, the server function that answers a call, and the one that
/// takes a notification. A method listed here and not in the trait (or the other way round, or
/// with another type) does not compile.
macro_rules! wire {
    ($Trait:ident, $Client:ident, $serve:ident, $told:ident,
     { $( $name:ident ( $arg:ty ) -> $ret:ty ; )* }
     notify { $( $note:ident ( $narg:ty ) ; )* }) => {
        #[doc = concat!("[`", stringify!($Trait), "`], over a [`Wire`].")]
        pub struct $Client(pub Arc<dyn Wire>);

        #[async_trait]
        impl $Trait for $Client {
            $(
                async fn $name(&self, meta: CallMeta, arg: $arg) -> Result<$ret, RpcError> {
                    let answer = self.0.send(stringify!($name), meta, encode(&arg)?).await?;
                    decode(stringify!($name), answer)
                }
            )*
            $(
                fn $note(&self, arg: $narg) {
                    // Nothing answers a notification, so one that would not encode goes nowhere.
                    if let Ok(params) = encode(&arg) {
                        self.0.notify(stringify!($note), params);
                    }
                }
            )*
        }

        #[doc = concat!("Answer one call of [`", stringify!($Trait), "`], by its name.")]
        pub async fn $serve(target: &dyn $Trait, method: &str, meta: CallMeta, params: Value) -> Result<Value, RpcError> {
            match method {
                $(
                    stringify!($name) => {
                        let arg: $arg = decode(method, params)?;
                        encode(&target.$name(meta, arg).await?)
                    }
                )*
                other => Err(unknown(other)),
            }
        }

        #[doc = concat!("Take one notification of [`", stringify!($Trait), "`], by its name.")]
        pub fn $told(target: &dyn $Trait, method: &str, params: Value) -> Result<(), RpcError> {
            match method {
                $(
                    stringify!($note) => {
                        target.$note(decode(method, params)?);
                        Ok(())
                    }
                )*
                other => Err(unknown(other)),
            }
        }
    };
}

/// [`Agent`]'s calls are the host's own, beside it: no metadata, no notifications.
macro_rules! bare_wire {
    ($Trait:ident, $Client:ident, $serve:ident, { $( $name:ident ( $arg:ty ) -> $ret:ty ; )* }) => {
        #[doc = concat!("[`", stringify!($Trait), "`], over a [`Wire`].")]
        pub struct $Client(pub Arc<dyn Wire>);

        #[async_trait]
        impl $Trait for $Client {
            $(
                async fn $name(&self, arg: $arg) -> Result<$ret, RpcError> {
                    let answer = self.0.send(stringify!($name), CallMeta::default(), encode(&arg)?).await?;
                    decode(stringify!($name), answer)
                }
            )*
        }

        #[doc = concat!("Answer one call of [`", stringify!($Trait), "`], by its name.")]
        pub async fn $serve(target: &dyn $Trait, method: &str, params: Value) -> Result<Value, RpcError> {
            match method {
                $(
                    stringify!($name) => {
                        let arg: $arg = decode(method, params)?;
                        encode(&target.$name(arg).await?)
                    }
                )*
                other => Err(unknown(other)),
            }
        }
    };
}

wire!(Kernel, KernelClient, serve_kernel, told_kernel, {
    register(Registration) -> ();
    withdraw(String) -> ();
    call(CallAs) -> Value;
    publish(Publish) -> ();
    subscribe(Subscription) -> Value;
    unsubscribe(Subscription) -> ();
    fetch(Fetch) -> Fetched;
    turn_chain(TurnRef) -> TurnChain;
} notify {
    progress(Progress);
});

bare_wire!(Agent, AgentClient, serve_agent, {
    has_instance(String) -> bool;
    install_agent(AgentInstall) -> Value;
    uninstall_agent(AgentRef) -> ();
    switch_agent(AgentSwitch) -> ();
    place_skills(Skills) -> ();
    remove_skills(Skills) -> ();
    reload_instance(String) -> ();
    expose_tools(Offer) -> ();
});

wire!(Control, ControlClient, serve_control, told_control, {
    place(Place) -> Placement;
    unplace(Unplace) -> Value;
    start(Start) -> Started;
    stop(Stop) -> ();
    forget(String) -> ();
    status(String) -> Value;
    call_method(MethodCall) -> Value;
    deliver(TopicValue) -> ();
    call_tool(ToolCall) -> ToolReply;
    hook(HookCall) -> HookReply;
    cancel(String) -> ();
    shutdown(String) -> ();
} notify {
    turn_started(TurnChain);
    turn_settled(TurnRef);
});

#[cfg(any(test, feature = "loopback"))]
pub use loopback::Loopback;

#[cfg(any(test, feature = "loopback"))]
mod loopback {
    use super::*;

    enum Target {
        Kernel(Arc<dyn Kernel>),
        Agent(Arc<dyn Agent>),
        Control(Arc<dyn Control>),
    }

    /// A [`Wire`] to a target in this process, through text both ways. Counts what it carried.
    pub struct Loopback {
        target: Target,
        carried: std::sync::atomic::AtomicU64,
    }

    impl Loopback {
        /// `kernel`, as a host sees it across a wire.
        pub fn kernel(kernel: Arc<dyn Kernel>) -> Arc<KernelClient> {
            Arc::new(KernelClient(Arc::new(Self::new(Target::Kernel(kernel)))))
        }

        /// `agent`, as a host would see it across a wire.
        pub fn agent(agent: Arc<dyn Agent>) -> Arc<AgentClient> {
            Arc::new(AgentClient(Arc::new(Self::new(Target::Agent(agent)))))
        }

        /// `control`, as a product sees it across a wire.
        pub fn control(control: Arc<dyn Control>) -> Arc<ControlClient> {
            Arc::new(ControlClient(Arc::new(Self::new(Target::Control(control)))))
        }

        fn new(target: Target) -> Self {
            Self {
                target,
                carried: Default::default(),
            }
        }
    }

    /// Through text, as a wire would carry it.
    fn trip<T: Serialize + DeserializeOwned>(value: &T) -> T {
        let text = serde_json::to_string(value).expect("a port value encodes");
        serde_json::from_str(&text).expect("a port value decodes as it was encoded")
    }

    #[async_trait]
    impl Wire for Loopback {
        async fn send(&self, method: &'static str, meta: CallMeta, params: Value) -> Result<Value, RpcError> {
            self.carried
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let (meta, params) = (trip(&meta), trip(&params));
            let answer = match &self.target {
                Target::Kernel(k) => serve_kernel(k.as_ref(), method, meta, params).await,
                Target::Agent(a) => serve_agent(a.as_ref(), method, params).await,
                Target::Control(c) => serve_control(c.as_ref(), method, meta, params).await,
            };
            trip(&answer)
        }

        fn notify(&self, method: &'static str, params: Value) {
            self.carried
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let params = trip(&params);
            let _ = match &self.target {
                Target::Kernel(k) => told_kernel(k.as_ref(), method, params),
                Target::Agent(_) => Err(unknown(method)),
                Target::Control(c) => told_control(c.as_ref(), method, params),
            };
        }
    }

    impl Loopback {
        /// Calls carried so far.
        pub fn carried(&self) -> u64 {
            self.carried.load(std::sync::atomic::Ordering::Relaxed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A kernel and an agent engine that keep what they were told, and answer what a subscription
    /// would.
    #[derive(Default)]
    struct Kept(Mutex<Vec<String>>);

    #[async_trait]
    impl Kernel for Kept {
        async fn register(&self, _: CallMeta, r: Registration) -> Result<(), RpcError> {
            self.0.lock().unwrap().push(format!("register {} {} {}", r.app, r.methods.len(), r.digest));
            Ok(())
        }
        async fn withdraw(&self, _: CallMeta, app: String) -> Result<(), RpcError> {
            Err(RpcError::refused(format!("{app} was never here")))
        }
        async fn call(&self, meta: CallMeta, c: CallAs) -> Result<Value, RpcError> {
            Ok(serde_json::json!({"called": c.method, "params": c.params, "meta": meta}))
        }
        async fn publish(&self, _: CallMeta, _: Publish) -> Result<(), RpcError> {
            Ok(())
        }
        async fn subscribe(&self, _: CallMeta, s: Subscription) -> Result<Value, RpcError> {
            Ok(serde_json::json!({"topic": s.topic, "value": null}))
        }
        async fn unsubscribe(&self, _: CallMeta, _: Subscription) -> Result<(), RpcError> {
            Ok(())
        }
        async fn fetch(&self, _: CallMeta, f: Fetch) -> Result<Fetched, RpcError> {
            Ok(Fetched { status: 200, headers: f.headers, body_b64: f.body_b64.unwrap_or_default() })
        }
        async fn turn_chain(&self, _: CallMeta, t: TurnRef) -> Result<TurnChain, RpcError> {
            Ok(TurnChain { instance: t.instance, session_id: t.session_id, chain: vec![ChainLink::new("acme.a", "1")] })
        }
        fn progress(&self, p: Progress) {
            self.0.lock().unwrap().push(format!("progress {}", p.text));
        }
    }

    #[async_trait]
    impl Agent for Kept {
        async fn has_instance(&self, instance: String) -> Result<bool, RpcError> {
            Ok(instance == "main")
        }
        async fn install_agent(&self, a: AgentInstall) -> Result<Value, RpcError> {
            Ok(serde_json::json!({"archive": a.archive}))
        }
        async fn uninstall_agent(&self, _: AgentRef) -> Result<(), RpcError> {
            Ok(())
        }
        async fn switch_agent(&self, _: AgentSwitch) -> Result<(), RpcError> {
            Ok(())
        }
        async fn place_skills(&self, _: Skills) -> Result<(), RpcError> {
            Ok(())
        }
        async fn remove_skills(&self, _: Skills) -> Result<(), RpcError> {
            Ok(())
        }
        async fn reload_instance(&self, _: String) -> Result<(), RpcError> {
            Ok(())
        }
        async fn expose_tools(&self, o: Offer) -> Result<(), RpcError> {
            self.0.lock().unwrap().push(format!("offer {} {}", o.app, o.tools.len()));
            Ok(())
        }
    }

    #[tokio::test]
    async fn every_call_and_answer_survives_the_trip() {
        let kept = Arc::new(Kept::default());
        let wire = Loopback::kernel(kept.clone());
        let agent = Loopback::agent(kept.clone());
        let registration = Registration {
            app: "acme.notes".into(),
            methods: vec![MethodDecl {
                name: "notes.add".into(),
                description: String::new(),
                subjects: vec!["device".into()],
            }],
            topics: vec!["notes".into()],
            spaces: vec![SpaceDecl::new("acme.notes").instance("main")],
            calls: vec!["topic:harnix.sessions.status".into()],
            tools: Vec::new(),
            digest: "d1".into(),
        };
        wire.register(CallMeta::default(), registration).await.unwrap();
        let meta = CallMeta::new("c2").parent("c1").chain([ChainLink::new("acme.notes", "c1")]).deadline_ms(9);
        let answer = wire
            .call(
                meta.clone(),
                CallAs {
                    app: "acme.notes".into(),
                    method: "harnix.session.send".into(),
                    params: serde_json::json!({"n": 1}),
                },
            )
            .await
            .unwrap();
        assert_eq!(answer["params"]["n"], serde_json::json!(1));
        assert_eq!(answer["meta"], serde_json::to_value(&meta).unwrap(), "a call's metadata crosses the wire");
        let turn = TurnRef { instance: "main".into(), session_id: "s".into() };
        assert_eq!(wire.turn_chain(CallMeta::default(), turn).await.unwrap().chain, [ChainLink::new("acme.a", "1")]);
        let fetched = Fetch { app: "acme.notes".into(), method: "GET".into(), url: "https://x".into(), headers: vec![], body_b64: Some("aGk=".into()) };
        assert_eq!(wire.fetch(CallMeta::default(), fetched).await.unwrap().body_b64, "aGk=");
        wire.progress(Progress { app: "acme.notes".into(), tool: "t".into(), call_id: "c".into(), text: "half".into() });
        assert!(agent.has_instance("main".into()).await.unwrap());
        assert!(!agent.has_instance("other".into()).await.unwrap());
        let refused = wire.withdraw(CallMeta::default(), "acme.notes".into()).await.unwrap_err();
        assert_eq!(refused.code, harnix_contract::codes::REFUSED);
        assert!(refused.message.contains("never here"));
        agent.expose_tools(Offer {
            app: "acme.notes".into(),
            title: None,
            version: "1.0.0".into(),
            component: true,
            tools: vec![ToolDef {
                name: "search".into(),
                description: "d".into(),
                doc: None,
                input_schema: "{}".into(),
                read_only: true,
                concurrency_safe: false,
            }],
        })
        .await
        .unwrap();
        assert_eq!(*kept.0.lock().unwrap(), ["register acme.notes 1 d1", "progress half", "offer acme.notes 1"]);
        // A name the port does not have, and a value it does not take, are errors, not panics.
        let kernel: &dyn Kernel = kept.as_ref();
        assert_eq!(
            serve_kernel(kernel, "format", CallMeta::default(), Value::Null).await.unwrap_err().code,
            harnix_contract::codes::METHOD_NOT_FOUND
        );
        assert_eq!(
            serve_kernel(kernel, "call", CallMeta::default(), serde_json::json!(3)).await.unwrap_err().code,
            harnix_contract::codes::INVALID_PARAMS
        );
        // A notification is not a call, nor the other way round.
        assert!(told_kernel(kernel, "call", Value::Null).is_err());
        assert_eq!(
            serve_kernel(kernel, "progress", CallMeta::default(), Value::Null).await.unwrap_err().code,
            harnix_contract::codes::METHOD_NOT_FOUND
        );
        // Each trait answers its own names only: the agent engine's are not the kernel's.
        assert_eq!(
            serve_kernel(kernel, "has_instance", CallMeta::default(), serde_json::json!("main")).await.unwrap_err().code,
            harnix_contract::codes::METHOD_NOT_FOUND
        );
    }

    #[test]
    fn replies_are_tagged_as_a_wire_reads_them() {
        let failed = ToolReply::Failed {
            failure: Failure::TimedOut,
            message: "late".into(),
        };
        assert_eq!(
            serde_json::to_value(&failed).unwrap(),
            serde_json::json!({"outcome": "failed", "failure": "timed_out", "message": "late"})
        );
        assert_eq!(
            serde_json::to_value(HookReply::Block("no".into())).unwrap(),
            serde_json::json!({"decision": "block", "text": "no"})
        );
        assert_eq!(serde_json::to_value(HookReply::Proceed).unwrap(), serde_json::json!({"decision": "proceed"}));
    }
}
