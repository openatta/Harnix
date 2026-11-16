//! `harnix-app.sessions.export`: a session written out, for reading or for keeping.
//!
//! Three forms. `markdown` is the conversation as a person reads it, with the feedback that stands
//! on each answer under it. `archive` is the session as
//! the engine packs it (`engine.session.export`): the session and its sub-agents' sessions at any depth,
//! cut at one moment, the images in its messages as files, its extension entries and a versioned
//! manifest — and its recording only when asked — as a zip. `bundle` is the older, Harnix-made
//! JSON document: the facts, every message, the entries; no sub-agents, no image files, and no
//! recording — a session's recording comes with the engine's archive, which this extension can
//! ask for, not from the recordings' files, which it cannot read. The answer carries the content itself (the zip as base64); the page turns it
//! into a file. One that would not fit a frame is refused with the way out (leave the recording
//! behind) rather than cut.
//!
//! The caller claims a space like any session call, and `engine.session.get` with that claim is what
//! checks it: a session of another space is refused by the hub, not read here.

use harnix_ext::{codes, Call, RpcError, MAX_FRAME_BYTES, MAIN_INSTANCE};
use serde_json::{json, Value};

use crate::{require_session_id, require_str};

/// What one page of history asks for.
const PAGE: u64 = 200;
/// A tool result is long more often than not; the transcript keeps it whole, the reading copy a
/// screenful.
const RESULT_CHARS: usize = 4000;

