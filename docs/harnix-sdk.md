# 在 Harnix 上做应用：二开指南

> 读者：在 Harnix 上做产品的人（Helm、IPDrun，以及以后的应用）。
>
> **对照的是 0.4 的全部里程碑**（M0–M9，Agix 钉在 0.2.9）。示例就是真实的签名：`crates/testapp`、
> `examples/hello-app` 是能编译、有测试的完整例子，`templates/app`、`templates/desktop` 是起步用的模板。
>
> **接口清单在 [SDK 参考](harnix-sdk-reference.md)**：每个类型、方法、profile 键、命令行开关、目录、
> 线上方法与主题、贡献点的形状，以及迁移时要知道的缺口。本指南讲怎么用、为什么。Helm、IPDrun 的迁移
> 照 [Helm 迁移说明](migration-helm.md)、[IPDrun 迁移说明](migration-ipdrun.md)。
>
> 用词照 [术语](glossary.md)。为什么是这个形状见 [概念与架构](concept_and_architecture.md)。

---

## 1. Harnix 是什么

**Harnix 是 Agix 应用的公共底座。** 三层分工：

| 层 | 管什么 | 你要不要改它 |
|---|---|---|
| **Agix** | Agent 引擎：轮次循环、工具、权限、上下文、压缩、插件包 | 不改。缺能力给 Agix 提 issue |
| **Harnix** | 把引擎变成产品的那一层：会话的持有与追赶、多设备、授权、传输、外壳、space、实例管理 | 不 fork。公共改动在 `harnix/` 子仓里做、回推 |
| **你的应用** | 领域服务、数据库、产品面板、场景与原生工具、品牌 | 全是你的 |

运行时按四层组织：

```text
Harnix 进程
 ├─ 实例 system（进程内）           ← 隔离单位：数据根、设置、provider、MCP、插件、场景
 │   ├─ 会话 a                      ← 一条轮次循环，同一时刻至多一轮
 │   │   └─ 任务：子 Agent、后台 Shell、定时唤醒……
 │   └─ 会话 b
 ├─ 实例 research（进程内，你的模块按模板建的，带你的原生工具）
 └─ 实例 sandbox（daemon，本机另一个进程，配置完全独立）
```

界面是三栏：左栏是 space 入口与条目，中区是所有 space 共用的对话，右栏是当前 space 的页签。
**一个 space 有三个部分，任意组合**：

| 部分 | 你写什么 | 在哪跑 |
|---|---|---|
| 前端 | React 组件 + 注册 | 浏览器 |
| 后端 | 一个 Rust `Module` | Harnix 进程内 |
| Agent | 原生工具、场景（经模块）；或 Agix 插件包 | Agix 实例里 |

**有几件事是写死的，你改不了，也不该想改**：传输的通道语义、授权（默认拒绝）、中枢（会话的唯一
持有者，「中枢永远比任何客户端先订阅」）、装配顺序。这些正是「刷新不丢内容、多设备一致、turn 不挂
在连接上」的来源。

## 2. 什么写在应用里，什么放进 Harnix

> **只有在公开契约之上做不出来，或者做出来会破坏 Harnix 不变量的东西，才进 Harnix。**

两个应用都要，不等于要进 Harnix。先在你的应用里用模块契约做；等两份实现证明是同一个形状，再提议
收进 Harnix。

| 例子 | 放哪 | 为什么 |
|---|---|---|
| 交易、关注组、需求规格、工位、xflow | 应用 | 领域的东西 |
| sqlite 运行态、git 原语、代码扫描 | 应用 | 模块自己选库、自己管数据 |
| 批准卡（交易要人点头） | 应用 | 模块契约够用，做法见 §10.1 |
| MCP 服务端、角色令牌 | 应用（0.4 内） | 0.4 之后 Harnix 再做，见 §10.4 |
| 无头会话、tap、OnBusy | Harnix | 要和客户端共用排队与 `seq`，在外面做不出来 |
| 会话和 space 的对应关系 | Harnix | 各写一份就会串会话 |
| 一个通用的界面组件缺了个开关 | Harnix（回推） | 契约的一部分 |

## 3. 应用仓库长什么样

```text
<app>/
├── Cargo.toml              workspace 根，唯一的锁文件
├── Cargo.lock
├── agix/                   子仓 → Agix（你自己钉）
├── harnix/                 子仓 → Harnix（不要初始化它里面的 agix/）
├── app/                    bin crate：main.rs 里 App::builder()…
├── spaces/
│   └── <x>/
│       ├── backend/        Rust crate，实现 harnix::Module（可选）
│       ├── agent/          原生工具与场景，可以并进 backend（可选）
│       └── ui/             pnpm 包：React 组件 + register()（可选）
├── pnpm-workspace.yaml     packages: spaces/*/ui（不收 harnix/ui/packages/*，见下）
├── profiles/<app>.toml
└── scripts/build.sh        pnpm build → cargo build
```

**`agix/` 和 `harnix/` 平级。** 你发现 Agix 的问题，直接在自己的 `agix/` 里开分支、回推 Agix，
不用先改 Harnix。

**两个 pnpm workspace，互不相交。** `harnix/` 用它自己的（`pnpm --dir harnix install`，外壳在那里构建，Vite
把 `react`、`@harnix/react`、`@harnix/core` 去重到外壳那一份）；应用根下自己的 workspace 只收 `spaces/*/ui`
和测试包，`@harnix/react` 用 `link:../../../harnix/ui/packages/react` 引进来，`react` 的版本写成和 Harnix 一样。
退路是根 workspace 收 `harnix/ui/packages/*`、只装一次；两种都写在 [Helm 迁移说明](migration-helm.md) P5 里。

