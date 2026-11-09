# Harnix SDK：用 Rust 在 Harnix 上做应用（模式 1）

> 读者：在 Harnix 上做产品的人（Helm、IPDrun、Grabix，以及以后的应用）。
>
> 这是**模式 1** 的文档：用 Rust 把 Harnix 装配成一个产品。三种模式怎么选见 [应用开发](app-development.md)；
> 往产品里装的插件 APP（模式 2 / 3）怎么写见 [插件](harnix-plugin.md)；Harnix 自带的 Harnix app 见 [Harnix app](harnix-app.md)，
> 系统 space 见 [系统 space](system-space.md)。
>
> 分两部分。**第一部分（§1–§14）是指南**：怎么做、为什么。**第二部分（§16 起）是接口清单**：每个类型、方法、
> profile 键、命令行开关、目录、线上方法与主题、贡献点的形状，以及迁移时要知道的缺口。指南讲到接口就链到第二部分，不重复。
>
> 示例就是真实的签名：`crates/testapp`、`examples/hello-app` 是能编译、有测试的完整例子，`templates/app`、
> `templates/desktop` 是起步用的模板。用词照 [术语](glossary.md)。为什么是这个形状见 [概念与架构](concept_and_architecture.md)。

---

## 1. Harnix 是什么

**Harnix 是 Agent 应用的公共底座**：一个框架，加上用这个框架做的一个 Harnix app（会话 space、项目 space）和一个
始终在的系统 space。你做的产品是框架上的另一个应用，和 Harnix app 同一种做法，只是更大。你的应用只认 Harnix：仓库里一个子仓 `harnix/`，代码里一个 crate
`harnix`（前端是 `@harnix/*` 的包）。Agent 引擎是 Harnix 带进来的，你不单独引入它、不钉它的版本，也不直接找它。

| 层 | 管什么 | 你要不要改它 |
|---|---|---|
| **Harnix**（含它带的 Agent 引擎） | 轮次循环、工具、权限、上下文、压缩、Agix 插件；会话的持有与追赶、多设备、授权、传输、外壳、space、实例管理 | 不 fork。缺什么、哪里坏了，提 Harnix 的 issue（§2.1）；公共改动在 `harnix/` 子仓里做、回推 |
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
| Agent | 原生工具、场景（经模块，类型从 `harnix::agent` 取）；或 Agix 插件（运行期包） | 实例里 |

**有几件事是写死的，你改不了，也不该想改**：传输的通道语义、授权（默认拒绝）、中枢（会话的唯一
持有者，「中枢永远比任何客户端先订阅」）、装配顺序。这些正是「刷新不丢内容、多设备一致、turn 不挂
在连接上」的来源。

要让别人往你的产品里装东西（第三方的工具、界面、后端逻辑），开模式 2：feature `apps` 加 profile 的 `[apps]`，
见 [应用开发](app-development.md) §7 与 [插件](harnix-plugin.md)。

## 2. 什么写在应用里，什么放进 Harnix

> **只有在公开契约之上做不出来，或者做出来会破坏 Harnix 不变量的东西，才进 Harnix。**

两个应用都要，不等于要进 Harnix。先在你的应用里用模块契约做；等两份实现证明是同一个形状，再提议
收进 Harnix。

| 例子 | 放哪 | 为什么 |
|---|---|---|
| 交易、关注组、需求规格、工位、xflow | 应用 | 领域的东西 |
| 通用的流程引擎：站、任务、出口条件、返工、租约 | Harnix 的可选 crate `harnix-flow`，应用注册才有 | IPDrun、XOps 两份实现是同一个形状（xops-01）；流程**定义**和领域的站仍在应用里。站的会话可落到任一实例、模板实例、space 与模型，经结构化输出交付、经权限规则限写；流程包可写、可在系统 space 里流程的对话里编写、可试运行（xops-02）。Harnix app `harnix` 自带它；`harnix` 库不带——二开应用要 `.module(Flow::builder()…build())` |
| sqlite 运行态、git 原语、代码扫描 | 应用 | 模块自己选库、自己管数据 |
| 批准卡（交易要人点头） | 应用 | 模块契约够用，做法见 §10.1 |
| MCP 服务端、角色令牌 | 应用（0.4 内） | 0.4 之后 Harnix 再做，见 §10.4 |
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
│       ├── backend/        Rust crate，实现 harnix::Module（可选）
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

1. 应用的 workspace 把 `harnix/crates/*` 收为**成员**，`exclude = ["harnix/agix", "harnix/crates/testapp"]`。
2. 根 `Cargo.toml` 里有一段带标记的区块，由 `harnix/scripts/sync-app-deps.mjs` 生成，**不要手改**。它把
   Harnix 的 crate 需要的依赖指到以应用根为基准的路径。升级 Harnix 之后重新跑一次。你自己的 crate 只写
   `harnix = { workspace = true }`，用不着这一段里的其他名字。
3. `[profile.release]`、`[patch]` 写在应用根，Harnix 根目录里的那些对你不生效。模板里已经带好了。

一份 profile 的样子（每一段都拒绝不认识的键：拼错了进程起不来，并说是哪一行）：

```toml
[brand]
name = "Helm"                    # 页面标题、左栏产品名
icon = "brand/helm.svg"          # 相对 profile
product_id = "helm"              # 程序自己的配置与状态放 ~/.helm（不配就是 ~/.harnix），见 §5.4

[shell]
spaces = ["sessions", "projects"]   # 留哪些内建 space：这里去掉了系统 space（产品里一般关掉调试面）与其中的流程
# new_session = false            # 不写是 true；false：哪个 space 都不画「新会话」、⌘K 不新建（会话由应用自己建）
# narrow_details = "beside"      # 不写是 over；beside：≤ 960 宽时右栏不盖住中区，三栏并排

[engine]                         # system 实例
scene = "default"

[[instances]]                    # 固定的实例
id = "ci-runner"
carrier = "spawn"                # spawn | attach | in_process
[instances.engine]               # 与 [engine] 同形：scene、scenes、model……
scene = "ci"

[[instances]]
id = "shared"
carrier = "attach"               # 别人起的 daemon，只给这个 Harnix 用
[instances.attach]
socket = "/run/agix/shared.sock" # 或 tcp = "127.0.0.1:7878" 加 token = "<凭据名>"

[[instance_templates]]           # 界面可以按它建实例；profile 里的模板只能 spawn 或 attach
id = "sandbox"
carrier = "spawn"
data_root = "sandbox/{team}/{name}"   # 在 ~/.<product_id>/instances/ 下；{name} 是实例名
params = ["team"]                # 除了 name 之外放开给调用方填的参数
ui = true
max = 4
[instance_templates.engine]
scene = "general"

[modules.watchlist]              # 你的模块的配置段
db = "watchlist.sqlite"
quote_api_key = { credential = "helm/quote-api" }   # 只写引用，值由 Credentials 给（默认读 $HARNIX_CREDENTIAL_HELM_QUOTE_API）

[ui]
runtime_modules = false          # 不加载运行期包的界面模块（见 §13）
```

## 4. 起步：从模板建一个应用

1. 复制 `harnix/templates/app` 到新仓库；`git submodule add` 加上 `harnix/`，再
   `git submodule update --init --recursive`。
2. 跑 `node harnix/scripts/sync-app-deps.mjs .`，再跑 `node harnix/scripts/check-pins.mjs .`。
3. `scripts/build.sh`，然后 `./target/release/<app> --profile profiles/<app>.toml`。
4. 打开启动链接，看到的是框架自带的部分：Harnix app 的会话、项目两个 space（[Harnix app](harnix-app.md)），底部的系统 space
   （[系统 space](system-space.md)），头像即设置。系统 space 里的「流程」页只在后端注册了流程模块时出现：Harnix app `harnix`
   注册了，你的应用 `.module(Flow…)` 了才有。
5. 在 `spaces/` 下加第一个 space，在 `app/src/main.rs` 里 `.module(...)` 加上它；它的界面入口
   `spaces/<x>/ui/src/index.tsx` 由模板的 `scripts/build.sh` 自动接进构建（§8.2）。
6. 在 profile 里写上 `[brand]`，`[shell] spaces` 留下你要的内建 space。

## 5. 后端：写一个模块

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
| `stop` | 退出（SIGINT、SIGTERM、`--exit-with-stdin` 下 stdin 关闭）或停用 | 收尾；倒序调。调到它时监听已经停了、连接都已关掉，不会再有调用进来。整个收尾（停监听，各模块的 `stop`，再到实例）最多 `harnix::SHUTDOWN_GRACE`（10 s），超了或收尾中再来一次信号，进程直接退出 |

**启动顺序**：注册 → 按顺序 `migrate` → 建实例 → 按顺序 `start` → 开始服务。

**退出**：Harnix 自己接 SIGINT 与 SIGTERM，走同一条收尾。模块不要在 `start` 里另装 SIGTERM 处理：两边都会收到信号，
模块那份会抢在 `stop` 前面把进程带走。

**失败只影响你自己**：注册冲突、`migrate` 失败、`start` 失败，都只停用这个模块和它的方法、路由、
space、实例，其余照常启动，原因写在诊断页。所以 `start` 里别吞错误：返回 `Err`，让诊断页说清楚。

**`ModuleContext` 有什么**：`id()`、`config()`、`data_dir()`、`credentials()`、`events()`、
`call(method, params)`（以 `Module(<id>)` 的身份经同一个准入点调任何方法，同样审计）、`instance(id)`
（只读的实例信息）、`listen()` / `listening()`（进程监听的地址，`start` 之后才有）、`provide()` / `state()`、`sessions()`（§7）、`instances()`（§6）。全表见 §17.5。
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
- 项目里，项目自己的数据与配置一律放 `<项目>/.agix/`（引擎在项目里用的目录），不另开 `<项目>/.<你的产品>/`。

## 6. Agent 侧

### 6.1 给 `system` 加原生工具和场景

原生工具、场景、prompt 块的类型都从 `harnix::agent` 取，内置场景从 `harnix::scenes` 取，不用另加依赖：

| 要写什么 | 类型 |
|---|---|
| 原生工具 | `harnix::agent::tool::{Tool, ToolContext, ToolResult, ProgressSender}`，错误 `harnix::agent::error::ToolError` |
| 场景 | `harnix::agent::interface::scene::{Scene, ScenePromptContext, TokenBudget}` |
| prompt 块 | `harnix::agent::interface::prompt::PromptBlock` |
| 包一个内置场景 | `harnix::scenes::harnix::HarnixScene`（缺省）、`harnix::scenes::code::CodeScene`、`harnix::scenes::mini::MiniScene` |

完整的例子是 `crates/testapp` 的 `NativeScene` 和 `FixedTool`。

```rust
fn engine(&self, e: &mut EngineContrib) {
    e.scene(research_scene());
    e.customize_session(|b, _cx| b.native_tool(QuoteTool::default()));
}
```

多个模块的 customizer 由 Harnix 按注册顺序组合。

