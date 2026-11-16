//! The scheduler's [`Host`] in Harnix: everything through the extension's context, as itself —
//! sessions through the hub (`harnix.session.*`), tasks through the flow extension
//! (`flow.start`, `flow.get`, open to this extension by name), the engines' own jobs through
//! their rows (`engine.schedule.list`, `delete`, under `engine.inspect` and `engine.control`).

use async_trait::async_trait;
use harnix_ext::{codes, ExtensionContext, OnBusy, RpcError, SessionRef, SessionSpec};
use serde_json::{json, Value};

use crate::core::Host;
use crate::timing::Plan;
use crate::wire::{ModelJob, NewSessionTarget, ScheduleList, SessionKey, TaskTarget, Timing};
use crate::TOPIC;

pub struct Hub {
    pub cx: ExtensionContext,
}

impl Hub {
    /// The space an instance is, when it is one's: a call reaching it claims that space.
    async fn space_of_instance(&self, instance: &str) -> Option<String> {
        let listed = self.cx.call("harnix.instances.list", json!({})).await.ok()?;
        let row = listed.get("instances")?.as_array()?.iter().find(|r| field(r, "id") == Some(instance))?.clone();
        field(&row, "owner").and_then(|o| o.strip_prefix("space:")).map(str::to_string)
    }
}

fn named(space: &str, session: &SessionKey) -> Value {
    json!({"instance": session.instance, "session_id": session.session_id, "space": space})
}

fn field<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