Cargo 这边要知道三件事（为什么这样做见设计 §11.2）：

1. 应用的 workspace 把 `harnix/crates/*` 收为**成员**，`exclude = ["agix", "harnix/agix", "harnix/crates/testapp"]`。
2. 根 `Cargo.toml` 里有一段带标记的区块，由 `harnix/scripts/sync-app-deps.mjs` 生成，**不要手改**。它把
   Harnix 需要的依赖（含 Agix 的入口 crate）指到以应用根为基准的路径。升级 Harnix 之后重新跑一次。
3. `[profile.release]`、`[patch]` 写在应用根，Harnix 根目录里的那些对你不生效。模板里已经带好了。

一份 profile 的样子（每一段都拒绝不认识的键：拼错了进程起不来，并说是哪一行）：

```toml
[brand]
name = "Helm"                    # 页面标题、左栏产品名
icon = "brand/helm.svg"          # 相对 profile
product_id = "helm"              # 程序自己的配置与状态放 ~/.helm（不配就是 ~/.harnix），见 §5.4

[shell]
spaces = ["sessions", "projects"]   # 留哪些内建 space：这里去掉了系统 space（产品里一般关掉调试面）

[engine]                         # system 实例
scene = "default"

[[instances]]                    # 固定的实例（M3）
id = "ci-runner"
carrier = "spawn"                # spawn | attach | in_process
[instances.engine]               # 与 [engine] 同形：scene、scenes、model……
scene = "ci"

[[instances]]
id = "shared"
carrier = "attach"               # 别人起的 daemon，只给这个 Harnix 用
[instances.attach]
socket = "/run/agix/shared.sock" # 或 tcp = "127.0.0.1:7878" 加 token = "<凭据名>"

[[instance_templates]]           # 界面可以按它建实例（M3）；profile 里的模板只能 spawn 或 attach
id = "sandbox"
carrier = "spawn"
data_root = "sandbox/{team}/{name}"   # 在 ~/.<product_id>/instances/ 下；{name} 是实例名
params = ["team"]                # 除了 name 之外放开给调用方填的参数
ui = true
max = 4
[instance_templates.engine]
scene = "general"

[modules.watchlist]              # 你的模块的配置段（M2）
db = "watchlist.sqlite"
quote_api_key = { credential = "helm/quote-api" }   # 只写引用，值由 Credentials 给（默认读 $HARNIX_CREDENTIAL_HELM_QUOTE_API）

[ui]
runtime_modules = false          # 不加载运行期包的界面模块（见 §13）
```

## 4. 起步：从模板建一个应用（M2 起）

1. 复制 `harnix/templates/app` 到新仓库；`git submodule add` 加上 `agix/` 和 `harnix/`，钉到 Harnix 当前
   钉的那个 Agix（`git -C harnix ls-tree HEAD agix` 看得到）。
2. 跑 `node harnix/scripts/sync-app-deps.mjs .`，再跑 `node harnix/scripts/check-pins.mjs .`。
3. `scripts/build.sh`，然后 `./target/release/<app> --profile profiles/<app>.toml`。
4. 打开启动链接，看到空应用：会话、项目、系统三个内建 space，头像即设置。
5. 在 `spaces/` 下加第一个 space，在 `app/src/main.rs` 里 `.module(...)` 加上它；它的界面入口
   `spaces/<x>/ui/src/index.tsx` 由模板的 `scripts/build.sh` 自动接进构建（§8.2）。
6. 在 profile 里写上 `[brand]`，`[shell] spaces` 留下你要的内建 space。

## 5. 后端：写一个模块（M2 起）

### 5.1 最小的模块

```rust
use harnix::prelude::*;
use serde::Deserialize;

#[derive(Default)]
pub struct Watchlist;

#[derive(Deserialize)]
struct Config {
    db: String,
    quote_api_key: CredentialRef,          // { credential = "helm/quote-api" }
}

struct State { db: Db, key: harnix::Secret }

#[derive(Deserialize)]
struct ListParams { filter: Option<String> }

#[harnix::async_trait]
impl Module for Watchlist {
    // 写这个模块时对着的模块 API 版本；和构建它的 harnix 不一致就编译失败
    const API_VERSION: u32 = harnix::MODULE_API_VERSION;

    fn manifest(&self) -> Manifest {
        Manifest::new("watchlist", env!("CARGO_PKG_VERSION"))
            .disclosure("读写关注组；提供「关注组」space")
    }

    fn methods(&self, m: &mut MethodRegistry) {
        // 全名 app.watchlist.list；声明给哪类主体用，没声明的一律拒绝
        m.method("list", Principal::Device)
            .describe("关注组列表")
            .schema(serde_json::json!({"type": "object"}))   // 可选，进诊断页
            .handler(list_groups);                           // 参数按类型反序列化，不符返回 INVALID_PARAMS
    }

    fn events(&self, e: &mut EventRegistry) {
        e.topic("groups");                    // 全名 app.watchlist.groups；推整值快照
        e.topic("job").param("job_id");       // 带参数：每组参数一个值
    }

    async fn start(&self, cx: ModuleContext) -> anyhow::Result<()> {
        let cfg: Config = cx.config()?;                                  // [modules.watchlist]
        let key = cx.credentials().get(&cfg.quote_api_key.credential)
            .map_err(anyhow::Error::msg)?;
        let db = Db::open(cx.data_dir().join(&cfg.db))?;                // 数据目录是你自己的
        cx.provide(State { db, key });                                   // handler 里用 cx.state::<State>()
        cx.events().publish("groups", serde_json::json!({}), Vec::<Group>::new()).await?;
        Ok(())
    }
}

async fn list_groups(cx: CallContext, p: ListParams) -> Result<Vec<Group>, RpcError> {
    cx.state::<State>()?.db.groups(p.filter)
}
```

