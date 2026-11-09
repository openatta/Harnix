//! What a runtime application's id is: the one rule the install records (`harnix-builtin`) and the
//! packages (`harnix-app-package`) both hold an id to, kept here because both depend on this crate
//! and on nothing else in common.

/// Whether `id` can be an application's id: dot-separated segments of lowercase letters, digits
/// and `-`, each starting with a letter, 64 bytes at most. A directory name under the install
/// records' directory is one exactly when this holds, so an id is never a path out of it.
pub fn valid_id(id: &str) -> bool {
    id.len() <= 64
        && id.split('.').all(|segment| {
            let mut chars = segment.chars();
            matches!(chars.next(), Some('a'..='z'))
                && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '-'))
        })
}

#[cfg(test)]
mod tests {
    use super::valid_id;

    #[test]
    fn an_id_is_dotted_lowercase_segments() {
        for id in ["acme.notes", "a", "a-1.b2", "example.showcase"] {
            assert!(valid_id(id), "{id:?}");
        }
        for id in ["Acme.notes", "acme..notes", "1acme", "acme.notes_x", "", "../etc", "acme/notes"] {
            assert!(!valid_id(id), "{id:?}");
        }
        assert!(!valid_id(&format!("a.{}", "b".repeat(63))));
    }
}
