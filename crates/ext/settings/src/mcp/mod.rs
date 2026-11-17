//! MCP servers from the settings page: listed with what the engine says of them, added, changed,
//! removed, switched off and on, reconnected, and signed in to — in the tier the person chooses.
//!
//! Every one through the engine, holding `engine.mcp`: `engine.mcp.addServer`, `updateServer`,
//! `removeServer` and `setEnabled` write the tier they are told (global, scene or project) under its
//! directory lock — the server's `env` and `headers` into its `settings.local.json`, the rest into
//! its `settings.json`, a switched-off server as `mcp_enabled.<name>: false` beside it — and settle
//! that one server's connection; `engine.mcp.reconnect` connects one again; `engine.mcp.status`
//! says what each is doing for the (project, scene) asked about, without starting a pair that has
//! not connected yet. A tier's servers are read with `engine.config.get`.
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
use serde_json::{json, Map, Value};

use crate::fields::TIERS;
use crate::wire::{McpAuthStarted, McpList, McpPaths, McpServer, McpStatus};
use servers::{invalid, Input};

/// The engine's code for a `project_root` that is not a directory (daemon protocol §9).
const PROJECT_NOT_FOUND: i32 = -32010;

/// What the MCP methods keep between calls: the sign-ins under way.
#[derive(Default)]
pub(crate) struct McpState {
    pub(crate) flows: auth::Flows,
}

pub(crate) fn register(r: &mut Registrar<'_, ()>) {
    let device = Access::device;
    r.method("mcp.list")
        .access(device())
        .describe("every MCP server of the tiers there are for this (project, scene), with what the engine says of it there")
        .raw(|call, params| async move { answer(list(&call, &At::of(&call, &params)?, Vec::new()).await?) });
    r.method("mcp.add")
        .access(device())
        .describe("add an MCP server to one tier {tier, name, config}, through the engine, which refuses an untrusted project's")
        .raw(|call, params| async move { put(&call, params, false).await });
    r.method("mcp.update")
        .access(device())
        .describe("change an MCP server a tier has {tier, name, config}; a value given in secret is kept with `keep`")
        .raw(|call, params| async move { put(&call, params, true).await });
    r.method("mcp.remove")
        .access(device())
        .describe("remove an MCP server from one tier {tier, name}, its secrets and its switch with it")
        .raw(|call, params| async move { remove(&call, params).await });
    r.method("mcp.enable")
        .access(device())
        .describe("switch an MCP server off or on in one tier {tier, name, enabled}: off, the engine does not start it")
        .raw(|call, params| async move { enable(&call, params).await });
    r.method("mcp.reconnect")
        .access(device())
        .describe("make the engine connect one MCP server of this (project, scene) again {name}, the others left as they are")
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

    /// Each tier's settings.json there is for this (project, scene), in [`TIERS`]' order: where the
    /// page says a tier's servers are declared.
    fn paths(&self, call: &Call) -> Rpc<Vec<(&'static str, Option<PathBuf>)>> {
        TIERS.iter().map(|tier| Ok((*tier, crate::tier_path(call, tier, self.scene.as_deref(), self.project_root.as_deref())?))).collect()
    }

    /// The tier a change names, when there is one here.
    fn tier(&self, params: &Value) -> Rpc<String> {
        let tier = require_str(params, "tier")?;
        if !TIERS.contains(&tier.as_str()) {
            return Err(invalid(format!("`{tier}` is not a tier: global, scene or project")));
        }
        if (tier == "scene" && self.scene.is_none()) || (tier == "project" && self.project_root.is_none()) {
            return Err(invalid(format!("no `{tier}` tier here: the scene tier needs a scene, the project tier a project")));
        }
        Ok(tier)
    }

    /// An engine call about one server of this (project, scene), into `tier` when it writes one.
    fn request(&self, name: &str, tier: Option<&str>) -> Value {
        let mut request = self.locate();
        request["name"] = json!(name);
        if let Some(tier) = tier {
            request["tier"] = json!(tier);
        }
        request
    }
}