```rust
// app/src/main.rs
fn main() -> anyhow::Result<()> {
    harnix::App::builder()
        .name("helm", env!("CARGO_PKG_VERSION"))
        .with_builtins()
        .module(watchlist::Watchlist::default())
        .module(trade::Trade::default())
        // .credentials(MyKeychain)       // 默认读环境变量
        .run()
}
```

### 5.2 trait 的全貌

| 函数 | 什么时候调 | 干什么 |
|---|---|---|
| `manifest` | 注册 | id、版本、`MODULE_API_VERSION`、披露文本 |
| `engine` | 注册 | 给 `system` 实例的原生贡献：场景、原生工具、`SessionCustomizer`、凭据源 |
| `templates` | 注册 | 你自己的实例模板（§6.2） |
| `spaces` | 注册 | 后端侧的 space 声明：会话策略、默认实例 |
| `methods` | 注册 | `app.<id>.*` 方法 |
| `events` | 注册 | 宿主主题，可带订阅参数 |
| `routes` | 注册 | `/apps/<id>/` 下的 HTTP 与 WS |
| `migrate` | 启动，`start` 之前 | 你自己的数据迁移 |
| `start` | 启动，实例建好之后 | 拿 `ModuleContext`，起你的服务 |
| `stop` | 退出（SIGINT、SIGTERM、`--exit-with-stdin` 下 stdin 关闭）或停用 | 收尾；倒序调。整个收尾（各模块的 `stop`，再到实例）最多 `harnix::SHUTDOWN_GRACE`（10 s），超了或收尾中再来一次信号，进程直接退出 |

**启动顺序**：注册 → 按顺序 `migrate` → 建实例 → 按顺序 `start` → 开始服务。

**退出**：Harnix 自己接 SIGINT 与 SIGTERM，走同一条收尾。模块不要在 `start` 里另装 SIGTERM 处理：两边都会收到信号，
模块那份会抢在 `stop` 前面把进程带走。

**失败只影响你自己**：注册冲突、`migrate` 失败、`start` 失败，都只停用这个模块和它的方法、路由、
space、实例，其余照常启动，原因写在诊断页。所以 `start` 里别吞错误：返回 `Err`，让诊断页说清楚。

**`ModuleContext` 有什么**：`id()`、`config()`、`data_dir()`、`credentials()`、`events()`、
`call(method, params)`（以 `Module(<id>)` 的身份经同一个准入点调任何方法，同样审计）、`instance(id)`
（只读的实例信息）、`listen()` / `listening()`（进程监听的地址，`start` 之后才有）、`provide()` / `state()`；M3 起还有 `sessions()`（§7）、`instances()`（§6）。
**没有**中枢的内部状态，也没有直连实例的 `EngineLink`：所有会话操作都经过中枢。

**诊断**：`harnix.diagnostics` 返回各版本号、每个模块的状态（启用 / 在哪一步停用、原因）、披露文本、
授权、方法（含主体与 schema）、主题、路由、迁移记录，以及 profile 里没有对应模块的 `[modules.<id>]`
（拼错了的段）。

**要的权限超出设备能调的**：在 manifest 里 `.grant("config.setProvider")` 声明，注册时写进授权表，
诊断页列出来。中枢独占的方法（`session.run_turn`、`session.subscribe` 等）不能授权给任何模块。

### 5.3 方法、主题、路由的规矩

- 方法名在 `app.<id>.*` 下，参数与返回带 schema（以后可以投影成 MCP 工具）。
- 主题推**整值快照**，不推增量。要按对象订阅就带参数：`app.trade.job { job_id }`。
- 主体：`Principal::Device`（一个人的设备）、`Principal::Module("id")`（某个模块调你）、
  `Principal::AnyModule`、`Principal::Kernel`。方法默认拒绝，`.allow(...)` 加更多主体。
- 主题默认给设备和模块订阅，`.only(&[...])` 收窄。
- 自有路由挂在 `/apps/<id>/` 下：`r.serve(&[Principal::Device], axum::Router)`。与界面同源，准入走
  同一套授权（带握手拿到的 `credential`：`Authorization: Bearer …` 或 `?credential=`；回环上也认
  进程令牌与启动链接的 cookie）。它**不享受**拓扑无关、`seq` 追赶这些保证，所以只在 Harnix 通道不合适
  时用（§9）。

### 5.4 数据与迁移

Harnix 只管什么时候迁、失败了怎么办，不管用什么迁。数据库、迁移工具、连接池你自己选。你的数据放在
`ctx.data_dir()` 里，和 Harnix 的账本分开。0.4 没有 `--dry-run` 和备份钩子，要的话在你的
`migrate` 里自己做。

**目录的规矩**（维护者 2026-10-26 定）：

- 程序自己的配置与状态（令牌、设备、工作区、`spaces.json`、`instances.json`、实例数据根、各模块的
  `data_dir`）放 `~/.<product_id>`，不配 `[brand]` 就是 `~/.harnix`；`HARNIX_STATE_DIR` 能挪。
  0.4.0-2026102602 及以前放在 `~/Documents/.harnix`，新版本不迁移、不读它。
