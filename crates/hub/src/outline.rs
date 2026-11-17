//! `harnix.session.outline`: a session's turns, one row each — where each starts in the transcript,
//! what was asked and the last thing answered, cut short. What a turn rail draws, and what a
//! trajectory groups its rows by.
//!
//! Read off the engine's transcript (`EngineOps::history`), which only grows between two calls
//! unless it is compacted or edited. So the hub keeps the last outline of the sessions most
//! recently asked about, and the next call reads on from the start of the last turn it kept — that
//! turn may have grown — once it finds the message that started it where it was. When it is not
//! there, the transcript changed under it, and the outline is read again from the start.

use harnix_contract::sync::lock;
use harnix_contract::wire::{params as frame_params, OutlineTurn, SessionOutline};
use harnix_contract::RpcError;
use harnix_engine_link::ops::{Answer, EngineOps, HistoryPage, Message};
use serde_json::Value;

use crate::{Hub, SessionKey};

/// About a line's worth of characters: how much of a person's words a turn's row carries.
pub const PROMPT_CHARS: usize = 100;
/// About three lines' worth: how much of a turn's reply its row carries.
pub const REPLY_CHARS: usize = 300;
/// How many sessions' outlines the hub keeps to read on from.
pub(crate) const OUTLINES_KEPT: usize = 64;
/// The most messages one read of the transcript asks for: the engine's own ceiling.
const PAGE: usize = 500;

/// An outline read so far, and how to find where to read on from.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Folded {
    pub turns: Vec<OutlineTurn>,
    /// The transcript's length when it was read.
    pub total: u64,
    /// The id of the message that started the last turn: what says the transcript still has it
    /// where it was.
    last_start: Option<String>,
}

impl Folded {
    /// Take `messages`, the next ones of the transcript, in order.
    pub(crate) fn fold(&mut self, messages: &[Message]) {
        for message in messages {
            if starts_turn(message) && !self.same_turn(message) {
                self.turns.push(OutlineTurn {
                    turn_id: message.turn_id.clone(),
                    seq: message.seq,
                    at: message.ts.clone(),
                    prompt: preview(&message.text, PROMPT_CHARS),
                    reply: String::new(),
                    marks: None,
                });
                self.last_start = message.id.clone();
            } else if message.role == "assistant" && !message.text.trim().is_empty() {
                if let Some(turn) = self.turns.last_mut() {
                    turn.reply = preview(&message.text, REPLY_CHARS);
                }
            }
        }
    }

    /// A person's message that the turn already open wrote too (a command and what it expanded
    /// to, say): it opens no turn of its own.
    fn same_turn(&self, message: &Message) -> bool {
        match (&message.turn_id, self.turns.last()) {
            (Some(id), Some(last)) => last.turn_id.as_ref() == Some(id),
            _ => false,
        }
    }
}

/// Whether a message is a person's words that start a turn: what they typed or the command they
/// ran. A message sent into a running turn (`steer`) is that turn's; what the engine or a tool
/// put in is nobody's turn. A log that does not say who put a message there is read by its text.
pub(crate) fn starts_turn(message: &Message) -> bool {
    if message.role != "user" {
        return false;
    }
    match message.origin.as_deref() {
        Some(origin) => matches!(origin, "user" | "command"),
        None => {
            let text = message.text.trim_start();
            !text.is_empty() && !text.starts_with("<system-reminder>")
        }
    }
}

