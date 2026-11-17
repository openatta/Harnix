//! A session's model calls, read back out of the engine's recording, and its inside as the engine
//! holds it now: the context its next call will carry, the output of its tasks.
//!
//! What the model was sent and what came back, call by call, summed per turn, what each Agent of
//! a session has used, and the request envelope's changes. The engine writes every call to a
//! recording; its reader (`harnix_ext::engine::readers`) reads it through the engine's protocol, and
//! these methods answer with what it read. The conversation's trajectory view and the right column's
//! usage tab read them for the open session. `context` and `tasks.output` ask the engine itself
//! (`engine.inspect`): the right column's context tab and output tab read them.
//!
//! And a person's few changes to a session's inside, where the product grants `engine.control`:
//! stopping a background task (`tasks.stop`, the output tab's), rewriting one of their messages or
//! taking messages out of the context (`messages.edit`, `messages.drop`, the message menu's). Each
//! is the engine's own operation, asked as this extension once the engine has answered for the
//! session as the caller; the messages only while the history still has the `total` the person
//! saw. Without the grant they are refused, and nothing else changes.
//!
//! A session's rows are a device's for the sessions of the space it claims (`RowScope::Session`):
//! before a recording is read, the session is asked of the engine as the caller, and the hub judges
//! the claim as it does for every read of a session (concept §5.4). `usage` names many
//! sessions at once and stays an open row, as the usage summary reads it; so does
//! `usage.daily`, the same figures per day and model for the summary.
//!
//! And this process's figures (`process`, a method and a topic): its versions, uptime, memory,
//! threads and who is connected, read from the host with `process.observe` where the product grants
//! it — the home page's global statistics; and each extension's log (`extensions.log`), the kernel's
//! `harnix.extensions.log` read with the same grant — the settings' diagnostics.

use std::sync::Arc;
use std::time::Duration;

use harnix_ext::engine::readers::{DayWindow, ReadError, Recordings};
use harnix_ext::{
    catalog, caps, codes, Access, Call, Extension, ExtensionContext, Manifest, Registrar, Rpc, RpcError, RowScope,
    TopicCompute, MAIN_INSTANCE,
};
use serde::Serialize;
use serde_json::{json, Value};

/// How many sessions one `usage` or `usage.daily` call may ask about.
const MAX_SESSIONS: usize = 200;
/// The days `usage.daily` sums when not told, and the most it sums.
const DEFAULT_DAYS: u64 = 30;
const MAX_DAYS: u64 = 366;
/// How often `observability.process` is computed again while watched: memory, threads and
/// connections change without an event to say so.
const PROCESS_EVERY: Duration = Duration::from_secs(5);

/// A person's changes to a session's inside: the method, the engine's operation it asks, the
/// parameters it passes on besides the session, and what it does.
const CHANGES: &[(&str, &str, &[&str], &str)] = &[
    (
        "tasks.stop",
        "engine.session.stopTask",
        &["task_id"],
        "stop one of a session's background tasks: a command with its process group, a sub-agent at its next checkpoint; `{stopped}`, false when it was over",
    ),
    (
        "messages.edit",
        "engine.session.editMessage",
        &["at", "text", "total"],
        "rewrite a message a person wrote in a session (text alone), by its place in the history, if the history still has `total` messages; `{edited}`",
    ),
    (
        "messages.drop",
        "engine.session.dropMessages",
        &["at", "total"],
        "take messages a person wrote, or tool results, out of a session's context from here on, by their places, if the history still has `total` messages; `{dropped}`",
    ),
];

#[derive(Default)]
pub struct Observability;

#[async_trait::async_trait]
impl Extension for Observability {
    type Config = ();

    fn manifest(&self) -> Manifest {
        Manifest::new("observability", env!("CARGO_PKG_VERSION"))
            .title("Observability")
            .disclosure("读本机的模型调用录像：每次调用的请求与应答、每轮的汇总、会话的用量、请求信封的变化；读会话下一次调用的上下文、任务的输出；替人停止会话的后台任务、改写或从上下文去掉人说的话；读本进程的版本、内存、线程与连接，读各扩展的日志")
            .requires([caps::RECORDINGS_READ, caps::ENGINE_INSPECT])
            // This process's figures (`process`), and a person's changes to a session's inside
            // (`tasks.stop`, `messages.*`). Without either, what needs it is refused, and nothing
            // else changes.
            .optional([caps::PROCESS_OBSERVE, caps::ENGINE_CONTROL])
    }

