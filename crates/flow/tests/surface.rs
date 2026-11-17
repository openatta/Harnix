//! The library's public surface, written down: what a host implements (the traits in
//! `kernel::host`), the types those traits hand it, the names `lib.rs` re-exports, and how the
//! stores and packs are built. A change that removes or alters an entry breaks the hosts that use
//! it, and `FLOW_API_VERSION` goes up with it; an addition goes on the list, with a line in the
//! version section of `docs/embed.en.md` and `docs/embed.zh-CN.md`.
//!
//! The source is read as text and parsed (`syn`), so the check holds wherever the directory is
//! copied to. `FLOW_SURFACE=print cargo test -p harnix-flow --test surface` prints what is found,
//! in the form of the tables below.

use std::path::{Path, PathBuf};

use quote::ToTokens;

/// The host traits: each trait's header (`#[async_trait]` included when it is one), then each
/// method's signature, `= default` when it has a body a host may leave alone.
const TRAITS: &[(&str, &str)] = &[
    ("RunStore", "#[async_trait]pubtraitRunStore:Send+Sync"),
    ("RunStore::alloc_id", "asyncfnalloc_id(&self,flow:&str)->Result<String,StoreError>"),
    ("RunStore::load", "asyncfnload(&self,run:&str)->Result<Option<RunRecord>,StoreError>"),
    ("RunStore::list", "asyncfnlist(&self,q:&RunQuery)->Result<Vec<RunSummary>,StoreError>"),
    ("RunStore::commit", "asyncfncommit(&self,cs:&ChangeSet)->Result<(),StoreError>"),
    ("RunStore::history", "asyncfnhistory(&self,run:&str)->Result<Vec<History>,StoreError>"),
    ("Packs", "pubtraitPacks:Send+Sync"),
    ("Packs::flows", "fnflows(&self)->Vec<String>"),
    ("Packs::flow", "fnflow(&self,name:&str)->Option<(FlowDef,String)>"),
    ("Packs::catalog", "fncatalog(&self)->Catalog"),
    ("Packs::flow_text", "fnflow_text(&self,_name:&str)->Option<String> = default"),
    ("Packs::part", "fnpart(&self,kind:PartKind,name:&str)->Option<(String,String)>"),
    ("Packs::parts", "fnparts(&self,_kind:PartKind)->Vec<String> = default"),
    ("Packs::skill", "fnskill(&self,name:&str)->Option<(String,String)> = default"),
    ("Packs::rule", "fnrule(&self,name:&str)->Option<(String,String)> = default"),
    ("Packs::policy", "fnpolicy(&self,name:&str)->Option<PolicyDef> = default"),
    ("Packs::gate", "fngate(&self,name:&str)->Option<GateDef> = default"),
    ("Packs::hook", "fnhook(&self,name:&str)->Option<HookDef> = default"),
    ("Packs::hooks", "fnhooks(&self)->Vec<HookDef> = default"),
    ("Packs::writable", "fnwritable(&self)->bool = default"),
    ("Packs::origin", "fnorigin(&self,_flow:&str)->Origin = default"),
    ("Packs::part_origin", "fnpart_origin(&self,_kind:PartKind,_name:&str)->Origin = default"),
    ("Packs::save_flow", "fnsave_flow(&self,_yaml:&str)->Result<String,String> = default"),
    ("Packs::delete_flow", "fndelete_flow(&self,_name:&str)->Result<(),String> = default"),
    ("Packs::save_part", "fnsave_part(&self,_kind:PartKind,_name:&str,_text:&str)->Result<String,String> = default"),
    ("Packs::delete_part", "fndelete_part(&self,_kind:PartKind,_name:&str)->Result<(),String> = default"),
    ("Packs::project", "fnproject(&self,_root:&str)->Option<std::sync::Arc<dynPacks>> = default"),
    ("Packs::withheld_flows", "fnwithheld_flows(&self)->Vec<(String,Withheld)> = default"),
    ("Authorizer", "pubtraitAuthorizer:Send+Sync"),
    ("Authorizer::check", "fncheck(&self,op:&Op,by:&Actor,run:Option<&RunRecord>)->Result<(),String>"),
    ("Tools", "#[async_trait]pubtraitTools:Send+Sync"),
    ("Tools::call", "asyncfncall(&self,tool:&str,args:&Value)->Result<Value,String>"),
    ("Gates", "#[async_trait]pubtraitGates:Send+Sync"),
    ("Gates::run", "asyncfnrun(&self,run:&RunRecord,station:&str,gate:&str)->GateResult"),
    ("Programs", "#[async_trait]pubtraitPrograms:Send+Sync"),
    ("Programs::run", "asyncfnrun(&self,step:&str,run:&RunRecord,item:Option<&Value>)->Result<Value,String>"),
    ("Clock", "pubtraitClock:Send+Sync"),
    ("Clock::now_ms", "fnnow_ms(&self)->u64"),
];

