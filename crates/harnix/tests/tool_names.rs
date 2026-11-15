//! The rule that names a package's tool for a session is written twice, since the assembly may
//! not depend on the package crate (`layering.rs`, l11): here the two are held to each other.

#[test]
fn the_host_and_the_package_crate_name_a_tool_alike() {
    let ids = ["flow", "my.ext", "my-ext", "My.Ext-2", "a_b", "x.y-z_w", "Acme.Notes"];
    let tools = ["echo", "do-it", "Do_It", "a.b", "UPPER-case", "with_under", "x-y_z"];
    for id in ids {
        for tool in tools {
            assert_eq!(
                harnix_assembly::testing::session_tool_name(id, tool),
                harnix_package::tool_name(id, tool),
                "`{id}` / `{tool}`"
            );
        }
    }
}