    fn register(&self, r: &mut Registrar<'_, ()>) {
        // A person's rows about one session: for the sessions of the space the call claims.
        let session = || Access::device().scoped(RowScope::Session);
        r.method("trace")
            .access(session())
            .describe("every model call of a session's current run, oldest first, with the tool calls each asked for")
            .raw(|call, params| async move {
                let (reader, session) = recording_of(&call, &params).await?;
                answer(reader.calls(&session).await)
            });
        r.method("trace.call")
            .access(session())
            .describe("one model call in full: the system blocks, the tool catalog, the messages as sent, the answer, and what its tool calls got back")
            .raw(|call, params| async move {
                let (reader, session) = recording_of(&call, &params).await?;
                let index = params
                    .get("index")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| RpcError::invalid_params("`index` is the call's position in the trace"))?;
                answer(reader.call(&session, index as usize).await)
            });
        r.method("turns")
            .access(session())
            .describe("the trace summed up per turn: time to first token, tokens, the cache's share, the context's growth")
            .raw(|call, params| async move {
                let (reader, session) = recording_of(&call, &params).await?;
                answer(reader.turns(&session).await)
            });
        r.method("usage")
            .access(Access::device())
            .describe("what each Agent of each session has used this run (the latest, not a total); a sleeping instance is not woken, and what it read before is marked stale")
            .raw(|call, params| async move { usage(&call, &params).await });
        r.method("usage.daily")
            .access(Access::device())
            .describe("what the tree of Agents under each session used this run, per day and model, over the last `days` days (30; at most 366) at `tz_offset_min` east of UTC; wakes nothing, as usage")
            .raw(|call, params| async move { usage_daily(&call, &params).await });
        r.method("calls")
            .access(session())
            .describe("the request envelope's timeline of a session's current run, one entry per change")
            .raw(|call, params| async move {
                let (reader, session) = recording_of(&call, &params).await?;
                answer(reader.envelopes(&session).await)
            });
        r.method("context")
            .access(session())
            .describe("what a session's next model call will carry, per block, its token counts estimated, with what the provider counted for the last call beside it")
            .raw(|call, params| async move { inspect_context(&call, &params).await });
        r.method("tasks.output")
            .access(session())
            .describe("the output of one of a session's background tasks: the whole, its tail, or from a byte on")
            .raw(|call, params| async move {
                let mut target = session_target(&params)?;
                // The engine's `taskOutput` knows nothing of spaces: the session is asked as the
                // caller first, so one the caller may not see is refused by the hub's own words.
                call.call("engine.session.get", target.clone()).await?;
                for key in ["task_id", "tail_bytes", "from_byte"] {
                    if let Some(value) = params.get(key) {
                        target[key] = value.clone();
                    }
                }
                call.call("engine.session.taskOutput", target).await
            });
        // A person's changes to a session's inside: each the engine's own, passed only what it names.
        for &(name, method, keys, describe) in CHANGES {
            r.method(name)
                .access(session())
                .describe(describe)
                .raw(move |call, params| async move { change(&call, &params, method, keys).await });
        }
        r.method("process")
            .access(Access::device())
            .describe("this process's versions, uptime, memory, threads and who is connected")
            .raw(|call: Call, _| async move { Ok(call.ext().process()?.to_json()) });
        r.topic("process").computed(Some(PROCESS_EVERY), Process);
        r.method("extensions.log")
            .access(Access::device())
            .describe("each extension's log, the newest lines: by extension, least level and after a line")
            .raw(|call: Call, params| async move { call.call("harnix.extensions.log", log_params(&params)).await });
        // Its UI package defines the usage summary's sections, for others to add theirs.
        for point in catalog::points_of("observability") {
            r.ui().point(point.decl());
        }
    }
}

/// What a read of the extension log passes on: its filter, nothing else the caller sent. Lines can
/// say more than a person meant a package to read: the kernel's route is for an extension granted
/// to look into the process, and a device reads it here, through that grant.
fn log_params(params: &Value) -> Value {
    let mut out = json!({});
    for key in ["extension", "level", "after", "limit"] {
        if let Some(value) = params.get(key) {
            out[key] = value.clone();
        }
    }
    out
}

/// `observability.process`, the topic: the same value as the method.
struct Process;

