# IPDrun 迁移到 Harnix 0.4

> 读者：在 IPDrun 仓库里做这次迁移的人，或者 Agent。照本文做，配合 [二开指南](harnix-sdk.md)、
> [SDK 参考](harnix-sdk-reference.md) 和能跑的范例 `harnix/examples/hello-app`，不需要问 Harnix 的作者。
>
> **对照的 Harnix**：`main` 上的 0.4（M0–M9：M8 是外壳框架、运行期界面模块 v2、品牌，M9-1 是桌面壳模板），
> 从 `Harnix-v0.4.0-2026102603` 起的发布标签都含这些。Agix 钉在 `Agix-v0.2.9-2026102603`。
> 二开指南与 SDK 参考已经对齐 M8、M9；本文 §1 再把 M8 的变化集中列一遍，方便对照。几份文档对不上时，
> 以 Harnix 的代码为准。
>
> **对照的 IPDrun**：分支 `mvp` 的 `0a76860`（阶段 4 验收、阶段 5 范围定稿，`v0.1.4`），工作区干净。
> 下文的路径都相对 IPDrun 仓库根。IPDrun 这边的总账是 `docs/nest-sync.md`（Harnix 仓库里那份
> `iprun-issue-01-sync.md` 是它的副本）。
>
> 本文只写在 Harnix 仓库里。**Harnix 这边不改 IPDrun 的任何文件**；改 IPDrun 是 IPDrun 自己的事。
>
> 用词照 [术语](glossary.md)：「实例」只指 Agix 实例；IPDrun 的工位、会话 agent 都是**会话**（无头会话），
> 不是实例。「插件」只指 Agix 的包；IPDrun 的后端是**模块**。

---

## 0. 结论先行

1. IPDrun 是 `nest-sync.md` 的「阶段二」：删掉 `vendor/nest`，改成依赖 Harnix 的基点版本。Harnix 0.4 已经
   按新结构重新实现了 `vendor/nest` 里的改动（N-xxx）和大部分需求（R-xx），**那些改动不合并**，IPDrun 改调用。
2. 要换掉的是三层壳：**装配**（`ipdrun-server` 照抄的那份、`ipdrun-agent` 自建的引擎与中枢）、**Agent 宿主**
   （`AttaHost` 从 `nest_hub::Hub` 换到 `ctx.sessions()`）、**界面外框**（`apps/web` 自己的三栏壳换成 Harnix
   外壳里的 space）。领域 crate（core、db、store、git、exec、scan、xflow、api、orch）基本不动。
3. 两件事 0.4 不提供，IPDrun 继续自己做：**MCP 服务端与角色令牌**（但要从同一端口挪到模块自己起的监听上）、
   **领域事件流**（65 种增量事件，挪到模块的自有链接上）。
4. 现在这棵树在这台机器上**编不过**：根 `Cargo.toml` 指向 `../Core`（AttaCore），那个目录已经不在了
   （`.claude/worktrees/Core` 也是断链）。所以没有「先保证旧的能编、再一步步换」的余地：P1 一开始就把依赖
   AttaCore 的 crate 移出 workspace，领域 crate 先在新布局上编过。
5. AttaCore 已改名 Agix，旧数据不迁（维护者 2026-10-25）：`~/.ipdrun/agent/atta/` 里的会话不会被恢复；
   代码里的 `.atta/`、`ATTA_CONFIG_HOME` 要改成 `.agix/`、`AGIX_CONFIG_HOME`。

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
| `tabs` 可以是函数 | `tabs: () => [...]`，每次绘制都读：IPDrun 的「条目栈」（最多三层）直接变成页签 |
| `SpaceContribution.sessions` | 这个界面 space 列出、新建的是另一个后端 space 的会话 |
| `r.tokens({...}, owner)` | 只许覆盖外壳已有的令牌名 |
| `r.strings(locale, {...}, owner)` | 语言包，key 必须在 `<owner>.` 下；`"zh-CN"` 或 `"en"` |
| `home.card` | HOME 没有了；卡片显示在**新对话的输入框下面** |
| 贡献点 API 2 | 运行期包的界面模块是预编译的 ES 模块，从 import map 引 `react`、`react-dom`、`react/jsx-runtime`、`@harnix/react`。IPDrun 的界面编进二进制，不受影响 |

**后端**：`harnix.process`、`harnix.system.trace`（一个会话每次模型调用的输入与输出）、主二进制的
`--exit-with-stdin`。

**桌面**：`harnix/templates/desktop`（Tauri 2，本地 / 远程两种模式，没有 IPC）。它就是 IPDrun `apps/tauri`
（P9 探针）的正式版：同样用 `POST /auth/link` 换一次性链接，取代 `apps/tauri`。

---

## 2. 现状盘点

### 2.1 IPDrun 现在有什么（与 Harnix 相关的部分）