/// `text` on one line — its runs of white space one space each — cut to `max` characters, the
/// last of them `…` when it was cut.
pub fn preview(text: &str, max: usize) -> String {
    let mut chars = text
        .split_whitespace()
        .enumerate()
        .flat_map(|(i, word)| (i > 0).then_some(' ').into_iter().chain(word.chars()));
    let head: String = chars.by_ref().take(max).collect();
    if chars.next().is_none() {
        return head;
    }
    let mut cut: String = head.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// The outline of `session`, read on from `kept` where the transcript still has its last turn.
pub(crate) async fn read(ops: &dyn EngineOps, session: &str, kept: Option<Folded>) -> Answer<Folded> {
    if let Some(mut kept) = kept {
        if let (Some(last), Some(id)) = (kept.turns.last(), kept.last_start.clone()) {
            let from = last.seq;
            let page = ops.history(session, from, PAGE).await?;
            if page.messages.first().and_then(|m| m.id.as_deref()) == Some(id.as_str()) {
                kept.turns.pop();
                kept.last_start = None;
                return read_on(ops, session, kept, from, page).await;
            }
        }
    }
    let first = ops.history(session, 0, PAGE).await?;
    read_on(ops, session, Folded::default(), 0, first).await
}

/// Fold `page`, read at `offset`, and every page after it.
async fn read_on(ops: &dyn EngineOps, session: &str, mut folded: Folded, mut offset: u64, mut page: HistoryPage) -> Answer<Folded> {
    loop {
        let read = page.messages.len() as u64;
        folded.fold(&page.messages);
        folded.total = page.total.max(offset + read);
        offset += read;
        if read == 0 || offset >= page.total {
            return Ok(folded);
        }
        page = ops.history(session, offset, PAGE).await?;
    }
}

impl Hub {
    /// `harnix.session.outline {instance?, session_id, space?}`: the session's turns
    /// ([`SessionOutline`]), read on from the outline kept for it.
    pub(crate) async fn outline(&self, params: Value) -> Result<Value, RpcError> {
        let key = self.key_of(&params)?;
        let linked = self.engine(&key.0)?;
        let kept = lock(&self.outlines).iter().find(|(k, _)| *k == key).map(|(_, f)| f.clone());
        let folded = read(linked.ops(), &key.1, kept).await?;
        self.keep_outline(&key, &folded);
        Ok(frame_params(&SessionOutline {
            instance: key.0,
            session_id: key.1,
            turns: folded.turns,
            total: folded.total,
        }))
    }

    fn keep_outline(&self, key: &SessionKey, folded: &Folded) {
        let mut kept = lock(&self.outlines);
        kept.retain(|(k, _)| k != key);
        kept.push_back((key.clone(), folded.clone()));
        while kept.len() > OUTLINES_KEPT {
            kept.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn said(seq: u64, role: &str, origin: &str, turn: &str, text: &str) -> Message {
        Message::new(seq, role, text).origin(origin).turn(turn).id(format!("r{seq}")).at(format!("2026-11-16T00:00:{seq:02}Z"))
    }

    /// Two turns: one with a tool call between its words and its answer, one a command; injected
    /// context and a steered message open none.
    fn log() -> Vec<Message> {
        vec![
            said(0, "user", "injected", "t1", "<system-reminder>remember</system-reminder>"),
            said(1, "user", "user", "t1", "Fix the\n\n  build"),
            said(2, "assistant", "assistant", "t1", "Looking."),
            said(3, "user", "tool_result", "t1", ""),
            said(4, "user", "steer", "t1", "also the tests"),
            said(5, "assistant", "assistant", "t1", "Done: the build passes."),
            said(6, "user", "command", "t2", "/review"),
            said(7, "user", "user", "t2", "expanded review prompt"),
            said(8, "assistant", "assistant", "t2", "  "),
        ]
    }

    #[test]
    fn a_log_folds_into_one_row_per_turn() {
        let mut folded = Folded::default();
        folded.fold(&log());
        let rows: Vec<_> = folded.turns.iter().map(|t| (t.turn_id.as_deref(), t.seq, t.prompt.as_str(), t.reply.as_str())).collect();
        assert_eq!(rows, [(Some("t1"), 1, "Fix the build", "Done: the build passes."), (Some("t2"), 6, "/review", "")]);
        assert_eq!(folded.turns[0].at.as_deref(), Some("2026-11-16T00:00:01Z"));
        assert_eq!(folded.last_start.as_deref(), Some("r6"));
    }

    #[test]
    fn a_log_that_does_not_say_who_wrote_a_message_is_read_by_its_text() {
        let old = |seq: u64, role: &str, text: &str| Message::new(seq, role, text);
        let mut folded = Folded::default();
        folded.fold(&[
            old(0, "user", "first"),
            old(1, "assistant", "one"),
            old(2, "user", ""), // a tool's result: no text of its own
            old(3, "user", "<system-reminder>x</system-reminder>"),
            old(4, "user", "second"),
        ]);
        let rows: Vec<_> = folded.turns.iter().map(|t| (t.turn_id.clone(), t.seq, t.prompt.clone(), t.reply.clone())).collect();
        assert_eq!(rows, [(None, 0, "first".into(), "one".into()), (None, 4, "second".into(), String::new())]);
    }

    #[test]
    fn previews_are_one_line_cut_to_their_length() {
        assert_eq!(preview("a\n b\t\tc", 10), "a b c");
        assert_eq!(preview("abcdef", 6), "abcdef");
        assert_eq!(preview("abcdefg", 6), "abcde…");
        assert_eq!(preview("abc def", 3), "ab…");
        assert_eq!(preview("abc", 3), "abc");
        assert_eq!(preview("  abc  ", 3), "abc");
        // Characters, not bytes.
        assert_eq!(preview("一二三四五", 3), "一二…");
        let long = "word ".repeat(200);
        let cut = preview(&long, REPLY_CHARS);
        assert_eq!(cut.chars().count(), REPLY_CHARS);
        assert!(cut.ends_with('…'));
    }
}