#### 拦截人的消息（prompt 中间件）

人的话进一轮之前，模块可以按自己的规则放行、改写或拒收，经外壳直接发给会话的（`harnix.send`）也算。挂在会话的 `Builder`
上，类型从 `harnix`（或 `harnix_module`）取：

```rust
use harnix::{PromptCall, PromptDecision, PromptMiddleware};

struct Policy;

#[harnix::async_trait]
impl PromptMiddleware for Policy {
    async fn inspect(&self, call: &PromptCall<'_>) -> PromptDecision {
        if call.content.contains("rm -rf") {
            PromptDecision::Block { message: Some("这类话这里不收".into()) }
        } else {
            PromptDecision::Continue          // 或 PromptDecision::Rewrite(新的话)
        }
    }
}

fn engine(&self, e: &mut EngineContrib) {
    e.customize_session(|b, _cx| b.prompt_middleware(Arc::new(Policy)));
}
```

- 次序：设置层的 `UserPromptSubmit` hook 之后，再按模块注册顺序（同一个模块里按添加顺序）。
- `call.delivery` 说这句话是开一轮（`Turn`）还是送进正在跑的一轮（`Steer`，`on_busy: "steer"`）。
- 开一轮时拒收：这一轮不调模型就结束，`turn_complete` 的 `stop_reason` 是 `stopped_by_hook`、`message` 是你给的理由，
  `harnix.turn_settled` 的 `outcome` 里也有这两项。
- steer 时拒收：那句话不进对话，这一轮照常跑完；客户端收到事件 `user_message_refused { message_id, content, message }`
  （经 `harnix.event`）。没有模块拒收时它不会出现。
- 外壳（`@harnix/core` 的会话状态）对两种拒收都在流里加一行说理由（文案 `messageRefused`，可选；应用没给时只显示理由），
  steer 的那条从「待送达」里拿掉，不成一条人的话。
- 载体：进程内实例的中间件跑在本进程。spawn 实例要同样生效，把它放进模板的 `native`（§6.2），它在 daemon 进程里再建一遍。

### 6.2 给 space 一台独立、隔离的实例

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

    // daemon：崩溃与资源也隔离；要原生贡献就写 `.native(…)`（见下）
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
- **原生贡献两种载体一致**：模板上写 `.native(|e| { e.scene(…); e.customize_session(…); })`。进程内模板把它并在
  `contrib` 之后；spawn 模板（`program` 为 `None`，即本二进制的 `daemon` 子命令）的 daemon 带 `--template <模块>.<模板>` 起来，
  照装配顺序对每个应用模块调一次 `templates()`——**不调 `start`、不开任何方法**——找到这个模板，在 daemon 进程里再调一遍闭包。
  所以 `templates()` 必须没有副作用、不依赖 `start` 之后的状态；闭包跑在 daemon 进程里，**不和模块共享内存**，要模块的状态经
  Harnix 的方法或文件拿。找不到这个模板时 daemon 以 78 退出，实例 `down`，理由里写明。模板贡献的场景要在 `EngineProfile.scenes`
  里列出才在实例上启用。
- **停下时收干净**：模板上写 `.reap_on_stop()`（profile 的模板写 `reap_on_stop = true`），缺省关。spawn 实例的 daemon 自成一个
  会话（`setsid`），daemon 退出、被杀、崩溃或看护重启时，Harnix 把还留在这个会话里的进程 TERM、等 1.5 s、KILL；同时在实例的全局
  设置层打开引擎的 `sandbox.reap_process_groups`，正常退出时引擎自己先收。进程内实例只有后一半（会话结束时收它起过的进程组）。
  daemon 在自己的会话里就收不到终端的 Ctrl-C，它的退出照旧跟着 stdin 关闭走。实例视图多一项 `reap: true`（spawn 时）。
  自己 `setsid` 出去的进程收不到。
  已经建好、正在重启或重连的，用 `wait_ready(instance, timeout)`。会话类调用本身不等。

让一个 space 默认用它：space 声明里写 `instance = "template:research"`（第一次在这个 space 里建会话
时按模板建，之后复用），或者写一台固定实例的 id。

**选哪种载体**：

| 要什么 | 用 |
|---|---|
| 原生 Rust 工具、`SessionCustomizer` | 进程内 |
| 和 `system` 不同的 MCP、凭据、插件 | 两种都行（进程内的几台互相隔离） |
| 崩溃、内存与 Harnix 隔开 | daemon |

一台 daemon 只服务一个 Harnix；不要让别的客户端也连它。spawn 的 daemon 由 Harnix 看护：挂了就重新拉起，
实例在这期间是 `degraded`，正在看的会话在它回来后 resync；Harnix 退出（哪怕被 `kill -9`）它也跟着退出。
Harnix 被杀后马上在同一目录重启时，新 daemon 等旧的那个收完尾、放开这台实例的数据根锁才建引擎，两个引擎不会同时写一份转录。
attach 的 daemon 握手时比对协议版本，不一致就 `down`，原因里写明两边的版本。

### 6.3 Agix 插件（运行期包）

Agent 侧的第三方扩展可以是一个 Agix 插件，装到实例里（QuickJS 脚本、skill、agent 类型），能力与披露都在引擎那边。
Harnix 只读包里的 `[[ui]]` 段：贡献点 API 2 的界面模块是预编译好的 ES 模块，从页面的 import map 拿 `react` 和
`@harnix/react`（§22.6，范例 `tests/fixtures/packages/demo-rows-v2/`）。Agix 插件**不能**新增 space，也不能往别的
space 加页签；`[ui] runtime_modules = false` 一条都不加载。

要的是后端逻辑、自己的方法与 space、隔离的界面，那是插件 APP（模式 2），见 [插件](harnix-plugin.md)。

## 7. 会话

### 7.1 两类调用方，一套语义

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
  落后了就先收到 `Lagged`，再 `catch_up()`：和刷新页面的客户端走同一条路。逐帧读、又要每一轮的结算的，用 `catch_up_settled()`：
  tap 满时另存的结算帧按位置放回 `next()` 读的队列（缓冲里还有它那一轮的事件就排在那些事件之后，否则排在回放之前）。
- **只要一轮怎么结束的**：`tap.next_settled().await` 只等 `harnix.turn_settled`，返回它的 params：`turn_id`、`result` 或 `error`，
  以及 `outcome`（`stop_reason`、`usage`，有才有的 `structured_output` 和 hook 截停时的 `message`，取自这一轮的 `turn_complete`；
  `text` 是这一轮的回答文本、`tool_calls` 是它调过的工具 `[{id, name, is_error?}]`，由中枢逐帧收集）。
  tap 满了时结算帧另存（最多 `SETTLED_HELD` 个），落后也拿得到，不必 `catch_up`。回答超过 64 KiB 的截尾（`text_truncated: true`），全文读 `session.history`。
- **带附件发**：`send_with(&s, text, SendOptions { attachments: vec![Attachment::file("docs/a.md")], ..Default::default() })`；
  `references` 是别的会话（同输入框的 `@session`）。带附件的消息不 steer，排到下一轮。
- `open(instance, session_id)` 拿一个已有的会话（内存里的或只在磁盘上的）；`interrupt`、`close` 同名。
- 实例重连期间的发送进队列，实例回来、会话 resync 之后照常跑。
- 会话的全名是 `(instance, session)`，任何地方都带着实例。

### 7.2 往对话里写卡片：扩展条目

```rust
ctx.sessions().append_entry(&s, Entry::new("alert", json!({ "symbol": "NVDA" }))).await?;
let mine = ctx.sessions().entries(&s).await?;   // 本模块写过的，最早的在前
```

条目进会话日志（`session.appendEntry`），刷新、重放、整段分叉都看得到；**只给人看，
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

### 8.1 包

| 包 | 你用不用 |
|---|---|
| `@harnix/react` | **用**。贡献注册、`useNest()`、i18n、令牌、图标、对话组件 |
| `@harnix/core` | 一般不直接用；要写不依赖 React 的逻辑时用它的 `/state`、`/client` |
| `@harnix/shell` | 不 import。它是应用入口，构建脚本把你的 `register()` 接进去 |
| `@harnix/testkit` | 写测试时用 |

你的 space 界面包只依赖 `@harnix/react`（和 React 本身）。

### 8.2 注册一个 space

**一个 space 就是三栏，三块都能定制，合起来才是一个 space**：

| 部分 | 字段 | 不写时 |
|---|---|---|
| 左栏中段 | `sidebar()` | 这个 space 的会话列表 |
| 中区 | `center`：`chat`（默认的对话，多数场景够用）、`minimized`（`overview()` + 一行问句）、`hidden`（只有 `overview()`，没有对话） | `chat` |
| 右栏 | `tabs`：数组，或随选中项变的函数 | 只有详情页 |

不随 space 变的只有外壳的固定部分：顶部的标识、新建对话、搜索，底部的头像（设置入口，外圈表示连接）。
Harnix 自带的 space——会话、项目、系统（后端有流程模块时流程在它里面，`flow`）——都是用例，你的产品用
`[shell] spaces` 决定留哪些；Harnix 本身做薄，产品面都是你的 space。

流程是开发应用的工具链，外壳在后端有流程模块时自己把它放进系统 space（[系统 space](system-space.md) §4）。
`[shell] spaces` 里的 `flow` 是它的开关；构建器、方法与主题见 §23。

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

`space` 的全部字段（角标、灰掉、只读、置顶块、提醒条、可点的编号、二级栏……）见 §22.3。

`command` 贡献可以写 `trigger: "@"`，它的候选就进 `@` 菜单（在外壳自己的文件、`@session:` 候选之后；前台 space 写了 `mention: { first: "contributed" }` 时在前），不写是 `/`。
`complete(query, ctx)` 的 `ctx` 是 `{space, session}`（前台 space 与它打开的会话）；候选带 `run(ctx)` 时选中即执行，不插入文字。

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
    harnix.composer.insert(`分析 ${g.name} 里的标的`, { session: s });   // 进 s 的输入框，挂上了才插
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

- 颜色只用令牌，文案只用 key：两条静态检查会拦。
- CSP 是 `script-src 'self'`（外加页面自己那段 import map 的 hash）：不能 `eval`，不能远程 import，
  不能 CDN，不能内联脚本与样式。
- 交互元素带 `data-testid`、状态用 `data-state`，组件对象照这个找元素。

### 8.6 外观：品牌、配色、文案

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
设置 · 诊断。

图标：`r.icons({ ship: "<path d='M3 3h18'/>" }, 'hello-app')` 之后 space 写 `icon: 'hello-app.ship'`。内容是 SVG 的内部（不含 `<svg>`），
画在 24 的网格、1.7 的描边上，外框由外壳加；只收形状元素与几何、描边属性，带脚本、事件、链接、样式的整个拒收。`examples/hello-app` 是完整的例子。

## 9. 前后端之间用哪条通道