/// What the traits hand a host and take from it: each struct's public fields and each enum's
/// variants, by shape.
const TYPES: &[(&str, &str)] = &[
    ("StoreError::Conflict", "(String)"),
    ("StoreError::Other", "(String)"),
    ("ChangeSet.records", "Vec<(RunRecord,Option<u64>)>"),
    ("ChangeSet.history", "Vec<History>"),
    ("Event.kind", "String"),
    ("Event.run", "String"),
    ("Event.at", "u64"),
    ("Event.station", "Option<String>"),
    ("Event.task", "Option<String>"),
    ("Event.data", "Value"),
    (
        "WorkItem::Task",
        "{run:String,station:String,round:u32,task:String,token:String,attempt:u32,executor:Executor,\
         skill:Option<String>,skill_digest:Option<String>,rules:Vec<(String,Option<String>)>,\
         permissions:Vec<Granted>,item:Option<Value>,output_schema:Option<Value>,protocol:Option<String>,\
         session_key:Option<String>,session:Option<String>,placement:Option<Placement>,\
         delivery:Option<Delivery>,scene:Option<String>}",
    ),
    ("WorkItem::Gate", "{run:String,station:String,round:u32,gate:String}"),
    ("WorkItem::CloseSession", "{session:String}"),
    ("Outcome.value", "Value"),
    ("Outcome.events", "Vec<Event>"),
    ("Outcome.work", "Vec<WorkItem>"),
    ("Outcome.bookkeeping", "bool"),
];

