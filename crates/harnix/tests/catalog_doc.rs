//! The catalog and its documentation, compared.
//!
//! A list of extension points that can go stale is worse than none: it gets
//! quoted as fact. So the table in `docs/contribution_points.md` is rendered
//! from `harnix_ext::catalog` and this fails when the two disagree —
//! change the catalog, not the table.

use std::path::{Path, PathBuf};

const BEGIN: &str = "<!-- BEGIN GENERATED TABLE — see harnix_ext::catalog::render_markdown -->";
const END: &str = "<!-- END GENERATED TABLE -->";
const EXT_BEGIN: &str = "<!-- BEGIN GENERATED EXTENSION TABLE — see harnix_ext::catalog::render_extension_markdown -->";
const EXT_END: &str = "<!-- END GENERATED EXTENSION TABLE -->";

fn doc_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/harnix is two levels below the root")
        .join("docs/contribution_points.md")
}

/// The text between two markers in the document.
fn between(text: &str, path: &Path, begin: &str, end: &str) -> String {
    let start = text
        .find(begin)
        .unwrap_or_else(|| panic!("{} has no marker `{begin}`", path.display()));
    let stop = text[start..]
        .find(end)
        .map(|at| start + at)
        .unwrap_or_else(|| panic!("{} has no marker `{end}`", path.display()));
    text[start + begin.len()..stop].trim().to_string()
}

#[test]
fn the_documented_table_is_the_generated_one() {
    let path = doc_path();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(
        between(&text, &path, BEGIN, END),
        harnix_ext::catalog::render_markdown().trim(),
        "\n{} is out of date. Regenerate it from harnix_ext::catalog.\n",
        path.display()
    );
    assert_eq!(
        between(&text, &path, EXT_BEGIN, EXT_END),
        harnix_ext::catalog::render_extension_markdown().trim(),
        "\n{} is out of date: the extensions' points. Regenerate it from harnix_ext::catalog.\n",
        path.display()
    );
}
