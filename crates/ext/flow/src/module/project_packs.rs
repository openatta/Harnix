//! A project's own flows (`<project>/.harnix/flows/`): read only while the project is trusted,
//! and trust is the engine's (`engine.project.status`, which `engine.control` reaches). Each
//! reader of a project's packs — `flow.defs {project}`, `flow.def`, `flow.start`, an authoring
//! conversation in a project — asks the engine first; what it hears is kept for the kernel's own
//! reads as a run goes on (`ProjectTrust`).

use std::sync::Arc;

use harnix_ext::RpcError;
use serde_json::{json, Value};

use super::shared::Shared;
use super::FLOW_INSTANCE;
use crate::kernel::host::Packs;
use crate::kernel::run::{RunQuery, RunStatus};

/// Whether the engine says `root` is trusted, kept for the kernel's reads. An engine that cannot
/// say is heard as not trusted.
pub(crate) async fn hear(sh: &Shared, root: &str) -> bool {
    let Some(cx) = sh.cx.get() else {
        return sh.projects.trusted(root);
    };
    let status = cx
        .call(
            "engine.project.status",
            json!({"instance": FLOW_INSTANCE, "project_root": root}),
        )
        .await;
    let trusted = match status {
        Ok(v) => v.get("trust").and_then(Value::as_str) == Some("trusted"),
        Err(e) => {
            tracing::warn!(root, error = %e.message, "a project's trust was not read");
            false
        }
    };
    sh.projects.set(root, trusted);
    trusted
}

/// [`hear`] each project an unfinished run works in: what it reads next is its project's.
pub(crate) async fn hear_active(sh: &Shared) {
    let Ok(engine) = sh.engine() else { return };
    let active = RunQuery {
        status: Some(RunStatus::Active),
        ..Default::default()
    };
    let Ok(runs) = engine.list(active).await else {
        return;
    };
    let mut roots: Vec<String> = runs
        .into_iter()
        .filter_map(|r| r.project_root)
        .collect();
    roots.sort();
    roots.dedup();
    for root in roots {
        hear(sh, &root).await;
    }
}

/// A project root a device or a conversation gives: an existing absolute directory, as given
/// but for surrounding space and a trailing `/`.
pub(crate) fn project_root(root: &str) -> Result<String, String> {
    let root = root.trim();
    let trimmed = root.trim_end_matches('/');
    let root = if trimmed.is_empty() { root } else { trimmed };
    if !std::path::Path::new(root).is_absolute() {
        return Err(format!("project_root {root:?} is not an absolute path"));
    }
    match std::fs::metadata(root) {
        Ok(m) if m.is_dir() => Ok(root.to_string()),
        _ => Err(format!("project_root {root:?} is not an existing directory")),
    }
}

/// The packs as a project sees them, its trust heard first; the application's without one.
pub(crate) async fn packs_in(sh: &Arc<Shared>, project: Option<&str>) -> Result<Arc<dyn Packs>, RpcError> {
    let engine = sh.engine()?;
    let Some(root) = project.filter(|r| !r.trim().is_empty()) else {
        return Ok(engine.packs().clone());
    };
    let root = project_root(root).map_err(RpcError::invalid_params)?;
    hear(sh, &root).await;
    Ok(engine.packs_for(Some(&root)))
}

