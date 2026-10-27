# Helm 迁移到 Harnix 0.4

> 读者：在 Helm 仓库里做这次迁移的人，或者 Agent。照本文做，配合 [二开指南](harnix-sdk.md)、
> [SDK 参考](harnix-sdk-reference.md) 和能跑的范例 `harnix/examples/hello-app`，不需要问 Harnix 的作者。
>
> **对照的 Harnix**：`main` 上的 0.4（M0–M9：M8 是外壳框架、运行期界面模块 v2、品牌，M9-1 是桌面壳模板），
> 从 `Harnix-v0.4.0-2026102603` 起的发布标签都含这些。Agix 钉在 `Agix-v0.2.9-2026102603`。
> 二开指南与 SDK 参考已经对齐 M8、M9；本文 §1 再把 M8 的变化集中列一遍，方便对照。几份文档对不上时，
> 以 Harnix 的代码为准。
>
> **对照的 Helm**：`main` 的 `dc19a44`（M0-S5，`helm-v0.1.5`），工作区里另有未提交的
> `docs/design/11-gui-workbench-v0.1.md` 与 `11-gui-demo/`。下文的路径都相对 Helm 仓库根。
>
> 本文只写在 Harnix 仓库里。**Harnix 这边不改 Helm 的任何文件**；改 Helm 是 Helm 自己的事。
>
> 用词照 [术语](glossary.md)。注意两个词：「实例」只指 Agix 实例；左栏里用户新建的东西（例如关注组）叫
> **条目**。「插件」只指 Agix 的包，Helm 的后端叫**模块**。

---

## 0. 结论先行

1. **Helm 今天没有接 Nest、Harnix 或 Agix。** 没有子仓、没有 vendor、没有一行依赖；Agent 是
   `helm-agent` 里一个跑录像夹具的 `RecordedCoreHost`。所以这次不是「换布局」，是**第一次接入**：把
   `helm-web` 的装配换成 `harnix::App::builder()`，把 `helm-agent` 换成 Harnix 里的 Agix 实例，把 Vue
   SPA 换成 Harnix 外壳里的 space。
2. 领域代码（`helm-contracts`、`helm-platform`、`helm-risk`、`helm-exec`、SQL 迁移、Python 服务）**几乎
   原样保留**。要改的是它们外面那一层：入口、鉴权、事件推送、界面、Agent 宿主、桌面壳。
3. 这会改掉 Helm `CLAUDE.md` §3「固定架构边界」里的三条（`helm-agent` 独立进程、Vue3 SPA、
   `helm-desktop` 加载本地 `web/dist`），也会改 `docs/design/07` §5.1 的 SD-02 / OD-08。**动手前由 Helm 的
   维护者在 Helm 仓库里改这两处并记决定**（§4 列了要定的事和建议）。
4. Harnix 外壳的三栏布局本来就照 Helm 的 `11-gui-workbench` 抄的，`tokens.css` 的配色也是那一套。
   Helm 的界面迁过来主要是**把页面内容搬进 space**，不用再做外壳。

---

## 1. M8 带来的用法（集中列一遍）

迁移时用下面这些，不用二开指南 §3、§8.2 里与之冲突的写法。

**profile**

| 写法 | 说明 |
|---|---|
| `[shell] spaces = ["sessions", "projects", "system"]` | 三个内建 space 各自可去掉，`[]` 一个不留。**取代 `[system_space]`**：写 `[system_space]` 进程拒绝启动。去掉 `system` 时系统 space 的方法一起撤 |
| `[brand] name / icon / product_id` | `name`：页面标题与左栏顶部；`icon`：SVG，路径相对 profile；`product_id`：小写字母、数字、`-`，字母开头 |
| 状态目录 | `~/.<product_id>`（`HARNIX_STATE_DIR` 可挪）：令牌、设备、`spaces.json`、`instances.json`、实例数据根 `instances/`、各模块的 `modules/<id>/`、上传。`~/Documents/.harnix` **不读**。Documents 里只放项目；项目自己的数据放 `<项目>/.agix/` |
| `[ui] runtime_modules = false` | 运行期包的 `[[ui]]` 一律不加载、不发出 |
| 每一段都拒绝不认识的键 | 拼错一个键进程就起不来，并说是哪一行 |

**前端**

| 写法 | 说明 |
|---|---|
| space = 左 `sidebar()` + 中 `center` + 右 `tabs` | `center`：`"chat"`（默认）、`"minimized"`（`overview()` 在上，对话收成一条）、`"hidden"`（只有 `overview()`）。不给 `sidebar()` 时左栏列这个 space 自己的会话 |
| `tabs` 可以是函数 | `tabs: () => [...固定页签, ...打开的对象]`，每次绘制都读，适合「打开的对象成为页签」 |
| `SpaceContribution.sessions` | 这个界面 space 列出、新建的是另一个后端 space 的会话（两种视图看同一批会话） |
| `r.tokens({...}, owner)` | 只许覆盖外壳已有的令牌名，新名字被拒并记进诊断页 |
| `r.strings(locale, {...}, owner)` | 语言包，key 必须在 `<owner>.` 下；`locale` 是 `"zh-CN"` 或 `"en"` |
| `home.card` | HOME 没有了；卡片显示在**新对话的输入框下面** |
| 贡献点 API 2 | 运行期包的界面模块是预编译的 ES 模块，从页面的 import map 引 `react`、`react-dom`、`react/jsx-runtime`、`@harnix/react`。编进二进制的模块（Helm 的就是）不受影响 |

**后端**：`harnix.process`（进程的版本、运行时长、内存，系统 space 用）、`harnix.system.trace`
（一个会话每次模型调用的输入与输出，来自录像）、主二进制的 `--exit-with-stdin`（stdin 关了就退出，桌面壳靠它
把子进程带走）。

**桌面**：`harnix/templates/desktop`（Tauri 2）。本地模式起应用的二进制、用 `POST /auth/link` 换一次性链接
打开窗口；远程模式开一个 `https://` 地址。没有业务 IPC。它取代 Helm 的 `desktop/`。

---

## 2. 现状盘点

### 2.1 Helm 现在有什么（与 Harnix 相关的部分）