| 面 | 现在 | 在哪 |
|---|---|---|
| workspace | 成员 `crates/*`、`vendor/nest/crates/*`；排除 `apps/tauri`；`rust-version = "1.88"`，AGPL-3.0-only | `Cargo.toml` |
| AttaCore | 路径依赖 `../Core/...`（`daemon`、`base`、`model`、`hooks`、`mcp`、`runtime`、`session`、`scene`…），用的是 `813dded`（0.2.6）；**目录已不在** | `Cargo.toml`、`Cargo.lock`（`daemon 0.2.6-2026102001`） |
| Nest 副本 | `vendor/nest`：`git archive 9854e11`（0.3.0）的拷贝，220 个跟踪文件，8 个 crate（`nest`、`nest-assembly`、`nest-authz`、`nest-builtin`、`nest-contract`、`nest-contrib`、`nest-hub`、`nest-transport`）、原生 JS 界面、测试；13 个文件带 `[IPDrun N-xxx]` | `vendor/nest/VENDORED.md`、`docs/nest-sync.md` §4 |
| 照抄的装配 | `ipdrun_server::start()` 照 `vendor/nest/crates/app/src/main.rs` 的顺序：引擎 → Hub → Gate → `nest_transport::router` → merge `/api/*`、`/mcp` → `axum::serve`；去掉了 Nest 的授权表与内置方法 | `crates/ipdrun-server/src/lib.rs:92 serve`、`:311 start` |
| 自建引擎与中枢 | `AttaHost::new` 调 `nest_assembly::build_engine`、`Hub::new(engine, Registry::new(), None, limits)` | `crates/ipdrun-agent/src/host.rs` |
| Agent 宿主 | `AttaHost` 在无头会话上实现 `AgentHost`（N-101 `create/open/send/interrupt/close_headless`、N-102 tap、N-103/N-108 `OnBusy`）；`ChatHost`（会话 agent）在它之上；每个会话一个私有项目根 `<sessions_dir>/<CV>`，项目层设置里写 `prompt_append`（场景提示）与 `mcp_servers.ipdrun`（会话令牌）；厂商与密钥写进引擎数据根的 `settings.json` / `settings.local.json`（0600） | `crates/ipdrun-agent/src/{host,chat,config,policy,events,guard}.rs` |
| 场景 | `Scene` 枚举 9 个（analyst、architect、dev、tester、attributor、scheduler、integrator、observer、process）；提示在 `pack/scenes/*.md`（7 个），因为当时 AttaCore 注册不了宿主场景（A11），用项目层 `prompt_append` 注入；analyst、architect 跑在引擎的 `coding` 场景 | `crates/ipdrun-core/src/scene.rs`、`pack/scenes/` |
| hook | 二进制自己的 `pre-tool`、`user-prompt` 子命令（`IPDRUN_HOOK_BIN`）：工具白名单与审计 | `crates/ipdrun/src/hook.rs`、`ipdrun-agent/src/policy.rs` |
| 操作 | 自己的 `OpTable`，约 200 个操作（`xreqm`、`xspec`、`xarch`、`xissue`、`xtalk`、`xflow`、`xorch`、`xrun`、`ipdrun`），带角色矩阵（Read / Propose / Direct）、`human_only`、幂等、长任务；唯一分发 `Api::dispatch(ctx, op, input)` | `crates/ipdrun-api/src/{table,dispatch,ctx}.rs`、各 `ops` |
| 接口 | `POST/GET /api/op/{name}`、`GET /api/ws`（事件与 call 帧）、`GET /api/info`、`POST /api/link`、`/mcp`、`/mcp/{*rest}`；testkit 构建多 `/test/*` | `crates/ipdrun-server/src/http.rs:178`、`ws.rs`、`testkit.rs` |
| 事件 | 65 种领域事件（`EVENT_TYPES`），outbox，经 `/api/ws` 推增量；会话流走 Nest 的 `/ws` | `crates/ipdrun-core/src/event.rs`、`ipdrun-db` |
| MCP | `ipdrun-mcp`（rmcp 3.5，streamable HTTP）：操作注册表 → MCP 工具，`/mcp/<组>`，`TokenResolver`、`ToolFilter`；**和 Nest 合在同一个端口上** | `crates/ipdrun-mcp` |
| 令牌 | owner 令牌 `~/.ipdrun/owner.token`（CLI、`/api/link`）；页面令牌 = human；`/mcp` 认会话令牌、工位令牌、外部令牌（`ipdrun_ext_…`、`ipdrun_obs_…`，库里存 SHA-256） | `crates/ipdrun-api/src/tokens.rs`、`ipdrun-server/src/home.rs` |
| 二进制 | `ipdrun`：手写的参数解析，子命令 `serve`、`init`、`open`、`check`、`mcp-config`、`version`，加上 hook 的 `pre-tool`、`user-prompt`；启动打印 `IPDRUN_READY {json}`；默认端口 47100 | `crates/ipdrun/src/main.rs` |
| 目录 | `IPDRUN_HOME`（默认 `~/.ipdrun/`）：`owner.token`、`run.json`、`agent/`（AttaCore 数据根）、`tls/`。项目里：`<项目>/ipdrun/`（提交进库的内容：`project.yaml`、`reqm/`、`spec/`、`arch/`…）、`<项目>/.ipdrun/`（忽略：`state.sqlite`、`sessions/CV-*`、`wt/_content` 内容工作树、可选 `models.yaml`） | `crates/ipdrun-server/src/home.rs`、`ipdrun-store` |
| 内容包 | `pack/`（场景、briefs、flows、gates、schemas、skills，44 个文件），`include_dir!` 进二进制 | `crates/ipdrun-xflow/src/defs.rs:13` |
| 前端包 | `@nest/client`（约 1k 行，协议客户端）、`@nest/ui`（约 5k 行，`ChatPanel` 等会话组件）；Harnix 的 `@harnix/core`、`@harnix/react` 就是从它们来的（D21） | `packages/nest-client`、`packages/nest-ui` |
| 界面 | `apps/web`：React 19 + Vite 7.3.7 + TanStack Query，约 26k 行；**自己的三栏壳**（`shell/`）与「类目」接口（`CategoryDef`：`page` / `conversation` / `list` / `layer`）；9 个类目：home、reqs、features、arch、dev、integ、issues、flows、settings；由 `--ui-dir` 整体替换 Nest 的界面 | `apps/web/src/shell/category.ts`、`route.ts` |
| 桌面 | `apps/tauri`（`ipdrun-shell`，P9 探针）：起 `nest` 二进制、读 `~/Documents/.nest` 的令牌、`POST /auth/link`；**没有接到 `ipdrun serve`** | `apps/tauri/src-tauri/src/{main,service}.rs` |
| 测试 | Rust：`crates/ipdrun/tests`（48 个文件，含 `api/`、`deepseek_stage1..4`、`layering.rs`）等；剧本模型（本机回环上的 Anthropic 兼容端点，按 playbook 回复）；`qa/`（Playwright 1.63）：`e2e:nest`（对 vendor 的 `nest`）、`e2e:stage1..4`（对 `ipdrun serve` + 剧本）；没有 CI 配置，`scripts/check.sh` 本地跑 | `crates/ipdrun-testkit/src/scripted_model.rs`、`qa/kit/*`、`qa/ui/*` |
| 许可 | IPDrun AGPL；`deny.toml` 允许宽松许可 + 自家 AGPL；`scripts/check-licenses.mjs` 查 npm | `deny.toml` |

### 2.2 早先按 issue 写的对照表哪里不对

| §14.2 写的 | 实际 |
|---|---|
| 「删掉 `vendor/harnix`」 | 目录是 **`vendor/nest`**（Nest 0.3.0 的 `git archive`，不是子仓）。没有 `vendor/harnix`、`vendor/agix` |
| 「N-004、N-005 包元数据、Agix 路径依赖」 | N-005 指的是 `../Core`（AttaCore 0.2.6），**目录已经不在**；不是 Agix |
| 「R-06 `packages/nest-client` → `@harnix/core/client`」 | 包名是 `@nest/client`；对。另外它还导出协议类型（→ `@harnix/core/protocol`） |
| 「R-06 `packages/nest-ui`（ChatPanel）」 | 包名 `@nest/ui`；`apps/web` 还用了 `NestUiProvider`、`SessionList`、`ChatComposer`、`TranscriptList`、`SegmentRow`、`ComposerNote`、`Avatar`、`useHello`、`useConnection`、`readDraft/writeDraft`、`lineDiff/hunks`、`markdown`、`Icon`——`@harnix/react` 都有同名导出（`NestUiProvider` → `HarnixUiProvider`，但 space 里用不着它） |
| 「R-07 `apps/tauri`：M9 之前先留着」 | M9-1 已经有 `templates/desktop`；`apps/tauri` 现在本来就没接到 `ipdrun serve`，直接换 |
| 「R-10 `ipdrun-server` 照抄的装配」 | 对，在 `crates/ipdrun-server/src/lib.rs`。**漏了第二份**：`crates/ipdrun-agent/src/host.rs` 的 `AttaHost::new` 自己建引擎和 `Hub` |
| 「`/api/op`、`/api/ws` → `app.ipdrun.*` 方法与主题」 | `/api/op` 可以；`/api/ws` 推的是**增量事件**，主题只推整值快照，不能一对一。先放自有链接（§5 P3） |
| 「R-04、R-05：在模块 `start()` 里起自己的监听」 | 对。现在 `/mcp` 和界面在**同一个端口**，挪到单独端口后，会话的 `mcp_servers.ipdrun.url`、`mcp-config` 生成的配置都要跟着改 |
| 「9 个 scene」 | `Scene` 枚举 9 个，`pack/scenes/` 只有 7 份提示；而且现在是经 `prompt_append` 注入的，不是引擎场景。Agix 0.2.6 起能注册宿主场景（A11 已落地），可以改成 `EngineContrib::scene` |
| 没提 | `IPDRUN_HOME` 与 `~/.<product_id>` 撞名（§5 P2）、`<项目>/.ipdrun/` 违反「项目数据放 `.agix/`」、CLI 子命令与 Harnix 的命令行冲突、hook 子命令、剧本模型、`apps/web` 自己的壳、`qa/` 的 `nest-server.ts`、`.atta` → `.agix` 改名 |

---

## 3. 对照表

动作：**保留**、**改写**、**删掉**、**等**（等 Harnix 或 Agix）。

