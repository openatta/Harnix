//! An extension's id: what its names, its routes, its directory and its subject are under.

use serde::{Deserialize, Serialize};

/// First segments no extension package may have: the kernel's, the engine's, and the words the
/// wire and the table build names with.
pub const RESERVED: &[&str] = &["harnix", "engine", "ext", "topic", "route", "proc", "ctl"];

/// An extension's id. A native extension's is one segment (`system`, `fleet`); a package's is
/// `<publisher>.<name>` (`example.showcase`). Each segment is lowercase letters, digits and `-`,
/// starting with a letter; the whole is at most 64 bytes, so an id is never a path out of the
/// directory it names.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ExtId(String);

/// Why a string is not an extension id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdError(String);

impl std::fmt::Display for IdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for IdError {}

fn segment(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some('a'..='z')) && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '-'))
}

impl ExtId {
    /// A native extension's id: one segment.
    pub fn native(s: &str) -> Result<Self, IdError> {
        if s.len() <= 64 && segment(s) {
            Ok(Self(s.to_string()))
        } else {
            Err(IdError(format!(
                "`{s}` is not a native extension's id: lowercase letters, digits and `-`, starting with a letter"
            )))
        }
    }

    /// A package's id: two segments or more, the first not a reserved word.
    pub fn package(s: &str) -> Result<Self, IdError> {
        let id = Self::parse(s)?;
        let first = id.first_segment();
        if !id.0.contains('.') {
            return Err(IdError(format!("`{s}` is not a package's id: `<publisher>.<name>`")));
        }
        if RESERVED.contains(&first) {
            return Err(IdError(format!("`{s}` is not a package's id: `{first}` is reserved")));
        }
        Ok(id)
    }

    /// Either kind: one segment or more. What a runtime application's id is held to until its ids
    /// are a package's.
    pub fn parse(s: &str) -> Result<Self, IdError> {
        if s.len() <= 64 && s.split('.').all(segment) {
            Ok(Self(s.to_string()))
        } else {
            Err(IdError(format!(
                "`{s}` is not an extension id: dot-separated segments of lowercase letters, digits and `-`, each starting with a letter"
            )))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The part before the first dot: a package's publisher, a native extension's whole id.
    pub fn first_segment(&self) -> &str {
        self.0.split('.').next().unwrap_or(&self.0)
    }
}

impl std::fmt::Display for ExtId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for ExtId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl PartialEq<str> for ExtId {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for ExtId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl TryFrom<String> for ExtId {
    type Error = IdError;
    fn try_from(s: String) -> Result<Self, IdError> {
        Self::parse(&s)
    }
}

impl From<ExtId> for String {
    fn from(id: ExtId) -> String {
        id.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_is_native_or_a_packages() {
        assert!(ExtId::native("system").is_ok());
        assert!(ExtId::native("acme.notes").is_err());
        assert!(ExtId::package("acme.notes").is_ok());
        assert!(ExtId::package("acme").is_err(), "one segment is a native id");
        for reserved in RESERVED {
            assert!(ExtId::package(&format!("{reserved}.x")).is_err(), "{reserved}");
        }
        for bad in ["", "Acme", "a..b", "1a", "a/b", "../x"] {
            assert!(ExtId::parse(bad).is_err(), "{bad:?}");
        }
        assert!(ExtId::parse(&format!("a.{}", "b".repeat(63))).is_err());
        assert_eq!(ExtId::package("acme.notes").unwrap().first_segment(), "acme");
        let parsed: Result<ExtId, _> = serde_json::from_str("\"Not An Id\"");
        assert!(parsed.is_err(), "an id read off the wire is checked");
    }
}