/// The servers one tier declares, as the engine reads them, secrets and all — they stay here: the
/// page is shown each as [`servers::view`] masks it. `None`: the project the tier is in is not
/// there.
async fn declared(call: &Call, at: &At, tier: &str) -> Rpc<Option<BTreeMap<String, Map<String, Value>>>> {
    let mut request = at.locate();
    request["tier"] = json!(tier);
    request["include_secrets"] = json!(true);
    match call.call("engine.config.get", request).await {
        Ok(read) => Ok(Some(servers::declared(read.get("settings"), read.get("local")))),
        Err(e) if e.code == PROJECT_NOT_FOUND => Ok(None),
        Err(e) => Err(e),
    }
}

/// Which servers are switched on for this (project, scene): `mcp_enabled` as the engine merged it
/// over the tiers, by name; a name it does not say is on.
async fn switches(call: &Call, at: &At) -> Map<String, Value> {
    let mut request = at.locate();
    request["tier"] = json!("effective");
    call.call("engine.config.get", request)
        .await
        .ok()
        .and_then(|read| read.get("settings")?.get("mcp_enabled")?.as_object().cloned())
        .unwrap_or_default()
}

/// What the engine says of each server of a (project, scene) — `located` says which — by name,
/// and whether that pair's servers have been connected; `None` when it could not be asked.
async fn statuses(call: &Call, located: Value) -> Option<(bool, BTreeMap<String, McpStatus>)> {
    let answer = call.call("engine.mcp.status", located).await.ok()?;
    let loaded = answer.get("loaded").and_then(Value::as_bool).unwrap_or(true);
    Some((loaded, statuses_in(&answer)))
}

/// The statuses in an answer that lists servers (`engine.mcp.status`, a change's answer).
fn statuses_in(answer: &Value) -> BTreeMap<String, McpStatus> {
    answer
        .get("servers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| Some((s.get("name")?.as_str()?.to_string(), servers::status_of(s))))
        .collect()
}

/// Every server of every tier there is, with the statuses the engine gave for this (project,
/// scene) (`changed`: those a change's answer gave, over the rest).
async fn list(call: &Call, at: &At, changed: Vec<Value>) -> Rpc<McpList> {
    let paths = at.paths(call)?;
    let status = statuses(call, at.locate()).await;
    let loaded = status.as_ref().map(|(loaded, _)| *loaded);
    let mut status = status.map(|(_, by_name)| by_name);
    let status_read = status.is_some() || !changed.is_empty();
    for answer in &changed {
        status.get_or_insert_with(BTreeMap::new).extend(statuses_in(answer));
    }
    let switches = switches(call, at).await;
    let mut tiers: Vec<(&str, Vec<McpServer>)> = Vec::new();
    for (tier, path) in &paths {
        if path.is_none() {
            continue;
        }
        let Some(written) = declared(call, at, tier).await? else { continue };
        let on = |name: &str| switches.get(name).and_then(Value::as_bool).unwrap_or(true);
        tiers.push((tier, written.iter().map(|(name, s)| servers::view(name, tier, on(name), s)).collect()));
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
        loaded,
        scene: at.scene.clone(),
        project_root: at.project_root.clone(),
    })
}