| IPDrun 现在 | Harnix 0.4 里 | 动作 |
|---|---|---|
| `vendor/nest/` 整个目录 | `harnix/` 子仓 | 删掉（P8） |
| 根 `Cargo.toml` 的 `../Core/...` 与 `nest-*` 依赖 | 生成区块（`sync-app-deps`），`agix/` 平级子仓 | 改写 |
| N-001、N-007 装配调 `assemble`；N-002、N-105、N-106、N-120、N-121 中枢修正 | Harnix 内部（M0） | 删掉 |
| N-004 crate 元数据写死 | Harnix 的正式做法 | 删掉（随 vendor 一起） |
| N-101 `create/open/send/interrupt/close_headless` | `ctx.sessions().create / open / send / interrupt / close` | 改写调用 |
| N-101 `SessionSpec`（scene、项目根、模型、权限模式与规则） | `harnix::SessionSpec { space, instance, scene, project_root, provider, model, permission_mode, permission_rules, output_schema, recording }` | 改写；**每个都写 `space` 和 `instance`** |
| N-102 `Hub::tap` | `ctx.sessions().tap(&s)`：有界（默认 512），**会收到 `TapEvent::Lagged { last_seq }`**，之后 `tap.catch_up()`；帧是 `{method, params}`，方法名是 `harnix.event` / `harnix.turn_settled` / `harnix.queue`（原来是 `nest.*`） | 改写，加 `Lagged` 处理 |
| N-103、N-108 `OnBusy::{Queue, Reject, Interrupt, Steer}` | `harnix::OnBusy`，同名 | 改 import |
| N-104 `EngineConfig.cwd` | `SessionSpec.project_root`；实例的数据根 | 改写 |
| N-107 按会话 provider、`output_schema` | `SessionSpec.provider`、`.output_schema` | 改写 |
| `is_running` | 从 tap 的帧自己记（`harnix.turn_settled` 之前是在跑）；界面用主题 `harnix.sessions.status` | 改写 |
| N-301 启动链接、N-302 令牌 0600、N-303 连接日志、N-601 Origin 白名单、N-602 Secure cookie | Harnix 内部：`--launch-link` / `[transport] launch_link`、`extra_origins`、`--tls-cert/--tls-key` | 删掉，改配置 |
| R-10 `ipdrun_server::serve/start` 照抄的装配 | `crates/ipdrun/src/main.rs`：子命令分派 + `harnix::App::builder()` | 删掉照抄 |
| `AttaHost::new` 自建引擎与 `Hub` | 进程内模板 `ipdrun.agents`，模块 `start()` 里 `ensure` | 删掉自建 |
| `ipdrun-server` 的 `state.rs`（接线 `register_flow` 等）、`http.rs`、`ws.rs`、`models.rs`、`config.rs`、`testkit.rs` | 模块 crate（`ipdrun-server` 改成模块，建议改名 `ipdrun-module`，模块 id `ipdrun`） | 改写 |
| `OpTable` + `Api::dispatch` | 保留为唯一分发；人能调的每个操作注册成方法 `app.ipdrun.<操作名>`（`Principal::Device`），handler 调 `dispatch(Ctx{ role: Human, … })` | 保留 + 接线 |
| `POST/GET /api/op/{name}` | 过渡期：`/apps/ipdrun/api/op/{name}`（`routes()`）；之后界面全走方法 | 改写，后删掉 |
| `GET /api/ws`（65 种增量事件、call 帧） | 自有链接 `/apps/ipdrun/ws`（`host.space("ipdrun").socket("ws")`）；call 帧改成方法；界面要「当前值」的改成主题 | 改写 |
| `GET /api/info` | 方法 `app.ipdrun.info`（或 `harnix.hello` + `harnix.diagnostics`） | 改写 |
| `POST /api/link`、`owner.token` | Harnix 的 `POST /auth/link` 与 `~/.ipdrun/token` | 删掉 |
| `/mcp`、`/mcp/<组>`（同端口） | 模块 `start()` 里自己起的监听，**单独端口**（`[modules.ipdrun] mcp_port`），令牌照旧由 `TokenResolver` 解析（SDK §10.4） | 改写（等 Harnix 0.4 之后的 MCP 再接管） |
| R-04 角色令牌（会话、工位、外部） | 0.4 不提供 | 保留在 IPDrun |
| 9 个场景（`prompt_append` 注入） | 第一步照旧；之后可改成 `EngineContrib::scene`（宿主场景） | 保留，可选改写 |
| hook 子命令 `pre-tool`、`user-prompt` | 照旧（`main.rs` 在交给 Harnix 之前分派）；以后可改成 `customize_session` 里的进程内 hook（Agix A18） | 保留 |
| 厂商与密钥写进引擎数据根 | 写进 `ctx.instance("ipdrun-agents").data_root` 下的 `settings.json` / `settings.local.json` | 改路径 |
| `IPDRUN_HOME` / `~/.ipdrun/` | `[brand] product_id = "ipdrun"` → Harnix 的状态目录也是 `~/.ipdrun/`；IPDrun 自己的东西进 `~/.ipdrun/modules/ipdrun/` | 改写（P2） |
| `<项目>/.ipdrun/` | `<项目>/.agix/ipdrun/` | 改写（P2，注意工作树） |
| `<项目>/ipdrun/`（提交进库的内容） | 不变：这是项目内容，不是隐藏的产品目录 | 保留 |
| `pack/`、xflow、scenes 内容、sqlite 运行态、git 原语、代码扫描 | 留在 IPDrun | 保留 |
| `@nest/client` | `@harnix/core`（`/client`、`/protocol`、`/state`）；space 里一般用不到，用 `useNest()` | 删掉 |
| `@nest/ui` | `@harnix/react`（对话主干同源）；产品面（语音、上下文环、快捷回复）在 IPDrun 里包一层 | 删掉 |
| `apps/web` 的壳（`shell/`、`route.ts`、`SessionsShell.tsx`、`App.tsx`、`main.tsx`） | Harnix 外壳 | 删掉 |
| `apps/web` 的 9 个类目 | 8 个 space + `settings.section`（§5 P5 的映射） | 改写 |
| `apps/tauri` | `harnix/templates/desktop` 的一份拷贝 | 删掉，换模板 |
| `qa/kit/nest-server.ts`、`ipdrun-server.ts`、`nest-api.ts` | `@harnix/testkit`（`startHarnix`、`connect`） | 改写 |
| 剧本模型（`--scripted`） | 照旧：模块在 testkit feature 下起它，并把它写成厂商（P4） | 保留，改接法 |
| R-08 录像回放、R-09 `data-testid` | `--replay-dir`；组件对象约定（`data-testid`、`data-state`） | 改写 |
| R-11、R-12 会话运行状态、hello 里的超时 | 主题 `harnix.sessions.status`；`harnix.hello` 的 limits | 改订阅 |
| R-13 content-type | Harnix 内部 | 删掉 |

---

## 4. 动手前要定的事

由 IPDrun 的维护者定，记进 IPDrun 的 `docs/architecture/decisions.md`（接在 XD71 之后）。

| # | 问题 | 建议 |
|---|---|---|
| D1 | 一个进程开一个项目（`serve <路径>`），还是像 Harnix 那样在界面里选项目 | 保留「一个进程一个项目」：项目路径由 `ipdrun serve <路径>` 带进来（P3 第 2 条），也可以在界面里 `ipdrun_project_open`（none 模式照旧）。Harnix 的「项目」space 去掉 |
| D2 | 保留哪些内建 space | 产品 `[shell] spaces = []`；开发 profile `["system"]`（看实例、会话、任务、每次模型调用的输入输出，排查工位很有用） |
| D3 | 会话放哪台实例 | 一台进程内实例 `ipdrun-agents`（模板 `ipdrun.agents`），所有类目与工位共用。要和引擎崩溃隔离再考虑 daemon（但那样原生贡献带不进去） |
| D4 | 事件流 | 第一步整套搬到自有链接；第二步把界面要的「列表 / 计数 / 状态」做成主题快照，增量只留给需要逐条的地方（时间线） |
| D5 | MCP 端口 | 单独端口，默认 `47101`（`[modules.ipdrun] mcp_port`）；只绑回环，除非显式配 |
| D6 | 运行期包 | IPDrun 不装第三方界面模块：`[ui] runtime_modules = false`。提议卡、决策卡这类「只许人」的操作挡不住同页的第三方代码 |

---

## 5. 分步计划

每一步结束时仓库能构建、测试是绿的（P1 只要求领域 crate），可以单独提交。命令都在 IPDrun 仓库根跑。

### P0 准备

1. 从 `mvp` 开分支（例如 `harnix-0.4`）。阶段 4 的验收记录（`cf32754`）就是行为基线；现在这棵树编不过
   （`../Core` 不在），不要花时间去恢复它。
2. 定 §4 的六件事。
3. 定 Harnix 钉哪个提交：钉最新的发布标签（`git -C harnix tag -l 'Harnix-v0.4.*'` 里最新的，
   `Harnix-v0.4.0-2026102603` 起都含 M8、M9），`agix/` 钉它记下的那个提交；`check-pins` 查两者一致。

**验收**：维护者签了 §4；`docs/nest-sync.md` 加一行「阶段二开始」。

### P1 仓库布局与依赖

1. 加两个平级子仓：

   ```sh
   git submodule add https://github.com/openatta/Harnix.git harnix
   git -C harnix checkout <P0 定的提交或标签>
   git submodule add https://github.com/openatta/agix.git agix
   git -C agix checkout "$(git -C harnix ls-tree HEAD agix | awk '{print $3}')"   # 和 Harnix 钉的同一个
   ```

   **不要**初始化 `harnix/agix`。删掉断链 `.claude/worktrees/Core`。
