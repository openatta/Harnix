//! What the engine does with a session's host skill directories when one that was listed is
//! renamed away, deleted, replaced or comes back: the behaviour the packages' skills rest on
//! (`<state>/ext/<id>/skills`, listed per session, written in place).
//!
//! No model and no network. The roots are under the target directory: the engine refuses a
//! directory in the system's per-user temporary one (`tempfile`'s default on macOS).

use std::path::{Path, PathBuf};
use std::time::Duration;

use harnix_engine::{Engine, EngineConfig, HostSide, InProcessEngine};
use harnix_engine_api::link::EngineLink;
use harnix_engine_api::EngineProfile;
use serde_json::{json, Value};

/// A limit, not a measurement: the engine's watcher settles for 300ms before it rescans.
const UP_TO: Duration = Duration::from_secs(20);

struct Rig {
    // Held for the directory's life.
    _root: tempfile::TempDir,
    skills: PathBuf,
    engine: std::sync::Arc<InProcessEngine>,
}

impl Rig {
    async fn start() -> Rig {
        let root = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
        // The real path: the engine compares resolved paths.
        let base = std::fs::canonicalize(root.path()).unwrap();
        let data = base.join("engine");
        std::fs::create_dir_all(&data).unwrap();
        let skills = base.join("ext").join("acme.notes").join("skills");
        let mut config = EngineConfig::from_profile(&EngineProfile::new(), &data);
        // The engine keeps its own marks in the working directory: not the repository's.
        config.cwd = Some(base.join("cwd"));
        std::fs::create_dir_all(base.join("cwd")).unwrap();
        let engine = Engine::start(config).await.expect("the engine starts");
        let engine = InProcessEngine::start(engine, "test", HostSide::none()).await.expect("the link is up");
        Rig { _root: root, skills, engine }
    }

    async fn create(&self) -> String {
        self.create_with(json!([{"path": self.skills}])).await.expect("a session").session_id.expect("its id")
    }

    async fn create_with(&self, dirs: Value) -> Result<harnix_engine_api::SessionCreated, harnix_contract::RpcError> {
        let params = json!({"project_root": null, "options": {"skill_dirs": dirs}});
        self.engine.link().ops().create(params, Default::default()).await
    }

