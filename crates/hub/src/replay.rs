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

use harnix_engine_api::events::{Joins, SessionEventKind};
use serde_json::Value;

/// Frames of one turn kept for catch-up, and the byte ceiling over them. A
/// long turn streams thousands of fragments; they are coalesced on the way
/// in, so these bounds are generous in practice.
pub const MAX_FRAMES: usize = 20_000;
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
/// Consecutive fragments of the answer or of thinking merge up to this size.
const COALESCE_BYTES: usize = 4096;

pub struct ReplayFrame {
    pub seq: u64,
    pub turn_id: String,
    pub event: Value,
    /// The stream it is a fragment of, and the field its text is in.
    fragment: Option<(Joins, &'static str)>,
}

pub struct Buffer {
    frames: VecDeque<ReplayFrame>,
    bytes: usize,
    max_frames: usize,
    /// Set when the oldest frames were dropped. The interface says so rather
    /// than showing a turn with a hole in it; history fills it after settling.
    pub truncated: bool,
}

impl Default for Buffer {
    fn default() -> Self {
        Self { frames: VecDeque::new(), bytes: 0, max_frames: MAX_FRAMES, truncated: false }
    }
}

impl Buffer {
    /// Keep at most `max_frames`, from now on.
    pub fn hold_to(&mut self, max_frames: usize) {
        self.max_frames = self.max_frames.min(max_frames);
    }

    pub fn push(&mut self, seq: u64, turn_id: &str, kind: &SessionEventKind, event: &Value) {
        // A merge grows the last frame, so it is held to the same ceiling as a new one.
        if self.merge(seq, turn_id, kind) {
            self.trim();
            return;
        }
        if let SessionEventKind::ToolCall { id, .. } = kind {
            self.forget_tool_args(id);
        }
        let fragment = match kind {
            SessionEventKind::Fragment(f) => Some((f.joins.clone(), f.field)),
            _ => None,
        };
        let bytes = event.to_string().len();
        self.frames.push_back(ReplayFrame {
            seq,
            turn_id: turn_id.to_string(),
            event: event.clone(),
            fragment,
        });
        self.bytes += bytes;
        self.trim();
    }

    /// Drop the oldest frames until both ceilings hold.
    fn trim(&mut self) {
        while self.frames.len() > self.max_frames || self.bytes > MAX_BYTES {
            let Some(dropped) = self.frames.pop_front() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(dropped.event.to_string().len());
            self.truncated = true;
        }
    }

    /// A fragment joins the previous frame when that is a fragment of the same stream in the same
    /// turn: the answer's and thinking's up to [`COALESCE_BYTES`], a tool call's arguments
    /// whatever their size — a large write streams its arguments a few bytes at a time, and kept
    /// frame by frame they would reach the ceiling long before the turn ends.
    fn merge(&mut self, seq: u64, turn_id: &str, kind: &SessionEventKind) -> bool {
        let SessionEventKind::Fragment(fragment) = kind else {
            return false;
        };
        let Some(last) = self.frames.back_mut() else {
            return false;
        };
        let Some((joins, field)) = &last.fragment else {
            return false;
        };
        let room = match joins {
            Joins::ToolArgs(_) => true,
            _ => last.event.get(*field).and_then(Value::as_str).is_some_and(|t| t.len() < COALESCE_BYTES),
        };
        if last.turn_id != turn_id || *joins != fragment.joins || *field != fragment.field || !room {
            return false;
        }
        let Some(Value::String(text)) = last.event.get_mut(*field) else {
            return false;
        };
        text.push_str(&fragment.text);
        last.seq = seq;
        self.bytes += fragment.text.len();
        true
    }

