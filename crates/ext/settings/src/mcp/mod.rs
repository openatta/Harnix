//! MCP servers from the settings page: listed with what the engine says of them, added, changed,
//! removed, switched off and on, reconnected, and signed in to — in the tier the person chooses.
//!
//! The engine's protocol writes a server into a **project** tier only (`engine.mcp.addServer`,
//! `mcp.updateServer`): it refuses an untrusted project, applies its MCP policy, and connects the
//! server at once. Those go through it, with `engine.mcp`. The **global** and **scene** tiers have
//! no engine method, and are written here the way the engine writes a project's — the server's
//! `env` and `headers` into the tier's `settings.local.json`, the rest into its `settings.json`,
//! under the tier's directory lock ([`servers::edit`]) — then the engine re-reads them
//! (`engine.config.reload`), which reconnects that (project, scene)'s servers. Nor has it a way to
//! remove a server or switch one off: removing is the same edit, and a server switched off is kept
//! aside in this extension's data ([`servers::Aside`]) until it is switched on.
//!
//! Nothing secret goes back to the page: a value given for an environment variable or a header is
//! shown as its last four characters at most ([`servers::shown_value`]).

pub(crate) mod auth;
pub(crate) mod context;
pub(crate) mod servers;

use std::collections::BTreeMap;
use std::path::PathBuf;

use harnix_ext::{codes, Access, Call, Registrar, RowScope, Rpc, RpcError};
use serde::Serialize;
use serde_json::{json, Value};

use crate::fields::TIERS;
use crate::wire::{McpAuthStarted, McpList, McpPaths, McpServer, McpStatus};
use servers::{invalid, Aside, Input};

/// Where the servers switched off are kept, in the extension's data.
const ASIDE_FILE: &str = "mcp-aside.json";

/// What the MCP methods keep between calls: the sign-ins under way.
#[derive(Default)]
pub(crate) struct McpState {
    pub(crate) flows: auth::Flows,
}

pub(crate) fn register(r: &mut Registrar<'_, ()>) {
    let device = Access::device;
    r.method("mcp.list")
        .access(device())
        .describe("every MCP server of the tiers there are for this (project, scene), with what the engine says of it")
        .raw(|call, params| async move { answer(list(&call, &At::of(&call, &params)?, Vec::new()).await?) });
    r.method("mcp.add")
        .access(device())
        .describe("add an MCP server to one tier {tier, name, config}; a project's through the engine, which refuses an untrusted one")
        .raw(|call, params| async move { put(&call, params, false).await });
    r.method("mcp.update")
        .access(device())
        .describe("change an MCP server a tier has {tier, name, config}; a value given in secret is kept with `keep`")
        .raw(|call, params| async move { put(&call, params, true).await });
    r.method("mcp.remove")
        .access(device())
        .describe("remove an MCP server from one tier {tier, name}, its secrets with it")
        .raw(|call, params| async move { remove(&call, params).await });
    r.method("mcp.enable")
        .access(device())
        .describe("switch an MCP server of one tier off or on {tier, name, enabled}: off, it is kept aside where the engine does not read it")
        .raw(|call, params| async move { enable(&call, params).await });
    r.method("mcp.reconnect")
        .access(device())
        .describe("make the engine connect this (project, scene)'s MCP servers again {tier, name}")
        .raw(|call, params| async move { reconnect(&call, params).await });
    r.method("mcp.authorize")
        .access(device())
        .describe("start signing in to a remote MCP server {name, origin}: where to send the person, and how the browser comes back")
        .raw(|call, params| async move { authorize(&call, params).await });
    r.method("mcp.authComplete")
        .access(device())
        .describe("hand the engine the address the sign-in's browser came back to {callback_url}, when its route could not")
        .raw(|call, params| async move {
            let callback = require_str(&params, "callback_url")?;
            answer(call.state::<McpState>()?.flows.complete(&call, &callback).await?)
        });
    r.method("mcp.authCancel")
        .access(device())
        .describe("give up a sign-in under way {flow_id}")
        .raw(|call, params| async move {
            let flow_id = require_str(&params, "flow_id")?;
            call.state::<McpState>()?.flows.forget(&flow_id).await;
            call.call("engine.mcp.auth.cancel", json!({"flow_id": flow_id})).await
        });
    r.method("mcp.logout")
        .access(device())
        .describe("sign out of a remote MCP server {name}: the engine revokes its token where it can and forgets it")
        .raw(|call, params| async move { logout(&call, params).await });
    r.method("mcp.context")
        .access(Access::device().scoped(RowScope::Session))
        .describe("the MCP part of a session's next model call {session_id}: each server's tools and its instructions, as the model sees them")
        .raw(|call, params| async move { mcp_context(&call, params).await });
    r.route("mcp", Access::device(), auth::Callback);
}