| 面 | 现在 | 在哪 |
|---|---|---|
| Rust workspace | 根在 `rust/`，7 个 crate，`rust-version = "1.89"`，AGPL-3.0 | `rust/Cargo.toml` |
| 服务入口 | `helm-web`：读环境变量 → 连库并迁移 → 起 2 个轮询工人（任务租约、outbox）→ `axum::serve` | `rust/crates/helm-web/src/main.rs`、`lib.rs:603 build_router` |
| HTTP 接口 | `/api/v1/*` 约 50 条路由；命令要 `Idempotency-Key` 头（UUID，即 `intent_id`）；`require_role` 按 Human / Agent / Scheduler 判 | `helm-web/src/lib.rs` |
| 事件 | `event_stream` 表带 `seq`；`GET /api/v1/events?after_seq=` 回补，`/api/v1/events/ws?access_token=&after_seq=` 每 300 ms 推增量 | `helm-web/src/lib.rs`、`web/src/api/events.ts` |
| 鉴权 | 三个静态令牌 `HELM_HUMAN_TOKEN` / `HELM_AGENT_TOKEN` / `HELM_SCHEDULER_TOKEN`，另有签名的 Agent 令牌 `hagt1.*`（`HELM_AGENT_TOKEN_SECRET`） | `helm-platform/src/auth*`、`config.rs` |
| 配置 | 全是 `HELM_*` 环境变量，没有 profile 文件 | `helm-platform/src/config.rs`、`ops/.env.example` |
| 数据 | SQLite（默认，`helm-dev.db`、`.local/helm.db`）或 Postgres（`HELM_DATABASE_URL`）；原始文档在 `HELM_DATA_DIR`（默认 `.local/data`）。全在仓库目录里，不碰家目录 | `db/migrations/{sqlite,postgres}`、`helm-platform/src/sqlite/*` |
| 操作注册表 | 53 个操作，带角色；Rust 侧 `OPERATIONS` 是它的镜像 | `contracts/operation-registry.yaml`、`helm-contracts` |
| Agent | `helm-agent`：一次性的演示进程，`RecordedCoreHost` 跑夹具两遍比哈希；可选经 stdio 连 `research_mcp`；经 HTTP 回调 `submit-proposal` | `rust/crates/helm-agent/src/{main,mcp,policy,callback}.rs`、`helm-testkit`（`CoreHost` trait） |
| 批准 | 只有交易批准：`approval.request` → `approval.decide`（只许人，`BEGIN IMMEDIATE`）→ D42 额度；`revoke`、`expire` 等；9 种 `approval.*` 事件 | `helm-web/src/lib.rs:1333`、`helm-platform/src/sqlite/execution.rs:1765`、`web/src/views/W06View.vue:150` |
| Python | `ingest_worker`：每个任务由 `helm-web` 起一次子进程，stdout 回一个 JSON；`research_mcp`：stdio MCP，只读工具 `query`、`compute` | `python/`，`helm-web/src/lib.rs:296 invoke_worker` |
| 界面 | Vue 3.5 + vue-router + Pinia + Vite 6；6 个视图加 `App.vue`（Vue 文件约 1.6k 行），`api/client.ts` 767 行、`events.ts` 94 行；没有 i18n（中文写死）；开发时 `localhost:5173`，产物 `web/dist` 由 `helm-web` 用 `ServeDir` 发 | `web/src/` |
| 界面设计 | 三栏工作台、L1–L6 导航、右栏页签、对话卡片、T0–T3 工具档；原型是单文件 HTML | `docs/design/11-gui-workbench-v0.1.md`、`11-gui-demo/index.html`（未提交） |
| 桌面 | `helm-desktop`（Tauri 2）：托盘、单实例、通知、更新检查；**加载本地 `web/dist`**（`frontendDist`），开发时 `devUrl` | `desktop/src-tauri/` |
| 测试 | Rust `helm-web/tests/api.rs`（7.4k 行，约 108 个）、`config_gate.rs`、`database_flow.rs`；Python 4 个；Playwright `web/e2e/` 5 个 spec；CI 5 个 job | `.github/workflows/ci.yml`、`ops/*.sh` |

### 2.2 早先按 issue 写的对照表哪里不对

§14.1 是照 Helm Issue 01（设计稿）写的，不是照代码：

| §14.1 写的 | 实际 |
|---|---|
| 「`harnix/` 子仓里套着 `agix/`」 | Helm 没有任何子仓（没有 `.gitmodules`），也没有 vendor 副本 |
| 「`harnix::Plugin`、`PluginContext`、`.plugin(...)`、`[plugins.<id>]`」 | 从来没写过。Helm 的入口是手写的 axum 服务，配置是环境变量 |
| 「7 个 Vue 文件（约 2.5k 行）、Pinia」 | 数对，但 2.5k 行里约 860 行是 TS（`client.ts`、`events.ts`）；Vue 文件约 1.6k 行，W06 一个就 807 行。Pinia 只存了一个令牌 |
| 「space 的『实例』（关注组）」 | 代码里没有关注组。设计 11 里 L1 的「关注」页签（覆盖范围、目标公司、产业链节点）才是它，迁过来叫**条目** |
| 「交易工具」 | 代码里 Agent 只有 `query`、`compute`（MCP）和 `propose` / `attest`（HTTP 回调）；交易工具还不存在。设计 11 §6.2 的 T1/T2 写工具也还没做 |
| 「Harnix 的批准原语」 | Helm 有自己完整的批准状态机（`approval_request` / `approval` / `approval_consumption`）。照 SDK §10.1 做的是**卡片与回答通道**，状态机仍然是 Helm 的 |
| 没提 | 事件流（`event_stream` + `seq` 增量）、三种令牌、Python 子进程、`helm-desktop` 加载本地文件、Postgres、`Idempotency-Key` |

---

## 3. 对照表

动作：**保留**（不改或只改调用方）、**改写**、**删掉**、**等**（等 Harnix 或 Agix）。