#[async_trait::async_trait]
impl TopicCompute for Process {
    async fn compute(&self, ext: &ExtensionContext, _params: &Value) -> Rpc<Value> {
        Ok(ext.process()?.to_json())
    }
}

/// `{instance?, session_ids}` → `{instance, sessions: {<id>: {recording, agents, run, stale?}}}`;
/// `agents` is the session's own figures first, then each sub-agent's with its parent.
async fn usage(call: &Call, params: &Value) -> Rpc<Value> {
    let instance = instance_of(params);
    let ids = session_ids(params)?;
    // An empty list asks nothing of the instance, whichever it names.
    let out = if ids.is_empty() {
        serde_json::Map::new()
    } else {
        let read = recordings(call, instance)?.usage(&ids).await.map_err(read_error)?;
        read.into_iter()
            .map(|(id, usage)| (id, serde_json::to_value(usage).unwrap_or(Value::Null)))
            .collect()
    };
    Ok(json!({"instance": instance, "sessions": out}))
}

/// `{instance?, session_ids, days?, tz_offset_min?}` → `{instance, days, since, sessions: {<id>:
/// {recording, days: [{day, model, …}], run, stale?}}}`: `since` is the first day counted.
async fn usage_daily(call: &Call, params: &Value) -> Rpc<Value> {
    let instance = instance_of(params);
    let ids = session_ids(params)?;
    let days = match params.get("days") {
        None | Some(Value::Null) => DEFAULT_DAYS,
        Some(v) => v
            .as_u64()
            .filter(|n| (1..=MAX_DAYS).contains(n))
            .ok_or_else(|| RpcError::invalid_params(format!("`days` is from 1 to {MAX_DAYS}")))?,
    };
    let offset = match params.get("tz_offset_min") {
        None | Some(Value::Null) => 0,
        Some(v) => v
            .as_i64()
            .filter(|n| n.abs() <= 14 * 60)
            .ok_or_else(|| RpcError::invalid_params("`tz_offset_min` is minutes east of UTC, at most 14 hours either way"))? as i32,
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let since = window_start(now, days, offset);
    let out = if ids.is_empty() {
        serde_json::Map::new()
    } else {
        let window = DayWindow::new(since, offset);
        let read = recordings(call, instance)?.usage_by_day(&ids, &window).await.map_err(read_error)?;
        read.into_iter()
            .map(|(id, days)| (id, serde_json::to_value(days).unwrap_or(Value::Null)))
            .collect()
    };
    Ok(json!({"instance": instance, "days": days, "since_ms": since, "sessions": out}))
}

/// The start of the first of the last `days` days at `offset` minutes east of UTC, `now` the last.
fn window_start(now: u64, days: u64, offset: i32) -> u64 {
    const DAY: i64 = 86_400_000;
    let shift = i64::from(offset) * 60_000;
    let today = (now as i64 + shift).div_euclid(DAY);
    let first = today - (days as i64 - 1);
    (first * DAY - shift).max(0) as u64
}

/// `session_ids`, at most [`MAX_SESSIONS`] of them.
fn session_ids(params: &Value) -> Rpc<Vec<String>> {
    let ids: Vec<String> = params
        .get("session_ids")
        .and_then(Value::as_array)
        .ok_or_else(|| RpcError::invalid_params("missing session_ids"))?
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    if ids.len() > MAX_SESSIONS {
        return Err(RpcError::invalid_params(format!(
            "at most {MAX_SESSIONS} sessions at once, not {}",
            ids.len()
        )));
    }
    Ok(ids)
}

fn instance_of(params: &Value) -> &str {
    params.get("instance").and_then(Value::as_str).unwrap_or(MAIN_INSTANCE)
}

/// The reader of an instance's recordings.
fn recordings(call: &Call, instance: &str) -> Rpc<Arc<dyn Recordings>> {
    call.ext()
        .engine(instance)
        .ok_or_else(|| RpcError::new(codes::INSTANCE_NOT_FOUND, format!("there is no instance `{instance}`")))?
        .recordings()
}

/// The session a read is about, as the engine is asked for it: `{session_id, instance?, space?}`
/// and nothing else the caller sent — the claim goes with it, for the hub to judge.
fn session_target(params: &Value) -> Rpc<Value> {
    let session = params
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| RpcError::invalid_params("missing session_id"))?;
    let mut target = json!({"session_id": session});
    for key in ["instance", "space"] {
        if let Some(value) = params.get(key).filter(|v| v.is_string()) {
            target[key] = value.clone();
        }
    }
    Ok(target)
}