/// What `lib.rs` offers at the root: each re-exported name with the module it comes from, the
/// public modules, and the constants it defines itself.
const EXPORTS: &[(&str, &str)] = &[
    ("FLOW_API_VERSION", "const:u32"),
    ("DEFINITION_FORMAT", "const:u32"),
    ("RUN_FORMAT", "const:u32"),
    ("dryrun", "mod"),
    ("kernel", "mod"),
    ("packs", "mod"),
    ("store", "mod"),
    ("wire", "mod"),
    ("Mocks", "dryrun"),
    ("Sandbox", "dryrun"),
    ("definition_schema", "kernel::def"),
    ("Assert", "kernel::def"),
    ("Executor", "kernel::def"),
    ("ExitCond", "kernel::def"),
    ("FlowDef", "kernel::def"),
    ("Guard", "kernel::def"),
    ("InputDef", "kernel::def"),
    ("MainSession", "kernel::def"),
    ("OnUnmet", "kernel::def"),
    ("Retry", "kernel::def"),
    ("SessionBinding", "kernel::def"),
    ("Station", "kernel::def"),
    ("TaskSpec", "kernel::def"),
    ("Work", "kernel::def"),
    ("DEFAULT_OUTPUT_DIR", "kernel::def"),
    ("DEFAULT_REWORK_LIMIT", "kernel::def"),
    ("MAIN_SESSION_KEY", "kernel::def"),
    ("MAX_REWORK_LIMIT", "kernel::def"),
    ("view", "kernel::engine"),
    ("worktree_may_end", "kernel::engine"),
    ("CreateIn", "kernel::engine"),
    ("Engine", "kernel::engine"),
    ("Prepared", "kernel::engine"),
    ("DEFAULT_LEASE_MS", "kernel::engine"),
    ("DEFAULT_RETRIES", "kernel::engine"),
    ("MAX_LEASE_MS", "kernel::engine"),
    ("MAX_SUBFLOW_DEPTH", "kernel::engine"),
    ("digest", "kernel::host"),
    ("AllowAll", "kernel::host"),
    ("Authorizer", "kernel::host"),
    ("Catalog", "kernel::host"),
    ("ChangeSet", "kernel::host"),
    ("Clock", "kernel::host"),
    ("Delivery", "kernel::host"),
    ("Event", "kernel::host"),
    ("FixedClock", "kernel::host"),
    ("GateResult", "kernel::host"),
    ("Gates", "kernel::host"),
    ("ManualClock", "kernel::host"),
    ("NoTools", "kernel::host"),
    ("Op", "kernel::host"),
    ("Origin", "kernel::host"),
    ("Outcome", "kernel::host"),
    ("Packs", "kernel::host"),
    ("Placement", "kernel::host"),
    ("Programs", "kernel::host"),
    ("RunStore", "kernel::host"),
    ("StoreError", "kernel::host"),
    ("SystemClock", "kernel::host"),
    ("Tools", "kernel::host"),
    ("Withheld", "kernel::host"),
    ("WorkItem", "kernel::host"),
    ("EVENT_KINDS", "kernel::host"),
    ("glob", "kernel::parts"),
    ("judge", "kernel::parts"),
    ("judge_under", "kernel::parts"),
    ("part_schema", "kernel::parts"),
    ("permissions_of", "kernel::parts"),
    ("rules_of", "kernel::parts"),
    ("Call", "kernel::parts"),
    ("Effect", "kernel::parts"),
    ("GateDef", "kernel::parts"),
    ("GateKind", "kernel::parts"),
    ("Granted", "kernel::parts"),
    ("HookDef", "kernel::parts"),
    ("HookKind", "kernel::parts"),
    ("PartKind", "kernel::parts"),
    ("Permission", "kernel::parts"),
    ("Pinned", "kernel::parts"),
    ("PolicyDef", "kernel::parts"),
    ("Verdict", "kernel::parts"),
    ("agent_tag", "kernel::protocol"),
    ("ask_rules", "kernel::protocol"),
    ("skill_body", "kernel::protocol"),
    ("STRUCTURED_OUTPUT_TOOL", "kernel::protocol"),
    ("run_schema", "kernel::run"),
    ("Actor", "kernel::run"),
    ("CheckRec", "kernel::run"),
    ("CheckStatus", "kernel::run"),
    ("ConfirmRec", "kernel::run"),
    ("ForcedRec", "kernel::run"),
    ("History", "kernel::run"),
    ("Lease", "kernel::run"),
    ("ParentRef", "kernel::run"),
    ("AskRec", "kernel::run"),
    ("PauseRec", "kernel::run"),
    ("Phase", "kernel::run"),
    ("RunQuery", "kernel::run"),
    ("RunRecord", "kernel::run"),
    ("RunStatus", "kernel::run"),
    ("RunSummary", "kernel::run"),
    ("RunView", "kernel::run"),
    ("StationRec", "kernel::run"),
    ("StationStatus", "kernel::run"),
    ("TaskRec", "kernel::run"),
    ("TaskStatus", "kernel::run"),
    ("Trigger", "kernel::run"),
    ("Usage", "kernel::run"),
    ("Workspace", "kernel::run"),
    ("WorktreeEnd", "kernel::run"),
    ("WorktreeOutcome", "kernel::run"),
    ("WorktreeRec", "kernel::run"),
    ("AGENT_KIND", "kernel::run"),
    ("DEVICE_KIND", "kernel::run"),
    ("FLOW_KIND", "kernel::run"),
    ("minimal", "kernel::schema"),
    ("part_problems", "kernel::validate"),
    ("validate", "kernel::validate"),
    ("validate_yaml", "kernel::validate"),
    ("Problem", "kernel::validate"),
    ("FlowError", "kernel"),
    ("Result", "kernel"),
    ("project_pack_dir", "packs"),
    ("DirPacks", "packs"),
    ("MemPacks", "packs"),
    ("ProjectPacks", "packs"),
    ("ProjectTrust", "packs"),
    ("FileStore", "store"),
    ("Hooked", "store"),
    ("MemStore", "store"),
    ("GUIDE_EN", "const:&str"),
    ("GUIDE_ZH", "const:&str"),
];