| Helm 现在 | Harnix 0.4 里 | 动作 |
|---|---|---|
| `rust/Cargo.toml`（workspace 根在 `rust/`） | 根 `Cargo.toml`：成员 `app`、`rust/crates/*`、`harnix/crates/*`，`exclude = ["agix", "harnix/agix", "harnix/crates/testapp", "desktop"]`，带生成区块 | 改写（挪到仓库根） |
| `helm-contracts`、`helm-risk`、`helm-exec` | 原样做库 | 保留 |
| `helm-platform`（库、迁移、领域 SQL、任务、outbox、审计） | 原样做库；`config.rs` 的环境变量改成模块配置段；`auth*` 的令牌校验不再用于人 | 保留，改 `config` 与 `auth` |
| `helm-web/src/main.rs`（装配、`axum::serve`） | `app/src/main.rs`：`harnix::App::builder()...module(helm::Helm)...run()` | 删掉 |
| `helm-web/src/lib.rs` 的处理函数 | 新 crate `helm-module`（模块 id `helm`）：`methods()` 调同一批处理逻辑；过渡期整个 `/api/v1` 路由挂在 `routes()` 下 | 改写 |
| 两个轮询工人（任务租约、outbox） | 模块 `start()` 里 `tokio::spawn`，`stop()` 里停 | 改写（挪位置） |
| `/api/v1/events` + `/events/ws`（`seq` 增量） | 第一步：自有链接 `/apps/helm/api/v1/events/ws`，前端 `host.space('helm').socket(...)`；之后界面要的状态改成主题整值快照（`app.helm.<读模型>`） | 改写，分两步 |
| `Idempotency-Key` 头 | 方法参数里的 `intent_id`（方法调用没有头）；走自有链接的过渡期照旧用头 | 改写 |
| `HELM_HUMAN_TOKEN` 与 `require_role(Human)` | 人就是 `Principal::Device`（配对过的设备或带进程令牌的页面）；审计记 `CallContext::device()` | 删掉令牌，改判定 |
| `HELM_AGENT_TOKEN`、`hagt1.*` 签名令牌、`/api/v1/agent/*` 回调 | Agent 的写操作是**原生工具**，在 `helm-module` 里实现，工具代码以 Agent 身份直接调领域函数；宿主绑定账户与角色，模型改不了 | 删掉令牌与回调 |
| `HELM_SCHEDULER_TOKEN` | 调度是模块自己的后台任务，以 Scheduler 身份调领域函数 | 删掉令牌 |
| 其余 `HELM_*` 环境变量 | `[modules.helm]` 段；库地址、数据源密钥写 `{ credential = "…" }` | 改写 |
| `helm-agent`（`CoreHost`、`RecordedCoreHost`、`mcp.rs`、`policy.rs`、`callback.rs`） | Agix 实例：进程内模板 `helm.agent`，原生工具与场景经模板的 `EngineContrib`；MCP 客户端是 Agix 的；白名单是场景的 `tools()` 与权限规则；夹具改成 Agix 录像 `--replay-dir` | 删掉 |
| `helm-testkit` 的 `CoreHost` / `FakeCoreHost` | 不再需要；领域测试不经 Agent | 删掉（领域测试用到的夹具留着） |
| `contracts/operation-registry.yaml`、`OPERATIONS` | 保留为唯一真相；用它生成 `app.helm.*` 方法与原生工具（设计 11 §6.3 的「写工具由 Operation 自动生成」照做） | 保留 |
| `contracts/openapi.yaml` | 过渡期描述 `/apps/helm/api/v1`；方法化以后由方法的 `schema` 取代 | 保留，后改写 |
| 批准状态机（`approval_*` 表与事件） | 不动。另加 SDK §10.1 的卡片：条目 `approval` + 主题 `app.helm.approvals` + 只许 `Device` 的方法 `app.helm.approval_decide` | 保留 + 加卡片 |
| `python/ingest_worker` | 模块照旧起子进程（同一段 `invoke_worker`），路径与解释器来自 `[modules.helm]` | 保留 |
| `python/research_mcp` | Agix 实例的 stdio MCP 服务器，写进实例的设置层（§5 P4） | 保留，改接法 |
| `web/`（Vue、router、Pinia、`client.ts`、`events.ts`） | `spaces/helm/ui`（React）：`register()` 注册 L1… 的 space、页签、卡片、设置节；`client.ts` 的请求函数保留，传输换成宿主 API | 改写 |
| `LoginView.vue`、`stores/session.ts`（粘贴令牌） | Harnix 的准入（进程令牌、启动链接、设备配对） | 删掉 |
| `App.vue` 的顶部导航与事件条 | 外壳：左栏 space 列表，头像外圈表示连接 | 删掉 |
| `HomeView.vue`（健康、任务、审计） | `home.card`（待办、动态）＋ 运维信息进 `settings.section` 或系统 space（开发时） | 改写 |
| `W01View` / `W05View` / `W06View` / `W08View` | L1 / L3 / L5 / L6 space 的右栏页签（§5 P5 的映射表） | 改写 |
| CSS 变量（`--bg`、`--accent`…） | Harnix 的令牌（`--paper`、`--accent`、`--state-*`…），配色用 `r.tokens` 覆盖已有名字 | 改写 |
| 中文写死 | `r.strings("zh-CN", …)`、`r.strings("en", …)`，key 在 `helm.` 下 | 改写 |
| `desktop/`（加载本地 `web/dist`） | `harnix/templates/desktop` 的一份拷贝，本地模式起 `helm` 二进制；托盘、单实例、通知、更新只在 Rust 侧加 | 删掉，换模板 |
| CORS（`localhost:5173`、`tauri://`） | 同源，不需要 | 删掉 |
| `web/e2e/` Playwright | `@harnix/testkit`（API 层 + 界面层，打 `@feature` / `@layer` 标签） | 改写 |
| `helm-web/tests/api.rs` | 过渡期照旧测路由（鉴权一处要改，§5 P3）；方法化之后改成 testkit 的 API 层，或直接测领域函数 | 保留，逐步改写 |
| `ops/*.sh`、`ops/Makefile` | 路径改到新布局；`dev.sh`、`package.sh` 改调 `scripts/build.sh` | 改写 |
| T0–T3 工具档、「允许一次 / 总是允许此类 / 拒绝」 | Agix 的权限（Harnix 的权限卡已经是这三个按钮）；T3 不走权限，走批准卡 | 改写 |

---

## 4. 动手前要定的事

由 Helm 的维护者定，记进 Helm 的 `docs/design/07`（SD-02、OD-08）和 `CLAUDE.md` §3。每条都给了建议。

| # | 问题 | 建议 |
|---|---|---|
| D1 | Agent 还是不是独立进程？07 §5.1 要求「研究进程崩溃不得阻断人工批准」 | **进程内模板**（`TemplateCarrier::InProcess`）：只有它能带原生工具。Agix 的工具 panic 是 unwind，不会带走进程；带不走的是 abort 和内存耗尽。若必须进程隔离：改用 spawn 模板（daemon），写工具不能是原生的，只能经 MCP，由模块起自己的 MCP 监听（SDK §10.4），鉴权、幂等、审计都在 Harnix 之外自己做——成本高，不建议在 M0/M1 做 |
| D2 | 「L1–L6」用哪套编号 | 用业务的六层（`docs/baseline/01` §2.1：情报、知识、研究、量化、交易、复盘），不是技术架构的 L1–L8（`06` §43） |
| D3 | `/api/v1` 留多久 | 两步走：先整体挂到 `/apps/helm/api/v1`（界面先能用），再按页面逐个改成方法与主题。过渡期以后，`/apps/helm/` 只留确实需要 HTTP 形状的（事件回补、导出） |
| D4 | 人的鉴权 | 交给 Harnix：本机是进程令牌 / 启动链接，远程是 TLS + 设备配对。Helm 自己不再发人的令牌 |
| D5 | 数据放哪 | SQLite 默认放模块数据目录 `~/.helm/modules/helm/helm.db`，原始文档放 `~/.helm/modules/helm/data/`；Postgres 照旧，连接串走凭据引用 |
| D6 | 未来的 L2–L6 怎么占位 | 先只注册已经做出来的层（M0 只有 L1）。外壳的 space 没有「置灰 + 角标」（§6 缺口 G6）；要占位就注册一个 `center: "hidden"` 的 space，`overview()` 写「M1 开放」 |

---

## 5. 分步计划

每一步结束时仓库能构建、测试是绿的，可以单独提交。步骤里的命令都在 Helm 仓库根跑。

### P0 准备

1. 在 Helm 上开分支（例如 `harnix-0.4`）；记下当前 `ops/live.sh` 的结果作为基线。
2. 定 §4 的六件事，改 Helm 的 `CLAUDE.md` §3 和 `docs/design/07` §5.1。
3. 定 Harnix 钉哪个提交：钉最新的发布标签（`git -C harnix tag -l 'Harnix-v0.4.*'` 里最新的，
   `Harnix-v0.4.0-2026102603` 起都含 M8、M9），`agix/` 钉它记下的那个提交；`check-pins` 查两者一致。

