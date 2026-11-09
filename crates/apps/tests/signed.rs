//! Signed applications against the runtime: the showcase signed by its publisher and
//! countersigned by a store, installed without dev mode; refused when a byte of it is not what was
//! signed — before it is unpacked, and again before it starts; disclosed, with only what an update
//! adds; and updated in place, its data kept, or left as it was when the new version fails.
//!
//! A stand-in kernel answers the engine methods the runtime calls, as in `runtime.rs`.
#![cfg(feature = "compile")]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use harnix_apps::signature::{Keys, SecretKey};
use harnix_apps::{Options, Runtime};
use harnix_builtin::apps::{AppsRuntime, InstallRecord};
use harnix_contract::{RpcError, Subject};
use harnix_module::{InstanceHandle, Kernel, ModuleContext, SessionRef, Tap};
use serde_json::{json, Value};

const ID: &str = "example.showcase";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn showcase_wasm() -> Vec<u8> {
    static BUILT: OnceLock<Vec<u8>> = OnceLock::new();
    BUILT
        .get_or_init(|| {
            let component = root().join("examples/apps/showcase/component");
            let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("showcase");
            let status = std::process::Command::new(env!("CARGO"))
                .args(["build", "--release", "--target", "wasm32-wasip2"])
                .env("CARGO_TARGET_DIR", &target)
                .current_dir(&component)
                .status()
                .expect("cargo runs");
            assert!(status.success(), "the showcase component did not build");
            std::fs::read(target.join("wasm32-wasip2/release/showcase_tools.wasm")).unwrap()
        })
        .clone()
}

#[derive(Default)]
struct FakeKernel {
    data_root: PathBuf,
    calls: Mutex<Vec<(String, Value)>>,
}

#[async_trait::async_trait]
impl Kernel for FakeKernel {
    async fn call(
        &self,
        _subject: &Subject,
        method: &str,
        params: Value,
    ) -> Result<Value, RpcError> {
        self.calls
            .lock()
            .unwrap()
            .push((method.to_string(), params));
        Ok(json!({}))
    }
    async fn publish(&self, _topic: &str, _params: Value, _value: Value) {}
    fn instance(&self, id: &str) -> Option<InstanceHandle> {
        (id == "system").then(|| InstanceHandle {
            id: id.to_string(),
            data_root: self.data_root.join(id),
            recordings_root: self.data_root.join("recordings"),
            history: None,
            active_scenes: vec![],
            model: String::new(),
        })
    }
    async fn tap(
        &self,
        _subject: &Subject,
        _session: &SessionRef,
        _capacity: usize,
    ) -> Result<Tap, RpcError> {
        Err(RpcError::refused("no taps here"))
    }
}

impl FakeKernel {
    fn called(&self, method: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| m == method)
            .count()
    }
}

struct Rig {
    dir: tempfile::TempDir,
    kernel: Arc<FakeKernel>,
    cx: ModuleContext,
    runtime: Arc<Runtime>,
    publisher: SecretKey,
    store: SecretKey,
}

fn key(byte: u8) -> SecretKey {
    SecretKey::parse(&hex::encode([byte; 32])).unwrap()
}

impl Rig {
    /// A process with `dev` off, trusting the store key `store` (and `trusted`, as the publisher).
    fn new(trusted_publisher: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let kernel = Arc::new(FakeKernel {
            data_root: dir.path().join("engine"),
            ..Default::default()
        });
        let cx = ModuleContext::new(
            "apps",
            "harnix.apps",
            Value::Null,
            dir.path().join("module"),
            Arc::new(harnix_module::EnvCredentials),
            kernel.clone(),
            vec![],
        );
        let (publisher, store) = (key(1), key(2));
        let mut keys = Keys {
            store: vec![store.public()],
            ..Keys::default()
        };
        if trusted_publisher {
            keys.trusted
                .insert("example".into(), vec![publisher.public()]);
        }
        let runtime = Runtime::new(Options {
            keys,
            ..Options::default()
        })
        .unwrap();
        Rig {
            dir,
            kernel,
            cx,
            runtime,
            publisher,
            store,
        }
    }

    fn apps(&self) -> PathBuf {
        self.dir.path().join("apps")
    }

