//! Workspaces, archived sessions, view preferences, and search.

use std::collections::HashMap;

use harnix_ext::engine::readers::SearchLimits;
use harnix_ext::{codes, Call, RpcError, MAIN_INSTANCE};
use serde_json::{json, Value};

use crate::store::{new_workspace_id, Workspace};
use crate::{default_title, require_session_id, require_str, App};

impl App {
    /// Workspaces are Harnix's own idea. The engine knows a session's
    /// `project_root` and nothing else, so grouping, ordering, collapse and
    /// archival live in `store` and are joined onto the session list here.
    /// The groups, the view preferences, and **the per-session overlay**.
    ///
    /// The overlay is here rather than folded into `harnix.sessions.list` because
    /// the hub does not know what a workspace is and must not learn: it holds
    /// sessions, and a title someone typed is not a property of a session,
    /// it is a note this product keeps about one.
    ///
    /// It was briefly nowhere at all. `harnix.sessions.list` used to fold it in;
    /// splitting the layers moved the store out of the hub and the fold went
    /// with it, so renaming a session wrote to disk and nothing ever read it
    /// back — along with archiving, grouping and every view preference. The
    /// interface tests did not notice because their fake backend still
    /// returned the old shape, which is the one way a fake earns its risk and
    /// then loses it.
    pub(crate) async fn workspaces_list(&self) -> Result<Value, RpcError> {
        let store = self.store.lock().await;
        let state = store.state();
        // Which workspace a session belongs to is **not** here, and cannot
        // be: it is a session's project root matched against a workspace's
        // path, and the store knows the paths while the hub knows the roots.
        // Reaching across for it would mean one of the two layers learning
        // about the other. The client holds both halves already, so the join
        // is its to make.
        let sessions: serde_json::Map<String, Value> = state
            .sessions
            .iter()
            .map(|(id, meta)| {
                (
                    id.clone(),
                    json!({"archived": meta.archived}),
                )
            })
            .collect();
        Ok(json!({
            "workspaces": state.workspaces,
            "prefs": state.prefs,
            "sessions": sessions,
        }))
    }

    pub(crate) async fn workspaces_create(&self, params: Value) -> Result<Value, RpcError> {
        let path = params
            .get("path")
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(path) = &path {
            if !std::path::Path::new(path).is_dir() {
                return Err(RpcError::new(
                    codes::INVALID_PARAMS,
                    format!("not a directory: {path}"),
                ));
            }
        }
        let title = params
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| default_title(path.as_deref()));