fn answer<T: Serialize>(value: T) -> Rpc<Value> {
    serde_json::to_value(value).map_err(|e| RpcError::new(codes::INTERNAL_ERROR, e.to_string()))
}

fn require_str(params: &Value, key: &str) -> Rpc<String> {
    crate::require_str(params, key)
}

/// The (project, scene) a call is about.
struct At {
    scene: Option<String>,
    project_root: Option<String>,
}

impl At {
    fn of(call: &Call, params: &Value) -> Rpc<Self> {
        Ok(Self {
            scene: crate::scene_of(call, params)?,
            project_root: params.get("project_root").and_then(Value::as_str).map(str::to_string),
        })
    }

    fn locate(&self) -> Value {
        crate::locate(self.scene.as_deref(), self.project_root.as_deref())
    }

    /// Each tier's settings.json there is for this (project, scene), in [`TIERS`]' order.
    fn paths(&self, call: &Call) -> Rpc<Vec<(&'static str, Option<PathBuf>)>> {
        TIERS.iter().map(|tier| Ok((*tier, crate::tier_path(call, tier, self.scene.as_deref(), self.project_root.as_deref())?))).collect()
    }

    /// The settings.json of the tier a change names.
    fn tier(&self, call: &Call, params: &Value) -> Rpc<(String, PathBuf)> {
        let tier = require_str(params, "tier")?;
        if !TIERS.contains(&tier.as_str()) {
            return Err(invalid(format!("`{tier}` is not a tier: global, scene or project")));
        }
        let path = crate::tier_path(call, &tier, self.scene.as_deref(), self.project_root.as_deref())?
            .ok_or_else(|| invalid(format!("no `{tier}` tier here: the scene tier needs a scene, the project tier a project")))?;
        Ok((tier, path))
    }
}

fn aside(call: &Call) -> Aside {
    Aside::new(call.ext().data_dir().join(ASIDE_FILE))
}

/// What the engine says of each server of its own (project, scene), by name; `None` when it could
/// not be asked.
async fn statuses(call: &Call) -> Option<BTreeMap<String, McpStatus>> {
    let answer = call.call("engine.mcp.status", json!({})).await.ok()?;
    Some(statuses_in(&answer))
}

