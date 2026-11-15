//! Who is calling.
//!
//! Four subjects, and admission makes only these. A subject carries an id and nothing else: what
//! an extension may do — its capabilities, how far it is trusted — is in its registration, looked
//! up where a call is decided, never claimed by the call.

use serde::{Deserialize, Serialize};

use super::ExtId;

/// The caller behind one request.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Subject {
    /// This binary, calling itself: the assembly, and the kernel's own rows.
    Kernel,
    /// A paired client, or a page on this machine with its token: one person.
    Device { id: String },
    /// An extension acting as itself: a compiled-in one through its context, a package's backend
    /// through its host.
    Extension {
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        id: ExtId,
    },
    /// An instance's engine, as the audit names what it did — an extension's engine sending more
    /// than it may. No row is open to it: no engine calls this process over HTTP.
    Engine { instance: String },
}

impl Subject {
    pub fn device(id: impl Into<String>) -> Self {
        Subject::Device { id: id.into() }
    }

    pub fn extension(id: ExtId) -> Self {
        Subject::Extension { id }
    }

    /// The extension it is, when it is one.
    pub fn extension_id(&self) -> Option<&ExtId> {
        match self {
            Subject::Extension { id } => Some(id),
            _ => None,
        }
    }

    /// `kernel` · `device:<id>` · `ext:<id>` · `engine:<instance>`.
    pub fn label(&self) -> String {
        match self {
            Subject::Kernel => "kernel".into(),
            Subject::Device { id } => format!("device:{id}"),
            Subject::Extension { id } => format!("ext:{id}"),
            Subject::Engine { instance } => format!("engine:{instance}"),
        }
    }
}
