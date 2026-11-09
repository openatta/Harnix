//! The host's [`Kernel`] and [`Agent`], over this process's kernel and its Agix: what an
//! application embedded here is lent.
//!
//! - **Its names** are registered with the kernel's application host
//!   (`harnix_module::AppHost`), under the `apps` module: each `[[method]]` a row in the
//!   authorization table with the principals its manifest names (and always the kernel, and the
//!   application itself), answered by [`Control::call_method`]; its topics and spaces beside them.
//! - **What it does as itself** — `host.call`, `publish`, `subscribe` — goes through that
//!   registration, as `module:<id>`: the kernel admits, grants and audits. The values of what it
//!   subscribed to go back to the host ([`Control::deliver`]), in order.
//! - **Its agent side** is this process's Agix: its agent package installed into the instances its
//!   manifest names (`plugin.install` and friends), its skills copied into each one's skills
//!   directory, marked as its own, and its tools in every session built here ([`crate::AppsPlugin`]
//!   reads what [`Agent::expose_tools`] offered).
//!
//! The kernel is reached through the `apps` module's context, which every call of the
//! `AppsRuntime` hands in; nothing here reaches it before the module has.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock, Weak};

use async_trait::async_trait;
use harnix_app_package::acquire::{internal, refused};
use harnix_app_package::package;
use harnix_app_port::{
    Agent, AgentInstall, AgentRef, AgentSwitch, CallAs, Control, Kernel, MethodCall, Offer,
    Progress, Publish, Registration, Skills, Subscription, TopicValue,
};
use harnix_contract::{codes, RpcError, Subject};
use harnix_module::{
    AppHandle, AppHandler, AppMethod, AppTopic, AppTopicSink, ModuleContext, Principal,
};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::runtime::Registry;

/// What marks a skill directory as an application's, inside an instance's skills directory.
const SKILL_MARK: &str = ".harnix-app";

fn read<T>(l: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(|e| e.into_inner())
}

fn write<T>(l: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    l.write().unwrap_or_else(|e| e.into_inner())
}

pub struct KernelPlatform {
    /// The `apps` module's context, from the first call it made.
    cx: RwLock<Option<ModuleContext>>,
    /// The host this lends to: what the kernel's handlers and topic forwarders call. Weak, so the
    /// kernel holding a handler does not hold the host.
    control: OnceLock<Weak<dyn Control>>,
    /// Each registered application's registration.
    handles: RwLock<BTreeMap<String, AppHandle>>,
    registry: Arc<Registry>,
}

impl KernelPlatform {
    pub fn new(registry: Arc<Registry>) -> Self {
        Self {
            cx: RwLock::new(None),
            control: OnceLock::new(),
            handles: RwLock::new(BTreeMap::new()),
            registry,
        }
    }

    pub fn lend_to(&self, control: Weak<dyn Control>) {
        let _ = self.control.set(control);
    }

    /// The `apps` module's context: every call it makes into the runtime hands it in.
    pub fn bind(&self, cx: &ModuleContext) {
        let mut slot = write(&self.cx);
        if slot.is_none() {
            *slot = Some(cx.clone());
        }
    }

    fn cx(&self) -> Result<ModuleContext, RpcError> {
        read(&self.cx)
            .clone()
            .ok_or_else(|| internal("the `apps` module has not reached the application runtime yet"))
    }

    fn handle(&self, app: &str) -> Result<AppHandle, RpcError> {
        read(&self.handles).get(app).cloned().ok_or_else(|| {
            RpcError::new(
                codes::INTERNAL_ERROR,
                format!("`{app}` is not registered with this process's application host"),
            )
        })
    }

    fn skills_dir(cx: &ModuleContext, instance: &str) -> Result<PathBuf, RpcError> {
        cx.instance(instance)
            .map(|h| h.data_root.join("skills"))
            .ok_or_else(|| refused(format!("there is no instance `{instance}`")))
    }
}

/// One `[[method]]` as the kernel has it: answered by the host.
struct Forward {
    app: String,
    name: String,
    control: Weak<dyn Control>,
}

#[async_trait]
impl AppHandler for Forward {
    async fn call(&self, caller: &Subject, params: Value) -> Result<Value, RpcError> {
        let Some(control) = self.control.upgrade() else {
            let mut e = RpcError::new(
                codes::INTERNAL_ERROR,
                format!("`app.{}.{}`: the application is not running", self.app, self.name),
            );
            e.data = Some(json!({"app": self.app, "failure": "stopped"}));
            return Err(e);
        };
        control
            .call_method(MethodCall {
                app: self.app.clone(),
                method: self.name.clone(),
                caller: caller.clone(),
                params,
            })
            .await
    }
}