/// Refuse a flow of the project's own while the project is not trusted: in a project its name
/// means the project's, which is not run until then.
pub(crate) fn withheld(packs: &dyn Packs, flow: &str) -> crate::Result<()> {
    match packs.withheld_flows().into_iter().find(|(n, _)| n == flow) {
        Some((_, crate::Withheld::Untrusted)) => Err(crate::FlowError::Denied(format!(
            "flow {flow} is the project's own, and the project is not trusted: trust the project first"
        ))),
        Some((_, crate::Withheld::Unreadable)) => Err(crate::FlowError::Invalid(format!(
            "flow {flow} is the project's own, and its file is not a definition of that name"
        ))),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use super::*;
    use crate::kernel::host::Origin;
    use crate::module::publish::defs_list;
    use crate::module::CatalogPacks;
    use crate::packs::{project_pack_dir, ProjectTrust};
    use crate::{Actor, CreateIn, Engine, FlowError, MemPacks, MemStore, Withheld};

    const BASE: &str = "name: shared\ntitle: Built in\nstations: [{id: a, exit: [{kind: confirm}]}]";
    const OWN: &str = "name: shared\ntitle: The project's\nstations: [{id: a, exit: [{kind: confirm}]}]";
    /// Valid only if a skill it names is there: it is not.
    const LACKING: &str = "name: lacking\nstations: [{id: a, executor: {kind: session, scene: flow-harnix}, skill: nowhere, output: {type: object}, exit: [{kind: output}]}]";

    fn engine(trust: Arc<ProjectTrust>) -> Engine {
        let packs = CatalogPacks {
            inner: OnceLock::new(),
            scenes: vec!["flow-harnix".into()],
            templates: vec![],
            guarded: vec![],
            main_scene: None,
            command_gates: false,
            projects: Some(trust),
            layout: OnceLock::new(),
        };
        let _ = packs.layout.set(super::super::layout(std::path::Path::new("/state/ext/flow/data")));
        let _ = packs.inner.set(Arc::new(MemPacks::new().flow(BASE).writable()));
        Engine::new(Arc::new(MemStore::new()), Arc::new(packs))
    }

    fn start(project: Option<&str>) -> CreateIn {
        CreateIn {
            flow: "shared".into(),
            subject: "s".into(),
            project_root: project.map(String::from),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn a_task_runs_its_projects_own_flow_once_the_project_is_trusted() {
        let root = tempfile::tempdir().unwrap();
        let at = root.path().to_str().unwrap().to_string();
        let dir = project_pack_dir(root.path(), &super::super::layout(std::path::Path::new("/state/ext/flow/data")));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("shared.yaml"), OWN).unwrap();
        std::fs::write(dir.join("lacking.yaml"), LACKING).unwrap();
        let trust = Arc::new(ProjectTrust::default());
        let eng = engine(trust.clone());
        let me = Actor::device("d");

        // Not trusted: listed for the project unread, and its name is not run there.
        let rows = defs_list(eng.packs_for(Some(&at)).as_ref());
        let row = rows.iter().find(|r| r.name == "shared").unwrap();
        assert_eq!((row.origin, row.valid, row.reason), (Origin::Project, false, Some(Withheld::Untrusted)));
        assert!(row.title.is_empty(), "nothing of an untrusted project's is read");
        assert!(matches!(withheld(eng.packs_for(Some(&at)).as_ref(), "shared"), Err(FlowError::Denied(_))));
        let r = eng.create(start(Some(&at)), me.clone()).await;
        assert!(matches!(r, Err(FlowError::NotFound(_))), "{r:?}");
        // Elsewhere the application's.
        let rows = defs_list(eng.packs().as_ref());
        assert_eq!(rows.iter().map(|r| (r.name.as_str(), r.origin)).collect::<Vec<_>>(), [("shared", Origin::Builtin)]);
        let o = eng.create(start(None), me.clone()).await.unwrap();
        let rec = eng.record(o.value["run"].as_str().unwrap()).await.unwrap();
        assert_eq!(rec.def.title, "Built in");

        // Trusted: the project's wins in the project, and the one that is not valid says so.
        trust.set(&at, true);
        let rows = defs_list(eng.packs_for(Some(&at)).as_ref());
        let row = rows.iter().find(|r| r.name == "shared").unwrap();
        assert_eq!((row.origin, row.valid, row.reason, row.title.as_str()), (Origin::Project, true, None, "The project's"));
        let lacking = rows.iter().find(|r| r.name == "lacking").unwrap();
        assert_eq!((lacking.origin, lacking.valid, lacking.reason), (Origin::Project, false, None));
        let o = eng.create(start(Some(&format!("{at}/"))), me.clone()).await.unwrap();
        let rec = eng.record(o.value["run"].as_str().unwrap()).await.unwrap();
        assert_eq!(rec.def.title, "The project's");
        assert_eq!(eng.packs_for_run(&rec.id).await.origin("shared"), Origin::Project);
        // A project flow that is not valid is not started.
        let r = eng.create(CreateIn { flow: "lacking".into(), ..start(Some(&at)) }, me).await;
        assert!(matches!(r, Err(FlowError::Invalid(_))), "{r:?}");
    }

    #[tokio::test]
    async fn saving_in_a_project_goes_to_its_pack_directory_once_valid() {
        let root = tempfile::tempdir().unwrap();
        let at = root.path().to_str().unwrap().to_string();
        let trust = Arc::new(ProjectTrust::default());
        let eng = engine(trust.clone());
        let me = Actor::device("d");
        let yaml = "name: fresh\nstations: [{id: a, exit: [{kind: confirm}]}]";
        let r = eng.save_flow_in(eng.packs_for(Some(&at)).as_ref(), yaml, me.clone()).await;
        assert!(r.is_err(), "nothing is written in an untrusted project");
        trust.set(&at, true);
        let packs = eng.packs_for(Some(&at));
        // Validation as ever: a text with problems is not saved.
        assert!(eng.save_flow_in(packs.as_ref(), LACKING, me.clone()).await.is_err());
        let v = eng.save_flow_in(packs.as_ref(), yaml, me).await.unwrap();
        assert_eq!(v["origin"], "project");
        assert!(project_pack_dir(root.path(), &super::super::layout(std::path::Path::new("/state/ext/flow/data"))).join("fresh.yaml").is_file());
        assert!(eng.packs().flow("fresh").is_none(), "the application's packs are untouched");
    }

    #[test]
    fn a_project_root_is_an_existing_absolute_directory() {
        let root = tempfile::tempdir().unwrap();
        let at = root.path().to_str().unwrap();
        assert_eq!(project_root(&format!(" {at}/ ")).unwrap(), at);
        assert!(project_root("relative").is_err());
        assert!(project_root(&format!("{at}/missing")).is_err());
    }
}
