//! Generated host-side bindings for `harnix:ext@1.0.0`.
//!
//! `wit/ext.wit` is the contract; this is the typed glue wasmtime derives
//! from it. Nothing here is written by hand: host and guest types maintained
//! separately drift, and a drifting ABI fails at call time rather than at
//! build time.
//!
//! Generated for the `backend-ext-events` world, a superset of every other. A
//! component of any world is driven through its exported interfaces one by one
//! (`GuestIndices`), so one set of bindings serves them all, and an interface a
//! component does not export — `events`, most often — is simply absent.

wasmtime::component::bindgen!({
    path: "wit",
    world: "backend-ext-events",
    // Host functions do real I/O (a hub call, an HTTP request), so a guest
    // calling one must be able to suspend instead of blocking a runtime
    // thread. That makes every export a future too.
    imports: { default: async },
    exports: { default: async },
});

/// The package the WIT is published under. A component carries it in its
/// export names, which is how its world is told apart from the manifest's
/// claim about it.
pub const WIT_PACKAGE: &str = "harnix:ext@1.0.0";

#[cfg(test)]
mod tests {
    #[test]
    fn the_package_constant_is_the_wit_package() {
        let wit = include_str!("../wit/ext.wit");
        assert!(
            wit.contains(&format!("package {};", super::WIT_PACKAGE)),
            "wit/ext.wit is not `{}`",
            super::WIT_PACKAGE
        );
    }
}