2. 根 `Cargo.toml`：

   ```toml
   [workspace]
   resolver = "2"
   members = ["crates/*", "harnix/crates/*"]
   exclude = ["agix", "harnix/agix", "harnix/crates/testapp", "apps/tauri", "desktop", "vendor",
              # P1 期间：还依赖 nest-* 的三个，P3/P4 改完再放回来
              "crates/ipdrun", "crates/ipdrun-server", "crates/ipdrun-agent"]

   [workspace.package]            # IPDrun 自己的 crate 继承；Harnix 的 crate 不继承（它们写死了 Apache-2.0）
   version = "0.1.4"
   edition = "2021"
   rust-version = "1.88"
   license = "AGPL-3.0-only"

   [workspace.dependencies]
   # BEGIN harnix — generated by harnix/scripts/sync-app-deps.mjs; do not edit
   # END harnix
   # IPDrun 自己的依赖（ipdrun-* 路径、rusqlite、gix、tree-sitter、rmcp 3.5…）写在区块下面
   ```

   删掉所有 `nest-* = { path = "vendor/nest/..." }` 和 `../Core/...` 的行；和区块重名的第三方依赖
   （`tokio`、`axum`、`serde`、`serde_json`、`anyhow`、`tracing`、`uuid`、`sha2`、`toml`、`clap`、`tempfile`…）
   删掉 IPDrun 那一行，版本以区块为准，要多的 feature 在 crate 里加。`members` 用 glob 时被 `exclude` 的目录
   不会进来；`vendor` 整个排除。
   `ipdrun-testkit` 留在 workspace 里（`ipdrun-orch` 的 dev-dependency 要它），但它的 dev-dependencies
   `ipdrun-agent`、`nest-hub` 先删掉，用到它们的测试（`hub_steer`、`chat_host`、`station_host`、`station_bash`、
   `session_provider`）先挪出 `tests/`，P4 改写后放回：成员的 dev-dependency 也要能解析，指向被排除、又依赖
   `nest-*` 的 crate 会让整个 workspace 解析失败。
3. `node harnix/scripts/sync-app-deps.mjs .`，`node harnix/scripts/check-pins.mjs .`。
4. 领域 crate 里的 AttaCore 时代名字改掉：`.atta` → `.agix`、`ATTA_CONFIG_HOME` → `AGIX_CONFIG_HOME`、
   注释里的 AttaCore → Agix（`grep -rn '\.atta\b\|ATTA_\|AttaCore' crates`，现在约 30 个文件，其中
   `ipdrun-store/src/init.rs`、`ipdrun-xflow/src/{gates,defs,validate}.rs` 是代码，其余多是注释和测试）。
5. `crates/ipdrun/tests/layering.rs` 允许的依赖边：去掉 `nest-*`，加 `harnix`（只许模块 crate 和 `ipdrun`
   依赖它；领域 crate 不许）。

**验收**：`cargo test -p ipdrun-core -p ipdrun-db -p ipdrun-store -p ipdrun-git -p ipdrun-exec -p ipdrun-scan
-p ipdrun-xflow -p ipdrun-api -p ipdrun-orch -p ipdrun-mcp -p ipdrun-testkit` 全绿；`sync-app-deps --check`、`check-pins`
通过；`cargo deny check licenses` 通过（Harnix 与 Agix 的 crate 是 Apache-2.0）。

### P2 profile、品牌与目录

1. `profiles/ipdrun.toml`（产品）与 `profiles/ipdrun-dev.toml`（开发）：

   ```toml
   [brand]
   name = "IPDrun"
   icon = "brand/ipdrun.svg"
   product_id = "ipdrun"             # 状态目录 ~/.ipdrun

   [shell]
   spaces = []                       # ipdrun-dev.toml 写 ["system"]

   [ui]
   runtime_modules = false

   [engine]
   scene = "coding"                  # system 实例；IPDrun 的会话不在它上面（P4）

   [transport]
   topologies = ["single_duplex", "split_streams"]
   host = "127.0.0.1"
   port = 47100
   launch_link = true                # 页面里不放令牌（N-301 的默认，桌面壳也要它）

   [modules.ipdrun]
   mcp_port = 47101
   # project = "/path/to/project"   # 不写：由 `ipdrun serve <路径>` 带进来，或在界面里打开
   # models = "/path/to/models.yaml"
   ```

   不要写 `[system_space]`。`[modules.ipdrun]` 的 `Config` 加 `#[serde(deny_unknown_fields)]`。
   局域网（原来的 `--lan`）：`host` 写局域网地址，配 `--tls-cert/--tls-key`，`extra_origins` 写页面的来源，
   设备走配对；Harnix 一个进程只监听一个地址。

2. **家目录**。`product_id = "ipdrun"` 让 Harnix 的状态目录正好是 IPDrun 现在的 `IPDRUN_HOME`：

   | 东西 | 以前（`~/.ipdrun/`） | 以后 |
   |---|---|---|
   | owner 令牌 | `owner.token` | 删掉；Harnix 的进程令牌是 `~/.ipdrun/token`（0600）。CLI 要令牌就读它 |
   | 运行记录 | `run.json`（pid、端口、url、项目） | 模块写 `~/.ipdrun/modules/ipdrun/run.json`（P3 第 2 条） |
   | AttaCore 数据根 | `agent/`（含 `atta/` 会话、设置） | 不再用。Agix 不读 AttaCore 的旧会话；厂商设置由模块重写进 `~/.ipdrun/instances/agents/`（P4）。旧目录留给用户自己删 |
   | 局域网证书 | `tls/` | 留在 `~/.ipdrun/modules/ipdrun/tls/`，启动时用 `--tls-cert/--tls-key` 指过去 |
   | Harnix 自己的 | 没有 | `token`、`state.json`、`spaces.json`、`instances.json`、`instances/`、`modules/`、`uploads/` |
   | 覆盖整个目录 | `IPDRUN_HOME` | `HARNIX_STATE_DIR`（`IPDRUN_HOME` 可以在 `main.rs` 里映射过去，兼容一段时间） |

3. **项目目录**。Harnix 的规矩：项目自己的数据放 `<项目>/.agix/`，不另开 `<项目>/.<产品>/`。

   | 以前 | 以后 |
   |---|---|
   | `<项目>/.ipdrun/state.sqlite` | `<项目>/.agix/ipdrun/state.sqlite` |
   | `<项目>/.ipdrun/sessions/CV-*`（私有项目根，里面是 `.atta/`） | `<项目>/.agix/ipdrun/sessions/CV-*`（里面是 `.agix/`） |
   | `<项目>/.ipdrun/wt/_content`（内容工作树） | `<项目>/.agix/ipdrun/wt/_content` |
   | `<项目>/.ipdrun/models.yaml` | `<项目>/.agix/ipdrun/models.yaml` |
   | `<项目>/ipdrun/`（提交进库的内容） | 不变 |

   在 `ipdrun_project_open` 里做一次性的搬家：`.agix/ipdrun/` 不在而 `.ipdrun/` 在时搬过去。**工作树不能
   直接 `mv`**（`.git/worktrees/` 里记着绝对路径），用 `git worktree move .ipdrun/wt/_content
   .agix/ipdrun/wt/_content`，或删掉后让 `ipdrun-store` 重建。`sessions/CV-*` 里的旧会话是 AttaCore 的，
   恢复不了，`ChatHost` 的「失效 ⇒ 新建」会接住；库里的会话号照旧可用作条目的键。`ipdrun init` 生成的
   `.gitignore` 改成忽略 `.agix/`。这一条 IPDrun 维护者若决定不挪，Harnix 不检查它，但同一台机器上装别的
   Agix 应用时 `.agix/` 才是大家共同认的地方。

**验收**：P3 的二进制带这份 profile 起得来；`harnix.diagnostics` 没有拼错的 `[modules.*]`；打开一个旧项目，
`.ipdrun/` 搬到了 `.agix/ipdrun/`，`git worktree list` 指向新位置，`ipdrun check` 通过。

### P3 后端：模块 `ipdrun` 与二进制

