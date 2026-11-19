//! The names a product gives the engine: the directory it keeps in each project, and what it calls
//! itself. Harnix's own are `.harnix` and `Harnix`; a branded application's follow its `[brand]`
//! (`.<product_id>`, its name), so its projects and its prompts carry its name, not Harnix's.
//!
//! Both are checked here by the engine's own rules, so a profile that names one the engine would
//! refuse stops the start before anything is built, saying which and why.
//!
//! The product's name is the engine's `product`. The engine's own name is another (`agix`: its
//! script library's global, its errors' name, the state directory under `$HOME` it guards as its
//! own); Harnix leaves it at the engine's default, so a plugin written for Agix runs as it is.

use std::path::{Path, PathBuf};

/// The project data directory of a product that names none: Harnix's.
pub const DEFAULT_PROJECT_DIR: &str = ".harnix";

/// The name the engine goes by in a product that names none: Harnix's.
pub const DEFAULT_PRODUCT_NAME: &str = "Harnix";

/// Project directories other tools own, which a product's cannot be: git's, the cross-tool skills
/// directory, the other agents' configuration roots, the editors' and forges' settings. The
/// engine's list; `crates/engine` checks the two agree.
const OTHER_TOOLS_DIRS: &[&str] = &[".git", ".agents", ".claude", ".codex", ".cursor", ".github", ".vscode", ".idea"];

/// Where the engine puts its worktrees, at a repository's root: not a project directory either.
const ENGINE_WORKTREES_DIR: &str = ".wkts";

/// What the engine keeps in a project directory's `state/` under its own names: a product's name
/// in lowercase is none of them, as `state/<product id>/` is the product's. The engine's list;
/// `crates/engine` checks the engine refuses each.
const STATE_RESERVED_NAMES: &[&str] =
    &["memory", "scratchpad", "teams", "telemetry", "tool-results", "history", "imported", "sessions", "cache", "tmp"];

/// The directory the engine keeps in each project, and the name it goes by.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductNames {
    project_dir: String,
    product_name: String,
}

impl Default for ProductNames {
    /// Harnix's own: `.harnix`, `Harnix`.
    fn default() -> Self {
        Self { project_dir: DEFAULT_PROJECT_DIR.into(), product_name: DEFAULT_PRODUCT_NAME.into() }
    }
}

impl ProductNames {
    /// Both names, checked ([`check_project_dir`], [`check_product_name`]).
    pub fn new(project_dir: &str, product_name: &str) -> Result<Self, NameError> {
        check_project_dir(project_dir).map_err(|reason| NameError::new("project_dir", project_dir, reason))?;
        check_product_name(product_name).map_err(|reason| NameError::new("product_name", product_name, reason))?;
        Ok(Self { project_dir: project_dir.into(), product_name: product_name.into() })
    }

    /// The project data directory's name, `.harnix` in Harnix: `<project>/<project_dir>` holds the
    /// engine's project tier and what Harnix writes for a project (a flow's `runs/`, `flows/`).
    pub fn project_dir(&self) -> &str {
        &self.project_dir
    }

    /// The name the engine goes by, `Harnix` in Harnix: the model's prompts, the `User-Agent` of
    /// its requests, its worktrees' branches (`<lowercase name>/worktree-`). The engine's
    /// `product`, not its own name (`agix`).
    pub fn product_name(&self) -> &str {
        &self.product_name
    }

    /// The name as a machine reads it — a branch prefix, a path component (the product's own
    /// `<project_dir>/state/<product id>/`, which the engine neither reads nor writes): lowercase.
    pub fn product_id(&self) -> String {
        self.product_name.to_ascii_lowercase()
    }

    /// `<project>/<project_dir>`.
    pub fn data_dir_of(&self, project: &Path) -> PathBuf {
        project.join(&self.project_dir)
    }
}

/// A name the engine would refuse, and why.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameError {
    /// `project_dir` or `product_name`.
    pub field: &'static str,
    pub name: String,
    pub reason: &'static str,
}

impl NameError {
    fn new(field: &'static str, name: &str, reason: &'static str) -> Self {
        Self { field, name: name.to_string(), reason }
    }
}

impl std::fmt::Display for NameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} `{}`: {}", self.field, self.name, self.reason)
    }
}

impl std::error::Error for NameError {}