**验收**：维护者签了 §4；`ops/live.sh` 的基线结果存档。

### P1 仓库布局

1. 加两个平级子仓：

   ```sh
   git submodule add https://github.com/openatta/Harnix.git harnix
   git -C harnix checkout <P0 定的提交或标签>
   git submodule add https://github.com/openatta/agix.git agix
   git -C agix checkout "$(git -C harnix ls-tree HEAD agix | awk '{print $3}')"   # 和 Harnix 钉的同一个
   ```

   **不要**初始化 `harnix/agix`。

2. Cargo workspace 挪到仓库根。新的根 `Cargo.toml`，照 `harnix/templates/app/Cargo.toml` 起头：

   ```toml
   [workspace]
   resolver = "2"
   members = ["app", "rust/crates/*", "harnix/crates/*"]
   exclude = ["agix", "harnix/agix", "harnix/crates/testapp", "desktop"]

   [workspace.package]            # Helm 自己的 crate 继承这段；Harnix 的 crate 不继承
   version = "0.1.5"
   edition = "2021"
   rust-version = "1.89"
   license = "AGPL-3.0-or-later"

   [workspace.dependencies]
   # BEGIN harnix — generated by harnix/scripts/sync-app-deps.mjs; do not edit
   # END harnix
   # Helm 自己的依赖写在区块下面

   [profile.release]
   strip = "symbols"
   lto = "thin"
   codegen-units = 1
   ```

   把 `rust/Cargo.toml` 的 `[workspace.dependencies]` 搬到区块下面，**和区块里重名的那几行删掉**
   （`tokio`、`axum`、`serde`、`serde_json`、`anyhow`、`tracing`、`uuid`、`sha2`、`hex`、`toml`、
   `reqwest` 等；同一张表里重名 TOML 会报错）。版本以区块的为准；要多的 feature 在 crate 里加
   `xxx = { workspace = true, features = [...] }`。删掉 `rust/Cargo.toml` 与 `rust/Cargo.lock`。

3. `node harnix/scripts/sync-app-deps.mjs .` 写区块，`node harnix/scripts/check-pins.mjs .`。
4. `ops/*.sh`、`ops/Makefile`、CI 里 `cd rust` 的地方改成仓库根。

**验收**：`cargo build --workspace`、`cargo test -p helm-web -p helm-platform` 和原来一样绿；
`sync-app-deps --check` 与 `check-pins` 通过；`sqlx` 和 Harnix 的依赖没有冲突（Harnix 本身不链
SQLite，这一步主要看 `cargo tree -d`）。

### P2 profile、品牌与目录

1. `profiles/helm.toml`（产品）和 `profiles/helm-dev.toml`（开发，多留系统 space）：

   ```toml
   [brand]
   name = "Helm"
   icon = "brand/helm.svg"          # 相对本文件
   product_id = "helm"              # 状态目录 ~/.helm

   [shell]
   spaces = []                      # 产品：内建的会话、项目、系统 space 都不要；helm-dev.toml 写 ["system"]

   [ui]
   runtime_modules = false          # 交易产品必须关（SDK §10.1 末尾）

   [engine]
   scene = "coding"                 # system 实例；Helm 的 space 不用它（P4）

   [transport]
   topologies = ["single_duplex", "split_streams"]
   host = "127.0.0.1"
   port = 8080

   [modules.helm]
   database_url = { credential = "helm/database-url" }   # 不给就用数据目录里的 helm.db
   python = "python3"
   python_path = "/opt/helm/python" # 写绝对路径：桌面壳起的进程工作目录是家目录
   stale_after_hours = 72
   job_timeout_secs = 60
   sec_user_agent = "Helm research <you@example.com>"
   ```

   不要写 `[system_space]`（已删）。`[modules.helm]` 的键由 Helm 的 `Config` 定；它也要
   `#[serde(deny_unknown_fields)]`，否则拼错的键悄悄不生效。

2. **数据目录**：Helm 以前的数据都在仓库目录里（`helm-dev.db`、`.local/`），Harnix 的规矩是
   `~/.helm/`。

   | 东西 | 以前 | 以后 |
   |---|---|---|
   | Harnix 的令牌、设备、`spaces.json`、实例账本 | 没有 | `~/.helm/`（Harnix 管） |
   | Helm 的 SQLite | `helm-dev.db`、`.local/helm.db` | `ctx.data_dir()/helm.db`，即 `~/.helm/modules/helm/helm.db` |
   | 原始文档与产物 | `.local/data` | `~/.helm/modules/helm/data/` |
   | Agent 实例的数据根（会话、设置、录像） | 没有 | `~/.helm/instances/<data_root>/`（Harnix 按模板建） |
   | Postgres | `HELM_DATABASE_URL` | 凭据 `helm/database-url`（默认读 `$HARNIX_CREDENTIAL_HELM_DATABASE_URL`） |

   开发库可以直接拷过去；`migrate()` 里**不要**自动去仓库目录找旧库。生产还没有数据，不需要迁移脚本。
   测试与多份并行：`HARNIX_STATE_DIR` 指到别处。

**验收**：P3 的二进制带这份 profile 起得来；`harnix.diagnostics` 里没有「profile 里没有对应模块的
`[modules.*]`」；`~/.helm/token` 生成，`~/Documents` 下没有新东西。

### P3 后端：一个模块 `helm`

1. 新 crate `rust/crates/helm-module`（包名随意，模块 id 是 `helm`），依赖 `harnix`、
   `helm-platform`、`helm-contracts`、`helm-risk`、`helm-exec`。把 `helm-web/src/lib.rs` 里与 HTTP 无关的
   部分（`AppState`、`invoke_worker`、`spawn_workers`）挪进来。
2. `app/`：bin crate `helm`。

   ```rust
   fn main() -> anyhow::Result<()> {
       harnix::App::builder()
           .name("helm", env!("CARGO_PKG_VERSION"))
           .with_builtins()
           .module(helm_module::Helm::default())
           .run()
   }
   ```

3. 模块的骨架：

   | 函数 | 做什么 |
   |---|---|
   | `manifest` | `Manifest::new("helm", env!("CARGO_PKG_VERSION")).disclosure("投资研究与交易：读写 Helm 的库，起 Python 采集任务，提供 L1… space")` |
   | `migrate` | 连库并跑 `db/migrations`（就是现在 `Database::connect` 里做的那一步），返回 `MigrationReport::applied(...)` |
   | `start` | 读 `Config`、连库、`cx.provide(State{..})`、起任务租约与 outbox 两个循环；失败就返回 `Err`（诊断页会写原因） |
   | `stop` | 停两个循环 |
   | `routes` | 过渡期：`r.serve(&[Principal::Device], helm_routes(state))`，即原来的 `/api/v1` 路由，挂在 `/apps/helm/api/v1/` |
   | `methods` | 从操作注册表生成，见下 |
   | `events` | 见第 6 条 |
   | `spaces`、`templates`、`engine` | P4 |

