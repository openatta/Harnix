//! Who is calling.
//!
//! Three subjects (concept §3.4). Installed code is **not** one of them: an
//! extension runs inside Agix, under Agix's capability gate, and never
//! reaches this process's method surface. A `Plugin` variant here would be a
//! second place deciding what an extension may do.
//!
//! A `Module` is a compile-time module — source the application author linked
//! in, trusted like the kernel. It is a subject of its own anyway, so the
//! table can say which module may call what and the audit can say which one
//! did: that is a guard against mistakes, not against the module (concept §6.2).
//!
//! A runtime application acting as itself is a `Module` too, by its
//! application id — a package's code, not the author's, and so not trusted
//! like one: the authorization table holds it to what its manifest's `hub`
//! lists (`harnix_authz::MethodTable::reach_only`). It is still the one
//! subject, so the table and the audit see it where they see every module.

use serde::{Deserialize, Serialize};

/// The caller behind one request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Subject {
    /// This binary, calling itself. Assembly and the hub's own turn loop.
    Kernel,
    /// A paired client. One process serves one user, so a device is that
    /// user on one machine — not a tenant.
    Device { id: String },
    /// A compile-time module, by its manifest id, calling through its
    /// `ModuleContext`; or a runtime application acting as itself, by its id.
    Module { id: String },
}

impl Subject {
    pub fn label(&self) -> String {
        match self {
            Subject::Kernel => "kernel".into(),
            Subject::Device { id } => format!("device:{id}"),
            Subject::Module { id } => format!("module:{id}"),
        }
    }
}
