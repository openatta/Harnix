//! A session's name, and the call chain a turn runs on behalf of.

/// A session's full name: the instance it is on and its id there. An identity key: it gets no
/// other field.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
pub struct SessionRef {
    pub instance: String,
    pub session_id: String,
}

impl SessionRef {
    pub fn new(instance: impl Into<String>, session_id: impl Into<String>) -> Self {
        Self { instance: instance.into(), session_id: session_id.into() }
    }
}

/// One link of a call chain: the extension that made a call, and that call's id. A tool call made
/// on behalf of a session carries the chain that led to the session's turn.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ChainLink {
    ext: String,
    call: String,
}

impl ChainLink {
    pub fn new(ext: impl Into<String>, call: impl Into<String>) -> Self {
        Self { ext: ext.into(), call: call.into() }
    }

    pub fn ext(&self) -> &str {
        &self.ext
    }

    pub fn call(&self) -> &str {
        &self.call
    }
}
