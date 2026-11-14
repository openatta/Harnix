//! One-shot upload grants: how a client sends what is too large for a message.
//!
//! An extension issues a grant and answers its token; the client sends the bytes to the upload
//! route with it, once; they land at the grant's path, and the extension reads them from there.

use std::path::{Path, PathBuf};

use crate::context::ExtensionContext;
use crate::Rpc;

pub struct Bulk<'a> {
    cx: &'a ExtensionContext,
}

impl<'a> Bulk<'a> {
    pub(crate) fn new(cx: &'a ExtensionContext) -> Self {
        Self { cx }
    }

    /// A grant for one upload of at most `max_bytes`.
    pub fn grant(&self, max_bytes: usize) -> Rpc<UploadGrant> {
        let (token, path) = self.cx.ports().bulk(self.cx.id(), None, max_bytes)?;
        Ok(UploadGrant { token, path, max_bytes })
    }

    /// [`grant`](Self::grant), the file named after what the client sends — `name`, made safe, at
    /// the end — for whatever reads it by its name or its extension.
    pub fn grant_named(&self, name: &str, max_bytes: usize) -> Rpc<UploadGrant> {
        let (token, path) = self.cx.ports().bulk(self.cx.id(), Some(name), max_bytes)?;
        Ok(UploadGrant { token, path, max_bytes })
    }
}

/// One upload, granted.
#[derive(Debug, Clone)]
pub struct UploadGrant {
    token: String,
    path: PathBuf,
    max_bytes: usize,
}

impl UploadGrant {
    /// What the client sends the bytes with.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Where the bytes will be, once sent.
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }
}
