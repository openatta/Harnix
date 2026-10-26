//! hello-app: HOME and two spaces, built the way an application on Harnix is built.
//!
//! | space | interface | backend | Agent |
//! |---|---|---|---|
//! | `greeter` | a tab and a HOME card (`ui/`) | the `greeter` module below | its own in-process instance, from the module's template |
//! | `notes` | a space (`ui/`) | — | the `system` instance |
//!
//! The greeter also shows the two channels for what a call and a topic do not fit
//! (docs/harnix-sdk.md §9): a long job whose progress is a topic with a parameter
//! (`count_to`, `app.greeter.job { job_id }`), and the module's own link under
//! `/apps/greeter/` — an HTTP route and a WebSocket.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use harnix::axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use harnix::axum::routing::get;
use harnix::prelude::*;
use serde::Deserialize;
use serde_json::json;

fn main() -> anyhow::Result<()> {
    harnix::App::builder()
        .name("hello-app", env!("CARGO_PKG_VERSION"))
        .with_builtins()
        .module(Greeter::default())
        .run()
}

#[derive(Default)]
struct Greeter {
    greeted: std::sync::Arc<AtomicU64>,
    jobs: std::sync::Arc<AtomicU64>,
}

#[derive(Deserialize)]
struct Greet {
    name: String,
}

#[derive(Deserialize)]
struct CountTo {
    to: u32,
    /// The conversation it was started from, if any: the result goes there as a card.
    #[serde(default)]
    instance: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
}

#[harnix::async_trait]
impl Module for Greeter {
    const API_VERSION: u32 = harnix::MODULE_API_VERSION;

    fn manifest(&self) -> Manifest {
        Manifest::new("greeter", "0.1.0").disclosure("greets people, and runs its sessions on an instance of its own")
    }

    fn templates(&self, t: &mut InstanceTemplates) {
        t.add(InstanceTemplate::new(
            "desk",
            TemplateCarrier::InProcess {
                engine: EngineProfile {
                    scene: "chat".into(),
                    ..EngineProfile::default()
                },
                contrib: EngineContrib::new(),
            },
        ));
    }

    fn spaces(&self, s: &mut SpaceRegistry) {
        s.add(SpaceDecl::new("greeter").instance("template:greeter.desk").scene("chat"));
    }

    fn methods(&self, m: &mut MethodRegistry) {
        let greeted = self.greeted.clone();
        m.method("greet", Principal::Device)
            .describe("says hello, and counts")
            .schema(json!({"type": "object", "required": ["name"], "properties": {"name": {"type": "string"}}}))
            .handler(move |cx: CallContext, p: Greet| {
                let greeted = greeted.clone();
                async move {
                    let n = greeted.fetch_add(1, Ordering::SeqCst) + 1;
                    cx.module().events().publish("greeted", json!({}), json!({"count": n})).await?;
                    Ok(json!({"greeting": format!("hello, {}", p.name), "count": n}))
                }
            });

        let jobs = self.jobs.clone();
        m.method("count_to", Principal::Device)
            .describe("counts to `to`, slowly: a long job, answered with its id, its progress on `app.greeter.job`")
            .schema(json!({"type": "object", "required": ["to"], "properties": {"to": {"type": "integer", "minimum": 1, "maximum": 100}}}))
            .handler(move |cx: CallContext, p: CountTo| {
                let jobs = jobs.clone();
                async move {
                    let job_id = format!("job-{}", jobs.fetch_add(1, Ordering::SeqCst) + 1);
                    let events = cx.module().events().clone();
                    let key = json!({"job_id": job_id});
                    let state = |done: u32| json!({"job_id": job_id, "done": done, "total": p.to, "finished": done == p.to});
                    // The first value is there before the id is handed out: whoever subscribes
                    // next reads it, however late.
                    events.publish("job", key.clone(), state(0)).await?;
                    let states: Vec<_> = (1..=p.to).map(state).collect();
                    let module = cx.module().clone();
                    let card = match (p.instance, p.session_id) {
                        (Some(instance), Some(session_id)) => Some(SessionRef { instance, session_id }),
                        _ => None,
                    };
                    let done = json!({"job_id": job_id, "total": p.to});
                    tokio::spawn(async move {
                        for value in states {
                            tokio::time::sleep(Duration::from_millis(40)).await;
                            let _ = events.publish("job", key.clone(), value).await;
                        }
                        if let Some(session) = card {
                            let _ = module.sessions().append_entry(&session, Entry::new("job_done", done)).await;
                        }
                    });
                    Ok(json!({"job_id": job_id}))
                }
            });
    }

    fn events(&self, e: &mut EventRegistry) {
        e.topic("greeted");
        e.topic("job").param("job_id");
    }

    fn routes(&self, r: &mut RouteRegistry) {
        let router = harnix::axum::Router::new()
            .route("/hello", get(|| async { "hello from the greeter's own route" }))
            .route("/echo", get(|ws: WebSocketUpgrade| async { ws.on_upgrade(echo) }));
        r.serve(&[Principal::Device], router);
    }

    async fn start(&self, cx: ModuleContext) -> anyhow::Result<()> {
        cx.events().publish("greeted", json!({}), json!({"count": 0})).await?;
        Ok(())
    }
}


async fn echo(mut socket: WebSocket) {
    while let Some(Ok(message)) = socket.recv().await {
        if let Message::Text(text) = message {
            if socket.send(Message::Text(format!("echo: {}", text.as_str()).into())).await.is_err() {
                return;
            }
        }
    }
}