4. **鉴权换掉**：经 `/apps/helm/` 进来的请求，Harnix 已经按「只许 `Device`」准入过了，到了 Helm 的
   处理函数就是一个人。把 `require_role` 换成「Human 固定」；`Authorization` 头这时是 Harnix 的凭据，
   不能再拿去和 `HELM_HUMAN_TOKEN` 比。审计里的「谁」：自有链接的处理函数拿不到设备 id（§6 G4），
   过渡期记 `device:unknown`；方法里用 `CallContext::device()`。
5. **方法化**（可以和 P5 一页一页并行做）：每个 Human 能调的操作一个方法 `app.helm.<操作名>`，主体
   `Principal::Device`，`schema` 取注册表里的。参数里带 `intent_id`（取代 `Idempotency-Key`）。只许 Agent
   的操作**不**开成方法，它们是 P4 的原生工具。`approval.decide` 见 P6。
6. **事件**：先保留 `/apps/helm/api/v1/events` 与 `/events/ws`（WebSocket 在浏览器里带不了头，用
   `?credential=`，`host.space('helm').socket(path)` 会自动带）。界面上要「当前值」的东西改成主题：
   `app.helm.w01`（数据面新鲜度与信号计数）、`app.helm.approvals`、`app.helm.positions { account }` 之类，
   outbox 循环在相关事件之后重算并 `publish` 整值。
7. **Python**：`invoke_worker` 原样；解释器、`PYTHONPATH`、数据目录来自 `Config` 与 `data_dir()`。长任务照
   SDK §10.2：方法返回 `job_id`，进度推 `app.helm.job { job_id }`，先推一次初始值再交出 `job_id`。

**验收**：

- `cargo test` 绿；`helm-web/tests/api.rs` 改成对着 `helm_routes(state)` 测（只改了鉴权的夹具），全绿。
- `./target/release/helm --profile profiles/helm-dev.toml` 起来，`--version` 打印 Helm、Harnix、Agix、
  模块 API、贡献点 API、协议。
- 用 testkit 的 API 层（P7 的框架，先写一个用例也行）：`app.helm.*` 至少一个读、一个写（带 `intent_id`，
  重复调用幂等）；不带凭据 `GET /apps/helm/api/v1/tasks` 是 401；`harnix.diagnostics` 里模块 `helm` 启用、
  方法与路由都列出来。

### P4 Agent：模板、场景、原生工具、MCP

1. **模板**（`templates()`）：一个进程内模板 `agent`，全名 `helm.agent`：

   ```rust
   let mut contrib = EngineContrib::new();
   for scene in helm_scenes() { contrib.scene(scene); }          // 每层一个场景：helm-intel、helm-trade…
   contrib.customize_session(|b, _cx| b.native_tool(...));      // 从操作注册表生成的写工具
   t.add(InstanceTemplate::new("agent", TemplateCarrier::InProcess {
       engine: EngineProfile { scene: "helm-intel".into(), ..EngineProfile::default() },
       contrib,
   }).data_root("agent"));
   ```

   `start()` 里 `ctx.instances().ensure("helm.agent", "helm-agent", BTreeMap::new())`。
2. **场景**：每层一个 `AgentScene`（Agix 的 `base::interface::scene::AgentScene`；`helm-module` 依赖
   `base = { workspace = true }`，它在生成的区块里）：实现 `id`、`name`、`description`、
   `build_system_prompt`、`tools()`。`tools()` 就是现在 `policy.rs` 的白名单：读工具
   （`mcp__research__query`、`mcp__research__compute`）加这一层允许的写工具。
3. **原生工具**：从操作注册表里「Agent 可调」的操作生成（设计 11 §6.3 的混合承载照做）。工具的实现直接调
   领域函数，actor 固定是 Agent，账户域与用途由宿主绑定，模型给的参数里不能选角色或账户。`approval.*`
   一个都不生成（Helm `CLAUDE.md`：Agent 不得代批）。
4. **研究 MCP**：`research_mcp` 是 Agix 的 stdio MCP 服务器。`ensure` 之后，模块从
   `ctx.instance("helm-agent")` 拿 `data_root`，把下面这段写进 `<data_root>/settings.json`（Agix 的全局设置层）：

   ```json
   { "mcp_servers": { "research": { "command": "python3",
                                     "args": ["-m", "research_mcp", "--database", "sqlite://<数据目录>/helm.db"],
                                     "env": { "PYTHONPATH": "<python 目录>" } } } }
   ```

   `--database` 就是 `helm-agent/src/mcp.rs` 现在传的那个（不给时它读 `HELM_DATABASE_URL`，默认值指向仓库里的
   `.local/helm.db`，迁移后不对，一定要传）。键的确切形状照 `agix/crates/core/src/settings.rs` 的 `mcp_servers`（IPDrun 的 `ipdrun-agent/src/host.rs`
   `mcp_server_config` 是一份能跑的写法）。**在这台实例的第一个会话之前写**；验证用 `mcp.status`。
   厂商与密钥同理写进 `settings.local.json`（0600），密钥来自 `[modules.helm]` 的凭据引用；
   IPDrun 的 `ipdrun-agent/src/config.rs` 就是这么做的。
5. **space 的会话策略**（`spaces()`）：

   ```rust
   s.add(SpaceDecl::new("intel").instance("helm-agent").scene("helm-intel"));
   // 以后：knowledge / research / quant / trade / review，各自的场景，同一台 helm-agent
   ```

   写固定实例 id `helm-agent`（`start()` 里 `ensure` 出来的那台），**不要**写 `"template:helm.agent"`：
   那样每个 space 第一次建会话时各按模板建一台 `space-<space>`，六层就是六台实例，第 4 条的设置要写六份。
   space id 一旦有会话绑定就不要改（`spaces.json` 永久记着）。
6. **权限档**：T0 读工具在场景的权限规则里允许；T1、T2 走 Agix 的权限提问（Harnix 中区画权限卡，按钮是
   「允许一次 / 本会话内允许 / 总是允许（写进本项目的本地设置）/ 拒绝」，对应设计 11 的三个按钮）；T2「默认关
   并二次提示」要的话在 Helm 的工具实现里再拦一次。T3 不是工具权限，是 P6 的批准卡。
7. **录像**：`RecordedCoreHost` 的夹具（`fixtures/agent/sample-run.json`）不能直接用。确定性测试改用 Agix
   的录像：`--live` 录一次，之后 `--replay-dir <Helm 的录像目录>` 回放（P7）。

**验收**：

- `harnix.instances.list` 里有 `helm-agent`，`up`。
- 在 `intel` space 里建一个会话，`scene` 是 `helm-intel`，`mcp.status` 里 `research` 已连接。
- 让 Agent 调一次 `query`、一次 T1 写工具：T1 弹权限卡，同意后落库，审计里 actor 是 Agent。
- 让 Agent 尝试任何 `approval.*`：工具表里没有它。

### P5 界面：Vue → React