1. **模块 crate**：`crates/ipdrun-server` 改成模块（建议改名 `crates/ipdrun-module`），依赖 `harnix`，不再依赖
   任何 `nest-*`。删掉 `lib.rs` 的 `serve` / `start` 装配；`state.rs` 的接线（`register_flow`、`register_work`、
   `bind_sessions`…）挪进模块的 `start()`。

   | 函数 | 做什么 |
   |---|---|
   | `manifest` | `Manifest::new("ipdrun", env!("CARGO_PKG_VERSION")).disclosure("IPD 开发：读写项目的 ipdrun/ 内容与 .agix/ipdrun/ 运行态，起工位会话，开 MCP 监听（端口见配置）")` |
   | `migrate` | 什么都不做：IPDrun 的库在项目里，打开项目时迁（`ipdrun-db` 现在就是这样） |
   | `start` | 读 `Config`；`ensure` 实例（P4）；建 `App`、`OpTable`、`Api`，`cx.provide(...)`；有项目就打开（none / ready 两种模式照旧）；起 outbox 推送、编排器、MCP 监听；写 `run.json` |
   | `stop` | 停编排器、MCP 监听，删 `run.json` |
   | `methods` | 见第 3 条 |
   | `events` | 第一步不声明；第二步按 §4 D4 加 |
   | `routes` | `r.serve(&[Principal::Device], router)`：`/ws`（原 `/api/ws`）、过渡期的 `/api/op/{name}`；testkit 构建再加 `/test/*` |
   | `spaces` | P5 的 8 个 space，`instance("ipdrun-agents")`，各自的场景 |
   | `templates` | P4 |

2. **二进制** `crates/ipdrun/src/main.rs`：Harnix 的命令行不认 IPDrun 的子命令，所以先分派自己的，剩下的
   交给 Harnix。

   ```rust
   fn main() -> anyhow::Result<()> {
       let args: Vec<String> = std::env::args().collect();
       if let Some(code) = hook::dispatch(&args) { std::process::exit(code); }   // pre-tool、user-prompt
       match args.get(1).map(String::as_str) {
           Some("init") => return cli::init(&args[2..]),
           Some("open") => return cli::open(&args[2..]),          // 读 ~/.ipdrun/token 与 run.json，POST /auth/link
           Some("check") => return cli::check(&args[2..]),
           Some("mcp-config") => return cli::mcp_config(&args[2..]),
           _ => {}
       }
       // `ipdrun serve [路径] [Harnix 的开关…]`：路径交给模块，其余原样给 Harnix
       let (project, rest) = cli::split_serve(&args);
       if let Some(p) = project { std::env::set_var("IPDRUN_PROJECT", p); }    // 模块的 Config 读它
       harnix::App::builder()
           .name("ipdrun", env!("CARGO_PKG_VERSION"))
           .with_builtins()
           .module(ipdrun_module::Ipdrun::default())
           .args(rest)                     // rest[0] 是程序名（clap 要它），serve 与项目路径已拿掉
           .run()
   }
   ```

   Harnix 自己的子命令（`daemon`、`ui export`）要原样透传：spawn 模板起的子进程跑的就是本二进制的 `daemon`。
   旧开关的去处：`--port`、`--ui-dir`、`--tls-cert`、`--tls-key` 是 Harnix 同名的；`--models` →
   `[modules.ipdrun] models`；`--lan` → profile 的 `[transport]`（P2）；`--scripted` → `[modules.ipdrun]
   playbook`（只在 testkit 构建里认）。`--profile` 默认值：没给就用随包的 `profiles/ipdrun.toml`（在
   `split_serve` 里补上）。`IPDRUN_READY {json}` 这一行不再需要：testkit 和桌面壳都有自己的就绪判断；
   要保留给脚本用，就在 `start()` 末尾打印。
3. **操作 → 方法**。启动时遍历 `OpTable`：

   ```rust
   fn methods(&self, m: &mut MethodRegistry) {
       for name in ops::table().names() {
           let spec = ops::table().spec(name).unwrap();
           if spec.access(Role::Human).is_none() { continue; }           // 人看不到的不开
           let (input, _) = ops::table().schemas(name).unwrap();
           m.method(name, Principal::Device)
               .describe(spec.doc)
               .schema(input.clone())
               .raw(move |cx, params| async move {
                   let api = cx.state::<Arc<Api>>()?;
                   let ctx = Ctx::human(api.app().clone(), Transport::Harnix, cx.device());
                   api.dispatch(ctx, name, params).await.map(into_value).map_err(into_rpc_error)
               });
       }
   }
   ```

   `methods()` 在 `start()` 之前调，所以表要能在没有 `App` 的时候建出来（只要规格，不要状态）；状态在 handler
   里从 `cx.state()` 拿。`ipdrun-api/src/ctx.rs` 的 `Transport` 加 `Harnix`、`Origin` 加
   `Device { id }`，`dispatch.rs` 的 `human_origin` 认这个组合（Harnix 已经保证只有 `Device` 进得来）。幂等键、
   长任务的 `job_id` 都在 `params` 里，照旧。`OpError` 的码映射成 `RpcError`（`not_found` / `hidden` 都用
   `-32601`，权限用 `REFUSED`）。方法全名是 `app.ipdrun.<操作名>`，例如 `app.ipdrun.xreqm_item_create`。
4. **事件流**。`ws.rs` 挪到 `routes()` 下的 `/ws`：浏览器用 `host.space("ipdrun").socket("ws")` 连（凭据由宿主
   带在 `?credential=` 上）。原来 `/api/ws` 上的 `call` 帧不要了，界面改调方法；`seq` 与断线回补照旧由 IPDrun
   自己做（自有链接没有 Harnix 的追赶）。
5. **MCP**。`ipdrun-mcp` 的路由不再 merge 进主端口：`start()` 里在 `127.0.0.1:<mcp_port>` 上单独
   `axum::serve`，`stop()` 里关。`TokenResolver`、`ToolFilter`、`/mcp/<组>` 不变。跟着改的：会话 agent 写进
   私有项目根的 `mcp_servers.ipdrun.url`、`ipdrun mcp-config` 生成的地址。MCP 工具的执行仍走
   `Api::dispatch`（角色来自令牌），与 Harnix 的授权无关，安全由 IPDrun 负责（SDK §10.4）。
6. **会话运行状态**（R-11）：界面订阅 `harnix.sessions.status`；模块侧从 tap 的帧自己记。

**验收**（P6 的 testkit 框架，先写这几个用例）：

- `ipdrun --version` 打印 IPDrun、Harnix、Agix、模块 API、贡献点 API、协议。
- `ipdrun serve <deployer-mini>` 起来；`harnix.diagnostics` 里模块 `ipdrun` 启用，方法约 200 个都列出来。
- `app.ipdrun.ipdrun_project_get` 返回项目；一个写操作带幂等键重复调，只落一次；`human_only` 的操作用模块
  主体调被拒。
- 不带凭据连 `/apps/ipdrun/ws` 是 401；带了能收到 `system.started` 之后的事件。
- 外部令牌连 `http://127.0.0.1:47101/mcp`，`tools/list` 按角色过滤，和以前一致。
- `ipdrun init`、`check`、`mcp-config` 照旧；`ipdrun open` 打开的是 Harnix 的启动链接。

### P4 Agent 宿主：从 `nest_hub` 换到 `ctx.sessions()`

1. **实例**（`templates()` + `start()`）：

   ```rust
   fn templates(&self, t: &mut InstanceTemplates) {
       let mut contrib = EngineContrib::new();
       // 第一步不加东西：场景照旧走 prompt_append，hook 照旧走二进制
       t.add(InstanceTemplate::new("agents", TemplateCarrier::InProcess {
           engine: EngineProfile { scene: "coding".into(), session_cap: 64, ..EngineProfile::default() },
           contrib,
       }).data_root("agents"));
   }
   // start():
   cx.instances().ensure("ipdrun.agents", "ipdrun-agents", BTreeMap::new()).await?;
   let agents = cx.instance("ipdrun-agents").expect("ensured");
   ```

   `session_cap` 按同时在跑的工位数定（默认 32）。space 声明写固定实例 id `instance("ipdrun-agents")`，**不要**
   写 `"template:ipdrun.agents"`：那样每个 space 各建一台 `space-<space>`，厂商设置要写八份。
