//! The catch-up buffer.
//!
//! The invariant everything else rests on: **the hub subscribes before any
//! client does.** A session is subscribed the first time anyone opens it, and
//! from then on nothing in it goes unobserved — so a client's catch-up
//! happens entirely inside this buffer and never depends on the engine
//! replaying anything (it does not) or on the transcript being current
//! (mid-turn, it is not).
//!
//! Three facts about Agix make this layer necessary rather than tidy:
//! `session.history` reads the on-disk log and the log is written once per
//! turn, so a running turn is invisible in history; reconnecting re-sends
//! nothing; and a dropped connection loses only its subscription while the
//! turn keeps running. Together: refresh mid-turn and you would see the
//! second half of an answer. Refreshing is the most basic thing a page does.

use std::collections::VecDeque;

use serde_json::Value;

/// Frames of one turn kept for catch-up, and the byte ceiling over them. A
/// long turn streams thousands of `text_delta` frames; they are coalesced on
/// the way in, so these bounds are generous in practice.
pub const MAX_FRAMES: usize = 20_000;
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
/// Consecutive `text_delta` frames merge up to this size.
const COALESCE_BYTES: usize = 4096;

pub struct ReplayFrame {
    pub seq: u64,
    pub turn_id: String,
    pub event: Value,
}

#[derive(Default)]
pub struct Buffer {
    frames: VecDeque<ReplayFrame>,
    bytes: usize,
    /// Set when the oldest frames were dropped. The interface says so rather
    /// than showing a turn with a hole in it; history fills it after settling.
    pub truncated: bool,
}

impl Buffer {
    pub fn push(&mut self, seq: u64, turn_id: &str, event: &Value) {
        if self.merge_delta(seq, turn_id, event) || self.merge_tool_args(seq, turn_id, event) {
            return;
        }
        if event.get("kind").and_then(Value::as_str) == Some("tool_use") {
            self.forget_tool_args(event.get("id"));
        }
        let bytes = event.to_string().len();
        self.frames.push_back(ReplayFrame {
            seq,
            turn_id: turn_id.to_string(),
            event: event.clone(),
        });
        self.bytes += bytes;
        while self.frames.len() > MAX_FRAMES || self.bytes > MAX_BYTES {
            let Some(dropped) = self.frames.pop_front() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(dropped.event.to_string().len());
            self.truncated = true;
        }
    }

    /// Consecutive `text_delta`s merge, and so do consecutive
    /// `thinking_delta`s — each with its own kind.
    fn merge_delta(&mut self, seq: u64, turn_id: &str, event: &Value) -> bool {
        let kind = event.get("kind").and_then(Value::as_str);
        if !matches!(kind, Some("text_delta" | "thinking_delta")) {
            return false;
        }
        let Some(last) = self.frames.back_mut() else {
            return false;
        };
        let mergeable = last.turn_id == turn_id
            && last.event.get("kind").and_then(Value::as_str) == kind
            && last
                .event
                .get("text")
                .and_then(Value::as_str)
                .is_some_and(|t| t.len() < COALESCE_BYTES);
        if !mergeable {
            return false;
        }
        let add = event.get("text").and_then(Value::as_str).unwrap_or("");
        if let Some(Value::String(text)) = last.event.get_mut("text") {
            text.push_str(add);
        }
        last.seq = seq;
        self.bytes += add.len();
        true
    }

    /// A `tool_args_delta` joins the previous one for the same call. A large
    /// write streams its arguments a few bytes at a time; kept frame by frame
    /// they would reach the ceiling long before the turn ends.
    fn merge_tool_args(&mut self, seq: u64, turn_id: &str, event: &Value) -> bool {
        if event.get("kind").and_then(Value::as_str) != Some("tool_args_delta") {
            return false;
        }
        let Some(last) = self.frames.back_mut() else {
            return false;
        };
        let mergeable = last.turn_id == turn_id
            && last.event.get("kind").and_then(Value::as_str) == Some("tool_args_delta")
            && last.event.get("id") == event.get("id");
        if !mergeable {
            return false;
        }
        let add = event
            .get("partial_json")
            .and_then(Value::as_str)
            .unwrap_or("");
        if let Some(Value::String(json)) = last.event.get_mut("partial_json") {
            json.push_str(add);
        }
        last.seq = seq;
        self.bytes += add.len();
        true
    }

    /// The complete `tool_use` carries the whole input again, so the
    /// fragments it was streamed in are no longer worth their bytes.
    fn forget_tool_args(&mut self, id: Option<&Value>) {
        let Some(id) = id else { return };
        let mut freed = 0;
        self.frames.retain(|f| {
            let fragment = f.event.get("kind").and_then(Value::as_str) == Some("tool_args_delta")
                && f.event.get("id") == Some(id);
            if fragment {
                freed += f.event.to_string().len();
            }
            !fragment
        });
        self.bytes = self.bytes.saturating_sub(freed);
    }

    pub fn clear(&mut self) {
        self.frames.clear();
        self.bytes = 0;
        self.truncated = false;
    }

    pub fn snapshot(&self) -> Vec<Value> {
        self.frames
            .iter()
            .map(|f| serde_json::json!({"seq": f.seq, "turn_id": f.turn_id, "event": f.event}))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn delta(text: &str) -> Value {
        json!({"kind": "text_delta", "text": text})
    }

    #[test]
    fn consecutive_deltas_coalesce() {
        let mut b = Buffer::default();
        b.push(1, "t", &delta("hel"));
        b.push(2, "t", &delta("lo"));
        let snap = b.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0]["event"]["text"], "hello");
        // The merged frame carries the newest seq, so a client that has seen
        // it does not ask for the first half again.
        assert_eq!(snap[0]["seq"], 2);
    }

    #[test]
    fn deltas_from_different_turns_do_not_merge() {
        let mut b = Buffer::default();
        b.push(1, "t1", &delta("a"));
        b.push(2, "t2", &delta("b"));
        assert_eq!(b.snapshot().len(), 2);
    }

    #[test]
    fn thinking_merges_with_thinking_not_with_text() {
        let mut b = Buffer::default();
        b.push(1, "t", &json!({"kind": "thinking_delta", "text": "hm"}));
        b.push(2, "t", &json!({"kind": "thinking_delta", "text": "m"}));
        b.push(3, "t", &delta("so"));
        let snap = b.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0]["event"]["text"], "hmm");
    }

    #[test]
    fn a_calls_arguments_merge_and_go_once_the_call_is_whole() {
        let mut b = Buffer::default();
        let args = |id: &str, part: &str| json!({"kind": "tool_args_delta", "id": id, "partial_json": part});
        b.push(
            1,
            "t",
            &json!({"kind": "tool_use_start", "id": "a", "name": "Write"}),
        );
        b.push(2, "t", &args("a", "{\"path\":"));
        b.push(3, "t", &args("a", "\"x\"}"));
        let snap = b.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[1]["event"]["partial_json"], "{\"path\":\"x\"}");

        b.push(
            4,
            "t",
            &json!({"kind": "tool_use", "id": "a", "name": "Write", "input": {"path": "x"}}),
        );
        let kinds: Vec<_> = b
            .snapshot()
            .iter()
            .map(|f| f["event"]["kind"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(kinds, ["tool_use_start", "tool_use"]);
    }

    #[test]
    fn dropping_the_oldest_is_reported() {
        let mut b = Buffer::default();
        for i in 0..(MAX_FRAMES + 2) {
            b.push(i as u64, "t", &json!({"kind": "tool_use", "id": i}));
        }
        assert!(b.truncated);
        assert!(b.snapshot().len() <= MAX_FRAMES);
    }
}
