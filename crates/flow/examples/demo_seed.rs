//! Runs for the flow space's demo (`scripts/harnix-demo.sh`, xops-03 §7): the demo flows driven
//! by the real kernel through a few states, written where the flow module keeps its runs.
//!
//! ```sh
//! cargo run --release -p harnix-flow --example demo_seed -- <pack dir> <runs dir>
//! ```
//!
//! Nothing is called outside the kernel: session and sub-agent tasks are handed in here, as a
//! dispatcher would after their turns, with the tokens such turns might have spent; the command
//! gates are answered as the module would after running them. No run is left with work in hand,
//! so the application starts without calling a model; the runs that are not over wait for a
//! person — a confirmation, a resume, an external hand-in. The timeline ends about now, so the
//! conversations made after the seed read as the runs' last moments.

use std::sync::Arc;

use harnix_flow::{
    Actor, CreateIn, DirPacks, Engine, FileStore, GateResult, ManualClock, MemPacks, Outcome,
    Usage, WorkItem,
};
use serde_json::{json, Value};

const MINUTE: u64 = 60_000;
/// How long the seeded timeline lasts: it starts this long before now.
const SPAN_MINUTES: u64 = 10 * 60 + 30;

/// The tasks an outcome dispatched: (station, task, token).
fn tasks(o: &Outcome) -> Vec<(String, String, String)> {
    o.work
        .iter()
        .filter_map(|w| match w {
            WorkItem::Task {
                station,
                task,
                token,
                ..
            } => Some((station.clone(), task.clone(), token.clone())),
            _ => None,
        })
        .collect()
}

struct Seed {
    eng: Engine,
    clock: Arc<ManualClock>,
}

impl Seed {
    fn wait(&self, minutes: u64) {
        self.clock.advance(minutes * MINUTE);
    }

    async fn start(&self, flow: &str, subject: &str, title: &str) -> (String, Outcome) {
        let o = self
            .eng
            .create(
                CreateIn {
                    flow: flow.into(),
                    subject: subject.into(),
                    inputs: Value::Null,
                    title: Some(title.into()),
                    parent: None,
                },
                person(),
            )
            .await
            .expect("created");
        let run = o.value["run"].as_str().expect("a run id").to_string();
        (run, o)
    }

    /// The command and host gates an outcome asks for, passed as their runs would have.
    async fn gates(&self, o: &Outcome) -> Vec<Outcome> {
        let mut out = vec![];
        for w in &o.work {
            if let WorkItem::Gate { run, station, round, gate } = w {
                self.wait(1);
                let r = self
                    .eng
                    .gate_result(run, station, *round, gate, GateResult { pass: true, detail: "ok".into() })
                    .await
                    .expect("gate answered");
                out.push(r);
            }
        }
        out
    }

    /// A dispatched task's turns (tokens counted), then its output handed in.
    async fn hand_in(
        &self,
        run: &str,
        (station, task, token): &(String, String, String),
        output: Value,
        turns: &[(u64, u64)],
        minutes: u64,
    ) -> Outcome {
        self.wait(minutes);
        for (input, output) in turns {
            let u = Usage::of_turn(&json!({"input_tokens": input, "output_tokens": output}));
            self.eng
                .account(run, Some((station, task)), u)
                .await
                .expect("counted");
        }
        let o = self
            .eng
            .submit(run, station, task, token, output, Actor::flow("dispatch"))
            .await
            .expect("handed in");
        // The work the gates' results led to is the outcome's.
        let mut merged = o.clone();
        for g in self.gates(&o).await {
            merged.work.extend(g.work);
        }
        merged
    }
}

