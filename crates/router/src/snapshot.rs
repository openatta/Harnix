//! The table as `harnix.diagnostics` reports it and `crates/harnix/tests/method-table.snap.json`
//! pins it: one `{name, kind, owner, access}` per row, sorted by kind and then name.

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
            let requires: Vec<&str> = access.requirement().map(|c| c.as_str()).into_iter().collect();
            let value = json!({
                "name": name,
                "kind": row.kind().as_str(),
                "owner": entry.owner.label(),
                "access": {
                    "device": access.opens_devices(),
                    "extensions": access.extensions_listed(),
                    "requires": requires,
                    "scope": access.scope().as_str(),
                    "refused": access.refusal(),
                    "hub_owned": access.is_hub_owned(),
                },
            });
            (row.kind(), name.to_string(), value)
        })
        .collect();
    rows.sort_by(|a, b| (a.0.as_str(), &a.1).cmp(&(b.0.as_str(), &b.1)));
    json!({"rows": rows.into_iter().map(|r| r.2).collect::<Vec<_>>()})
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use harnix_contract::s1::{caps, RowScope};
    use harnix_kernel::{Access, CallContext, Handler, KernelPart, Owner, Rpc};

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
                    Row::method("engine.plugin.install", Access::capability(caps::ENGINE_PLUGINS), Arc::new(Nothing)),
                    Row::method("engine.config.set", Access::refused("no"), Arc::new(Nothing)),
                    Row::method("engine.plugin.list", Access::device().with_extensions(), Arc::new(Nothing)),
                    Row::method("engine.session.get", Access::device().with_extensions().scoped(RowScope::Session), Arc::new(Nothing)),
                ],
            )
            .unwrap();
        table
            .install(&Owner::Kernel(KernelPart::Hub), vec![Row::topic("harnix.instances", Access::device().with_extensions())])
            .unwrap();
        let snap = of(&table);
        let rows = snap["rows"].as_array().unwrap();
        let names: Vec<_> = rows.iter().map(|r| (r["kind"].as_str().unwrap(), r["name"].as_str().unwrap())).collect();
        assert_eq!(
            names,
            [
                ("method", "engine.config.set"),
                ("method", "engine.plugin.install"),
                ("method", "engine.plugin.list"),
                ("method", "engine.session.get"),
                ("topic", "harnix.instances")
            ]
        );
        assert_eq!(rows[1]["owner"], "engine");
        assert_eq!(rows[1]["access"]["requires"], json!(["engine.plugins"]));
        assert_eq!(rows[1]["access"]["device"], false);
        assert_eq!(rows[1]["access"]["extensions"], json!(["*"]));
        assert_eq!(rows[0]["access"]["refused"], "no");
        assert_eq!(rows[0]["access"]["extensions"], json!([]));
        assert_eq!(rows[2]["access"]["requires"], json!([]));
        assert_eq!(rows[2]["access"]["scope"], "open");
        assert_eq!(rows[3]["access"]["scope"], "session");
        assert_eq!(rows[4]["owner"], "kernel:hub");
    }
}