1. **两个 pnpm workspace，互不相交**（Harnix 的模板与 hello-app 就是这么构建的）：
   - `harnix/` 自己的 workspace：`pnpm --dir harnix install --frozen-lockfile` 装外壳、`@harnix/*` 和 Harnix 的
     Playwright，用 Harnix 自己的锁文件。外壳在这里构建，Vite 把 `react`、`react-dom`、`@harnix/react`、
     `@harnix/core` 去重到外壳那一份，所以 Helm 的界面入口在 Harnix 的 workspace 之外也能编进去。
   - Helm 根下自己的 workspace，**不**收 `harnix/ui/packages/*`（收了的话两次 install 会互相覆盖那几个包的
     `node_modules`）：

     ```yaml
     # pnpm-workspace.yaml
     packages:
       - spaces/*/ui
       - tests
     allowBuilds:
       esbuild: false
     ```

     根 `package.json` 写 `"packageManager": "pnpm@11.22.0"`（Harnix 用的版本）。

   新包 `spaces/helm/ui`（`@helm/ui`），照 `harnix/examples/hello-app/ui/package.json` 改两处：

   ```json
   {
     "name": "@helm/ui", "private": true, "type": "module",
     "exports": { ".": "./src/index.tsx" },
     "scripts": { "typecheck": "tsc -p tsconfig.json" },
     "dependencies": { "@harnix/react": "link:../../../harnix/ui/packages/react" },
     "peerDependencies": { "react": "^19.0.0" },
     "devDependencies": { "@types/react": "19.3.0", "react": "19.3.1", "typescript": "5.9.3" }
   }
   ```

   `@harnix/react` 用 `link:`（不是 `workspace:*`）：它的依赖从 Harnix 那边的 `node_modules` 解析；`react`、
   `@types/react` 的版本和 Harnix 写成一样，类型检查才不会出现两份 React 类型。其余第三方依赖（zustand、
   图表库）照常写进这个包，构建时从 Helm 的 `node_modules` 解析。`tsconfig.json` 照 hello-app 的，`extends`
   改成指向 `harnix/tsconfig.base.json`。

   这套摆法在外部应用上还没跑过（SDK 参考 §9），P5 的验收就是验证它。走不通时的退路：Helm 根的 workspace 收
   `harnix/ui/packages/*`、只做一次根下的 `pnpm install`，外壳用 `pnpm --filter @harnix/shell build` 构建，
   `allowBuilds`、`overrides`（`vite: 7.3.7`）抄 `harnix/pnpm-workspace.yaml` 的；这时就不要再
   `pnpm --dir harnix install`。
2. `scripts/build.sh`（照 `harnix/templates/app/scripts/build.sh` 与 `examples/hello-app/scripts/build.sh`）：

   ```sh
   #!/usr/bin/env bash
   set -euo pipefail
   cd "$(dirname "$0")/.."
   app="$PWD"
   release=""; [ "${1:-}" = "--release" ] && release="--release"
   node harnix/scripts/sync-app-deps.mjs . --check
   node harnix/scripts/check-pins.mjs . $release
   pnpm --dir harnix install --frozen-lockfile
   pnpm install --frozen-lockfile
   HARNIX_UI_MODULES="$app/spaces/helm/ui/src/index.tsx" HARNIX_UI_OUT="$app/target/ui" \
     pnpm --dir harnix --filter @harnix/shell build
   HARNIX_UI_DIST="$app/target/ui" cargo build --release -p helm
   node harnix/scripts/check-lock.mjs .
   ```

   Harnix 模板的 `build.sh` 会把 `spaces/*/ui/src/index.tsx` 按名字顺序自动接进来；钉到含这个修改的
   Harnix 之后，可以直接用模板的，再补上 `pnpm install` 与 `-p helm`。

3. `register(r, host)` 的内容：

   - **品牌**：`r.tokens({ "--accent": …, … }, "helm")` 只覆盖已有令牌名。Helm 设计 11 §2 的语义色对到
     Harnix 已有的名字：`--bg` → `--paper` / `--hx-bg`，`--bg-side` → `--paper-side`，`--surface` →
     `--paper-surface`，`--bubble` → `--paper-bubble`，`--line` → `--paper-line`，`--fg/-2/-3` →
     `--ink/-2/-3`，`--ok/--warn/--bad` → `--state-success/-warn/-error`。证据等级 E1–E4 用
     `--state-success`、`--state-info`、`--state-warn`、`--label-tertiary`（E4 加虚线边框）。外壳的配色本来
     就照 Helm 原型取的，多半只需要核对，不需要覆盖。
   - **文案**：`r.strings("zh-CN", { "helm.intel": "情报", … }, "helm")`，再来一份 `en`。组件里只写
     `host.t("helm.…")`。
   - **space**（id 与 P4 的 `SpaceDecl` 一致）：

     ```tsx
     r.register("space", {
       id: "intel", icon: "globe", order: 10,
       title: () => host.t("helm.intel"),
       subtitle: () => host.t("helm.intel.subtitle"),
       center: "chat",
       tabs: () => [...intelTabs(host), ...openedObjects()],   // 情报 / 数据源 / 任务 / 关注，加上打开的对象（最多 6 个）
       suggestions: () => [host.t("helm.intel.s1"), host.t("helm.intel.s2")],
       notice: () => host.t("helm.notice"),                    // 「Agent 只提案；写操作需确认；不构成投资建议」
     }, "helm");
     ```

   - **对话里的卡片**：情报卡、任务卡是工具结果的画法 → `tool.row`（`match` 工具名）；任务跑完往会话里留
     一张 → 条目 + `flow.block`（SDK §7.2）。点卡片打开右栏详情：`host.reveal({ kind: "helm.intel", id })`，
     由 `details.panel` 认领。
   - **HOME 的待办与动态** → `home.card`（在新对话的输入框下面）。
   - **设置**：自动确认策略、数据源凭据（只用安全表单，调只许 `Device` 的方法，绝不经对话）→
     `settings.section`。主题与语言由外壳管，不用再做。
4. 页面到 space 的映射（M0 只做 L1；其余随 Helm 的里程碑）：

   | Vue 现在 | 到哪 | 备注 |
   |---|---|---|
   | `W01View.vue`（数据面：新鲜度、带证据链的信号、待办） | `intel` 的「情报」「数据源」「任务」页签 | 订阅 `app.helm.w01`；采集按钮调 `app.helm.jobs_start`（名字按注册表） |
   | 设计 11 的「关注」页签 | `intel` 的「关注」页签 | 覆盖范围、目标公司、产业链节点是**条目**；要在左栏列就用 `sidebar()` |
   | `W05View.vue`（机会、门、证据） | `research` space（L3） | — |
   | `W06View.vue`（待批、D42 额度、订单、预占、持仓、成交、现金、对账） | `trade` space（L5）的「持仓」「批准」「风险」「账本」页签 | 807 行，拆成四个组件；「待人工决定」改成 P6 的批准卡与列表 |
   | `W08View.vue`（计划对实际、复盘、EXIT） | `review` space（L6） | — |
   | `HomeView.vue` | `home.card` ＋ `settings.section`「运维」 | 演示任务按钮只在开发 profile 里显示 |
   | `LoginView.vue`、`stores/session.ts`、`router.ts`、`App.vue` | 删掉 | 准入、路由、外框都是外壳的 |

