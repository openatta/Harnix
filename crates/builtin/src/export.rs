//! `harnix.sessions.export`: a session written out, for reading or for keeping.
//!
//! Three forms. `markdown` is the conversation as a person reads it. `archive` is the session as
//! the engine packs it (`session.export`): the session and its sub-agents' sessions at any depth,
//! cut at one moment, the images in its messages as files, its extension entries and a versioned
//! manifest — and its recording only when asked — as a zip. `bundle` is the older, Harnix-made
//! JSON document: the facts, every message, the entries, the recording when asked; no sub-agents,
//! no image files. The answer carries the content itself (the zip as base64); the page turns it
//! into a file. One that would not fit a frame is refused with the way out (leave the recording
//! behind) rather than cut.
//!
//! The caller claims a space like any session call, and `session.get` with that claim is what
//! checks it: a session of another space is refused by the hub, not read here.

use std::path::Path;

use harnix_contract::{codes, RpcError, MAX_FRAME_BYTES};
use serde_json::{json, Value};

use crate::{require_session_id, require_str, Builtin};

/// What one page of history asks for.
const PAGE: u64 = 200;
/// A tool result is long more often than not; the transcript keeps it whole, the reading copy a
/// screenful.
const RESULT_CHARS: usize = 4000;

impl Builtin {
    pub(crate) async fn session_export(&self, params: Value) -> Result<Value, RpcError> {
        let session_id = require_session_id(&params)?;
        let format = require_str(&params, "format")?;
        if !matches!(format.as_str(), "markdown" | "bundle" | "archive") {
            return Err(RpcError::invalid_params(format!(
                "format is markdown, archive or bundle, not `{format}`"
            )));
        }
        let with_recording = params
            .get("recording")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let instance = params
            .get("instance")
            .cloned()
            .unwrap_or(json!(harnix_contract::SYSTEM_INSTANCE));
        let mut target = json!({"instance": instance, "session_id": session_id});
        if let Some(space) = params.get("space").filter(|s| !s.is_null()) {
            target["space"] = space.clone();
        }

        let info = self.call("sessions", "session.get", target.clone()).await?;
        if format == "archive" {
            let title = self.title_of(&session_id).await.or_else(|| {
                info.get("name").and_then(Value::as_str).map(str::to_string)
            });
            return self
                .archive(&target, &file_stem(title.as_deref(), &session_id), with_recording)
                .await;
        }
        let messages = self.all_messages(&target).await?;
        let entries = self
            .call("sessions", "session.entries", target.clone())
            .await
            .ok()
            .and_then(|v| v.get("entries").cloned())
            .unwrap_or(json!([]));
        let title = self.title_of(&session_id).await.or_else(|| {
            info.get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
        let stem = file_stem(title.as_deref(), &session_id);

        let answer = if format == "markdown" {
            json!({
                "format": "markdown",
                "file": format!("{stem}.md"),
                "content": markdown(title.as_deref(), &session_id, &info, &messages),
            })
        } else {
            let mut bundle = json!({
                "kind": "harnix-session",
                "version": 1,
                "session": info,
                "title": title,
                "messages": messages,
                "entries": entries,
            });
            if with_recording {
                let instance = instance.as_str().unwrap_or(harnix_contract::SYSTEM_INSTANCE);
                let dir = self.recording_dir(instance, &session_id)?;
                bundle["recording"] = recording(&dir);
            }
            json!({"format": "bundle", "file": format!("{stem}.json"), "content": bundle})
        };
        // Leave room for the frame's envelope around the answer.
        if serde_json::to_vec(&answer).map_or(0, |b| b.len()) > MAX_FRAME_BYTES - 64 * 1024 {
            return Err(RpcError::new(
                codes::REFUSED,
                "this export is larger than one frame; export it without the recording, or as markdown",
            ));
        }
        Ok(answer)
    }

    /// The whole transcript, page by page.
    async fn all_messages(&self, target: &Value) -> Result<Vec<Value>, RpcError> {
        let mut messages = Vec::new();
        loop {
            let mut page = target.clone();
            page["offset"] = json!(messages.len());
            page["limit"] = json!(PAGE);
            let answer = self.call("sessions", "session.history", page).await?;
            let got = answer
                .get("messages")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let total = answer.get("total").and_then(Value::as_u64).unwrap_or(0);
            if got.is_empty() {
                break;
            }
            messages.extend(got);
            if messages.len() as u64 >= total {
                break;
            }
        }
        Ok(messages)
    }

    /// The engine's archive of the session, written to a directory of its own under the system's
    /// temporary one and read back: the engine writes where it runs, which for every carrier but
    /// an attached daemon is this machine. The directory goes either way.
    async fn archive(&self, target: &Value, stem: &str, with_recording: bool) -> Result<Value, RpcError> {
        let dir = std::env::temp_dir().join(format!("harnix-export-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir)
            .map_err(|e| RpcError::new(codes::INTERNAL_ERROR, format!("no room to export into: {e}")))?;
        let file = format!("{stem}.zip");
        let path = dir.join(&file);
        let mut include = vec!["sidechains", "attachments"];
        if with_recording {
            include.push("recording");
        }
        let mut ask = target.clone();
        ask["path"] = json!(path);
        ask["format"] = json!("zip");
        ask["include"] = json!(include);
        let made = self.call("sessions", "session.export", ask).await;
        let bytes = made.as_ref().ok().map(|_| std::fs::read(&path));
        let _ = std::fs::remove_dir_all(&dir);
        let made = made?;
        let bytes = bytes.unwrap_or_else(|| Ok(Vec::new())).map_err(|e| {
            RpcError::new(
                codes::REFUSED,
                format!("the engine wrote the archive where this process cannot read it: {e}"),
            )
        })?;
        use base64::Engine as _;
        Ok(json!({
            "format": "archive",
            "file": file,
            "encoding": "base64",
            "content": base64::engine::general_purpose::STANDARD.encode(&bytes),
            "bytes": bytes.len(),
            "sessions": made.get("sessions").and_then(Value::as_array).map_or(0, Vec::len),
            "attachments": made.get("attachments").cloned().unwrap_or(json!(0)),
            "recording": made.get("recording").cloned().unwrap_or(json!(false)),
            "warnings": made.get("warnings").cloned().unwrap_or(json!([])),
        }))
    }

    /// The title a person gave the session here, if any (the engine's name otherwise).
    async fn title_of(&self, session_id: &str) -> Option<String> {
        self.store.lock().await.title_of(session_id)
    }
}

/// The session's recording, as its files: `calls.jsonl` and the blobs it names. Missing is null.
fn recording(dir: &Path) -> Value {
    let Ok(calls) = std::fs::read_to_string(dir.join("calls.jsonl")) else {
        return Value::Null;
    };
    let mut blobs = serde_json::Map::new();
    if let Ok(read) = std::fs::read_dir(dir.join("blobs")) {
        for entry in read.flatten() {
            if let Ok(text) = std::fs::read_to_string(entry.path()) {
                blobs.insert(entry.file_name().to_string_lossy().into_owned(), json!(text));
            }
        }
    }
    json!({"calls": calls, "blobs": blobs})
}

/// A file name from the title: letters, digits and `-`, short; the id when there is no title.
fn file_stem(title: Option<&str>, session_id: &str) -> String {
    let base: String = title
        .unwrap_or(session_id)
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let base = base.trim_matches('-');
    let base: String = base.chars().take(60).collect();
    if base.is_empty() {
        session_id.to_string()
    } else {
        base
    }
}

/// The conversation as a person reads it: who said what, the tool calls with their input and a
/// screenful of their result, injected context and compaction summaries folded away.
fn markdown(title: Option<&str>, session_id: &str, info: &Value, messages: &[Value]) -> String {
    let mut out = format!("# {}\n\n", title.unwrap_or(session_id));
    let scene = info.get("scene").and_then(Value::as_str).unwrap_or("");
    out.push_str(&format!("> session `{session_id}`{}\n\n", if scene.is_empty() { String::new() } else { format!(" · scene `{scene}`") }));
    for message in messages {
        let role = message.get("role").and_then(Value::as_str).unwrap_or("");
        let origin = message.get("origin").and_then(Value::as_str);
        let parts: Vec<Value> = match message.get("content") {
            Some(Value::String(text)) => vec![json!({"type": "text", "text": text})],
            Some(Value::Array(parts)) => parts.clone(),
            _ => Vec::new(),
        };
        for part in parts {
            match part.get("type").and_then(Value::as_str) {
                Some("text") => {
                    let text = part.get("text").and_then(Value::as_str).unwrap_or("").trim();
                    if text.is_empty() {
                        continue;
                    }
                    match (role, origin) {
                        ("user", Some("injected" | "compact" | "extension")) => {
                            let label = if origin == Some("compact") { "Compacted context" } else { "Injected context" };
                            out.push_str(&format!("<details><summary>{label}</summary>\n\n{text}\n\n</details>\n\n"));
                        }
                        ("user", _) => out.push_str(&format!("**User**\n\n{text}\n\n")),
                        _ => out.push_str(&format!("**Assistant**\n\n{text}\n\n")),
                    }
                }
                Some("tool_use") => {
                    let name = part.get("name").and_then(Value::as_str).unwrap_or("tool");
                    let input = serde_json::to_string_pretty(part.get("input").unwrap_or(&Value::Null)).unwrap_or_default();
                    out.push_str(&format!("**Tool call** `{name}`\n\n```json\n{input}\n```\n\n"));
                }
                Some("tool_result") => {
                    let text = result_text(part.get("content"));
                    let clipped: String = text.chars().take(RESULT_CHARS).collect();
                    let more = if text.chars().count() > RESULT_CHARS { "\n…" } else { "" };
                    let label = if part.get("is_error").and_then(Value::as_bool) == Some(true) { "Tool error" } else { "Tool result" };
                    out.push_str(&format!("**{label}**\n\n```\n{clipped}{more}\n```\n\n"));
                }
                _ => {}
            }
        }
    }
    out
}

fn result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reading_copy_says_who_said_what_and_folds_what_nobody_said() {
        let messages = vec![
            json!({"role": "user", "origin": "injected", "content": "<system-reminder>cwd</system-reminder>"}),
            json!({"role": "user", "origin": "user", "content": "list the files"}),
            json!({"role": "assistant", "content": [{"type": "text", "text": "Listing."}, {"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "ls"}}]}),
            json!({"role": "user", "origin": "tool_result", "content": [{"type": "tool_result", "tool_use_id": "t1", "content": [{"type": "text", "text": "a.txt"}]}]}),
            json!({"role": "user", "origin": "compact", "content": "earlier, in short"}),
        ];
        let md = markdown(Some("Files"), "S1", &json!({"scene": "harnix"}), &messages);
        assert!(md.starts_with("# Files\n\n> session `S1` · scene `harnix`"));
        assert!(md.contains("<details><summary>Injected context</summary>"));
        assert!(md.contains("**User**\n\nlist the files"));
        assert!(md.contains("**Assistant**\n\nListing."));
        assert!(md.contains("**Tool call** `Bash`\n\n```json\n{\n  \"command\": \"ls\"\n}\n```"));
        assert!(md.contains("**Tool result**\n\n```\na.txt\n```"));
        assert!(md.contains("<details><summary>Compacted context</summary>\n\nearlier, in short"));
    }

    #[test]
    fn a_file_name_is_made_from_the_title() {
        assert_eq!(file_stem(Some("Q3 plan: draft #2"), "S1"), "Q3-plan--draft--2");
        assert_eq!(file_stem(Some("？？"), "S1"), "S1");
        assert_eq!(file_stem(None, "S1"), "S1");
    }
}
