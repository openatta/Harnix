//! The readers of what an engine writes down: its recordings ([`RecordingsDir`]) and its
//! transcripts ([`TranscriptStore`]). The formats are Agix's; this is the one place that parses
//! them.

mod recordings;
mod transcripts;
mod usage;

pub use recordings::RecordingsDir;
pub use transcripts::TranscriptStore;

use harnix_engine_api::readers::ReadError;

/// Reading a recording is file work and JSON over all of it: done on the blocking pool, so a large
/// one does not hold a runtime worker.
async fn off_the_runtime<T: Send + 'static>(f: impl FnOnce() -> Result<T, ReadError> + Send + 'static) -> Result<T, ReadError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ReadError::Unreadable(e.to_string()))?
}