- Documents 里只放项目，不放隐藏目录。
- 项目里，项目自己的数据与配置一律放 `<项目>/.agix/`，不另开 `<项目>/.<你的产品>/`。

## 6. Agent 侧

### 6.1 给 `system` 加原生工具和场景（M2 起）

```rust
fn engine(&self, e: &mut EngineContrib) {
    e.scene(research_scene());
    e.customize_session(|b, _cx| b.native_tool(QuoteTool::default()));
}
```

多个模块的 customizer 由 Harnix 按注册顺序组合。

### 6.2 给 space 一台独立、隔离的实例（M3 起）

实例只能按模板建。**模板是你在 `templates()` 里声明的**，模块和界面都按它建，Harnix 记进账本、
重启后统一重建。

```rust
fn templates(&self, t: &mut InstanceTemplates) {
    // 进程内：能带原生贡献。适合「这个 space 要一台带交易工具的引擎」
    let mut contrib = EngineContrib::new();
    contrib.scene(research_scene());
    contrib.customize_session(|b, _cx| b.native_tool(OrderTool::default()));
    t.add(InstanceTemplate::new("research", TemplateCarrier::InProcess {
        engine: EngineProfile { scene: "research".into(), ..EngineProfile::default() },
        contrib,
    }));

    // daemon：崩溃与资源也隔离；不能带原生贡献
    t.add(
        InstanceTemplate::new("sandbox", TemplateCarrier::Spawn {
            engine: EngineProfile { scene: "general".into(), ..EngineProfile::default() },
            program: None,               // None = 本二进制的 `daemon` 子命令，版本天然一致
            args: vec![],
        })
        .data_root("sandbox/{name}")
        .ui()                            // 允许界面按它建
        .max(4),
    );
}

async fn start(&self, ctx: ModuleContext) -> anyhow::Result<()> {
    // 模板的全名带模块 id 前缀：`<模块>.<模板>`。幂等：这个名字已经按这个模板建过就直接返回
    ctx.instances().ensure("helm.research", "helm-research", BTreeMap::new()).await?;
    ...
}
```

- 实例的 id 就是建它时给的 `name`，全进程唯一；数据根在 `.harnix/instances/` 下，按模板的
  `data_root` 填。名字和参数只能是字母、数字、`-`、`_`、`.`，不能以 `.` 开头。
- 模块能按**自己的**模板和 profile 的模板建；界面只能按 `ui` 模板建，只能填模板放开的参数，多填
  一个就拒绝。`ctx.instances().stop(id)` 停掉并从账本删掉（数据根留着）。
- 建过的实例记在 `.harnix/instances.json`，重启后统一重建；模板没了或模块停用了，它显示为
  `down` 并写明原因。
- `ensure` 幂等，几个任务同时 `ensure` 同一个名字也只建一台，不用自己加锁。
- 按模板建的实例有自己的数据根，默认读不到「设置 → 模型」里配的 provider（那些在 `system` 的全局层）。要它们，模板上写
  `.inherit_providers()`（profile 的模板写 `inherit_providers = true`）：建实例时和设置页每次改 provider 之后，Harnix 把 `system`
  的 provider 并进实例的全局层再让它重读，实例自己配的同名 provider 优先。
- **spawn 实例 `ensure` 之后不是马上可用**：daemon 要零点几秒才起来，这期间的会话调用答 `INSTANCE_UNAVAILABLE`
  （`the instance is starting`）。接着就要用它的，调 `ctx.instances().ensure_ready(template, name, params, timeout)`
  （设备侧：`harnix.instances.ensure { …, wait: "ready" }`）：`ready` 了才返回，`down` 立即报错并带理由，超时同样。
  已经建好、正在重启或重连的，用 `wait_ready(instance, timeout)`。会话类调用本身不等。

让一个 space 默认用它：space 声明里写 `instance = "template:research"`（第一次在这个 space 里建会话
时按模板建，之后复用），或者写一台固定实例的 id。

**选哪种载体**：

| 要什么 | 用 |
|---|---|
| 原生 Rust 工具、`SessionCustomizer` | 进程内 |
| 和 `system` 不同的 MCP、凭据、插件 | 两种都行（Agix 0.2.7 起进程内的几台互相隔离） |
| 崩溃、内存与 Harnix 隔开 | daemon |

一台 daemon 只服务一个 Harnix；不要让别的客户端也连它。spawn 的 daemon 由 Harnix 看护：挂了就重新拉起，
实例在这期间是 `degraded`，正在看的会话在它回来后 resync；Harnix 退出（哪怕被 `kill -9`）它也跟着退出。
Harnix 被杀后马上在同一目录重启时，新 daemon 等旧的那个收完尾、放开这台实例的数据根锁才建引擎，两个引擎不会同时写一份转录。
attach 的 daemon 握手时比对协议版本，不一致就 `down`，原因里写明两边的版本。

### 6.3 运行期 Agix 插件包

Agent 侧的第三方扩展照旧装到 Agix（WASM / QuickJS），能力门控与披露都在引擎那边。Harnix 只读包里
的 `[[ui]]` 段：贡献点 API 2 的界面模块是预编译好的 ES 模块，从页面的 import map 拿 `react` 和
`@harnix/react`（SDK 参考 §6.6，范例 `tests/fixtures/packages/demo-rows-v2/`）。0.4 里运行期包**不能**
新增 space，也不能往别的 space 加页签；`[ui] runtime_modules = false` 一条都不加载。