/// How the stores and packs are built and what else their own impls offer: every public method of
/// their inherent impls.
const BUILDERS: &[(&str, &str)] = &[
    ("MemStore::new", "fnnew()->Self"),
    ("FileStore::new", "fnnew(dir:implInto<PathBuf>)->Self"),
    ("FileStore::dir", "fndir(&self)->&Path"),
    ("Hooked::new", "fnnew(inner:S)->Self"),
    ("Hooked::ids", "fnids(mutself,ids:implFn(&str)->String+Send+Sync+'static)->Self"),
    ("Hooked::check_commits", "fncheck_commits(mutself,check:implFn(&ChangeSet)->Result<(),StoreError>+Send+Sync+'static,)->Self"),
    ("Hooked::inner", "fninner(&self)->&S"),
    ("MemPacks::new", "fnnew()->Self"),
    ("MemPacks::flow", "fnflow(self,yaml:&str)->Self"),
    ("MemPacks::skill", "fnskill(self,name:&str,body:&str)->Self"),
    ("MemPacks::part", "fnpart(self,kind:PartKind,name:&str,text:&str)->Self"),
    ("MemPacks::rule", "fnrule(self,name:&str,text:&str)->Self"),
    ("MemPacks::policy", "fnpolicy(self,yaml:&str)->Self"),
    ("MemPacks::gate_def", "fngate_def(self,yaml:&str)->Self"),
    ("MemPacks::hook", "fnhook(self,yaml:&str)->Self"),
    ("MemPacks::scene", "fnscene(self,name:&str)->Self"),
    ("MemPacks::step", "fnstep(self,name:&str)->Self"),
    ("MemPacks::check", "fncheck(self,name:&str)->Self"),
    ("MemPacks::tool", "fntool(self,name:&str)->Self"),
    ("MemPacks::command_gates", "fncommand_gates(self)->Self"),
    ("MemPacks::template", "fntemplate(self,name:&str)->Self"),
    ("MemPacks::instance", "fninstance(self,name:&str)->Self"),
    ("MemPacks::space", "fnspace(self,name:&str)->Self"),
    ("MemPacks::model", "fnmodel(self,name:&str)->Self"),
    ("MemPacks::submit_scene", "fnsubmit_scene(self,name:&str)->Self"),
    ("MemPacks::main_scene", "fnmain_scene(self,name:&str)->Self"),
    ("MemPacks::writable", "fnwritable(self)->Self"),
    ("MemPacks::put_flow", "fnput_flow(&self,yaml:&str)"),
    ("MemPacks::put_part", "fnput_part(&self,kind:PartKind,name:&str,text:&str)"),
    ("MemPacks::flow_text", "fnflow_text(&self,name:&str)->Option<String>"),
    ("DirPacks::new", "fnnew(dirs:implIntoIterator<Item=implInto<PathBuf>>)->Self"),
    ("DirPacks::writable", "fnwritable(dir:implInto<PathBuf>)->Self"),
    ("DirPacks::with_writable", "fnwith_writable(mutself,dir:implInto<PathBuf>)->Self"),
    ("DirPacks::with_builtin", "fnwith_builtin(mutself,packs:MemPacks)->Self"),
    ("DirPacks::with_catalog", "fnwith_catalog(mutself,catalog:Catalog)->Self"),
    ("DirPacks::writable_dir", "fnwritable_dir(&self)->Option<&Path>"),
    ("DirPacks::flow_text", "fnflow_text(&self,name:&str)->Option<String>"),
    ("ProjectPacks::new", "fnnew(base:Arc<dynPacks>,root:implInto<PathBuf>,trusted:bool)->Self"),
    ("ProjectPacks::dir", "fndir(&self)->&Path"),
    ("ProjectPacks::trusted", "fntrusted(&self)->bool"),
    ("ProjectTrust::set", "fnset(&self,root:&str,trusted:bool)"),
    ("ProjectTrust::trusted", "fntrusted(&self,root:&str)->bool"),
    ("ProjectTrust::any_trusted", "fnany_trusted(&self)->bool"),
];