| 需求 | 用 |
|---|---|
| 一问一答 | `call('app.<id>.x')` |
| 状态更新 | 宿主主题，整值快照 |
| 长任务进度 | 方法返回任务 id，进度推到带参数的主题 `app.<id>.job { job_id }` |
| 大文件 | 大负载旁路 |
| 你自己的流式协议、第三方库要的 HTTP 形状 | 自有链接 `/apps/<id>/`（没有 `seq` 追赶，断线你自己处理） |

能用前四种就别用第五种。

不是页面、而是应用自己的 Rust 程序（命令行、集成测试、桌面壳）要连运行中的进程：用 `harnix-client`（§27），
握手、`/rpc`、事件流、启动链接都在里面，不链服务端，协议版本随它编进来。

## 10. 常见做法

### 10.1 批准卡：这个动作必须由人点头

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
- `<app> --version` 打印应用、Harnix、引擎、模块 API、贡献点 API、协议的版本（排查问题时贴进 issue）。
- **一个钉**：`harnix/` 钉一个 Harnix 的标签（`Harnix-v<x.y.z>-<日期><序号>`），它带的东西随它一起钉死。
  `harnix/scripts/check-pins.mjs` 查两件事：`harnix/` 指向的提交在 Harnix 上游存在；`harnix/` 带的东西检出的正是它钉的那一版
  （忘了 `--recursive` 时它会告诉你）。
- **升级 Harnix**：bump `harnix/` 子仓 → `git submodule update --init --recursive` → `sync-app-deps` →
  `check-pins`、`check-lock` → 跑测试。0.4.z 里协议与模块 API 只做加法，升 z 不该打破你。
- **从旧布局迁过来**（应用根下还有自己的 `agix/` 子仓的）：`git submodule deinit -f agix && git rm agix`，
  `git -C harnix submodule update --init agix`，再跑 `sync-app-deps`、`check-pins`；代码里的 `base::…`、`scene::…`
  换成 `harnix::agent::…`、`harnix::scenes::…`，应用 crate 的依赖里删掉 `base`、`scene` 等。迁移之前两个脚本照常工作，只多一条告警。
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
- testkit 的导出见 §24；怎么跑、怎么录像、覆盖表怎么读见 [测试](testing.md)。

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
| 模块是内核级信任 | — | 你的模块能做的事就是这个进程能做的事；第三方代码不要做成模块，做成插件 APP（[插件](harnix-plugin.md)），它在 WASM 沙箱里、按清单的能力行事 |
| 系统 space | 关掉时方法一起撤 | 产品里一般关掉；它能看到完整的系统提示词 |
| 密钥 | — | 只在 profile 里写引用，值走 `CredentialSource` |

## 14. 迁移现有代码

### 14.0 迁移分析怎么做

在自己的仓库里做分析时，输入是：本文（尤其 §26 的缺口表）、能跑
的范例 `examples/hello-app`。**以你仓库里现在的代码为准**，对不上就按代码改。

逐项过一遍，每项落成「保留 / 改写 / 删掉 / 等 Harnix」：

1. **后端**：每个领域服务 → 一个 `Module`；HTTP/WS 接口 → `methods()` / `events()`，确实要自己协议的
   → `routes()`；后台任务 → `start()` 里起，状态用主题推；配置 → `[modules.<id>]`，密钥 → `CredentialRef`。
2. **Agent 侧**：原生工具、场景 → `engine()` 或进程内模板；要隔离的 → spawn 模板；无头会话、tap、
   `OnBusy` → `ctx.sessions()`（tap 要处理 `Lagged`）。
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

1. 换布局：只留 `harnix/` 一个子仓（`git submodule update --init --recursive`）；workspace 收 `harnix/crates/*`；
   `sync-app-deps`、`check-pins`。代码里的引擎类型改从 `harnix::agent`、`harnix::scenes` 取（§6.1）。
2. 后端改成模块：一个 space 的后端一个 `Module`；领域方法挪到 `methods()`。
3. 前端换成 React 包：每个 space 一个 `ui/` 包，`register()` 注册。
4. 测试换成 `@harnix/testkit`。
5. 删掉 vendor 副本和照抄的代码，`check-pins` 通过，没有补丁。

每个要应用动手的版本另有一份迁移说明（在 Harnix 仓库的 `docs/migrations/`，只在维护者本地）：从哪一版升到哪一版要改什么。

---

# 第二部分：接口清单

## 15. 这一部分怎么读

这里列的是已经落地的接口。写法：签名照抄代码，默认值照抄代码。代码和这里对不上时以代码为准，并且这里是错的，要改。
插件 APP 自己的接口（清单、WIT、`harnix.apps.*`、`[apps]`、桥）在 [插件](harnix-plugin.md) 的第二部分；这里只列模式 1 用到的，
以及两边共用的部分。

## 16. 版本号

| 名字 | 现在 | 在哪 | 什么时候升 |
|---|---|---|---|
| Harnix 版本 | `0.4.1-2026110806` | 各 crate 的 `Cargo.toml`；`harnix::HARNIX_VERSION` | 每次发布（`scripts/set-version.sh` 连这一格一起改，`tests/crate-metadata.mjs` 查它和 crate 一致） |
| 模块 API | `1` | `harnix::MODULE_API_VERSION` | 模块契约有模块看得见的变化。**编译期**检查：`Module::API_VERSION` 不等就编译不过 |
| 贡献点 API | `2` | `harnix::CONTRIB_API_VERSION`；`harnix.hello` 里也有 | 贡献点的契约变了。2：运行期包的界面模块是预编译的 ES 模块，从页面的 import map 拿 React；**React 的大版本是它的一部分** |
| 协议 | `4` | `harnix_contract::PROTOCOL_VERSION` | 握手比对，不一致拒绝（`HANDSHAKE_REFUSED`） |
| 应用接口（插件 APP） | `1` | `harnix-app.toml` 的 `[app] harnix_api`；`harnix_app_package::HARNIX_API` | 插件 APP 的契约（WIT world 与清单）有破坏性变化。见 [插件](harnix-plugin.md) §13 |

`<app> --version` 打印上面全部，加上引擎的版本。0.4.z 内协议与模块 API 只做加法。

---

## 17. Rust：`harnix` crate

应用只依赖 `harnix` 一个 crate（它是 `harnix-app` 与 `harnix-module` 的再导出）。模块源码一般只要
`use harnix::prelude::*;`。

```text
harnix::{App, AppBuilder, ModuleStatus, Namespace, HARNIX_VERSION, SHUTDOWN_GRACE,
         codes, RpcError, Subject, SYSTEM_INSTANCE,
         AppHandle, AppHandler, AppHost, AppMethod, AppSpec, AppSummary, AppTopic, AppWithdrawn,
         Attachment, SendOptions, AttachTo, CallContext, CredentialRef, Credentials, EngineContrib, EngineProfile, Entry,
         EnvCredentials, EventRegistry, Events, Handler, InstanceHandle, InstanceTemplate,
         InstanceTemplates, Instances, Listen, Manifest, MethodRegistry, MigrateContext, MigrationReport,
         Module, ModuleContext, OnBusy, Principal, RouteRegistry, Secret, SessionRef, SessionSpec,
         Sessions, SpaceDecl, SpaceRegistry, Tap, TapEvent, TemplateCarrier,
         PromptAttachment, PromptAttachmentKind, PromptCall, PromptDecision, PromptDelivery, PromptMiddleware,
         CONTRIB_API_VERSION, MODULE_API_VERSION, SETTLED_HELD, TAP_CAPACITY,
         async_trait, axum}
```

### 17.1 `App::builder()` → `AppBuilder`

| 方法 | 说明 |
|---|---|
| `.name(name, version)` | 应用名与版本：`--version`、`--help` 用它。界面上的产品名、图标、产品 id 由 profile 的 `[brand]` 给 |
| `.with_builtins()` | 加上 Harnix 自带的模块（会话与工作区、文件、搜索、设置、Agix 插件、录像、系统 space、设备；`[apps] enabled` 时还有 `apps`）。一般都要 |
| `.module(m)` | 加一个模块；**顺序就是注册、`migrate`、`start` 的顺序**，也是 customizer 的组合顺序 |
| `.credentials(source)` | 换掉凭据源（默认 `EnvCredentials`：`helm/quote-api` → `$HARNIX_CREDENTIAL_HELM_QUOTE_API`） |
| `.profile_from_args()` / `.args(iter)` | 命令行从哪来；测试里用 `.args` |
| `.run()` | 跑起来，`anyhow::Result<()>` |

### 17.2 `trait Module`

```rust
#[harnix::async_trait]
pub trait Module: Send + Sync + 'static {
    const API_VERSION: u32;                                     // 写 harnix::MODULE_API_VERSION
    fn manifest(&self) -> Manifest;                             // 唯一必须实现的
    fn engine(&self, _: &mut EngineContrib) {}                  // 给 system 实例的原生贡献
    fn templates(&self, _: &mut InstanceTemplates) {}           // 实例模板
    fn spaces(&self, _: &mut SpaceRegistry) {}                  // 后端侧的 space 声明
    fn methods(&self, _: &mut MethodRegistry) {}                // app.<id>.*
    fn events(&self, _: &mut EventRegistry) {}                  // app.<id>.* 主题
    fn routes(&self, _: &mut RouteRegistry) {}                  // /apps/<id>/
    async fn migrate(&self, _: &MigrateContext) -> anyhow::Result<MigrationReport> { Ok(MigrationReport::none()) }
    async fn start(&self, _: ModuleContext) -> anyhow::Result<()> { Ok(()) }
    async fn stop(&self) {}
}
```

顺序：所有模块的注册函数各调一次 → 按顺序 `migrate` → 建实例 → 按顺序 `start` → 开始服务。注册冲突、
`migrate` 或 `start` 返回 `Err`：只停用这个模块和它的方法、主题、路由、space、实例，原因进诊断页。

退出：SIGINT、SIGTERM、`--exit-with-stdin` 下 stdin 关闭，都是倒序调各模块的 `stop`，再停实例、删本进程的上传目录。
整个收尾最多 `SHUTDOWN_GRACE`（10 s）；超了，或收尾中再收到一次 SIGINT / SIGTERM，记一条日志后以 1 退出。

### 17.3 声明