## 7. 会话

### 7.1 两类调用方，一套语义（M3 起）

界面（客户端）和你的模块（服务端）都能驱动会话，排队、中断、结算、广播完全一样。无头会话照样出现在
它所属 space 的会话列表里，界面随时可以 attach 旁观。

```rust
let s = ctx.sessions().create(SessionSpec {
    instance: Some("helm-research".into()),   // 省略就是 system
    scene: Some("research".into()),
    project_root: Some(root),
    provider: Some("anthropic".into()),
    model: Some(model),
    permission_mode: Some("default".into()),
    output_schema: None,
    ..Default::default()
}).await?;

ctx.sessions().send(&s, "总结今天的异动", OnBusy::Queue).await?;

let mut tap = ctx.sessions().tap(&s).await?;
while let Some(item) = tap.next().await {
    match item {
        TapEvent::Frame(frame) => handle(frame),          // 和客户端收到的帧一样：{method, params}
        TapEvent::Lagged { last_seq } => {                // 你读慢了
            let attached = tap.catch_up().await?;         // 缓冲里还有的补回来
            // 已经落进 transcript 的那部分按 attached["history_total"] 用 session.history 读
        }
    }
}
```

- `OnBusy`：`Queue`（排队，默认）、`Reject`、`Interrupt`（中断本轮、丢掉排着的，然后跑这条）、`Steer`
  （在下一个步骤边界送进运行中的一轮；带附件的照样排队）。
- **tap 会落后**：它是有界通道（默认 512 帧，`tap_with_capacity` 可调），中枢不会为了等你而卡住会话。
  落后了就先收到 `Lagged`，再 `catch_up()`：和刷新页面的客户端走同一条路。
- `open(instance, session_id)` 拿一个已有的会话（内存里的或只在磁盘上的）；`interrupt`、`close` 同名。
- 实例重连期间的发送进队列，实例回来、会话 resync 之后照常跑。
- 会话的全名是 `(instance, session)`，任何地方都带着实例。

### 7.2 往对话里写卡片：扩展条目（M7 起）

```rust
ctx.sessions().append_entry(&s, Entry::new("alert", json!({ "symbol": "NVDA" }))).await?;
let mine = ctx.sessions().entries(&s).await?;   // 本模块写过的，最早的在前
```

条目进会话日志（Agix 0.2.8 的 `session.appendEntry`），刷新、重放、整段分叉都看得到；**只给人看，
不给模型看**。命名空间由中枢定成 `app.<你的模块 id>`，模块写不了别人的，设备一条也写不了（人说的话
走 `harnix.send`）。写的时候正在看这个会话的人立刻收到。

前端里它是一块 `kind: "custom"`、`type: "entry"` 的块，`data` 是 `{ id, ns, event, payload, at }`
（`at` 是它前面的消息数）。用 `flow.block` 认领自己的：

```tsx
r.register('flow.block', {
  id: 'watchlist.alert',
  match: (b) => b.kind === 'custom' && b.type === 'entry' && b.data?.ns === 'app.watchlist' && b.data.event === 'alert',
  render: (b) => <AlertCard entry={b.data} />,
}, 'watchlist');
```

没人认领的条目不画。

## 8. 前端

### 8.1 包（M4 起）

| 包 | 你用不用 |
|---|---|
| `@harnix/react` | **用**。贡献注册、`useNest()`、i18n、令牌、图标、对话组件 |
| `@harnix/core` | 一般不直接用；要写不依赖 React 的逻辑时用它的 `/state`、`/client` |
| `@harnix/shell` | 不 import。它是应用入口，构建脚本把你的 `register()` 接进去 |
| `@harnix/testkit` | 写测试时用 |

你的 space 界面包只依赖 `@harnix/react`（和 React 本身）。

### 8.2 注册一个 space（M5 起；三栏的形状 M8 起）

**一个 space 就是三栏，三块都能定制，合起来才是一个 space**：

| 部分 | 字段 | 不写时 |
|---|---|---|
| 左栏中段 | `sidebar()` | 这个 space 的会话列表 |
| 中区 | `center`：`chat`（默认的对话，多数场景够用）、`minimized`（`overview()` + 一行问句）、`hidden`（只有 `overview()`，没有对话） | `chat` |
| 右栏 | `tabs`：数组，或随选中项变的函数 | 只有详情页 |

不随 space 变的只有外壳的固定部分：顶部的标识、新建对话、搜索，底部的头像（设置入口，外圈表示连接）。
Harnix 自带的三个 space——会话、项目、系统——都是用例，你的产品用 `[shell] spaces` 决定留哪些；Harnix
本身做薄，产品面都是你的 space。

```tsx
// ui/src/index.tsx —— 构建时由 virtual:harnix-modules 接进外壳（examples/hello-app 是完整的例子）
import type { NestHost, Registry } from '@harnix/react';
import { QuotesTab } from './QuotesTab';
import { Summary } from './Summary';

export function register(r: Registry, host: NestHost) {
  r.register('space', {
    id: 'watchlist',
    icon: 'globe',                              // 外壳图标集里的名字
    title: () => host.t('watchlist.title'),     // 文案走 i18n，不写死
    subtitle: () => host.t('watchlist.subtitle'),
    order: 20,
    center: 'chat',                             // chat | minimized（总览 + 问一句）| hidden（只有总览）
    tabs: [{ id: 'quotes', title: () => host.t('watchlist.tab.quotes'), render: () => <QuotesTab /> }],
    suggestions: () => ['今天的异动', '给我一份周报'],
    notice: () => 'Agent 只提案，下单要人点头',
  }, 'watchlist');
  r.register('home.card', { id: 'watchlist.summary', title: () => 'Watchlist', render: () => <Summary /> }, 'watchlist');
}
```