        let mut store = self.store.lock().await;
        if let Some(existing) = store.workspace_for(path.as_deref()) {
            return Ok(json!({"workspace": existing, "existed": true}));
        }
        let workspace = Workspace {
            id: new_workspace_id(),
            path,
            title,
            collapsed: false,
            session_order: Vec::new(),
        };
        let created = store.update(|state| {
            state.workspaces.push(workspace);
            state.workspaces.last().cloned()
        });
        Ok(json!({"workspace": created, "existed": false}))
    }

    pub(crate) async fn workspaces_update(&self, params: Value) -> Result<Value, RpcError> {
        let id = require_str(&params, "id")?;
        let mut store = self.store.lock().await;
        let updated = store.update(|state| {
            let workspace = state.workspaces.iter_mut().find(|w| w.id == id)?;
            if let Some(title) = params.get("title").and_then(Value::as_str) {
                let trimmed = title.trim();
                if !trimmed.is_empty() {
                    workspace.title = trimmed.to_string();
                }
            }
            if let Some(collapsed) = params.get("collapsed").and_then(Value::as_bool) {
                workspace.collapsed = collapsed;
            }
            if let Some(order) = params.get("session_order").and_then(Value::as_array) {
                workspace.session_order = order
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect();
            }
            Some(workspace.clone())
        });
        match updated {
            Some(workspace) => Ok(json!({"workspace": workspace})),
            None => Err(RpcError::missing("unknown workspace")),
        }
    }

    /// Move a workspace before another (or to the end). The whole order comes
    /// back, so a client never has to reconstruct it from a delta.
    pub(crate) async fn workspaces_reorder(&self, params: Value) -> Result<Value, RpcError> {
        let id = require_str(&params, "id")?;
        let before = params
            .get("before_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        let mut store = self.store.lock().await;
        let workspaces = store.update(|state| {
            let from = state.workspaces.iter().position(|w| w.id == id)?;
            let moved = state.workspaces.remove(from);
            let at = before
                .as_deref()
                .and_then(|anchor| state.workspaces.iter().position(|w| w.id == anchor))
                .unwrap_or(state.workspaces.len());
            state.workspaces.insert(at, moved);
            Some(state.workspaces.clone())
        });
        match workspaces {
            Some(workspaces) => Ok(json!({"workspaces": workspaces})),
            None => Err(RpcError::missing("unknown workspace")),
        }
    }

    /// Forget the grouping only. Sessions and transcripts are untouched — a
    /// removed workspace's sessions reappear ungrouped.
    pub(crate) async fn workspaces_remove(&self, params: Value) -> Result<Value, RpcError> {
        let id = require_str(&params, "id")?;
        let mut store = self.store.lock().await;
        let workspaces = store.update(|state| {
            state.workspaces.retain(|w| w.id != id);
            state.workspaces.clone()
        });
        Ok(json!({"workspaces": workspaces}))
    }

    pub(crate) async fn session_archive(&self, call: &Call, params: Value) -> Result<Value, RpcError> {
        let session_id = require_session_id(&params)?;
        session_must_exist(call, &session_id).await?;
        let archived = params
            .get("archived")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let mut store = self.store.lock().await;
        store.update(|state| {
            let meta = state.sessions.entry(session_id.clone()).or_default();
            meta.archived = archived;
        });
        Ok(json!({"session_id": session_id, "archived": archived}))
    }

    pub(crate) async fn prefs_set(&self, params: Value) -> Result<Value, RpcError> {
        let key = require_str(&params, "key")?;
        let value = params.get("value").cloned().unwrap_or(Value::Null);
        let mut store = self.store.lock().await;
        let prefs = store.update(|state| {
            if value.is_null() {
                state.prefs.remove(&key);
            } else {
                state.prefs.insert(key.clone(), value.clone());
            }
            state.prefs.clone()
        });
        Ok(json!({"prefs": prefs}))
    }

    /// Content search across sessions.
    ///
    /// Ordering comes from `engine.session.list` (most recent first); the scan itself
    /// is the engine's transcript reader, which does not page `engine.session.history`.
    pub(crate) async fn search(&self, call: &Call, params: Value) -> Result<Value, RpcError> {
        let query = require_str(&params, "query")?;
        let transcripts = call
            .ext()
            .engine(MAIN_INSTANCE)
            .ok_or_else(|| RpcError::new(codes::INSTANCE_NOT_FOUND, "there is no main instance to search"))?
            .transcripts()?;

        let listed = call.call("engine.session.list", json!({})).await?;
        let sessions: Vec<String> = listed
            .get("sessions")
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .filter_map(|row| row.get("session_id").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();

        let mut limits = SearchLimits::default();
        if let Some(limit) = params.get("limit").and_then(Value::as_u64) {
            limits = limits.hits(limit as usize);
        }
        let outcome = transcripts.search(&sessions, &query, &limits).await;

        // The session's title is the engine's name for it, in the list read above.
        let titles: HashMap<String, Value> = listed
            .get("sessions")
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .filter_map(|row| Some((row.get("session_id")?.as_str()?.to_string(), row.get("name")?.clone())))
                    .collect()
            })
            .unwrap_or_default();
        let hits: Vec<Value> = outcome
            .hits
            .iter()
            .map(|hit| {
                json!({
                    "session_id": hit.session_id,
                    "name": titles.get(&hit.session_id).cloned().unwrap_or(Value::Null),
                    "role": hit.role,
                    "snippet": hit.snippet,
                    "ts": hit.ts,
                })
            })
            .collect();

        Ok(json!({
            "hits": hits,
            "scanned": outcome.scanned,
            "truncated": outcome.truncated,
        }))
    }
}

/// Refuse to write an overlay row for a session the engine does not have.
///
/// This writes into a map keyed by session id with `entry(id).or_default()`, so any id at all
/// grew a row — and the file it lives in is never cleaned. A mistyped id left a row behind that
/// joined to nothing, forever, and the caller was told it worked.
async fn session_must_exist(call: &Call, session_id: &str) -> Result<(), RpcError> {
    call.call("engine.session.get", json!({"session_id": session_id})).await.map(|_| ())
}