2. **厂商设置**：`config.rs` 的 `HostConfig.data_root` 改成 `agents.data_root`（即
   `~/.ipdrun/instances/agents/`）；`settings.json` / `settings.local.json`（0600）的写法不变。在这台实例的第一个
   会话之前写；`models.yaml` 变了重写，并对已有会话按现在的配置重设（`ChatHost` 现在的「恢复时重设厂商与
   模型」照旧）。
3. **`AttaHost` 改写**：持有 `harnix::Sessions`（`cx.sessions()`）而不是 `Arc<Hub>`。

   | 以前（`nest_hub`） | 以后（`harnix`） |
   |---|---|
   | `hub.create_headless(SessionSpec{..})` | `sessions.create(SessionSpec { space: Some(<类目的 space>), instance: Some("ipdrun-agents".into()), scene, project_root: Some(<私有根或工作树>), provider, model, permission_mode, permission_rules, output_schema, ..Default::default() })` |
   | `open_headless(id)` | `sessions.open("ipdrun-agents", id)` |
   | `send_headless(id, msg, on_busy)` | `sessions.send(&s, text, on_busy)`：**只收文本**；`InputMessage` 里的附件要先另想办法（§6 G6） |
   | `interrupt_headless` / `close_headless` | `sessions.interrupt(&s)` / `sessions.close(&s)` |
   | `hub.tap(id)` → 通道 | `sessions.tap(&s)` → `Tap`：`next()` 给 `TapEvent::Frame(json)` 或 `TapEvent::Lagged { last_seq }` |
   | 帧 `nest.event` / `nest.turn_settled` / `nest.queue` | `harnix.event` / `harnix.turn_settled` / `harnix.queue`，`params` 形状照旧（以 `@harnix/core/protocol` 的生成类型为准） |
   | `is_running` | 自己从帧记：收到 `user_message`（或 `run_turn` 的应答）到 `harnix.turn_settled` 之间是在跑 |

   **`Lagged` 必须处理**：工位一轮跑很久、帧很多（`tool_args_delta`）时 tap 会落后。收到 `Lagged` 就
   `tap.catch_up()`，返回值里的 `history_total` 之前的部分用 `ctx.call("session.history", …)` 补；`events.rs`
   的 `TurnTracker` 要能接受「中间这段是补来的」。也可以 `tap_with_capacity(&s, 4096)` 减少落后，但不能代替处理。
   `nest-sync` 记的「tap 对 `open_turn` 的 `user_message` 收两次」在 Harnix 的 tap 上要重新验一遍。
4. **space**：每个会话都绑一个 space（建了就不能换）。类目会话、条目会话绑到它所在类目的 space（`reqs`、
   `features`…）；工位会话绑到 `dev`（开发、测试工位）或 `integ`（集成工位）。不写 `space` 会落到内建的会话
   space，而产品 profile 里它不显示。
5. **场景**（可以放到 P8 之后）：Agix 0.2.6 起能注册宿主场景。把 `pack/scenes/*.md` 做成 `AgentScene`，
   `contrib.scene(...)` 加进模板；`tools()` 就是 `policy.rs` 的白名单（MCP 工具名 `mcp__ipdrun__<操作>` 照旧）。
   这样可以去掉 `prompt_append` 的注入，以及「analyst、architect 只能跑在 coding 场景上」的退路（attacore-020
   是否已修，先在 Agix 里核对）。
6. **hook**：`IPDRUN_HOOK_BIN` 指向 `ipdrun` 二进制本身，P3 的 `main.rs` 已先分派 `pre-tool`、`user-prompt`，
   不用改。以后想去掉子进程，改成 `customize_session` 里的进程内 hook（Agix A18 已落地）。
7. **剧本模型**：testkit 构建里，`start()` 读到 `[modules.ipdrun] playbook` 就起
   `ipdrun_testkit::scripted_model`，把它写成厂商（`api_type: anthropic`、`base_url`、`API_KEY`、`MODEL`），
   和现在 `load_models` 做的一样。`ipdrun-testkit` 依赖 `nest-hub` 的测试（`hub_steer`、`chat_host`、
   `station_host`…）改成对 `harnix` 的 `Sessions`。

**验收**：`crates/ipdrun-agent` 放回 workspace，`ipdrun-testkit` 挪出去的测试改写后放回，`cargo test` 全绿；`stage1-api`、`stage2-api`
的剧本跑通（会话 agent 经 MCP 落提议、工位跑完一轮、`OnBusy::Steer` 在步骤边界送达）；工位的会话在系统 space
（开发 profile）里按实例 → 会话 → 任务看得到；人为把 tap 容量调到 8，长工位一轮不丢事件（`Lagged` 路径被走到）。

### P5 界面：`apps/web` → space

1. **包**。`packages/nest-client`、`packages/nest-ui` 删掉。`apps/web` 改成一个界面包（建议搬到
   `spaces/ipdrun/ui`，包名 `@ipdrun/ui`），不再有自己的 `vite.config`、`index.html`、`main.tsx`。

   **两个 pnpm workspace，互不相交**（Harnix 的模板与 hello-app 就是这么构建的）：
   - `harnix/` 自己的 workspace：`pnpm --dir harnix install --frozen-lockfile` 装外壳、`@harnix/*` 和 Harnix 的
     Playwright，用 Harnix 的锁文件。外壳在这里构建，Vite 把 `react`、`react-dom`、`@harnix/react`、
     `@harnix/core` 去重到外壳那一份，所以 IPDrun 的界面入口在 Harnix 的 workspace 之外也能编进去。
   - IPDrun 根下自己的 workspace，**不**收 `harnix/ui/packages/*`（收了的话两次 install 会互相覆盖那几个包的
     `node_modules`）：

     ```yaml
     packages:
       - spaces/*/ui
       - qa
     allowBuilds:
       '@swc/core': false
       esbuild: false
     overrides:
       vite: 7.3.7
     ```

   `spaces/ipdrun/ui/package.json`：

   ```json
   {
     "name": "@ipdrun/ui", "private": true, "type": "module", "license": "AGPL-3.0-only",
     "exports": { ".": "./src/index.tsx" },
     "scripts": { "typecheck": "tsc -p tsconfig.json", "test": "vitest run" },
     "dependencies": { "@harnix/react": "link:../../../harnix/ui/packages/react", "@tanstack/react-query": "5.104.4" },
     "peerDependencies": { "react": "^19.0.0" },
     "devDependencies": { "@types/react": "19.3.0", "react": "19.3.1", "react-dom": "19.3.1",
                          "typescript": "5.9.3", "vitest": "5.0.5", "jsdom": "30.1.3", "@testing-library/react": "16.3.3" }
   }
   ```

   `@harnix/react` 用 `link:`（不是 `workspace:*`）：它的依赖从 Harnix 那边的 `node_modules` 解析；`react`、
   `@types/react` 的版本和 Harnix 写成一样（IPDrun 本来就是同一版），类型检查与 Vitest 里才只有一份 React。
   `tsconfig.json` 的 `extends` 指向 `harnix/tsconfig.base.json`（或保留 IPDrun 自己的 `tsconfig.base.json`）。
   `scripts/check-licenses.mjs` 把 `harnix/` 那边装进来的依赖也算进去。

   这套摆法在外部应用上还没跑过（SDK 参考 §9），P5 的验收就是验证它。走不通时的退路：IPDrun 根的 workspace 收
   `harnix/ui/packages/*`、只做一次根下的 `pnpm install`，`@harnix/react` 写 `workspace:*`，外壳用
   `pnpm --filter @harnix/shell build` 构建；这时就不要再 `pnpm --dir harnix install`。