后端在模块里声明同一个 space 的会话策略（不声明就是默认：`system` 实例、实例的默认场景）：

```rust
fn spaces(&self, s: &mut SpaceRegistry) {
    s.add(SpaceDecl::new("watchlist").instance("template:helm.research").scene("research"));
}
```

会话在哪个 space 里建，就永远属于它（账本 `spaces.json`）。两个界面 space 可以列同一个后端 space 的
会话：项目 space 就是写了 `sessions: 'sessions'` 的会话 space 的另一种列法。外壳每个会话调用都带上前台 space，
中枢核对，不符就 `SPACE_MISMATCH`。要用别的 space 的会话，在输入框里 `@session:` 选它：发出去的是
它说过的话的一份副本。

构建：`HARNIX_UI_MODULES=<你的 index.tsx，逗号分隔> HARNIX_UI_OUT=<目录> pnpm --dir harnix --filter
@harnix/shell build`，然后 `HARNIX_UI_DIST=<同一目录> cargo build`。模板的 `scripts/build.sh` 把
`spaces/*/ui/src/index.tsx` 自动接进去；hello-app 的 `scripts/build.sh` 是写死一个入口的例子。

贡献点 v2 共 8 个：`space`、`home.card`、`tool.row`、`flow.block`、`details.panel`、`sidebar.group`、
`command`、`settings.section`。

### 8.3 `useNest()`

```tsx
import { useEffect, useState } from 'react';
import { useNest } from '@harnix/react';

export function QuotesTab() {
  const harnix = useNest();
  const [groups, setGroups] = useState<Group[]>([]);

  useEffect(() => harnix.subscribe('app.watchlist.groups', undefined, setGroups), [harnix]);

  const ask = async (g: Group) => {
    const s = await harnix.session.create({ title: g.name });   // 自动绑定前台 space
    harnix.session.open(s);
    harnix.composer.insert(`分析 ${g.name} 里的标的`);
  };
  ...
}
```

| 能力 | 说明 |
|---|---|
| `call(method, params)` | 包括 `app.<id>.*` |
| `subscribe(topic, params?, onValue)` | 宿主主题，可带参数；返回取消函数 |
| `session.*` | `current`、`context`、`list`（本 space）、`open`、`create`。会话都是 `(instance, session)` |
| `instances.*` | `list`、`sessions(instance)`、`tasks(instance, session)`、`create(template, name, params)`、`stop(id)` |
| `composer.insert(text)` | 往输入框插字（光标处，没有焦点就接在末尾） |
| `reveal(item)` | 点消息里的卡片，右栏打开它的详情（`item.kind` 由 `details.panel` 认领）。M5 有了右栏页签后加 `panelId` |
| `space(id).fetch(path, init?)` / `space(id).socket(path)` | 模块 `id` 的自有链接 `/apps/<id>/<path>`：同源，凭据由宿主带上；`path` 走不出这个前缀（M7） |
| `t()` | i18n，包括你自己的语言包（§8.6） |

**你拿不到 Harnix 自己的连接**，所以同一份代码在任何拓扑下都能跑。`call` 只放行 `harnix.reachable` 里
列出的方法，界面上的人调不了的，你的组件也调不了。**没有全局的「当前会话」**：每个
space 记自己的当前会话，`session.current()` 拿到的永远是前台 space 的。

### 8.4 对话组件

中区的对话由外壳画，你一般不用自己画。要在自己的页签里放一个对话（例如一个工位的转录），用
`@harnix/react` 的 `ChatPanel`，传 `(instance, session)`。产品特有的东西（语音、快捷回复……）在你的应用
里包一层，不要往 `ChatPanel` 里加。

### 8.5 规矩

- 颜色只用令牌，文案只用 key：两条静态检查会拦。
- CSP 是 `script-src 'self'`（外加页面自己那段 import map 的 hash）：不能 `eval`，不能远程 import，
  不能 CDN，不能内联脚本与样式。
- 交互元素带 `data-testid`、状态用 `data-state`，组件对象照这个找元素。

### 8.6 外观：品牌、配色、文案（M8 起）

应用不 fork `ui/` 就能有自己的样子：

```toml
# profile
[brand]
name = "Hello"            # 页面标题、左栏产品名
icon = "brand.svg"        # 左栏图标
product_id = "hello-app"  # ~/.hello-app
```

```tsx
// 你的界面入口 register() 里
r.tokens({ '--accent': '#2f6fb3', '--accent-soft': 'rgba(47,111,179,.12)', '--accent-line': 'rgba(47,111,179,.4)' }, 'hello-app');
r.strings('en', { 'hello-app.greeter': 'Greeter' }, 'hello-app');
r.strings('zh-CN', { 'hello-app.greeter': '问候' }, 'hello-app');
// 之后 host.t('hello-app.greeter')
```

令牌只许覆盖主题已有的名字（不另造颜色）；语言包的 key 必须以你的模块 id 开头。被拒的都记进
设置 · 诊断。`examples/hello-app` 是完整的例子。

## 9. 前后端之间用哪条通道