/// The statuses in an answer that lists servers (`engine.mcp.status`, `engine.mcp.addServer`).
fn statuses_in(answer: &Value) -> BTreeMap<String, McpStatus> {
    answer
        .get("servers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| Some((s.get("name")?.as_str()?.to_string(), servers::status_of(s))))
        .collect()
}

/// Every server of every tier there is, with the statuses the engine gave (`changed`: those a
/// change's answer gave, over the rest).
async fn list(call: &Call, at: &At, changed: Vec<Value>) -> Rpc<McpList> {
    let paths = at.paths(call)?;
    let mut status = statuses(call).await;
    let status_read = status.is_some() || !changed.is_empty();
    for answer in &changed {
        status.get_or_insert_with(BTreeMap::new).extend(statuses_in(answer));
    }
    let aside = aside(call);
    let mut tiers: Vec<(&str, Vec<McpServer>)> = Vec::new();
    for (tier, path) in &paths {
        let Some(path) = path else { continue };
        let written = servers::read(path)?;
        let mut shown: Vec<McpServer> = written.servers().iter().map(|(name, s)| servers::view(name, tier, true, s)).collect();
        shown.extend(aside.of(path)?.iter().filter(|(name, _)| !written.declares(name)).map(|(name, s)| servers::view(name, tier, false, s)));
        shown.sort_by(|a, b| a.name.cmp(&b.name));
        tiers.push((tier, shown));
    }
    Ok(McpList {
        servers: settle(tiers, status.as_ref()),
        paths: McpPaths {
            global: paths[0].1.as_ref().map(|p| p.display().to_string()),
            scene: paths[1].1.as_ref().map(|p| p.display().to_string()),
            project: paths[2].1.as_ref().map(|p| p.display().to_string()),
        },
        status_read,
        written: None,
    })
}

/// The tiers' servers in one list: a server a higher tier also declares is that one's, and the
/// engine's status goes to the one it runs.
fn settle(tiers: Vec<(&str, Vec<McpServer>)>, status: Option<&BTreeMap<String, McpStatus>>) -> Vec<McpServer> {
    let mut above: BTreeMap<String, String> = BTreeMap::new();
    let mut settled: Vec<Vec<McpServer>> = Vec::new();
    for (tier, mut shown) in tiers.into_iter().rev() {
        for server in shown.iter_mut().filter(|s| s.enabled) {
            match above.get(&server.name) {
                Some(higher) => server.overridden_by = Some(higher.clone()),
                None => {
                    server.status = status.and_then(|s| s.get(&server.name)).cloned();
                    above.insert(server.name.clone(), tier.to_string());
                }
            }
        }
        settled.push(shown);
    }
    settled.into_iter().rev().flatten().collect()
}

/// Make the engine read the tiers again for this (project, scene), and for its own when this is
/// another project's — `engine.mcp.status` reports its own.
async fn reload(call: &Call, at: &At) {
    if let Err(e) = call.call("engine.config.reload", at.locate()).await {
        tracing::warn!(error = %e.message, "the engine did not re-read its settings after an MCP change");
    }
    if at.project_root.is_some() {
        let _ = call.call("engine.config.reload", json!({})).await;
    }
}

/// `mcp.add` / `mcp.update`.
async fn put(call: &Call, params: Value, must_exist: bool) -> Rpc<Value> {
    let at = At::of(call, &params)?;
    let (tier, path) = at.tier(call, &params)?;
    let name = require_str(&params, "name")?;
    if !must_exist {
        servers::check_name(&name)?;
    }
    let input: Input = serde_json::from_value(params.get("config").cloned().unwrap_or(Value::Null))
        .map_err(|e| invalid(format!("config: {e}")))?;
    if aside(call).holds(&path, &name)? {
        return Err(invalid(format!("`{name}` is switched off in the {tier} tier: switch it on to change it, or remove it")));
    }
    let before = servers::read(&path)?.servers().remove(&name);
    match (must_exist, before.is_some()) {
        (false, true) => return Err(invalid(format!("the {tier} tier already has a server named `{name}`"))),
        (true, false) => return Err(invalid(format!("the {tier} tier has no server named `{name}`"))),
        _ => {}
    }
    let (public, secret) = servers::build(&input, &tier, before.as_ref())?;
    let mut changed = Vec::new();
    let written = if tier == "project" {
        let mut config = public;
        config.extend(secret);
        let mut request = at.locate();
        request["name"] = json!(name);
        request["config"] = Value::Object(config);
        let method = if must_exist { "engine.mcp.updateServer" } else { "engine.mcp.addServer" };
        let answer = call.call(method, request).await?;
        let written = answer.get("written_to").and_then(Value::as_str).map(str::to_string);
        changed.push(answer);
        written.unwrap_or_else(|| path.display().to_string())
    } else {
        servers::edit(&path, |files| {
            // Read again under the lock: another writer may have been first.
            if files.declares(&name) != must_exist {
                return Err(invalid(format!("the {tier} tier changed meanwhile: read it again")));
            }
            files.put(&name, public, secret);
            Ok(())
        })?;
        reload(call, &at).await;
        path.display().to_string()
    };
    let mut out = list(call, &at, changed).await?;
    out.written = Some(written);
    answer(out)
}

/// `mcp.remove`: out of the tier's two files, or from aside when it was switched off.
async fn remove(call: &Call, params: Value) -> Rpc<Value> {
    let at = At::of(call, &params)?;
    let (tier, path) = at.tier(call, &params)?;
    let name = require_str(&params, "name")?;
    let was_aside = aside(call).take(&path, &name)?.is_some();
    let was_written = servers::read(&path)?.declares(&name)
        && servers::edit(&path, |files| {
            let there = files.declares(&name);
            files.take(&name);
            Ok(there)
        })?;
    if !was_aside && !was_written {
        return Err(invalid(format!("the {tier} tier has no server named `{name}`")));
    }
    if was_written {
        reload(call, &at).await;
    }
    let mut out = list(call, &at, Vec::new()).await?;
    out.written = Some(path.display().to_string());
    answer(out)
}

/// `mcp.enable {enabled}`: off, the server's two halves go aside and out of the tier; on, back.
async fn enable(call: &Call, params: Value) -> Rpc<Value> {
    let at = At::of(call, &params)?;
    let (tier, path) = at.tier(call, &params)?;
    let name = require_str(&params, "name")?;
    let enabled = params.get("enabled").and_then(Value::as_bool).ok_or_else(|| invalid("missing enabled: true or false"))?;
    let aside = aside(call);
    if enabled {
        let Some(kept) = aside.of(&path)?.remove(&name) else {
            if servers::read(&path)?.declares(&name) {
                return answer(list(call, &at, Vec::new()).await?);
            }
            return Err(invalid(format!("the {tier} tier has no server named `{name}`")));
        };
        let (public, secret) = servers::split(&kept);
        // Back into the tier first, then out of aside: a failure in between leaves it in both, and
        // the tier's is the one shown.
        servers::edit(&path, |files| {
            if !files.declares(&name) {
                files.put(&name, public, secret);
            }
            Ok(())
        })?;
        aside.take(&path, &name)?;
    } else {
        if aside.holds(&path, &name)? {
            return answer(list(call, &at, Vec::new()).await?);
        }
        servers::edit(&path, |files| {
            if !files.declares(&name) {
                return Err(invalid(format!("the {tier} tier has no server named `{name}`")));
            }
            let (public, secret) = files.take(&name);
            // Aside before the tier's files are written: a failure in between loses nothing.
            aside.keep(&path, &name, public, secret)
        })?;
    }
    reload(call, &at).await;
    let mut out = list(call, &at, Vec::new()).await?;
    out.written = Some(path.display().to_string());
    answer(out)
}

/// `mcp.reconnect`: the engine connects this (project, scene)'s servers again, this one with them.
async fn reconnect(call: &Call, params: Value) -> Rpc<Value> {
    let at = At::of(call, &params)?;
    let (tier, path) = at.tier(call, &params)?;
    let name = require_str(&params, "name")?;
    if !servers::read(&path)?.declares(&name) {
        return Err(invalid(format!("the {tier} tier has no server named `{name}` switched on")));
    }
    reload(call, &at).await;
    answer(list(call, &at, Vec::new()).await?)
}

/// `mcp.authorize {name, origin?, scopes?}`.
async fn authorize(call: &Call, params: Value) -> Rpc<Value> {
    let at = At::of(call, &params)?;
    let name = require_str(&params, "name")?;
    let redirect = auth::redirect_for(params.get("origin").and_then(Value::as_str), call.ext().id().as_str())?;
    let started = call.call("engine.mcp.auth.start", auth::start_params(&name, &redirect, &at.locate(), params.get("scopes"))).await?;
    let field = |key: &str| started.get(key).and_then(Value::as_str).map(str::to_string);
    let (Some(flow_id), Some(authorize_url)) = (field("flow_id"), field("authorize_url")) else {
        return Err(RpcError::new(codes::INTERNAL_ERROR, "the engine started no sign-in"));
    };
    if !(authorize_url.starts_with("https://") || authorize_url.starts_with("http://")) {
        return Err(RpcError::new(codes::INTERNAL_ERROR, "the engine's sign-in page is not a web page"));
    }
    let expires_in = started.get("expires_in").and_then(Value::as_u64).unwrap_or(600);
    let redirect_uri = field("redirect_uri").unwrap_or_default();
    call.state::<McpState>()?.flows.begin(&flow_id, &name, &authorize_url, &redirect_uri, expires_in).await;
    let external = matches!(redirect, auth::Redirect::External(_));
    answer(McpAuthStarted {
        flow_id,
        authorize_url,
        redirect: if external { "external" } else { "loopback" }.into(),
        callback: external.then(|| format!("/ext/{}{}", call.ext().id(), auth::CALLBACK)),
        expires_in,
    })
}

/// `mcp.logout {name}`.
async fn logout(call: &Call, params: Value) -> Rpc<Value> {
    let at = At::of(call, &params)?;
    let name = require_str(&params, "name")?;
    let mut request = at.locate();
    request["server"] = json!(name);
    call.call("engine.mcp.auth.logout", request).await?;
    answer(list(call, &at, Vec::new()).await?)
}

/// `mcp.context {instance?, session_id, space?}`: checked as the device's like any read of a
/// session, then read off the engine's account of the next call.
async fn mcp_context(call: &Call, params: Value) -> Rpc<Value> {
    let session_id = require_str(&params, "session_id")?;
    let mut target = json!({"session_id": session_id});
    for key in ["instance", "space"] {
        if let Some(value) = params.get(key).filter(|v| !v.is_null()) {
            target[key] = value.clone();
        }
    }
    call.call("engine.session.get", target.clone()).await?;
    let inspect = call.call("engine.session.inspectContext", target).await?;
    let status = statuses(call).await.unwrap_or_default();
    answer(context::context_of(&inspect, &status))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::McpAuthStatus;

    fn server(name: &str, tier: &str, enabled: bool) -> McpServer {
        servers::view(name, tier, enabled, json!({"type": "stdio", "command": "x"}).as_object().unwrap())
    }

    #[test]
    fn a_server_a_higher_tier_also_declares_is_that_ones_and_only_it_has_the_status() {
        let status = McpStatus { state: "connected".into(), tool_count: 3, error: None, limited: None, auth: McpAuthStatus { state: "none".into(), ..Default::default() } };
        let statuses = BTreeMap::from([("fs".to_string(), status.clone()), ("off".to_string(), status.clone())]);
        let settled = settle(
            vec![
                ("global", vec![server("fs", "global", true), server("gh", "global", true)]),
                ("scene", vec![server("off", "scene", false)]),
                ("project", vec![server("fs", "project", true)]),
            ],
            Some(&statuses),
        );
        let row = |name: &str, tier: &str| settled.iter().find(|s| s.name == name && s.tier == tier).unwrap();
        assert_eq!(settled.iter().map(|s| (s.tier.as_str(), s.name.as_str())).collect::<Vec<_>>(), [("global", "fs"), ("global", "gh"), ("scene", "off"), ("project", "fs")]);
        assert_eq!(row("fs", "global").overridden_by.as_deref(), Some("project"));
        assert!(row("fs", "global").status.is_none());
        assert_eq!(row("fs", "project").status.as_ref(), Some(&status));
        assert!(row("gh", "global").status.is_none() && row("gh", "global").overridden_by.is_none());
        // Switched off: nothing of the engine's is its.
        assert!(row("off", "scene").status.is_none());
    }

    #[test]
    fn the_statuses_a_change_answers_with_are_read_by_name() {
        let answer = json!({"written_to": "/p/.harnix/settings.json", "servers": [{"name": "fs", "transport": "stdio", "tool_count": 2, "state": "failed", "error": "no such command", "auth": {"state": "none"}}]});
        let read = statuses_in(&answer);
        assert_eq!(read["fs"].state, "failed");
        assert_eq!(read["fs"].error.as_deref(), Some("no such command"));
    }
}