/// One of [`CHANGES`]: the session asked of the engine as the caller first — the engine's
/// operation knows nothing of spaces, so one the caller may not see is refused by the hub's own
/// words — then the operation, passed the session and `keys` alone. A message is changed only
/// against the history its person saw: `total` is required, and the engine refuses it when the
/// history has changed since.
async fn change(call: &Call, params: &Value, method: &str, keys: &[&str]) -> Rpc<Value> {
    let target = session_target(params)?;
    let ask = change_params(&target, params, keys)?;
    call.call("engine.session.get", target).await?;
    call.call(method, ask).await
}

/// What a change passes on: the session, and the keys it names from what the caller sent.
fn change_params(target: &Value, params: &Value, keys: &[&str]) -> Rpc<Value> {
    if keys.contains(&"total") && !params.get("total").is_some_and(Value::is_u64) {
        return Err(RpcError::invalid_params("`total` is how many messages the history had when the message was seen"));
    }
    let mut out = target.clone();
    for key in keys {
        if let Some(value) = params.get(*key) {
            out[*key] = value.clone();
        }
    }
    Ok(out)
}

/// `context {instance?, session_id, space?}`: the engine's `engine.session.inspectContext` — what the
/// next call will carry, per block, its token counts estimated — with `measured`: what the provider
/// counted for the last call that was made (`engine.session.get`'s `context`, read before the
/// context), absent before the first one. The interface sizes the bar by the measured figure and only divides it by the
/// estimates, so an estimate never passes for a count.
async fn inspect_context(call: &Call, params: &Value) -> Rpc<Value> {
    let target = session_target(params)?;
    // Asked as the caller before anything else: the engine's `inspectContext` knows nothing of
    // spaces, so a session the caller may not see is refused here, by the hub.
    let info = call.call("engine.session.get", target.clone()).await?;
    let mut context = call.call("engine.session.inspectContext", target).await?;
    let measured = info
        .get("context")
        .cloned()
        .filter(|c| c.get("tokens").and_then(Value::as_u64).is_some_and(|n| n > 0));
    if let (Some(measured), Some(obj)) = (measured, context.as_object_mut()) {
        obj.insert("measured".into(), measured);
    }
    Ok(context)
}

/// The reader and the session of `{instance?, session_id, space?}`, once the engine has answered
/// for the session as the caller: a session the caller may not see is refused before its recording
/// is touched, by the hub's own words.
async fn recording_of(call: &Call, params: &Value) -> Rpc<(Arc<dyn Recordings>, String)> {
    let target = session_target(params)?;
    call.call("engine.session.get", target.clone()).await?;
    let session = target["session_id"].as_str().unwrap_or_default().to_string();
    Ok((recordings(call, instance_of(params))?, session))
}

/// A reader's answer as the method's: what it read, or the error a client is told.
fn answer<T: Serialize>(read: Result<T, ReadError>) -> Rpc<Value> {
    let read = read.map_err(read_error)?;
    serde_json::to_value(read).map_err(|e| RpcError::new(codes::INTERNAL_ERROR, e.to_string()))
}