| 需求 | 用 |
|---|---|
| 一问一答 | `call('app.<id>.x')` |
| 状态更新 | 宿主主题，整值快照 |
| 长任务进度 | 方法返回任务 id，进度推到带参数的主题 `app.<id>.job { job_id }` |
| 大文件 | 大负载旁路 |
| 你自己的流式协议、第三方库要的 HTTP 形状 | 自有链接 `/apps/<id>/`（没有 `seq` 追赶，断线你自己处理） |

能用前四种就别用第五种。

## 10. 常见做法

### 10.1 批准卡：这个动作必须由人点头（M7 起）

引擎的权限管「这个工具能不能跑」；「这个业务动作要不要人确认」归你。用公开契约就能做，
`examples/hello-app` 的 greeter 是一份能跑的范例（没有改 Harnix 的任何 crate）：

1. **请求**：你的代码要做那件事时，建一张票（id、会话、标题、状态、截止时间），存在你的模块里。
   范例里是 `app.greeter.request_approval`。
2. **记录**：`append_entry(&session, Entry::new("approval", json!(票)))`，卡片进时间线（§7.2）。
3. **推送**：带参数的主题 `app.<id>.approvals { instance, session_id }`，推这个会话所有票的当前状态。
   卡片订阅它，所以几台设备看到的是同一个状态。
4. **回答**：方法 `app.<id>.approve { ticket, decision }`，**主体只许 `Principal::Device`**——引擎、
   别的模块、连你自己的模块都调不了（范例的 `answer_for_a_person` 演示了这个拒绝）。票的状态只从
   「待批」转一次，先答者生效，之后的回答拒绝。
5. **超时**：建票时起一个任务，到截止时间还是「待批」就按拒绝处理，`by` 记成 `timeout`。
6. **审计**：每个决定再写一条扩展条目 `approval_decided`：哪张票、结果、谁（`CallContext::device()`）。
7. **画卡**：`flow.block` 认领 `ns === "app.<id>"` 且 `event === "approval"` 的条目，按钮调 `approve`。

票存在内存里，进程重启就没了；要跨重启，把票写进你模块的数据目录（`ModuleContext::data_dir()`）。

**要知道它防不住什么**：「只许 `Device`」挡得住引擎、模块、内核替人回答，挡不住装进来的第三方界面
模块（它和页面同级）。交易类产品请在 profile 里设 `[ui] runtime_modules = false`。

### 10.2 长任务进度

方法 `app.<id>.start_job` 返回 `job_id`；进度推到 `app.<id>.job { job_id }`，每次推完整状态
（进度、阶段、结果或错误）。界面订阅这个主题。完成时想在对话里留一张卡，就写一条扩展条目。

**先推一次初始状态再把 `job_id` 交出去**：主题是整值快照，订阅得再晚也先拿到当前值，于是不会漏掉
开头。`examples/hello-app` 的 `count_to` 就是这样写的：它的进度条订阅 `app.greeter.job`，跑完往
发起它的那个会话里写一条 `job_done`，由 `flow.block` 画成卡片。

### 10.3 一个 space 要一台自己的 Agent

见 §6.2。要原生工具用进程内模板，要配置完全独立用 daemon 模板；space 声明里写
`instance = "template:<id>"`。

### 10.4 给外部 Agent 开 MCP（0.4 内）

0.4 不提供 MCP 服务端和角色令牌。在那之前：在你的模块 `start()` 里起**你自己的监听**（单独的端口），
用你自己的令牌解析。它不在 Harnix 的授权、审计与保证之内，安全由你负责。Harnix 在 0.4 之后提供这件事时，
你的方法注册表（带 schema）可以直接投影成 MCP 工具。

## 11. 构建、版本、升级

- `scripts/build.sh`：`pnpm build` → `cargo build`。没有界面产物直接 `cargo build` 会明确失败；
  `--headless` 可以不要界面。
- `<app> --version` 打印应用、Harnix、Agix、模块 API、贡献点 API、协议的版本。
- **两个钉**：`harnix/` 钉一个 Harnix 的标签（`Harnix-v<x.y.z>-<日期><序号>`），`agix/` 钉一个 Agix 的提交。
  `harnix/scripts/check-pins.mjs` 管两者的关系：
  - `agix/` 和 Harnix 钉的一样：通过。
  - 比 Harnix 钉的新，并且在 Agix 上存在：开发分支告警；发布前要么回到一样，要么先让 Harnix 升到这一版。
  - 只在本地、不在上游：失败。
- **升级 Harnix**：bump `harnix/` 子仓 → 把 `agix/` 对齐到 Harnix 新钉的版本 → `sync-app-deps` →
  `check-pins`、`check-lock` → 跑测试。0.4.z 里协议与模块 API 只做加法，升 z 不该打破你。
- **公共改动**：直接在你的 `harnix/` 子仓里开分支 → 回推 Harnix → 合并、打标签 → 你 bump 子仓。
  不许有长期补丁：子仓指针只能指向上游存在的提交。
- **桌面版**（M9 起）：复制 `harnix/templates/desktop`，改名字、图标，把你的二进制打成 sidecar。本机模式
  壳起你的应用、用启动链接让窗口进来、关窗口时停掉它；远程模式窗口开一个 `https://` 的 Harnix。业务流量
  全是页面自己的 HTTP/WS，壳不加 IPC（`check-no-ipc.mjs` 查）；唯一的例外是 macOS 无标题栏时页面请求拖动窗口。
  详见它的 README。

## 12. 测试（M1 起）

