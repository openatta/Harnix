//! hello-app: HOME and two spaces, built the way an application on Harnix is built.
//!
//! | space | interface | backend | Agent |
//! |---|---|---|---|
//! | `greeter` | a tab and a HOME card (`ui/`) | the `greeter` module below | its own in-process instance, from the module's template |
//! | `notes` | a space (`ui/`) | — | the `system` instance |

use std::sync::atomic::{AtomicU64, Ordering};

use harnix::prelude::*;
use serde::Deserialize;
use serde_json::json;

fn main() -> anyhow::Result<()> {
    harnix::App::builder()
        .name("hello-app", env!("CARGO_PKG_VERSION"))
        .with_builtins()
        .module(Greeter::default())
        .run()
}

#[derive(Default)]
struct Greeter {
    greeted: std::sync::Arc<AtomicU64>,
}

#[derive(Deserialize)]
struct Greet {
    name: String,
}

#[harnix::async_trait]
impl Module for Greeter {
    const API_VERSION: u32 = harnix::MODULE_API_VERSION;

    fn manifest(&self) -> Manifest {
        Manifest::new("greeter", "0.1.0").disclosure("greets people, and runs its sessions on an instance of its own")
    }

    fn templates(&self, t: &mut InstanceTemplates) {
        t.add(InstanceTemplate::new(
            "desk",
            TemplateCarrier::InProcess {
                engine: EngineProfile {
                    scene: "chat".into(),
                    ..EngineProfile::default()
                },
                contrib: EngineContrib::new(),
            },
        ));
    }

    fn spaces(&self, s: &mut SpaceRegistry) {
        s.add(SpaceDecl::new("greeter").instance("template:greeter.desk").scene("chat"));
    }

    fn methods(&self, m: &mut MethodRegistry) {
        let greeted = self.greeted.clone();
        m.method("greet", Principal::Device)
            .describe("says hello, and counts")
            .schema(json!({"type": "object", "required": ["name"], "properties": {"name": {"type": "string"}}}))
            .handler(move |cx: CallContext, p: Greet| {
                let greeted = greeted.clone();
                async move {
                    let n = greeted.fetch_add(1, Ordering::SeqCst) + 1;
                    cx.module().events().publish("greeted", json!({}), json!({"count": n})).await?;
                    Ok(json!({"greeting": format!("hello, {}", p.name), "count": n}))
                }
            });
    }

    fn events(&self, e: &mut EventRegistry) {
        e.topic("greeted");
    }

    async fn start(&self, cx: ModuleContext) -> anyhow::Result<()> {
        cx.events().publish("greeted", json!({}), json!({"count": 0})).await?;
        Ok(())
    }
}

