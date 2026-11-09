//! The port as names and JSON: each method of [`Kernel`], [`Agent`] and [`Control`] is a name and
//! one JSON value, answered by one JSON value or an [`RpcError`].
//!
//! [`serve_kernel`], [`serve_agent`] and [`serve_control`] answer a call so named on whatever
//! implements the trait; [`KernelClient`], [`AgentClient`] and [`ControlClient`] implement the
//! trait by sending each call to a [`Wire`]. (An edition with a process per application carries
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
    async fn send(&self, method: &'static str, params: Value) -> Result<Value, RpcError>;
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

/// Declares the wire side of a trait from its method list: the client that sends each call, and
/// the server function that answers one. A method listed here and not in the trait (or the other
/// way round, or with another type) does not compile.
macro_rules! wire {
    ($Trait:ident, $Client:ident, $serve:ident, { $( $name:ident ( $arg:ty ) -> $ret:ty ; )* }) => {
        #[doc = concat!("[`", stringify!($Trait), "`], over a [`Wire`].")]
        pub struct $Client(pub Arc<dyn Wire>);

        #[async_trait]
        impl $Trait for $Client {
            $(
                async fn $name(&self, arg: $arg) -> Result<$ret, RpcError> {
                    let answer = self.0.send(stringify!($name), encode(&arg)?).await?;
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

wire!(Kernel, KernelClient, serve_kernel, {
    register(Registration) -> ();
    withdraw(String) -> ();
    call(CallAs) -> Value;
    publish(Publish) -> ();
    subscribe(Subscription) -> Value;
    unsubscribe(Subscription) -> ();
    progress(Progress) -> ();
});

wire!(Agent, AgentClient, serve_agent, {
    has_instance(String) -> bool;
    install_agent(AgentInstall) -> Value;
    uninstall_agent(AgentRef) -> ();
    switch_agent(AgentSwitch) -> ();
    place_skills(Skills) -> ();
    remove_skills(Skills) -> ();
    reload_instance(String) -> ();
    expose_tools(Offer) -> ();
});

wire!(Control, ControlClient, serve_control, {
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
        async fn send(&self, method: &'static str, params: Value) -> Result<Value, RpcError> {
            self.carried
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let params = trip(&params);
            let answer = match &self.target {
                Target::Kernel(k) => serve_kernel(k.as_ref(), method, params).await,
                Target::Agent(a) => serve_agent(a.as_ref(), method, params).await,
                Target::Control(c) => serve_control(c.as_ref(), method, params).await,
            };
            trip(&answer)
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
        async fn register(&self, r: Registration) -> Result<(), RpcError> {
            self.0.lock().unwrap().push(format!("register {} {}", r.app, r.methods.len()));
            Ok(())
        }
        async fn withdraw(&self, app: String) -> Result<(), RpcError> {
            Err(RpcError::refused(format!("{app} was never here")))
        }
        async fn call(&self, c: CallAs) -> Result<Value, RpcError> {
            Ok(serde_json::json!({"called": c.method, "params": c.params, "chain": c.chain}))
        }
        async fn publish(&self, _: Publish) -> Result<(), RpcError> {
            Ok(())
        }
        async fn subscribe(&self, s: Subscription) -> Result<Value, RpcError> {
            Ok(serde_json::json!({"topic": s.topic, "value": null}))
        }
        async fn unsubscribe(&self, _: Subscription) -> Result<(), RpcError> {
            Ok(())
        }
        async fn progress(&self, _: Progress) -> Result<(), RpcError> {
            Ok(())
        }
    }

    #[async_trait]
    impl Agent for Kept {
        async fn has_instance(&self, instance: String) -> Result<bool, RpcError> {
            Ok(instance == "system")
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
            spaces: vec![SpaceDecl::new("acme.notes").instance("system")],
            hub: vec!["topic:harnix.sessions.status".into()],
        };
        wire.register(registration).await.unwrap();
        let answer = wire
            .call(CallAs {
                app: "acme.notes".into(),
                method: "harnix.send".into(),
                params: serde_json::json!({"n": 1}),
                chain: vec!["acme.notes".into()],
            })
            .await
            .unwrap();
        assert_eq!(answer["params"]["n"], serde_json::json!(1));
        assert_eq!(answer["chain"], serde_json::json!(["acme.notes"]), "the call chain crosses the wire");
        assert!(agent.has_instance("system".into()).await.unwrap());
        assert!(!agent.has_instance("other".into()).await.unwrap());
        let refused = wire.withdraw("acme.notes".into()).await.unwrap_err();
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
        assert_eq!(*kept.0.lock().unwrap(), ["register acme.notes 1", "offer acme.notes 1"]);
        // A name the port does not have, and a value it does not take, are errors, not panics.
        let kernel: &dyn Kernel = kept.as_ref();
        assert_eq!(
            serve_kernel(kernel, "format", Value::Null).await.unwrap_err().code,
            harnix_contract::codes::METHOD_NOT_FOUND
        );
        assert_eq!(
            serve_kernel(kernel, "call", serde_json::json!(3)).await.unwrap_err().code,
            harnix_contract::codes::INVALID_PARAMS
        );
        // Each trait answers its own names only: the agent engine's are not the kernel's.
        assert_eq!(
            serve_kernel(kernel, "has_instance", serde_json::json!("system")).await.unwrap_err().code,
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
