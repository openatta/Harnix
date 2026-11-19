//! A project's own flows (`<project>/<dir>/flows/`): read only while the project is trusted, and
//! trust is the host's (`SessionHost::project_trusted`; Harnix asks its engine). Each reader of a
//! project's packs — a start, a run in the project, the host's own readers — asks first; what is
//! heard is kept for the kernel's own reads as a run goes on (`ProjectTrust`), and a host that
//! hears of a change pushes it (`Runner::set_trust`).

use std::path::Path;

use super::Runner;
use crate::kernel::host::Packs;
use crate::kernel::run::{RunQuery, RunStatus};

/// Whether the host says `root` is trusted, kept for the kernel's reads. A host that cannot say
/// is heard as not trusted.
pub(crate) async fn hear(rn: &Runner, root: &str) -> bool {
    let trusted = match rn.host().project_trusted(Path::new(root)).await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(root, error = %e, "a project's trust was not read");
            false
        }
    };
    rn.trust().set(root, trusted);
    trusted
}

/// [`hear`] each project an unfinished run works in: what it reads next is its project's.
pub(crate) async fn hear_active(rn: &Runner) {
    let active = RunQuery { status: Some(RunStatus::Active), ..Default::default() };
    let Ok(runs) = rn.engine().list(active).await else {
        return;
    };
    let mut roots: Vec<String> = runs
        .into_iter()
        .filter_map(|r| r.project_root)
        .collect();
    roots.sort();
    roots.dedup();
    for root in roots {
        hear(rn, &root).await;
    }
}

/// Refuse a flow of the project's own while the project is not trusted: in a project its name
/// means the project's, which is not run until then. A plugin's flow another plugin's is offered
/// over withholds nothing: the name runs the one offered.
pub fn withheld(packs: &dyn Packs, flow: &str) -> crate::Result<()> {
    match packs.withheld_flows().into_iter().find(|(n, why)| n == flow && *why != crate::Withheld::Shadowed) {
        Some((_, crate::Withheld::Untrusted)) => Err(crate::FlowError::Denied(format!(
            "flow {flow} is the project's own, and the project is not trusted: trust the project first"
        ))),
        Some((_, crate::Withheld::Unreadable)) => Err(crate::FlowError::Invalid(format!(
            "flow {flow} is the project's own, and its file is not a definition of that name"
        ))),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::kernel::host::Origin;
    use crate::packs::{project_pack_dir, ProjectTrust};
    use crate::runner::{CatalogPacks, RunnerConfig, ScriptedHost};
    use crate::wire::defs_list;
    use crate::{Actor, CreateIn, Engine, FlowError, MemPacks, MemStore, ProjectLayout, Withheld};

    fn layout() -> ProjectLayout {
        ProjectLayout::new(".harnix", "/state/ext/flow/data/worktrees", "harnix/flow")
    }

    const BASE: &str = "name: shared\ntitle: Built in\nstations: [{id: a, exit: [{kind: confirm}]}]";
    const OWN: &str = "name: shared\ntitle: The project's\nstations: [{id: a, exit: [{kind: confirm}]}]";
    /// Valid only if a skill it names is there: it is not.
    const LACKING: &str = "name: lacking\nstations: [{id: a, executor: {kind: session, scene: flow-harnix}, skill: nowhere, output: {type: object}, exit: [{kind: output}]}]";

    fn engine(trust: Arc<ProjectTrust>) -> Engine {
        let cfg = RunnerConfig { profiles: vec!["flow-harnix".into()], guarded: vec![], trust, ..Default::default() };
        let packs = CatalogPacks::over(&cfg, vec![], Arc::new(MemPacks::new().flow(BASE).writable()));
        packs.set_layout(layout());
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
        let dir = project_pack_dir(root.path(), &layout());
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
        assert!(project_pack_dir(root.path(), &layout()).join("fresh.yaml").is_file());
        assert!(eng.packs().flow("fresh").is_none(), "the application's packs are untouched");
    }

    /// Trust is the host's word, kept: asked once per read, and pushed when the host hears of a
    /// change.
    #[tokio::test]
    async fn trust_is_heard_from_the_host_and_kept() {
        let host = ScriptedHost::new();
        let trust = Arc::new(ProjectTrust::default());
        let eng = Arc::new(engine(trust.clone()));
        let rn = crate::runner::Runner::for_tests_with(eng, host.clone(), RunnerConfig { trust: trust.clone(), ..Default::default() });
        assert!(!hear(&rn, "/p").await);
        host.trust(Path::new("/p"), true);
        assert!(hear(&rn, "/p").await);
        assert!(trust.trusted("/p"));
        rn.set_trust(Path::new("/p"), false);
        assert!(!trust.trusted("/p"), "what the host pushes is kept too");
        host.unavailable(true);
        assert!(!hear(&rn, "/p").await, "a host that cannot say is heard as not trusted");
    }
}
