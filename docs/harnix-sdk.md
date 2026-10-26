# 在 Harnix 上做应用：二开指南

> 读者：在 Harnix 上做产品的人（Helm、IPDrun，以及以后的应用）。
>
> **这份指南写的是 0.4 的目标形态，开发还没开始。** 每节标了从哪个里程碑起可用（里程碑见
> [roadmap](roadmap.md)，当前进度见 [status](status.md)）。代码示例里的签名是**示意**：写清楚形状，
> 具体名字以实现为准，实现落地时回来改这份文档。
>
> 用词照 [术语](glossary.md)。为什么这样设计见 [0.4 设计](harnix-0.4-design-and-plan.md)。

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
├── harnix/                   子仓 → Harnix（不要初始化它里面的 agix/）
├── app/                    bin crate：main.rs 里 App::builder()…
├── spaces/
│   └── <x>/
│       ├── backend/        Rust crate，实现 harnix::Module（可选）
│       ├── agent/          原生工具与场景，可以并进 backend（可选）
│       └── ui/             pnpm 包：React 组件 + register()（可选）
├── pnpm-workspace.yaml     packages: harnix/ui/packages/*, spaces/*/ui
├── profiles/<app>.toml
└── scripts/build.sh        pnpm build → cargo build
```

**`agix/` 和 `harnix/` 平级。** 你发现 Agix 的问题，直接在自己的 `agix/` 里开分支、回推 Agix，
不用先改 Harnix。

Cargo 这边要知道三件事（为什么这样做见设计 §11.2）：

1. 应用的 workspace 把 `harnix/crates/*` 收为**成员**，`exclude = ["core"]`。
2. 根 `Cargo.toml` 里有一段带标记的区块，由 `harnix/scripts/sync-app-deps` 生成，**不要手改**。它把
   Harnix 需要的依赖（含 Agix 的入口 crate）指到以应用根为基准的路径。升级 Harnix 之后重新跑一次。
3. `[profile.release]`、`[patch]` 写在应用根，Harnix 根目录里的那些对你不生效。模板里已经带好了。

一份 profile 的样子（0.4 目标形态）：

```toml
[brand]                          # M8
name = "Helm"
product_id = "helm"              # 决定 .harnix/ 账本命名空间与 token 文件位置

[engine]                         # system 实例
scene = "default"

[[instances]]                    # 固定的实例（M3）
id = "ci-runner"
carrier = "spawn"                # spawn | attach | in_process
scene = "ci"

[[instance_templates]]           # 界面可以按它建实例（M3）
id = "sandbox"
carrier = "spawn"
scene = "general"
data_root = "instances/sandbox/{name}"
params = ["name"]
ui = true

[modules.watchlist]              # 你的模块的配置段（M2）
db = "watchlist.sqlite"
quote_api_key = { credential = "helm/quote-api" }   # 只写引用，值由 CredentialSource 给

[system_space]
enabled = false                  # 产品里关掉调试面（M6）

[ui]
runtime_modules = false          # 不加载运行期包的界面模块（M8，见 §13）
```

## 4. 起步：从模板建一个应用（M2 起）

1. 复制 `harnix/templates/app` 到新仓库；`git submodule add` 加上 `agix/` 和 `harnix/`，钉到 Harnix 当前
   钉的那个 Agix（`harnix/` 里 `git ls-tree HEAD core` 看得到）。
2. 跑 `harnix/scripts/sync-app-deps`，再跑 `harnix/scripts/check-pins`。
3. `scripts/build.sh`，然后 `./target/debug/<app> --profile profiles/<app>.toml`。
4. 打开启动链接，看到空应用（会话 space + 设置）。
5. 在 `spaces/` 下加第一个 space，在 `app/src/main.rs` 里 `.module(...)` 加上它。

## 5. 后端：写一个模块（M2 起）

### 5.1 最小的模块

```rust
use harnix::prelude::*;

#[derive(Default)]
pub struct Watchlist;

#[async_trait]
impl Module for Watchlist {
    fn manifest(&self) -> Manifest {
        Manifest::new("watchlist", env!("CARGO_PKG_VERSION"))
            .disclosure("读写关注组；提供「关注组」space")
    }

    fn methods(&self, m: &mut MethodRegistry) {
        // 全名 app.watchlist.list；声明给哪类主体用，没声明的一律拒绝
        m.method("list", Principal::Device)
            .params::<ListParams>()
            .result::<Vec<Group>>()
            .handler(list_groups);
    }

    fn events(&self, e: &mut EventRegistry) {
        // 全名 app.watchlist.groups；推整值快照
        e.topic("groups").snapshot::<Vec<Group>>();
    }

    async fn start(&self, ctx: ModuleContext) -> anyhow::Result<()> {
        let cfg: WatchlistConfig = ctx.config()?;            // [modules.watchlist]
        let key = ctx.credentials().get(&cfg.quote_api_key).await?;
        let db = Db::open(ctx.data_dir().join(&cfg.db))?;    // 数据目录是你自己的
        ctx.provide(State { db, key, events: ctx.events() }); // handler 里用 cx.state::<State>()
        Ok(())
    }
}

async fn list_groups(cx: CallContext, p: ListParams) -> Result<Vec<Group>, AppError> {
    cx.state::<State>().db.groups(p.filter)
}
```

```rust
// app/src/main.rs
fn main() -> anyhow::Result<()> {
    harnix::App::builder()
        .profile_from_args()
        .with_builtins()
        .module(watchlist::Watchlist::default())
        .module(trade::Trade::default())
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
| `stop` | 退出或停用 | 收尾 |

**启动顺序**：注册 → 按顺序 `migrate` → 建实例 → 按顺序 `start` → 开始服务。

**失败只影响你自己**：注册冲突、`migrate` 失败、`start` 失败，都只停用这个模块和它的方法、路由、
space、实例，其余照常启动，原因写在诊断页。所以 `start` 里别吞错误：返回 `Err`，让诊断页说清楚。

**`ModuleContext` 有什么**：`config()`、`data_dir()`、`credentials()`、`events()`、`sessions()`（§7）、
`instances()`（§6）、`provide()`。**没有**中枢的内部状态，也没有直连实例的 `EngineLink`：所有会话操作
都经过中枢。

### 5.3 方法、主题、路由的规矩

- 方法名在 `app.<id>.*` 下，参数与返回带 schema（以后可以投影成 MCP 工具）。
- 主题推**整值快照**，不推增量。要按对象订阅就带参数：`app.trade.job { job_id }`。
- 主体：`Device`（一个人的设备）、`Module(id)`（别的模块调你）、`Kernel`。方法默认拒绝。
- 自有路由挂在 `/apps/<id>/` 下，与界面同源，准入走同一套授权。它**不享受**拓扑无关、`seq` 追赶
  这些保证，所以只在 Harnix 通道不合适时用（§9）。

### 5.4 数据与迁移

Harnix 只管什么时候迁、失败了怎么办，不管用什么迁。数据库、迁移工具、连接池你自己选。你的数据放在
`ctx.data_dir()` 里，和 Harnix 的 `.harnix/` 账本分开。0.4 没有 `--dry-run` 和备份钩子，要的话在你的
`migrate` 里自己做。

## 6. Agent 侧

### 6.1 给 `system` 加原生工具和场景（M2 起）

```rust
fn engine(&self, e: &mut EngineContrib) {
    e.scene(research_scene());
    e.customize_session(|b| b.native_tool(QuoteTool::default()));
}
```

多个模块的 customizer 由 Harnix 按注册顺序组合。

### 6.2 给 space 一台独立、隔离的实例（M3 起）

实例只能按模板建。**模板是你在 `templates()` 里声明的**，模块和界面都按它建，Harnix 记进账本、
重启后统一重建。

```rust
fn templates(&self, t: &mut InstanceTemplates) {
    // 进程内：能带原生贡献。适合「这个 space 要一台带交易工具的引擎」
    t.in_process("research")
        .scene(research_scene())
        .customize_session(|b| b.native_tool(OrderTool::default()))
        .data_root("instances/research");

    // daemon：配置完全独立（不同的 MCP、凭据），崩溃与资源也隔离；不能带原生贡献
    t.spawn("sandbox")
        .scene("general")
        .data_root("instances/sandbox/{name}")
        .param("name")
        .ui(true);                       // 允许界面按它建
}

async fn start(&self, ctx: ModuleContext) -> anyhow::Result<()> {
    // 幂等：账本里有就复用，没有就建
    let research = ctx.instances().ensure("research", "research", Params::none()).await?;
    ...
}
```

让一个 space 默认用它：space 声明里写 `instance = "template:research"`（第一次在这个 space 里建会话
时按模板建，之后复用），或者写一台固定实例的 id。

**选哪种载体**：

| 要什么 | 用 |
|---|---|
| 原生 Rust 工具、`SessionCustomizer` | 进程内 |
| 和 `system` 不同的 MCP、凭据、插件 | daemon（进程内实例要等 Agix issue 03 修好才能配置不同） |
| 崩溃、内存与 Harnix 隔开 | daemon |

一台 daemon 只服务一个 Harnix；不要让别的客户端也连它。

### 6.3 运行期 Agix 插件包

Agent 侧的第三方扩展照旧装到 Agix（WASM / QuickJS），能力门控与披露都在引擎那边。Harnix 只读包里
的 `[[ui]]` 段。0.4 里运行期包**不能**新增 space，也不能往别的 space 加页签。

## 7. 会话

### 7.1 两类调用方，一套语义（M3 起）

界面（客户端）和你的模块（服务端）都能驱动会话，排队、中断、结算、广播完全一样。无头会话照样出现在
它所属 space 的会话列表里，界面随时可以 attach 旁观。

```rust
let s = ctx.sessions().create(SessionSpec {
    instance: research.id(),
    space: "watchlist".into(),
    scene: Some("research".into()),
    project_root: Some(root),
    provider: Some("anthropic".into()),
    model: Some(model),
    permission_mode: PermissionMode::Default,
    output_schema: None,
    ..Default::default()
}).await?;

ctx.sessions().send(&s, "总结今天的异动", OnBusy::Queue).await?;

let mut tap = ctx.sessions().tap(&s).await?;
while let Some(item) = tap.next().await {
    match item {
        TapItem::Event(ev) => handle(ev),
        TapItem::Lagged { last_seq } => tap.catch_up(last_seq).await?, // 你读慢了，补齐
    }
}
```

- `OnBusy`：`Queue`（排队，默认）、`Reject`、`Interrupt`（插队首并中断本轮）、`Steer`（在步骤边界
  送进运行中的一轮）。
- **tap 会落后**：它是有界通道，中枢不会为了等你而卡住会话。读慢了就收到 `Lagged`，按它补齐。
- 会话的全名是 `(instance, session)`，任何地方都带着实例。

### 7.2 往对话里写卡片：扩展条目（M7 起）

```rust
ctx.sessions().append_entry(&s, Entry::new("watchlist.alert", json!({ "symbol": "NVDA" }))).await?;
```

条目进会话日志，刷新、重放、分叉都看得到；**只给人看，不给模型看**。前端用 `flow.block` 按
`kind` 画（§8.3）。

## 8. 前端

### 8.1 包（M4 起）

| 包 | 你用不用 |
|---|---|
| `@harnix/react` | **用**。贡献注册、`useNest()`、i18n、令牌、图标、对话组件 |
| `@harnix/core` | 一般不直接用；要写不依赖 React 的逻辑时用它的 `/state`、`/client` |
| `@harnix/shell` | 不 import。它是应用入口，构建脚本把你的 `register()` 接进去 |
| `@harnix/testkit` | 写测试时用 |

你的 space 界面包只依赖 `@harnix/react`（和 React 本身）。

### 8.2 注册一个 space（M5 起）

```tsx
// spaces/watchlist/ui/src/index.tsx
import type { Registry } from '@harnix/react';
import { QuotesTab } from './QuotesTab';
import { AlertCard } from './AlertCard';
import { Summary } from './Summary';

export function register(r: Registry) {
  r.space({
    id: 'watchlist',
    icon: 'list',
    title: 'watchlist.title',              // i18n key，不写死文案
    center: 'chat',                        // chat | minimized | hidden
    entries: { source: 'app.watchlist.groups', create: 'app.watchlist.create' },
    tabs: [{ id: 'quotes', title: 'watchlist.tab.quotes', component: QuotesTab }],
    session: { scene: 'research', instance: 'template:research' },
  });
  r.homeCard({ id: 'watchlist.summary', component: Summary });
  r.flowBlock({ kind: 'watchlist.alert', component: AlertCard });
}
```

构建脚本按 `main.rs` 里的模块顺序生成 `virtual:harnix-modules`，把每个 space 的 `register` 接进外壳，
Vite 一次打包，产物嵌进二进制。

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
| `instances.*` | `list`、`sessions`、`tasks`、`create(template, params)`、`stop` |
| `composer.insert(text)` | 往输入框插字 |
| `reveal(panelId, subject)` | 点消息里的卡片，右栏切到对应页签并打开详情 |
| `space.fetch()` / `space.socket()` | 你模块的自有链接（M7） |
| `t()`、令牌、图标 | i18n 与主题 |

**你拿不到 Harnix 自己的连接**，所以同一份代码在任何拓扑下都能跑。**没有全局的「当前会话」**：每个
space 记自己的当前会话，`session.current()` 拿到的永远是前台 space 的。

### 8.4 对话组件

中区的对话由外壳画，你一般不用自己画。要在自己的页签里放一个对话（例如一个工位的转录），用
`@harnix/react` 的 `ChatPanel`，传 `(instance, session)`。产品特有的东西（语音、快捷回复……）在你的应用
里包一层，不要往 `ChatPanel` 里加。

### 8.5 规矩

- 颜色只用令牌，文案只用 key：两条静态检查会拦。
- CSP 是 `script-src 'self'`：不能 `eval`，不能远程 import，不能 CDN。
- 交互元素带 `data-testid`、状态用 `data-state`，组件对象照这个找元素。

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

引擎的权限管「这个工具能不能跑」；「这个业务动作要不要人确认」归你。用公开契约就能做：

1. **请求**：你的代码要下单时，建一张票（id、会话、标题、摘要、选项、截止时间），存在你的模块里。
2. **推送**：带参数的主题 `app.trade.pending { instance, session }`，推这个会话的待批票。
3. **记录**：`append_entry(session, Entry::new("trade.approval", 票))`，卡片进时间线。
4. **回答**：方法 `app.trade.approve { ticket, option }`，**主体只许 `Device`**。票的状态只能从
   「待批」转一次（先答者生效），之后的回答拒绝。
5. **超时**：到截止时间还没答，按拒绝处理。
6. **审计**：决定再写一条扩展条目：谁、什么时候、哪台设备、选了哪项（`CallContext` 里有设备信息）。
7. **画卡**：`r.flowBlock({ kind: 'trade.approval', component })`，按钮调 `app.trade.approve`。

**要知道它防不住什么**：「只许 `Device`」挡得住引擎、模块、内核替人回答，挡不住装进来的第三方界面
模块（它和页面同级）。交易类产品请在 profile 里设 `[ui] runtime_modules = false`。

### 10.2 长任务进度

方法 `app.<id>.start_job` 返回 `job_id`；进度推到 `app.<id>.job { job_id }`，每次推完整状态
（进度、阶段、结果或错误）。界面订阅这个主题。完成时想在对话里留一张卡，就写一条扩展条目。

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
- `<app> --version` 打印应用、Harnix、Agix、模块 API、贡献点 API 的版本。
- **两个钉**：`harnix/` 钉一个 Harnix 的标签（`Harnix-v<x.y.z>-<日期><序号>`），`agix/` 钉一个 Agix 的提交。
  `harnix/scripts/check-pins` 管两者的关系：
  - `agix/` 和 Harnix 钉的一样：通过。
  - 比 Harnix 钉的新，并且在 Agix 上存在：开发分支告警；发布前要么回到一样，要么先让 Harnix 升到这一版。
  - 只在本地、不在上游：失败。
- **升级 Harnix**：bump `harnix/` 子仓 → 把 `agix/` 对齐到 Harnix 新钉的版本 → `sync-app-deps` →
  `check-pins`、`check-lock` → 跑测试。0.4.z 里协议与模块 API 只做加法，升 z 不该打破你。
- **公共改动**：直接在你的 `harnix/` 子仓里开分支 → 回推 Harnix → 合并、打标签 → 你 bump 子仓。
  不许有长期补丁：子仓指针只能指向上游存在的提交。

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

通用的顺序：

1. 把 Agix 升到 Harnix 钉的那一版。
2. 换布局：`harnix/`、`agix/` 平级的子仓；workspace 收 `harnix/crates/*`；`sync-app-deps`、`check-pins`。
3. 后端改成模块：一个 space 的后端一个 `Module`；领域方法挪到 `methods()`。
4. 前端换成 React 包：每个 space 一个 `ui/` 包，`register()` 注册。
5. 测试换成 `@harnix/testkit`。
6. 删掉 vendor 副本和照抄的代码，`check-pins` 通过，没有补丁。

### 14.1 Helm

| 现在 | 0.4 里 |
|---|---|
| `harnix/` 子仓里套着 `agix/` | `agix/`、`harnix/` 平级；workspace **收** `harnix/crates/*` 为成员（不是 Helm Issue 01 写的 `exclude = ["harnix"]`） |
| `harnix::Plugin`、`PluginContext`、`.plugin(...)`、`[plugins.<id>]` | `harnix::Module`、`ModuleContext`、`.module(...)`、`[modules.<id>]` |
| 7 个 Vue 文件（约 2.5k 行）、Pinia | React 组件，状态用组件内 state 或 zustand；`useNest()` 的能力相同 |
| space 的「实例」（关注组） | 叫**条目**；「实例」只指 Agix 实例 |
| 交易工具 | 进程内模板带原生工具（§6.2），交易 space 写 `instance = "template:<id>"` |
| Harnix 的批准原语 | 0.4 不提供，照 §10.1 自己做；profile 设 `[ui] runtime_modules = false` |
| 运行期包加 space | 0.4 不支持 |

### 14.2 IPDrun

`vendor/harnix` 里的改动**不合并**，0.4 用新结构重新实现了它们。对照表：

| IPDrun 现在 | 0.4 里 | IPDrun 要做的 |
|---|---|---|
| N-001、N-007 装配调 `assemble` | Harnix 内部 | 删掉 |
| N-002、N-105、N-106、N-120、N-121 | Harnix 内部 | 删掉 |
| N-004、N-005 包元数据、Agix 路径依赖 | 正式布局（§3） | 换布局 |
| N-101 `create_headless` 等 | `ctx.sessions().create / open / send / interrupt / close` | 改调用；`SessionSpec` 多了 `instance`、`space` |
| N-102 tap | `ctx.sessions().tap()`，**会收到 `Lagged`** | 处理 `Lagged`：按 `last_seq` 补齐 |
| N-103、N-108 `OnBusy` 与 steer | 同名 | 改 import |
| N-104 `EngineConfig.cwd` | `SessionSpec.project_root`；实例的数据根 | 改配置 |
| N-107 按会话 provider、`output_schema` | `SessionSpec` | 改调用 |
| N-301…N-602 启动链接、令牌文件、日志、Origin、Secure cookie | Harnix 内部，profile / 命令行开关 | 删掉，改配置 |
| R-10 `ipdrun-server` 照抄的装配 | `App::builder()` | 删掉照抄，`main.rs` 改成 builder |
| `/api/op`、`/api/ws` | `app.ipdrun.*` 方法与主题；确实要自己协议的放 `/apps/ipdrun/` | 领域操作挪进 `methods()` / `events()` |
| R-01 应用注册宿主方法 | `methods()` | — |
| R-04、R-05 角色令牌、`ipdrun-mcp` | 0.4 不提供 | 在模块 `start()` 里起自己的监听（§10.4），等 Harnix 0.4 之后接管 |
| R-06 `packages/nest-client` | `@harnix/core/client` | 删掉自己的，改依赖 |
| R-06 `packages/nest-ui`（ChatPanel） | `@harnix/react` 的对话主干 | 删掉主干；语音、上下文环、快捷回复这类产品面在 IPDrun 里包一层 |
| R-07 `apps/tauri` | `harnix-desktop` 模板（M9） | M9 之前先留着 |
| R-08、R-09 录像回放、`data-testid` | `@harnix/testkit`、组件对象约定 | 测试换框架 |
| R-11、R-12 会话运行状态、hello 超时 | `harnix.sessions.status`、`harnix.hello` | 改订阅 |
| sqlite 运行态、git 原语、代码扫描、xflow、9 个 scene、`pack/` | 留在 IPDrun | 各自做成模块或原生贡献 |

## 15. 各能力从哪个里程碑起可用

| 能力 | 里程碑 |
|---|---|
| `App::builder()`、`Module`、`app.<id>.*`、主题、`/apps/<id>/` 路由、配置段、凭据、迁移、失败隔离、应用模板 | M2（`0.4.0`） |
| 实例、模板、daemon、`ctx.instances`、无头会话、tap、`OnBusy`、带参数的主题、实例树 | M3 |
| React 外壳、`@harnix/react`（贡献点 v1）、对话组件 | M4 |
| space、`home.card`、贡献点 v2、宿主 API v2、`examples/hello-app` | M5 |
| 系统 space | M6 |
| 扩展条目、`space.fetch/socket`、批准卡范例 | M7 |
| 运行期界面模块 v2、`[ui] runtime_modules`、`[brand]`、令牌覆盖与语言包 | M8 |
| 桌面壳模板 | M9 |