- 功能测试对着**真的二进制**跑：API 层（不开浏览器）和 Playwright 层，都用 `@harnix/testkit` 起进程。
- 模型默认走录像回放（确定、离线、不花钱）；`--live` 连真实模型并可以写回录像。
- 每个套件有自己的 HOME、数据根、端口，互不串。
- 用例打标签 `@feature:<功能>`、`@layer:api|ui`，覆盖表从标签生成。

```ts
import { test, expect } from '@harnix/testkit';

test('关注组列表 @feature:watchlist @layer:api', async ({ api }) => {
  const groups = await api.call('app.watchlist.list', {});
  expect(groups).toEqual([]);
});
```

## 13. 安全：二开要知道的

| 约束 | 是安全边界吗 | 你要注意什么 |
|---|---|---|
| 方法、路由默认拒绝，按主体放行 | 是 | 每个方法都想清楚给谁用 |
| 界面只能按 `ui = true` 的模板建实例 | 是 | 别把危险参数做成模板参数 |
| `SPACE_MISMATCH`（会话与 space 对不上就拒绝） | **不是**，只防 bug | 别拿它当权限：任何同页代码都能声称任何 space |
| 「只许 `Device`」 | 只对后端成立 | 挡不住第三方界面模块，见 §10.1 |
| 模块是内核级信任 | — | 你的模块能做的事就是这个进程能做的事；第三方代码不要做成模块 |
| 系统 space | 关掉时方法一起撤 | 产品里一般关掉；它能看到完整的系统提示词 |
| 密钥 | — | 只在 profile 里写引用，值走 `CredentialSource` |

## 14. 迁移现有代码

### 14.0 迁移分析怎么做

在自己的仓库里做分析时，输入是：本指南、[SDK 参考](harnix-sdk-reference.md)（尤其 §9 的缺口表）、能跑
的范例 `examples/hello-app`。Helm、IPDrun 已经各有一份写好的迁移说明（§14.1）；**以你仓库里现在的代码为准**，
对不上就按代码改。

逐项过一遍，每项落成「保留 / 改写 / 删掉 / 等 Harnix」：

1. **后端**：每个领域服务 → 一个 `Module`；HTTP/WS 接口 → `methods()` / `events()`，确实要自己协议的
   → `routes()`；后台任务 → `start()` 里起，状态用主题推；配置 → `[modules.<id>]`，密钥 → `CredentialRef`。
2. **Agent 侧**：原生工具、场景 → `engine()` 或进程内模板；要隔离的 → spawn 模板；无头会话、tap、
   `OnBusy` → `ctx.sessions()`（tap 要处理 `Lagged`）。
3. **界面**：每个产品面 → 一个 space（`register()`）；页面里的卡片 → 条目 + `flow.block`；
   工具结果的特殊画法 → `tool.row`；详情 → `details.panel`；设置页 → `settings.section`。Vue / 自写
   组件全部重写成 React，状态用组件内 state 或 zustand（自己依赖）。
4. **数据与目录**：现有数据在哪、M8 之后放哪（§5.4）；要不要写 `migrate`。
5. **测试**：换成 `@harnix/testkit`（API 层 + Playwright 层），用例打 `@feature` / `@layer` 标签；
   录像放在应用自己的目录。
6. **缺口**：用到的能力在 SDK 参考 §9 里的，标出等哪个里程碑，或者先在应用里怎么顶着。
7. **回推 Harnix 的候选**：两个应用都写了、形状一样、在公开契约之上做不出来的，列出来（§2）。

建议的顺序：先换布局和后端，再换界面（space、品牌），再换测试，再上桌面壳，最后清理。

### 14.0.1 通用步骤


1. 把 Agix 升到 Harnix 钉的那一版。
2. 换布局：`harnix/`、`agix/` 平级的子仓；workspace 收 `harnix/crates/*`；`sync-app-deps`、`check-pins`。
3. 后端改成模块：一个 space 的后端一个 `Module`；领域方法挪到 `methods()`。
4. 前端换成 React 包：每个 space 一个 `ui/` 包，`register()` 注册。
5. 测试换成 `@harnix/testkit`。
6. 删掉 vendor 副本和照抄的代码，`check-pins` 通过，没有补丁。

### 14.1 Helm、IPDrun

两份逐步的迁移说明，照它们各自仓库里现在的代码写的：

- [Helm 迁移说明](migration-helm.md)
- [IPDrun 迁移说明](migration-ipdrun.md)

## 15. 各能力从哪个里程碑起可用

| 能力 | 里程碑 |
|---|---|
| `App::builder()`、`Module`、`app.<id>.*`、主题、`/apps/<id>/` 路由、配置段、凭据、迁移、失败隔离、应用模板 | M2 |
| 实例、模板、daemon、`ctx.instances`、无头会话、tap、`OnBusy`、带参数的主题、实例树 | M3 |
| React 外壳、`@harnix/react`、对话组件 | M4 |
| space、贡献点 v2、宿主 API v2、`examples/hello-app` | M5 |
| 系统 space | M6 |
| 扩展条目、`space.fetch/socket`、批准卡范例 | M7 |
| 外壳框架（三栏的 space、可去掉的内建 space、头像、拖宽、系统控制台与调用的输入输出）、运行期界面模块 v2、`[ui] runtime_modules`、`[brand]`、`~/.<product_id>`、令牌覆盖与语言包、`harnix.process` | M8 |
| 桌面壳模板、`--exit-with-stdin`、迁移说明 | M9 |
