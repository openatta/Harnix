//! Records the target triple this crate is built for. It is part of the
//! engine fingerprint, and `TARGET` is only visible to build scripts.
fn main() {
    let target = std::env::var("TARGET").expect("cargo sets TARGET for build scripts");
    println!("cargo:rustc-env=HARNIX_WASM_TARGET={target}");
    println!("cargo:rerun-if-changed=build.rs");
}
