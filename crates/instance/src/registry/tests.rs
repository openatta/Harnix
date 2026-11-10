use std::sync::{Mutex, OnceLock, Weak};

use harnix_contract::Subject;
use harnix_kernel::TopicChanged;

use super::*;

/// What an observer saw, in order, and whether each instance was listed when it saw it.
#[derive(Default)]
struct Seen {
    registry: OnceLock<Weak<InstanceRegistry>>,
    log: Mutex<Vec<String>>,
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
    async fn added(&self, id: &str, _link: Arc<dyn EngineLink>, trust: InstanceTrust) {
        assert_eq!(trust, InstanceTrust::Product);
        self.note("added", id);
    }
    async fn removed(&self, id: &str) {
        self.note("removed", id);
    }
    async fn restarted(&self, id: &str, link: Arc<dyn EngineLink>) {
        assert!(!link.status().borrow().is_ready());
        self.note("restarted", id);
    }
    fn space(&self, claimed: Option<&str>) -> Option<String> {
        let space = claimed.unwrap_or("sessions");
        (space == "sessions").then(|| space.to_string())
    }
}

fn registry(dir: &std::path::Path) -> (Arc<InstanceRegistry>, Arc<Seen>) {
    let mut config = RegistryConfig {
        templates: BTreeMap::new(),
        ledger: Ledger::load(dir.join("instances.json")).unwrap(),
        factory: Factory::new(Arc::new(|_: &str| None)).unwrap(),
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
        .create_instance(&Subject::extension(harnix_contract::ExtId::parse("n").unwrap()), json!({"template": "m.work", "name": "x"}), true)
        .await
        .unwrap_err();
    assert_eq!(e.code, codes::REFUSED, "another module's template");
}