/// Values of an application's subscriptions waiting for the host. Past this the application is not
/// keeping up with what it subscribed to, and dropping beats growing without bound; the host's own
/// queue counts what it drops after this one.
const TOPIC_QUEUE: usize = 256;

/// The kernel's frames for an application's subscriptions, to the host in the order they came.
struct TopicValues {
    app: String,
    tx: mpsc::Sender<(String, Value)>,
}

impl TopicValues {
    fn new(app: &str, control: Weak<dyn Control>) -> Self {
        let (tx, mut rx) = mpsc::channel::<(String, Value)>(TOPIC_QUEUE);
        let name = app.to_string();
        let app = app.to_string();
        tokio::spawn(async move {
            while let Some((topic, value)) = rx.recv().await {
                let Some(control) = control.upgrade() else { break };
                let value = TopicValue {
                    app: app.clone(),
                    topic,
                    value,
                };
                let _ = control.deliver(value).await;
            }
        });
        Self { app: name, tx }
    }
}

impl AppTopicSink for TopicValues {
    fn deliver(&self, topic: &str, _params: &Value, value: &Value) {
        if let Err(mpsc::error::TrySendError::Full(_)) =
            self.tx.try_send((topic.to_string(), value.clone()))
        {
            tracing::warn!(app = %self.app, %topic, "an application's topic queue is full; a value dropped");
        }
    }

    fn ended(&self, topic: &str, _params: &Value, reason: &str) {
        tracing::info!(%topic, %reason, "a topic an application subscribed to ended");
    }
}

/// Who may call one of an application's methods: the subjects it names, and always the kernel. A
/// name that is none of them is refused rather than read as one: the manifest checks it, but a
/// registration may come from over a link, and an unknown subject must not open a method to
/// anybody.
fn principals(app: &str, method: &str, subjects: &[String]) -> Result<Vec<Principal>, RpcError> {
    let mut principals = Vec::new();
    for subject in subjects {
        principals.push(match subject.as_str() {
            "device" => Principal::Device,
            "module" => Principal::AnyModule,
            "kernel" => Principal::Kernel,
            other => {
                return Err(refused(format!(
                    "`app.{app}.{method}` names the subject `{other}`, and a method's subject is \
                     `device`, `module` or `kernel`"
                )))
            }
        });
    }
    principals.push(Principal::Kernel);
    principals.dedup();
    Ok(principals)
}

#[async_trait]
impl Kernel for KernelPlatform {
    async fn register(&self, r: Registration) -> Result<(), RpcError> {
        // Before anything is registered, and whether or not there is a host to register with.
        for m in &r.methods {
            principals(&r.app, &m.name, &m.subjects)?;
        }
        let cx = self.cx()?;
        // A process without an application host: nothing is declared, and what the application
        // does as itself is refused as not registered.
        let Some(host) = cx.apps() else {
            return Ok(());
        };
        let control = self
            .control
            .get()
            .cloned()
            .ok_or_else(|| internal("this platform is lent to no application host"))?;
        let mut spec = harnix_module::AppSpec::new(r.app.clone())
            .hub(r.hub.clone())
            .topic_sink(Arc::new(TopicValues::new(&r.app, control.clone())));
        for m in r.methods {
            let principals = principals(&r.app, &m.name, &m.subjects)?;
            spec = spec.method_with(AppMethod {
                name: m.name.clone(),
                principals,
                description: m.description,
                handler: Arc::new(Forward {
                    app: r.app.clone(),
                    name: m.name,
                    control: control.clone(),
                }),
            });
        }
        for t in r.topics {
            spec = spec.topic_with(AppTopic {
                name: t,
                params: Vec::new(),
                principals: vec![Principal::Device, Principal::AnyModule],
            });
        }
        for space in r.spaces {
            spec = spec.space(space);
        }
        let handle = host.register(spec).await?;
        write(&self.handles).insert(r.app, handle);
        Ok(())
    }

    async fn withdraw(&self, app: String) -> Result<(), RpcError> {
        let handle = write(&self.handles).remove(&app);
        match handle {
            Some(handle) => handle.withdraw().await.map(|_| ()),
            None => Err(RpcError::invalid_params(format!("`{app}` is not registered"))),
        }
    }

    async fn call(&self, call: CallAs) -> Result<Value, RpcError> {
        self.handle(&call.app)?.call(&call.method, call.params).await
    }

    async fn publish(&self, p: Publish) -> Result<(), RpcError> {
        self.handle(&p.app)?
            .publish(&p.topic, json!({}), p.value)
            .await
    }

    async fn subscribe(&self, s: Subscription) -> Result<Value, RpcError> {
        self.handle(&s.app)?.subscribe(&s.topic, s.params).await
    }