/// Whether `name` can be a project data directory's: one path component of `[a-z0-9._-]`, at most
/// 64 long, starting with `.`, not `.` or `..`, no other tool's, and not the engine's worktrees'.
pub fn check_project_dir(name: &str) -> Result<(), &'static str> {
    if name.is_empty() {
        return Err("it is empty");
    }
    if name.len() > 64 {
        return Err("it is longer than 64 characters");
    }
    if !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "._-".contains(c)) {
        return Err("only lowercase letters, digits, `.`, `_` and `-` are allowed");
    }
    if name == "." || name == ".." {
        return Err("it is not a directory of its own");
    }
    if !name.starts_with('.') {
        return Err("it must start with `.`, as a directory a project does not use for its own files");
    }
    if OTHER_TOOLS_DIRS.contains(&name) {
        return Err("that directory belongs to another tool");
    }
    if name == ENGINE_WORKTREES_DIR {
        return Err("that is where the engine's worktrees go");
    }
    Ok(())
}

/// Whether `name` can be the name the engine goes by: an ASCII letter, then up to 31 ASCII
/// letters, digits, `-` or `_` — a path component, a git ref component and an HTTP token as is —
/// whose lowercase is none of the names the engine keeps in a project's `state/`.
pub fn check_product_name(name: &str) -> Result<(), &'static str> {
    let mut chars = name.chars();
    match chars.next() {
        None => return Err("it is empty"),
        Some(c) if !c.is_ascii_alphabetic() => return Err("it must start with an ASCII letter"),
        Some(_) => {}
    }
    if name.len() > 32 {
        return Err("it is longer than 32 characters");
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("only ASCII letters, digits, `-` and `_` are allowed");
    }
    if STATE_RESERVED_NAMES.contains(&name.to_ascii_lowercase().as_str()) {
        return Err("the engine keeps its own state under that name in a project's `state/`");
    }
    Ok(())
}

/// Names the engine refuses and names it takes, for `crates/engine` to hold the engine to the
/// same answers.
#[doc(hidden)]
pub mod samples {
    pub const BAD_PROJECT_DIRS: &[&str] = &["", "harnix", ".", "..", ".Acme", ".a/b", ".git", ".claude", ".agents", ".github", ".wkts"];
    pub const GOOD_PROJECT_DIRS: &[&str] = &[".harnix", ".acme", ".hello-app", ".a.b_c-1"];
    pub const BAD_PRODUCT_NAMES: &[&str] = &["", "1acme", "Acme Desk", "Acme/1", "Héllo", "Memory", "scratchpad", "History", "tool-results"];
    pub const GOOD_PRODUCT_NAMES: &[&str] = &["Harnix", "Acme", "hello-app", "Helm_2"];
    /// The names the engine keeps in a project's `state/`: no product's, in any case.
    pub const RESERVED_PRODUCT_NAMES: &[&str] = super::STATE_RESERVED_NAMES;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harnix_keeps_its_own_names() {
        let names = ProductNames::default();
        assert_eq!((names.project_dir(), names.product_name(), names.product_id().as_str()), (".harnix", "Harnix", "harnix"));
        assert_eq!(names.data_dir_of(Path::new("/p")), PathBuf::from("/p/.harnix"));
        assert_eq!(ProductNames::new(".harnix", "Harnix").unwrap(), names);
    }

    #[test]
    fn a_name_the_engine_would_refuse_is_refused_saying_which_and_why() {
        for bad in samples::BAD_PROJECT_DIRS {
            let e = ProductNames::new(bad, "Acme").unwrap_err();
            assert_eq!((e.field, e.name.as_str()), ("project_dir", *bad));
        }
        for bad in samples::BAD_PRODUCT_NAMES {
            let e = ProductNames::new(".acme", bad).unwrap_err();
            assert_eq!((e.field, e.name.as_str()), ("product_name", *bad));
        }
        assert!(check_project_dir(&format!(".{}", "a".repeat(64))).is_err());
        assert!(check_product_name(&format!("A{}", "b".repeat(32))).is_err());
        for reserved in samples::RESERVED_PRODUCT_NAMES {
            assert!(check_product_name(reserved).is_err(), "{reserved}");
            assert!(check_product_name(&reserved.to_ascii_uppercase()).is_err(), "{reserved}");
        }
        assert!(check_product_name("State").is_ok(), "`state` itself is not one of the names in it");
        assert_eq!(
            ProductNames::new(".git", "Acme").unwrap_err().to_string(),
            "project_dir `.git`: that directory belongs to another tool"
        );
    }

    #[test]
    fn names_the_engine_takes_are_taken() {
        for dir in samples::GOOD_PROJECT_DIRS {
            for name in samples::GOOD_PRODUCT_NAMES {
                let names = ProductNames::new(dir, name).unwrap();
                assert_eq!((names.project_dir(), names.product_name()), (*dir, *name));
            }
        }
    }
}
