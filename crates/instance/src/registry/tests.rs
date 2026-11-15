use std::sync::{Mutex, OnceLock, Weak};

use harnix_contract::Subject;
use harnix_kernel::TopicChanged;

use super::*;

/// What an observer saw, in order, and whether each instance was listed when it saw it.
#[derive(Default)]
struct Seen {
    registry: OnceLock<Weak<InstanceRegistry>>,
    log: Mutex<Vec<String>>,
    /// What it says when asked whether an instance's sessions are in use.
    engaged: std::sync::atomic::AtomicBool,
    /// The link it was handed for each instance, last first.
    links: Mutex<HashMap<String, Arc<dyn EngineLink>>>,
}

impl Seen {
    fn note(&self, what: &str, id: &str) {
        assert!(!holds(Side::Registry), "called holding the registry's lock");
        let listed = self.registry.get().and_then(Weak::upgrade).is_some_and(|r| r.facts(id).is_some());
        lock(&self.log).push(format!("{what} {id} listed={listed}"));
    }

    fn log(&self) -> Vec<String> {
        lock(&self.log).iter().filter(|l| !l.starts_with("changed")).cloned().collect()
    }
}

impl TopicChanged for Seen {
    fn changed(&self, topic: &str, _params: Value) {
        lock(&self.log).push(format!("changed {topic}"));
    }
}

#[async_trait::async_trait]
impl InstanceObserver<dyn EngineLink> for Seen {
    async fn added(&self, id: &str, link: Arc<dyn EngineLink>, trust: InstanceTrust) {
        assert_eq!(trust.is_extension(), id.starts_with("proc."), "{id}: {trust:?}");
        lock(&self.links).insert(id.to_string(), link);
        self.note("added", id);
    }
    async fn removed(&self, id: &str) {
        self.note("removed", id);
    }
    async fn restarted(&self, id: &str, link: Arc<dyn EngineLink>) {
        assert!(!link.status().borrow().is_ready());
        self.note("restarted", id);
    }
    async fn engaged(&self, _id: &str) -> bool {
        self.engaged.load(std::sync::atomic::Ordering::Relaxed)
    }
    fn space(&self, claimed: Option<&str>) -> Option<String> {
        let space = claimed.unwrap_or("home");
        (space == "home").then(|| space.to_string())
    }
}

fn registry(dir: &std::path::Path) -> (Arc<InstanceRegistry>, Arc<Seen>) {
    registry_recording(dir, harnix_engine::Recording::new(None))
}

/// [`registry`], its links recording as `recording` says.
fn registry_recording(dir: &std::path::Path, recording: Arc<harnix_engine::Recording>) -> (Arc<InstanceRegistry>, Arc<Seen>) {
    let mut factory = Factory::new(Arc::new(|_: &str| None)).unwrap();
    factory.recording = recording;
    let mut config = RegistryConfig {
        templates: BTreeMap::new(),
        ledger: Ledger::load(dir.join("instances.json")).unwrap(),
        factory,
        instances_dir: dir.join("instances"),
        disabled: HashSet::new(),
        fixed: BTreeMap::new(),
        grants: Arc::new(harnix_kernel::GrantTable::new()),
    };
    let template = InstanceTemplate::in_process("m.work", crate::EngineProfile::new());
    config.add_template(template.clone(), Owner::Module("m".into())).unwrap();
    assert!(config.add_template(template, Owner::Kernel).is_err(), "a template id is taken once");
    let registry = InstanceRegistry::new(config);
    let seen = Arc::new(Seen::default());
    let _ = seen.registry.set(Arc::downgrade(&registry));
    registry.observe(seen.clone());
    (registry, seen)
}

fn down(reason: &str) -> Built {
    Built::new(DownLink::without_template(reason.into()), Facts::default())
}

/// The hub takes an instance over before anyone can see it, and is told it is gone only once
/// nobody can.
#[tokio::test]
async fn an_instance_is_listed_only_after_the_hub_has_it() {
    let dir = tempfile::tempdir().unwrap();
    let (registry, seen) = registry(dir.path());
    registry.add("w", down("test"), Owner::Kernel).await;
    assert!(registry.facts("w").is_some());
    assert_eq!(seen.log(), ["added w listed=false"]);
    assert!(lock(&seen.log).iter().any(|l| l == "changed harnix.instances"), "the list is republished");

    registry.add("v", down("test"), Owner::Space("s".into())).await;
    registry.remove("v").await.unwrap();
    assert!(registry.facts("v").is_none());
    assert_eq!(seen.log()[2], "removed v listed=false");
    assert_eq!(registry.remove("v").await.unwrap_err().code, codes::INSTANCE_NOT_FOUND);
}