    async fn unsubscribe(&self, s: Subscription) -> Result<(), RpcError> {
        self.handle(&s.app)?.unsubscribe(&s.topic, s.params).await
    }

    async fn progress(&self, p: Progress) -> Result<(), RpcError> {
        self.registry.told(&p.call_id, &p.text);
        Ok(())
    }
}

#[async_trait]
impl Agent for KernelPlatform {

    async fn has_instance(&self, instance: String) -> Result<bool, RpcError> {
        Ok(self.cx()?.instance(&instance).is_some())
    }

    async fn install_agent(&self, a: AgentInstall) -> Result<Value, RpcError> {
        let params = json!({
            "name": a.name,
            "version": a.version,
            "download_url": format!("file://{}", a.archive.display()),
            "instance": a.instance,
        });
        let answer = self.cx()?.call("plugin.install", params).await?;
        Ok(answer.get("disclosure").cloned().unwrap_or(answer))
    }

    async fn uninstall_agent(&self, a: AgentRef) -> Result<(), RpcError> {
        let params = json!({"name": a.name, "version": a.version, "instance": a.instance});
        self.cx()?.call("plugin.uninstall", params).await.map(|_| ())
    }

    async fn switch_agent(&self, s: AgentSwitch) -> Result<(), RpcError> {
        let method = if s.on { "plugin.enable" } else { "plugin.disable" };
        self.cx()?
            .call(method, json!({"name": s.name, "instance": s.instance}))
            .await
            .map(|_| ())
    }

    /// Copy `<source>/<name>/` into the instance's skills directory, marked as the application's.
    /// A directory of that name that is not the application's is somebody else's skill, and
    /// refuses it.
    async fn place_skills(&self, s: Skills) -> Result<(), RpcError> {
        let (id, instance) = (&s.app, &s.instance);
        let dir = Self::skills_dir(&self.cx()?, instance)?;
        for name in &s.names {
            let target = dir.join(name);
            if target.exists()
                && std::fs::read_to_string(target.join(SKILL_MARK))
                    .ok()
                    .as_deref()
                    != Some(id.as_str())
            {
                return Err(refused(format!(
                    "the instance `{instance}` has a skill `{name}` of its own; `{id}` would replace it"
                )));
            }
            let _ = std::fs::remove_dir_all(&target);
            let source = s.source.join(name);
            let mut files = Vec::new();
            package::walk(&source, &source, &mut files)
                .map_err(|e| internal(format!("{}: {e}", source.display())))?;
            for (relative, path) in files {
                let to = target.join(&relative);
                if let Some(parent) = to.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| internal(format!("{}: {e}", parent.display())))?;
                }
                std::fs::copy(&path, &to).map_err(|e| internal(format!("{}: {e}", to.display())))?;
            }
            std::fs::write(target.join(SKILL_MARK), id).map_err(|e| internal(e.to_string()))?;
        }
        Ok(())
    }

    /// Remove the skills marked as the application's, and nothing else.
    async fn remove_skills(&self, s: Skills) -> Result<(), RpcError> {
        let Ok(dir) = Self::skills_dir(&self.cx()?, &s.instance) else {
            return Ok(());
        };
        for name in &s.names {
            let target = dir.join(name);
            if std::fs::read_to_string(target.join(SKILL_MARK))
                .ok()
                .as_deref()
                == Some(s.app.as_str())
            {
                if let Err(e) = std::fs::remove_dir_all(&target) {
                    tracing::warn!(app = %s.app, skill = %name, error = %e, "a skill was not removed");
                }
            }
        }
        Ok(())
    }

    async fn reload_instance(&self, instance: String) -> Result<(), RpcError> {
        self.cx()?
            .call("config.reload", json!({"instance": instance}))
            .await
            .map(|_| ())
    }

    async fn expose_tools(&self, offer: Offer) -> Result<(), RpcError> {
        self.registry.offer(offer);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_subject_is_one_of_three_or_the_registration_is_refused() {
        let named = |subjects: &[&str]| {
            principals("acme.notes", "save", &subjects.iter().map(|s| s.to_string()).collect::<Vec<_>>())
        };
        assert_eq!(named(&["device"]).unwrap(), [Principal::Device, Principal::Kernel]);
        assert_eq!(
            named(&["module", "kernel"]).unwrap(),
            [Principal::AnyModule, Principal::Kernel]
        );
        assert_eq!(named(&[]).unwrap(), [Principal::Kernel]);
        let err = named(&["device", "everyone"]).unwrap_err();
        assert_eq!(err.code, codes::REFUSED);
        assert!(err.message.contains("`app.acme.notes.save` names the subject `everyone`"), "{}", err.message);
    }
}