    /// The showcase, `edit`ed in its manifest, packed for this engine and signed by its publisher
    /// (and countersigned by the store when `countersigned`).
    fn package(&self, name: &str, countersigned: bool, edit: impl Fn(String) -> String) -> PathBuf {
        let source = root().join("examples/apps/showcase");
        let app = self.dir.path().join("src").join(name);
        for part in ["ui", "agent"] {
            copy_tree(&source.join(part), &app.join(part));
        }
        let manifest = std::fs::read_to_string(source.join("harnix-app.toml")).unwrap();
        std::fs::write(app.join("harnix-app.toml"), edit(manifest)).unwrap();
        let wasm = showcase_wasm();
        let cwasm = self.runtime.engine().compile(&wasm).unwrap();
        let out = self.dir.path().join(format!("{name}.hxapp"));
        let artifact = harnix_apps::package::Artifact {
            fingerprint: self.runtime.engine().fingerprint(),
            cwasm: &cwasm,
            wasm: Some(&wasm),
        };
        harnix_apps::package::pack(&app, Some(artifact), &out).unwrap();
        harnix_apps::publish::sign(&out, &self.publisher, &out).unwrap();
        if countersigned {
            harnix_apps::publish::countersign(&out, &self.store, &[], &out).unwrap();
        }
        out
    }

    async fn install(&self, package: &Path) -> Result<InstallRecord, RpcError> {
        let info = self.runtime.inspect(package)?;
        let app_dir = self.apps().join(&info.id);
        let (parts, _summary) = self.runtime.install(&self.cx, package, &app_dir).await?;
        Ok(InstallRecord {
            id: info.id,
            version: info.version,
            title: info.title,
            enabled: true,
            parts,
            ..InstallRecord::default()
        })
    }
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            copy_tree(&path, &to.join(entry.file_name()));
        } else {
            std::fs::copy(&path, to.join(entry.file_name())).unwrap();
        }
    }
}

/// The same zip with `name` replaced by `bytes`, its `SIGNATURES` left as it was.
fn tampered(package: &Path, name: &str, bytes: &[u8], out: &Path) {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(package).unwrap()).unwrap();
    let mut zip = zip::ZipWriter::new(std::fs::File::create(out).unwrap());
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).unwrap();
        let entry_name = entry.name().to_string();
        let mut content = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut content).unwrap();
        zip.start_file(
            entry_name.as_str(),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        std::io::Write::write_all(&mut zip, if entry_name == name { bytes } else { &content })
            .unwrap();
    }
    zip.finish().unwrap();
}

fn grow(manifest: String) -> String {
    manifest
        .replace("version = \"0.1.0\"", "version = \"0.2.0\"")
        .replace("storage_mb = 1", "storage_mb = 4")
        .replace(
            "env = [\"SHOWCASE_TOKEN\"]",
            "env = [\"SHOWCASE_TOKEN\", \"SHOWCASE_EXTRA\"]",
        )
}

#[tokio::test(flavor = "multi_thread")]
async fn countersigned_by_the_store_it_installs_and_runs_without_dev_mode() {
    let rig = Rig::new(false);
    let package = rig.package("good", true, |m| m);

    let disclosed = rig.runtime.disclose(&package, None).unwrap();
    assert_eq!(disclosed.trust, "store");
    assert!(disclosed.needs_consent);
    assert_eq!(disclosed.disclosure["capabilities"]["storage_mb"], json!(1));
    assert_eq!(
        disclosed.disclosure["capabilities"]["fs"],
        json!(["${project}"])
    );
    assert_eq!(
        disclosed.disclosure["publisher_key"],
        json!(rig.publisher.public().hex())
    );
    // The same package is the same consent; another package is another.
    assert_eq!(
        rig.runtime.disclose(&package, None).unwrap().digest,
        disclosed.digest
    );
    let other = rig.package("other", true, |m| {
        m.replace(
            "greeting = \"hello from the showcase\"",
            "greeting = \"hi\"",
        )
    });
    assert_ne!(
        rig.runtime.disclose(&other, None).unwrap().digest,
        disclosed.digest
    );

    let record = rig.install(&package).await.unwrap();
    assert_eq!(record.parts["trust"], json!("store"));
    assert_eq!(rig.runtime.status(ID).await["state"], json!("running"));

    // A byte of the installed component changed on disk: it does not start again.
    rig.runtime.unload(&rig.cx, &record).await;
    let fingerprint = rig.runtime.engine().fingerprint().to_string();
    let cwasm = rig
        .apps()
        .join(ID)
        .join("0.1.0/wasm")
        .join(format!("tools.{fingerprint}.cwasm"));
    let mut bytes = std::fs::read(&cwasm).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    std::fs::write(&cwasm, bytes).unwrap();
    let err = rig
        .runtime
        .load(&rig.cx, &record, &rig.apps().join(ID))
        .await
        .unwrap_err();
    assert!(
        err.message.contains("is not the file that was signed"),
        "{}",
        err.message
    );
    assert_eq!(rig.runtime.status(ID).await["state"], json!("failed"));
}