    /// The whole tool call carries its input again, so the fragments it was streamed in are no
    /// longer worth their bytes.
    fn forget_tool_args(&mut self, id: &str) {
        let mut freed = 0;
        self.frames.retain(|f| {
            let fragment = matches!(&f.fragment, Some((Joins::ToolArgs(of), _)) if of == id);
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
    use harnix_engine_api::events::Fragment;
    use serde_json::json;

    /// What the engine's translation makes of these events.
    fn kind(event: &Value) -> SessionEventKind {
        let field = |k: &str| event.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        match event["kind"].as_str().unwrap() {
            "text_delta" => SessionEventKind::Fragment(Fragment::new(Joins::Answer, "text", field("text"))),
            "thinking_delta" => SessionEventKind::Fragment(Fragment::new(Joins::Thinking, "text", field("text"))),
            "tool_args_delta" => {
                SessionEventKind::Fragment(Fragment::new(Joins::ToolArgs(field("id")), "partial_json", field("partial_json")))
            }
            "tool_use" => SessionEventKind::ToolCall { id: event["id"].to_string().trim_matches('"').to_string(), name: field("name") },
            _ => SessionEventKind::Other,
        }
    }

    trait Push {
        fn add(&mut self, seq: u64, turn_id: &str, event: &Value);
    }

    impl Push for Buffer {
        fn add(&mut self, seq: u64, turn_id: &str, event: &Value) {
            self.push(seq, turn_id, &kind(event), event)
        }
    }

    fn delta(text: &str) -> Value {
        json!({"kind": "text_delta", "text": text})
    }

    /// Merging into the last frame is held to the byte ceiling too: one call's arguments
    /// streamed a few bytes at a time used to grow past it unchecked.
    #[test]
    fn merged_bytes_count_against_the_ceiling() {
        let mut b = Buffer::default();
        b.add(1, "t", &delta("first"));
        let part = "x".repeat(64 * 1024);
        let args = json!({"kind": "tool_args_delta", "id": "c", "partial_json": part});
        for seq in 2..(2 + (MAX_BYTES / part.len()) as u64 + 8) {
            b.add(seq, "t", &args);
        }
        assert!(b.bytes <= MAX_BYTES, "{} bytes kept", b.bytes);
        assert!(b.truncated);
    }

    #[test]
    fn consecutive_deltas_coalesce() {
        let mut b = Buffer::default();
        b.add(1, "t", &delta("hel"));
        b.add(2, "t", &delta("lo"));
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
        b.add(1, "t1", &delta("a"));
        b.add(2, "t2", &delta("b"));
        assert_eq!(b.snapshot().len(), 2);
    }

    #[test]
    fn thinking_merges_with_thinking_not_with_text() {
        let mut b = Buffer::default();
        b.add(1, "t", &json!({"kind": "thinking_delta", "text": "hm"}));
        b.add(2, "t", &json!({"kind": "thinking_delta", "text": "m"}));
        b.add(3, "t", &delta("so"));
        let snap = b.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0]["event"]["text"], "hmm");
    }

    #[test]
    fn a_calls_arguments_merge_and_go_once_the_call_is_whole() {
        let mut b = Buffer::default();
        let args = |id: &str, part: &str| json!({"kind": "tool_args_delta", "id": id, "partial_json": part});
        b.add(
            1,
            "t",
            &json!({"kind": "tool_use_start", "id": "a", "name": "Write"}),
        );
        b.add(2, "t", &args("a", "{\"path\":"));
        b.add(3, "t", &args("a", "\"x\"}"));
        let snap = b.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[1]["event"]["partial_json"], "{\"path\":\"x\"}");

        b.add(
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
    fn a_buffer_held_lower_keeps_fewer() {
        let mut b = Buffer::default();
        b.hold_to(4);
        for i in 0..10u64 {
            b.add(i, "t", &json!({"kind": "tool_use", "id": i}));
        }
        assert!(b.truncated);
        assert_eq!(b.snapshot().len(), 4);
    }

    #[test]
    fn dropping_the_oldest_is_reported() {
        let mut b = Buffer::default();
        for i in 0..(MAX_FRAMES + 2) {
            b.add(i as u64, "t", &json!({"kind": "tool_use", "id": i}));
        }
        assert!(b.truncated);
        assert!(b.snapshot().len() <= MAX_FRAMES);
    }
}
