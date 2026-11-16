//! Content search across transcripts, by the engine's `session.search`.

use harnix_engine_api::readers::{ReadError, SearchHit, SearchLimits, SearchResult};
use serde_json::json;

use super::wire::SearchAnswer;
use super::ProtocolReader;
use crate::translate::SESSION_SEARCH;

/// The engine's upper bounds, which it clamps to anyway.
const SNIPPET_MAX: usize = 500;
const DEADLINE_MAX_MS: u64 = 10_000;

/// `user`, `steer` and `command` are a person's; `assistant` the model's.
fn role(origin: &str) -> &'static str {
    if origin == "assistant" {
        "assistant"
    } else {
        "user"
    }
}

impl ProtocolReader {
    pub(super) async fn search_of(&self, sessions: &[String], query: &str, limits: &SearchLimits) -> Result<SearchResult, ReadError> {
        let query = query.trim();
        // The engine refuses an empty query: there is nothing to find.
        if query.is_empty() {
            return Ok(SearchResult::new(Vec::new(), 0, false));
        }
        let ids = &sessions[..sessions.len().min(limits.max_sessions)];
        if ids.is_empty() {
            return Ok(SearchResult::new(Vec::new(), 0, false));
        }
        let params = json!({
            "query": query,
            "session_ids": ids,
            "limit": limits.max_hits,
            "max_sessions": limits.max_sessions,
            "deadline_ms": (limits.deadline.as_millis() as u64).min(DEADLINE_MAX_MS),
            "snippet_chars": limits.snippet.min(SNIPPET_MAX),
        });
        let answer: SearchAnswer = self.ask(SESSION_SEARCH, params).await?;
        let hits = answer
            .hits
            .into_iter()
            .map(|h| SearchHit::new(&h.session_id, role(&h.origin), h.snippet, h.ts).at(h.seq))
            .collect();
        Ok(SearchResult::new(hits, answer.scanned, answer.truncated || sessions.len() > limits.max_sessions))
    }
}