fn person() -> Actor {
    Actor::new("device", "demo")
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [pack, runs] = args.as_slice() else {
        eprintln!("usage: demo_seed <pack dir> <runs dir>");
        std::process::exit(2);
    };
    // The empty application's catalog: its wrapped scenes, command gates on.
    let catalog = MemPacks::new().main_scene("flow-harnix").command_gates();
    let packs = DirPacks::new([pack.clone()]).with_builtin(catalog);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    let clock = Arc::new(ManualClock::new(now - SPAN_MINUTES * MINUTE));
    let eng = Engine::new(Arc::new(FileStore::new(runs)), Arc::new(packs))
        .with_clock(clock.clone())
        .with_main_scene("flow-harnix")
        .with_submit_scenes(["flow-harnix"]);
    let s = Seed { eng, clock };

    // demo-write: one finished, one waiting for the confirmation.
    for (subject, finish) in [("发布说明 v1.2", true), ("API 变更提案", false)] {
        let (run, o) = s.start("demo-write", subject, subject).await;
        let o = s
            .hand_in(&run, &tasks(&o)[0], json!({"summary": format!("{subject}：目标、范围、关键决定。")}), &[(12_400, 860), (14_100, 420)], 4)
            .await;
        s.hand_in(&run, &tasks(&o)[0], json!({"findings": ["范围写清楚了", "缺一节回滚方案"]}), &[(9_800, 610)], 6)
            .await;
        if finish {
            s.wait(25);
            s.eng
                .confirm(&run, "sign", true, Some("可以发布"), person())
                .await
                .expect("confirmed");
        }
        s.wait(90);
    }

    // demo-parallel: paused while collecting — the collection finished, nothing after it began.
    {
        let (run, o) = s.start("demo-parallel", "repo: payments", "支付服务评审").await;
        s.wait(2);
        s.eng.pause(&run, person()).await.expect("paused");
        s.hand_in(&run, &tasks(&o)[0], json!({"files": ["src/pay.rs", "docs/pay.md"]}), &[(8_200, 540)], 3)
            .await;
        s.wait(60);
    }

    // demo-each: cancelled half way through the modules.
    {
        let (run, o) = s.start("demo-each", "repo: shop", "商城模块风险").await;
        let o = s
            .hand_in(&run, &tasks(&o)[0], json!({"modules": ["auth", "billing", "search"]}), &[(7_300, 300)], 2)
            .await;
        let first = tasks(&o);
        s.hand_in(&run, &first[0], json!({"risk": "medium"}), &[(11_000, 900)], 5)
            .await;
        s.wait(3);
        s.eng
            .cancel(&run, "范围变了，重新来", person())
            .await
            .expect("cancelled");
        s.wait(45);
    }

    // demo-handoff: one sent back once and then approved; one waiting for the supplier.
    {
        let (run, _) = s.start("demo-handoff", "合同附件", "合同附件交付").await;
        let supplier = Actor::new("external", "supplier");
        for (round, pass) in [(1, false), (2, true)] {
            s.wait(30);
            let c = s
                .eng
                .claim(&run, "prepare", "prepare", supplier.clone(), 60 * MINUTE)
                .await
                .expect("claimed");
            let token = c.value["token"].as_str().expect("a token").to_string();
            s.wait(20);
            let o = s
                .eng
                .submit(&run, "prepare", "prepare", &token, json!({"link": format!("https://example.com/annex-v{round}.pdf")}), supplier.clone())
                .await
                .expect("handed in");
            s.gates(&o).await;
            s.wait(15);
            let reason = if pass { "可以" } else { "缺签字页" };
            s.eng
                .confirm(&run, "approve", pass, Some(reason), person())
                .await
                .expect("decided");
            if !pass {
                s.eng
                    .rework(&run, "prepare", None, reason, person())
                    .await
                    .expect("sent back");
            }
        }
        s.wait(120);
        s.start("demo-handoff", "供应商资质", "供应商资质材料").await;
    }

    for r in s.eng.list(Default::default()).await.expect("listed") {
        println!(
            "{}\t{}\t{}",
            r.id,
            r.phase.map(|p| p.state).unwrap_or_default(),
            r.title
        );
    }
}