/// A disabled module's instances stay listed, down, on a new link the hub is handed.
#[tokio::test]
async fn a_disabled_modules_instances_are_handed_over_down() {
    let dir = tempfile::tempdir().unwrap();
    let (registry, seen) = registry(dir.path());
    registry.add("mine", down("test"), Owner::Module("m".into())).await;
    registry.add("other", down("test"), Owner::Kernel).await;
    registry.module_disabled("m").await;
    assert_eq!(seen.log(), ["added mine listed=false", "added other listed=false", "restarted mine listed=true"]);
    let view = registry.view();
    let mine = view["instances"].as_array().unwrap().iter().find(|i| i["id"] == "mine").unwrap().clone();
    assert_eq!(mine["status"]["reason"], "module `m` is disabled");
    assert_eq!(mine["owner"], "module:m");
}

/// A name being changed is waited for: a second operation on it starts after the first ends.
#[tokio::test]
async fn one_operation_on_a_name_at_a_time() {
    let dir = tempfile::tempdir().unwrap();
    let (registry, _) = registry(dir.path());
    let first = registry.claim("w").await;
    let waiting = {
        let registry = registry.clone();
        tokio::spawn(async move {
            let _second = registry.claim("w").await;
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(!waiting.is_finished(), "the second claim went ahead of the first");
    let other = registry.claim("v").await;
    drop(other);
    drop(first);
    tokio::time::timeout(std::time::Duration::from_secs(5), waiting).await.unwrap().unwrap();
    assert!(registry.state().busy.is_empty());
}

/// A space the hub does not have is refused before anything is built.
#[tokio::test]
async fn a_claimed_space_is_one_the_hub_has() {
    let dir = tempfile::tempdir().unwrap();
    let (registry, _) = registry(dir.path());
    let e = registry
        .create_instance(&Subject::Kernel, json!({"template": "m.work", "name": "x", "space": "nowhere"}), true)
        .await
        .unwrap_err();
    assert_eq!(e.code, codes::SPACE_MISMATCH);
    let e = registry
        .create_instance(&Subject::extension(harnix_contract::ExtId::native("n").unwrap()), json!({"template": "m.work", "name": "x"}), true)
        .await
        .unwrap_err();
    assert_eq!(e.code, codes::REFUSED, "another module's template");
}

/// An engine at the other end of a stream, in this test: it answers every request with one value
/// that is every answer the link reads — the protocol it speaks, and `sessions` — and keeps them.
fn engine(sessions: Value) -> (std::os::unix::net::UnixStream, Arc<Mutex<Vec<Value>>>) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let (ours, theirs) = std::os::unix::net::UnixStream::pair().unwrap();
    theirs.set_nonblocking(true).unwrap();
    let asked = Arc::new(Mutex::new(Vec::new()));
    let kept = asked.clone();
    tokio::spawn(async move {
        let (r, mut w) = tokio::net::UnixStream::from_std(theirs).unwrap().into_split();
        let mut lines = tokio::io::BufReader::new(r).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let request: Value = serde_json::from_str(&line).unwrap();
            lock(&kept).push(request.clone());
            let result = json!({"protocol_version": harnix_engine::protocol_version(), "sessions": sessions, "session_id": "s1"});
            let reply = json!({"jsonrpc": "2.0", "id": request["id"], "result": result});
            if w.write_all(format!("{reply}\n").as_bytes()).await.is_err() {
                break;
            }
        }
    });
    (ours, asked)
}

/// Wakes by handing the adoption a new engine's stream, once it is set.
#[derive(Default)]
struct Waking {
    adoption: OnceLock<Weak<Adoption>>,
    woken: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl Wake for Waking {
    async fn wake(&self) -> Rpc<()> {
        self.woken.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let adoption = self.adoption.get().and_then(Weak::upgrade).expect("adopted");
        adoption.attach(engine(json!([{"session_id": "s1", "status": "active"}])).0).await
    }
}

fn row(registry: &InstanceRegistry, id: &str) -> Value {
    registry.view()["instances"].as_array().unwrap().iter().find(|i| i["id"] == id).cloned().unwrap_or(Value::Null)
}

/// An extension's engine is listed as its own, carried by each run of its process in turn; let go
/// while idle, it answers its list without waking and wakes for anything else.
#[tokio::test]
async fn an_adopted_engine_is_the_extensions_and_sleeps_until_it_is_needed() {
    let dir = tempfile::tempdir().unwrap();
    let (registry, seen) = registry(dir.path());
    let waking = Arc::new(Waking::default());
    let ext = harnix_contract::ExtId::package("acme.board").unwrap();
    let spaces = vec!["acme.board".to_string()];
    let spec = Adopting { ext, data_root: dir.path().join("run/engine"), inherit_providers: false, sandboxed: false, wake: waking.clone(), spaces };
    let adoption = registry.adopt(spec).await.unwrap();
    let _ = waking.adoption.set(Arc::downgrade(&adoption));
    let mut notes = adoption.notes().expect("the first caller has its notes");
    assert_eq!(seen.log(), ["added proc.acme.board listed=false"]);
    let listed = row(&registry, "proc.acme.board");
    assert_eq!((listed["carrier"].as_str(), listed["trust"].as_str()), (Some("adopted"), Some("extension")), "{listed}");
    assert_eq!(listed["owner"], "module:acme.board");
    assert_eq!(listed["status"]["state"], "starting");
    assert!(dir.path().join("run/engine").is_dir());
    assert_eq!(registry.facts("proc.acme.board").unwrap().trust.label(), "extension");

    // Its process's first run attaches: up, and what is in its memory is in use.
    let (stream, _) = engine(json!([{"session_id": "s1", "status": "active"}]));
    adoption.attach(stream).await.unwrap();
    assert_eq!(row(&registry, "proc.acme.board")["status"]["state"], "ready");
    assert!(adoption.engaged().await, "a session in its memory");

    // Gone, and back on the next run.
    adoption.detach("its process exited").await;
    assert_eq!(row(&registry, "proc.acme.board")["status"]["state"], "degraded");
    let (stream, _) = engine(json!([{"session_id": "s1", "status": "stored"}]));
    adoption.attach(stream).await.unwrap();
    assert!(!adoption.engaged().await, "nothing in its memory, nobody watching");
    seen.engaged.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(adoption.engaged().await, "the hub has one of its sessions in use");
    seen.engaged.store(false, std::sync::atomic::Ordering::Relaxed);

    // Let go while idle: listed hibernating; its list answered from before, waking nothing.
    adoption.hibernate().await;
    assert_eq!(row(&registry, "proc.acme.board")["status"]["state"], "hibernating");
    let link = adoption.link();
    let link = link.ops();
    let rows = link.list().await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(waking.woken.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert!(!adoption.engaged().await);
    // Anything else wakes it, once.
    let (a, b) = tokio::join!(link.get("s1"), link.tasks("s1"));
    a.unwrap();
    b.unwrap();
    assert_eq!(waking.woken.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert_eq!(row(&registry, "proc.acme.board")["status"]["state"], "ready");

    // The hub's turns on it reach its carrier, with their chains.
    registry.factory.turns.start("proc.acme.board", "s1", vec![harnix_contract::ChainLink::new("acme.board", "c1")].into());
    registry.factory.turns.settle("proc.acme.board", "s1");
    registry.factory.turns.start("main", "s9", vec![harnix_contract::ChainLink::new("acme.other", "c2")].into());
    assert_eq!(adoption.turn_chain("s1"), Vec::<harnix_contract::ChainLink>::new());
    let first = notes.recv().await.unwrap();
    assert_eq!(first, EngineNote::TurnStarted { session_id: "s1".into(), chain: vec![harnix_contract::ChainLink::new("acme.board", "c1")] });
    assert_eq!(notes.recv().await.unwrap(), EngineNote::TurnSettled { session_id: "s1".into() });

    // Flooding: the carrier is told, once a run; the instance stays.
    registry.flooding("proc.acme.board", 3);
    registry.flooding("proc.acme.board", 4);
    assert_eq!(notes.recv().await.unwrap(), EngineNote::Flooded);
    assert!(notes.try_recv().is_err(), "told once");
    assert!(registry.facts("proc.acme.board").is_some());
    // The extension's, and its spaces': what the hub holds a call naming it to.
    let whose = InstanceBook::listed(&*registry, "proc.acme.board").unwrap();
    assert_eq!((whose.extension.as_ref().map(|e| e.as_str()), whose.spaces.as_slice()), (Some("acme.board"), &["acme.board".to_string()][..]));

    // Nobody stops or restarts it but its extension.
    assert_eq!(registry.remove("proc.acme.board").await.unwrap_err().code, codes::REFUSED);
    assert_eq!(registry.rebuild("proc.acme.board").await.unwrap_err().code, codes::REFUSED);
    let again = Adopting {
        ext: harnix_contract::ExtId::package("acme.board").unwrap(),
        data_root: dir.path().join("run/engine"),
        inherit_providers: false,
        sandboxed: false,
        wake: waking.clone(),
        spaces: Vec::new(),
    };
    assert_eq!(registry.adopt(again).await.err().map(|e| e.code), Some(codes::REFUSED));

    registry.release("acme.board").await;
    assert!(registry.facts("proc.acme.board").is_none());
    assert_eq!(seen.log().last().unwrap(), "removed proc.acme.board listed=false");
}

/// The hub's link to an extension's engine goes through the process's recorder, as every other
/// link does — in replay a session created on it plays the recording it names — and still wakes it.
#[tokio::test]
async fn an_adopted_engine_records_and_replays_as_the_process_does() {
    let dir = tempfile::tempdir().unwrap();
    let replay = dir.path().join("recordings");
    let (registry, seen) = registry_recording(dir.path(), harnix_engine::Recording::new(Some(replay.clone())));
    let waking = Arc::new(Waking::default());
    let spec = Adopting {
        ext: harnix_contract::ExtId::package("acme.board").unwrap(),
        data_root: dir.path().join("run/engine"),
        inherit_providers: false,
        sandboxed: false,
        wake: waking.clone(),
        spaces: vec!["acme.board".to_string()],
    };
    let adoption = registry.adopt(spec).await.unwrap();
    let _ = waking.adoption.set(Arc::downgrade(&adoption));
    let (stream, asked) = engine(json!([{"session_id": "s1", "status": "active"}]));
    adoption.attach(stream).await.unwrap();
    let link = lock(&seen.links).get("proc.acme.board").cloned().expect("the hub has its link");

    link.ops().create(json!({"options": {"recorder": {"name": "says-ready"}}})).await.unwrap();
    let created = lock(&asked).iter().find(|r| r["params"]["options"]["recorder"].is_object()).cloned().expect("a create");
    let recorder = &created["params"]["options"]["recorder"];
    assert_eq!((recorder["mode"].as_str(), recorder["name"].as_str()), (Some("replay"), Some("says-ready")), "{created}");
    assert_eq!(recorder["dir"].as_str(), replay.to_str());

    // Let go while idle, a call through the recorder wakes it, once.
    adoption.hibernate().await;
    link.ops().get("s1").await.unwrap();
    assert_eq!(waking.woken.load(std::sync::atomic::Ordering::Relaxed), 1);
    registry.release("acme.board").await;
}

/// In the host's sandbox an extension's engine runs its commands unwrapped, since macOS refuses a
/// sandbox nested in that one; out of it, the engine keeps its own. As the engine itself reads its
/// settings, and through a merge of `main`'s providers.
#[tokio::test]
async fn an_engine_in_the_hosts_sandbox_has_its_own_command_sandbox_off() {
    use harnix_engine::agix::base::settings::Settings;
    let disabled = |root: &std::path::Path| {
        Settings::load(root.to_path_buf(), root.join("scenes/code"), root.join("work/.agix"), "code", "m").sandbox.dangerously_disable_sandbox
    };
    let dir = tempfile::tempdir().unwrap();
    let (registry, _) = registry(dir.path());
    let root = dir.path().join("run/engine");
    let adopting = |sandboxed| Adopting {
        ext: harnix_contract::ExtId::package("acme.board").unwrap(),
        data_root: root.clone(),
        inherit_providers: false,
        sandboxed,
        wake: Arc::new(Waking::default()),
        spaces: Vec::new(),
    };

    registry.adopt(adopting(true)).await.unwrap();
    assert!(disabled(&root), "its engine wraps its commands in a sandbox of its own");
    let system = dir.path().join("system");
    std::fs::create_dir_all(&system).unwrap();
    std::fs::write(system.join("settings.json"), json!({"providers": {"a": {"api_type": "anthropic"}}}).to_string()).unwrap();
    assert!(crate::inherit::merge(&system, &root).unwrap());
    assert!(disabled(&root), "a merge of providers turned its own sandbox back on");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(root.join("settings.json")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    // Started again with the host's sandbox off: its own is on again, the rest of its settings kept.
    registry.release("acme.board").await;
    registry.adopt(adopting(false)).await.unwrap();
    assert!(!disabled(&root), "out of the host's sandbox, its engine has none of its own");
    let written: Value = serde_json::from_slice(&std::fs::read(root.join("settings.json")).unwrap()).unwrap();
    assert!(written["providers"].get("a").is_some(), "{written}");
}
