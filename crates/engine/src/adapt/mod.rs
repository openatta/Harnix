//! Harnix's engine types as Agix's: each adapter is the one place its Agix trait is implemented,
//! and each has a test beside it that reads that trait's methods out of Agix's source, so a
//! method Agix adds turns a test red here instead of being answered by a default no one chose.

pub(crate) mod gate;
pub(crate) mod provider;
pub(crate) mod scene;
pub(crate) mod tool;

/// The names of the methods a trait declares in `source`: the `fn`s one brace deep from the line
/// that starts with `header` to the brace that closes it.
#[cfg(test)]
pub(crate) fn trait_methods(source: &str, header: &str) -> Vec<String> {
    let start = source
        .lines()
        .position(|l| l.trim_start().starts_with(header))
        .unwrap_or_else(|| panic!("no `{header}` in the source"));
    let mut names = Vec::new();
    let mut depth = 0i32;
    let mut opened = false;
    for line in source.lines().skip(start) {
        let trimmed = line.trim_start();
        if depth == 1 {
            let rest = trimmed.strip_prefix("async fn ").or_else(|| trimmed.strip_prefix("fn "));
            if let Some(rest) = rest {
                names.push(rest.split(['(', '<']).next().unwrap().to_string());
            }
        }
        let opens = line.matches('{').count() as i32;
        depth += opens - line.matches('}').count() as i32;
        opened |= opens > 0;
        if opened && depth == 0 {
            break;
        }
    }
    names
}

/// The methods an `impl` block in `source` writes: the `fn`s between the line starting with
/// `header` and the first line that is a lone `}`.
#[cfg(test)]
pub(crate) fn impl_methods(source: &str, header: &str) -> Vec<String> {
    let start = source.find(header).unwrap_or_else(|| panic!("no `{header}` in the source"));
    let end = start + source[start..].find("\n}\n").expect("the impl's end");
    source[start..end]
        .lines()
        .filter_map(|l| {
            let t = l.trim_start();
            t.strip_prefix("async fn ").or_else(|| t.strip_prefix("fn "))
        })
        .map(|r| r.split(['(', '<']).next().unwrap().to_string())
        .collect()
}

/// `written` and `defaulted` together are every method `agix` declares, and `written` is exactly
/// what the adapter implements.
#[cfg(test)]
pub(crate) fn assert_covers(agix: Vec<String>, implemented: Vec<String>, written: &[&str], defaulted: &[&str]) {
    let sorted = |mut v: Vec<String>| {
        v.sort();
        v
    };
    let listed = sorted(written.iter().chain(defaulted).map(|s| s.to_string()).collect());
    assert_eq!(listed, sorted(agix), "Agix's trait and this adapter's lists name different methods");
    assert_eq!(
        sorted(implemented),
        sorted(written.iter().map(|s| s.to_string()).collect()),
        "the adapter implements other methods than it lists"
    );
}

/// Agix's source file at `rel` (under `agix/`), beside this checkout.
#[cfg(test)]
pub(crate) fn agix_source(rel: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../agix").join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}