fn read_error(e: ReadError) -> RpcError {
    match e {
        ReadError::Missing(m) => RpcError::missing(m),
        ReadError::Unreadable(m) => RpcError::new(codes::INTERNAL_ERROR, m),
        ReadError::Unavailable(m) => RpcError::new(codes::INSTANCE_UNAVAILABLE, m),
        other => RpcError::new(codes::INTERNAL_ERROR, other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harnix_ext::{ErasedExtension, ExtId};

    /// The rows about one session — reads and changes alike — are a device's, narrowed to the
    /// sessions of the space it claims;
    /// `usage`, which names many sessions, and `process` and `extensions.log`, about none, are a
    /// device's and open.
    #[test]
    fn a_sessions_rows_are_a_devices_for_the_sessions_of_its_space() {
        let id = ExtId::native("observability").unwrap();
        let described = ErasedExtension::register(&Observability, &id, &Value::Null).unwrap();
        let access = |name: &str| {
            described
                .methods()
                .iter()
                .find(|m| m.name() == name)
                .unwrap_or_else(|| panic!("no method {name}"))
                .access()
                .clone()
        };
        for name in [
            "observability.trace",
            "observability.trace.call",
            "observability.turns",
            "observability.calls",
            "observability.context",
            "observability.tasks.output",
            "observability.tasks.stop",
            "observability.messages.edit",
            "observability.messages.drop",
        ] {
            let row = access(name);
            assert!(row.opens_devices(), "{name} is a device's");
            assert!(!row.opens_every_extension(), "{name} is not an extension's");
            assert_eq!(row.scope(), RowScope::Session, "{name} is about one session");
        }
        for name in ["observability.usage", "observability.usage.daily", "observability.process", "observability.extensions.log"] {
            assert!(access(name).opens_devices(), "{name} is a device's");
            assert_eq!(access(name).scope(), RowScope::Open, "{name} is about no one session");
        }
    }

    /// A change passes on the session and what it names, nothing else the caller sent; a message is
    /// changed only against the count of messages its person saw.
    #[test]
    fn a_change_passes_on_only_what_it_names() {
        let sent = json!({"session_id": "S1", "instance": "side", "space": "acme", "task_id": "T", "at": 3, "text": "x", "total": 8, "extra": 1});
        let target = session_target(&sent).unwrap();
        assert_eq!(
            change_params(&target, &sent, &["task_id"]).unwrap(),
            json!({"session_id": "S1", "instance": "side", "space": "acme", "task_id": "T"})
        );
        assert_eq!(
            change_params(&target, &sent, &["at", "text", "total"]).unwrap(),
            json!({"session_id": "S1", "instance": "side", "space": "acme", "at": 3, "text": "x", "total": 8})
        );
        for without in [json!({"session_id": "S1", "at": [3]}), json!({"session_id": "S1", "at": [3], "total": "8"})] {
            let target = session_target(&without).unwrap();
            assert!(change_params(&target, &without, &["at", "total"]).is_err(), "{without}");
        }
        let names: Vec<&str> = CHANGES.iter().map(|(n, ..)| *n).collect();
        assert_eq!(names, ["tasks.stop", "messages.edit", "messages.drop"]);
    }

    /// A read of the extension log passes on its filter and nothing else.
    #[test]
    fn a_log_read_passes_on_only_its_filter() {
        let params = json!({"extension": "flow", "level": "warn", "after": 3, "limit": 10, "space": "acme", "session_id": "S"});
        assert_eq!(log_params(&params), json!({"extension": "flow", "level": "warn", "after": 3, "limit": 10}));
        assert_eq!(log_params(&json!({})), json!({}));
    }

    /// The window starts at midnight where the caller is, `days - 1` days before today there.
    #[test]
    fn the_window_starts_at_the_callers_midnight() {
        let day = 86_400_000;
        // 2025-11-15T23:30Z: an hour east it is already the 16th.
        let now = 20_407 * day + 23 * 3_600_000 + 30 * 60_000;
        assert_eq!(window_start(now, 1, 0), 20_407 * day);
        assert_eq!(window_start(now, 30, 0), (20_407 - 29) * day);
        assert_eq!(window_start(now, 1, 60), 20_408 * day - 3_600_000);
        assert_eq!(window_start(now, 1, -300), 20_407 * day + 5 * 3_600_000);
        assert_eq!(window_start(0, 30, 0), 0, "never before the epoch");
    }

    #[test]
    fn at_most_two_hundred_sessions_are_asked_at_once() {
        let many: Vec<String> = (0..201).map(|i| format!("s{i}")).collect();
        assert!(session_ids(&json!({"session_ids": many})).is_err());
        assert_eq!(session_ids(&json!({"session_ids": ["a", 3, "b"]})).unwrap(), ["a", "b"]);
        assert!(session_ids(&json!({})).is_err());
    }

    /// What the engine is asked about the session: the session, the instance and the space the
    /// caller claimed, nothing else it sent.
    #[test]
    fn the_session_is_asked_of_the_engine_with_the_callers_claim() {
        let target = session_target(&json!({"session_id": "S1", "instance": "side", "space": "acme", "index": 3})).unwrap();
        assert_eq!(target, json!({"session_id": "S1", "instance": "side", "space": "acme"}));
        assert_eq!(session_target(&json!({"session_id": "S1", "space": null})).unwrap(), json!({"session_id": "S1"}));
        assert!(session_target(&json!({"instance": "side"})).is_err());
        assert!(session_target(&json!({"session_id": ""})).is_err());
    }
}
