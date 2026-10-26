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
//!
//! And an approval (docs/harnix-sdk.md §10.1), built on the public contract alone: a ticket
//! written into the conversation as an entry, the session's pending tickets on a topic, an answer
//! only a person can give (`approve` is for `Device`), the first answer winning, silence past the
//! deadline counting as no, and every decision written down as another entry.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use harnix::axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use harnix::axum::routing::get;
use harnix::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

fn main() -> anyhow::Result<()> {
    harnix::App::builder()
        .name("hello-app", env!("CARGO_PKG_VERSION"))
        .with_builtins()
        .module(Greeter::default())
        .run()
}

#[derive(Default)]
struct Greeter {
    greeted: Arc<AtomicU64>,
    jobs: Arc<AtomicU64>,
    approvals: Arc<Approvals>,
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

        let approvals = self.approvals.clone();
        m.method("request_approval", Principal::Device)
            .describe("asks a person to approve something, in a conversation; no answer by the deadline is a no")
            .schema(json!({"type": "object", "required": ["instance", "session_id", "title"], "properties": {
                "instance": {"type": "string"}, "session_id": {"type": "string"}, "title": {"type": "string"},
                "timeout_ms": {"type": "integer", "minimum": 100}}}))
            .handler(move |cx: CallContext, p: RequestApproval| {
                let approvals = approvals.clone();
                async move {
                    let module = cx.module().clone();
                    let ticket = approvals.open(&p);
                    let session = ticket.session();
                    module
                        .sessions()
                        .append_entry(&session, Entry::new("approval", json!(ticket)))
                        .await?;
                    approvals.publish(&module, &session).await?;
                    let (id, wait) = (ticket.id.clone(), Duration::from_millis(p.timeout_ms));
                    tokio::spawn(async move {
                        tokio::time::sleep(wait).await;
                        // Already answered is the usual case, not an error.
                        let _ = settle(&module, &approvals, &id, Decision::Reject, "timeout").await;
                    });
                    Ok(json!(ticket))
                }
            });

        // A person's answer: `Device` only, so neither the Agent nor a module — this one included —
        // can give it for them.
        let approvals = self.approvals.clone();
        m.method("approve", Principal::Device)
            .describe("a person's answer to an approval; the first answer stands")
            .schema(json!({"type": "object", "required": ["ticket", "decision"], "properties": {
                "ticket": {"type": "string"}, "decision": {"enum": ["approve", "reject"]}}}))
            .handler(move |cx: CallContext, p: Answer| {
                let approvals = approvals.clone();
                async move {
                    let by = cx.device().unwrap_or("?").to_string();
                    settle(cx.module(), &approvals, &p.ticket, p.decision, &by).await
                }
            });

        m.method("answer_for_a_person", Principal::Device)
            .describe("tries to approve as this module, and reports the refusal it gets")
            .handler(|cx: CallContext, p: Value| async move {
                let tried = cx
                    .module()
                    .call("app.greeter.approve", json!({"ticket": p["ticket"], "decision": "approve"}))
                    .await;
                Ok(match tried {
                    Ok(answer) => json!({"answered": answer}),
                    Err(e) => json!({"refused": {"code": e.code, "message": e.message}}),
                })
            });
    }

    fn events(&self, e: &mut EventRegistry) {
        e.topic("greeted");
        e.topic("job").param("job_id");
        e.topic("approvals").param("instance").param("session_id");
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

/* ── approvals ─────────────────────────────────────────────────────────── */

#[derive(Deserialize)]
struct RequestApproval {
    instance: String,
    session_id: String,
    title: String,
    #[serde(default = "a_minute")]
    timeout_ms: u64,
}

fn a_minute() -> u64 {
    60_000
}

#[derive(Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Decision {
    Approve,
    Reject,
}

#[derive(Deserialize)]
struct Answer {
    ticket: String,
    decision: Decision,
}

#[derive(Serialize, Clone)]
struct Ticket {
    id: String,
    instance: String,
    session_id: String,
    title: String,
    /// `pending`, then `approved` or `rejected`, once.
    state: &'static str,
    /// Who decided: a device, or `timeout`.
    by: Option<String>,
    deadline_ms: u64,
}

impl Ticket {
    fn session(&self) -> SessionRef {
        SessionRef {
            instance: self.instance.clone(),
            session_id: self.session_id.clone(),
        }
    }
}

#[derive(Default)]
struct Approvals {
    tickets: Mutex<BTreeMap<String, Ticket>>,
    next: AtomicU64,
}

impl Approvals {
    fn open(&self, p: &RequestApproval) -> Ticket {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
        let ticket = Ticket {
            id: format!("approval-{}", self.next.fetch_add(1, Ordering::SeqCst) + 1),
            instance: p.instance.clone(),
            session_id: p.session_id.clone(),
            title: p.title.clone(),
            state: "pending",
            by: None,
            deadline_ms: now + p.timeout_ms,
        };
        self.tickets.lock().unwrap().insert(ticket.id.clone(), ticket.clone());
        ticket
    }

    /// The one transition out of `pending`. Everything after the first answer is refused.
    fn decide(&self, id: &str, decision: Decision, by: &str) -> Result<Ticket, RpcError> {
        let mut tickets = self.tickets.lock().unwrap();
        let ticket = tickets
            .get_mut(id)
            .ok_or_else(|| RpcError::new(harnix::codes::INVALID_PARAMS, format!("no approval `{id}`")))?;
        if ticket.state != "pending" {
            return Err(RpcError::new(
                harnix::codes::REFUSED,
                format!("`{id}` was already {} by {}", ticket.state, ticket.by.as_deref().unwrap_or("?")),
            ));
        }
        ticket.state = if decision == Decision::Approve { "approved" } else { "rejected" };
        ticket.by = Some(by.to_string());
        Ok(ticket.clone())
    }

    async fn publish(&self, module: &ModuleContext, session: &SessionRef) -> Result<(), RpcError> {
        let tickets: Vec<Ticket> = self
            .tickets
            .lock()
            .unwrap()
            .values()
            .filter(|t| t.instance == session.instance && t.session_id == session.session_id)
            .cloned()
            .collect();
        module
            .events()
            .publish(
                "approvals",
                json!({"instance": session.instance, "session_id": session.session_id}),
                json!({"tickets": tickets}),
            )
            .await
    }
}

/// Decide, write the decision into the conversation, and tell whoever is watching.
async fn settle(module: &ModuleContext, approvals: &Approvals, id: &str, decision: Decision, by: &str) -> Result<Value, RpcError> {
    let ticket = approvals.decide(id, decision, by)?;
    let session = ticket.session();
    module
        .sessions()
        .append_entry(&session, Entry::new("approval_decided", json!({"ticket": ticket.id, "state": ticket.state, "by": by})))
        .await?;
    approvals.publish(module, &session).await?;
    Ok(json!(ticket))
}