| 类型 | API |
|---|---|
| `Manifest` | `Manifest::new(id, version)`、`.disclosure(text)`（诊断页的披露文本）、`.grant(method)`（要设备够不着的方法，比如 `config.setProvider`；中枢独占的方法不能 grant）。id：小写字母、数字、`-`，以字母开头 |
| `MethodRegistry` | `m.method(name, Principal)` → `.allow(Principal)`、`.schema(json)`、`.describe(text)`，最后 `.handler(f)` / `.raw(f)` / `.with(Arc<dyn Handler>)`。全名 `app.<id>.<name>` |
| handler | `async fn(cx: CallContext, p: P) -> Result<R, RpcError>`，`P: DeserializeOwned`（不符返回 `INVALID_PARAMS`），`R: Serialize`。`.raw` 收发 `serde_json::Value` |
| `EventRegistry` | `e.topic(name)` → `.param(name)`（按参数分值）、`.only(&[Principal])`（默认设备与模块都能订阅）。全名 `app.<id>.<name>` |
| `Events` | `cx.events().publish(topic, params_json, value).await`：推**整值快照**；`topic` 写短名 |
| `RouteRegistry` | `r.serve(&[Principal], axum::Router)`：挂在 `/apps/<id>/` 下，同源，同一套准入 |
| `SpaceRegistry` | `s.add(SpaceDecl::new(id).instance("template:<模块>.<模板>" \| "<实例 id>").scene(scene))`。不声明：`system` 实例、实例的默认场景 |
| `InstanceTemplates` | `t.add(InstanceTemplate::new(id, carrier).data_root(pattern).param(name).ui().max(n).inherit_providers())`。`inherit_providers()`：这个模板的实例也看得到 `system` 的 provider（设置页配的）——建实例时、以及每次 `harnix.settings.setProvider` 之后，Harnix 把它们并进实例数据根的 `settings.json`（0600）再让它 `config.reload`；Harnix 写过哪些 id 记在 `.inherited-providers.json`，只动这些，实例自己配的同名 provider 不覆盖。`.reap_on_stop()`（ipdrun-07）：停下或重启时收掉会话留下的进程（spawn：daemon 自成会话，退出后 TERM→1.5 s→KILL 会话里剩下的；两种载体都在实例全局层打开 `sandbox.reap_process_groups`），实例视图带 `reap: true`。`.native(\|e\| …)`（ipdrun-10）：两种载体都有的原生贡献，spawn 时 daemon 带 `--template <全名>` 在自己进程里再建一遍（只调各模块的 `templates()`），找不到以 78 退出、实例 `down`。全名 `<模块 id>.<模板 id>`；`data_root` 默认 `{name}`，可用 `{name}` 和声明过的参数 |
| `TemplateCarrier` | `InProcess { engine, contrib }`（只有模块的模板能带原生贡献）、`Spawn { engine, program: Option<PathBuf>, args }`（`None` = 本二进制的 `daemon` 子命令）、`Attach(AttachTo { socket, tcp, token })` |
| `harnix::agent`、`harnix::scenes` | 写 Agent 侧要的类型，应用不另加依赖。`agent::tool::{Tool, ToolContext, ToolResult, ProgressSender}`、`agent::error::ToolError`（原生工具）；`agent::interface::scene::{Scene, ScenePromptContext, TokenBudget}`（场景）；`agent::interface::prompt::PromptBlock`（prompt 块）；`scenes::harnix::HarnixScene`（缺省场景）、`scenes::code::CodeScene`、`scenes::mini::MiniScene`。路径与引擎的模块一一对应 |
| `EngineContrib` | `.scene(Arc<dyn Scene>)`、`.customize_session(\|builder, cx\| …)`、`.credentials(Arc<dyn CredentialSource>)`、`.extend(other)` |
| `PromptMiddleware`、`PromptCall`、`PromptDecision`、`PromptDelivery`、`PromptAttachment`、`PromptAttachmentKind` | prompt 中间件（ipdrun-13），从 `harnix` / `harnix_module` 导出：`customize_session(\|b, _\| b.prompt_middleware(Arc::new(…)))`。设置层 hook 之后、按注册顺序跑；开一轮时 `Block` 让这一轮以 `stopped_by_hook` 结束、不调模型，steer 时 `Block` 发 `user_message_refused` 事件。消息带的附件要看就实现 `inspect_with_attachments`（缺省交给 `inspect`），附件经只读的 `PromptAttachment`（类型 `PromptAttachmentKind`）给出；steer 的消息不带附件 |
| `EngineProfile` | `scene = "harnix"`、`scenes = []`、`model = "claude-sonnet-4-6"`、`max_tokens = 2000`、`session_cap = 32`、`session_idle_timeout_secs = 3600`、`permission_prompt_timeout_secs = 300`（括号里是默认值） |

### 17.4 `Principal`

| 值 | 是谁 |
|---|---|
| `Device` | 一台配对过的设备，也就是一个人（界面上的点击） |
| `Module(String)` | 某一个模块 |
| `AnyModule` | 任何模块 |
| `Kernel` | 本二进制自己 |

方法、路由默认拒绝；声明时给一个主体，`.allow` 再加。

### 17.5 运行期：`ModuleContext`、`CallContext`

| `ModuleContext` | 说明 |
|---|---|
| `id()` | 模块 id |
| `config::<T>()` / `config_value()` | profile 的 `[modules.<id>]` 段 |
| `data_dir()` | 这个模块自己的数据目录（现在是 `<状态目录>/modules/<id>`，见 §20） |
| `credentials()` | `&dyn Credentials`：`.get(reference) -> Result<Secret, String>` |
| `events()` | `&Events` |
| `call(method, params)` | 以 `Module(<id>)` 的身份调任何方法，和设备走同一个准入点、同样审计 |
| `sessions()` | `Sessions`，见 §17.6 |
| `instances()` | `Instances`：`create(template, name, params)`、`ensure(template, name, params)`（幂等，并发调同名也只建一台，`start` 里用）、`ensure_ready(template, name, params, timeout)`（同上，等到 `ready` 再返回；`down` 或超时报 `INSTANCE_UNAVAILABLE`）、`wait_ready(instance, timeout)`、`stop(instance)`（数据根留着） |
| `instance(id)` | 只读的 `InstanceHandle` |
| `listen()` / `listening().await` | 进程监听在哪：`Listen { addrs, url }`（`addrs` 是实际绑上的地址，`--port 0` 时是系统给的端口；`url` 是 `http(s)://<第一个地址>/`，不带令牌）。模块的 `start` 在监听**之前**，所以 `start` 里 `listen()` 是 `None`；要写运行记录就 `tokio::spawn` 一个任务 `listening().await` |
| `provide(v)` / `provide_arc(arc)` / `state::<T>()` | 模块状态，handler 里用 `cx.state::<T>()` 拿 |
| `apps()` | `Option<AppHost>`：插件 APP 的宿主（§17.8）。只在 `[apps] enabled = true` 时有，而且只给代管插件 APP 的内建模块 `apps`；别的模块拿到 `None` |

`CallContext`：`subject()`、`device()`（是设备调的就给设备 id）、`module()`、`state::<T>()`。
`MigrateContext { id, data_dir, config }`；`MigrationReport::none()` / `::applied(steps)`。

**没有**中枢内部状态，**没有**直连引擎的链接：所有会话操作都经过中枢。

### 17.6 会话：`ctx.sessions()`

| 方法 | 说明 |
|---|---|
| `create(SessionSpec) -> SessionRef` | `SessionSpec { space, instance, scene, project_root, provider, model, permission_mode, permission_rules, output_schema, recording }`，全是 `Option`，`..Default::default()` |
| `open(instance, session_id)` | 已有的会话（内存里的或只在磁盘上的） |
| `send(&s, text, OnBusy)` | `OnBusy::{Queue（默认）, Reject, Interrupt, Steer}` |
| `send_with(&s, text, SendOptions)` | `SendOptions { on_busy, attachments: Vec<Attachment>, references: Vec<SessionRef> }`；`Attachment::{file(path), text(path, content), image(media_type, base64)}`；`references` 同输入框的 `@session`（那个会话的转录作为文本附件）；带附件的不 steer（ipdrun-44） |
| `interrupt(&s)` / `close(&s)` | — |
| `append_entry(&s, Entry::new(event, payload))` | 扩展条目：命名空间由中枢定为 `app.<模块 id>`；只给人看 |
| `entries(&s) -> Vec<Value>` | 本模块写过的条目，最早的在前 |
| `tap(&s)` / `tap_with_capacity(&s, n)` | 帧流，默认容量 `TAP_CAPACITY = 512`。`tap.next()` → `TapEvent::Frame(json)` 或 `TapEvent::Lagged { last_seq }`；落后了 `tap.catch_up()`。`tap.next_settled() -> Option<Value>`：跳过别的帧，只等下一个 `harnix.turn_settled` 的 params；满了时结算帧另存（最多 `SETTLED_HELD = 64`），落后也不丢。`tap.catch_up_settled()`：同 `catch_up`，另存的结算帧按位置放回 `next()` 读的队列（ipdrun-41）。`outcome` 有 `stop_reason`、`usage`、`text`（回答文本，最多 64 KiB，超了 `text_truncated`）、`tool_calls`（`[{id, name, is_error?}]`），有才有的 `structured_output`、`message` |

`SessionRef { instance, session_id }`：会话的全名永远带实例。

### 17.7 错误：`RpcError`、`harnix::codes`

`RpcError::new(code, msg)`、`::invalid_params`、`::not_found`、`::refused`。

| 码 | 名字 | 含义 |
|---|---|---|
| -32700 / -32600 / -32601 / -32602 / -32603 | JSON-RPC 标准 | 解析、请求、没有这个方法、参数、内部 |
| -31000 | `REFUSED` | 方法存在，这个主体不许调（和「没有这个方法」是两回事） |
| -31001 | `HANDSHAKE_REFUSED` | 握手没谈拢：版本、贡献点 API、拓扑 |
| -31002 | `INSTANCE_NOT_FOUND` | 没有这台实例 |
| -31003 | `SPACE_MISMATCH` | 会话不属于调用方声称的 space。**防 bug，不是权限** |
| -31004 | `INSTANCE_UNAVAILABLE` | 实例在，但现在够不着（daemon 重启中、协议版本不符） |
| -31005 | `BUSY` | 这条单双向连接上已有 `limits.max_in_flight`（16）个请求没答；这一个没执行，等一个答了再发。`data.max_in_flight` 是那个数 |

引擎的错误码原样透传（例如会话不存在是 -32000）。

### 17.8 插件 APP 的宿主：`AppHost`（`[apps] enabled`）

进程跑起来之后才进来、也能再撤走的方法、主题与 space，全名 `app.<应用 id>.*`。应用 id 是点分的小写段（`fake`、`acme.notes`）。
这是内核给内建模块 `apps` 的接口：它照每个插件 APP 的清单登记（[插件](harnix-plugin.md)）。应用的模块用不到它；测试应用经隐藏的
`AppBuilder::app_host(module)` 也给自己的一个模块（[测试](testing.md) §8）。

