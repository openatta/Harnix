//! The table as `harnix.diagnostics` reports it and `crates/harnix/tests/method-table.snap.json`
//! pins it: one `{name, kind, owner, access}` per row, sorted by kind and then name, and the
//! aliases beside them.

use harnix_kernel::Owner;
use serde_json::{json, Value};

use crate::table::{RowKind, RouteTable};

pub(crate) fn of(table: &RouteTable) -> Value {
    let mut rows: Vec<(RowKind, String, Value)> = table
        .entries()
        .map(|entry| {
            let row = &entry.row;
            let key = row.name();
            let name = match row.kind() {
                RowKind::Method => key,
                RowKind::Topic => key.strip_prefix(crate::names::TOPIC).unwrap_or(key),
                RowKind::Route => key.strip_prefix(crate::names::ROUTE).unwrap_or(key),
            };
            let access = row.access();
            // What only a grant opens, by the row's own name: a row of the kernel's or the
            // engine's that nothing but the kernel reaches by itself, and that a module's manifest
            // may still name.
            let by_grant = matches!(entry.owner, Owner::Engine | Owner::Kernel(_))
                && !access.opens_devices()
                && !access.is_hub_owned()
                && !access.opens_every_extension();
            let requires: Vec<&str> = if by_grant { vec![key] } else { vec![] };
            let value = json!({
                "name": name,
                "kind": row.kind().as_str(),
                "owner": entry.owner.label(),
                "access": {
                    "device": access.opens_devices(),
                    "extensions": access.extensions(),
                    "engine": false,
                    "requires": requires,
                    "scope": null,
                    "refused": access.refusal(),
                    "hub_owned": access.is_hub_owned(),
                },
            });
            (row.kind(), name.to_string(), value)
        })
        .collect();
    rows.sort_by(|a, b| (a.0.as_str(), &a.1).cmp(&(b.0.as_str(), &b.1)));
    let aliases: Vec<Value> = table
        .aliases()
        .map(|(old, new)| json!({"name": old, "to": new}))
        .collect();
    json!({"rows": rows.into_iter().map(|r| r.2).collect::<Vec<_>>(), "aliases": aliases})
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use harnix_kernel::{Access, CallContext, Handler, KernelPart, Rpc};

    use super::*;
    use crate::table::Row;

    struct Nothing;

    #[async_trait::async_trait]
    impl Handler for Nothing {
        async fn call(&self, _: CallContext, _: Value) -> Rpc<Value> {
            Ok(Value::Null)
        }
    }

    #[test]
    fn a_row_is_projected_with_its_owner_and_what_opens_it() {
        let mut table = RouteTable::new();
        table
            .install(
                &Owner::Engine,
                vec![
                    Row::method("plugin.install", Access::kernel(), Arc::new(Nothing)),
                    Row::method("config.set", Access::refused("no"), Arc::new(Nothing)),
                    Row::method("plugin.list", Access::device(), Arc::new(Nothing)),
                ],
            )
            .unwrap();
        table
            .install(&Owner::Kernel(KernelPart::Hub), vec![Row::topic("harnix.instances", Access::device())])
            .unwrap();
        table.grant("plugin.install", "packages").unwrap();
        table.grant("plugin.list", "packages").unwrap();
        table.alias("plugin.add", "plugin.install").unwrap();
        let snap = of(&table);
        let rows = snap["rows"].as_array().unwrap();
        let names: Vec<_> = rows.iter().map(|r| (r["kind"].as_str().unwrap(), r["name"].as_str().unwrap())).collect();
        assert_eq!(
            names,
            [("method", "config.set"), ("method", "plugin.install"), ("method", "plugin.list"), ("topic", "harnix.instances")]
        );
        assert_eq!(rows[1]["owner"], "engine");
        assert_eq!(rows[1]["access"]["requires"], json!(["plugin.install"]));
        assert_eq!(rows[1]["access"]["extensions"], json!(["packages"]));
        assert_eq!(rows[0]["access"]["refused"], "no");
        assert_eq!(rows[2]["access"]["extensions"], json!(["*", "packages"]));
        assert_eq!(rows[2]["access"]["requires"], json!([]));
        assert_eq!(rows[3]["owner"], "kernel:hub");
        assert_eq!(snap["aliases"], json!([{"name": "plugin.add", "to": "plugin.install"}]));
    }
}
