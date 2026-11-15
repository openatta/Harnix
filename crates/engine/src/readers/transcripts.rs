//! Content search across transcripts.
//!
//! The engine is a library in this process, so a search reads the history store directly instead
//! of paging `session.history` over the RPC surface: that path projects every message and
//! serializes it to JSON, all of it discarded by a substring test. DSH indexes this into SQLite,
//! which is worth doing when a scan stops being fast and is not worth doing before — so the scan
//! is bounded three ways: sessions visited, hits returned, and a wall clock deadline, and it
//! reports which bound it hit.

use std::sync::Arc;
use std::time::Instant;

use agix::history::{ContentBlock, HistoryStore, LogEntry};
use harnix_engine_api::readers::{SearchHit, SearchLimits, SearchResult, Transcripts};

/// An engine's transcripts, as its history store keeps them.
pub struct TranscriptStore(Arc<dyn HistoryStore>);

impl TranscriptStore {
    pub fn new(store: Arc<dyn HistoryStore>) -> Self {
        Self(store)
    }
}

#[async_trait::async_trait]
impl Transcripts for TranscriptStore {
    async fn search(&self, sessions: &[String], query: &str, limits: &SearchLimits) -> SearchResult {
        search(self.0.as_ref(), sessions, query, limits).await
    }
}

/// Case-insensitive substring search over the transcripts of `sessions`, most recent first (the
/// caller decides that order).
async fn search(store: &dyn HistoryStore, sessions: &[String], query: &str, limits: &SearchLimits) -> SearchResult {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return SearchResult::new(Vec::new(), 0, false);
    }

    let started = Instant::now();
    let mut hits = Vec::new();
    let mut scanned = 0usize;
    let mut truncated = sessions.len() > limits.max_sessions;

    for session_id in sessions.iter().take(limits.max_sessions) {
        if hits.len() >= limits.max_hits {
            truncated = true;
            break;
        }
        if started.elapsed() > limits.deadline {
            truncated = true;
            break;
        }
        let Ok(id) = agix::session::SessionId::parse(session_id) else {
            continue;
        };
        let Ok(entries) = store.load(id).await else {
            continue;
        };
        scanned += 1;

        for entry in &entries {
            let (role, text) = match &entry.entry {
                LogEntry::User { content } => ("user", blocks_text(content)),
                LogEntry::Assistant { content, .. } => ("assistant", blocks_text(content)),
                _ => continue,
            };
            // Injected context is not something anyone searched for.
            if text.starts_with("<system-reminder>") {
                continue;
            }
            let Some(at) = text.to_lowercase().find(&needle) else {
                continue;
            };
            hits.push(SearchHit::new(session_id, role, snippet_around(&text, at, limits.snippet), entry.ts.to_string()));
            break; // one hit per session keeps the list readable
        }
    }

    SearchResult::new(hits, scanned, truncated)
}

fn blocks_text(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `width` characters around the match, cut on character boundaries.
fn snippet_around(text: &str, at: usize, width: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let hit = text[..at].chars().count();
    let start = hit.saturating_sub(width / 3);
    let end = (start + width).min(chars.len());
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.extend(chars[start..end].iter());
    if end < chars.len() {
        out.push('…');
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snippet_marks_both_cuts() {
        let text = "the quick brown fox jumps over the lazy dog and keeps going for a while";
        let at = text.find("lazy").unwrap();
        let snippet = snippet_around(text, at, 20);
        assert!(snippet.contains("lazy"), "{snippet}");
        assert!(snippet.starts_with('…') && snippet.ends_with('…'), "{snippet}");
    }

    #[test]
    fn a_short_text_is_not_cut() {
        let snippet = snippet_around("hello world", 6, 40);
        assert_eq!(snippet, "hello world");
    }
}