| 类型 | API |
|---|---|
| `AppSpec` | `AppSpec::new(id)` → `.method(name, &[Principal], \|caller: Subject, params: Value\| async { … })`（或 `.method_with(AppMethod { name, principals, description, handler: Arc<dyn AppHandler> })`）、`.topic(name, &[参数名])`（设备与模块可订；`.topic_with(AppTopic)` 自定）、`.space(SpaceDecl)`（id 是应用 id 或 `<id>.<名>`，落在已有的实例上，不能是模板） |
| `AppHost` | `cx.apps()?` 拿到；`register(spec).await -> AppHandle`（全有或全无）、`withdraw(id).await -> AppWithdrawn`（只有登记它的模块能撤）、`registered() -> Vec<AppSummary { id, by, methods, topics, spaces, since_ms }>` |
| `AppHandle` | `id()`、`publish(topic, params, value).await`（只能推自己声明的主题，参数按声明取）、`withdraw().await`；撤下之后都报错 |
| `AppHandler` | `async fn call(&self, caller: &Subject, params: Value) -> Result<Value, RpcError>`：调用已过准入 |

规则：方法与主题是授权表里的行，和编译期模块同一个准入点、同一套审计；只能在自己的命名空间里；id 与任何编译期模块（不管它有没有方法）
或已登记的应用重叠（相同，或一个是另一个的点前缀）就拒绝，名字已被占就拒绝——编译进来的优先；拒绝时什么都不写。
撤下时先出授权表（之后的调用答 `-32601`，和从没有过一样），在途的调用照常做完；主题的每个订阅者收到 `harnix.topic_ended`；space 从
`harnix.spaces.list` 消失。登记、撤下、被拒的每个名字都记审计（主体是宿主模块，带 `at_ms`；方法写 `app.<id>.<名>`、`topic:…`、`space:…`，
`decision` 是 `declare` / `withdraw` / `refuse`）。

---

## 18. profile（TOML）

**每一段都是 `deny_unknown_fields`**：写了不认识的键，进程拒绝启动并说哪一行。

```toml
[engine]                       # system 实例；字段同 EngineProfile（§17.3）
scene = "harnix"
scenes = []
model = "claude-sonnet-4-6"
max_tokens = 2000
session_cap = 32
session_idle_timeout_secs = 3600
permission_prompt_timeout_secs = 300

[transport]
topologies = ["single_duplex", "split_streams"]   # 还有 "request_only"
host = "127.0.0.1"             # 非回环要 TLS 和配对过的设备
port = 4090
launch_link = false            # true：令牌不进页面，浏览器凭一次性启动链接换 cookie
extra_origins = []             # 回环之外允许的浏览器 Origin，例如 "https://192.168.1.20:4317"

[brand]
name = "Helm"                  # 页面标题、左栏产品名；不写是 Harnix
icon = "brand/helm.svg"        # 品牌图标（SVG），相对 profile 所在目录；服务在 /brand/icon.svg
product_id = "helm"            # 程序自己的目录 ~/.helm（§20）；小写字母、数字、`-`，字母开头；不写是 harnix

[shell]
spaces = ["sessions", "projects", "system", "flow"]   # 留哪些内建 space；不写是都留，[] 是一个不留
                               # flow：流程，只在后端注册了流程模块时出现（xops-02 §7）；留着 system 时在系统 space 里
                               # （控制台的「流程」页，不列为 space），没有 system 时是一个单独的流程 space
                               # 去掉 system 时它的方法一起撤：谁也声称不了系统 space，也不能跨 space 操作
model_switch = false           # true：加 harnix.sessions.setModel，没有系统 space 也能在输入框换模型（只限调用方声称的 space 的会话）
new_session = true             # false：整个外壳不画「新会话」、⌘K 不归外壳（每个 space 都这样；space 自己的 newSession 照旧叠加）；只是界面
narrow_details = "over"        # ≤ 960 宽时打开的右栏：over（缺省）盖住中区（按最大化）；beside 三栏并排（左栏收成 rail、右栏变窄），< 720 仍按最大化

[ui]
dir = "…"                      # 从这个目录出界面（同 --ui-dir）；不写用编进二进制的
runtime_modules = true         # false：运行期包的 [[ui]] 一律不提供、不服务；交易类产品要关

[apps]
enabled = false                # true：插件 APP——加 harnix.apps.*、模块拿得到 AppHost、加 harnix.spaces 主题；不写或 false 时这些一概没有
dev = false                    # true：未签名的包也能从文件装、能启动（开发应用用）
store = "https://…/index.json" # 商店目录（https:// 或本机 file://）；不写用内置的。目录里列什么都要验签才装
[apps.trusted_publishers]      # 受信发布者：发布者 id → 签它的 ed25519 公钥（hex）；它签全的包不经商店副署也能装（trust = trusted）
# acme = ["<64 位 hex>"]
[apps.config."acme.notes"]     # 按键盖过插件 APP 自己的 [config]；整节的说明见插件文档 §19

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

[[instance_templates]]         # profile 里的模板只能 spawn 或 attach
id = "sandbox"
carrier = "spawn"
data_root = "sandbox/{team}/{name}"   # 默认 "{name}"
params = ["team"]
ui = true                      # 界面可以按它建
max = 4
inherit_providers = true       # 可选：它的实例也看得到「设置 → 模型」里配的 provider（并进实例的全局层，设置页一改就推过去）
reap_on_stop = true            # 可选：停下或重启时收掉它的会话留下的进程（ipdrun-07）
[instance_templates.engine]
scene = "general"

[modules.watchlist]            # 各模块自己的段，模块用 ctx.config() 读
db = "watchlist.sqlite"
quote_api_key = { credential = "helm/quote-api" }
```

---

## 19. 命令行与环境变量

| 开关 | 说明 |
|---|---|
| `--profile <file>` | 上面的 profile；下面的开关只在给了的时候覆盖它 |
| `--port`、`--host` | 监听 |
| `--scene`、`--scenes a,b`、`--model` | `system` 实例的默认值 |
| `--ui-dir <dir>` / `--headless` | 换界面目录 / 不出界面（纯 RPC 节点） |
| `--replay-dir <dir>` | 模型调用改成录像回放（测试用；只有运维能开，客户端要不到） |
| `--replay-strict` | 和 `--replay-dir` 一起：请求与录下的不一样就让那次调用失败（缺省只报告）；回放的会话按录像那天定日期（ipdrun-45） |
| `--tls-cert`、`--tls-key` | 绑非回环必须 |
| `--agix-dir` | 引擎目录，默认 `$AGIX_CONFIG_HOME`，再默认 `~/.agix` |
| `--data-dir` | 项目目录，默认 `$HARNIX_DATA_DIR`，再默认 `~/Documents` |
| `--launch-link` | 同 `[transport] launch_link` |
| `--exit-with-stdin`（隐藏） | stdin 关了就退出：桌面壳带走它的子进程 |
| `ui export <dir>` | 把编进去的界面写出来，给 CDN / 反代 |
| `daemon …`（隐藏） | spawn 的实例跑的就是它。Harnix 起它时带 `--instance <id>`：它在 `<数据根>/.daemon-<id>.lock` 上持独占锁（内容是 pid），先等同一台实例的旧 daemon 放手（最多 `--lock-wait` 秒，默认 10）再建引擎；等不到就非零退出，实例记为 `down`，理由「the data root is held by pid N」。模板写了 `program` 的不带这两个参数、不拿锁 |

| 环境变量 | 说明 |
|---|---|
| `AGIX_CONFIG_HOME` | 引擎目录 |
| `HARNIX_DATA_DIR` | 项目目录 |
| `HARNIX_STATE_DIR` | 程序自己的目录，单独挪（§20） |
| `HARNIX_CREDENTIAL_<名字>` | 默认凭据源读这里；名字是引用转大写、非字母数字转 `_` |
| `HARNIX_UI_MODULES`、`HARNIX_UI_OUT` | 界面构建：应用的界面入口（逗号分隔绝对路径）、输出目录（§22.5） |
| `HARNIX_UI_DIST` | `cargo build` 时嵌进二进制的界面目录 |

---

## 20. 目录

| 路径 | 是什么 |
|---|---|
| `~/.agix/` | 引擎：设置分层、transcript、记忆、技能、录像。和 `agixd` 共用 |
| `~/.<product_id>/`（不配 `[brand]` 就是 `~/.harnix/`；`HARNIX_STATE_DIR` 可挪） | 程序自己的配置与状态：`token`、`devices.json`（配对过的设备，与 `token` 一样只有属主可读；重启后仍在）、`state.json`（工作区、会话标题、偏好）、`spaces.json`、`instances.json`、`instances/<data_root>/`（按模板建的实例的数据根）、`modules/<id>/`（`ctx.data_dir()`）、`apps/<应用 id>/`（插件 APP：安装记录 `install.json`、解开的包、`data/`、`kv/`，`[apps] enabled` 时；见 [插件](harnix-plugin.md) §23）、`uploads/<pid>/`、`harnix.lock`（进程活着就持有的独占锁，内容 `{"pid", "url"}`，`url` 在监听后写上：一个状态目录只能有一个进程，第二个启动时报「`<目录>` is in use by pid N (url)」并以非零退出） |
| `~/Documents/` | 只放项目：选项目从这里开始，新建项目建在这里。Documents 里不放隐藏目录 |
| `<项目>/.agix/` | 项目自己的数据与配置**一律**放这里（引擎的项目层设置也在这），不开 `<项目>/.<产品>/` |
| `~/Documents/.harnix/`（0.4.0-2026102602 及以前） | **不迁移、不读**。升级后旧的令牌、工作区分组、实例账本都从空开始 |

模块的数据（`ctx.data_dir()`）随产品 id 走；同一台机器上装 Helm 和 IPDrun 不会抢令牌和账本。

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
| `/plugins/<包>/ui/<模块>` | 运行期包的界面模块文件（`[ui] runtime_modules = false` 时不服务） |
| `/brand/icon.svg` | `[brand] icon` |
| `/apps/<模块 id>/…` | 模块的自有路由 |

握手拿到的 `credential`：`Authorization: Bearer …` 或 `?credential=`；回环上也认进程令牌和启动链接的
cookie。一帧最大 16 MiB，一次上传最大 32 MiB。

### 21.2 方法

**权威清单在运行时**：`harnix.reachable` 列出当前主体能调的；`harnix.diagnostics` 列出全部（含主体、
schema、来自哪个模块）。下表是分组索引。

