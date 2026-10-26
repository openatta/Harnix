//! The interface is compiled into the binary from a built `dist` directory: Harnix's own
//! `ui/dist`, or the one an application built with its spaces in it (`HARNIX_UI_DIST`, design
//! §11.4). Without it this crate does not build, and says what to run — an empty interface embedded
//! silently would be a binary that serves a blank page.

use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=HARNIX_UI_DIST");
    let dist = match std::env::var_os("HARNIX_UI_DIST") {
        Some(path) => PathBuf::from(path),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/dist"),
    };
    let dist = dist.canonicalize().unwrap_or(dist);
    println!("cargo:rerun-if-changed={}", dist.display());
    if !dist.join("index.html").is_file() {
        panic!(
            "\n\nThe interface is not built: {} has no index.html.\n\
             Run `pnpm install && pnpm --filter @harnix/shell build` first \
             (scripts/build.sh does both, then cargo build).\n\n",
            dist.display()
        );
    }
    println!("cargo:rustc-env=HARNIX_UI_DIST={}", dist.display());
}