/// The types whose inherent impls [`BUILDERS`] lists, and the file each is in.
const BUILT: &[(&str, &str)] = &[
    ("MemStore", "src/store.rs"),
    ("FileStore", "src/store.rs"),
    ("Hooked", "src/store.rs"),
    ("MemPacks", "src/packs.rs"),
    ("DirPacks", "src/packs.rs"),
    ("ProjectPacks", "src/packs.rs"),
    ("ProjectTrust", "src/packs.rs"),
];

const HOST: &str = "src/kernel/host.rs";
const LIB: &str = "src/lib.rs";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn parse(file: &str) -> syn::File {
    let path = root().join(file);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    syn::parse_file(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn squash(tokens: impl ToTokens) -> String {
    tokens.to_token_stream().to_string().chars().filter(|c| !c.is_whitespace()).collect()
}

fn public(vis: &syn::Visibility) -> bool {
    matches!(vis, syn::Visibility::Public(_))
}

fn async_trait(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| a.path().segments.last().is_some_and(|s| s.ident == "async_trait"))
}

/// `expected` must all be in `found`, unchanged; anything else in `found` is an addition not yet
/// written down.
fn compare(what: &str, table: &str, expected: &[(&str, &str)], found: &[(String, String)]) -> Vec<String> {
    if std::env::var_os("FLOW_SURFACE").is_some_and(|v| v == "print") {
        println!("const {table}: &[(&str, &str)] = &[");
        for (name, shape) in found {
            println!("    ({name:?}, {shape:?}),");
        }
        println!("];");
    }
    let mut problems = Vec::new();
    for (name, shape) in expected {
        match found.iter().find(|(n, _)| n == name) {
            None => problems.push(format!("{what} `{name}` is gone: that breaks every host using it")),
            Some((_, actual)) if actual != shape => {
                problems.push(format!("{what} `{name}` changed from `{shape}` to `{actual}`: that breaks hosts"))
            }
            Some(_) => {}
        }
    }
    for (name, shape) in found {
        if !expected.iter().any(|(n, _)| n == name) {
            problems.push(format!(
                "{what} `{name}` (`{shape}`) is new: add it to {table} in tests/surface.rs and a line to the \
                 version section of docs/embed.*.md"
            ));
        }
    }
    problems
}

fn traits() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for item in parse(HOST).items {
        let syn::Item::Trait(t) = item else { continue };
        if !public(&t.vis) {
            continue;
        }
        let marker = if async_trait(&t.attrs) { "#[async_trait]" } else { "" };
        let bounds = if t.supertraits.is_empty() { String::new() } else { format!(":{}", squash(&t.supertraits)) };
        out.push((t.ident.to_string(), format!("{marker}pubtrait{}{}{bounds}", t.ident, squash(&t.generics))));
        for item in &t.items {
            if let syn::TraitItem::Fn(f) = item {
                let default = if f.default.is_some() { " = default" } else { "" };
                out.push((format!("{}::{}", t.ident, f.sig.ident), format!("{}{default}", squash(&f.sig))));
            }
        }
    }
    out
}