pub(crate) async fn session_export(call: Call, params: Value) -> Result<Value, RpcError> {
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
        if with_recording && format != "archive" {
            return Err(RpcError::new(
                codes::REFUSED,
                "a session's recording comes with the engine's archive: `format: archive`",
            ));
        }
        let instance = params
            .get("instance")
            .cloned()
            .unwrap_or(json!(MAIN_INSTANCE));
        let mut target = json!({"instance": instance, "session_id": session_id});
        if let Some(space) = params.get("space").filter(|s| !s.is_null()) {
            target["space"] = space.clone();
        }

        let info = call.call("engine.session.get", target.clone()).await?;
        if format == "archive" {
            let title = title_of(&info);
            return archive(&call, &target, &file_stem(title.as_deref(), &session_id), with_recording)
                .await;
        }
        let messages = all_messages(&call, &target).await?;
        let entries = call
            .call("engine.session.entries", target.clone())
            .await
            .ok()
            .and_then(|v| v.get("entries").cloned())
            .unwrap_or(json!([]));
        let title = title_of(&info);
        let stem = file_stem(title.as_deref(), &session_id);

        let answer = if format == "markdown" {
            json!({
                "format": "markdown",
                "file": format!("{stem}.md"),
                "content": markdown(title.as_deref(), &session_id, &info, &messages, &entries),
            })
        } else {
            let bundle = json!({
                "kind": "harnix-session",
                "version": 1,
                "session": info,
                "title": title,
                "messages": messages,
                "entries": entries,
            });
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
async fn all_messages(call: &Call, target: &Value) -> Result<Vec<Value>, RpcError> {
        let mut messages = Vec::new();
        loop {
            let mut page = target.clone();
            page["offset"] = json!(messages.len());
            page["limit"] = json!(PAGE);
            let answer = call.call("engine.session.history", page).await?;
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
async fn archive(call: &Call, target: &Value, stem: &str, with_recording: bool) -> Result<Value, RpcError> {
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
        let made = call.call("engine.session.export", ask).await;
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

/// The session's title: the engine's name for it.
fn title_of(info: &Value) -> Option<String> {
    info.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).map(str::to_string)
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

/// The feedback entries' namespace and event: the hub's (harnix-app.md §2).
const FEEDBACK_NS: &str = "harnix.feedback";
const FEEDBACK_EVENT: &str = "rating";

/// The feedback that stands in the log, oldest entry first: the last word on each turn (and call)
/// wins, one taken back (`rating: null`) leaves none. Each as its payload.
fn standing_feedback(entries: &Value) -> Vec<Value> {
    let mut standing: Vec<Value> = Vec::new();
    for entry in entries.as_array().into_iter().flatten() {
        if entry.get("ns").and_then(Value::as_str) != Some(FEEDBACK_NS)
            || entry.get("event").and_then(Value::as_str) != Some(FEEDBACK_EVENT)
        {
            continue;
        }
        let Some(payload) = entry.get("payload").filter(|p| p.get("turn_id").and_then(Value::as_str).is_some()) else {
            continue;
        };
        let target = |p: &Value| (p.get("turn_id").cloned(), p.get("call_id").cloned());
        match standing.iter_mut().find(|p| target(p) == target(payload)) {
            Some(slot) => *slot = payload.clone(),
            None => standing.push(payload.clone()),
        }
    }
    standing.retain(|p| matches!(p.get("rating").and_then(Value::as_str), Some("up" | "down")));
    standing
}

/// One piece of feedback as a quoted line: the rating, what was wrong, the note.
fn feedback_line(feedback: &Value) -> String {
    let field = |k: &str| feedback.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
    let mut line = format!("> **Feedback**: {}", field("rating").unwrap_or(""));
    if let Some(category) = field("category") {
        line.push_str(&format!(" ({category})"));
    }
    if let Some(note) = field("note") {
        line.push_str(&format!(" — {}", note.replace('\n', " ")));
    }
    line + "\n\n"
}

/// The conversation as a person reads it: who said what, the tool calls with their input and a
/// screenful of their result, injected context and compaction summaries folded away; under each
/// turn's last message, the feedback that stands on it, and what stands on turns no longer in the
/// transcript at the end.
fn markdown(title: Option<&str>, session_id: &str, info: &Value, messages: &[Value], entries: &Value) -> String {
    let mut out = format!("# {}\n\n", title.unwrap_or(session_id));
    let scene = info.get("scene").and_then(Value::as_str).unwrap_or("");
    out.push_str(&format!("> session `{session_id}`{}\n\n", if scene.is_empty() { String::new() } else { format!(" · scene `{scene}`") }));
    let mut feedback = standing_feedback(entries);
    let turn_of = |m: &Value| m.get("turn_id").and_then(Value::as_str).map(str::to_string);
    for (i, message) in messages.iter().enumerate() {
        markdown_message(&mut out, message);
        let Some(turn) = turn_of(message) else { continue };
        if messages.get(i + 1).and_then(turn_of).as_deref() == Some(turn.as_str()) {
            continue;
        }
        feedback.retain(|f| {
            let here = f.get("turn_id").and_then(Value::as_str) == Some(turn.as_str());
            if here {
                out.push_str(&feedback_line(f));
            }
            !here
        });
    }
    for f in &feedback {
        let turn = f.get("turn_id").and_then(Value::as_str).unwrap_or("");
        out.push_str(&format!("> turn `{turn}`\n>\n{}", feedback_line(f)));
    }
    out
}

/// One message of [`markdown`].
fn markdown_message(out: &mut String, message: &Value) {
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
        let md = markdown(Some("Files"), "S1", &json!({"scene": "harnix"}), &messages, &json!([]));
        assert!(md.starts_with("# Files\n\n> session `S1` · scene `harnix`"));
        assert!(md.contains("<details><summary>Injected context</summary>"));
        assert!(md.contains("**User**\n\nlist the files"));
        assert!(md.contains("**Assistant**\n\nListing."));
        assert!(md.contains("**Tool call** `Bash`\n\n```json\n{\n  \"command\": \"ls\"\n}\n```"));
        assert!(md.contains("**Tool result**\n\n```\na.txt\n```"));
        assert!(md.contains("<details><summary>Compacted context</summary>\n\nearlier, in short"));
    }

    #[test]
    fn the_feedback_that_stands_is_under_its_turns_last_message() {
        let rated = |turn: &str, rating: Value, extra: Value| {
            let mut payload = json!({"turn_id": turn, "rating": rating, "at": 1, "by": "device:d"});
            payload.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            json!({"id": "e", "ns": FEEDBACK_NS, "event": FEEDBACK_EVENT, "payload": payload, "at": 2})
        };
        let messages = vec![
            json!({"role": "user", "origin": "user", "turn_id": "t1", "content": "first"}),
            json!({"role": "assistant", "turn_id": "t1", "content": "one"}),
            json!({"role": "user", "origin": "user", "turn_id": "t2", "content": "second"}),
            json!({"role": "assistant", "turn_id": "t2", "content": "two"}),
        ];
        let entries = json!([
            rated("t1", json!("up"), json!({})),
            {"id": "c", "ns": "harnix.changes", "event": "turn", "payload": {"turn_id": "t1"}},
            rated("t1", json!("down"), json!({"category": "inaccurate", "note": "the wrong\nfile"})),
            rated("t2", json!("up"), json!({})),
            rated("t2", Value::Null, json!({})),
            rated("t0", json!("up"), json!({})),
        ]);
        let md = markdown(None, "S1", &json!({}), &messages, &entries);
        // t1 changed its mind; t2 was taken back; t0 is no longer in the transcript.
        assert!(md.contains("**Assistant**\n\none\n\n> **Feedback**: down (inaccurate) — the wrong file\n\n**User**\n\nsecond"), "{md}");
        assert_eq!(md.matches("**Feedback**").count(), 2, "{md}");
        assert!(md.ends_with("> turn `t0`\n>\n> **Feedback**: up\n\n"), "{md}");
    }

    #[test]
    fn a_file_name_is_made_from_the_title() {
        assert_eq!(file_stem(Some("Q3 plan: draft #2"), "S1"), "Q3-plan--draft--2");
        assert_eq!(file_stem(Some("？？"), "S1"), "S1");
        assert_eq!(file_stem(None, "S1"), "S1");
    }
}