/// The tiers' servers in one list: a server a higher tier also declares is that one's, and the
/// engine's status goes to the one it runs.
fn settle(tiers: Vec<(&str, Vec<McpServer>)>, status: Option<&BTreeMap<String, McpStatus>>) -> Vec<McpServer> {
    let mut above: BTreeMap<String, String> = BTreeMap::new();
    let mut settled: Vec<Vec<McpServer>> = Vec::new();
    for (tier, mut shown) in tiers.into_iter().rev() {
        for server in shown.iter_mut() {
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

/// A change's answer as the page reads it: the servers as they stand after it, with the file it
/// wrote.
async fn changed(call: &Call, at: &At, done: Value) -> Rpc<Value> {
    let written = done.get("written_to").and_then(Value::as_str).map(str::to_string);
    let mut out = list(call, at, vec![done]).await?;
    out.written = written;
    answer(out)
}

/// `mcp.add` / `mcp.update`.
async fn put(call: &Call, params: Value, must_exist: bool) -> Rpc<Value> {
    let at = At::of(call, &params)?;
    let tier = at.tier(&params)?;
    let name = require_str(&params, "name")?;
    if !must_exist {
        servers::check_name(&name)?;
    }
    let input: Input = serde_json::from_value(params.get("config").cloned().unwrap_or(Value::Null))
        .map_err(|e| invalid(format!("config: {e}")))?;
    let before = declared(call, &at, &tier).await?.unwrap_or_default().remove(&name);
    match (must_exist, before.is_some()) {
        (false, true) => return Err(invalid(format!("the {tier} tier already has a server named `{name}`"))),
        (true, false) => return Err(invalid(format!("the {tier} tier has no server named `{name}`"))),
        _ => {}
    }
    let mut request = at.request(&name, Some(&tier));
    request["config"] = Value::Object(servers::build(&input, &tier, before.as_ref())?);
    let method = if must_exist { "engine.mcp.updateServer" } else { "engine.mcp.addServer" };
    let done = call.call(method, request).await?;
    changed(call, &at, done).await
}

/// `mcp.remove`: out of the tier's two files, with its switch.
async fn remove(call: &Call, params: Value) -> Rpc<Value> {
    let at = At::of(call, &params)?;
    let tier = at.tier(&params)?;
    let name = require_str(&params, "name")?;
    let done = call.call("engine.mcp.removeServer", at.request(&name, Some(&tier))).await?;
    changed(call, &at, done).await
}

/// `mcp.enable {enabled}`: the server's switch in the tier, `mcp_enabled.<name>`.
async fn enable(call: &Call, params: Value) -> Rpc<Value> {
    let at = At::of(call, &params)?;
    let tier = at.tier(&params)?;
    let name = require_str(&params, "name")?;
    let enabled = params.get("enabled").and_then(Value::as_bool).ok_or_else(|| invalid("missing enabled: true or false"))?;
    let mut request = at.request(&name, Some(&tier));
    request["enabled"] = json!(enabled);
    let done = call.call("engine.mcp.setEnabled", request).await?;
    changed(call, &at, done).await
}

/// `mcp.reconnect`: the engine connects this one server of this (project, scene) again, by its
/// settings as they are now; the others are left as they are.
async fn reconnect(call: &Call, params: Value) -> Rpc<Value> {
    let at = At::of(call, &params)?;
    let name = require_str(&params, "name")?;
    let done = call.call("engine.mcp.reconnect", at.request(&name, None)).await?;
    let servers: Vec<Value> = done.get("server").cloned().into_iter().collect();
    answer(list(call, &at, vec![json!({"servers": servers})]).await?)
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
    let session = call.call("engine.session.get", target.clone()).await?;
    let inspect = call.call("engine.session.inspectContext", target).await?;
    // The session's own (project, scene): what its servers are doing is that pair's.
    let pair = |key: &str| session.get(key).and_then(Value::as_str).map(str::to_string);
    let located = crate::locate(pair("scene").as_deref(), pair("project_root").as_deref());
    let status = statuses(call, located).await.map(|(_, by_name)| by_name).unwrap_or_default();
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
        // Switched off: what the engine says of it is that it is.
        assert_eq!(row("off", "scene").status.as_ref(), Some(&status));
    }

    #[test]
    fn a_change_names_the_server_its_tier_and_the_pair_it_is_about() {
        let at = At { scene: Some("code".into()), project_root: Some("/p".into()) };
        assert_eq!(at.request("fs", Some("global")), json!({"scene": "code", "project_root": "/p", "name": "fs", "tier": "global"}));
        // A reconnect writes no tier.
        assert_eq!(at.request("fs", None), json!({"scene": "code", "project_root": "/p", "name": "fs"}));
        let no_project = At { scene: Some("code".into()), project_root: None };
        assert!(no_project.tier(&json!({"tier": "project"})).is_err());
        assert_eq!(no_project.tier(&json!({"tier": "scene"})).unwrap(), "scene");
        assert!(no_project.tier(&json!({"tier": "user"})).is_err());
        assert_eq!(no_project.locate(), json!({"scene": "code"}));
    }

    #[test]
    fn the_statuses_a_change_answers_with_are_read_by_name() {
        let answer = json!({"written_to": "/p/.harnix/settings.json", "servers": [{"name": "fs", "transport": "stdio", "tool_count": 2, "state": "failed", "error": "no such command", "auth": {"state": "none"}}]});
        let read = statuses_in(&answer);
        assert_eq!(read["fs"].state, "failed");
        assert_eq!(read["fs"].error.as_deref(), Some("no such command"));
    }
}