5. **数据层**：`api/client.ts` 的请求函数留着，把底下的 `fetch(apiBase + path, { headers: { Authorization } })`
   换成：已方法化的走 `host.call("app.helm.<op>", params)`，还没方法化的走
   `host.space("helm").fetch("api/v1/<path>", init)`（凭据由宿主带）。`events.ts` 换成
   `host.space("helm").socket("api/v1/events/ws?after_seq=…")`；能改成主题的直接 `host.subscribe(...)`。
   `localStorage` 里的令牌、`sessionStorage` 里的 `intent_id` 键保留后者（幂等键还是要在客户端生成）。
   状态用组件内 state；跨组件要共享再自己依赖 zustand（装在 Helm 自己的包里）。
6. 规矩：颜色只用令牌、文案只用 key、交互元素带 `data-testid`、状态用 `data-state`；CSP 是
   `script-src 'self'; style-src 'self'`：不内联样式（`style={{...}}` 也算）、不 `eval`、不走 CDN。
   Harnix 的两条静态检查只查 Harnix 自己的包（§6 G7），Helm 要在自己的 CI 里加同样的检查。

**验收**：`pnpm -r typecheck` 过；`scripts/build.sh` 过；打开启动链接，左栏只有 Helm 的 space（产品
profile）或多一个系统 space（开发 profile），标题与图标是 Helm 的；`intel` 的四个页签有数据，改动经事件
实时刷新、不轮询；诊断页里没有被拒的贡献（令牌名、语言包 key 都合规）。

### P6 批准卡（SDK §10.1 落到 Helm 的状态机上）

Helm 的批准状态机不动，加一条「人在对话里批」的通道：

1. **请求**：`approval.request` 成功后，如果请求带了会话（Agent 在会话里提案、或人在某个会话里发起），
   `append_entry(&session, Entry::new("approval", json!({ request_id, title, summary, impact, deadline })))`。
2. **推送**：主题 `app.helm.approvals`（不带参数：全部待批，给 `trade` 的「批准」页签）和
   `app.helm.approvals { instance, session_id }`（这个会话的票，给卡片）。状态变了就推整值。
3. **回答**：方法 `app.helm.approval_decide { request_id, decision: "APPROVE" | "REJECT", reason, intent_id }`，
   **只许 `Principal::Device`**，内部调现有的 `decide_approval`，actor 是 Human，审计记
   `CallContext::device()`。引擎、别的模块、Helm 自己的模块都调不了。先答者生效（`decide_approval` 的
   `BEGIN IMMEDIATE` 本来就保证）。
4. **超时**：用现有的 `approval.expire`，由模块的调度循环在截止时间触发；卡片显示「已过期」。
5. **审计**：每个决定再写一条条目 `approval_decided`（哪张、结果、谁、哪台设备）；Helm 自己的
   `audit_log` 照写。
6. **画卡**：`flow.block` 认领 `ns === "app.helm"` 且 `event === "approval"` 的条目；卡片订阅带参数的主题，
   按钮调 `app.helm.approval_decide`。范例：`harnix/examples/hello-app/ui/src/index.tsx` 的 `ApprovalCard`。
7. profile 的 `[ui] runtime_modules = false` 必须在：「只许 `Device`」挡不住和页面同级的第三方界面模块。

**验收**（testkit，P7）：模块主体调 `app.helm.approval_decide` 被拒（`REFUSED`）；超时后卡片是「已过期」，
之后的回答被拒；两台设备同时答，先到的生效，另一台被拒；Agent 的工具表里没有任何 `approval.*`；
`audit_log` 与条目都能看到是谁、哪台设备。

### P7 测试换成 `@harnix/testkit`

1. 根下 `tests/`（pnpm 包，`@helm/tests`）：依赖 `"@harnix/testkit": "link:../harnix/ui/packages/testkit"`，
   **不**自己装 `@playwright/test`。用 Harnix 那一份 Playwright 跑：
   `pnpm --dir harnix exec playwright test -c "$PWD/tests/playwright.config.ts"`。用例只从 `@harnix/testkit`
   引 `test`、`expect`（它再导出 Playwright 的），配置文件对 `@playwright/test` 只做 `import type`：这样全程只有
   一份 Playwright（两份会报「`@playwright/test` 被加载了两次」）。配置照 `harnix/tests/live/playwright.config.ts`。
   若 P5 走了退路（根 workspace 收 `harnix/ui/packages/*`），这里就写 `workspace:*`，`@playwright/test` 装
   `1.62.1`，直接 `pnpm exec playwright test`。
2. 起进程：

   ```ts
   import { startHarnix, test, expect } from "@harnix/testkit";
   const backend = await startHarnix({
     bin: `${APP}/target/release/helm`,          // 一定写绝对路径，否则 testkit 会去 harnix/ 里 cargo build
     uiDir: `${APP}/target/ui`,
     profile: HELM_PROFILE_WITHOUT_TRANSPORT,    // testkit 自己写 [transport]，这里只给 [brand] [shell] [modules.helm]…
     args: ["--replay-dir", `${APP}/tests/recordings`],
     env: { HARNIX_CREDENTIAL_HELM_DATABASE_URL: "…" },   // 需要时
   });
   ```

   每个用例自己的 HOME、数据根、端口；状态目录在 `<scratch>/home/.helm`（testkit 从 profile 里的
   `product_id` 找令牌）。
3. 用例打标签 `@feature:<功能>`、`@layer:api|ui`。先移这几组：P3 的方法与路由、P6 的批准四个负例、
   `web/e2e/w01`、`w06` 的主流程（改成界面层，组件对象照 `data-testid`）。
4. 录像：Agent 的用例 `--live` 录一次（要 `.env` 里的模型凭据；testkit 只读 Harnix 根的 `.env`，Helm
   用 `env` 选项传，§6 G8），录像放 `tests/recordings`，之后都回放。
5. Python 测试、领域的 Rust 测试照旧。`ops/live.sh` 改成：`scripts/build.sh` → `cargo test` → pytest →
   Playwright（testkit）。

**验收**：`ops/live.sh` 全绿；回放模式离线、确定；覆盖表能从标签生成。

### P8 桌面壳

1. 删掉 `desktop/`，拷一份 `harnix/templates/desktop` 到 `desktop/`（它自成 Cargo workspace，已在 P1
   的 `exclude` 里）。
2. 改名：`src-tauri/Cargo.toml` 的 `name = "helm-desktop"`；`tauri.conf.json` 的 `productName = "Helm"`、
   `identifier = "com.helm.desktop"`（沿用旧的，用户配置目录不变）、`version`；`src/config.rs` 的
   `ENV_PREFIX = "HELM_DESKTOP"`、`SIDECAR = "helm"`；`dist/` 的标题与文案。图标：`cargo tauri icon`。
3. sidecar：`target/release/helm` 按三元组拷进 `src-tauri/binaries/`，`bundle.externalBin = ["binaries/helm"]`。
   随包带 `desktop.toml`：`mode = "local"`、`[local] profile = "helm.toml"`（profile 一起打包）。
