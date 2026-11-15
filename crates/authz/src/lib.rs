//! Deciding one call against one row, and the record of it.
//!
//! A row says how far it is open ([`harnix_kernel::Access`]); [`decide`] is the one place that
//! reads that against a subject. The route table holds the rows and asks here for every call, so
//! "may this subject call this" has one answer however the call arrived.
//!
//! Kinds of subject, judged differently. A device is a person, and in a single-user process the
//! answer to "may this person do it" is nearly always yes — but a device holds no capability. An
//! extension is let in by a row's declaration, and holds what it was granted: privilege comes from
//! capabilities, never from which extension it is. The kernel is this binary, and reaches
//! everything the hub does not own. An engine is named in the audit of what an extension's engine
//! sends, and no row is open to it.
//!
//! Authorization happens at the method, not at the connection. A multi-channel topology gives
//! one subject several connections; hanging the decision on a connection would give several
//! answers to one question.
//!
//! This layer does not know what a frame looks like or which channel carried it.

mod admission;
mod audit;
mod devices;

pub use admission::{ChildBindings, ChildSubject};
pub use audit::{Audit, AuditEntry, AuditSink, Change, Decision};
pub use devices::challenge as devices_challenge;
pub use devices::{tokens_equal, Device, Devices, PairError, Persist, PAIRING_TTL};

use harnix_contract::s1::{Capability, ExtId, RowScope};
use harnix_contract::Subject;
use harnix_kernel::{Access, GrantBook};

/// Why an extension is refused a row whose capability it does not hold.
pub const LACKS_CAPABILITY: &str = "this needs a capability this extension was not granted";

/// Whether `subject` may call a row open this far: let in by the row, and holding what the row
/// requires. Refused with the row's own reason when it has one — refused on purpose — and
/// otherwise with what was missing.
pub fn decide(subject: &Subject, grants: &dyn GrantBook, access: &Access) -> Decision {
    let refused = |fallback: &'static str| Decision::Refuse(access.refusal().unwrap_or(fallback));
    if access.is_hub_owned() {
        return refused("the hub owns this");
    }
    match subject {
        Subject::Kernel => Decision::Allow,
        Subject::Device { .. } if access.opens_devices() => Decision::Allow,
        Subject::Device { .. } => refused(
            "this method is not reachable by a device; it was declared for the kernel or for extensions",
        ),
        Subject::Extension { id } if !access.opens_extension(id) => {
            refused("this method was not declared for this extension")
        }
        Subject::Extension { id } => match access.requirement() {
            Some(cap) if !grants.holds(id, cap) => refused(LACKS_CAPABILITY),
            _ => Decision::Allow,
        },
        _ => refused("this method was not declared for this caller"),
    }
}

/// The capability `subject` was refused for, when that was why: what a refusal names in its data.
pub fn missing_capability<'a>(subject: &Subject, grants: &dyn GrantBook, access: &'a Access) -> Option<&'a Capability> {
    let id = subject.extension_id()?;
    access
        .requirement()
        .filter(|cap| access.opens_extension(id) && !grants.holds(id, cap))
}

/// Why a package, or a page in a frame, is refused a row about a space or a session.
pub const OUTSIDE_OWN_SPACE: &str = "a package reaches only its own space and that space's sessions";

/// How far a call by (or on behalf of) a package is narrowed: a row about a space or a session
/// only with a space claimed, and that space its own — `<id>` or `<id>.<name>`. Whether the
/// session is bound to the space claimed is the hub's to check, as for every claim.
pub fn narrow(ext: &ExtId, scope: RowScope, claimed: Option<&str>) -> Decision {
    match scope {
        RowScope::Open => Decision::Allow,
        _ => match claimed {
            Some(space) if own_space(ext, space) => Decision::Allow,
            _ => Decision::Refuse(OUTSIDE_OWN_SPACE),
        },
    }
}

/// Whether `space` is in `ext`'s namespace, by whole segments.
pub fn own_space(ext: &ExtId, space: &str) -> bool {
    let id = ext.as_str();
    space == id || space.strip_prefix(id).is_some_and(|rest| rest.starts_with('.'))
}

#[cfg(test)]
mod tests {
    use harnix_contract::s1::{caps, Trust};
    use harnix_kernel::GrantTable;

    use super::*;

    fn device() -> Subject {
        Subject::device("d")
    }

    fn id(s: &str) -> ExtId {
        ExtId::native(s).or_else(|_| ExtId::package(s)).unwrap()
    }

    fn ext(s: &str) -> Subject {
        Subject::extension(id(s))
    }

    #[test]
    fn a_declaration_admits_exactly_who_it_names_and_the_kernel() {
        let grants = GrantTable::new();
        let access = Access::device().with_extension(id("trade"));
        assert_eq!(decide(&device(), &grants, &access), Decision::Allow);
        assert_eq!(decide(&ext("trade"), &grants, &access), Decision::Allow);
        let only = Access::only(id("trade"));
        assert!(matches!(decide(&ext("other"), &grants, &only), Decision::Refuse(_)));
        assert!(matches!(decide(&device(), &grants, &only), Decision::Refuse(_)));
        assert_eq!(decide(&Subject::Kernel, &grants, &only), Decision::Allow);
    }

    #[test]
    fn a_capability_row_opens_to_its_holders_and_the_hubs_own_to_nobody() {
        let grants = GrantTable::new();
        grants.set(id("settings"), [caps::ENGINE_CONFIG_WRITE], Trust::Origin);
        let access = Access::capability(caps::ENGINE_CONFIG_WRITE).refusing("no");
        assert_eq!(decide(&ext("settings"), &grants, &access), Decision::Allow);
        assert_eq!(decide(&ext("other"), &grants, &access), Decision::Refuse("no"));
        assert_eq!(missing_capability(&ext("other"), &grants, &access), Some(&caps::ENGINE_CONFIG_WRITE));
        assert_eq!(missing_capability(&ext("settings"), &grants, &access), None);
        assert_eq!(decide(&device(), &grants, &access), Decision::Refuse("no"));
        assert_eq!(decide(&Subject::Kernel, &grants, &access), Decision::Allow);

        let hubs = Access::hub_owned("hub");
        assert_eq!(decide(&ext("settings"), &grants, &hubs), Decision::Refuse("hub"));
        assert_eq!(decide(&Subject::Kernel, &grants, &hubs), Decision::Refuse("hub"));
    }

    #[test]
    fn a_package_is_narrowed_to_its_own_space() {
        let me = id("acme.notes");
        assert_eq!(narrow(&me, RowScope::Open, None), Decision::Allow);
        assert_eq!(narrow(&me, RowScope::Session, Some("acme.notes")), Decision::Allow);
        assert_eq!(narrow(&me, RowScope::Space, Some("acme.notes.board")), Decision::Allow);
        assert_eq!(narrow(&me, RowScope::Space, None), Decision::Refuse(OUTSIDE_OWN_SPACE));
        assert_eq!(narrow(&me, RowScope::Session, Some("sessions")), Decision::Refuse(OUTSIDE_OWN_SPACE));
        assert_eq!(narrow(&me, RowScope::Session, Some("acme.notesx")), Decision::Refuse(OUTSIDE_OWN_SPACE));
    }
}