| 组 | 方法 | 谁 |
|---|---|---|
| 握手与会话驱动（中枢） | `harnix.hello`（含 `shell`：品牌、留下的内建 space、`runtime_modules`、`new_session`、`narrow_details`、目录）、`harnix.sessions`、`harnix.attach`、`harnix.detach`、`harnix.send`、`harnix.queue.remove`、`harnix.queue.edit {session_id, item_id, message}`（改排队中一条的话，位置与附件不变；已开始或已撤回的答 REFUSED）、`harnix.subscribe`、`harnix.unsubscribe` | 设备 |
| 进程（中枢） | `harnix.process`：版本、运行时长、峰值内存、`threads`（操作系统计的本进程线程数，取不到为 null）、已连接用户（设备、连接方式、User-Agent、连上的时间） | 设备 |
| 实例（中枢） | `harnix.instances.list / templates / sessions / create / ensure / stop / restart / wait`、`harnix.sessions.tasks`。`create` / `ensure` 可带 `wait: "ready"`、`timeout_ms`（缺省 60000）：等这台变成 `ready` 再答，变成 `down` 立即以 `-31004` 报错（带理由），超时同样；`wait {instance, timeout_ms?}` 是对已有实例的同一种等。并发 `ensure` 同名同模板都拿到同一台 | 设备（`create` 只认 `ui` 模板） |
| space、条目（中枢） | `harnix.spaces.list`、`harnix.entries.append`（只许模块与内核） | — |
| 确认人（中枢） | `harnix.prompts.answerer {session_id, tool_use_id}`：回答该工具权限卡的设备（`device:<id>`），没有设备回答时是 `null`；读一次即忘，最多留最近 1024 条（helm-01）。外壳回答权限卡时带 `tool_use_id`，中枢记下回答者、转给引擎前去掉 | 模块、内核 |
| 引擎直通 | `session.create / list / get / history / entries / fork / resume / respondToPrompt`、`session.interrupt / close / delete`、`scene.list / describe / activate`、`commands.list`、`mcp.status`、`plugin.list`、`daemon.status / ping / doctor` | 设备 |
| 应用层 | `harnix.contributions`、`harnix.diagnostics`、`harnix.audit`、`harnix.reachable` | 设备 |
| 自带模块（`with_builtins`） | `harnix.workspaces.*`、`harnix.prefs.set`、`harnix.projects.create`、`harnix.recentProjects`、`harnix.listDirectory`、`harnix.files`、`harnix.search`、`harnix.upload.begin`、`harnix.sessions.rename / archive`、`harnix.sessions.export {instance?, session_id, space?, format: markdown | archive | bundle, recording?}`（答 `{format, file, content}`：markdown 是可读的对话；archive 是引擎打的 zip（`encoding: "base64"`，另带 `bytes`、`sessions`、`attachments`、`recording`、`warnings`）——会话与任意深度的侧链在同一时刻的快照、消息里图片的本体、扩展条目、带版本的 `manifest.json`，`recording: true` 时带录像；bundle 是 Harnix 拼的 JSON——会话信息、全部消息、扩展条目，`recording: true` 时带录像，不含侧链与附件本体；超过一帧时拒绝）、`harnix.settings.*`（`setProvider` 的返回多一个 `reloaded_instances`：这次推到了哪些继承 provider 的实例）、`harnix.plugins.*`、`harnix.devices.*` | 设备 |
| `[shell] model_switch = true` 时 | `harnix.sessions.setModel {instance?, session_id, model, provider?, space?}`：引擎的 `session.setModel`，只作用于调用方声称的 space（不写是 `sessions`）的会话，会话属别的 space 答 `SPACE_MISMATCH`；声称 `system` 也答 `SPACE_MISMATCH`（跨 space 换模型是 `harnix.system.model` 的事） | 设备 |
| 中枢，只给内核（自带的设置模块经 grant 调） | `harnix.instances.inheritProviders`：把 `system` 的 provider 并进所有 `inherit_providers` 模板的实例并让变了的重读，答 `{reloaded: [id…]}` | 内核 |
| 系统 space（`[shell] spaces` 留着 `system` 时） | `harnix.system.context / messages.drop / messages.edit / tasks.stop / tasks.message / tasks.output / compact / clear / rename / model / calls`（`tasks.message {task_id, message}` 给在跑的子 Agent 或团队成员发话，它在下一个步骤读到；答 `{delivered, reason?}`）（`tasks.output {task_id, tail_bytes? | from_byte?}` 是任务到目前的输出：不带是最后 8 KiB；`from_byte` 从那个偏移读到末尾，应答带 `from_byte` / `next_byte` / `dropped` / `final`，跟随时下一次从 `next_byte` 接着读，`dropped > 0` 是中间已丢掉的字节数，`final` 之后不再长）；`harnix.system.goal`（会话的目标：`{goal: {objective, phase, armed, max_rounds, rounds_started, blocked_reason} | null}`）、`harnix.system.goal.set {objective, max_rounds?}`（设定；会话空闲时引擎立刻开第一轮；已有未完成的目标时拒绝）、`harnix.system.goal.update {action: pause | resume | edit | clear, objective?, max_rounds?}`（答同 `goal`）；`harnix.system.trace`（这一次运行里每次模型调用的输入与输出、它要的工具调用和结果，以及 `timing {sent_at, ttft_ms, generate_ms, tokens_per_s}`）、`harnix.system.trace.call {index}`（一次调用的全文）、`harnix.system.turns`（按轮汇总：调用数、首字时间、模型与工具时间、进出 token、按 token 计的缓存命中、上下文较上一轮的增量）、`harnix.system.usage {instance?, session_ids}`（一次至多 200 个会话，各自本次运行里每个 Agent 用了多少：`{sessions: {<id>: {recording, agents: [{session_id, parent_session_id, calls, auxiliary, turns, models, input_tokens, cache_read_tokens, cache_write_tokens, output_tokens, model_ms, errors, first_at, last_at}]}}}`，会话自己的在前，子 Agent 带父会话，整棵树都在；只读录像的调用行与结束行，录像没变就不重读）；`harnix.system.context` 带 `measured`（上一次调用 provider 计的数，估算之外） | 设备 |
| 检视（内核，模块可 grant） | `session.tasks`、`session.taskOutput`、`session.stopTask`、`session.inspectContext`、`session.dropMessages`、`session.editMessage` | 内核 |
| `[apps] enabled = true` 时（`apps` 自带模块） | `harnix.apps.list`、`.enable`、`.disable`、`.uninstall`、`.clearData`；带插件 APP 运行时的构建（cargo feature `apps`）另有 `.installFile`、`.catalog`、`.install`、`.update`。参数与答见 [插件](harnix-plugin.md) §16 | 设备 |
| 你的模块 | `app.<id>.*` | 你声明的 |
| 插件 APP | `app.<应用 id>.*` | 清单 `[[method]]` 的 `subject`（[插件](harnix-plugin.md) §18） |

明确拒绝并说明原因的（不是「没有这个方法」）：引擎里交凭据、开端点的方法；`session.run_turn`、
`session.subscribe` 这类中枢独占的方法；`plugin.install` 等包生命周期（走 `harnix.plugins.*`）；
`mcp.addServer`。

### 21.3 主题

| 主题 | 参数 | 值 |
|---|---|---|
| `harnix.instances` | — | 实例树 |
| `harnix.instances.sessions` | `instance` | 一台实例的会话：每行带 `running`（有一轮在跑）、`queued`、`tasks`（还在跑的任务数——任务可以比起它的那一轮活得久）、`waiting`（在等人：`approval` 是权限询问，`answer` 是模型的提问，`plan` 是离开计划模式要人审阅计划，没有为 null）、`started_by`（`harnix` 是模块或内核起的，`user` 是设备上的人起的；记录从这一版起，之前建的都算 `user`）、`turn_started_ms`（在跑的这一轮从什么时候开始，毫秒；没在跑为 null）、`goal`（hub 最近听到的目标——引擎推的 `goal_changed` 或经过 hub 的目标调用的答，同 `harnix.system.goal` 的 `goal`；没有目标为 null，hub 还没听到过就没有这个键） |
| `harnix.sessions.tasks` | `instance`、`session_id` | 一个会话的任务，和 `teams`：它的每个团队 hub 最近听到的 `team_progress`（`team, team_id, stage, stage_index, stage_count, status, members, failed`） |
| `harnix.sessions.status` | — | 各会话的运行状态 |
| `harnix.process` | — | 同 `harnix.process` 方法的答；有人订阅时 hub 每 5 秒重算一次推出 |
| `app.<id>.<主题>` | 你声明的 | 你推的整值快照 |
| `harnix.spaces` | — | 同 `harnix.spaces.list` 的答；插件 APP 加减 space 时推。只在 `[apps] enabled` 时有 |
| `harnix.apps`、`harnix.apps.progress` | — | 插件 APP 的快照、工具调用的进度；见 [插件](harnix-plugin.md) §17 |

插件 APP 撤下时，它的主题的每个订阅者按订阅的参数组各收到一次 `harnix.topic_ended { topic, params, reason }`，之后不再推；
再订答 `-32601`，等它重新登记。

会话事件不是主题：`harnix.attach` 之后经 `harnix.event` 帧来，带 `seq`，断线按 `seq` 追赶。
条目以 `harnix.event { event: { kind: "extension_entry", entry } }` 推给正在看的人。

一轮结束时中枢发 `harnix.turn_settled { instance, session_id, turn_id, result | error, outcome? }`。`outcome` 是这一轮
`turn_complete` 的要点：`stop_reason`、`usage`，有才有的 `structured_output`、`message`（例如 hook 截停的理由）；中枢没看到
`turn_complete` 时没有这个字段。回答的全文不在里面，读 `session.history`。

---

## 22. 前端

### 22.1 包

| 包 | 用不用 |
|---|---|
| `@harnix/react` | 用：`Registry`、`useNest()`、`ChatPanel` 与对话部件、i18n（`t`、`useT`、`setLocale`）、`Icon`、`markdown`、样式 `@harnix/react/styles.css`（只用令牌） |
| `@harnix/core` | 少用：`/client`（连接）、`/state`（会话状态机）、`/protocol`（帧与生成的契约类型） |
| `@harnix/shell` | 不 import；它是应用入口 |
| `@harnix/testkit` | 测试用（§24） |

React 19（现在 `19.3.1`）。界面包写成 `peerDependencies: { react: "^19" }`，依赖 `@harnix/react: workspace:*`。

自己建 `HarnixClient` 时：`refreshToken`（浏览器里传 `tokenFromPage()`，外壳就是这么传的）让已打开的页面在服务同端口重启、换了令牌后自己接上；
连续 10 次重连失败又取不到新令牌时状态是 `closed`、说明是 `TOKEN_STALE`。会话控制器 `send(text, attachments, { onBusy: "steer" })` 与
`ChatPanel` 的 `onBusy` 让一轮在跑时的消息在下一步送进这一轮（缺省仍排队）。

### 22.2 入口

```ts
export function register(r: Registry, host: NestHost): void
```

`r.register(point, contribution, owner)`：`owner` 写模块 id。形状不对（缺必需的函数）或 `id` 重复：
这一条被拒，记进诊断页，别的照常。贡献渲染出错只影响这一条。

### 22.3 八个贡献点