4. 旧壳的托盘、单实例、通知、更新检查照模板 README「可以加的」在 Rust 侧加回来；`capabilities/` 不给页面
   任何权限，`withGlobalTauri` 保持 `false`。
5. `ops/package.sh --tauri` 改成：`scripts/build.sh --release` → 拷 sidecar → `cargo tauri build`。

**验收**：`node harnix/templates/desktop/check-no-ipc.mjs desktop` 通过；
`HELM_DESKTOP_PROGRAM=$PWD/target/release/helm ./desktop/src-tauri/target/debug/helm-desktop --smoke`
退出码 0；打开窗口，Web Inspector 的 Network 里只有对 `127.0.0.1` 的 HTTP 与 WebSocket；关窗口后 `helm`
进程退出（`kill -9` 壳也一样，靠 `--exit-with-stdin`）。

### P9 清理

1. 删 `rust/crates/helm-web`（剩下的路由已经搬进 `helm-module`）、`rust/crates/helm-agent`、
   `helm-testkit` 的 `CoreHost` 部分、`web/`、`ops/e2e-server.sh`、`ops/.env.example` 里的令牌项。
2. 自有链接只留确实需要 HTTP 形状的（事件回补、审计导出）；其余都是方法与主题。`contracts/openapi.yaml`
   只描述剩下的路由。
3. 文档：Helm 的 `docs/design/07`、`08/02-agent-runtime.md`、`10`、`11` 改成「Harnix 应用」的说法；
   `README.md` 写新的构建与运行。
4. 发布前：`check-pins --release`（`agix/` 必须和 Harnix 钉的一样）、`check-lock`、`ops/live.sh`。
5. 构建缓存：Harnix + Agix 的 release 构建很占盘，收尾时清 `target/`。

**验收**：仓库里没有 Vue、没有 `HELM_*_TOKEN`、没有 `/api/v1/agent/*`；`check-pins --release` 通过，两个
子仓都指向上游存在的提交，没有补丁；`ops/live.sh` 全绿。

---

## 6. 风险与缺口

### 6.1 Harnix / Agix 现在没有的，Helm 先怎么顶

| # | 缺口 | 对 Helm 的影响 | 先怎么做 | 什么时候补 |
|---|---|---|---|---|
| G1 | 批准原语不进 Harnix | 要自己做卡片与通道 | P6，照 SDK §10.1；状态机本来就是 Helm 的 | 不做（D26：两个应用都做完、形状一致才考虑） |
| G2 | 主题只推整值快照，没有追加模式 | `event_stream` 的 `seq` 增量推不进主题 | 过渡期留自有链接的 WebSocket；界面要的东西改成读模型快照主题 | 0.4 之后，有真实需求再做 |
| G3 | 没有 MCP 服务端、角色令牌 | 设计 07 想让外部 Agent（Claude Desktop）只读接入 | 外部只读接入继续用 `research_mcp`（它本来就是独立的 MCP 服务）；要开写操作就在模块 `start()` 里起自己的监听（SDK §10.4），安全自己负责 | 0.4 之后 |
| G4 | 自有链接的处理函数拿不到调用者是谁 | 过渡期 `/apps/helm/` 的审计记不到设备 id | 审计要求高的操作先方法化（方法有 `CallContext::device()`） | 可回推（§6.2） |
| G5 | 模块不能声明「这台实例的 Agix 设置」 | MCP 服务器、厂商、权限规则要模块自己写进实例数据根的设置文件 | P4 第 4 条；第一个会话之前写，`mcp.status` 验证 | 可回推（§6.2） |
| G6 | space 没有「置灰 + 角标」 | 设计 11 的 L2–L6 占位做不出原样 | 不注册，或注册 `center: "hidden"` 的占位 space | 看 Helm 是否坚持要 |
| G7 | 「只用令牌」「只用 key」两条静态检查只查 Harnix 自己的包 | Helm 的界面包没人拦 | Helm 的 CI 自己加（照 `harnix/tests/style-lint.mjs` 的规则写一份） | 可回推 |
| G8 | testkit 的 live 模式只读 Harnix 根的 `.env`，且只认 Anthropic 的变量 | Helm 录像时模型凭据要自己传 | `startHarnix({ env })`；`modelMode()` 不可靠时自己判 | 可回推 |
| G9 | 应用自己的 pnpm workspace 摆法没在外部应用上验证过 | P5 可能卡在依赖解析 | P5 用 Harnix 模板的两个 workspace 的做法，并写了退路（根 workspace 收 `harnix/ui/packages/*`） | M9 由两个消费者定下来 |
| G10 | `6fe3687` 的 `templates/app/scripts/build.sh` 没接 `HARNIX_UI_MODULES` | 照那一版的模板抄会编不进界面 | 用 P5 的 `build.sh` | Harnix 已在改（按 `spaces/*/ui/src/index.tsx` 自动接），随下一次发布 |
| G11 | M8、M9-1 还没进发布标签 | 钉提交而不是标签 | P0 第 3 条 | 下一次 Harnix 发布 |
| G12 | 迁移没有 `--dry-run` 与备份钩子 | 生产 Postgres 升级前没有预演 | `migrate` 里自己先备份（`ops/backup-restore.sh` 的做法），或升级前手工跑 | 0.4 之后（D29） |
| G13 | 进程内实例和 Harnix 同进程 | Agent 侧 abort / 内存耗尽会带走批准与账本 | §4 D1；设 `session_cap`、模型 `max_tokens`；真要隔离走 daemon + MCP | 设计如此 |

### 6.2 回推 Harnix 的候选

两个应用（Helm、IPDrun）都在用同一种形状自己做的东西。先在 Helm 里做，等 IPDrun 也做完、形状一致，再提给
Harnix（SDK §2）。

| 候选 | Helm 怎么做 | IPDrun 怎么做 | 进 Harnix 的样子 |
|---|---|---|---|
| 带 `seq` 的领域事件流 | `event_stream` + HTTP 回补 + WS | 65 种事件经 `/api/ws` | 主题的追加模式，复用中枢的 `seq` 追赶 |
| 操作注册表 → 方法 + 工具 + MCP | `operation-registry.yaml` 生成方法与原生工具 | `OpTable` 生成 `/api/op` 与 MCP 工具，按角色过滤 | 方法注册表投影成 MCP 工具与角色令牌（已在「0.4 之后」） |
| 方法的幂等键 | `Idempotency-Key` → `intent_id` | `OpSpec.idempotency` | 方法声明 `.idempotent()`，中枢按键去重 |
| 模块给实例写 Agix 设置 | 写 `mcp_servers`、厂商到 `<data_root>/settings*.json` | 同样写厂商与 `mcp_servers` | `EngineContrib` / 模板上声明设置，Harnix 在建实例时写 |
| 人在对话里点头 | 批准卡 | 提议卡、决策卡（只许人的操作） | 批准原语（D26） |
| 自有链接拿到调用者 | 审计要设备 id | — | 准入时把 `Subject` 放进请求的 extensions（小改动） |
| 应用的界面静态检查 | 自己写 | 自己写 | `style-lint`、i18n 检查接受应用的包目录 |