2. **`scripts/build.sh`**（照 `harnix/templates/app/scripts/build.sh` 与 `examples/hello-app/scripts/build.sh`）：

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
   HARNIX_UI_MODULES="$app/spaces/ipdrun/ui/src/index.tsx" HARNIX_UI_OUT="$app/target/ui" \
     pnpm --dir harnix --filter @harnix/shell build
   HARNIX_UI_DIST="$app/target/ui" cargo build --release -p ipdrun ${IPDRUN_FEATURES:+--features "$IPDRUN_FEATURES"}
   node harnix/scripts/check-lock.mjs .
   ```

   testkit 构建：`IPDRUN_FEATURES=testkit scripts/build.sh`（同一份界面）。`scripts/run-local.sh` 改调它。
   Harnix 模板的 `build.sh` 会把 `spaces/*/ui/src/index.tsx` 按名字顺序自动接进来；钉到含这个修改的
   Harnix 之后可以直接用模板的，再补上 `pnpm install` 与 `-p ipdrun`。
3. **类目 → space**。IPDrun 的 `CategoryDef` 几乎就是 space 的形状：

   | `CategoryDef` | space |
   |---|---|
   | `conversation(ctx)`（类目会话或条目会话，跟着栈走） | `center: "chat"`；打开条目时 `host.session.open(条目的会话)`，回到列表时开回类目会话。会话号由 `xtalk_session_*` 给 |
   | `list(ctx)`（右栏底层） | 第一个页签 |
   | `layer(id, ctx, depth)`（栈，最多三层） | `tabs: () => [列表, ...栈里的层]`；面包屑就是页签条 |
   | `page(ctx)`（首页、设置：整页、没有对话） | `center: "hidden"` + `overview()` |
   | 勾选（带进类目会话） | 页签里的组件自己管，发送前 `host.composer.insert(...)` 带上 |
   | 工位转录（`TranscriptList`） | `ChatPanel` 的转录样子，传 `(instance, session)`，放在 `dev` / `integ` 的页签里 |

   | 类目 | space id | `center` | 说明 |
   |---|---|---|---|
   | home（仪表盘） | `home` | `"hidden"` | `overview()` 是现在的首页；也可以拆几张 `home.card` |
   | reqs | `reqs` | `"chat"` | 场景 analyst |
   | features | `features` | `"chat"` | — |
   | arch | `arch` | `"chat"` | 场景 architect |
   | dev | `dev` | `"chat"` | 没有类目会话（B-25）：不选工位时中区显示空状态，或 `"minimized"` + 工位总览 |
   | integ | `integ` | `"chat"` | 场景 integrator；用例页的会话是 attributor |
   | issues | `issues` | `"chat"` | 场景 attributor |
   | flows | `flows` | `"chat"` | xflow 的流程、门、技能 |
   | settings | — | — | 拆成 `settings.section`（模型、令牌、项目配置…），进头像里的设置 |

   space id 与 P3 的 `SpaceDecl` 一致；一旦有会话绑定就不要改。
4. **组件换包**：`@nest/ui` 的 import 全换成 `@harnix/react`（同名：`ChatPanel`、`ChatComposer`、
   `TranscriptList`、`SessionList`、`ConnectionBadge`、`SegmentRow`、`ComposerNote`、`Avatar`、`Icon`、
   `markdown`、`lineDiff`、`hunks`、`readDraft`、`writeDraft`、`useHello`、`useConnection`、`useStrings`，类型
   `ToolRowContribution`、`UserRowContribution`、`ChatCommand`、`QuickReply`…）。`NestUiProvider` 不要了，
   外壳已经提供；`@nest/client` 的直接连接不要了，一律经 `useNest()`。语音、上下文环、快捷回复这些 IPDrun 的
   产品面留在 IPDrun 的包里包一层，不往 `ChatPanel` 里加。
5. **数据层**：`apps/web/src/api/client.ts` 的 `op(name, input)` 换成 `host.call("app.ipdrun." + name, input)`；
   `events.ts` 换成 `host.space("ipdrun").socket("ws")`，收到事件照旧让 TanStack Query 失效对应的查询。
   `VITE_NEST_TOKEN`、`NEST_URL` 代理、`GET /api/info` 的探测都删掉。
6. **对话里的卡片**：提议卡、决策卡、任务卡如果是工具结果的画法 → `tool.row`；如果是模块往会话里写的 → 条目
   + `flow.block`（`append_entry`，SDK §7.2）。决策、合并这类「只许人」的回答照 SDK §10.1：方法只许 `Device`，
   状态经主题推，卡片订阅（IPDrun 的提议 / 决策状态机不动）。
7. **外观与文案**：`nest-` 前缀的类名与 `--nest-*` 令牌换成 Harnix 的（`@harnix/react/styles.css` 的
   `--hx-*` 与语义令牌）；IPDrun 的配色用 `r.tokens({...}, "ipdrun")` 覆盖**已有**令牌名；文案
   `r.strings("zh-CN", { "ipdrun.reqs": "需求", … }, "ipdrun")` 和一份 `en`。CSP 是
   `script-src 'self'; style-src 'self'`：没有内联样式、`eval`、CDN。Harnix 的两条静态检查只查 Harnix 自己的包，
   IPDrun 自己加（§6 G8）。

**验收**：`pnpm -r typecheck`、`pnpm -r test`（Vitest）过；`scripts/build.sh` 过；打开启动链接，左栏是 IPDrun 的
8 个 space（产品 profile），标题与图标是 IPDrun 的；每个类目的列表、条目栈、类目会话、工位转录都能用；事件到了
列表刷新；诊断页没有被拒的贡献。

### P6 测试换成 `@harnix/testkit`

1. `qa/` 依赖 `"@harnix/testkit": "link:../harnix/ui/packages/testkit"`，**去掉**自己的 `@playwright/test`（现在是
   1.63）：用 Harnix 那一份（1.62.1）跑，`pnpm --dir harnix exec playwright test -c "$PWD/qa/ui/<配置>.ts"`。用例
   只从 `@harnix/testkit` 引 `test`、`expect`，配置文件对 `@playwright/test` 只做 `import type`：这样全程只有一份
   Playwright（两份会报「被加载了两次」）。P5 走了退路的话，写 `workspace:*`、装 `@playwright/test@1.62.1`。
   `qa/kit/nest-server.ts`、`ipdrun-server.ts`、`nest-api.ts` 删掉，换成：

   ```ts
   import { startHarnix, connect, test, expect } from "@harnix/testkit";
   const backend = await startHarnix({
     bin: `${APP}/target/release/ipdrun`,       // 绝对路径；否则 testkit 会去 harnix/ 里 cargo build
     uiDir: `${APP}/target/ui`,
     profile: PROFILE_WITHOUT_TRANSPORT,         // testkit 自己写 [transport]：这里只给 [brand] [shell] [ui] [modules.ipdrun]
     args: ["serve", PROJECT],                   // 要配合 P3 的 split_serve：testkit 在前面加了 --profile 等开关，
                                                 // split_serve 必须能在任意位置认出 serve 与路径
     env: { IPDRUN_PROJECT: PROJECT },           // 或者只用环境变量，最省事
   });
   ```

   testkit 的进程有自己的 HOME（`~/.ipdrun` 在 `<scratch>/home/.ipdrun`）、端口、数据根，`cwd` 是它建的空项目；
   IPDrun 的用例要的是 `qa/fixtures/projects/deployer-mini`，用 `seed` 或在用例里拷进 `backend.scratch`。
2. 用例打标签 `@feature:<功能>`、`@layer:api|ui`。先移：`e2e:stage1..4` 的 API 部分（剧本模型，
   `[modules.ipdrun] playbook`），再移界面部分（组件对象 `qa/ui/components` 照 `data-testid`，只改选择器的
   前缀）。`e2e:nest` 删掉（它测的是 vendor 的 Nest，Harnix 自己的测试已经覆盖）。
3. Rust 的 `crates/ipdrun/tests/api/*`（现在对着 `ipdrun-server` 起服务）改成两类：领域的直接测 `Api::dispatch`；
   要经传输的挪到 testkit 的 API 层。`deepseek_stage*`（真模型）保留为 live 层，凭据用 `env` 选项传（testkit 只读
   Harnix 根的 `.env`，§6 G9）。
4. `scripts/check.sh`：加 `sync-app-deps --check`、`check-pins`、`check-lock`、`check-no-ipc`；e2e 用新的 `qa`。

**验收**：`scripts/check.sh` 与 `pnpm e2e` 全绿；阶段 1–4 的验收用例都在新框架里过；覆盖表能从标签生成。

### P7 桌面壳

1. 删掉 `apps/tauri`，拷一份 `harnix/templates/desktop` 到 `desktop/`（自成 Cargo workspace，已在 `exclude` 里）。
2. 改名：`src-tauri/Cargo.toml` 的 `name = "ipdrun-desktop"`；`tauri.conf.json` 的 `productName = "IPDrun"`、
   `identifier = "dev.openatta.ipdrun"`（沿用 `apps/tauri` 的，用户配置目录不变）、`version`；`src/config.rs` 的
   `ENV_PREFIX = "IPDRUN_DESKTOP"`、`SIDECAR = "ipdrun"`；`dist/` 的标题与文案（`apps/tauri/splash` 的内容可以
   搬过来）。图标：`cargo tauri icon`。
3. 本地模式要打开哪个项目：`desktop.toml` 的 `[local] args = ["serve", "<项目>"]`，或者不给项目、在界面里打开
   （none 模式）。壳已经加了 `--host 127.0.0.1 --port <端口> --launch-link`，`args` 里不要重复；P3 的
   `split_serve` 要能接受这些开关出现在 `serve` 前面。壳从 `~/.ipdrun/token` 读令牌（`[brand] product_id`
   决定），和 IPDrun 的 `owner.token` 无关。
4. sidecar：`target/release/ipdrun` 按三元组拷进 `src-tauri/binaries/`，`bundle.externalBin = ["binaries/ipdrun"]`，
   profile 随包带。`cargo deny check licenses` 对 `desktop/src-tauri` 也跑一遍（Tauri 带进来的 MPL-2.0 crate
   照 IPDrun overview §11 登记）。

**验收**：`node harnix/templates/desktop/check-no-ipc.mjs desktop` 通过；
`IPDRUN_DESKTOP_PROGRAM=$PWD/target/release/ipdrun ./desktop/src-tauri/target/debug/ipdrun-desktop --smoke`
退出码 0；窗口里的 Network 只有回环上的 HTTP 与 WebSocket（MCP 端口是外部 Agent 的，窗口不连）；关窗口、
`kill -9` 壳，`ipdrun` 进程都跟着退出。

### P8 清理

1. 删 `vendor/`、`packages/`、`apps/`（已搬空）、`qa/kit` 里的旧启动器、`crates/ipdrun-server` 的旧装配残留、
   `home.rs` 里 `owner.token` 相关的代码、`.claude/worktrees/Core`。
2. `ipdrun-mcp` 里「将来搬进 Nest」的注释改成「等 Harnix 0.4 之后的 MCP 服务端」。
3. 文档：`docs/nest-sync.md` 记阶段二完成（Harnix 版本、Agix 提交），N-xxx 全部标「Harnix 已实现，删除」，
   R-xx 按 §3 标去处；`docs/architecture/overview`、`design/{agent-host,api,shells,events}` 改成 Harnix 的说法；
   `README.md` 的构建与运行。
4. 发布前：`check-pins --release`（`agix/` 必须和 Harnix 钉的一样）、`check-lock`、`scripts/check.sh`、`pnpm e2e`。
5. 构建缓存：Harnix + Agix 的 release 构建很占盘，收尾时清 `target/`。

**验收**（roadmap M9 对 IPDrun 的要求）：两个子仓指针都指向上游存在的提交，`check-pins --release` 通过，没有补丁，
`vendor/` 不在了；IPDrun 自己的测试通过。

---

## 6. 风险与缺口

### 6.1 Harnix / Agix 现在没有的，IPDrun 先怎么顶

| # | 缺口 | 对 IPDrun 的影响 | 先怎么做 | 什么时候补 |
|---|---|---|---|---|
| G1 | 没有 MCP 服务端、角色令牌 | `/mcp` 与角色令牌留在 IPDrun | P3 第 5 条：单独端口，安全自己负责 | 0.4 之后（D10、D27） |
| G2 | 主题只推整值快照，没有追加模式 | 65 种增量事件进不了主题 | 自有链接 `/apps/ipdrun/ws`，`seq` 与回补自己做 | 0.4 之后，有真实需求再做 |
| G3 | Harnix 的命令行不认应用的子命令 | `init`、`open`、`check`、`mcp-config`、hook 要在交给 Harnix 前自己分派；`serve <路径>` 要自己拆 | P3 第 2 条 | 可回推（让 builder 接受应用的子命令） |
| G4 | 模块拿不到自己监听的地址与端口 | `run.json`、`ipdrun open` 要知道端口 | `split_serve` 从命令行和 profile 里自己解析端口，写进 `run.json` | 可回推（`ModuleContext` 给出监听地址） |
| G5 | 模块订阅不了宿主主题（例如 `harnix.sessions.status`） | `is_running` 只能自己从 tap 记 | P4 第 3 条 | 看需要 |
| G6 | `ctx.sessions().send` 只收文本 | `InputMessage` 若带图片 / 附件，服务端发不出去 | 先只发文本；附件放进项目目录，消息里给路径 | 可回推 |
| G7 | 模块不能声明「这台实例的 Agix 设置」 | 厂商、MCP 要模块自己写进实例数据根的设置文件 | P4 第 2 条；第一个会话之前写 | 可回推（§6.2） |
| G8 | 「只用令牌」「只用 key」两条静态检查只查 Harnix 自己的包 | IPDrun 的界面包没人拦 | IPDrun 的 `check` 里自己加（照 `harnix/tests/style-lint.mjs`） | 可回推 |
| G9 | testkit 的 live 模式只读 Harnix 根的 `.env`，`hasModel` 只认 Anthropic 的变量 | DeepSeek 的 live 用例判不出有没有模型 | `startHarnix({ env })` 自己传；自己判有没有凭据，没有就 `inconclusive()` | 可回推 |
| G10 | testkit 自己写 `[transport]`、自己定 `cwd` | IPDrun 的 profile 不能整份传；项目要自己放进 scratch | P6 第 1 条 | 看需要 |
| G11 | 应用自己的 pnpm workspace 摆法没在外部应用上验证过 | P5 可能卡在依赖解析 | P5 用 Harnix 模板的两个 workspace 的做法，并写了退路（根 workspace 收 `harnix/ui/packages/*`） | M9 由两个消费者定下来 |
| G12 | `6fe3687` 的 `templates/app/scripts/build.sh` 没接 `HARNIX_UI_MODULES` | 照那一版的模板抄会编不进界面 | 用 P5 的 `build.sh` | Harnix 已在改（按 `spaces/*/ui/src/index.tsx` 自动接），随下一次发布 |
| G13 | M8、M9-1 还没进发布标签 | 钉提交而不是标签 | P0 第 3 条 | 下一次 Harnix 发布 |
| G15 | 二进制里有两份 rmcp（Agix 的 1.8、`ipdrun-mcp` 的 3.5） | 体积 | 接受；Harnix 的体积预算不管应用的二进制 | 等 Agix 升 rmcp |
| G16 | Agix 侧：`thinking` 无签名不回传（A14 的尾巴）、`subprocess_env` 缺省继承（A13） | 和现在一样 | 照旧 `thinking_mode: off`、写白名单 | 在 Agix 提 issue（不绕过） |

### 6.2 回推 Harnix 的候选

两个应用都在用同一种形状自己做的东西。先在 IPDrun 里做，等 Helm 也做完、形状一致，再提给 Harnix（SDK §2）。

| 候选 | IPDrun 怎么做 | Helm 怎么做 | 进 Harnix 的样子 |
|---|---|---|---|
| 带 `seq` 的领域事件流 | 65 种事件经 `/apps/ipdrun/ws` | `event_stream` + 回补 + WS | 主题的追加模式，复用中枢的 `seq` 追赶 |
| 操作注册表 → 方法 + MCP + 角色 | `OpTable` 自动注册成 `app.ipdrun.*`，同一张表出 MCP 工具，角色矩阵过滤 | `operation-registry.yaml` 出方法与原生工具 | 方法注册表投影成 MCP 工具与角色令牌（已在「0.4 之后」）；`ipdrun-mcp` 不依赖任何 `ipdrun-*`，是现成的起点 |
| 方法的幂等键 | `OpSpec.idempotency` | `intent_id` | 方法声明 `.idempotent()`，中枢按键去重 |
| 模块给实例写 Agix 设置 | 厂商、密钥（0600）、`mcp_servers` | 同样 | 模板 / `EngineContrib` 上声明设置，建实例时由 Harnix 写 |
| 人在对话里点头 | 提议卡、决策卡 | 批准卡 | 批准原语（D26） |
| 应用的界面静态检查 | 自己写 | 自己写 | `style-lint`、i18n 检查接受应用的包目录 |
| testkit 对外部应用更友好 | 传 profile、项目、非 Anthropic 凭据 | 同样 | `startHarnix` 接受完整 profile、`project` 目录、应用自己的 `.env` |