| 点 | 形状（必需的加粗） |
|---|---|
| `space` | **`id`**、**`icon`**、**`title()`**、`order`、`place: "top" \| "bottom"`（默认 top；bottom 列在左栏底部、系统之上，给全局而非业务面的入口）、`group`（顶部列表里的组名：按 `order` 相邻的两个组不同就在中间画分隔线，不写算 ""；不重排）、`within`（另一个 space 的 id：这个 space 显示在它里面——不列在左栏，由代码 `host.spaces.switch` 进入〔如从那个 space 的页面〕，在前台时点亮那个 space 的入口；系统 space 里的流程就是）、`details: "open" \| "closed"`（首次进入时右栏开合，默认 closed；之后每个 space 各记各的）、`center: "chat" \| "minimized" \| "hidden"`（默认 chat）、`sidebar()`（左栏中段）、`overview()`（中区不是对话时）、`tabs: [{ id, title(), render(), align? }]` 或返回它的函数（随选中项变；`align: "end"` 的页签排在条的右端，给不常用的）、`sessions`（列哪个后端 space 的会话，默认自己的 id）、`subtitle()`、`suggestions()`、`notice()`、`placeholder()`；可选的 `badge()`（左栏角标）、`disabled()`（返回一句话 ⇒ 入口灰掉、点不动、悬停是这句话，`spaces.switch` 到它不切；落地时存着的灰着就落到第一个没灰的；前台的后来灰了不自动换走）、`readOnly(session)`、`pinned(session)`、`composerNotice(session)`、`ids()`、`composer: { steer }`、`hideCommands: string[]`、`mention: { first: "contributed" \| "files" }`（`@` 菜单里谁的候选在前，默认外壳自己的）、`tab()`（本 space 当前的右栏页签：带到前台与 `redraw()` 时读，是它的页签就选中）、`listSession(session)`（返回 false ⇒ 这个会话不列在左栏）、`reenter()`（前台 space 的入口被再点一次）、`newSession`（false ⇒ 没有「新会话」）、`headerActions(session)`（标题栏里自己的操作）、`detailsWidth`（右栏缺省宽度：px，小于 1 时是窗口的比例，如 0.45；至少 320；右栏在哪个 space 都能拖到窗口的 70%，同时给中区留 400）、`emptyConversation(session)`（会话还没有消息时画在转录顶部的说明）、`secondary()`（**二级栏**：`{ key, title(), render(), onClose?(), width? }`，返回非空就在中区对话旁边打开；能拖宽（300–760，缺省 440）、能最大化 / 还原（按 space 记住）、能关——关时调 `onClose`，space 放掉选中项再 `redraw()`；返回空就收起），每次画时读，抛错只丢那一样 |
| `home.card` | **`id`**、**`render()`**、`title()`、`order`：画在新对话的输入框下面 |
| `tool.row` | **`id`**、**`match(toolBlock)`**、**`render(toolBlock)`**、`title(toolBlock)`（这一行给工具起的名字：问这次调用的权限卡用同一个名字，不写就用工具的显示名） |
| `details.panel` | **`id`**、**`match(item)`**、**`render(item)`**、`title(item)`；`item = { kind, name?, … }`，由 `host.reveal(item)` 打开 |
| `flow.block` | **`id`**、**`match(block)`**、**`render(block)`**：对话流里的块，先于内建的匹配 |
| `sidebar.group` | **`id`**、**`group(sessions)`** → `[{ key, title, hint?, workspace?, rows }]`、`label()` |
| `command` | **`id`**、**`complete(query, ctx?)`** → `[{ replacement, label, description?, source?, run?(ctx) }]`、`trigger: "/" \| "@"`（默认 `/`；`@` 的候选排在外壳自己的文件、会话候选之后）。`ctx = { space, session }`；带 `run` 的候选选中即执行、不插入 |
| `settings.section` | **`id`**、**`label()`**、**`render()`** |

`space` 的可选函数怎么用（外壳每次画时读，抛错只丢那一样、诊断页记一条）：

| 字段 | 画在哪 |
|---|---|
| `badge()` | 左栏入口上的角标（数字或很短的字；null、0、"" 不画），收起成图标条时是右上角的点 |
| `disabled()` | 返回一句话 ⇒ 这个 space 暂时进不去：左栏入口灰掉、点不动、悬停是这句话；`host.spaces.switch` 到它不切（照常落定）。连上后落地时存着的 space 灰着，就落到 `order` 最小、没灰的那个。前台的 space 后来灰了，外壳不自动换走。`false` / `null` / `undefined`：照旧；状态变了调 `host.redraw()` |
| `readOnly(session)` | 返回一句话就把这个会话的输入框换成它。只是界面：服务端照旧收 `harnix.send`，要真拦在后端拦 |
| `pinned(session)` | 消息流顶上固定的一块（等人回答的问题、要批准的卡） |
| `composerNotice(session)` | 打开的会话里输入框上方的提醒条。`notice()` 只在新对话首屏；两处都要就写 `composerNotice: () => notice()` |
| `ids()` | 正文里哪些字是可点的编号（`ChatIds`：带 `g` 的正则、`onOpen`、`canOpen`） |
| `composer: { steer: true }` | 输入框底栏有「立即送达」开关：一轮在跑时，消息在下一步送进这一轮（`on_busy: "steer"`）。在引擎的 UserPromptSubmit hook 也管 steer 之前，steer 的消息绕过那个 hook |
| `hideCommands: [...]` | 这些命令名不出现在本 space 的 `/` 菜单里（只藏别的 owner 的，所以你的同名命令能顶上）；手打的照旧当文字发出 |
| `mention: { first: "contributed" }` | 本 space 在前台时，`@` 菜单里 `trigger: "@"` 的候选排在外壳自己的（文件、`@session:`）前面；菜单只列前几项，文件多时贡献的候选就不会被挤掉。不写或 `"files"`：照旧 |
| `group`（字段） | 左栏顶部列表里的组名：按 `order` 相邻的两个 space 组不同，中间画一条分隔线；不写算 `""`，都不写就没有线。不重排，同组的靠 `order` 挨在一起 |
| `tab()` | 本 space 当前的右栏页签（它的页签 id 或 `"details"`）：带到前台时、`host.redraw()` 时读，是这个 space 的页签就选中它（不打开右栏）；`undefined` 或不认识的 id 不管。写了它，人点的页签换 space 回来也还在。跟着人点要听 `details.onTab` |

条目在流里是 `{ kind: "custom", type: "entry", data: { id, ns, event, payload, at } }`，用
`flow.block` 认领；没人认领不画。

### 22.4 `NestHost`（`useNest()`）

| 成员 | 说明 |
|---|---|
| `call<T>(method, params?)` | 只放行 `harnix.reachable` 里的 |
| `subscribe(topic, params \| undefined, fn)` | 返回取消函数。按 `(topic, 键排序后的 params)` 计数：同一页共用一份订阅，最后一个退订才发 `harnix.unsubscribe`，后来者先拿到已有的值，重连后自动重订 |
| `session.current() / context() / list() / open(ref) / create(spec?)` | 都是前台 space 的；`create` 自动绑定前台 space |
| `session.open(ref, { switchSpace: true })` | 去会话所属的 space（按中枢账本）打开，先把它带到前台；返回的 promise 在打开后落定，没有这个会话就 reject |
| `session.open(ref, { switchSpace?, reveal: key })` / `session.reveal(key)` | 打开后（`reveal(key)`：当前会话）等转录追上（有上限），在对话里找 `[data-reveal="<key>"]`（`flow.block` 的渲染给自己的块加上），没有就找 id 是 `key` 的块，滚到中间、`data-revealed` 闪 2.4 秒；返回 `Promise<boolean>`：找到没有 |
| `session.onChange(fn)` | 前台 space 的当前会话变了（打开、新建、关掉、被删、换 space），同一个值不重复；`@harnix/react` 的 `useCurrentSession()` 是它的 hook |
| `spaces.switch(id) / current()` | 把 space 带到前台，新 space 画出来之后落定；没有这个 space 时 reject；它的 `disabled()` 此刻给了字符串时不切、照常落定（ipdrun-48） |
| `details.open(tab?) / close() / isOpen() / tab() / onTab(fn)` | 右栏开合与选中页签（`tab` 是前台 space 的页签 id，或 `"details"`）。经 `open(tab)` 选的页签按 space 记着；人点的换 space 后回到第一个（同以前；space 写了 `tab()` 时也记着） |
| `redraw()` | 让外壳重读前台 space 的函数（`tabs()`、`badge()`…），不动注册表版本 |
| `settings.open(section?)` | 打开设置面板的一节；内建的 id：`general`、`scenes`、`models`、`mcp`、`plugins`、`devices`、`diagnostics`、`about`；没有这一节时打开第一节 |
| `instances.list() / sessions(instance) / tasks(instance, sessionId) / create(template, name, params?) / stop(instance)` | 答的是协议里对应方法的结果，类型取自 `@harnix/core` 的 `Methods`：`{ instances: InstanceRow[] }`、`{ instance, sessions: SessionRow[] }`、`{ active, tasks: TaskRow[] }`、`InstanceRow`、`{ stopped }` |
| `composer.insert(text, { session? })` | 不给 `session`：插进页上的输入框（同以前）。给了：那个会话的输入框在页上就插进去，不在就替它存着（再插接在后面），它挂上时放进去 |
| `modal.isOpen() / onChange(fn)` | 外壳自己的模态层开着没有；开合变了通知，返回退订。模态层是一个栈：对话框上再开一个是叠在上面，不算开合。开着时 Esc 只关最上面一个，不再传到 document / window 上的监听；⌘K、⌘\、⌘/ 不生效；对话框下面的页面是 inert 的 |
| `shell.badge(n)` | 页面标题前缀 `(n) `：n 取整、> 0 才带；null / 0 去掉（ipdrun-33） |
| `shell.context(c \| null)` | 左栏品牌下面一行应用的上下文，`c = { label, title?, tone?: "active" \| "muted", onClick? }`（类型 `ShellContext`）：`active` 前面一个圆点，`muted` 浅色字，`title` 是悬停提示，给了 `onClick` 是按钮；每次调用替换上一次，null 或空 `label` 去掉；不随 space 变，收成图标条时不画（ipdrun-49） |
| `space(moduleId).fetch(path, init?) / socket(path)` | 模块自有链接 |
| `reveal(item)` | 右栏打开详情 |
| `t(key, …)` | 文案 |

拿不到 Harnix 自己的连接。

### 22.4.1 外观：令牌覆盖与语言包

| 调用 | 说明 |
|---|---|
| `r.tokens({ "--accent": "#2f6fb3", … }, owner)` | 只许覆盖主题已有的令牌名（`:root` 上声明过的）；新名字、`url(` 之类的值被拒并记进诊断。后注册的赢 |
| `r.strings("zh-CN" \| "en", { "<owner>.key": "…" }, owner)` | 语言包；key 必须以 `<owner>.` 开头，别的被拒。之后 `host.t("<owner>.key")` |
| `r.icons({ name: "<path …/>" }, owner)` | 应用自己的图标，全名 `<owner>.<name>`（space 的 `icon` 写全名）。内容是 SVG 内部，只收 `path`、`circle`、`rect`、`line`、`polyline`、`polygon`、`ellipse`、`g` 与几何、描边属性；带脚本、`on*`、链接、样式、`url(` 的整个拒收、记进诊断 |

三者都不是贡献点（贡献点还是 8 个）。编进来的模块和运行期 v2 包都能用；运行期包拿到的注册表把 `owner`
钉成包名。页面标题、左栏产品名与图标来自 profile 的 `[brand]`。

