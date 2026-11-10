//! Content search across transcripts.
//!
//! The engine's transcript reader (`harnix_engine::readers::TranscriptStore`) does the reading:
//! a scan of the history store, bounded by sessions visited, hits returned and a deadline, which
//! says which bound it hit. These are its terms, under the names this crate gave them.

pub use harnix_engine_api::readers::{SearchHit as Hit, SearchLimits as Limits, SearchResult as Outcome};