#[async_trait]
impl Host for Hub {
    async fn default_space(&self) -> String {
        let listed = self.cx.call("harnix.spaces.list", json!({})).await.unwrap_or(Value::Null);
        listed
            .get("spaces")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|s| s.get("default_space").and_then(Value::as_bool) == Some(true))
            .and_then(|s| field(s, "id"))
            .unwrap_or_default()
            .to_string()
    }

    async fn session_project(&self, space: &str, session: &SessionKey) -> Result<Option<String>, RpcError> {
        // `engine.session.get` is checked against the space the call claims, as a device's is.
        let got = self.cx.call("engine.session.get", named(space, session)).await?;
        Ok(field(&got, "project_root").map(str::to_string))
    }

    async fn send(&self, space: &str, session: &SessionKey, prompt: &str) -> Result<Option<String>, RpcError> {
        let s = SessionRef::new(session.instance.clone(), session.session_id.clone());
        let sent = self.cx.sessions().in_space(space).send(&s, prompt, OnBusy::Queue).await?;
        Ok(sent.get("item").and_then(|i| field(i, "item_id")).map(str::to_string))
    }

    async fn still_queued(&self, space: &str, session: &SessionKey, item: &str, prompt: &str) -> bool {
        // Editing a queued item to what it says changes nothing, and is refused once it has left
        // the queue.
        let mut params = named(space, session);
        params["item_id"] = json!(item);
        params["message"] = json!(prompt);
        self.cx.call("harnix.session.queue.edit", params).await.is_ok()
    }

    async fn create_session(&self, space: &str, t: &NewSessionTarget) -> Result<SessionKey, RpcError> {
        let mut spec = SessionSpec::new();
        if let Some(root) = &t.project_root {
            spec = spec.project_root(root);
        }
        if let Some(scene) = &t.scene {
            spec = spec.scene(scene);
        }
        if let Some(instance) = &t.instance {
            spec = spec.instance(instance);
        }
        if let Some(provider) = &t.provider {
            spec = spec.provider(provider);
        }
        if let Some(model) = &t.model {
            spec = spec.model(model);
        }
        let s = self.cx.sessions().in_space(space).create(spec).await?;
        Ok(SessionKey { instance: s.instance, session_id: s.session_id })
    }

    async fn rename(&self, space: &str, session: &SessionKey, title: &str) -> Result<(), RpcError> {
        let mut params = named(space, session);
        params["title"] = json!(title);
        self.cx.call("harnix.session.rename", params).await.map(|_| ())
    }

    async fn start_task(&self, t: &TaskTarget, schedule: &str, fire_at: u64) -> Result<String, RpcError> {
        let mut params = json!({
            "flow": t.flow,
            "subject": t.subject,
            "project_root": t.project_root,
            "trigger": {"kind": "schedule", "schedule": schedule, "fire_at": fire_at},
        });
        if !t.inputs.is_null() {
            params["inputs"] = t.inputs.clone();
        }
        for (k, v) in [("title", &t.title), ("output_dir", &t.output_dir), ("workspace", &t.workspace)] {
            if let Some(v) = v {
                params[k] = json!(v);
            }
        }
        let started = self.cx.call("flow.start", params).await?;
        field(&started, "run")
            .map(str::to_string)
            .ok_or_else(|| RpcError::new(codes::INTERNAL_ERROR, format!("flow.start answered no run: {started}")))
    }

    async fn run_active(&self, run: &str) -> Result<bool, RpcError> {
        let got = self.cx.call("flow.get", json!({"run": run})).await?;
        Ok(got.pointer("/record/status").and_then(Value::as_str) == Some("active"))
    }

    async fn model_jobs(&self, space: Option<&str>) -> (Vec<ModelJob>, Vec<String>) {
        let (mut jobs, mut unreachable) = (Vec::new(), Vec::new());
        let listed = self.cx.call("harnix.instances.list", json!({})).await.unwrap_or(Value::Null);
        let instances = listed.get("instances").and_then(Value::as_array).cloned().unwrap_or_default();
        let utc = jiff::tz::TimeZone::UTC;
        let now = crate::core::system_clock()();
        for row in instances {
            let (Some(instance), Some("ready")) = (field(&row, "id"), field(&row, "status")) else { continue };
            // An instance that is a space's answers that space's calls.
            let owner = field(&row, "owner").and_then(|o| o.strip_prefix("space:"));
            let mut ask = json!({"instance": instance});
            if let Some(claim) = owner.or(space) {
                ask["space"] = json!(claim);
            }
            let found = match self.cx.call("engine.schedule.list", ask.clone()).await {
                Ok(found) => found,
                Err(e) => {
                    if e.code != codes::REFUSED && e.code != codes::SPACE_MISMATCH {
                        unreachable.push(instance.to_string());
                    }
                    continue;
                }
            };
            // A space's own: its sessions' jobs only.
            let in_space: Option<Vec<String>> = match space {
                Some(_) => {
                    let sessions = self.cx.call("engine.session.list", ask).await.unwrap_or(Value::Null);
                    Some(
                        sessions
                            .get("sessions")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter_map(|s| field(s, "session_id").map(str::to_string))
                            .collect(),
                    )
                }
                None => None,
            };
            for job in found.get("jobs").and_then(Value::as_array).into_iter().flatten() {
                let (Some(id), Some(session_id), Some(cron)) = (field(job, "id"), field(job, "session_id"), field(job, "cron"))
                else {
                    continue;
                };
                if in_space.as_ref().is_some_and(|ids| !ids.iter().any(|s| s == session_id)) {
                    continue;
                }
                let next_fire = Plan::of(&Timing::Cron { expr: cron.to_string() }, &utc, now)
                    .ok()
                    .and_then(|p| p.next_after(&utc, now));
                jobs.push(ModelJob {
                    instance: instance.to_string(),
                    id: id.to_string(),
                    session_id: session_id.to_string(),
                    cron: cron.to_string(),
                    prompt: field(job, "prompt").unwrap_or_default().to_string(),
                    recurring: job.get("recurring").and_then(Value::as_bool).unwrap_or(true),
                    created_ms: job.get("created_ms").and_then(Value::as_u64),
                    next_fire,
                });
            }
        }
        jobs.sort_by_key(|j| j.next_fire.unwrap_or(u64::MAX));
        (jobs, unreachable)
    }

    async fn delete_model_job(&self, space: Option<&str>, instance: &str, id: &str) -> Result<bool, RpcError> {
        // A call that claims a space deletes that space's sessions' jobs only.
        if let Some(space) = space {
            let (jobs, _) = self.model_jobs(Some(space)).await;
            if !jobs.iter().any(|j| j.instance == instance && j.id == id) {
                return Err(RpcError::new(
                    codes::SPACE_MISMATCH,
                    format!("job `{id}` on instance `{instance}` is not one of the space `{space}`'s sessions'"),
                ));
            }
        }
        let mut ask = json!({"instance": instance, "id": id});
        if let Some(claim) = self.space_of_instance(instance).await.or(space.map(str::to_string)) {
            ask["space"] = json!(claim);
        }
        let done = self.cx.call("engine.schedule.delete", ask).await?;
        Ok(done.get("deleted").and_then(Value::as_bool).unwrap_or(false))
    }

    async fn publish(&self, list: ScheduleList) {
        if let Err(e) = self.cx.topics().publish(TOPIC, json!({}), list).await {
            tracing::warn!("schedule.list not published: {}", e.message);
        }
    }
}