#[tokio::test(flavor = "multi_thread")]
async fn what_is_not_what_was_signed_is_refused_before_it_is_unpacked() {
    let rig = Rig::new(false);
    let package = rig.package("good", true, |m| m);
    let out = rig.dir.path().join("tampered.hxapp");
    tampered(
        &package,
        "ui/showcase.js",
        b"export function register() { /* someone else's */ }",
        &out,
    );
    let err = rig.install(&out).await.unwrap_err();
    assert!(
        err.message
            .contains("`ui/showcase.js` is not the file that was signed"),
        "{}",
        err.message
    );
    assert!(!rig.apps().join(ID).exists());
    assert_eq!(rig.kernel.called("plugin.install"), 0);

    // Signed by its publisher only, whom this process does not trust: not installed.
    let publisher_only = rig.package("publisher-only", false, |m| m);
    let err = rig.install(&publisher_only).await.unwrap_err();
    assert!(
        err.message.contains("not one the profile trusts"),
        "{}",
        err.message
    );

    // Countersigned by a store this process has no key for.
    let elsewhere = rig.dir.path().join("elsewhere.hxapp");
    harnix_apps::publish::countersign(&publisher_only, &key(7), &[], &elsewhere).unwrap();
    let err = rig.install(&elsewhere).await.unwrap_err();
    assert!(
        err.message.contains("a store this Harnix does not trust"),
        "{}",
        err.message
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_trusted_publisher_installs_without_the_store() {
    let rig = Rig::new(true);
    let record = rig
        .install(&rig.package("trusted", false, |m| m))
        .await
        .unwrap();
    assert_eq!(record.parts["trust"], json!("trusted"));
    assert_eq!(rig.runtime.status(ID).await["state"], json!("running"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_update_discloses_what_it_adds_keeps_the_data_and_undoes_itself_when_it_fails() {
    let rig = Rig::new(false);
    let record = rig.install(&rig.package("v1", true, |m| m)).await.unwrap();
    let app_dir = rig.apps().join(ID);
    std::fs::create_dir_all(app_dir.join("kv")).unwrap();
    std::fs::write(app_dir.join("kv/kept"), "the application's own").unwrap();

    let v2 = rig.package("v2", true, grow);
    let disclosed = rig
        .runtime
        .disclose(&v2, Some((&record, &app_dir)))
        .unwrap();
    let update = &disclosed.disclosure["update"];
    assert_eq!(update["from"], json!("0.1.0"));
    assert_eq!(
        update["added"],
        json!([
            {"capability": "env", "value": "SHOWCASE_EXTRA"},
            {"capability": "storage_mb", "value": 4, "from": 1},
        ])
    );
    assert!(disclosed.needs_consent);
    // Consent to installing it fresh is not consent to it as an update.
    assert_ne!(
        disclosed.digest,
        rig.runtime.disclose(&v2, None).unwrap().digest
    );
    // An update asking for nothing more needs no consent.
    let same = rig.package("v2-same", true, |m| {
        m.replace("version = \"0.1.0\"", "version = \"0.2.0\"")
    });
    assert!(
        !rig.runtime
            .disclose(&same, Some((&record, &app_dir)))
            .unwrap()
            .needs_consent
    );

    // A version that cannot start (its agent package goes to an instance there is not): the
    // installed one is back, running, as it was.
    let broken = rig.package("v3-broken", true, |m| {
        grow(m).replace("0.2.0", "0.3.0").replace(
            "install_into = [\"system\"]",
            "install_into = [\"nowhere\"]",
        )
    });
    let err = rig
        .runtime
        .update(&rig.cx, &record, &broken, &app_dir)
        .await
        .unwrap_err();
    assert!(err.message.contains("no such instance"), "{}", err.message);
    assert_eq!(rig.runtime.status(ID).await["state"], json!("running"));
    assert!(app_dir.join("0.1.0").is_dir());
    assert!(!app_dir.join("0.3.0").exists());

    let (parts, summary) = rig
        .runtime
        .update(&rig.cx, &record, &v2, &app_dir)
        .await
        .unwrap();
    assert_eq!(parts["trust"], json!("store"));
    assert_eq!(summary["from"], json!("0.1.0"));
    assert!(app_dir.join("0.2.0").is_dir());
    assert!(
        !app_dir.join("0.1.0").exists(),
        "the old version's files stayed"
    );
    assert_eq!(
        std::fs::read_to_string(app_dir.join("kv/kept")).unwrap(),
        "the application's own"
    );
    assert_eq!(rig.runtime.status(ID).await["state"], json!("running"));
    let updated = InstallRecord {
        version: "0.2.0".into(),
        parts,
        ..record.clone()
    };
    assert_eq!(
        rig.runtime.describe(&updated, &app_dir)["capabilities"]["env"],
        json!(["SHOWCASE_TOKEN", "SHOWCASE_EXTRA"])
    );
}