    /// The first listed directory's `state` and skill names, as `session.get` reports them.
    async fn state(&self, sid: &str) -> (String, Vec<String>) {
        let info = self.engine.link().ops().get(sid).await.expect("the session");
        let dir = &info.value()["skill_dirs"][0];
        let names = dir["skills"].as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_string)).collect();
        (dir["state"].as_str().unwrap_or("?").to_string(), names)
    }

    /// The names of the host skills `commands.list` gives the session.
    async fn commands(&self, sid: &str) -> Vec<String> {
        let list = self.engine.link().ops().passthrough("commands.list", json!({"session_id": sid})).await.expect("commands");
        let mut names: Vec<String> = list["commands"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| c["source"] == "host")
            .filter_map(|c| c["name"].as_str().map(|n| n.trim_start_matches('/').to_string()))
            .collect();
        names.sort();
        names
    }

    fn beside(&self, name: &str) -> PathBuf {
        self.skills.parent().unwrap().join(name)
    }

    /// Wait for `state` to say `want`, and say how it got there.
    async fn becomes(&self, sid: &str, want: (&str, &[&str])) {
        let want = (want.0.to_string(), want.1.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        let start = std::time::Instant::now();
        loop {
            let now = self.state(sid).await;
            if now == want {
                return;
            }
            assert!(start.elapsed() < UP_TO, "never became {want:?}; last saw {now:?}");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn commands_become(&self, sid: &str, has: &str, expect: bool) {
        let start = std::time::Instant::now();
        loop {
            let now = self.commands(sid).await;
            if now.iter().any(|n| n == has) == expect {
                return;
            }
            assert!(start.elapsed() < UP_TO, "`{has}` stayed {}; commands {now:?}", if expect { "absent" } else { "present" });
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

fn skill(dir: &Path, name: &str, description: &str) {
    let at = dir.join(name);
    std::fs::create_dir_all(&at).unwrap();
    std::fs::write(at.join("SKILL.md"), format!("---\nname: {name}\ndescription: {description}\n---\nBody of {name}.\n")).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_listed_dir_renamed_away_leaves_and_comes_back() {
    let rig = Rig::start().await;
    skill(&rig.skills, "tour", "a tour");
    let sid = rig.create().await;
    rig.becomes(&sid, ("ok", &["tour"])).await;
    rig.commands_become(&sid, "tour", true).await;

    let off = rig.beside("skills.off");
    std::fs::rename(&rig.skills, &off).unwrap();
    rig.becomes(&sid, ("missing", &[])).await;
    rig.commands_become(&sid, "tour", false).await;

    std::fs::rename(&off, &rig.skills).unwrap();
    rig.becomes(&sid, ("ok", &["tour"])).await;
    rig.commands_become(&sid, "tour", true).await;

    // The second time: the watch must have survived the first.
    std::fs::rename(&rig.skills, &off).unwrap();
    rig.becomes(&sid, ("missing", &[])).await;
    std::fs::rename(&off, &rig.skills).unwrap();
    skill(&rig.skills, "guide", "a guide");
    rig.becomes(&sid, ("ok", &["guide", "tour"])).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dir_replaced_in_place_is_read_again() {
    let rig = Rig::start().await;
    skill(&rig.skills, "tour", "old tour");
    skill(&rig.skills, "gone", "going");
    let sid = rig.create().await;
    rig.becomes(&sid, ("ok", &["gone", "tour"])).await;

    // The package upgrade: the new version beside, entries moved in one by one.
    let new = rig.beside("skills.new");
    let old = rig.beside("skills.old");
    skill(&new, "tour", "new tour");
    skill(&new, "guide", "a guide");
    std::fs::create_dir_all(&old).unwrap();
    for entry in std::fs::read_dir(&rig.skills).unwrap() {
        let entry = entry.unwrap();
        std::fs::rename(entry.path(), old.join(entry.file_name())).unwrap();
    }
    for entry in std::fs::read_dir(&new).unwrap() {
        let entry = entry.unwrap();
        std::fs::rename(entry.path(), rig.skills.join(entry.file_name())).unwrap();
    }
    std::fs::remove_dir_all(&old).unwrap();
    std::fs::remove_dir_all(&new).unwrap();
    rig.becomes(&sid, ("ok", &["guide", "tour"])).await;

    let off = rig.beside("skills.off");
    std::fs::rename(&rig.skills, &off).unwrap();
    rig.becomes(&sid, ("missing", &[])).await;
}

/// Recorded behaviour: whether replacing the directory by renaming one over it keeps the engine
/// watching. The packages do not do it either way (they replace in place).
#[tokio::test(flavor = "multi_thread")]
#[cfg_attr(target_os = "linux", ignore = "inotify follows the directory's inode: a swapped-in directory is not watched")]
async fn a_dir_swapped_by_renaming_over_it() {
    let rig = Rig::start().await;
    skill(&rig.skills, "tour", "old tour");
    let sid = rig.create().await;
    rig.becomes(&sid, ("ok", &["tour"])).await;

    let new = rig.beside("skills.new");
    let old = rig.beside("skills.old");
    skill(&new, "guide", "a guide");
    std::fs::rename(&rig.skills, &old).unwrap();
    std::fs::rename(&new, &rig.skills).unwrap();
    std::fs::remove_dir_all(&old).unwrap();
    rig.becomes(&sid, ("ok", &["guide"])).await;

    std::fs::rename(&rig.skills, rig.beside("skills.off")).unwrap();
    rig.becomes(&sid, ("missing", &[])).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_deleted_dir_that_comes_back() {
    let rig = Rig::start().await;
    skill(&rig.skills, "tour", "a tour");
    let sid = rig.create().await;
    rig.becomes(&sid, ("ok", &["tour"])).await;

    std::fs::remove_dir_all(&rig.skills).unwrap();
    rig.becomes(&sid, ("missing", &[])).await;

    skill(&rig.skills, "tour", "a tour");
    // Without a model no turn runs, so only the watcher can bring it back before one; what it
    // does is recorded here and the packages do not rely on it.
    let start = std::time::Instant::now();
    let mut now = rig.state(&sid).await;
    while now.0 != "ok" && start.elapsed() < Duration::from_secs(5) {
        tokio::time::sleep(Duration::from_millis(100)).await;
        now = rig.state(&sid).await;
    }
    eprintln!("deleted and recreated: after {:?} the directory is {now:?}", start.elapsed());
    assert!(now.0 == "missing" || now.0 == "ok", "{now:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dir_the_session_cannot_list_is_named() {
    let rig = Rig::start().await;
    let elsewhere = tempfile::tempdir().unwrap();
    let data_skills = rig._root.path().join("engine").join("skills");
    std::fs::create_dir_all(&data_skills).unwrap();
    let dirs = json!([
        {"path": rig.skills},
        {"path": data_skills},
        {"path": "relative/skills"},
        {"path": elsewhere.path()},
    ]);
    let e = match rig.create_with(dirs).await {
        Ok(_) => panic!("a session was created"),
        Err(e) => e,
    };
    assert_eq!(e.code, -32602, "{e:?}");
    let fields: Vec<String> = e.data.as_ref().unwrap()["invalid_options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["field"].as_str().unwrap().to_string())
        .collect();
    assert!(fields.iter().all(|f| f.starts_with("skill_dirs[") && f.ends_with("].path")), "{fields:?}");
    assert!(fields.contains(&"skill_dirs[1].path".to_string()) && fields.contains(&"skill_dirs[2].path".to_string()), "{fields:?}");
    // Where the temp directory is refused it is a fourth; elsewhere it is not.
    assert!(!fields.contains(&"skill_dirs[0].path".to_string()), "{fields:?}");
}
