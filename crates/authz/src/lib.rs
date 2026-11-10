//! Deciding one call against one row, and the record of it.
//!
//! A row says how far it is open ([`harnix_kernel::Access`]); [`decide`] is the one place that
//! reads that against a subject. The route table holds the rows and asks here for every call, so
//! "may this subject call this" has one answer however the call arrived.
//!
//! Three kinds of subject, judged differently (concept_and_architecture.md §3.4). A device is a
//! person, and in a single-user process the answer to "may this person do it" is nearly always
//! yes. A module is code the application author linked in: trusted like the kernel, and still a
//! subject of its own, so a row says which modules may call it and the audit says which one did.
//! The kernel is this binary.
//!
//! Authorization happens at the method, not at the connection. A multi-channel topology gives
//! one subject several connections; hanging the decision on a connection would give several
//! answers to one question.
//!
//! This layer does not know what a frame looks like or which channel carried it.

mod audit;
mod devices;

pub use audit::{Audit, AuditEntry, AuditSink, Change, Decision};
pub use devices::challenge as devices_challenge;
pub use devices::{tokens_equal, Device, Devices, PairError, Persist, PAIRING_TTL};

use harnix_contract::Subject;
use harnix_kernel::Access;

/// One kind of caller a module method, topic or route is declared for.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Principal {
    /// A paired device: a person.
    Device,
    /// One module, by id.
    Module(String),
    /// Any module.
    AnyModule,
    /// This binary.
    Kernel,
}

/// How far a row declared for these principals is open.
pub fn access_for(principals: &[Principal]) -> Access {
    let mut access = Access::nobody();
    for p in principals {
        access = match p {
            Principal::Device => access.with_device(),
            Principal::Kernel => access.with_kernel(),
            Principal::AnyModule => access.with_extensions(),
            Principal::Module(id) => access.with_extension(id.clone()),
        };
    }
    access
}

/// Whether `subject` may call a row open this far. Refused with the row's own reason when it has
/// one — refused on purpose — and otherwise with what was missing.
pub fn decide(subject: &Subject, access: &Access) -> Decision {
    let allowed = match subject {
        Subject::Kernel => access.opens_kernel(),
        Subject::Device { .. } => access.opens_devices(),
        Subject::Extension { id } => !access.is_hub_owned() && access.opens_extension(id.as_str()),
        _ => false,
    };
    if allowed {
        return Decision::Allow;
    }
    Decision::Refuse(access.refusal().unwrap_or(match subject {
        Subject::Kernel => "this is not the kernel's to call",
        Subject::Device { .. } => {
            "this method is not reachable by a device; it was declared for the kernel or for modules"
        }
        _ => "this method was not declared for, or granted to, this extension",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> Subject {
        Subject::Device { id: "d".into() }
    }

    fn module(id: &str) -> Subject {
        Subject::extension(harnix_contract::ExtId::parse(id).unwrap())
    }

    #[test]
    fn a_declaration_admits_exactly_who_it_names() {
        let access = access_for(&[Principal::Device, Principal::Module("trade".into())]);
        assert_eq!(decide(&device(), &access), Decision::Allow);
        assert_eq!(decide(&module("trade"), &access), Decision::Allow);
        assert!(matches!(decide(&module("other"), &access), Decision::Refuse(_)));
        assert!(matches!(decide(&Subject::Kernel, &access), Decision::Refuse(_)));
    }

    #[test]
    fn a_grant_opens_a_refusal_to_one_module_and_the_hubs_own_to_nobody() {
        let mut access = Access::refused("no");
        access.grant("settings");
        assert_eq!(decide(&module("settings"), &access), Decision::Allow);
        assert_eq!(decide(&module("other"), &access), Decision::Refuse("no"));
        assert_eq!(decide(&device(), &access), Decision::Refuse("no"));

        let mut hubs = Access::hub_owned("hub");
        hubs.grant("settings");
        assert_eq!(decide(&module("settings"), &hubs), Decision::Refuse("hub"));
        assert_eq!(decide(&Subject::Kernel, &hubs), Decision::Refuse("hub"));
    }
}
