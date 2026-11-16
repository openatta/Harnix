# Harnix 原生扩展：用 Rust 在 Harnix 上做应用

> 读者：在 Harnix 上做产品的人（Helm、IPDrun、Grabix，以及以后的应用）。
>
> 这是**原生扩展**的文档：用 Rust 写扩展，把 Harnix 装配成一个产品。原生扩展是模式 1（纯 Rust 应用）的全部，
> 也是模式 2 / 3 里产品自己那一半——产品自己的领域逻辑写成原生扩展，第三方的东西装成扩展包。
> 三种模式怎么选见 [应用开发](app-development.md)；扩展包（WASM 扩展）怎么写见 [扩展包](harnix-ext-package.md)；
> Harnix 自带的 Harnix app 见 [Harnix app](harnix-app.md)，系统 space 见 [系统 space](system-space.md)。
>
> 分两部分。**第一部分（§1–§14）是指南**：怎么做、为什么。**第二部分（§16 起）是接口清单**：每个类型、方法、
> profile 键、命令行开关、目录、线上方法与主题、贡献点的形状，以及迁移时要知道的缺口。指南讲到接口就链到第二部分，不重复。
>
> 示例就是真实的签名：`crates/testapp`、`examples/hello-app` 是能编译、有测试的完整例子，`templates/app`、
> `templates/desktop` 是起步用的模板。用词照 [术语](glossary.md)。为什么是这个形状见 [概念与架构](concept_and_architecture.md)。

---

## 1. Harnix 是什么

**Harnix 是 Agent 应用的公共底座**：一个框架，加上用这个框架做的一组第一方扩展（Harnix app 的会话 space 与项目 space、
始终在的系统 space、设置、设备……）。你做的产品是框架上的另一组扩展，和第一方扩展同一种做法，只是更大。
你的应用只认 Harnix：仓库里一个子仓 `harnix/`，代码里一个 crate `harnix`（前端是 `@harnix/*` 的包）。
Agent 引擎是 Harnix 带进来的，你不单独引入它、不钉它的版本，也不直接找它。

| 层 | 管什么 | 你要不要改它 |
|---|---|---|
| **Harnix**（含它带的 Agent 引擎） | 轮次循环、工具、权限、上下文、压缩、Agix 插件；会话的持有与追赶、多设备、授权、传输、外壳、space、实例管理 | 不 fork。缺什么、哪里坏了，提 Harnix 的 issue（§2.1）；公共改动在 `harnix/` 子仓里做、回推 |
| **你的应用** | 领域服务、数据库、产品面板、场景与原生工具、品牌 | 全是你的 |

运行时按四层组织：

```text
Harnix 进程
 ├─ 实例 main（进程内）             ← 隔离单位：数据根、设置、provider、MCP、插件、场景
 │   ├─ 会话 a                      ← 一条轮次循环，同一时刻至多一轮
 │   │   └─ 任务：子 Agent、后台 Shell、定时唤醒……
 │   └─ 会话 b
 ├─ 实例 research（进程内，你的扩展按模板建的，带你的原生工具）
 └─ 实例 sandbox（daemon，本机另一个进程，配置完全独立）
```

界面是三栏：左栏是 space 入口与条目，中区是所有 space 共用的对话，右栏是当前 space 的页签。
**一个 space 有三个部分，任意组合**：

| 部分 | 你写什么 | 在哪跑 |
|---|---|---|
| 前端 | React 组件 + 注册（一个 UI 包） | 浏览器 |
| 后端 | 一个 Rust `Extension` | Harnix 进程内 |
| Agent | 原生工具、场景层（经扩展的 `engine()`，类型从 `harnix::engine` 取）；或 Agix 插件（运行期包） | 实例里 |

**有几件事是写死的，你改不了，也不该想改**：传输的通道语义、授权（默认拒绝）、中枢（会话的唯一
持有者，「中枢永远比任何客户端先订阅」）、装配顺序。这些正是「刷新不丢内容、多设备一致、turn 不挂
在连接上」的来源。

**扩展是一个概念，三种载体**：原生（本文，Rust 编进二进制，信任按出身）、WASM 同进程（模式 2，缺省）、WASM 进程（模式 3，选用）。
后两种是扩展包，由第一方扩展 `packages` 代为登记，权力来自人的同意；它们登记进的是同一张路由表。
要让别人往你的产品里装东西（第三方的工具、界面、后端逻辑），见 [应用开发](app-development.md) 与 [扩展包](harnix-ext-package.md)：
Harnix 自己发布的二进制缺省就带模式 2；你的产品用 `harnix::first_party::standard_with(..)` 或自己列 `packages` 即可。

## 2. 什么写在应用里，什么放进 Harnix

> **只有在公开契约之上做不出来，或者做出来会破坏 Harnix 不变量的东西，才进 Harnix。**

两个应用都要，不等于要进 Harnix。先在你的应用里用扩展接口做；等两份实现证明是同一个形状，再提议
收进 Harnix。

| 例子 | 放哪 | 为什么 |
|---|---|---|
| 交易、关注组、需求规格、工位、xflow | 应用 | 领域的东西 |
| 通用的流程引擎：站、任务、出口条件、返工、租约 | Harnix 的可选第一方扩展 `harnix-ext-flow`，应用列进去才有 | IPDrun、XOps 两份实现是同一个形状；流程**定义**和领域的站仍在应用里。站的会话可落到任一实例、模板实例、space 与模型，经结构化输出交付、经权限规则限写。Harnix 自己的二进制带它；`harnix` 门面不带（§23） |
| sqlite 运行态、git 原语、代码扫描 | 应用 | 扩展自己选库、自己管数据 |
| 批准卡（交易要人点头） | 应用 | 扩展接口够用，做法见 §10.1 |
| MCP 服务端、角色令牌 | 应用 | Harnix 暂不提供，见 §10.4 |
| 无头会话、tap、OnBusy | Harnix | 要和客户端共用排队与 `seq`，在外面做不出来 |
| 会话和 space 的对应关系 | Harnix | 各写一份就会串会话 |
| 一个通用的界面组件缺了个开关 | Harnix（回推） | 契约的一部分 |

### 2.1 问题与需求：只提 Harnix 的 issue