fn fields(fields: &syn::Fields) -> String {
    match fields {
        syn::Fields::Unit => String::new(),
        syn::Fields::Unnamed(u) => {
            format!("({})", u.unnamed.iter().map(|f| squash(&f.ty)).collect::<Vec<_>>().join(","))
        }
        syn::Fields::Named(n) => format!(
            "{{{}}}",
            n.named
                .iter()
                .map(|f| format!("{}:{}", f.ident.as_ref().unwrap(), squash(&f.ty)))
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}

fn types() -> Vec<(String, String)> {
    const NAMES: &[&str] = &["StoreError", "ChangeSet", "Event", "WorkItem", "Outcome"];
    let mut out = Vec::new();
    for item in parse(HOST).items {
        match item {
            syn::Item::Struct(s) if NAMES.contains(&s.ident.to_string().as_str()) => {
                for f in s.fields.iter().filter(|f| public(&f.vis)) {
                    out.push((format!("{}.{}", s.ident, f.ident.as_ref().unwrap()), squash(&f.ty)));
                }
            }
            syn::Item::Enum(e) if NAMES.contains(&e.ident.to_string().as_str()) => {
                for v in &e.variants {
                    out.push((format!("{}::{}", e.ident, v.ident), fields(&v.fields)));
                }
            }
            _ => {}
        }
    }
    for name in NAMES {
        assert!(
            out.iter().any(|(n, _)| n.starts_with(&format!("{name}.")) || n.starts_with(&format!("{name}::"))),
            "`{name}` is not in {HOST}"
        );
    }
    out
}

fn exports() -> Vec<(String, String)> {
    fn flatten(prefix: &str, tree: &syn::UseTree, out: &mut Vec<(String, String)>) {
        match tree {
            syn::UseTree::Path(p) => {
                let prefix = if prefix.is_empty() { p.ident.to_string() } else { format!("{prefix}::{}", p.ident) };
                flatten(&prefix, &p.tree, out);
            }
            syn::UseTree::Name(n) => out.push((n.ident.to_string(), prefix.to_string())),
            syn::UseTree::Rename(r) => out.push((r.rename.to_string(), format!("{prefix}::{}", r.ident))),
            syn::UseTree::Glob(_) => out.push((format!("{prefix}::*"), prefix.to_string())),
            syn::UseTree::Group(g) => g.items.iter().for_each(|t| flatten(prefix, t, out)),
        }
    }
    let mut out = Vec::new();
    for item in parse(LIB).items {
        match item {
            syn::Item::Use(u) if public(&u.vis) => flatten("", &u.tree, &mut out),
            syn::Item::Mod(m) if public(&m.vis) => out.push((m.ident.to_string(), "mod".into())),
            syn::Item::Const(c) if public(&c.vis) => out.push((c.ident.to_string(), format!("const:{}", squash(&c.ty)))),
            _ => {}
        }
    }
    out
}

fn builders() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (ty, file) in BUILT {
        let before = out.len();
        for item in parse(file).items {
            let syn::Item::Impl(i) = item else { continue };
            if i.trait_.is_some() {
                continue;
            }
            let syn::Type::Path(p) = &*i.self_ty else { continue };
            if p.path.segments.last().map(|s| s.ident.to_string()).as_deref() != Some(ty) {
                continue;
            }
            for item in &i.items {
                if let syn::ImplItem::Fn(f) = item {
                    if public(&f.vis) {
                        out.push((format!("{ty}::{}", f.sig.ident), squash(&f.sig)));
                    }
                }
            }
        }
        assert!(out.len() > before, "`{ty}` has no public method in {file}");
    }
    out
}

#[test]
fn the_host_traits_keep_their_methods() {
    let problems = compare("trait item", "TRAITS", TRAITS, &traits());
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn what_the_traits_hand_over_keeps_its_shape() {
    let problems = compare("field or variant", "TYPES", TYPES, &types());
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_root_keeps_its_names() {
    let problems = compare("export", "EXPORTS", EXPORTS, &exports());
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_stores_and_packs_are_built_as_they_were() {
    let problems = compare("method", "BUILDERS", BUILDERS, &builders());
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