### 22.4.2 外壳的框架

一个 space 就是三栏：左栏中段是它的 `sidebar()`，中区默认是对话（`center`），右栏是它的 `tabs`。不随
space 变的只有：顶部的标识、新建对话、搜索；底部的头像（就是设置入口，外圈颜色表示连接状态；点开是
设置、主题、语言、关于、连接详情）。两边的栏都能拖宽度。内建的 space——Harnix app 的会话（按时间）、项目（树，
一次展开一个），以及系统（入口在底部）——都是用例，`[shell] spaces` 决定留哪些（[Harnix app](harnix-app.md)、[系统 space](system-space.md)）。

### 22.5 构建

```sh
HARNIX_UI_MODULES=<各 space 的 ui/src/index.tsx，逗号分隔绝对路径> HARNIX_UI_OUT=<目录> \
  pnpm --dir harnix --filter @harnix/shell build
HARNIX_UI_DIST=<同一目录> cargo build --release -p <app>
```

外壳的 Vite 配置把 `react`、`react-dom`、`@harnix/react`、`@harnix/core` 去重到外壳那一份，所以应用的
界面入口在 Harnix 的 pnpm workspace 之外也能解析到它们；**别的第三方依赖**要在应用自己的
`node_modules` 里。完整的例子是 `examples/hello-app/scripts/build.sh`。

规矩：颜色只用令牌、文案只用 key（静态检查会拦）；CSP `script-src 'self'; style-src 'self'`：没有
`eval`、远程 import、CDN、内联脚本与样式；交互元素带 `data-testid`、状态用 `data-state`。

### 22.6 Agix 插件（运行期包）的界面模块

```toml
[[ui]]
point = "tool.row"
module = "rows.js"     # 包的 ui/ 下
api_version = 2
```

- 模块是**预编译好的 ES 模块**，不带 React：`import { jsx } from "react/jsx-runtime"`、
  `import { useNest } from "@harnix/react"`。页面的内联 import map 把 `react`、`react-dom`、
  `react-dom/client`、`react/jsx-runtime`、`@harnix/react` 指到外壳自己用的那几份，页面上只有一个 React。
- 导出 `register(registry, host)`；`registry.register(point, c)`、`registry.tokens(t)`、
  `registry.strings(locale, d)`，`owner` 一律是包名。
- import map 是页面里唯一的内联脚本，服务端出页面时按它的 `sha256` 写进 CSP；没有 `unsafe-inline`、
  `unsafe-eval`。
- `api_version` 不是 2（或没写，按 v1 算）：这一条报「这一条不生效」并写明原因，包的其余部分照常。
- `[ui] runtime_modules = false`：一条都不提供、不服务。
- 运行期包仍然**不能**新增 space。范例：`tests/fixtures/packages/demo-rows-v2/`。

---

## 23. 流程模块：`harnix-flow`、`@harnix/flow`

可选模块：应用 `.module(harnix_flow::Flow::builder()….build())` 才有；Harnix app `harnix` 注册了它（`crates/cli/src/main.rs`）。
对象与写法（站、执行者、出口条件、规则、策略与权限、门禁、钩子……）见流程指南 `crates/flow/docs/guide.zh-CN.md`
（系统 space 里流程的参考页就是它，`app.flow.guide {lang}` 也给它）。

| 构建器 | 意思 |
|---|---|
| `.packs(impl Packs)` / `.packs_with(\|data_dir\| …)` | 流程包：`MemPacks`（代码里给）、`DirPacks`（目录分层，`with_builtin` + `with_writable`） |
| `.store(impl RunStore)` / `.store_with(…)` | 运行存在哪；缺省模块数据目录下 `runs/` 的 `FileStore` |
| `.scene(Arc<dyn Scene>)` | 给流程用的 scene，包装成 `flow-<id>`（带 `flow_submit`、`flow_state`、`flow_propose`）。一个都不给时用内置的 `harnix`、`code`、`mini`（`flow-harnix`、`flow-code`、`flow-mini`，见 `harnix_flow::builtin_scenes`） |
| `.main_scene(id)` | 运行主会话缺省的 scene（缺省第一个；不给 scene 时是 `harnix`）。流程定义里 `main: {scene: flow-code}`、站点的 `scene:` 可各自另选 |
| `.template(InstanceTemplate)` | 站可以放上去的模板（`flow.<id>`） |
| `.programs(impl Programs)` / `.tools(impl Tools)` / `.gates(impl Gates)` | 应用的程序（`kind: program`）、只读工具（`tool` 门禁）、检查（`host` 门禁，名字也要进目录） |
| `.command_gates(bool)` | 让 `command` 门禁在本机跑命令（缺省关） |
| `.guarded_tools(…)` | 没有权限放行就拒绝的工具（缺省引擎的 shell、文件编辑、notebook、解释器） |
| `.authorizer(impl Authorizer)` | 谁能做什么（确认、认领、交付、编辑流程……），`who` 交给它解释 |
| `.space(id)` | 流程会话所在的 space，缺省 `flow` |
| `.recording(…)` / `.main_recording(…)` / `.conversation_recording(…)` | 回放时任务会话、主会话、流程会话用哪份录像（只在 `--replay-dir` 下有意义） |

主会话的 scene 需要项目时，模块给每个运行建工作目录（模块数据目录下 `workspaces/<run>`，也是引用 `${workspace}`），
并写入子 Agent 站缺省用的 agent 类型 `flow-agent`。钩子是流程包里的文件（`hooks/`），模块自己投递，没有 `Hooks` trait。

方法 `app.flow.*`（设备调用）：读——`defs`、`def`、`list`、`get`、`history`、`catalog`、`schema`、`validate`、
`parts {kind}`、`part`、`hooks`、`guide`、`sessions {run}`；写流程与件——`save`、`delete`、`part.save`、`part.delete`；
运行——`start`、`cancel`、`pause`、`resume`、`confirm`、`force`、`rework`、`retry`、`release`、`proposal`；外部任务——`claim`、
`heartbeat`、`submit`、`fail`；对话——`session {flow?}`、`focus`；试运行——`dry_run.start` / `step` / `run` / `list`。
主题：`app.flow.runs`、`app.flow.run {run}`、`app.flow.defs`、`app.flow.focus {instance, session_id}`、`app.flow.dry_runs`。每个方法的参数与返回写在它注册时的说明里（`harnix.reachable` 列出）。

界面：外壳在后端有流程模块时自己把流程放进系统 space（`registerSystemFlows(registry, { host, within: "system" })`：控制台多一个「流程」页，
点一条流程，左栏变成这条流程的上下文），没有系统 space 时注册单独的流程 space；自己编外壳的应用调 `registerFlowSpace(registry, { host })`，
或 `registerSystemFlows` 再把它返回的 `render()` 画在自己的页面上。界面怎么走见 [系统 space](system-space.md) §5。

## 24. 测试：`@harnix/testkit`

| 导出 | 说明 |
|---|---|
| `startHarnix(opts) → Backend` | 起一个进程，自己的 HOME、数据根、端口。`opts.bin` 写应用二进制的绝对路径；`profile`（追加的 TOML）、`env`、`args`、`replay`、`seed`、`uiDir`、`scratch` |
| `Backend` | `url`、`port`、`token`、`scratch`、`state`（`$HOME/.<product_id>`）、`project`、`pid`、`log()`、`stop()`（发 SIGTERM、立刻删目录，不等退出；还在退出的进程写进去的文件可能留下，不报错）、`dispose()`（等进程退出——15 s 后 SIGKILL——再删目录；收尾用它，夹具与 `defineSuite` 已用它）、`halt(signal?)`、`restart()` |
| `connect(backend) → TestClient` | `call`、`refused`（期望被拒，返回错误）、`waitFor(pred, { timeout, describe })`、`events`、`eventsOfKind`、`text`、`close` |
| `test`、`expect` | Playwright Test，带 `api` 夹具 |
| `inconclusive(reason)` | 环境不满足时标成「没结论」而不是通过 |

用例打标签 `@feature:<功能>`、`@layer:api|ui`，覆盖表从标签生成。`replay: true` 读的是 Harnix 自己的
`tests/live/fixtures/recordings`；应用要用自己的录像，`args: ["--replay-dir", <目录>]`。

---

## 25. 应用仓库用的脚本

| 脚本 | 什么时候跑 |
|---|---|
| `harnix/scripts/sync-app-deps.mjs <app> [--check]` | 升 Harnix 之后：重写应用根 `Cargo.toml` 里带标记的依赖块；`--check` 只比对 |
| `harnix/scripts/check-pins.mjs <app> [--release]` | 构建前：`harnix/` 指向的提交在上游存在；它带的东西检出的正是它钉的那一版。应用根下还有旧布局的 `agix/` 时照样检查，并告警提示迁移 |
| `harnix/scripts/check-lock.mjs <app>` | 构建后：应用的锁文件里 Harnix 的依赖是否和 Harnix 的一致 |
| `harnix/scripts/set-version.sh` | 不是给应用的：Harnix 自己发布用 |

仓库地址：`https://github.com/openatta/Harnix.git`（GitHub 上已从 `openatta/harnix` 改名，旧地址会转）。问题与需求都提到
这里的 issue（§2.1）。

---

## 26. 迁移时要知道的缺口

| 缺口 | 影响 | 什么时候补 |
|---|---|---|
| 没有 MCP 服务端、角色令牌 | 外部 Agent 要连：模块 `start()` 里自己起监听 | 0.4 之后 |
| 批准原语 | 不进 Harnix：照 §10.1 用条目 + 主题 + 只许设备的方法自己做 | 不做 |
| Agix 插件新增 space | Agix 插件的 `[[ui]]` 只能贡献 8 个点里除 `space` 之外的 | 不做：要自己的 space 的第三方代码写成插件 APP（[插件](harnix-plugin.md)） |
| 子 Agent 的进度 | 任务页能读 Shell / 监视器边跑边出的输出、子 Agent 的回答，但看不到子 Agent 正在第几步；要看过程走它的侧链会话 | 引擎，有需要再提 Harnix issue |
| 应用自己的 pnpm workspace 怎么摆 | 外部应用靠 Vite 去重解析 `react`、`@harnix/react`；类型检查与别的依赖要应用自己装（见迁移说明） | 消费者落地时定 |
| 桌面壳在 Windows 上没有干净的停止 | 关窗口 5 秒后直接结束子进程，模块的 `stop` 不跑 | 有需要再做 |

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

壳替自己做的事（通知、角标、原生对话框）经 `harnix-client` 当子进程的客户端：`Started::client()`（ipdrun-43）。

业务流量全是页面自己的 HTTP/WS：没有 command、没有 invoke handler、没有全局 Tauri 对象，
`check-no-ipc.mjs` 在 offline 层查。`--smoke` 不开窗口走一遍起、准入、停。详见它的 README。