缺一个能力、碰到一个缺陷，不管它最后落在哪一层，都在 [Harnix 的 GitHub issue](https://github.com/openatta/Harnix/issues)
里提：标题 `<项目>-NN：一句话`，打上你项目的标签，正文写现象或需求（带代码位置）、想要的接口、验收方式。
在哪一层实现、要不要改引擎、什么时候进哪个版本，由 Harnix 决定；落地后 issue 里回复提交与版本标签，你升级 `harnix/` 子仓就拿到。
应用不跟踪引擎的版本，也不在引擎那边开分支。

## 3. 应用仓库长什么样

```text
<app>/
├── Cargo.toml              workspace 根，唯一的锁文件
├── Cargo.lock
├── harnix/                 子仓 → Harnix（唯一的子仓；--recursive 带上它依赖的一切）
├── app/                    bin crate：main.rs 里 App::builder()…
├── spaces/
│   └── <x>/
│       ├── backend/        Rust crate，实现 harnix::ext::Extension（可选）
│       ├── agent/          原生工具与场景，可以并进 backend（可选）
│       └── ui/             pnpm 包：React 组件 + register()（可选）
├── pnpm-workspace.yaml     packages: spaces/*/ui（不收 harnix/ui/packages/*，见下）
├── profiles/<app>.toml
└── scripts/build.sh        pnpm build → cargo build
```

**只有 `harnix/` 一个子仓。** 它依赖的东西（包括 Agent 引擎）由它自己带，`git submodule update --init --recursive`
一次取齐，版本永远是这一版 Harnix 测过的那一份。你要的改动不管在哪一层，都提 Harnix 的 issue（§2.1）。

**两个 pnpm workspace，互不相交。** `harnix/` 用它自己的（`pnpm --dir harnix install`，外壳在那里构建，Vite
把 `react`、`@harnix/react`、`@harnix/core` 去重到外壳那一份）；应用根下自己的 workspace 只收 `spaces/*/ui`
和测试包，`@harnix/react` 用 `link:../../../harnix/ui/packages/react` 引进来，`react` 的版本写成和 Harnix 一样。
退路是根 workspace 收 `harnix/ui/packages/*`、只装一次。
照这样做还要知道三点：

- 两边都要装：模板的 `scripts/build.sh` 先 `pnpm --dir harnix install`，应用根有 `pnpm-workspace.yaml` 时再 `pnpm install`，然后才构建外壳——
  外壳构建要从应用的 workspace 里解析 space 的 import（第三方包也是）。
- 类型检查：`@harnix/react` 的 `exports` 指向 `.ts` 源码，应用的 `tsc` 会一起检查它，所以应用的 tsconfig 要有
  `allowImportingTsExtensions`（继承 `harnix/tsconfig.base.json` 就有；继承自己的 base 时自己加）。
- 组件测试里的两份 React：见 §12。

Cargo 这边要知道三件事：

1. 应用的 workspace 把 `harnix/crates/*`、`harnix/crates/ext/*` 与 `harnix/crates/carrier/*` 收为**成员**，
   `exclude = ["harnix/agix", "harnix/crates/ext", "harnix/crates/carrier", "harnix/crates/testapp"]`：
   crate 分在 `ext/`、`carrier/` 两层之后，只写 `harnix/crates/*` 不够，Cargo 会把没有清单的 `crates/ext` 当成成员而报错。
2. 根 `Cargo.toml` 里有一段带标记的区块，由 `harnix/scripts/sync-app-deps.mjs` 生成，**不要手改**。它把
   Harnix 的 crate 需要的依赖指到以应用根为基准的路径。升级 Harnix 之后重新跑一次。你自己的 crate 只写
   `harnix = { workspace = true }`，用不着这一段里的其他名字。
3. `[profile.release]`、`[patch]` 写在应用根，Harnix 根目录里的那些对你不生效。模板里已经带好了。

**profile 是产品不写代码就能定的那一半**，一个产品一份（`profiles/<app>.toml`）：

| 段 | 定什么 |
|---|---|
| `[brand]` | 产品是谁：名字、图标、程序自己的目录 `~/.<product_id>`（§5.4、§8.6） |
| `[shell]`、`[ui]` | 外壳的几样行为；界面从哪来、扩展包的界面进不进页面（§13） |
| `[extensions]` | `off`：关掉链进来的扩展，连同它的行一起撤；`default_space` |
| `[engine]` | `main` 实例的场景与模型 |
| `[[instances]]`、`[[instance_templates]]` | 固定的实例；界面能按它建实例的模板（§6.2） |
| `[ext.<id>]` | 你的扩展自己的节，`Extension::Config` 读；密钥只写引用（`{ credential = "helm/quote-api" }`） |
| `[ext.packages]`、`[ext.plugins]` | 扩展包的宿主（商店、开发模式、载体、沙箱）、Agix 插件的界面模块 |
| `[transport]` | 怎么监听 |

全部的键、缺省与例子见 §18。**每一节有一个所有者**：没有所有者的节、节里不认识的键，都拒绝启动并说是哪一行；
`[ext.<id>]` 的内容由那个扩展的 `Config` 类型读，你的扩展可以选择拒绝未知键（第一方的一律拒绝）。

## 4. 起步：从模板建一个应用

1. 复制 `harnix/templates/app` 到新仓库；`git submodule add` 加上 `harnix/`，再
   `git submodule update --init --recursive`。
2. 跑 `node harnix/scripts/sync-app-deps.mjs .`，再跑 `node harnix/scripts/check-pins.mjs .`。
3. `scripts/build.sh`，然后 `./target/release/<app> --profile profiles/<app>.toml`。
4. 打开启动链接，看到的是你在 `main.rs` 里列进去的第一方扩展的界面：模板列的是 `harnix::first_party::standard()`——
   Harnix app 的会话、项目两个 space（[Harnix app](harnix-app.md)），底部的系统 space（[系统 space](system-space.md)），头像即设置。
   流程页只在你列了 `flow` 扩展时出现（§23）。
5. 在 `spaces/` 下加第一个 space，在 `app/src/main.rs` 里 `.extension(...)` 加上它；它的界面入口
   `spaces/<x>/ui/src/index.tsx` 由模板的 `scripts/build.sh` 自动接进构建（§8.2）。
6. 在 profile 里写上 `[brand]`；不要某个第一方扩展就不列它，或者 `[extensions] off = ["system"]`。

## 5. 后端：写一个扩展

### 5.1 最小的扩展

```rust
use harnix::prelude::*;
use serde::Deserialize;

#[derive(Default)]
pub struct Watchlist;

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    db: String,
    quote_api_key: Option<CredentialRef>,  // { credential = "helm/quote-api" }；没有 Default，缺省要 Option
}

struct State { db: Db, key: Option<harnix::Secret> }

#[derive(Deserialize)]
struct ListParams { filter: Option<String> }

#[harnix::async_trait]
impl Extension for Watchlist {
    type Config = Config;                  // [ext.watchlist]；没有配置写 ()

    fn manifest(&self) -> Manifest {
        Manifest::new("watchlist", env!("CARGO_PKG_VERSION"))
            .disclosure("读写关注组；提供「关注组」space")
    }

    fn register(&self, r: &mut Registrar<'_, Config>) {
        // 全名 watchlist.list；缺省谁都不能调，这里给设备
        r.method("list")
            .access(Access::device())
            .describe("关注组列表")
            .schema(serde_json::json!({"type": "object"}))   // 可选，进诊断页
            .handler(list_groups);                           // 参数按类型反序列化，不符返回 INVALID_PARAMS
        r.topic("groups").published();                       // watchlist.groups；推整值快照
        r.topic("job").param("job_id").published();          // 带参数：每组参数一个值
        r.space(SpaceDecl::new("watchlist"));
    }

    async fn start(&self, cx: ExtensionContext) -> anyhow::Result<()> {
        let cfg: Config = cx.config()?;                                  // [ext.watchlist]
        let key = match &cfg.quote_api_key {
            Some(r) => Some(cx.credentials().get(&r.credential).map_err(anyhow::Error::msg)?),
            None => None,
        };
        let db = Db::open(cx.data_dir().join(&cfg.db))?;                // 数据目录是你自己的
        cx.provide(State { db, key });                                   // handler 里用 call.state::<State>()
        cx.topics().publish("groups", serde_json::json!({}), Vec::<Group>::new()).await?;
        Ok(())
    }
}

async fn list_groups(call: Call, p: ListParams) -> Result<Vec<Group>, RpcError> {
    call.state::<State>()?.db.groups(p.filter)
}
```

```rust
// app/src/main.rs
fn main() -> anyhow::Result<()> {
    harnix::App::builder()
        .name("helm", env!("CARGO_PKG_VERSION"))
        .extensions(harnix::first_party::standard())
        .extension(watchlist::Watchlist::default()).ungranted()   // 不要特权就 ungranted；要就 .grant([...])，见 §5.2
        .extension(trade::Trade::default()).ungranted()
        // .credentials(MyKeychain)       // 默认读环境变量
        .run()
}
```

### 5.2 `Extension` 的全貌，与能力

trait 的函数分两类（每个何时调、做什么见 §17.2）：

- **`manifest` 与 `register`** 只**声明**这个扩展有什么：各调一次，不该有副作用。扩展包的描述由 `packages` 照清单拼，
  原生扩展的描述就是 `register` 说出来的——同一个 `ExtensionDescriptor`、同一条宿主路径。
- **生命周期函数**（`migrate`、`start`、`stop`）做事。

**启动顺序**：注册（同时查授予）→ 按顺序 `migrate` → 建实例 → 按顺序 `start` → 开始服务。所以 `start` 里实例已经在，监听还没开。

**退出**：Harnix 自己接 SIGINT 与 SIGTERM，走同一条收尾，倒序调 `stop`。扩展不要在 `start` 里另装 SIGTERM 处理：两边都会收到信号，
扩展那份会抢在 `stop` 前面把进程带走。

**失败只影响你自己**：注册冲突、`migrate` 失败、`start` 失败，都只停用这个扩展和它的方法、路由、
space、实例，其余照常启动，原因写在诊断页。所以 `start` 里别吞错误：返回 `Err`，让诊断页说清楚。
已经交给 `main` 的引擎贡献摘不掉，留到下次启动，诊断写明。

**`ExtensionContext` 是扩展拿到的全部**：配置、数据目录、凭据、主题，以 `ext:<id>` 的身份经同一个准入点调方法，会话（§7）与实例（§6），
进程监听的地址（全表见 §17.5）。**没有**中枢、没有直连实例的 `EngineLink`、没有路由表与授予表：所有会话操作都经过中枢，这正是
「刷新不丢、多设备一致」对扩展也成立的原因。

**诊断**：`harnix.extensions`（每个扩展的信任、载体、版本、授予、行数、停用理由）与 `harnix.diagnostics`（每行的所有者、谁能调、范围、schema）
是排查扩展的第一处；profile 里没有对应扩展的 `[ext.<id>]`（拼错的节）也在那里。

**能力：特权是有名字的授予，没有「第一方」这一类，也没有按 id 给的特权。**

- 扩展在 `Manifest::requires([...])` 里写它**必须**有的能力，`optional([...])` 里写有了更好的（没给就按 `cx.grants().has(..)` 降级）。
  产品在装配时 `.extension(e).grant([caps::...])` 授予，或 `.ungranted()`。
- **启动时检查，违反就拒绝启动**并写明扩展与能力：`requires` 的每一项都要授予；授予的必须是内核目录（`harnix::caps`）或某个已链进的
  扩展声明过的 `native` 项，并且在 `requires` / `optional` 里——多给的特权是看不见的特权。
- 内核目录列在 §17.4；要新的能力，在 `Manifest::defines(CapabilitySpec::new("<id>.<名>", 披露))` 里声明，自己的行用 `Access::capability(..)` 要求它，
  授予照常在装配时写。例：IPDrun 的 `billing` 声明 `billing.charge`，只让持有它的扩展调某几个方法，不改 Harnix。
- 能力只在三处查：路由器（`Access::requires`）、`ExtensionContext` 的服务（调用时查，缺了答 `REFUSED`，`data.capability` 写明是哪项）、
  实例登记表的拥有者检查。
- **设备自己永远没有能力。** 设备调你的方法，你以自己的身份、用自己的能力去调内核或引擎；两次调用各判定一次，审计里是一棵树。
  `Manifest::disclosure` 写明你替设备做什么。

### 5.3 方法、主题、路由的规矩

- 方法名在 `<扩展 id>.*` 下，参数与返回带 schema。**名字的所有者是已登记的、与它整段前缀匹配的最长 id**：原生 `example` 与扩展包
  `example.showcase` 不能同时登记，冲突时原生优先。
- 主题推**整值快照**，不推增量。要按对象订阅就带参数：`trade.job { job_id }`。`.published()`：你推整值；`.computed(every, f)`：有人订时才算、变了才推。
- **一行对谁开放，写在 `Access` 上**（§17.4）。方法缺省谁都不能调。要记住两个写法：
  - `Access::device()`：**只对设备（与内核）**，不含扩展。「人经界面做的决定」都该是它：批准、授予、设置、配对、系统动作。别的扩展、
    扩展包、引擎都调不了它。
  - 扩展也要调的行，**明写 `Access::device().with_extensions()`**：`Access::extensions()` 是只有扩展，`Access::capability(c)` 是只有持有能力 c 的扩展，
    `Access::only(id)` 是只给某一个扩展。
  - 主题缺省设备与任何扩展可订。
  - 扩展包被收窄到它自己的 space，也够不着不对扩展包开放的行；你的行默认不对它们开放，除非你写了 `with_extensions()` 并且它在同意记录的 `calls` 里。
- 自有路由挂在 `/ext/<id>/` 下：`r.route(prefix, access, handler)`，用 `harnix::ext::http::axum::router(axum::Router)` 包一个 axum 路由。
  与界面同源，准入走同一套授权（带握手拿到的 `credential`：`Authorization: Bearer …` 或 `?credential=`；回环上也认
  进程令牌与启动链接的 cookie）；一个扩展的路由共用一种 `Access`。它**不享受**拓扑无关、`seq` 追赶这些保证，所以只在 Harnix 通道不合适
  时用（§9）。`r.static_dir(prefix, dir, access)` 出只读目录。

### 5.4 数据与迁移

Harnix 只管什么时候迁、失败了怎么办，不管用什么迁。数据库、迁移工具、连接池你自己选。你的数据放在
`cx.data_dir()`（`~/.<product_id>/ext/<id>/data/`）里，和 Harnix 的账本分开。`migrate` 迁的是**你自己的**数据，不是 Harnix 的。
没有 `--dry-run` 和备份钩子，要的话在你的 `migrate` 里自己做。

**目录的规矩**（维护者 2026-10-26 定）：

- 程序自己的配置与状态（令牌、设备、`kernel/spaces.json`、`kernel/instances.json`、实例数据根、各扩展的
  `data_dir`）放 `~/.<product_id>`，不配 `[brand]` 就是 `~/.harnix`；`HARNIX_STATE_DIR` 能挪。
- 项目根（选项目从这里开始、新项目建在这里）是 `cx.projects_dir()`：缺省 `~/Documents`，`--data-dir` / `HARNIX_DATA_DIR` 能改。
  Documents 里只放项目，不放隐藏目录。
- 项目里，项目自己的数据与配置一律放 `<项目>/.agix/`（引擎在项目里用的目录），不另开 `<项目>/.<你的产品>/`。

## 6. Agent 侧

### 6.1 给 `main` 加原生工具和场景

扩展的 `register` 里 `r.engine()` 拿到一个 `EngineContribution`，里面放要给 `main` 实例的东西。类型都是 Harnix 自己的词，从 `harnix::engine` 取，
**没有 Agix 的类型**，不用另加依赖：

| 要写什么 | 类型 |
|---|---|
| 原生工具 | `harnix::engine::Tool`（`spec()`、`permission()`、`call(ToolCx, Value)`）、`ToolSpec`、`ToolOutput`、`ToolError`、`Permission` |
| 给实例的每个会话提供工具 | `harnix::engine::ToolProvider`（`tools()`、可选 `hooks()`、可选 `changes()`，都不带参数：工具集是实例级的，不按会话定） |
| 一次工具调用之前的拦截 | `harnix::engine::ToolHook`：`HookDecision::{Proceed, Block, AddContext}`；`ToolCallFacts` 带 `name()`、`input()`、`session_id()`、`depth()`、`cwd()` |
| 场景 | `harnix::engine::SceneLayer`（建在 `SceneBase::builtin("harnix")` 之类的底上：`.prompt(..)`、`.tool(..)`、`.interactive(..)`、`.requires_project(..)`、`.auto_name(..)`） |
| 拦截人的消息 | `harnix::engine::PromptGate`：`PromptDecision::{Proceed, Block}` |

完整的例子是 `crates/testapp`。

```rust
fn register(&self, r: &mut Registrar<'_, ()>) {
    r.engine().scene(research_scene());
    r.engine().provider(Arc::new(QuoteTools::default()));
}
```

多个扩展的贡献由 Harnix 按装配顺序组合。`r.engine_everywhere()` 是给**每一台进程内实例**的：只许工具提供者；场景、门、钩子只属于 `main`。**引擎贡献是原生才有的**：扩展包不能给引擎贡献，只能经清单的 `[[tool]]`
与 `agent/` 装进产品的实例（见 [扩展包](harnix-ext-package.md)）。

没有 Agix 自己类型的逃生口：贡献只用上面这些词，引擎的变化由 Harnix 吸收。

这些贡献都在**本进程**里执行：引擎（进程内的或 spawn 的 daemon）经它的 daemon 链路向 Harnix 要工具的执行、钩子与门的裁决，所以两种载体上行为一致。
几条规则：

- **工具集是实例级的**：`ToolProvider::tools()` 与 `hooks()` 不带会话；实例的引擎连上时问一次，之后每次 `changes()` 响了再问一次。
  之后新建的会话有新的工具集，已有的会话保持原样。工具名要符合引擎的字母表（字母、数字、`_`、`-`），一个会话自己的工具至多 64 个。
- **`ToolHook` 在权限检查之前就看到调用**；`ToolCallFacts` 带发起调用的会话（`session_id()`，子 Agent 的调用是它自己的 id）、`depth()`（人开的会话为 0，子 Agent 逐层加 1）与 `cwd()`。
- **工具在哪干活**：`ToolCx::project()` 是会话的项目；`cwd()` 是发起调用的会话的工作目录——模型在 git worktree 里起的子 Agent 是那个 worktree
  （`<项目>/.agix/worktrees/<名>`），读写文件要照它，否则同 `project()`。`depth()` 与钩子的同义。
  `HookDecision::AddContext` 的文字在这次调用放行之后、模型的下一步之前，作为 system-reminder 交给模型。
- **宿主工具的权限**由链路按 `Tool::permission` 代答：`Allow` 与 `Deny` 直接生效，`Ask` 时把工具自己的问句（`Permission::Ask(text)`）附在转给界面的询问上，界面显示它。
- **场景层由中枢在建会话时展开**：会话建在层的基场景上，再按层补上 `prompt.append`、这个会话自己的宿主工具、`attended`、`auto_name` 与 `permission_rules`。
  `requires_project` 由中枢检查（没有项目的会话建不了）；`PromptText::cached` 保留但不起作用。
  不同场景层里**同名的工具必须是同一个对象**（同一个 `Arc`）：宿主工具按连接登记，一个名字只有一个执行者；要在几层里共用，只造一份。
- **会话的提示词**经 `harnix.session.setPrompt` 改（权限同 `setModel`）；`engine.session.setPrompt` 归中枢独占，因为中枢要让场景层的文字在前。
- **中枢拒收调用方给的 `host_tools`、`skill_dirs`、`attended`、`auto_name`、`prompt`、`prompt_types`**：这些由中枢按场景层与会话所在的 space 自己填，会话 `options` 里写了就报错；认不出的字段同样报错。

#### 拦截人的消息（`PromptGate`）

人的话进一轮之前，扩展可以按自己的规则放行或拒收；经外壳直接发给会话的（`harnix.session.send`）也算。放进 `r.engine().prompt_gate(Arc::new(Policy))`：

```rust
use harnix::engine::{PromptDecision, PromptFacts, PromptGate};

struct Policy;

#[harnix::async_trait]
impl PromptGate for Policy {
    async fn inspect(&self, msg: &PromptFacts) -> PromptDecision {
        if msg.content().contains("rm -rf") {
            PromptDecision::Block("这类话这里不收".into())
        } else {
            PromptDecision::Proceed
        }
    }
}
```

- 次序：设置层的 `UserPromptSubmit` hook 之后，再按装配顺序。
- `msg.delivery()` 说这句话是开一轮（`Turn`）还是送进正在跑的一轮（`Steer`，`on_busy: "steer"`）。
- 开一轮时拒收：这一轮不调模型就结束，`turn_complete` 的 `stop_reason` 是 `stopped_by_hook`、`message` 是你给的理由，
  `harnix.turn_settled` 的 `outcome` 里也有这两项。
- steer 时拒收：那句话不进对话，这一轮照常跑完；客户端收到事件 `user_message_refused { message_id, content, message }`
  （经 `harnix.event`）。没有扩展拒收时它不会出现。
- 外壳（`@harnix/core` 的会话状态）对两种拒收都在流里加一行说理由，steer 的那条从「待送达」里拿掉，不成一条人的话。
- 载体：进程内实例与 spawn 实例的门都跑在本进程（经 daemon 链路）；spawn 实例要同样生效，把它放进模板的 `contribute`（§6.2）。attach 的 daemon 是别人的，什么都不登记。

### 6.2 给 space 一台独立、隔离的实例

实例只能按模板建。**模板是你在 `register` 里声明的**（`r.template(..)`），扩展和界面都按它建，Harnix 记进账本、
重启后统一重建。

```rust
fn register(&self, r: &mut Registrar<'_, ()>) {
    // 进程内：能带原生贡献。适合「这个 space 要一台带交易工具的引擎」
    r.template(
        InstanceTemplate::in_process("research", EngineProfile::new().scene("research"))
            .contribute(|e| {
                e.scene(research_scene());
                e.provider(Arc::new(OrderTools::default()));
            }),
    );

    // daemon：崩溃与资源也隔离；要原生贡献也写 `.contribute(…)`（见下，工具与门仍在本进程执行）
    r.template(
        InstanceTemplate::spawn("sandbox", EngineProfile::new().scene("general"))
            .data_root("sandbox/{name}")
            .ui()                            // 允许界面按它建
            .max(4),
    );
}

async fn start(&self, cx: ExtensionContext) -> anyhow::Result<()> {
    // 模板的全名带扩展 id 前缀：`<扩展>.<模板>`。幂等：这个名字已经按这个模板建过就直接返回
    cx.instances().ensure("helm.research", "helm-research", BTreeMap::new()).await?;
    ...
}
```

- **谁能建**：扩展能按**自己的**模板和 profile 的模板建；界面只能按 `ui` 模板建，只能填模板放开的参数，多填一个就拒绝。
  所以别把危险的东西做成模板参数（§13）。
- **实例是持久的**：建过的记在账本里，重启后统一重建；模板没了或扩展停用了，它显示为 `down` 并写明原因。`stop` 才从账本删掉
  （数据根留着）。`ensure` 幂等，并发也只建一台，不用自己加锁。
- **spawn 实例 `ensure` 之后不是马上可用**：daemon 要零点几秒才起来，这期间的会话调用答 `INSTANCE_UNAVAILABLE`。
  接着就要用它的，用 `ensure_ready`；已经建好、正在重启或重连的，用 `wait_ready`。会话类调用本身不等。
- **原生贡献进程内与 spawn 一致**：模板上写 `.contribute(|e| { … })`。闭包在**本进程**里调一次，贡献经 daemon 链路登记给这台实例的引擎，
  所以闭包可以和扩展共享内存，`register` 也没有「要能在另一个进程里重走」的限制。attach 的 daemon 是别人的：模板上的贡献不登记，只记一条警告。
- **按模板建的实例有自己的数据根**，所以默认读不到「设置 → 模型」里配的 provider；要它们，模板上写 `.inherit_providers()`。
  会话起的进程要在实例停下时收干净，写 `.reap_on_stop()`。

每个构建方法的规则与缺省（名字的字符、`data_root` 的写法、继承 provider 与收进程的细节、找不到模板时 daemon 怎么退出）见 §17.3。

让一个 space 默认用它：space 声明里写 `.instance("template:research")`（第一次在这个 space 里建会话
时按模板建，之后复用），或者写一台固定实例的 id。

**选哪种载体**：

| 要什么 | 用 |
|---|---|
| 原生 Rust 工具、`ToolProvider` | 进程内 |
| 和 `main` 不同的 MCP、凭据、插件 | 两种都行（进程内的几台互相隔离） |
| 崩溃、内存与 Harnix 隔开 | daemon |

一台 daemon 只服务一个 Harnix；不要让别的客户端也连它。spawn 的 daemon 由 Harnix 看护：挂了就重新拉起，
实例在这期间是 `degraded`，正在看的会话在它回来后 resync；Harnix 退出（哪怕被 `kill -9`）它也跟着退出。
Harnix 被杀后马上在同一目录重启时，新 daemon 等旧的那个收完尾、放开这台实例的数据根锁才建引擎，两个引擎不会同时写一份转录。
attach 的 daemon 握手时比对协议版本，不一致就 `down`，原因里写明两边的版本。

**只给某个 space 用的实例（有归属的实例）有一条规则**：为某个 space 建的实例归那个 space；点名它的调用（建会话、`harnix.session.move`、
全部 `engine.*` 直通）只对它的 space、跨 space 的调用与内核开放，其余答 `SPACE_MISMATCH`，在唤醒之前判。不归任何 space 的实例
（`main`、profile 的实例、扩展经 `instances().create` 建的）照旧谁都能点名。原生扩展用归 space 的实例，要经 `cx.sessions().in_space(space)` 声称那个 space（§7.1）。

### 6.3 Agix 插件（运行期包）

Agent 侧的第三方扩展可以是一个 Agix 插件，装到实例里（QuickJS 脚本、skill、agent 类型），能力与披露都在引擎那边；管理它的是第一方扩展 `plugins`。
Harnix 只读包里的 `[[ui]]` 段：贡献点 API 2 的界面模块是预编译好的 ES 模块，从页面的 import map 拿 `react` 和
`@harnix/react`（§22.6，范例 `tests/fixtures/packages/demo-rows-v2/`）。Agix 插件**不能**新增 space，也不能往别的
space 加页签；`[ext.plugins] ui_modules = false` 一条都不加载。

要的是后端逻辑、自己的方法与 space、隔离的界面，那是扩展包，见 [扩展包](harnix-ext-package.md)。

## 7. 会话

### 7.1 两类调用方，一套语义

界面（客户端）和你的扩展（服务端）都能驱动会话，排队、中断、结算、广播完全一样。无头会话照样出现在
它所属 space 的会话列表里，界面随时可以 attach 旁观。

```rust
let s = cx.sessions().create(
    SessionSpec::new()
        .space("watchlist")
        .instance("helm-research")        // 省略就是 space 的默认实例（space 不声明时是 main）
        .scene("research")
        .project_root(root)
        .provider("anthropic")
        .model(&model)
        .permission_mode("default"),
).await?;

cx.sessions().send(&s, "总结今天的异动", OnBusy::Queue).await?;

let mut tap = cx.sessions().tap(&s).await?;
while let Some(item) = tap.next().await {
    match item {
        TapEvent::Frame(frame) => handle(frame),          // 和客户端收到的帧一样：{method, params}
        TapEvent::Lagged { last_seq } => {                // 你读慢了
            let attached = tap.catch_up().await?;         // 缓冲里还有的补回来
            // 已经落进 transcript 的那部分按 attached["history_total"] 用 engine.session.history 读
        }
    }
}
```

- **会话忙时这条话怎么办**由 `OnBusy` 定：排队（默认）、拒绝、中断本轮再跑、或送进正在跑的一轮（steer）。和人在输入框里的选择是同一组。
- **tap 会落后**：它是有界通道，中枢不会为了等你而卡住会话。落后了先收到 `Lagged`，再 `catch_up()`：和刷新页面的客户端走同一条路，
  所以扩展不需要另一套追赶逻辑。
- **只要一轮怎么结束的**，用 `tap.next_settled()`：它只等 `harnix.turn_settled`，落后也不丢，不必 `catch_up`。结果里有停止原因、用量、
  回答文本与调过的工具；全文读 `engine.session.history`。
- 实例重连期间的发送进队列，实例回来、会话 resync 之后照常跑。会话的全名是 `(instance, session)`，任何地方都带着实例。
- **space 的声称**：`cx.sessions().in_space(space)` 返回的句柄，经它的每个调用都声称那个 space；用归 space 的实例时必须这样（§6.2）。
  一个会话在哪个 space 里建，就永远属于它。

每个方法、`OnBusy` 的各值、附件、容量与结算帧的细节见 §17.6。

### 7.2 往对话里写卡片：扩展条目

```rust
cx.sessions().append_entry(&s, Entry::new("alert", json!({ "symbol": "NVDA" }))).await?;
let mine = cx.sessions().entries(&s).await?;   // 本扩展写过的，最早的在前
```

条目进会话日志（`harnix.session.entries.append`），刷新、重放、整段分叉都看得到；**只给人看，
不给模型看**。命名空间由中枢定成**你的扩展 id**，扩展写不了别人的，设备一条也写不了（人说的话
走 `harnix.session.send`）。写的时候正在看这个会话的人立刻收到。

前端里它是一块 `kind: "custom"`、`type: "entry"` 的块，`data` 是 `{ id, ns, event, payload, at }`
（`at` 是它前面的消息数）。用 `flow.block` 认领自己的：

```tsx
r.register('flow.block', {
  id: 'watchlist.alert',
  match: (b) => b.kind === 'custom' && b.type === 'entry' && b.data?.ns === 'watchlist' && b.data.event === 'alert',
  render: (b) => <AlertCard entry={b.data} />,
}, 'watchlist');
```

没人认领的条目不画。

### 7.3 对回答的反馈

人在对话里对一轮回答点赞或踩（可选类别、写一句备注），是中枢记的条目，不是哪个扩展的：

```text
harnix.session.feedback {instance?, session_id, space?, turn_id, call_id?, rating: "up" | "down" | null, category?, note?}
  → {feedback, entry}
条目 harnix.feedback / rating，载荷 {turn_id, call_id?, rating, category?, note?, at, by}
```

- **只设备调**（`Access::device()`，`RowScope::Session`）：这是人的判断，和换模型一样；按调用方声称的 space 查它看不看得到这个会话。
  扩展不替人评。
- `category` 是 `inaccurate`、`unhelpful`、`unsafe`、`slow`、`other` 之一；`note` 至多 4000 字，去掉首尾空白，空的不记。`rating: null` 是撤回，
  这时类别与备注不记。`at` 是毫秒时间戳，`by` 是 `device:<id>`。
- **改、撤都是再记一条**：同一轮（与同一个 `call_id`）最后一条为准，之前的留在日志里。读法只有一种：Rust 的
  `harnix_contract::wire::current_feedback`，界面的 `@harnix/core` `standingFeedback`。
- 它是条目，所以和 §7.2 一样：进日志、刷新、重放、整段分叉都在，模型看不见；导出带上（`harnix-app.sessions.export` 的三种格式，见
  [Harnix app](harnix-app.md) §2）。正在看这个会话的设备经 `extension_entry` 立即收到，界面据此更新按钮。
- `turn_id` 就是这一轮的帧、transcript 消息和录像里调用的 `turn_id`；`call_id` 留给只评某一次调用的调用方，中枢不解释它，对话里的按钮不带。
  轨迹在调用上标反馈时：同一会话里 `turn_id` 相同、不属子 Agent 的那一轮的最后一次调用，就是被评的那个回答；带 `call_id` 的标在调用方指的那一次上。

**隐私：缺省不上传。** Harnix 只把反馈记进本机的会话日志，不往任何地方送。每记一条，中枢在主题 `harnix.feedback`（无参数）上推一次
`{instance, session_id, space, entry_id, feedback}`（整值是最近的一条），**只有被授予内核能力 `feedback.read` 的原生扩展能订**，设备永远订不到；
标准装配里谁都没有这项能力。要收集反馈（汇总、送到自己的服务）的应用，自己写一个扩展、在装配里授予它 `feedback.read`，并在它的披露里说清楚送到哪里；
送不送、送什么由应用定。主题只是通知，漏了的以会话日志为准（`engine.session.entries {namespaces: ["harnix.feedback"]}`）。

## 8. 前端

### 8.1 包

| 包 | 你用不用 |
|---|---|
| `@harnix/react` | **用**。贡献注册、`useHost()`、i18n、令牌、图标、对话组件 |
| `@harnix/core` | 一般不直接用；要写不依赖 React 的逻辑时用它的 `/state`、`/client` |
| `@harnix/shell` | 不 import。它是外壳核心，构建脚本把你的 `register()` 接进去 |
| `@harnix/testkit` | 写测试时用 |

你的 space 界面包只依赖 `@harnix/react`（和 React 本身）。第一方扩展各有自己的 UI 包（`@harnix/system`、`@harnix/harnix-app`……），
由构建编进外壳、按 `harnix.hello` 里启用的扩展激活；你的界面入口排在它们之后，总是调。

### 8.2 注册一个 space

**一个 space 就是三栏，三块都能定制，合起来才是一个 space**：

| 部分 | 字段 | 不写时 |
|---|---|---|
| 左栏中段 | `sidebar` | 这个 space 的会话列表 |
| 中区 | `center.mode`：`chat`（默认的对话，多数场景够用）、`minimized`（`overview()` + 一行问句）、`hidden`（只有 `overview()`，没有对话） | `chat` |
| 右栏 | `details.tabs`：数组，或随选中项变的函数 | 只有详情页 |

不随 space 变的只有外壳的固定部分：顶部的标识、新建对话、搜索，底部的头像（设置入口，外圈表示连接）。
Harnix 自带的 space——会话、项目、系统（后端有流程扩展时流程在它里面）——都是用例，你的产品用
列哪些扩展决定留哪些；Harnix 本身做薄，产品面都是你的 space。

```tsx
// ui/src/index.tsx —— 构建时由 virtual:harnix-modules 接进外壳（examples/hello-app 是完整的例子）
import type { HostApi, Registry } from '@harnix/react';
import { QuotesTab } from './QuotesTab';
import { Summary } from './Summary';

export function register(r: Registry, host: HostApi) {
  r.register('space', {
    id: 'watchlist',
    icon: 'globe',                              // 外壳图标集里的名字
    title: () => host.t('watchlist.title'),     // 文案走 i18n，不写死
    order: 20,
    center: { mode: 'chat' },                   // chat | minimized（总览 + 问一句）| hidden（只有总览）
    details: { tabs: [{ id: 'quotes', title: () => host.t('watchlist.tab.quotes'), render: () => <QuotesTab /> }] },
    conversation: {
      subtitle: () => host.t('watchlist.subtitle'),
      suggestions: () => [{ label: '今天的异动' }, { label: '给我一份周报' }],
      notice: () => 'Agent 只提案，下单要人点头',
    },
  }, 'watchlist');
  r.register('home.card', { id: 'watchlist.summary', title: () => 'Watchlist', render: () => <Summary /> }, 'watchlist');
}
```

入口可以带 `export const extension = "watchlist"`：它是哪个后端扩展的界面。写了，外壳只在那个扩展启用时才调它的 `register`，
扩展停用时它的注册按 owner 整批撤下；不写（应用自己的入口）总是调。

后端在扩展里声明同一个 space 的会话策略（不声明就是默认：`main` 实例、实例的默认场景）：

```rust
r.space(SpaceDecl::new("watchlist").instance("template:helm.research").scene("research"));
```

会话在哪个 space 里建，就永远属于它（账本 `kernel/spaces.json`，中枢读写）。两个界面 space 可以列同一个后端 space 的
会话：项目 space 就是写了 `sessions: 'sessions'` 的会话 space 的另一种列法。外壳每个会话调用都带上前台 space，
中枢核对，不符就 `SPACE_MISMATCH`。要用别的 space 的会话，在输入框里 `@session:` 选它：发出去的是
它说过的话的一份副本。内核不认识任何 space id：`sessions`、`projects`、`system`、`flow` 都是各自的扩展声明的。

构建：模板的 `scripts/build.sh` 把 `spaces/*/ui/src/index.tsx` 自动接进去；手工怎么构建、去重了哪些包见 §22.5。

`space` 只是十一个内核贡献点之一：往工具行、对话流、右栏详情、右栏页签、左栏分组、命令菜单、设置、会话动作、快捷键里加东西的是另外十个，
你的扩展还能自己定义点（`<你的 id>.<名>`）。每个点的形状见 §22.3 与 [贡献点](contribution_points.md)。

### 8.3 `useHost()`

```tsx
import { useEffect, useState } from 'react';
import { useHost } from '@harnix/react';

export function QuotesTab() {
  const host = useHost();
  const [groups, setGroups] = useState<Group[]>([]);

  useEffect(() => host.subscribe('watchlist.groups', undefined, setGroups), [host]);

  const ask = async (g: Group) => {
    const s = await host.session.create({ title: g.name });   // 自动绑定前台 space
    host.session.open(s);
    host.composer.insert(`分析 ${g.name} 里的标的`, { session: s });   // 进 s 的输入框，挂上了才插
  };
  ...
}
```

全部成员见 §22.4。

**你拿不到 Harnix 自己的连接**，所以同一份代码在任何拓扑下都能跑。`call` 只放行 `harnix.reachable` 里
列出的方法，界面上的人调不了的，你的组件也调不了。**没有全局的「当前会话」**：每个
space 记自己的当前会话，`session.current()` 拿到的永远是前台 space 的。

### 8.4 对话组件

中区的对话由外壳画，你一般不用自己画。要在自己的页签里放一个对话（例如一个工位的转录），用
`@harnix/react` 的 `ChatPanel`，传 `(instance, session)`。产品特有的东西（语音、快捷回复……）在你的应用
里包一层，不要往 `ChatPanel` 里加。

### 8.5 规矩

界面的规矩为的是两件事：换主题、换语言时你的部件不坏（所以颜色只用令牌、文案只用 key，静态检查会拦）；页面的 CSP 不为谁放宽
（所以没有 `eval`、远程 import、CDN、内联脚本与样式）。测试照 `data-testid` 与 `data-state` 找元素。条文见 §22.5。

### 8.6 外观：品牌、配色、文案

应用不 fork `ui/` 就能有自己的样子：名字、图标与程序目录写在 profile 的 `[brand]`（§18）；配色、文案与图标在界面入口里给：

```tsx
// 你的界面入口 register() 里
r.tokens({ '--accent': '#2f6fb3', '--accent-soft': 'rgba(47,111,179,.12)', '--accent-line': 'rgba(47,111,179,.4)' }, 'hello-app');
r.strings('en', { 'hello-app.greeter': 'Greeter' }, 'hello-app');
r.strings('zh-CN', { 'hello-app.greeter': '问候' }, 'hello-app');
// 之后 host.t('hello-app.greeter')
```

令牌只覆盖主题已有的名字，所以你的配色在暗色主题与外壳的每个部件上都还成立；不另造颜色。图标画在外壳的网格与描边上，看起来是一套。
三者各收什么、拒什么见 §22.4.1；`examples/hello-app` 是完整的例子。

## 9. 前后端之间用哪条通道

| 需求 | 用 |
|---|---|
| 一问一答 | `call('<id>.x')` |
| 状态更新 | 宿主主题，整值快照 |
| 长任务进度 | 方法返回任务 id，进度推到带参数的主题 `<id>.job { job_id }` |
| 大文件 | 大负载旁路（`cx.bulk().grant(..)` 给一次性上传凭据，`harnix.bulk.upload`） |
| 你自己的流式协议、第三方库要的 HTTP 形状 | 自有链接 `/ext/<id>/`（没有 `seq` 追赶，断线你自己处理） |

能用前四种就别用第五种。

不是页面、而是应用自己的 Rust 程序（命令行、集成测试、桌面壳）要连运行中的进程：用 `harnix-client`（§27），
握手、`/rpc`、事件流、启动链接都在里面，不链服务端，协议版本随它编进来。

## 10. 常见做法

### 10.1 批准卡：这个动作必须由人点头

引擎的权限管「这个工具能不能跑」；「这个业务动作要不要人确认」归你。用公开契约就能做，
`examples/hello-app` 的 greeter 是一份能跑的范例（没有改 Harnix 的任何 crate）：

1. **请求**：你的代码要做那件事时，建一张票（id、会话、标题、状态、截止时间），存在你的扩展里。
   范例里是 `greeter.request_approval`。
2. **记录**：`append_entry(&session, Entry::new("approval", json!(票)))`，卡片进时间线（§7.2）。
3. **推送**：带参数的主题 `<id>.approvals { instance, session_id }`，推这个会话所有票的当前状态。
   卡片订阅它，所以几台设备看到的是同一个状态。
4. **回答**：方法 `<id>.approve { ticket, decision }`，**`Access::device()`——只对设备开**：引擎、
   别的扩展、连你自己的扩展都调不了（范例的 `answer_for_a_person` 演示了这个拒绝）。**不要写 `.with_extensions()`**：
   那会让扩展替人答。票的状态只从「待批」转一次，先答者生效，之后的回答拒绝。
5. **超时**：建票时起一个任务，到截止时间还是「待批」就按拒绝处理，`by` 记成 `timeout`。
6. **审计**：每个决定再写一条扩展条目 `approval_decided`：哪张票、结果、谁（`Call::device()`）。
7. **画卡**：`flow.block` 认领 `ns === "<id>"` 且 `event === "approval"` 的条目，按钮调 `approve`。

票存在内存里，进程重启就没了；要跨重启，把票写进你扩展的数据目录（`ExtensionContext::data_dir()`）。

**要知道它防不住什么**：「只许设备」挡得住引擎、扩展、内核替人回答，挡不住装进来的第三方界面
（受信发布者的进页面模块和页面同级；其余扩展包的 iframe 也被服务端按同意的 `calls` 收窄，够不着这一行）。
交易类产品请在 profile 里设 `[ui] runtime_modules = false`。

### 10.2 长任务进度

方法 `<id>.start_job` 返回 `job_id`；进度推到 `<id>.job { job_id }`，每次推完整状态
（进度、阶段、结果或错误）。界面订阅这个主题。完成时想在对话里留一张卡，就写一条扩展条目。

**先推一次初始状态再把 `job_id` 交出去**：主题是整值快照，订阅得再晚也先拿到当前值，于是不会漏掉
开头。`examples/hello-app` 的 `count_to` 就是这样写的：它的进度条订阅 `greeter.job`，跑完往
发起它的那个会话里写一条 `job_done`，由 `flow.block` 画成卡片。

### 10.3 一个 space 要一台自己的 Agent

见 §6.2。要原生工具用进程内模板，要配置完全独立用 daemon 模板；space 声明里写
`.instance("template:<id>")`。

### 10.4 给外部 Agent 开 MCP

Harnix 不提供 MCP 服务端和角色令牌。
在那之前：在你的扩展 `start()` 里起**你自己的监听**（单独的端口），或挂一条自己的路由，用你自己的令牌解析。
它不在 Harnix 的授权、审计与保证之内，安全由你负责。

## 11. 构建、版本、升级

- `scripts/build.sh`：`pnpm build` → `cargo build`。没有界面产物直接 `cargo build` 会明确失败；
  `--headless` 可以不要界面。
- `<app> --version` 打印应用、Harnix、引擎、扩展 API、贡献点 API、协议的版本（排查问题时贴进 issue）。
- **一个钉**：`harnix/` 钉一个 Harnix 的标签（`Harnix-v<x.y.z>-<日期><序号>`），它带的东西随它一起钉死，所以引擎的版本从来不是
  你要选的。`check-pins` 查这个钉是真的（各脚本查什么见 §25）。
- **升级 Harnix**：bump `harnix/` 子仓 → `git submodule update --init --recursive` → `sync-app-deps` →
  `check-pins`、`check-lock` → 跑测试。从 0.4.4 起扩展接口与线上协议只做加法，升 z 不该打破你。
- **从 0.4.2 及更早迁过来**：见 Harnix 仓库本地的 `docs/migrations/0.4.3.md`（升到 0.4.4 的是 `0.4.4.md`，维护者给）和 §14。
- **公共改动**：直接在你的 `harnix/` 子仓里开分支 → 回推 Harnix → 合并、打标签 → 你 bump 子仓。
  不许有长期补丁：子仓指针只能指向上游存在的提交。
- **桌面壳**：复制 `harnix/templates/desktop`，改名字、图标，把你的二进制打成 sidecar。本机模式
  壳起你的应用、用启动链接让窗口进来、关窗口时停掉它；远程模式窗口开一个 `https://` 的 Harnix。业务流量
  全是页面自己的 HTTP/WS，壳不加 IPC（`check-no-ipc.mjs` 查）；唯一的例外是 macOS 无标题栏时页面请求拖动窗口。
  详见它的 README。

## 12. 测试

- 功能测试对着**真的二进制**跑：API 层（不开浏览器）和 Playwright 层，都用 `@harnix/testkit` 起进程。
- 模型默认走录像回放（确定、离线、不花钱）；`--live` 连真实模型并可以写回录像。
- 每个套件有自己的 HOME、数据根、端口，互不串。
- 用例打标签 `@feature:<功能>`、`@layer:api|ui`，覆盖表从标签生成。
- 应用自己的 vitest 组件测试：`link:` 进来的 `@harnix/react` 会从 `harnix/` 那边解析 `react`，测试里就有两份 React、hook 报错。
  在应用的 vitest 配置里写 `resolve: { dedupe: ["react", "react-dom"] }`——和外壳构建时 Vite 的去重是同一件事，方向相反。
- 收尾用 `await backend.dispose()`（等进程退出再删目录），不要用 `stop()`。
- Rust 一侧：`harnix::testing`（feature `testing`，**不稳定**）能不起监听地起一个进程（`testing::Kernel`：真的路由器与中枢，引擎走回放），
  或不起进程列出路由表（`testing::describe`）。
- testkit 的导出见 §24；怎么跑、怎么录像、覆盖表怎么读见 [测试](testing.md)。

```ts
import { test, expect } from '@harnix/testkit';

test('关注组列表 @feature:watchlist @layer:api', async ({ api }) => {
  const groups = await api.call('watchlist.list', {});
  expect(groups).toEqual([]);
});
```

## 13. 安全：二开要知道的

| 约束 | 是安全边界吗 | 你要注意什么 |
|---|---|---|
| 方法、路由默认拒绝，按 `Access` 放行 | 是 | 每个方法都想清楚给谁用：`device()` 只给人，`with_extensions()` 才让别的扩展调 |
| 能力在装配时显式授予 | 是 | 多给的特权是看不见的特权，所以授予必须在 `requires` / `optional` 里 |
| 界面只能按 `ui` 模板建实例 | 是 | 别把危险参数做成模板参数 |
| `SPACE_MISMATCH`（会话与 space 对不上就拒绝） | **不是**，只防 bug | 别拿它当权限：任何同页代码都能声称任何 space；真正的边界是 `engine.inspect`、`engine.control`、`instances.admin` |
| 「只许设备」 | 只对后端成立 | 挡不住受信发布者的进页面模块，见 §10.1 |
| 原生扩展是出身即信任 | — | 你的扩展能做的事就是这个进程能做的事；第三方代码不要做成原生扩展，做成扩展包（[扩展包](harnix-ext-package.md)），它在 WASM 沙箱里、按同意的能力行事 |
| 系统 space | 不链它或 `[extensions] off` 时方法一起撤 | 产品里一般关掉；它能看到完整的系统提示词 |
| 密钥 | — | 只在 profile 里写引用，值走 `Credentials`（§17.5） |

## 14. 迁移现有代码

### 14.0 迁移分析怎么做

在自己的仓库里做分析时，输入是：本文（尤其 §26 的缺口表）、能跑
的范例 `examples/hello-app`。**以你仓库里现在的代码为准**，对不上就按代码改。

逐项过一遍，每项落成「保留 / 改写 / 删掉 / 等 Harnix」：

1. **后端**：每个领域服务 → 一个 `Extension`；HTTP/WS 接口 → `r.method(..)` / `r.topic(..)`，确实要自己协议的
   → `r.route(..)`；后台任务 → `start()` 里起，状态用主题推；配置 → `[ext.<id>]`，密钥 → `CredentialRef`。
2. **Agent 侧**：原生工具、场景 → `r.engine()` 或进程内模板；要隔离的 → spawn 模板；无头会话、tap、
   `OnBusy` → `cx.sessions()`（tap 要处理 `Lagged`）。
3. **界面**：每个产品面 → 一个 space（`register()`）；页面里的卡片 → 条目 + `flow.block`；
   工具结果的特殊画法 → `tool.row`；详情 → `details.panel`；设置页 → `settings.section`。Vue / 自写
   组件全部重写成 React，状态用组件内 state 或 zustand（自己依赖）。
4. **数据与目录**：现有数据在哪、现在放哪（§5.4）；要不要写 `migrate`。
5. **测试**：换成 `@harnix/testkit`（API 层 + Playwright 层），用例打 `@feature` / `@layer` 标签；
   录像放在应用自己的目录。
6. **缺口**：用到的能力在 §26 里的，标出等什么，或者先在应用里怎么顶着。
7. **回推 Harnix 的候选**：两个应用都写了、形状一样、在公开契约之上做不出来的，列出来（§2）。

建议的顺序：先换布局和后端，再换界面（space、品牌），再换测试，再上桌面壳，最后清理。

### 14.1 通用步骤

1. 换布局：只留 `harnix/` 一个子仓（`git submodule update --init --recursive`）；workspace 收 `harnix/crates/*`、`harnix/crates/ext/*` 与 `harnix/crates/carrier/*`；
   `sync-app-deps`、`check-pins`。代码里的引擎类型改从 `harnix::engine` 取（§6.1）。
2. 后端改成扩展：一个 space 的后端一个 `Extension`；领域方法挪到 `register` 里的 `r.method(..)`；授予写在 `main.rs`。
3. 前端换成 React 包：每个 space 一个 `ui/` 包，`register()` 注册。
4. 测试换成 `@harnix/testkit`。
5. 删掉 vendor 副本和照抄的代码，`check-pins` 通过，没有补丁。

每个要应用动手的版本另有一份迁移说明（在 Harnix 仓库的 `docs/migrations/`，只在维护者本地）：从哪一版升到哪一版要改什么。

---

# 第二部分：接口清单

## 15. 这一部分怎么读

这里列的是已经落地的接口。写法：签名照抄代码，默认值照抄代码。代码和这里对不上时以代码为准，并且这里是错的，要改。
扩展包自己的接口（清单、WIT、`packages.*`、`[ext.packages]`、桥）在 [扩展包](harnix-ext-package.md) 的第二部分；这里只列原生扩展用到的，
以及两边共用的部分。线上方法的权威清单是 `crates/harnix/tests/method-table.snap.json`（每一行的所有者、谁能调、范围）与运行时的 `harnix.reachable`。

## 16. 版本号

| 名字 | 现在 | 在哪 | 什么时候升 |
|---|---|---|---|
| Harnix 版本 | `0.4.4-2026111502` | 各 crate 的 `Cargo.toml`；`harnix::HARNIX_VERSION` | 每次发布（`scripts/set-version.sh` 连这一格一起改，`tests/crate-metadata.mjs` 查它和 crate 一致） |
| 扩展 API | `1` | `harnix::ext::EXT_API_VERSION` | 写进文档、`--version` 打印的数，不做编译期检查：兼容靠 crate 的版本，从 0.4.4 起扩展接口只做加法（公开的结构与枚举都是 `non_exhaustive` 或只有私有字段，一个 feature 只加函数与模块）；破坏性的变化升它，也升 crate 的大版本。`harnix::ext::http::axum` 与 `harnix::axum`（feature `axum`）、`harnix::testing`（feature `testing`）不在承诺内 |
| 贡献点 API | `4` | `harnix::CONTRIB_API_VERSION`；`harnix.hello` 里也有 | 贡献点的契约变了。2：运行期包的界面模块是预编译的 ES 模块，从页面的 import map 拿 React；**React 的大版本是它的一部分**。3：宿主 API `HostApi`/`useHost()`、第九个内核点 `session.action`、扩展自己定义的点（`definePoint`）、`space` 分组、按 `hello.extensions` 激活。4：第十个内核点 `sidebar.tab`（右栏的页签类型）、宿主 API `host.sidebar`（导航控制器）、页签域 `useTabInfo()`；第十一个内核点 `shortcut`（有快捷键的命令，§22.8）；只做加法，照 3 写的模块照常 |
| 协议 | `5` | `harnix_contract::PROTOCOL_VERSION` | 握手比对，不一致拒绝（`HANDSHAKE_REFUSED`）。5：名字规整（内核 `harnix.<区域>.<动词>`、引擎直通 `engine.*`、扩展在自己的 id 下），默认实例叫 `main`，新增错误码 `CANCELLED`、`DEADLINE_EXCEEDED`、`EXTENSION_UNAVAILABLE`。`harnix.event` 帧里的事件负载与 `engine.*` 的答是 Agix 的形状，不在加法承诺里，由 `hello.engine_protocol` 标明 |
| 扩展包接口 | `2` | `harnix-ext.toml` 的 `[app] harnix_api`；`harnix_package::HARNIX_API` | 扩展包的契约（WIT world 与清单）有破坏性变化。见 [扩展包](harnix-ext-package.md) |
| 扩展线 | `1` | `harnix_wasm_port::EXT_WIRE_VERSION` | 产品与它自己的扩展进程之间那条线（`harnix-ext-wire`）的帧或方法改了形状。两端是同一次构建，扩展进程的 `proc.hello` 报的数不等就被拒绝；应用不直接用它 |

`<app> --version` 打印上面全部，加上引擎的版本。从 0.4.4 起线上协议与扩展接口只做加法；契约里露出的 Agix 类型除外。
这张表的六个数由 `tests/crate-metadata.mjs` 对着代码里的常量查。

稳定的层次：S1（扩展接口：`harnix-ext`、`harnix-engine-api`、`harnix_contract::s1`，三者的**全部**公开项）与 S2（线上协议、扩展线、扩展包清单与 WIT、桥）
是长期稳定的；S3（`harnix-kernel`、`harnix-engine-link`、`harnix-host-api`、各骨架 crate 的内部接口）随骨架变，扩展作者不该用。
S1 与 S3 的界线是依赖图：`harnix-ext`、`harnix` 门面、`harnix-host-api` 不依赖 `harnix-engine-link`，写不出它的名字。层次规则由 `crates/harnix/tests/layering.rs` 守着。

---

## 17. Rust：`harnix` crate

应用只依赖 `harnix` 一个 crate（它重导出装配、扩展接口、引擎词汇与第一方扩展）。扩展源码一般只要
`use harnix::prelude::*;`。

```text
harnix::{App, AppBuilder, ExtensionSlot, ExtensionStatus, HARNIX_VERSION, SHUTDOWN_GRACE,
         caps, codes, Capability, CapabilitySpec, ExtId, RpcError, Subject, MAIN_INSTANCE,
         Access, Attachment, Call, CredentialRef, Credentials, Entry, EnvCredentials,
         Extension, ExtensionContext, ExtensionSet, Manifest, MigrateContext, MigrationReport,
         OnBusy, Registrar, RowScope, Secret, SendOptions, SessionRef, SessionSpec, SpaceDecl, Tap, TapEvent,
         CONTRIB_API_VERSION, EXT_API_VERSION, SETTLED_HELD, TAP_CAPACITY,
         ext, engine, first_party, testing（feature）, async_trait, axum（feature）}
```

`harnix::ext` 是扩展接口的整个模块（`harnix-ext`）；`harnix::engine` 是扩展给引擎的东西与读引擎的读者，即 `harnix-engine-api` 的**全部**公开项，
与 `harnix::ext::engine` 是同一张清单。门面只依赖 `harnix-assembly` 与 `harnix-ext`：根部的每一项都是这两个 crate 转出的，
引擎链接、引擎操作、引擎事件（`harnix-engine-link`，S3）不在其中，应用拿不到也不该拿。

### 17.1 `App::builder()` → `AppBuilder`

| 方法 | 说明 |
|---|---|
| `.name(name, version)` | 应用名与版本：`--version`、`--help` 用它。界面上的产品名、图标、产品 id 由 profile 的 `[brand]` 给 |
| `.extensions(set)` | 加一组扩展（`ExtensionSet`），各带自己的授予。`harnix::first_party::standard()`：`harnix-app`、`system`、`observability`、`settings`、`plugins`、`devices`，各自带 Harnix 给它们的授予；`standard_with(packages)`、`standard_with_flow(flow)` 再加 `packages`、`flow` |
| `.extension(e)` | 加一个扩展，返回 `ExtensionSlot`：`.grant([caps::…])` 或 `.ungranted()`。**顺序就是注册、`migrate`、`start` 的顺序**，也是引擎贡献的组合顺序 |
| `.credentials(source)` | 换掉 `Credentials` 的实现（默认 `EnvCredentials`：`helm/quote-api` → `$HARNIX_CREDENTIAL_HELM_QUOTE_API`） |
| `.profile_from_args()` / `.args(iter)` | 命令行从哪来；测试里用 `.args` |
| `.child_entry(name, entry)` | 登记一个子命令入口（`harnix::App::builder()` 在 feature `proc` 时已登记 `ext-proc`，产品不用写） |
| `.default_engine(EngineProfile)` | 命令行没给 `--profile` 时用的 `[engine]`：产品自己的缺省，代替 `EngineProfile::default()`。给了 profile 就照 profile；`--scene`、`--scenes`、`--model` 照旧盖在两者上面。`harnix` 二进制用它把 `code` 列进可选场景（`scenes = ["code"]`，缺省场景仍是 `harnix`） |
| `.run()` | 跑起来，`anyhow::Result<()>` |

`harnix::first_party` 里还有每个第一方扩展的类型（`System`、`HarnixApp`、`Settings`、`Plugins`、`Observability`、`Devices`，以及按 feature 的 `Flow`、`Packages`）
与它们的授予常量（`PACKAGES_GRANTS`、`FLOW_GRANTS`）：产品要自己挑几个、或调整配置时，各用它的类型。

### 17.2 `trait Extension`

```rust
#[harnix::async_trait]
pub trait Extension: Send + Sync + 'static {
    type Config: DeserializeOwned + Default + Send + Sync + 'static;   // [ext.<id>]；没有写 ()
    fn manifest(&self) -> Manifest;                                    // 必须实现
    fn register(&self, r: &mut Registrar<'_, Self::Config>);           // 必须实现
    async fn migrate(&self, _: &MigrateContext) -> anyhow::Result<MigrationReport> { Ok(MigrationReport::none()) }
    async fn start(&self, _: ExtensionContext) -> anyhow::Result<()> { Ok(()) }
    async fn stop(&self) {}
}
```

| 函数 | 什么时候调 | 干什么 |
|---|---|---|
| `manifest` | 注册 | id、版本、标题、披露文本、要的能力 `requires`、可选的 `optional`、声明的能力 `defines`（§17.3） |
| `register` | 注册 | 它提供的一切：方法、主题、space、工具规格、路由、界面、引擎贡献、实例模板（§17.3）。先按 `Config` 读 `[ext.<id>]`，读不了只停用这个扩展 |
| `migrate` | 启动，`start` 之前 | 你自己的数据迁移 |
| `start` | 启动，实例建好之后、监听之前 | 拿 `ExtensionContext`，起你的服务 |
| `stop` | 退出或停用，倒序 | 收尾。调到它时监听已经停了、连接都已关掉，不会再有调用进来 |

`ErasedExtension` 是它的类型擦除（封闭：只有对每个 `E: Extension` 的那一个实现），`ExtensionSet::with(e, grant)` 与 `.and(other)` 用它把不同类型放进一张表。

顺序：所有扩展的注册各调一次（同时查授予、装路由表、中枢的主题与 space 账本、传输的路由、界面板）→ 按顺序 `migrate` → 建实例（`main`、`[[instances]]`、账本里的）→
按顺序 `start` → 开始服务。注册冲突、`migrate` 或 `start` 返回 `Err`：只停用这个扩展和它的方法、主题、路由、space、实例，原因进诊断。

退出：SIGINT、SIGTERM、`--exit-with-stdin` 下 stdin 关闭，都是先停监听，再倒序调各扩展的 `stop`，再停实例、删本进程的上传目录。
整个收尾最多 `SHUTDOWN_GRACE`（10 s）；超了，或收尾中再收到一次 SIGINT / SIGTERM，记一条日志后以 1 退出。

### 17.3 声明

| 类型 | API |
|---|---|
| `Manifest` | `Manifest::new(id, version)`、`.title(t)`、`.disclosure(text)`（诊断页与披露里那段话：它做什么、替谁做）、`.requires([caps])`、`.optional([caps])`、`.defines(CapabilitySpec)`（名字必须是 `<id>.<名>`）。id：原生扩展 `[a-z][a-z0-9-]*`，不带点；读：`id()`、`version()`、`title_text()`、`disclosure_text()`、`required()`、`optionals()`、`defined()` |
| `Registrar` | `r.id()`、`r.config()`、`r.method(name)`、`r.topic(name)`、`r.space(SpaceDecl)`、`r.tool(ToolSpec)`（列在诊断里的工具规格，实现由 `engine()` 给）、`r.route(prefix, Access, handler)`、`r.static_dir(prefix, dir, Access)`、`r.ui()`、`r.engine()`、`r.engine_everywhere()`、`r.template(InstanceTemplate)`；后三个只限原生 |
| `MethodBuilder` | `.access(Access)`、`.schema(json)`、`.describe(text)`，最后 `.handler(f)` / `.raw(f)` / `.with(Arc<dyn MethodHandler>)`。全名 `<id>.<name>`；缺省谁都不能调 |
| handler | `async fn(call: Call, p: P) -> Result<R, RpcError>`，`P: DeserializeOwned`（不符返回 `INVALID_PARAMS`），`R: Serialize`。`.raw` 收发 `serde_json::Value` |
| `TopicBuilder` | `.param(name)`（按参数分值）、`.access(Access)`、`.published()`（缺省：扩展推整值）、`.computed(every, TopicCompute)`（有人订时才算，变了才推）。全名 `<id>.<name>`；缺省设备与任何扩展可订；没有恰好叫 `<id>` 的主题 |
| `Topics` | `cx.topics().publish(topic, params_json, value).await`：推**整值快照**，`topic` 写短名；`.changed(topic, params)`（计算型主题变了）；`.subscribe(全名, params)` → `TopicStream`（以自己身份订别的主题） |
| `SpaceDecl` | `SpaceDecl::new(id)`、`.instance("main" \| "<实例 id>" \| "template:<模板>")`、`.scene(scene)`、`.cross_space()`（声称它的设备调用可以观看、操作任何 space 的会话：防误的标志，只限原生，诊断与披露列出）、`.default_space()`（不声称 space 的设备调用按它检查；一个进程至多一个，`[extensions] default_space` 能盖过）。原生扩展可以声明任何没被占的 id；不声明：`main` 实例、实例的默认场景。只有界面没有后端的 space 不用声明 |
| 路由 | `harnix::ext::http`：`Request`、`Response`、`RouteHandler`（稳定，只依赖 `http` 1.x）；`harnix::ext::http::axum::router(axum::Router)`（feature `axum`，绑 axum 0.8，**不在稳定承诺内**） |
| `UiDecl`（`r.ui()`） | `.bundle(id)`（编进外壳的 UI 包认的扩展 id，缺省自己的 id）、`.surface(..)`、`.inline()`、`.module(..)`、`.point(PointDecl)`：扩展定义的贡献点，进生成的贡献点目录。后三项里的 `surface`、`inline` 是给扩展包的；原生扩展的界面是编进外壳的 UI 包，后端只声明哪个包是它的 |
| `InstanceTemplate` | `InstanceTemplate::in_process(id, EngineProfile)` / `::spawn(id, EngineProfile)` / `::attach(id, AttachTo)`，再 `.data_root(pattern)`、`.param(name)`、`.ui()`、`.max(n)`、`.inherit_providers()`、`.reap_on_stop()`、`.contribute(\|e\| …)`。全名 `<扩展 id>.<模板 id>`。实例的 id 就是建它时给的 `name`，全进程唯一；名字和参数只能是字母、数字、`-`、`_`、`.`，不能以 `.` 开头。`data_root` 默认 `{name}`，可用 `{name}` 和声明过的参数，在 `~/.<product_id>/instances/` 下；建过的实例记在 `kernel/instances.json`。`.ui()`：界面能按它建（只填 `param` 放开的参数）。`.max(n)`：至多 n 台。`.inherit_providers()`：这个模板的实例也看得到 `main` 的 provider（设置页配的）——建实例时、以及每次 `settings.setProvider` 之后，Harnix 把它们（连同引擎放在 `main` 全局层 `settings.local.json` 里的密钥）并进实例数据根的 `settings.json`（0600）再让它 `engine.config.reload`；Harnix 写过哪些 id 记在 `.inherited-providers.json`，只动这些，实例自己配的同名 provider 不覆盖。`.reap_on_stop()`（缺省关）：停下或重启时收掉会话留下的进程。spawn 的 daemon 自成一个会话（`setsid`），它退出、被杀、崩溃或看护重启时，Harnix 把还留在这个会话里的进程 TERM、等 1.5 s、KILL；两种载体都在实例的全局设置层打开引擎的 `sandbox.reap_process_groups`（正常退出时引擎自己先收，进程内实例只有这一半）。spawn 的 daemon 的引擎还把会话命令所在的进程组记在数据根的 `daemon/groups/harnix.json`：daemon 被 `kill -9` 后看护重启时，新引擎先停掉上一次留下的，再建会话。daemon 在自己的会话里收不到终端的 Ctrl-C，退出照旧跟着 stdin 关闭走；自己 `setsid` 出去的进程收不到；实例视图带 `reap: true`。`.contribute(\|e\| …)`：两种载体都有的原生贡献。进程内模板把它并在载体自己的贡献之后；spawn 模板的贡献留在产品进程里：它的工具、钩子与门经通往 daemon 的链路提供、在这里作答，场景层的会话建在 daemon 的基础场景上并带上该层追加的东西；attach 的 daemon 没有。场景层不是引擎自己的场景，不列进 `EngineProfile.scenes`：列了的会被滤掉，当缺省场景则被拒 |
| `TemplateCarrier` | `InProcess { engine }`（只有扩展的模板能带原生贡献）、`Spawn { engine, program: Option<PathBuf>, args }`（`None` = 本二进制的 `daemon` 子命令）、`Attach(AttachTo { socket \| tcp + token })` |
| `EngineContribution`（`r.engine()`） | `.scene(SceneLayer)`、`.provider(Arc<dyn ToolProvider>)`、`.prompt_gate(Arc<dyn PromptGate>)` |
| `engine::EngineProfile` | `scene = "harnix"`、`scenes = []`、`model = "claude-sonnet-4-6"`、`max_tokens = 2000`、`session_cap = 32`、`session_idle_timeout_secs = 3600`、`permission_prompt_timeout_secs = 300`（括号里是默认值）；builder：`EngineProfile::new().scene(..).scenes(..).model(..)` |
| `engine::Tool` 等 | `Tool`、`ToolSpec::new(name, description, schema)`（`.doc`、`.source`、`.read_only`、`.concurrency_safe`、`.destructive`）、`ToolCx`（`session_id()`、`project()`、`cwd()`、`depth()`、`chain()`、`cancel()`、`progress()`）、`ToolOutput`、`ToolError`、`Permission::{Allow, Ask, Deny}`、`ToolProvider`、`ToolHook`、`ToolCallFacts`、`HookDecision`、`SceneLayer`、`SceneBase::{builtin, builtin_with, named}`、`PromptText::{cached, plain}`、`PromptGate`、`PromptFacts`、`PromptDecision`、`Delivery`、读者 `engine::readers::{Recordings, Transcripts}` |

### 17.4 `Access`、能力、`Trust`、`Subject`

**主体 `Subject`**：准入只发四种。

| 值 | 是谁 |
|---|---|
| `Kernel` | 本二进制：装配、内核行的内部调用 |
| `Device { id }` | 一台配对过的设备，或本机带令牌的页面：一个人 |
| `Extension { id }` | 一个扩展以自己的身份：原生经 `ExtensionContext`；扩展包的后端经 `host.call`；扩展进程经扩展线 |
| `Engine { instance }` | 一台实例的引擎，只在审计里出现（记它做的事，如扩展的引擎发得太多）；没有哪一行对它开，也没有引擎回头调本进程的 HTTP 端点 |

标签：`kernel`、`device:<id>`、`ext:<id>`、`engine:<instance>`。0.4.2 的 `Principal` 不存在了：`Principal::Device` → `Access::device()`。

**一行对谁开放 `Access`**（方法、主题、路由的每一行都有一个）：

| 构造 | 谁能调 |
|---|---|
| `Access::kernel()` | 只有内核，直到 `with_*` 再开 |
| `Access::device()` | 设备与内核；**不含扩展**。扩展要另写 `.with_extensions()` |
| `Access::extensions()` | 只有扩展（与内核） |
| `Access::only(id)` | 只有某一个扩展 |
| `Access::capability(cap)` | 只有内核与持有 cap 的扩展；设备永远不在里面 |
| `Access::refused(why)` | 只有内核，对别人说明理由 |
| `Access::hub_owned(why)` | 谁都不行、不能授予：绕过中枢的方法 |
| `.with_device()`、`.with_extensions()`、`.with_extension(id)`、`.requiring(cap)`、`.refusing(why)`、`.scoped(RowScope)` | 在上面的基础上加 |

`RowScope`：`Open`（与 space、会话无关）、`Space`（作用在一个 space 上）、`Session`（作用在一个会话上）；扩展包的调用按它收窄到自己的 space 与会话。

**两条要记住的**：

- `Access::device()` 是**只对设备（与内核）开**。人经界面做的决定——批准与拒绝、授予与撤销、设置、设备、包与安装、系统动作——都该是它：第一方扩展的行
  （`system.*`、`settings.*`、`packages.*`……）、`harnix.session.setModel`、`harnix.bulk.upload`、示例 hello-app 的全部行。第一方扩展替设备做时用的是它自己的能力。
- **扩展也要调的行，明写 `Access::device().with_extensions()`**：中枢的会话行与主题、引擎直通、实例登记表、内核自述（`harnix.hello`、`harnix.ui`、`harnix.extensions`、
  `harnix.diagnostics`、`harnix.audit`、`harnix.reachable`）、扩展发布的主题（缺省）。`harnix.session.respond` 对扩展开（flow 按策略作答）；
  扩展包被收窄到自己的 space，所以包只能答自己 space 里会话的提示。`tests/live/api/method-table.spec.ts` 守着：批准、拒绝、授予、撤销、安装、卸载、配对这类不要能力的行不许列扩展。

**信任 `Trust`**：写在扩展的登记里，一处。`Origin`（原生，出身即信任：由每行的 `Access` 判，不另设白名单）；`Consent { calls, carrier }`（扩展包：只到同意记录里的 `calls` 与它自己的名字，
再按 `RowScope` 收窄；`carrier` 是 `PackageCarrier::{Wasm, Process}`）。`cx.grants().trust()` 读它。

**内核能力目录 `harnix::caps`**：

| 内核能力 | 意思 | 原生 | 扩展包 | 谁用 |
|---|---|---|---|---|
| `instances.admin` | 停、重启任何非内核拥有的实例；按任何模板建实例、列出全部模板 | ✔ | | system |
| `engine.inspect` | 读**任何**会话的内部：`inspectContext`、`tasks`、`taskOutput`、`goal`、`history` | ✔ | | system、flow、settings（会话的 MCP 上下文，可选） |
| `engine.control` | 改任何会话的内部：`stopTask`、`messageTask`、`dropMessages`、`editMessage`、`compact`、`clear`、`setModel`、`rename`、`setGoal`、`updateGoal`；读、改项目的信任：`engine.project.status`、`engine.project.trust` | ✔ | | system、flow、harnix-app（项目信任，可选） |
| `engine.config.read` / `engine.config.write` | 读 / 写引擎设置与 provider；后者含 `engine.config.reload`、`harnix.instances.inheritProviders`、宿主服务 `SettingsTiers`（§17.8） | ✔ | | settings、packages（写，可选） |
| `engine.plugins` | Agix 插件的装、卸、启停、重读；往实例里放 skill | ✔ | | plugins、packages |
| `engine.export` | `engine.session.export`、`harnix.session.import` | ✔ | | harnix-app |
| `engine.mcp` | 引擎的 MCP 服务器与登录：`engine.mcp.addServer`、`updateServer`（只写项目层）、`auth.start / complete / cancel / logout` | ✔ | | settings（可选） |
| `recordings.read` · `transcripts.read` | 录像与用量的读者 · 转录的读者；都经引擎的协议读（`session.calls`、`call`、`callBlob`、`search`），每台实例各有一套，实例连不上读者返回 `Unavailable` | ✔ | | observability · harnix-app |
| `process.observe` | 版本、运行时长、内存、线程、已连接的客户端；各扩展的日志 | ✔ | | system |
| `devices.manage` | 配对码、列设备、撤销设备 | ✔ | | devices |
| `extensions.manage` | 拿 `harnix-host-api` 的 `HostServices`：运行期登记扩展、拉起扩展进程、收养扩展进程的引擎 | ✔ | | packages |
| `ui.page` | 进页面的 ES 模块 | ✔ | 只 `trusted` 发布者 | — |
| `events.all_tools` | 看到每个会话的每一次工具调用 | | ✔ | — |
| `feedback.read` | 订主题 `harnix.feedback`：收到人对回复的每一条反馈，评分、类别、备注原文，连同会话（§7.3） | ✔ | | —（缺省谁都没有） |
| `net:<主机>` · `env:<名字>` · `storage:<MB>` · `fs:data` · `fs:project` | 扩展包的资源能力 | | ✔ | — |

**跨 space 不是一项能力**：同页的代码能声称任何 space，所以「在一个 space 里看别的 space 的会话」只防误。它是 space 声明上的标志 `cross_space()`，只有原生扩展能声明；
真正的跨 space 读写权力在 `engine.inspect`、`engine.control`、`instances.admin`，它们才是安全边界。

### 17.5 运行期：`ExtensionContext`、`Call`

| `ExtensionContext` | 说明 |
|---|---|
| `id()` | 扩展 id |
| `grants()` | `GrantsView`：`has(&cap)`、`capabilities()`、`trust()` |
| `config::<T>()` | profile 的 `[ext.<id>]` 节 |
| `data_dir()` | 这个扩展自己的数据目录（`<状态目录>/ext/<id>/data/`，见 §20） |
| `projects_dir()` | 本产品的项目根：选项目的起点、新项目建在这里 |
| `credentials()` | `&dyn Credentials`：`.get(reference) -> Result<Secret, String>` |
| `topics()` | `Topics`（§17.3） |
| `call(method, params)` | 以 `ext:<id>` 的身份调任何方法（根调用），和设备走同一个准入点、同样审计 |
| `sessions()` | `Sessions`，见 §17.6 |
| `instances()` | `Instances`：`create(template, name, params)`、`ensure(template, name, params)`（幂等，并发调同名也只建一台，`start` 里用）、`ensure_ready(template, name, params, timeout)`（同上，等到 `ready` 再返回；`down` 或超时报 `INSTANCE_UNAVAILABLE`）、`wait_ready(instance, timeout)`、`stop(instance)`（数据根留着） |
| `engine(instance)` | `Option<EngineView>`，一台实例的只读视图：`instance()`、`active_scenes()`、`model()`、`trust()`；要能力的：`recordings()`（`recordings.read`；`calls`、`call`、`turns`、`envelopes`、`usage`）、`transcripts()`（`transcripts.read`；`search`，返回 `Result`，实例连不上是 `ReadError::Unavailable`）。每台实例都有读者：`main`、进程内的模板实例、spawn、attach 的实例与 `proc.<id>`。
录像按根会话存：整棵会话树（侧链、子 Agent）的调用在根会话的一份录像里，`calls` 的 `index` 是它在这份录像里的位置，行上有 `session_id` 与 `parent_session_id`。录像只覆盖最近一次运行（daemon 重启后第一次调用会把它重写），`calls`、`turns`、`envelopes`、`usage` 的结果带 `run`，读者见过上一次运行消失时标 `earlier_run`，用量不是跨运行的合计。`calls` 的列表只有每次调用的概况与工具调用的名字、字数，工具的输入与结果在 `call` 的详情里。`usage` 不叫醒休眠的模式 3 进程，给读过的旧数并标 `stale`，其余读者的调用会叫醒它 |
| `process()` | `process.observe`：版本、运行时长、内存、线程、已连接的客户端 |
| `bulk()` | 一次性上传凭据：`grant(max_bytes)`、`grant_named(name, max_bytes)`（文件名以清理过的 `name` 结尾，给按名字或后缀读它的，如 `plugins.upload`） |
| `ui()` | `UiPublisher`：运行期发布的界面条目（`plugins` 发 Agix 包的 `[[ui]]`） |
| `devices()` | `devices.manage`：`list`、`pair_begin`、`pair_complete`、`revoke` |
| `service::<S>()` | 一个有名字的宿主服务；`S` 只给名字（`NAME`）和交出来的类型（`Handle`），要什么能力由提供它的宿主按名字定、按授予查——请求方自己写的类型说了不算。0.4.4 的服务是 `harnix_host_api::HostServices`（`extensions.manage`，只有宿主组件用）与过渡的 `SettingsTiers`（`engine.config.write`，只有 `settings` 用）（§17.8） |
| `listen()` / `listening().await` | 进程监听在哪：`Listen { addrs, url }`（`addrs` 是实际绑上的地址，`--port 0` 时是系统给的端口；`url` 是 `http(s)://<第一个地址>/`，不带令牌）。扩展的 `start` 在监听**之前**，所以 `start` 里 `listen()` 是 `None`；要写运行记录就 `tokio::spawn` 一个任务 `listening().await` |
| `provide(v)` / `provide_arc(arc)` / `state::<T>()` | 扩展状态，handler 里用 `call.state::<T>()` 拿 |

`ExtensionContext` 没有公开构造；调不属于自己的能力答 `REFUSED`，`data = {reason: "capability", capability}`。
**拿不到的**：中枢、`EngineLink`、路由表、授予表、数据根的路径、载体的管道。

`Call`（handler 拿到的那一次调用）：`subject()`、`device()`（是设备调的就给设备 id）、`on_behalf_of()`、`ext()`（`&ExtensionContext`）、`state::<T>()`、
`cancelled().await`、`deadline()`、`run(future)`（与取消、截止时间赛跑：`CANCELLED` / `DEADLINE_EXCEEDED`）、`call(method, params)`（以这个扩展的身份、作为子调用，
链、取消、截止时间都带过去）、`meta()`。`MigrateContext { id, data_dir, config }`；`MigrationReport::none()` / `::applied(steps)`。

### 17.6 会话：`cx.sessions()`

| 方法 | 说明 |
|---|---|
| `create(SessionSpec) -> SessionRef` | `SessionSpec::new()` 再 `.space(..)`、`.instance(..)`、`.scene(..)`、`.project_root(..)`、`.provider(..)`、`.model(..)`、`.permission_mode(..)`、`.permission_rules(..)`、`.output_schema(..)`、`.recording(..)` |
| `open(instance, session_id)` | 已有的会话（内存里的或只在磁盘上的） |
| `send(&s, text, OnBusy)` | `OnBusy::{Queue（默认）, Reject, Interrupt, Steer}` |
| `send_with(&s, text, SendOptions)` | `SendOptions::new().on_busy(..).attachment(..).reference(SessionRef)`；`Attachment::{file(path), text(path, content), image(media_type, base64)}`；`reference` 同输入框的 `@session`（那个会话的转录作为文本附件）；带附件的不 steer |
| `interrupt(&s)` / `close(&s)` | — |
| `move_to(&s, space)` | 把会话连同转录改归另一个 space（`harnix.session.move`）：给把一类会话从一个 space 挪到另一个的应用。已经在那里就不变；答 `{space, from, moved}`。设备调不了（会话归哪个 space 是应用定的）；没有那个 space、或那是系统 space、或实例归别的 space，答 `SPACE_MISMATCH` |
| `append_entry(&s, Entry::new(event, payload))` | 扩展条目：命名空间由中枢定为你的扩展 id；只给人看 |
| `entries(&s) -> Vec<Value>` | 本扩展写过的条目，最早的在前 |
| `tap(&s)` / `tap_with_capacity(&s, n)` | 帧流，默认容量 `TAP_CAPACITY = 512`。`tap.next()` → `TapEvent::Frame(json)` 或 `TapEvent::Lagged { last_seq }`；落后了 `tap.catch_up()`；`tap.last_seq()` 是读到的最后一帧的 `seq`。`tap.next_settled() -> Option<Value>`：跳过别的帧，只等下一个 `harnix.turn_settled` 的 params；满了时结算帧另存（最多 `SETTLED_HELD = 64`），落后也不丢。`tap.catch_up_settled()`：同 `catch_up`，另存的结算帧按位置放回 `next()` 读的队列。`outcome` 有 `stop_reason`、`usage`、`text`（回答文本，最多 64 KiB，超了 `text_truncated`）、`tool_calls`（`[{id, name, is_error?}]`），有才有的 `structured_output`、`message` |
| `in_space(space)` | 返回同样的句柄，经它的每个调用都声称那个 space（落在 create / open / send / send_with / interrupt / close / entries / append_entry；`move_to` 的 `space` 仍是目标）。用归 space 的实例时要它；`SessionRef` 不加字段：它是身份键 |

没有记录的会话（引擎自己建的，如子 Agent 的）归 space 的顺序：引擎报了父会话就随父会话的 space；否则是它所在实例的 space（实例归 space 时）；再否则是缺省 space。
`SessionRef::new(instance, session_id)`（字段 `instance`、`session_id` 公开，`#[non_exhaustive]`，不会再加字段）：会话的全名永远带实例。

### 17.7 错误：`RpcError`、`harnix::codes`

`RpcError::new(code, msg)`、`::with_data(code, msg, data)`、`::invalid_params`、`::not_found`（没有这个**方法**，-32601）、`::missing`（`NOT_FOUND`）、`::conflict`（`CONFLICT`）、`::refused`；
字段 `code`、`message`、`data` 公开，结构 `#[non_exhaustive]`：读字段、`RpcError { data: Some(d), .. }` 模式都行，构造只经构造函数。

| 码 | 名字 | 含义 |
|---|---|---|
| -32700 / -32600 / -32601 / -32602 / -32603 | JSON-RPC 标准 | 解析、请求、没有这个方法、参数、内部 |
| -31000 | `REFUSED` | 方法存在，这个主体不许调（和「没有这个方法」是两回事）。能力不够：`data = {reason: "capability", capability}`；白名单之外 `reason: "outside_calls"`；范围之外 `reason: "scope"` |
| -31001 | `HANDSHAKE_REFUSED` | 握手没谈拢：版本、贡献点 API、拓扑 |
| -31002 | `INSTANCE_NOT_FOUND` | 没有这台实例 |
| -31003 | `SPACE_MISMATCH` | 会话不属于调用方声称的 space。**防 bug，不是权限** |
| -31004 | `INSTANCE_UNAVAILABLE` | 实例在，但现在够不着（daemon 重启中、协议版本不符、扩展进程重起期间的 `proc.<id>`） |
| -31005 | `BUSY` | 这条单双向连接上已有 `limits.max_in_flight`（16）个请求没答，或排队与在途已满；这一个没执行，等一个答了再发。`data.limit` 写是哪个上限 |
| -31006 | `NOT_FOUND` | 方法在，问的那样东西不在：会话还没有录像、录像里没有第 n 次调用、没有这个工作区、没有这个流程运行、扩展发布了自己没声明的主题。实例不在仍是 `INSTANCE_NOT_FOUND` |
| -31007 | `CONFLICT` | 要动的东西已经变了，这次调用现在做不成：流程任务的租约不是你的了、运行的版本已经往前走。重读再试 |
| -31008 | `CANCELLED` | 调用被取消 |
| -31009 | `DEADLINE_EXCEEDED` | 过了截止时间（扩展包后端超过 `timeout_ms` 也是它） |
| -31010 | `EXTENSION_UNAVAILABLE` | 扩展在、现在答不了：`data.failure` ∈ `starting`、`stopped`、`faulted`（被置于一旁）、`reentrant` |

分界：**实例**够不着是 `INSTANCE_UNAVAILABLE`；**扩展**的方法与工具够不着是 `EXTENSION_UNAVAILABLE`。引擎的错误码（`-32000` 段）原样透传（例如会话不存在是 -32000）。

### 17.8 宿主组件：`harnix-host-api`

`packages` 与两个载体是**宿主组件**：写成扩展的形状、经同一个登记口与授予进来，但要碰几样普通扩展不该看到的管道（运行期登记、拉起扩展进程、收养引擎）。
那几样在 `harnix-host-api`（S3），经 `cx.service::<HostServices>()` 拿，要 `extensions.manage`。**应用的扩展用不到它**；测试应用的 `fakehost` 经它演示。
接口（`HostApi`、`DynamicRegistrar`、`ExtensionHandle`、`Carrier`、`AdoptedEngine`……）与宿主对动态描述做的七项结构检查写在
[扩展包](harnix-ext-package.md) 的接口清单里。S3 随骨架变，不在扩展接口的稳定承诺内。

同一个 crate 里还有一个**过渡**服务 `SettingsTiers`（`cx.service::<SettingsTiers>()`，要 `engine.config.write`，句柄 `dyn SettingsFiles`：
`global(instance)`、`scene(instance, scene)` 给出一台实例的引擎读的全局层与场景层 `settings.json` 在哪）。它是 Agix 设置目录的布局，只有第一方 `settings` 用——
引擎接下全局层的写入（经协议）之后连它一起删；所以在 S3，不在 S1。

### 17.9 日志：扩展日志

扩展照常用 `tracing` 写日志，不用自己标是谁的。本进程另记一份**扩展日志**：属于某个扩展的行，按扩展分开、有界地留最新的（每个扩展至多 500 行，全进程至多 4000 行，
超出时先丢全进程最旧的；一行的消息至多 2 KiB），和进程自己的日志分开过滤，`RUST_LOG` 不影响它。系统 space 的「扩展日志」页签读它（[系统 space](system-space.md) §6.1）。

一行算谁的，按这个顺序认：

| 来源 | 怎么认 | `origin` |
|---|---|---|
| 扩展进程（模式 3）的 stderr | 载体读 stderr 的那一端把每行记成这个扩展的（字段 `extension`、`stream = "stderr"`），级别照子进程那行写的 | `process` |
| 宿主关于某个扩展的行 | 事件带 `extension` 字段（停用、迁移、扩展进程起来了……） | `host` |
| WASM 扩展包 | 它的 `host::log`，以及它的载体关于它的行（字段 `app`）；`debug`、`trace` 的也留 | `package` |
| 原生扩展 | 宿主在扩展被要求做的每件事外面开一个 span（`harnix_kernel::EXTENSION_SPAN`，即 `harnix.extension`，字段 `extension`）：应答一个方法、计算一个主题、订阅、`start`、`stop`。span 里 `info` 及以上的事件都是它的；一个扩展调另一个，被调的那个应答时写的是被调者的 | `call` |

**原生扩展自己 `tokio::spawn` 的任务不在那个 span 里**，它写的行不进扩展日志（进程日志里照有）。要它们也算进来，把当前 span 带过去：
`tokio::spawn(work.instrument(tracing::Span::current()))`，或在事件上写 `extension = %cx.id()`。

读它：`harnix.extensions.log {extension?, level?, after?, limit?}`（要 `process.observe`；设备经 `system.extensions.log`）。`level` 是要的最低级别，`after` 是一行的 `seq`
（全进程递增），答 `after` 之后、合乎条件的最新 `limit` 行（缺省 500，至多 1000），旧的在前：`{lines: [{seq, at_ms, extension, level, origin, target, message}], last, extensions: [{extension, lines}], dropped}`。
`last` 是日志里最新一行的 `seq`，下一次读拿它当 `after`；`dropped` 是开机以来丢掉的行数。类型在 `harnix_contract::wire`（`ExtensionLog`）。

---

## 18. profile（TOML）

**Harnix 自己的每一节都是 `deny_unknown_fields`，且每一节有一个所有者**：写了不认识的键、没有所有者的节，进程拒绝启动并说哪一行。
`[ext.<id>]` 的内容由那个扩展的 `Config` 类型读（第一方扩展一律拒绝未知键；你的扩展自己定）；装配不认这个 id、又是扩展包 id 形状的节交给 `packages`，
它也没装就进诊断（包可能刚卸掉）。

**时间带单位**：秒的键以 `_secs` 结尾（`session_idle_timeout_secs`、`idle_secs`）。新加的键照这个写；已有的键与命令行开关不改名
（改名会让应用的配置失效），没写单位的在旁边注明。

```toml
[engine]                       # main 实例；字段同 EngineProfile（§17.3）；不写 profile 时是产品的 default_engine（§17.1）
scene = "harnix"
scenes = []
model = "claude-sonnet-4-6"
max_tokens = 2000
session_cap = 32
session_idle_timeout_secs = 3600   # 会话闲置多久从内存里放下（转录在盘上，再开即回）
permission_prompt_timeout_secs = 300   # 权限询问等人多久

[transport]
topologies = ["single_duplex"]  # 缺省；还有 "split_streams"（在代理后面时加上）、"request_only"
host = "127.0.0.1"             # IP 字面量（不是主机名），启动时解析，不是地址就起不来；非回环要 TLS 和配对过的设备
port = 4080
launch_link = false            # true：令牌不进页面，浏览器凭一次性启动链接换 cookie
extra_origins = []             # 回环之外允许的浏览器 Origin，例如 "https://192.168.1.20:4317"

[brand]
name = "Helm"                  # 页面标题、左栏产品名；不写是 Harnix
icon = "brand/helm.svg"        # 品牌图标（SVG），相对 profile 所在目录；服务在 /brand/icon.svg
product_id = "helm"            # 程序自己的目录 ~/.helm（§20）；小写字母、数字、`-`，字母开头；不写是 harnix

[shell]
model_switch = false           # true：加 harnix.session.setModel，没有系统 space 也能在输入框换模型（只限调用方声称的 space 的会话）
new_session = true             # false：整个外壳不画「新会话」、⌘K 不归外壳（每个 space 都这样；space 自己的 newSession 照旧叠加）；只是界面
narrow_details = "over"        # ≤ 960 宽时打开的右栏：over（缺省）是从右边盖住中区的抽屉；beside 三栏并排（左栏收成 rail、右栏变窄），< 720 仍是抽屉

[ui]
dir = "…"                      # 从这个目录出界面（同 --ui-dir）；不写用编进二进制的
runtime_modules = true         # false：扩展包的界面——受信包的模块、其余包的面——一律不进页面、不服务；交易类产品要关

[extensions]
off = ["system"]               # 链进来、但关掉的扩展：注册（别人占不了它的名字），但不装、不迁移、不启动；方法连同授予一起撤，声称它的 space 的调用答 SPACE_MISMATCH
default_space = "sessions"     # 盖过扩展用 SpaceDecl::default_space() 声明的缺省 space；必须是某个启用的扩展声明的

[ext.plugins]                  # plugins 扩展
ui_modules = true              # false：Agix 包的 [[ui]] 模块一律不进页面

[ext.packages]                 # packages 宿主组件；没有 enabled：链进来就开，要关写 [extensions] off = ["packages"]
dev = false                    # true：未签名的包也能从文件装、能启动（开发应用用）
store = "https://…/index.json" # 商店目录（https:// 或本机 file://）；不写用内置的。目录里列什么都要验签才装
carrier = "wasm"               # wasm（模式 2，缺省）| process（模式 3）
# proc_program、sandbox、secrets."<id>"、trusted_publishers 见扩展包文档的 profile 一节
[ext.packages.trusted_publishers]   # 受信发布者：发布者 id → 签它的 ed25519 公钥（hex）；它签全的包不经商店副署也能装（trust = trusted）
# acme = ["<64 位 hex>"]

[ext."acme.notes"]             # 一个扩展包：按键盖过它清单的 [config]

[[instances]]                  # 固定实例
id = "ci"
carrier = "spawn"              # in_process | spawn | attach
data_root = "ci"               # 可选
program = "/path/to/daemon"    # 可选，spawn 用；默认本二进制的 daemon 子命令
args = []
[instances.engine]             # 同 [engine]
scene = "ci"
[instances.attach]             # attach 用
socket = "/run/agix/shared.sock"   # 或 tcp = "127.0.0.1:7878" + token = "<凭据名>"

[[instance_templates]]         # profile 里的模板只能 spawn 或 attach（没有原生贡献可给进程内的）
id = "sandbox"
carrier = "spawn"
data_root = "sandbox/{team}/{name}"   # 默认 "{name}"
params = ["team"]
ui = true                      # 界面可以按它建
max = 4
inherit_providers = true       # 可选：它的实例也看得到「设置 → 模型」里配的 provider（并进实例的全局层，设置页一改就推过去）
reap_on_stop = true            # 可选：停下或重启时收掉它的会话留下的进程
[instance_templates.engine]
scene = "general"

[ext.watchlist]                # 你的扩展自己的节，Extension::Config 读
db = "watchlist.sqlite"
quote_api_key = { credential = "helm/quote-api" }
```

要关掉一个第一方 space，不链它或写 `[extensions] off`；0.4.2 的 `[shell] spaces`、`[modules.*]`、`[apps.*]`、`[ui] runtime_modules`（改了意思）的写法都不存在了（迁移说明有对照）。

---

## 19. 命令行与环境变量

| 开关 | 说明 |
|---|---|
| `--profile <file>` | 上面的 profile；下面的开关只在给了的时候覆盖它 |
| `--port`、`--host` | 监听 |
| `--scene`、`--scenes a,b`、`--model` | `main` 实例的默认值 |
| `--ui-dir <dir>` / `--headless` | 换界面目录 / 不出界面（纯 RPC 节点） |
| `--replay-dir <dir>` | 模型调用改成录像回放（测试用；只有运维能开，客户端要不到）。扩展进程的沙箱此时只读放行这个目录 |
| `--replay-strict` | 和 `--replay-dir` 一起：请求与录下的不一样就让那次调用失败（缺省只报告）；回放的会话按录像那天定日期 |
| `--tls-cert`、`--tls-key` | 绑非回环必须 |
| `--agix-dir` | 引擎目录，默认 `$AGIX_CONFIG_HOME`，再默认 `~/.agix` |
| `--data-dir` | 项目目录（`ExtensionContext::projects_dir()`），默认 `$HARNIX_DATA_DIR`，再默认 `~/Documents` |
| `--launch-link` | 同 `[transport] launch_link` |
| `--exit-with-stdin`（隐藏） | stdin 关了就退出：桌面壳带走它的子进程 |
| `ui export <dir>` | 把编进去的界面写出来，给 CDN / 反代 |
| `daemon …`（隐藏） | spawn 的实例跑的就是它，开关取自模板的 `[engine]`：`--session-idle-timeout <秒>`、`--permission-prompt-timeout <秒>`（profile 里是带 `_secs` 的同名键）、`--lock-wait <秒>`，名字里不带单位，值都是秒。Harnix 起它时带 `--instance <id>`：它在 `<数据根>/.daemon-<id>.lock` 上持独占锁（内容是 pid），先等同一台实例的旧 daemon 放手（最多 `--lock-wait` 秒，默认 10）再建引擎；等不到就非零退出，实例记为 `down`，理由「the data root is held by pid N」。`reap_on_stop` 的模板再带 `--reap`：引擎把会话命令的进程组记在数据根里，起来时先停掉上一次被杀时留下的。模板写了 `program` 的不带这几个参数、不拿锁 |
| `ext-proc …`（隐藏，feature `proc`） | 模式 3 的扩展进程：产品二进制自己以这个子命令重新执行；不读 profile、不拿状态目录的锁。用不到它——Harnix 拉起它 |

| 环境变量 | 说明 |
|---|---|
| `AGIX_CONFIG_HOME` | 引擎目录 |
| `HARNIX_DATA_DIR` | 项目目录 |
| `HARNIX_STATE_DIR` | 程序自己的目录，单独挪（§20） |
| `HARNIX_CREDENTIAL_<名字>` | 默认的 `Credentials` 读这里；名字是引用转大写、非字母数字转 `_` |
| `HARNIX_UI_MODULES`、`HARNIX_UI_FIRST_PARTY`、`HARNIX_UI_OUT` | 界面构建：应用的界面入口（逗号分隔绝对路径）、编进来的第一方 UI 包（不设是全部，空串是一个也不要）、输出目录（§22.5） |
| `HARNIX_UI_DIST` | `cargo build` 时嵌进二进制的界面目录 |

---

## 20. 目录

| 路径 | 是什么 |
|---|---|
| `~/.agix/` | 引擎：设置分层、transcript、记忆、技能、录像。和 `agixd` 共用 |
| `~/.<product_id>/`（不配 `[brand]` 就是 `~/.harnix/`；`HARNIX_STATE_DIR` 可挪） | 程序自己的配置与状态。每一处有一个所有者，0.4.4 只读写下面这些位置 |
| 　`token`、`devices.json`、`harnix.lock` | 进程令牌、配对过的设备（与 `token` 一样只有属主可读；重启后仍在）、进程锁（进程活着就持有的独占锁，内容 `{"pid", "url"}`，`url` 在监听后写上：一个状态目录只能有一个进程，第二个启动时报「`<目录>` is in use by pid N (url)」并以非零退出） |
| 　`kernel/spaces.json` | 会话 ↔ space 的绑定、space 按模板建的实例、Harnix 起的会话（中枢读写） |
| 　`kernel/instances.json`、`instances/<data_root>/` | 实例账本、按模板建的实例的数据根 |
| 　`run/` | spawn 的 daemon 的私有套接字目录（0700） |
| 　`uploads/<pid>/` | 本进程的上传 |
| 　`ext/<id>/data/` | 原生扩展 `<id>` 的 `data_dir()`（例如 `ext/harnix-app/data/state.json`：工作区、归档、偏好） |
| 　`ext/<id>/install.json`、`ext/<id>/<version>/` | 扩展包的安装记录与**同意记录**（扩展进程看不见它）、解开并验过签的包（`packages` 拥有，见 [扩展包](harnix-ext-package.md)） |
| 　`ext/<id>/run/{data,kv,engine,work,home,tmp}` | 扩展包的数据、kv、（模式 3）它自己 Agix 的根、它的会话的项目目录、`HOME`、`TMPDIR`（载体拥有） |
| `~/Documents/` | 只放项目：选项目从这里开始，新建项目建在这里（`projects_dir`，`--data-dir` 能改）。Documents 里不放隐藏目录 |
| `<项目>/.agix/` | 项目自己的数据与配置**一律**放这里（引擎的项目层设置也在这），不开 `<项目>/.<产品>/` |

扩展的数据（`cx.data_dir()`）随产品 id 走；同一台机器上装 Helm 和 IPDrun 不会抢令牌和账本。

**一刀切**：0.4.3 不读、不迁移、不识别 0.4.2 及更早的状态（`state.json`、`spaces.json`、`instances.json`、`modules/`、`apps/`……）。升级要删什么见本地的迁移说明。

---

## 21. 线上协议

### 21.1 端点

| 路径 | 用途 |
|---|---|
| `/handshake` | 握手：协议版本、贡献点 API、拓扑 |
| `/ws` | `single_duplex`：一条双向连接 |
| `/rpc`、`/events/session`、`/events/host`、`/respond` | `split_streams` / `request_only` |
| `/pair`、`/challenge` | 设备配对 |
| `/auth/link`、`/auth/{code}` | 启动链接 |
| `/ext/plugins/ui/<包>/<模块>` | Agix 包的界面模块文件（要凭据；`[ext.plugins] ui_modules = false` 时不服务） |
| `/ext/<包 id>/ui/…` | 扩展包的界面文件（公开：iframe 不带凭据） |
| `/brand/icon.svg` | `[brand] icon` |
| `/ext/<扩展 id>/…` | 扩展的自有路由与静态文件，同一个扩展的路由共用一种 `Access` |
| `/ext/settings/mcp/callback` | MCP 服务器 OAuth 登录的回跳（§22.9）：交给引擎完成登录，留下一页说明结果；不带凭据的答 401，由打开它的页面接手 |

握手拿到的 `credential`：`Authorization: Bearer …` 或 `?credential=`；回环上也认进程令牌和启动链接的
cookie。一帧最大 16 MiB（`MAX_FRAME_BYTES`），一次上传最大 32 MiB。

`split_streams` 的 `/rpc`、`/respond` 一个请求一个答，答的是 JSON-RPC 帧，和 `/ws` 上同一请求的答一样：`id` 是请求的
`id`（读不出时为 `null`），错误带 `code`、`message` 和 `data`。先读帧、后认凭据：超过上限的帧（-32600，消息里写上限）、
不是 JSON 的（-32700）、没有 `method` 的（-32600）答 HTTP 200 和错误帧，带不带凭据都一样；帧读得出、凭据缺了或不认识
（进程重启过、闲置太久、设备被撤销）答 HTTP 401，正文仍是错误帧（`HANDSHAKE_REFUSED`，-31001），也带请求的 `id`。

### 21.2 方法

**权威清单是 `crates/harnix/tests/method-table.snap.json`**（每一行的所有者、谁能调、要什么能力、范围，由 LIVE 套件 `method-table.spec.ts` 对着运行中的进程比）与运行时的 `harnix.reachable`
（当前主体能调的）、`harnix.diagnostics`（全部，含 schema）。下表是分组索引，参数与答写在各方法注册时的说明里，不在这里重抄。

**命名**：内核的 `harnix.<区域>.<动词>`（单数区域是对一个东西的操作，复数是集合：`harnix.session.send`、`harnix.sessions.list`）；引擎直通 `engine.*`；
扩展在自己的 id 下（`system.context`、`flow.start`、`example.showcase.notes.add`）。第一方扩展也只在自己的 id 下，没有「预留命名空间」这种特权。

| 组 | 方法 | 谁能调 |
|---|---|---|
| 握手与自述（装配） | `harnix.hello`（见下）、`harnix.reachable`、`harnix.diagnostics`、`harnix.audit`、`harnix.extensions`（每个扩展：id、信任、载体、版本、启用、授予、行数、停用理由）、`harnix.ui`（方法与主题同值：每个扩展的界面条目、Agix 包的界面模块、扩展包的面）、`harnix.bulk.upload` | 设备、扩展（`bulk.upload` 只设备） |
| 会话驱动（中枢） | `harnix.sessions.list {space? \| instance?}`、`harnix.session.attach`、`.detach`、`.send`、`.queue.remove`、`.queue.edit {session_id, item_id, message}`（改排队中一条的话，位置与附件不变；已开始或已撤回的答 REFUSED）、`.create`、`.interrupt`、`.close`、`.delete`、`.fork`、`.respond {instance?, session_id, prompt_id, prompt_type?, decision? | answer?, tool_use_id?}`（回答一个提问，先答者生效、后来的静默成功；权限与计划审阅（`prompt_type` 缺省或 `permission`）给 `decision`，模型的提问（`question`）给 `answer: {type: answer, text} | {type: decline, reason?}`，MCP server 的询问（`mcp_elicitation`）给 `answer: {type: accept, content?} | {type: decline} | {type: cancel}`，形状不合答 -32602；第一个被收下的回答由中枢写进会话的流（事件 `ask_answered {prompt_id, prompt_type, decision? | answer?, extension?}`，表单的 `content` 不带），类型 `RespondParams`、`AskReply`、`AskAnswered` 在 `harnix_contract::wire`；提问与询问只推给设备建的会话；flow 按策略作答）、`.tasks`、`.rename`（引擎的标题，标题只在引擎）、`.move {instance?, session_id, space}`、`.entries.append`、`.feedback {instance?, session_id, space?, turn_id, call_id?, rating, category?, note?}`（对一轮回答的反馈，只设备调，§7.3）、`.answerer {session_id, tool_use_id}`（回答该工具权限卡的设备 `device:<id>`，没有设备回答时是 `null`；读一次即忘，最多留最近 1024 条）、`.outline {instance?, session_id, space?}`（会话的各轮，见下）、`harnix.topics.subscribe`、`.unsubscribe`、`harnix.spaces.list` | 设备、扩展（再按 `topic:<名>` 与范围判） |
| 回退（中枢） | `harnix.session.rewind {instance?, session_id, space?, to: "start" \| {turn} \| {before_turn} \| {tool_use}, scope?, dry_run?, fork_first?, force?, allow_partial?}`：文件、对话或两者回到之前一点，先 `dry_run` 看计划（[Harnix app](harnix-app.md) §5.1）；答 `RewindResult`。设备只对它声称的 space 的会话，不跨 space（同 `setModel`）；做完向每个观看者推 `harnix.session_resync` | 设备、`engine.control` |
| 会话看得到的（中枢） | `harnix.session.skills {instance?, session_id, space?}`：会话看得到的 skill（`engine.commands.list` 里展开成提示词的那些），每个带来源层 `tier`（`space`：Harnix 按 space 给会话列的扩展包目录，带 `package`；`project`：项目的 `.agents/skills`；`plugin`：Agix 插件包，带 `plugin`；`user`：引擎的全局层或场景层，引擎不分；`builtin`）、`dir`、`withheld`（项目未受信时被扣下一部分：内联命令不跑、`allowed-tools` 不给）；另有 `dirs`（Harnix 列的目录、状态、被遮住的同名 skill）、`listed`（会话不在引擎内存里时为 `false`、`skills` 为空）、`trust`。`harnix.session.hooks {instance?, session_id, space?}`：会话的轮次跑的 hook，`source` 是 `global` / `scene` / `project`（设置三层的 `hooks_config`）或 `host`（本进程在连接上登记的，带 `providers`），每条只有 `event`、`kind`（`command` / `prompt` / `http` / `agent` / `host` / `other`）、`matcher`（`if` 或工具模式，至多 80 字）、`label`、`on_error`、`in_force`、`withheld`（项目未受信）、`replaced_by`（同一事件被高层的列表整个替换）——**不带命令、地址、请求头、提示词**；`unread` 列出读不到的层。类型在 `harnix_contract::wire`（`SessionSkills`、`SessionHooks`） | 设备 |
| 扩展日志（装配） | `harnix.extensions.log {extension?, level?, after?, limit?}`（§17.9） | `process.observe` |
| `[shell] model_switch = true` 时 | `harnix.session.setModel {instance?, session_id, model, provider?, space?}`：只作用于调用方声称的 space 的会话，会话属别的 space 答 `SPACE_MISMATCH`；扩展改模型用 `engine.session.setModel`（要 `engine.control`） | 设备 |
| 实例（登记表） | `harnix.instances.list / templates / create / ensure / stop / restart / wait`。`create` / `ensure` 可带 `wait: "ready"`、`timeout_ms`（缺省 60000）：等这台变成 `ready` 再答，变成 `down` 立即以 `-31004` 报错（带理由），超时同样；`wait {instance, timeout_ms?}` 是对已有实例的同一种等。并发 `ensure` 同名同模板都拿到同一台。`templates` 只列调用方能建的 | 设备、扩展（`create` 只认 `ui` 模板；跨拥有者要 `instances.admin`） |
| 　 | `harnix.instances.inheritProviders`：把 `main` 的 provider 并进所有 `inherit_providers` 模板的实例并让变了的重读，答 `{reloaded: [id…]}` | `engine.config.write` |
| 引擎直通 | `engine.session.list / get / history / entries / resume`、`engine.scene.list / describe / activate`、`engine.commands.list`、`engine.mcp.status`、`engine.plugin.list`、`engine.daemon.status / ping / doctor` | 设备、扩展 |
| 　 | `engine.session.tasks / taskOutput / inspectContext / goal` | `engine.inspect` |
| 　 | `engine.session.stopTask / messageTask / dropMessages / editMessage / compact / clear / setModel / rename / setGoal / updateGoal` | `engine.control` |
| 　 | `engine.project.status / trust`（项目信任；信任记在引擎的全局设置里，所以扩展的引擎上同它的设置一样拒绝） | `engine.control` |
| 　 | `engine.session.export` · `engine.config.get / getProvider` · `engine.config.set / setProvider / update / reload` · `engine.plugin.install / uninstall / enable / disable / reload` · `engine.mcp.addServer / updateServer / auth.start / auth.complete / auth.cancel / auth.logout` | `engine.export` · `engine.config.read` · `engine.config.write`（设备永远不能） · `engine.plugins` · `engine.mcp`（设备永远不能；扩展的引擎上同它的设置一样拒绝） |
| 　 | `engine.import.*` | 只有内核 |
| 　 | `engine.session.run_turn / subscribe / unsubscribe`、`engine.daemon.subscribeEvents / shutdown` | **中枢独占**：谁都不能、不能授予 |
| `harnix-app` | `harnix-app.workspaces.*`、`.prefs.set`、`.projects.create`、`.recentProjects`、`.listDirectory`、`.files {session_id | project_root, query?, limit?}`（只走会话的项目或这里的项目，见 [Harnix app](harnix-app.md) §6）、`.search`、`.sessions.archive`、`.sessions.importUpload {name}`（一次性上传凭据，答 `{token, url, max_bytes}`；页面把 zip 送到 `url`，走大负载旁路）、`.sessions.import {token, space?, project_root?}`（把上传的数据包装成新会话，答引擎的 `{root_session_id, id_map, sessions, warnings}` 加 `instance`、`space`；包里的项目这台机器上没有就装成无项目会话并在 `warnings` 里说）、`.sessions.export {instance?, session_id, space?, format: markdown \| archive \| bundle, recording?}`（答 `{format, file, content}`：markdown 是可读的对话；archive 是引擎打的 zip（`encoding: "base64"`，另带 `bytes`、`sessions`、`attachments`、`recording`、`warnings`）；bundle 是 Harnix 拼的 JSON；超过一帧时拒绝）、`harnix-app.describe`（项目目录等）、`.projects.status {project_root, instance?, scene?}`、`.projects.trust {project_root, trusted, instance?}`、`.sessions.projectStatus {instance?, session_id, space?}`（项目信任，答的形状见 [Harnix app](harnix-app.md) §3）；主题 `harnix-app.trust` | 设备 |
| `system` | `system.context / messages.drop / messages.edit / tasks.stop / tasks.message / tasks.output / compact / clear / rename / model / goal / goal.set / goal.update`、`system.instances.stop / restart / create / templates`（面板列全部模板）、`system.process`（版本、运行时长、峰值内存、`threads`、已连接用户）、`system.extensions.log`（同 `harnix.extensions.log`，§17.9） | 设备 |
| `observability` | `observability.trace`、`.trace.call {index}`、`.turns`、`.usage {instance?, session_ids}`（一次至多 200 个会话）、`.calls {instance?, session_id}` | 设备 |
| `settings`、`plugins`、`devices` | `settings.*`（`setProvider` 的返回多一个 `reloaded_instances`；MCP 服务器的 `settings.mcp.*` 见 §22.9）、`plugins.*`、`devices.*` | 设备（`settings.mcp.context` 按会话） |
| `packages` | `packages.*`：列、启停、装、升、卸、清数据、同意、商店。参数与答见 [扩展包](harnix-ext-package.md) | 设备 |
| `flow` | `flow.*`（§23） | 设备 |
| 你的扩展 | `<id>.*` | 你声明的 |
| 扩展包 | `<扩展包 id>.*` | 清单 `[[method]]` 的 `subject` |

明确拒绝并说明原因的（不是「没有这个方法」）：引擎里交凭据、开端点的方法；中枢独占的方法；`engine.mcp.addServer`、`engine.mcp.auth.*` 等没授予的能力行。

**`harnix.hello`**（装配的行）：

```text
{ protocol_version, contrib_api_version, engine_protocol, limits,
  defaults: { instance: "main", space: <缺省 space> | null },
  engine: { model, active_scenes, scene, data_root, has_credentials, providers, status },   // main 实例
  instances: [{ id, carrier, trust, status, version, protocol_version }],
  permission_prompt_timeout_secs, scenes, commands,
  shell: { brand: { name, icon, product_id }, new_session, narrow_details, model_switch, user },
  extensions: [{ id, trust, carrier?, version, enabled }] }   // 外壳据此同步激活编译进来的 UI 包；harnix.ui 管之后的变化
```

### 21.3 主题

| 主题 | 参数 | 值 |
|---|---|---|
| `harnix.instances` | — | 实例树 |
| `harnix.instances.sessions` | `instance` | 一台实例的会话：每行带 `running`（有一轮在跑）、`queued`、`tasks`（还在跑的任务数——任务可以比起它的那一轮活得久）、`waiting`（在等人：`approval` 是权限询问，`answer` 是模型的提问或 MCP server 的询问，`plan` 是离开计划模式要人审阅计划，没有为 null）、`started_by`（`harnix` 是扩展或内核起的，`user` 是设备上的人起的）、`turn_started_ms`、`goal`（hub 最近听到的目标；没有目标为 null，hub 还没听到过就没有这个键） |
| `harnix.session.tasks` | `instance`、`session_id` | 一个会话的任务，和 `teams`：它的每个团队 hub 最近听到的 `team_progress`（`team, team_id, stage, stage_index, stage_count, status, members, failed`） |
| `harnix.sessions.status` | — | 各会话的运行状态 |
| `harnix.feedback` | — | 最近记下的一条反馈：`{instance, session_id, space, entry_id, feedback}`，每记一条推一次；只有持 `feedback.read` 的扩展能订（§7.3） |
| `harnix.spaces` | — | 同 `harnix.spaces.list` 的答；总在 |
| `harnix.ui` | — | 每个扩展的界面条目、Agix 包的界面模块、扩展包的面（也是同名方法） |
| `system.process` | — | 同 `system.process` 方法的答；有人订阅时每 5 秒重算一次推出 |
| `packages.list`、`packages.progress` | — | 扩展包的快照、工具调用的进度；见 [扩展包](harnix-ext-package.md) |
| `flow.*` | — | §23 |
| `<id>.<主题>` | 你声明的 | 你推的整值快照 |

扩展包撤下时，它的主题的每个订阅者按订阅的参数组各收到一次 `harnix.topic_ended { topic, params, reason }`，之后不再推；
再订答 `-32601`，等它重新登记。

通知不改名：`harnix.event`、`harnix.host_event`、`harnix.topic`、`harnix.topic_ended`、`harnix.turn_settled`、`harnix.session_resync`。
会话事件不是主题：`harnix.session.attach` 之后经 `harnix.event` 帧来，带 `seq`，断线按 `seq` 追赶。
条目以 `harnix.event { event: { kind: "extension_entry", entry } }` 推给正在看的人。

**`harnix.session.outline`**：一个会话的各轮，一轮一行，旧的在前，轮次导航与轨迹都按它分轮。从引擎的历史读出来，谁能读这个会话的
`engine.session.history` 谁就能调它（设备、扩展，声称的 space 照样核对）。

```text
{ instance, session_id, total,          // total：读的时候历史里有几条消息，seq 的坐标
  turns: [{ turn_id,                    // 这一轮的 id，和实时帧的 turn_id 相同；写下轮 id 之前的老日志为 null
            seq,                        // 开这一轮的那条人的话在历史里的位置：engine.session.history 的 offset
            at,                         // 那条话的时间（ISO 8601，引擎写的）；日志没写为 null
            prompt,                     // 人的话，合成一行，截到约一行（PROMPT_CHARS = 100 个字符，截了以 … 结尾）
            reply,                      // 这一轮最后一段文字回复，合成一行，截到约三行（REPLY_CHARS = 300）；还没说话为空
            marks? }] }                 // [{kind, label}]：这一轮上还发生了什么（流程的站开始：kind "station"）；中枢不写，没有就不出现
```

开一轮的是人打的话或跑的命令（`origin` 为 `user`、`command`）；运行中送进来的话（`steer`）、注入的上下文、工具结果不开新的一轮，
同一轮 id 的第二条人的话也不开；没有 `origin` 的老日志按文字认。中枢为最近问过的 64 个会话留着上次的结果，下次从留下的最后一轮读起
（那一轮可能长了）；那一轮开头的消息不在原处（压缩、改过消息）就从头再读。所以客户端可以每收到这个会话的 `harnix.turn_settled` 就再读一次。
正在跑的那一轮通常还不在历史里：界面把它从自己画着的对话里补上（`@harnix/core` 的 `mergeTurns`）。

一轮结束时中枢发 `harnix.turn_settled { instance, session_id, turn_id, result | error, outcome? }`。`outcome` 是这一轮
`turn_complete` 的要点：`stop_reason`、`usage`，有才有的 `structured_output`、`message`（例如 hook 截停的理由）；中枢没看到
`turn_complete` 时没有这个字段。回答的全文不在里面，读 `engine.session.history`。

---

## 22. 前端

### 22.1 包

| 包 | 用不用 |
|---|---|
| `@harnix/react` | 用：`Registry`、`useHost()`、`ChatPanel` 与对话部件、i18n（`t`、`useT`、`setLocale`）、`Glyph`、`markdown`、样式 `@harnix/react/styles.css`（只用令牌） |
| `@harnix/core` | 少用：`/client`（连接）、`/state`（会话状态机）、`/protocol`（从 Rust 生成的协议类型：`harnix.hello`、帧与行、`observability.*` 与 `packages.list` 的答，见 [testing.md](testing.md) §10；引擎的形状手写在 `engine.ts`）；**唯一认识 Agix 事件形状的解析层** |
| `@harnix/shell` | 不 import；它是外壳核心 |
| `@harnix/{system,harnix-app,settings,plugins,devices,packages,observability,flow}` | 第一方 UI 包，各是一个第一方扩展的界面；不 import，构建编进外壳 |
| `@harnix/ext-sdk`、`@harnix/ext-kit` | iframe 一侧：桥与令牌、基础部件；扩展包的界面用（[扩展包](harnix-ext-package.md)） |
| `@harnix/testkit` | 测试用（§24） |

React 19（现在 `19.3.1`）。界面包写成 `peerDependencies: { react: "^19" }`，依赖 `@harnix/react: workspace:*`。

自己建 `HarnixClient` 时：`refreshToken`（浏览器里传 `tokenFromPage()`，外壳就是这么传的）让已打开的页面在服务同端口重启、换了令牌后自己接上；
连续 10 次重连失败又取不到新令牌时状态是 `closed`、说明是 `TOKEN_STALE`。会话控制器 `send(text, attachments, { onBusy: "steer" })` 与
`ChatPanel` 的 `onBusy` 让一轮在跑时的消息在下一步送进这一轮（缺省仍排队）。

### 22.2 入口

```ts
export const extension = "watchlist";     // 可选：它是哪个后端扩展的界面
export function register(r: Registry, host: HostApi): Dispose | void
```

`r.register(point, contribution, owner)`：`owner` 写扩展 id。形状不对（缺必需的函数）或 `id` 重复：
这一条被拒，记进诊断页，别的照常。贡献渲染出错只影响这一条。

- **激活是同步的**：`hello.extensions` 带着启用的扩展，外壳在握手之后、首屏之前就对每个 `extension` 启用了的包调 `register`。之后 `harnix.ui` 推来变化：
  新启用的包补调 `register`，被停用的扩展的注册按 owner 整批撤下、外壳重画（`register` 可以返回一个 `Dispose`，批次撤下时调）；重连时 `hello` 再来一次，照同样的规则比对。
  不写 `extension` 的入口（应用自己的）总是调。
- **扩展定义的贡献点**：`r.definePoint(name, spec)`，名字必须是 `<自己的扩展 id>.<名>`；别的包 `r.register(name, c, owner)`；注册进还没定义的点先挂着，
  全部注册完还没人定义就丢掉并记进诊断。定义者用 `r.list(name)` / `useContributions(name)` 读；后端在 `UiDecl.points` 里声明同一个点，进生成的目录。
- **接不上就自己来**：`r.register(point, c, owner, { orElse: () => … })`——点在全部注册结束时还没定义就调 `orElse`。

### 22.3 十一个贡献点

内核贡献点 **11 个**，形状与每个点的字段见 [贡献点](contribution_points.md)（点的清单由代码生成）：

| 点 | 什么 |
|---|---|
| `space` | 一个 space：三栏各自的定制（`sidebar`、`center`、`conversation`、`details`）与可选的 `store`（值变了不用记得 `redraw()`） |
| `home.card` | 新对话的输入框下面的卡片 |
| `tool.row` | 一次工具调用在对话里的一行 |
| `details.panel` | 右栏的详情，由 `host.reveal(item)` 打开 |
| `flow.block` | 对话流里的块，先于内建的匹配；条目 `{ kind: "custom", type: "entry", data: { id, ns, event, payload, at } }` 用它认领 |
| `sidebar.group` | 左栏的分组 |
| `command` | `/` 与 `@` 的候选 |
| `settings.section` | 设置里的一节；`covers: ["models"]`：外壳要人去配模型时打开它 |
| `session.action` | 对一个会话、它里面一次工具调用、或人的一条话的动作：外壳在标题栏、会话菜单、工具行上问它；`at: ["arg"]` 的是工具行上的参数——第一个认领这次调用的把参数变成链接（文件工具的路径打开预览）。工具的 `target` 带调用的 `input`。`at: ["message"]` 的进人的一条话下面的消息菜单，`target` 是 `{kind: "message", session, space, turnId, text}`（外壳自己的「回到这里」在这里，[Harnix app](harnix-app.md) §5.1）。可选的 `disabled(target)` 答一句话时，这一项画成灰的、悬停说这句话 |
| `sidebar.tab` | 右栏的一种页签类型：按 `patterns` 认领 `harnix-resource://` 地址，或按 `kind` 打开的页；§22.7 |
| `shortcut` | 一条有快捷键的命令：按平台的缺省键、生效的地方、动作；设置的「快捷键」列出它、可改键；§22.8 |

`settings.section` 可以不给 `render`，只声明字段，外壳按声明画表单（类型对应控件、「恢复缺省」、有风险的值先确认、
值在读取之后被别处改过时提示「保留我的修改并复核」）：

```ts
r.register("settings.section", {
  id: "prefs", label: () => t("myext.prefs"),
  fields: [                                  // FieldDecl[]，见 @harnix/react 的 fields.tsx
    { key: "wrap", type: "toggle", label: "myext.wrap", default: false },
    { key: "size", type: "number", label: "myext.size", min: 10, max: 24, default: 14 },
  ],
  store,                                     // 可选：FieldStore {load(), save(changes, revision?)}；缺省存在这个浏览器（harnix.pref.<owner>.<key>）
}, owner);
```

`type` 是 `toggle`、`select`（`options`，`optionLabels` 给选项的文案键）、`number`（`min`、`max`）、
`text`（`suggestions`：可选的候选，不在其中的也收）、`secret`（只写不读）、`list`、
`rules`（`columns`：`{key, options?, required?, label?, check?}`，`check: "tool"` 的格是权限规则的工具，`Name` 或 `Name(内容)`，
分成工具名和括号里的路径、命令或网址两格来填）、`route`（`routes: [{provider, default_model, models}]`：值是一个 provider 的 id，
或 `{provider, model}`，引擎 `task_models` 条目的两种写法）、`json`（只显示，不写；`writable` 为空）；
`additive: true` 的字段各层并联生效（`permission_rules`）：表单改的是所选那层自己的行；
`risk: {values?, message}` 让这些值先确认。引擎设置的字段由后端 `settings` 扩展的字段表声明，`settings.describe` 发给界面
（模型名的 `suggestions`、路由的 `routes` 按这一对 (项目, 场景) 已配置的 provider 填），
`settings.write {tier, changes, revision?}` 整份校验、按版本号拒绝过期的写入（`CONFLICT`），路由到没配置的 provider 也拒绝。
provider 的 `api_key` 到界面只有 `***` 加末 4 位。

扩展定义的点（0.4.4 的第一方）：`system.console.tab`、`system.inspector.tab`，不算进「11 个」。
`space` 的分组（0.4.2 的 25 个平铺成员按画在哪分四组）：

```ts
interface SpaceContribution {
  id: string; icon: string; title(): string; order?: number; place?: "top" | "bottom"; group?: string; within?: string;
  sessions?: string;                       // 列哪个后端 space 的会话
  accept?(selection: unknown): void;       // host.navigate.open 交来的选择
  sidebar?: { render?(): ReactNode; search?(q, submit?): void; listSession?(s): boolean; badge?(): Badge; disabled?(): string | null };
  center?: { mode?: "chat" | "minimized" | "hidden"; overview?(): ReactNode; secondary?(): Secondary | null; emptyConversation?(s): ReactNode };
  conversation?: { readOnly?(s); pinned?(s); composerNotice?(s); ids?(); composer?: { steer?: boolean };
                   hideCommands?: string[]; mention?: { first?: "contributed" | "files" }; headerActions?(s);
                   newSession?: boolean; newSessionDefaults?(); placeholder?(); subtitle?(); suggestions?(); notice?() };
  details?: { tabs?: DetailTab[] | (() => DetailTab[]); tab?(): string | undefined; initial?: "open" | "closed"; width?: number; reenter?(): void };
  store?: { subscribe(fn: () => void): () => void; getSnapshot(): unknown };
}
```

每个可选函数外壳每次画时读，抛错只丢那一样、诊断页记一条。

### 22.4 `HostApi`（`useHost()`）

0.4.2 的 `NestHost`/`useNest()` 改名 `HostApi`/`useHost()`，保留它的全部成员，加了几样：

| 成员 | 说明 |
|---|---|
| `call<T>(method, params?)` | 只放行 `harnix.reachable` 里的 |
| `subscribe(topic, params \| undefined, fn)` | 返回取消函数。按 `(topic, 键排序后的 params)` 计数：同一页共用一份订阅，最后一个退订才发 `harnix.topics.unsubscribe`，后来者先拿到已有的值，重连后自动重订 |
| `session.current() / context() / list() / open(ref) / create(spec?)` | 都是前台 space 的；`create` 自动绑定前台 space |
| `session.open(ref, { switchSpace: true })` | 去会话所属的 space（按中枢账本）打开，先把它带到前台；返回的 promise 在打开后落定，没有这个会话就 reject |
| `session.open(ref, { switchSpace?, reveal: key })` / `session.reveal(key)` | 打开后等转录追上（有上限），在对话里找 `[data-reveal="<key>"]`，没有就找 id 是 `key` 的块，滚到中间、`data-revealed` 闪 2.4 秒；返回 `Promise<boolean>` |
| `session.onChange(fn)` | 前台 space 的当前会话变了，同一个值不重复；`useCurrentSession()` 是它的 hook |
| `session.isOpen(ref)` / `session.menu(ref, anchor)` | 这个页面有没有开着这个会话 / 外壳的会话菜单（自己的项加每个 `session.action` at `menu`） |
| `spaces.switch(id) / current()` | 把 space 带到前台，新 space 画出来之后落定；没有这个 space 时 reject；它的 `disabled()` 此刻给了字符串时不切、照常落定 |
| `details.open(tab?) / close() / isOpen() / tab() / onTab(fn)` | 右栏开合与选中页签（`tab` 是前台 space 的页签 id、一个 `scope: "space"` 页签类型的 `kind`，或 `"details"`）。语义同 0.4.4：它说的是页签栏里 space 那一侧 |
| `sidebar.openResource(address, { params?, kind?, revealIfOpened?, replaceTab? }) / openTab(kind, { params?, … }) / close(tabId) / active() / isExpanded() / toggleExpanded(open?)` | 右栏作为会话的停靠面（§22.7）：进右栏的每条路都走它。`openResource`/`openTab` 答页签 id，同一内容已开着就聚焦它，打开时展开右栏；没人认领的地址、没有的 `kind`、没有打开的会话都抛错 |
| `redraw()` | 让外壳重读前台 space 的函数（`tabs()`、`badge()`…），不动注册表版本 |
| `settings.open(section?)` | 打开设置面板的一节；没有这一节时打开第一节 |
| `instances.list() / sessions(instance) / tasks(instance, sessionId) / create(template, name, params?) / stop(instance)` | 答的是协议里对应方法的结果，类型取自 `@harnix/core` 的 `Methods` |
| `composer.insert(text, { session? })` | 不给 `session`：插进页上的输入框。给了：那个会话的输入框在页上就插进去，不在就替它存着，它挂上时放进去 |
| `modal.isOpen() / onChange(fn)` | 外壳自己的模态层开着没有；模态层是一个栈。开着时 Esc 只关最上面一个；对话框下面的页面是 inert 的 |
| `shell.badge(n)` / `shell.context(c \| null)` / `shell.notice(text, tone?)` | 页面标题前缀 `(n) ` / 左栏品牌下面一行应用的上下文 / 外壳横幅里的一行 |
| `events.onTurnSettled(fn)` / `events.onSessionEvent(ref, fn)` / `events.onHostEvent(fn)` | 一轮结束 / 这个页面 attach 着的会话的事件（`raw` 是 Agix 的形状、不稳定；`parsed` 是 `@harnix/core` 解析过的视图；`trust: "extension"` 的会话内容按不受信渲染）/ 宿主事件 |
| `navigate.open({ space, session?, selection? })` | 跨 space 导航：把 space 带到前台，把 `selection` 交给它的 `accept` |
| `dialogs.confirm / prompt / open / menu` | 外壳自己的层（焦点、Esc、inert）；扩展不自己造 |
| `extensions.list() / onChange(fn)` | 诊断与「关于」用；不拿它决定接不接到别人的点上（那由点有没有定义决定） |
| `defaults` | `hello.defaults`：`{ instance, space }`，外壳与扩展不写死 `"main"`、`"sessions"` |
| `ext(id).fetch(path, init?) / socket(path)` | 扩展自己的链接（`/ext/<id>/…`） |
| `reveal(item)` | 右栏打开详情 |
| `t(key, …)` | 文案 |

拿不到 Harnix 自己的连接。

### 22.4.1 外观：令牌覆盖与语言包

| 调用 | 说明 |
|---|---|
| `r.tokens({ "--accent": "#2f6fb3", … }, owner)` | 只许覆盖主题已有的令牌名（`:root` 上声明过的）；新名字、`url(` 之类的值被拒并记进诊断。后注册的赢 |
| `r.strings("zh-CN" \| "en", { "<owner>.key": "…" }, owner)` | 语言包；key 必须以 `<owner>.` 开头，别的被拒。之后 `host.t("<owner>.key")` |
| `r.icons({ name: "<path …/>" }, owner)` | 应用自己的图标，全名 `<owner>.<name>`（space 的 `icon` 写全名）。内容是 SVG 内部（不含 `<svg>`，外框由外壳加），画在 24 的网格、1.7 的描边上；只收 `path`、`circle`、`rect`、`line`、`polyline`、`polygon`、`ellipse`、`g` 与几何、描边属性；带脚本、`on*`、链接、样式、`url(` 的整个拒收、记进诊断 |

三者都不是贡献点。编进来的模块和运行期 v2 包都能用；运行期包拿到的注册表把 `owner` 钉成包名。页面标题、左栏产品名与图标来自 profile 的 `[brand]`。

### 22.4.2 外壳的框架

一个 space 就是三栏：左栏中段是它的 `sidebar`，中区默认是对话（`center`），右栏是它的 `details`。不随
space 变的只有：顶部的标识、新建对话、搜索；底部的头像（就是设置入口，外圈颜色表示连接状态；点开是
设置、主题、语言、关于、连接详情）。两边的栏都能拖宽度。外壳核心只留每个产品都要的：外框、space 切换、三栏与窄屏抽屉、对话区、设置面板的框
（通用、连接、诊断、关于）、扩展包界面的运行时；它不认识任何第一方 UI 包。内建的 space——Harnix app 的会话（按时间）、项目（树，
一次展开一个），以及系统（入口在底部）——都是各自扩展的用例（[Harnix app](harnix-app.md)、[系统 space](system-space.md)）。

### 22.5 构建

```sh
HARNIX_UI_MODULES=<各 space 的 ui/src/index.tsx，逗号分隔绝对路径> HARNIX_UI_OUT=<目录> \
  pnpm --dir harnix --filter @harnix/shell build
HARNIX_UI_DIST=<同一目录> cargo build --release -p <app>
```

`HARNIX_UI_FIRST_PARTY` 选编进来哪几个第一方 UI 包（不设是全部，空串是一个也不要）；应用自己的入口经 `HARNIX_UI_MODULES`，排在它们之后。
外壳的 Vite 配置把 `react`、`react-dom`、`@harnix/react`、`@harnix/core` 去重到外壳那一份，所以应用的
界面入口在 Harnix 的 pnpm workspace 之外也能解析到它们；**别的第三方依赖**要在应用自己的
`node_modules` 里。完整的例子是 `examples/hello-app/scripts/build.sh`。

规矩：颜色只用令牌、文案只用 key（`tests/style-lint.mjs` 与 i18n 的测试会拦）；CSP `script-src 'self'; style-src 'self'`（外加页面那段
import map 的 hash，§22.6）：没有 `eval`、远程 import、CDN、内联脚本与样式；交互元素带 `data-testid`、状态用 `data-state`，测试照这个找元素。

### 22.6 Agix 插件（运行期包）的界面模块

```toml
[[ui]]
point = "tool.row"
module = "rows.js"     # 包的 ui/ 下
api_version = 4
```

- 模块是**预编译好的 ES 模块**，不带 React：`import { jsx } from "react/jsx-runtime"`、
  `import { useHost } from "@harnix/react"`。页面的内联 import map 把 `react`、`react-dom`、
  `react-dom/client`、`react/jsx-runtime`、`@harnix/react` 指到外壳自己用的那几份，页面上只有一个 React。
- 导出 `register(registry, host)`；`registry.register(point, c)`、`registry.tokens(t)`、
  `registry.strings(locale, d)`，`owner` 一律是包名。
- import map 是页面里唯一的内联脚本，服务端出页面时按它的 `sha256` 写进 CSP；没有 `unsafe-inline`、
  `unsafe-eval`。
- `api_version` 不等于贡献点 API（§16，现在 4；没写按 v1 算）：这一条报「这一条不生效」并写明原因，包的其余部分照常。
- `[ext.plugins] ui_modules = false`：一条都不提供、不服务。
- 运行期包仍然**不能**新增 space。范例：`tests/fixtures/packages/demo-rows-v2/`。

### 22.7 右栏：停靠面、页签类型与导航控制器

右栏是**每个会话一个停靠面**：会话里打开的页签（文件预览、用量、流程进度、后台任务的输出……）记在这个会话名下，切会话就换一份，
刷新后还在。本版只有一格；布局是一棵树（`SurfaceState.layout`），分栏、浮窗、全屏以后往里加节点，存的东西不变。
space 的 `details.tabs`、`host.reveal(item)` 打开的详情、`scope: "space"` 的页签类型是同一条页签栏的另一侧——**space 那一侧**，
语义与 0.4.4 相同（`host.details.*` 管的就是它）；哪一侧在前面由最后一次操作决定：开或点会话页签，会话那一侧在前；
选 space 页签或 `reveal`，space 那一侧在前；space 那一侧什么都没有时直接显示会话那一侧。

**页签类型**是内核点 `sidebar.tab` 的一条注册：

```ts
r.register("sidebar.tab", {
  id: "observability.usage",          // 这份实现在全部注册里的身份：<owner>.<kind> 是自然的取法
  kind: "usage",                      // openTab(kind) 打开的名字；一个 kind 至多一份 extension 加一份 builtin/fallback，extension 生效，撤掉后 builtin 恢复
  scope: "session",                   // 缺省；"space" 的类型在每个 space 的页签栏里常驻一格、不可关，id 就是它的 kind
  priority: "extension",              // 缺省；外壳自己的写 "builtin"，谁都该胜过它的通用查看器写 "fallback"
  patterns: ["harnix-resource://file/**", "*.md"],   // 资源类才给：含 ":" 的匹配整个地址，不含的匹配地址路径的任意深度、忽略大小写
  canOpen: (address) => true,         // 否决一次命中
  title: (address) => "…",            // 页签上的字，每次画时读；资源页签给地址
  guide: [{ id: "usage", title: () => "用量", description: () => "…", icon: "context", order: 20 }],  // 引导页上的入口卡
  multiple: false,                    // true：每次 openTab 都开一个新页签（终端、输出流）
  keepMounted: false,                 // true：正文在切页签、切会话、收起时保留（终端的回滚、树的展开）
  render: () => <UsageTab />,         // 正文，用 useTabInfo() 知道自己是哪一页
}, owner);
```

**资源地址** `harnix-resource://<类型>/<路径>`（`resourceAddress("file", "src/a.ts")`）。谁打开它：`patterns` 命中的类型先按优先级档
（extension、builtin、fallback）排，再按命中模式的长度，再按注册顺序；`canOpen` 剔除候选；`kind` 指定时跳过认领、只问它的 `canOpen`。
页类（引导、用量、文件树）不给 `patterns`，按 `kind` 打开。

**导航控制器** `host.sidebar`：

| 调用 | 说明 |
|---|---|
| `openResource(address, { params?, kind?, revealIfOpened?, replaceTab? })` | 认领的类型打开它；答页签 id。同一 `(kind, address)` 已开着就聚焦它、递交新的 `params`、`revision` 加一；`revealIfOpened: false` 另开一个。右栏随之展开 |
| `openTab(kind, { params?, revealIfOpened?, replaceTab? })` | 按 kind 打开一页；`multiple` 的类型每次都是新页签。`scope: "space"` 的类型：选中它那一格（等于 `details.open(kind)`） |
| `close(tabId)` | 关一个页签；先跑它的关闭处理器，处理器抛错则留着 |
| `active()` | 打开的会话在前面的页签 `{ id, kind, address }`，没有就 null |
| `isExpanded()` / `toggleExpanded(open?)` | 右栏开合，等于 `details.isOpen()` / `open()`、`close()` |

没人认领的地址、没有的 `kind`、要开会话页签却没有打开的会话：抛错——那是接线错误，不是用户错误。`replaceTab` 给当前页签的 id，
新页签就落在它的位置、它关掉（引导页就是这么让位的）。

**页签域**：正文里 `const { tab } = useTabInfo()`：

| 字段 | 说明 |
|---|---|
| `tab.id`、`tab.kind`、`tab.address` | 这一页是谁；页类的 `address` 是 null |
| `tab.session` | 它在哪个会话的停靠面里（`{instance, sessionId}`）：讲「这个会话」的页读它；`scope: "space"` 的类型是 null |
| `tab.visible` | 正文在前面（没被别的页签挡着、不是后台会话的、右栏开着） |
| `tab.navigation.params`、`tab.navigation.revision` | 最后一次导航交来的参数，和导航的次数（第一次打开是 1）：带行号跳转的预览页看它变 |
| `tab.signal` | 页签关掉、会话被删时 abort |
| `tab.actions.close()`、`openResource(…)`、`openTab(…)` | 作用于**自己的会话**的控制器 |
| `tab.actions.onClose(handler)` | 关闭或被替换之前同步调；抛错则这一页留着。收起右栏、切会话、刷新都不调 |

**引导页**是外壳的 builtin（`kind: "guide"`）：会话的停靠面还空着、右栏打开、space 那一侧也空着时自动开它；恰好只有一个 `guide` 入口时
直接开那一页。页签栏上的「＋」也开它。扩展可以注册自己的 `kind: "guide"` 顶替它。

**存储**：浏览器本地 `harnix.sidebar.v1.<instance>/<session>`，读出时校验（版本、格里的页签都在、选中项在格里、计数器不回头），不合法就只清这一份；
存不进去就只留在内存。`params`、`revision`、`signal`、关闭处理器不存。会话被删，它那一份随之删掉。

本版的页签由各自的 UI 包注册：`@harnix/observability` 的上下文、用量，`@harnix/harnix-app` 的文件、预览、改动（[Harnix app](harnix-app.md) §6），`@harnix/flow` 的流程，`@harnix/system` 的输出（`kind: "output"`，`multiple` 且 `keepMounted`，地址 `harnix-resource://output/<实例>/<会话>/<任务>`，由 Bash、Monitor 工具行的 `session.action`「查看输出」打开）。
外壳只有引导页。

### 22.8 快捷键：`shortcut`

一个键是一条**命令**的：内核点 `shortcut` 的一条注册。外壳一个监听器按键找命令、跑它；设置的「快捷键」一节列出全部命令，人可以改键
（存在这个浏览器里，`harnix.shortcuts.<命令 id>`）。不要自己往 `window` 上挂 `keydown` 做快捷键：那样设置里列不出、改不了，也查不到冲突。

```ts
r.register("shortcut", {
  id: "observability.usage.open",       // 命令名，全页唯一：<owner>.<做什么>；人改的键记在它名下
  title: () => host.t("observability.openUsage"),   // 设置里列出的名字，每次画时读
  keys: "Mod+Shift+U",                  // 缺省键；或 { mac: "Ctrl+Alt+U", other: "Alt+U" }，某个平台写 null 就是那里没有；不写就是没有键，等人去设
  scope: "global",                      // 缺省；"composer"：焦点在输入框里才生效；"sidebar"：焦点在右栏里才生效
  enabled: () => true,                  // 此刻 false：这个键留给页面，像没有这条命令（抛错算 false）
  run: (ctx) => host.sidebar.openTab("usage"),     // ctx：{ space, session, scope }；抛错或 reject 进外壳的横幅
}, owner);
```

**键的写法**：修饰键加一个键，用 `+` 连：`Mod`（Mac 上是 ⌘，别处是 Ctrl）、`Ctrl`、`Alt`（⌥）、`Shift`、`Meta`（⌘）；键是字母、数字、
`/ \ , . ; ' [ ] - =`、反引号、`Enter`、`Space`、`Tab`、`Backspace`、`Delete`、`ArrowUp`…（`Up` 也行）、`Home`、`End`、`PageUp`、`PageDown`、`F1`–`F24`，
也可以直接写 `KeyboardEvent.code`（`KeyK`、`Slash`）。按物理键比对（`code`），所以 ⌥K 在 Mac 上也是 K。写错的整条被拒、记进诊断；`id` 重复的后来者被拒。

**哪条命令跑**：一个键按下时，焦点所在处（输入框、右栏）的命令先于全局的，同一处的按注册顺序取第一条；`enabled()` 为 false 的不算。
对话框开着时都不跑；焦点在文字框里时，不带 Ctrl、⌘、⌥ 的键（功能键除外）是打字，不跑。Esc 归层（对话框、菜单、抽屉），不能做命令的键。

**冲突**：两条命令同键、又在同一处生效（全局的处处生效；输入框与右栏互不相交）就是冲突：设置里两行都标出来，按注册顺序跑第一条。
人录键时撞上别的命令，提示「已被『…』占用」、不改。人录的键还要带 Ctrl、⌘ 或 ⌥（功能键除外），复制、粘贴、剪切、全选、撤销、重做与
浏览器自己的窗口、标签键（`Mod+W`、`Mod+T`、`Mod+N`…）不收。

**工具函数**（`@harnix/react`）：`parseKeys(text, platform)`、`writeKeys(chord)`、`keyLabel(chord, platform)`（`⌘K` / `Ctrl+K`，提示里显示键用它）、
`chordOf(event)`、`shortcutPlatform()`、`shortcutConflicts(bindings)`、`pickShortcut(bindings, chord, scopes)`。

外壳自己的命令：`shell.session.new`（`Mod+K`）、`shell.sidebar.toggle`（`Mod+\`）、`shell.composer.focus`（`Mod+/`）、
`shell.details.toggle`、`shell.settings.open`（后两个缺省没有键）；还有对话的轮次导航 `chat.turn.previous`（`Alt+ArrowUp`，⌥↑）、
`chat.turn.next`（`Alt+ArrowDown`，⌥↓）：`@harnix/react` 的 `registerTurnShortcuts(registry, owner)`，外壳和它自己的命令一起注册，
不用外壳的页面自己调。它们是全局的，读着对话、焦点不在输入框里时也能用；焦点在有字的文字框里时让给文字框（Mac 上 ⌥↑ / ⌥↓ 是按段移光标），
空的输入框不算。作用在焦点所在的那个对话的轮次轨上，焦点不在任何对话里时是最后打开的那个。扩展包的页面（iframe）里按下的全局键交给外壳：外壳在环境的 `keys` 里说它们现在是哪些
（[扩展包](harnix-ext-package.md)）。

### 22.9 MCP 服务器：`settings.mcp.*`

后端 `settings` 扩展替设备管引擎的 MCP 服务器。设备自己没有能力；扩展持 `engine.mcp`（可选，标准装配授予），
直接调 `engine.mcp.addServer / updateServer / auth.*` 的设备照旧被拒。答的形状由 `harnix_ext_settings::wire` 生成，
在 `@harnix/core` 里（`McpList`、`McpServer`、`McpAuthStarted`、`McpAuthDone`、`McpContext`……）。

| 方法 | 参数 | 做什么 |
|---|---|---|
| `settings.mcp.list` | `{scene?, project_root?}` | 这一对 (项目, 场景) 有的每一层（全局、场景、项目）的服务器，连停用的；带引擎说的状态（`engine.mcp.status`，只报引擎自己那一对的） |
| `settings.mcp.add` / `.update` | `{tier, name, config, scene?, project_root?}` | 加一个 / 改一个已有的。`config`：`{transport: stdio \| http \| sse, command?, args?, env?, url?, headers?}`；`env`、`headers` 的每个值是 `{var}`（引擎的环境变量）、`{secret}`（新给的值）或 `{keep: true}`（保留已写的）之一 |
| `settings.mcp.remove` | `{tier, name, …}` | 从这一层删掉，连同它的值 |
| `settings.mcp.enable` | `{tier, name, enabled, …}` | 停用：两半都挪出这一层的文件，存进扩展自己的数据（`mcp-aside.json`，0600），引擎不再读到；启用：原样放回 |
| `settings.mcp.reconnect` | `{tier, name, …}` | 让引擎重读这一对的设置，它的服务器都重连 |
| `settings.mcp.authorize` | `{name, origin?, scopes?, …}` | 发起 OAuth 登录（`engine.mcp.auth.start`），答 `{flow_id, authorize_url, redirect, callback?, expires_in}` |
| `settings.mcp.authComplete` | `{callback_url}` | 把浏览器回到的地址交给引擎（`engine.mcp.auth.complete`）；同一次登录第二次交，答第一次的结果 |
| `settings.mcp.authCancel` / `.logout` | `{flow_id}` / `{name, …}` | 放弃一次登录 / 退出登录（引擎能吊销就吊销令牌） |
| `settings.mcp.context` | `{instance?, session_id, space?}`，按会话判 | 会话下一次调模型时带的 MCP 部分，见下；要 `engine.inspect`（可选，标准装配授予） |

**写在哪一层**：项目层经引擎写（`engine.mcp.addServer / updateServer`）：引擎拒绝没受信的项目（`-32022`）、按它的 MCP 策略拒绝
（`-32020`），写了就连。全局层与场景层引擎没有写的方法，由扩展照引擎的写法写文件：`env`、`headers` 进那一层的 `settings.local.json`
（0600，不提交），其余进 `settings.json`，都在那一层目录的锁下读改写，写完让引擎 `engine.config.reload`。删、停用、启用引擎都没有方法，同样写文件。

**校验**：名字是字母、数字、`-`、`_`，字母或数字开头，至多 64 个字符，不含 `__`（工具名是 `mcp__<名字>__<工具>`）；stdio 要命令，
http、sse 要 `http(s)` 的 URL，两边的键不混用；环境变量名是 `[A-Za-z_][A-Za-z0-9_]*`，请求头名是 token、值里不能有换行。
`{var: NAME}` 写成 `${NAME}`；`{secret}` 里的 `$` 写成 `$$`，引擎不展开它。项目层的服务器读不到引擎的环境，所以那里 `{var}` 被拒。
加一个已有的名字、改一个没有的、改一个停用着的，都拒绝（`INVALID_PARAMS`，写明哪一项）。

**读回来**：值只到 `{kind: "env", var}` 或 `{kind: "secret", masked}`（`***` 加末 4 位，短于 12 个字符的只有 `***`）；`oauth` 块只有
`grant`、`client_id`、`issuer`、`scopes`。两层有同名的，低的那条带 `overridden_by`，状态只给引擎在用的那条。要 `private_key_jwt` 的带
`unsupported: ["private_key_jwt"]`：Harnix 的引擎不编 `mcp-oauth-jwt`。

**登录**：页面先开一个窗口（浏览器只在点击里让开），调 `authorize` 时带上自己的 `location.origin`。这个源是 `https`、或回环地址上的
`http` 时，引擎以 `external` 发起，回跳到扩展的路由 `/ext/settings/mcp/callback`；路由按回调的 `state` 找到这次登录、交给引擎，
留下一页（`data-harnix-mcp-auth="done" | "failed"`，不跑脚本，`Cache-Control: no-store`）。窗口能被准入（启动链接的 cookie）时路由就完成了；
带设备凭据的页面准入不了这个窗口（答 401），页面读到窗口回到了本源，把地址交给 `authComplete`。两条路谁先到谁交，后到的拿到同一个结果。
别的源（局域网上的明文 `http`）引擎不接受，退到引擎自己在 `127.0.0.1` 上的回环端口：只有引擎那台机器上的浏览器回得去。

**会话的 MCP 上下文**（右栏「上下文」页签用）：

```text
settings.mcp.context → {
  servers: [{
    name,                                   // 服务器
    tools: [{name, tokens, schema_bytes}],  // 这一次请求里模型看得到的工具（mcp__<服务器>__<工具>）；等 ToolSearch 加载的延迟工具不在内
    instructions?,                          // 服务器自己给的指令，就是模型在它的 `## <服务器>` 标题下读到的那段
    status?: McpStatus,                     // 引擎说的状态（同 settings.mcp.list）
  }],                                       // 按名字排；请求里有工具或指令的，和引擎列出的，都在
  instructions_tokens?,                     // 指令块（mcp.instructions）的估算
}
```

读自 `engine.session.inspectContext`（会话正在跑一轮时引擎答 `SESSION_BUSY`），先以设备的身份 `engine.session.get` 一次，看它能不能看这个会话。

---

## 23. 流程扩展：`harnix-ext-flow`、`@harnix/flow`

可选的第一方扩展：产品自己列进去才有（`harnix::first_party::standard_with_flow(Flow::builder()….build())`，授予 `FLOW_GRANTS`：`engine.control`、`engine.inspect`）；
Harnix 自己的二进制注册了它（`crates/cli/src/main.rs`）。
对象与写法（站、执行者、出口条件、规则、策略与权限、门禁、钩子……）见流程指南 `crates/ext/flow/docs/guide.zh-CN.md`
（系统 space 里流程的参考页就是它，`flow.guide {lang}` 也给它）。

| 构建器 | 意思 |
|---|---|
| `.packs(impl Packs)` / `.packs_arc(Arc<dyn Packs>)` / `.packs_with(\|data_dir\| …)` | 流程包：`MemPacks`（代码里给）、`DirPacks`（目录分层，`with_builtin` + `with_writable`）；`packs_with` 在启动时从扩展的数据目录建。项目自带的流程包（`<项目>/.harnix/flows/`，`ProjectPacks`）由扩展自己叠在上面，只给这个项目的任务，项目受信才读，应用不用配 |
| `.store(impl RunStore)` / `.store_with(…)` | 运行存在哪；缺省扩展数据目录下 `runs/` 的 `FileStore` |
| `.scene(SceneBase)` | 给流程用的 scene 底，包装成 `flow-<id>`（带 `flow_submit`、`flow_state`、`flow_propose`）。一个都不给时用内置的 `harnix`、`code`、`mini`（`flow-harnix`、`flow-code`、`flow-mini`） |
| `.author_scene(SceneBase)` | 流程的对话（`flow-author`）包装的 scene；缺省 `harnix` |
| `.main_scene(id)` | 运行主会话缺省的 scene（缺省第一个；不给 scene 时是 `harnix`）。流程定义里 `main: {scene: flow-code}`、站点的 `scene:` 可各自另选 |
| `.template(InstanceTemplate)` | 站可以放上去的模板（`flow.<id>`） |
| `.ensure_ms(ms)` | 站的模板实例等它 ready 的上限，缺省 60000，至少 1000 |
| `.programs(impl Programs)` / `.tools(impl Tools)` / `.gates(impl Gates)` | 应用的程序（`kind: program`）、只读工具（`tool` 门禁）、检查（`host` 门禁，名字也要进目录） |
| `.lease_ms(ms)` / `.tick_ms(ms)` | 扩展自己执行任务时持的租约（干活时续），缺省 600000、至少 1000；多久找一次过期的租约，缺省 1000、至少 50 |
| `.command_gates(bool)` | 让 `command` 门禁在本机跑命令（缺省关） |
| `.guarded_tools(…)` | 没有权限放行就拒绝的工具（缺省引擎的 shell、文件编辑、notebook、解释器） |
| `.authorizer(impl Authorizer)` | 谁能做什么（确认、认领、交付、编辑流程……），`who` 交给它解释 |
| `.space(id)` | 流程会话所在的 space，缺省 `flow` |
| `.recording(…)` / `.main_recording(…)` / `.conversation_recording(…)` | 回放时任务会话、主会话、流程会话用哪份录像（只在 `--replay-dir` 下有意义） |

主会话的 scene 需要项目时，扩展给每个运行建工作目录（扩展数据目录下 `workspaces/<run>`，也是引用 `${workspace}`），
不往里写任何东西。没写 `agent_type` 的子 Agent 站用引擎不带类型的子 Agent：跑在主会话的 scene 上、用它的工具。钩子是流程包里的文件（`hooks/`），扩展自己投递，没有 `Hooks` trait。

方法 `flow.*`（设备调用）：读——`defs`、`def`、`list`、`get`、`history`、`catalog`、`schema`、`validate`、
`parts {kind}`、`part`、`hooks`、`guide`、`sessions {run}`；写流程与件——`save`、`delete`、`part.save`、`part.delete`；
运行——`start`、`cancel`、`pause`、`resume`、`confirm`、`force`、`rework`、`retry`、`release`、`proposal`；外部任务——`claim`、
`heartbeat`、`submit`、`fail`；对话——`session {flow?}`、`focus`；试运行——`dry_run.start` / `step` / `run` / `list`。
错误码按类：没有这个运行、站、任务或流程答 `NOT_FOUND`；不合运行的状态或契约答 `INVALID_PARAMS`；租约不是你的、
版本已经往前走答 `CONFLICT`；`data.flow` 是内核自己的类别。`heartbeat` 只续租约：不进运行的历史，也不重发 `flow.runs`。
主题：`flow.runs`、`flow.run {run}`、`flow.defs`、`flow.focus {instance, session_id}`、`flow.dry_runs`。每个方法的参数与返回写在它注册时的说明里（`harnix.reachable` 列出）。

界面：`@harnix/flow` 经 `r.definitions()` 找定义了 `system.console.tab` 的 space（系统 space），把流程注册进那个点：控制台多一个「流程」页，
点一条流程，左栏变成这条流程的上下文；没有人定义这个点时（系统扩展没链进来）`orElse` 注册自己的 `flow` space。自己编外壳的应用调 `registerFlowSpace(registry, { host })`。
界面怎么走见 [系统 space](system-space.md)。

## 24. 测试：`@harnix/testkit`

| 导出 | 说明 |
|---|---|
| `startHarnix(opts) → Backend` | 起一个进程，自己的 HOME、数据根、端口。`opts.bin` 写应用二进制的绝对路径；`profile`（追加的 TOML）、`env`、`args`、`topologies`、`replay`、`replayStrict`、`seed`、`uiDir`、`scratch`、`fixedScratch` |
| `Backend` | `url`、`port`、`token`、`scratch`、`state`（`$HOME/.<product_id>`）、`project`、`pid`、`log()`、`stop()`（已弃用：发 SIGTERM、立刻删目录，不等退出）、`dispose()`（等进程退出——15 s 后 SIGKILL——再删目录；收尾用它，夹具与 `defineSuite` 已用它）、`halt(signal?)`、`restart()` |
| `connect(backend, { topology? }) → TestClient` | 缺省说这次运行的拓扑（`HARNIX_TOPOLOGY`，没设是 `single_duplex`）。`call`、`refused`（期望被拒，返回错误）、`waitFor(pred, { timeout, describe })`、`events`、`eventsOfKind`、`text`、`close` |
| `runTurn(api, session, message, { decision?, timeout? })` | 一轮：发 `message`，用 `decision`（缺省 `permit`）回答它引起的每个权限询问（子 Agent 的也算），等它结算；答 `{ settled, asks }`。缺省等 90 s |
| `provideModel(...)` | 扩展的引擎只从设置页的 provider 拿模型；要真模型的套件用它在起来之后配一个（见 [测试](testing.md)） |
| `dropSessions(api)` | 后端上每个实例、每个 space 的会话都停下、删掉。给共用一个后端的测试。从不抛错 |
| `alive(pid)` / `gone(pid, within?)` | 进程还在不在；等它在 `within`（缺省 3 s）内退出 |
| `STRICT_ROOT`、`LaunchOptions.fixedScratch` | 请求要和录下的逐字节一样时（`--replay-strict`），后端在固定的目录里跑：提示词里有项目、记忆、草稿的路径。`fixedScratch` 先清空这个目录；`record-fixture.mjs` 在 `<STRICT_ROOT>/<录像>` 里录 |
| `test`、`expect` | Playwright Test，带 `api` 夹具 |
| `inconclusive(reason)` | 环境不满足时标成「没结论」而不是通过 |
| `--carrier` | 选扩展包的载体（`wasm` / `process`），对扩展包的套件跑两遍；见 [测试](testing.md) |

用例打标签 `@feature:<功能>`、`@layer:api|ui`，覆盖表从标签生成。`replay: true` 读的是 Harnix 自己的
`tests/live/fixtures/recordings`；应用要用自己的录像，`args: ["--replay-dir", <目录>]`。

---

## 25. 应用仓库用的脚本

| 脚本 | 什么时候跑 |
|---|---|
| `harnix/scripts/sync-app-deps.mjs <app> [--check]` | 升 Harnix 之后：重写应用根 `Cargo.toml` 里带标记的依赖块；`--check` 只比对 |
| `harnix/scripts/check-pins.mjs <app> [--release]` | 构建前：`harnix/` 指向的提交在上游存在；它带的东西检出的正是它钉的那一版（忘了 `--recursive` 时它会说） |
| `harnix/scripts/check-lock.mjs <app>` | 构建后：应用的锁文件里 Harnix 的依赖是否和 Harnix 的一致 |
| `harnix/scripts/set-version.sh` | 不是给应用的：Harnix 自己发布用 |

仓库地址：`https://github.com/openatta/Harnix.git`（GitHub 上已从 `openatta/harnix` 改名，旧地址会转）。问题与需求都提到
这里的 issue（§2.1）。

---

## 26. 迁移时要知道的缺口

| 缺口 | 影响 | 什么时候补 |
|---|---|---|
| 没有 MCP 服务端、角色令牌 | 外部 Agent 要连：扩展 `start()` 里自己起监听 | 0.4.4 之后 |
| 批准原语 | 不进 Harnix：照 §10.1 用条目 + 主题 + 只许设备的方法自己做 | 不做 |
| `harnix.session.respond` 没有单独的能力 | 现状是对扩展开（flow 按策略作答），扩展包被收窄到自己的 space | 0.4.4 之后再议（如加 `sessions.answer`，只授 flow） |
| Agix 插件新增 space | Agix 插件的 `[[ui]]` 只能贡献内核点里除 `space` 之外的 | 不做：要自己的 space 的第三方代码写成扩展包（[扩展包](harnix-ext-package.md)） |
| 子 Agent 的进度 | 任务页能读 Shell / 监视器边跑边出的输出、子 Agent 的回答，但看不到子 Agent 正在第几步；要看过程走它的侧链会话 | 引擎，有需要再提 Harnix issue |
| 应用自己的 pnpm workspace 怎么摆 | 外部应用靠 Vite 去重解析 `react`、`@harnix/react`；类型检查与别的依赖要应用自己装（见迁移说明） | 消费者落地时定 |
| 桌面壳在 Windows 上没有干净的停止 | 关窗口 5 秒后直接结束子进程，扩展的 `stop` 不跑 | 有需要再做 |

## 27. 轻量客户端：`harnix-client`

应用自己的命令行、集成测试、桌面壳连自己起的那个进程用它，不链服务端（只依赖 `harnix-contract`、`serde_json`；同步、明文 HTTP，本机与局域网）：

| 调用 | 说明 |
|---|---|
| `Client::connect(url, token) -> Client` | split_streams 握手（进程令牌换凭据），版本取本 crate 编进来的 `PROTOCOL_VERSION` / `CONTRIB_API_VERSION` |
| `client.call(method, params)` / `call_within(…, wait)` | 一次 `/rpc`；缺省等 60 秒（`with_timeout` 改） |
| `client.events(Face::Session \| Face::Host, idle) -> EventStream` | SSE；`next()` 给 `StreamItem::Frame(json)` 或等满 `idle` 给 `Idle`；流断了报错 |
| `launch_link(url, token) -> String` | `POST /auth/link`，完整地址 |
| `ClientError { kind, code, message, data }` | `kind`：`Transport`、`Handshake`、`Unauthenticated`（凭据失效，重连）、`Rpc` |
| feature `tokio` | `AsyncClient`（同样的调用放在 `spawn_blocking` 里）、`launch_link_async` |

## 28. 桌面壳：`templates/desktop`

Tauri 2，两种模式：

- **本机**：壳起应用（`--host 127.0.0.1 --port <空闲> --launch-link --exit-with-stdin`），读
  `~/.<product_id>/token`，`POST /auth/link` 换一条一次性链接给窗口；关窗口时 SIGINT 停应用，壳被强杀时
  应用读到 stdin 断开自己退出。
- **远程**：窗口开一个配置好的 `https://` Harnix，配对走网页自己的流程。

壳替自己做的事（通知、角标、原生对话框）经 `harnix-client` 当子进程的客户端：`Started::client()`。

业务流量全是页面自己的 HTTP/WS：没有 command、没有 invoke handler、没有全局 Tauri 对象，
`check-no-ipc.mjs` 在 offline 层查。`--smoke` 不开窗口走一遍起、准入、停。详见它的 README。
