//! Feedback on answers, kept in the session's log.
//!
//! A person's thumbs up or down on a turn's answer — with what was wrong, and a note — is an
//! entry (`harnix.feedback` / `rating`) in the session's log, through the same seam the turn's
//! changes use: for people and never for the model; a refresh, a replay and a whole fork keep it,
//! an export carries it, and an interface that does not know it skips it. Changing it or taking it
//! back files another entry; the last one for the turn stands (`wire::current_feedback`), and the
//! ones before it stay in the log.
//!
//! Nothing here sends it anywhere. Each piece is told on the topic `harnix.feedback`, which only
//! an extension holding `feedback.read` may subscribe to: whether feedback leaves the machine is
//! that extension's doing, and so the product's choice (harnix-ext.md §7.3).

use harnix_contract::wire::{
    self, Feedback, FeedbackCategory, FeedbackFiled, FeedbackNotice, Rating, FEEDBACK_EVENT, FEEDBACK_NOTE_MAX_CHARS,
    FEEDBACK_NS, FEEDBACK_TOPIC,
};
use harnix_contract::{RpcError, Subject};
use harnix_engine_link::ops::Entry;
use serde_json::{json, Value};

use crate::Hub;

/// The longest turn or call id taken: the engine's are far shorter.
const MAX_ID_CHARS: usize = 200;

/// An id the caller names: a non-empty word of no control characters.
fn id_of(params: &Value, key: &str) -> Result<Option<String>, RpcError> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(id)) if !id.is_empty() && id.chars().count() <= MAX_ID_CHARS && !id.chars().any(char::is_control) => {
            Ok(Some(id.clone()))
        }
        Some(_) => Err(RpcError::invalid_params(format!(
            "`{key}` is an id of 1–{MAX_ID_CHARS} characters"
        ))),
    }
}

/// The feedback a call gives: `{turn_id, call_id?, rating: "up" | "down" | null, category?, note?}`,
/// by `by`, at `at`. Taking it back (`rating: null`) carries no category or note.
pub(crate) fn feedback_of(params: &Value, by: String, at: u64) -> Result<Feedback, RpcError> {
    let turn_id = id_of(params, "turn_id")?.ok_or_else(|| RpcError::invalid_params("missing turn_id"))?;
    let call_id = id_of(params, "call_id")?;
    let rating: Option<Rating> = match params.get("rating") {
        None => return Err(RpcError::invalid_params("`rating` is up, down, or null to take it back")),
        Some(v) => serde_json::from_value(v.clone())
            .map_err(|_| RpcError::invalid_params("`rating` is up, down, or null to take it back"))?,
    };
    let category: Option<FeedbackCategory> = match params.get("category") {
        None | Some(Value::Null) => None,
        Some(v) => Some(serde_json::from_value(v.clone()).map_err(|_| {
            RpcError::invalid_params("`category` is one of inaccurate, unhelpful, unsafe, slow, other")
        })?),
    };
    let note = match params.get("note") {
        None | Some(Value::Null) => None,
        Some(Value::String(note)) => {
            let note = note.trim();
            if note.chars().count() > FEEDBACK_NOTE_MAX_CHARS {
                return Err(RpcError::invalid_params(format!(
                    "a note is at most {FEEDBACK_NOTE_MAX_CHARS} characters"
                )));
            }
            (!note.is_empty()).then(|| note.to_string())
        }
        Some(_) => return Err(RpcError::invalid_params("`note` is text")),
    };
    let (category, note) = if rating.is_some() { (category, note) } else { (None, None) };
    Ok(Feedback { turn_id, call_id, rating, category, note, at, by })
}

impl Hub {
    /// `harnix.session.feedback {instance?, session_id, space?, turn_id, call_id?, rating, category?, note?}`:
    /// a person's feedback on a turn's answer, filed in the session's log; answers what was filed
    /// and the entry. Whoever watches the session sees the entry pushed as any other; a holder of
    /// `feedback.read` hears it on the topic.
    pub(crate) async fn feedback(&self, subject: &Subject, params: Value) -> Result<Value, RpcError> {
        let key = self.key_of(&params)?;
        let feedback = feedback_of(&params, subject.label(), crate::now_ms())?;
        let entry = Entry::new(FEEDBACK_NS, FEEDBACK_EVENT, wire::params(&feedback));
        let filed = self.engine(&key.0)?.ops().append_entry(&key.1, entry).await?;
        self.home_of(&key.0);
        let notice = FeedbackNotice {
            instance: key.0.clone(),
            session_id: key.1.clone(),
            space: self.space_known(&key),
            entry_id: filed.get("id").and_then(Value::as_str).unwrap_or_default().to_string(),
            feedback: feedback.clone(),
        };
        self.publish(FEEDBACK_TOPIC, json!({}), wire::params(&notice)).await;
        Ok(wire::params(&FeedbackFiled { feedback, entry: filed }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn given(params: Value) -> Result<Feedback, RpcError> {
        feedback_of(&params, "device:d".into(), 9)
    }

    #[test]
    fn a_rating_with_what_was_wrong_and_a_note() {
        let f = given(json!({"turn_id": "t1", "rating": "down", "category": "inaccurate", "note": "  wrong file  "})).unwrap();
        assert_eq!(
            wire::params(&f),
            json!({"turn_id": "t1", "rating": "down", "category": "inaccurate", "note": "wrong file", "at": 9, "by": "device:d"})
        );
        let f = given(json!({"turn_id": "t1", "call_id": "c1", "rating": "up", "note": "   "})).unwrap();
        assert_eq!((f.call_id.as_deref(), f.note), (Some("c1"), None), "a blank note is none");
    }

    #[test]
    fn taking_it_back_carries_nothing_but_the_turn() {
        let f = given(json!({"turn_id": "t1", "rating": null, "category": "slow", "note": "x"})).unwrap();
        assert_eq!(wire::params(&f), json!({"turn_id": "t1", "rating": null, "at": 9, "by": "device:d"}));
    }

    #[test]
    fn what_is_not_feedback_is_refused_by_name() {
        let refused = |p: Value| given(p).unwrap_err().message;
        assert!(refused(json!({"rating": "up"})).contains("turn_id"));
        assert!(refused(json!({"turn_id": "", "rating": "up"})).contains("turn_id"));
        assert!(refused(json!({"turn_id": "t\u{7}", "rating": "up"})).contains("turn_id"));
        assert!(refused(json!({"turn_id": "t"})).contains("rating"), "a rating must be said, if only as null");
        assert!(refused(json!({"turn_id": "t", "rating": "sideways"})).contains("rating"));
        assert!(refused(json!({"turn_id": "t", "rating": "up", "category": "boring"})).contains("category"));
        assert!(refused(json!({"turn_id": "t", "rating": "up", "note": 3})).contains("note"));
        let long = "字".repeat(FEEDBACK_NOTE_MAX_CHARS + 1);
        assert!(refused(json!({"turn_id": "t", "rating": "up", "note": long})).contains("at most"));
        given(json!({"turn_id": "t", "rating": "up", "note": "字".repeat(FEEDBACK_NOTE_MAX_CHARS)})).unwrap();
    }
}
