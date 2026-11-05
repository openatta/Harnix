//! The pure kernel: definitions, validation, scheduling, exit judgement, rework,
//! leases and the protocol layer. It starts no session and touches no disk; it
//! reads and commits through the host traits in [`host`].

pub mod def;
pub mod engine;
pub mod host;
pub mod protocol;
pub mod refs;
pub mod run;
mod txn;
pub mod validate;

/// Why an operation did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowError {
    /// No such run, station, task or flow.
    NotFound(String),
    /// The request does not fit the run's state or the definition.
    Invalid(String),
    /// The authorizer said no.
    Denied(String),
    /// The token is not the task's current one, or its lease has run out.
    Lease(String),
    /// The store kept reporting conflicting revs; nothing was committed.
    Conflict(String),
    /// The store failed.
    Store(String),
}

impl std::fmt::Display for FlowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(m) => write!(f, "not found: {m}"),
            Self::Invalid(m) => write!(f, "invalid: {m}"),
            Self::Denied(m) => write!(f, "denied: {m}"),
            Self::Lease(m) => write!(f, "lease: {m}"),
            Self::Conflict(m) => write!(f, "conflict: {m}"),
            Self::Store(m) => write!(f, "store: {m}"),
        }
    }
}

impl std::error::Error for FlowError {}

impl FlowError {
    /// A short machine-readable code: `not_found`, `invalid`, `denied`, `lease`, `conflict`, `store`.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "not_found",
            Self::Invalid(_) => "invalid",
            Self::Denied(_) => "denied",
            Self::Lease(_) => "lease",
            Self::Conflict(_) => "conflict",
            Self::Store(_) => "store",
        }
    }
}

impl From<host::StoreError> for FlowError {
    fn from(e: host::StoreError) -> Self {
        match e {
            host::StoreError::Conflict(m) => Self::Conflict(m),
            host::StoreError::Other(m) => Self::Store(m),
        }
    }
}

pub type Result<T> = std::result::Result<T, FlowError>;
