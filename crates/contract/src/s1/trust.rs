//! How far an extension is trusted: written in its registration, in one place.

use std::collections::BTreeSet;

/// Where a package's code runs.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageCarrier {
    /// In this process, in a WASM virtual machine.
    Wasm,
    /// In a process of its own.
    Process,
}

/// How far an extension is trusted.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trust {
    /// By origin: linked into the product. Each row's access decides; there is no list of calls
    /// besides.
    Origin,
    /// By consent: a package. It reaches only what its consent lists and its own names, and every
    /// row it reaches is narrowed to its own space and that space's sessions.
    Consent {
        calls: BTreeSet<String>,
        carrier: PackageCarrier,
    },
}

impl Trust {
    pub fn consent(calls: impl IntoIterator<Item = String>, carrier: PackageCarrier) -> Self {
        Trust::Consent {
            calls: calls.into_iter().collect(),
            carrier,
        }
    }

    /// What its consent lists, when it is a package.
    pub fn calls(&self) -> Option<&BTreeSet<String>> {
        match self {
            Trust::Consent { calls, .. } => Some(calls),
            Trust::Origin => None,
        }
    }

    pub fn is_consent(&self) -> bool {
        matches!(self, Trust::Consent { .. })
    }
}
