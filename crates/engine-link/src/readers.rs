//! Both readers of one instance, as the instance layer keeps them.

use std::sync::Arc;

use harnix_engine_api::readers::{Recordings, Transcripts};

/// Both readers of one instance: what its link reads through.
#[non_exhaustive]
#[derive(Clone)]
pub struct Readers {
    pub recordings: Arc<dyn Recordings>,
    pub transcripts: Arc<dyn Transcripts>,
}

impl Readers {
    pub fn new(recordings: Arc<dyn Recordings>, transcripts: Arc<dyn Transcripts>) -> Self {
        Self { recordings, transcripts }
    }
}
