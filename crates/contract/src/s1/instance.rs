//! Whose engine an instance runs.

use super::ExtId;

/// Whose engine an instance runs: what the hub trusts its frames as far as, and what an
/// extension's view of the instance says.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceTrust {
    /// One the product started or connected to: in this process, spawned, or attached.
    Product,
    /// An engine in an extension's own process: under third-party control.
    Extension(ExtId),
}

impl InstanceTrust {
    pub fn is_extension(&self) -> bool {
        matches!(self, InstanceTrust::Extension(_))
    }

    /// As the wire says it: `"product"` or `"extension"`.
    pub fn label(&self) -> &'static str {
        match self {
            InstanceTrust::Product => "product",
            InstanceTrust::Extension(_) => "extension",
        }
    }
}
