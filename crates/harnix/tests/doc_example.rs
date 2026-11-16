//! `docs/harnix-ext.md` §5.1's example extension, as written there with its two placeholders
//! (`Db`, `Group`, and what `State.key` is used for) filled in: it has to compile against the facade.

use harnix::prelude::*;
use serde::Deserialize;

#[derive(Default)]
pub struct Watchlist;

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    db: String,
    quote_api_key: Option<CredentialRef>,  // { credential = "helm/quote-api" }
}

#[allow(dead_code)]
struct State {
    db: Db,
    key: Option<harnix::Secret>,
}

struct Db;
impl Db {
    fn open(_path: std::path::PathBuf) -> anyhow::Result<Db> {
        Ok(Db)
    }
    fn groups(&self, _filter: Option<String>) -> Result<Vec<Group>, RpcError> {
        Ok(Vec::new())
    }
}

#[derive(serde::Serialize)]
struct Group;

#[derive(Deserialize)]
struct ListParams {
    filter: Option<String>,
}

#[harnix::async_trait]
impl Extension for Watchlist {
    type Config = Config;

    fn manifest(&self) -> Manifest {
        Manifest::new("watchlist", env!("CARGO_PKG_VERSION")).disclosure("读写关注组；提供「关注组」space")
    }

    fn register(&self, r: &mut Registrar<'_, Config>) {
        r.method("list")
            .access(Access::device())
            .describe("关注组列表")
            .schema(serde_json::json!({"type": "object"}))
            .handler(list_groups);
        r.topic("groups").published();
        r.topic("job").param("job_id").published();
        r.space(SpaceDecl::new("watchlist"));
    }

    async fn start(&self, cx: ExtensionContext) -> anyhow::Result<()> {
        let cfg: Config = cx.config()?;
        let key = match &cfg.quote_api_key {
            Some(reference) => Some(cx.credentials().get(&reference.credential).map_err(anyhow::Error::msg)?),
            None => None,
        };
        let db = Db::open(cx.data_dir().join(&cfg.db))?;
        cx.provide(State { db, key });
        cx.topics().publish("groups", serde_json::json!({}), Vec::<Group>::new()).await?;
        Ok(())
    }
}

async fn list_groups(call: Call, p: ListParams) -> Result<Vec<Group>, RpcError> {
    call.state::<State>()?.db.groups(p.filter)
}

#[test]
fn the_example_extension_registers() {
    let _ = harnix::App::builder().extension(Watchlist).ungranted();
}
