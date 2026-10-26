# Harnix 开发状态

> 这是接手开发的入口：上下文清掉、换了机器、隔了几周，都从这里接上。
> 计划见 [roadmap](roadmap.md)，架构见 [0.4 设计](harnix-0.4-design-and-plan.md)，用词见 [术语](glossary.md)。

---

## 现在

| 项 | 值 |
|---|---|
| 阶段 | **M7 模块的对话与链接能力**（M0–M6 已发布为 `Harnix-v0.4.0-2026102601`） |
| 当前里程碑 | M7（维护者 2026-10-26 说开始） |
| 当前任务 | 见「进行中」 |
| 下一步 | 见「进行中」。Agix 缺口卡住的先在 Agix 里解决，不卡的先做 Harnix 再回头做 Agix（维护者 2026-10-26） |
| 最后更新 | 2026-10-26 |

## 怎么接上

1. 读本文的「现在」「进行中」「待决」三节。
2. 读 roadmap 里当前里程碑那一节，再按需读设计文档里它引用的章节。
3. `git status`、`git log --oneline -5`、`git -C agix log --oneline -1`，对照下面的「代码基线」。
   对不上就先查清楚，不要在不知道的状态上接着做。
4. `scripts/check.sh --fast` 看一眼当前是不是绿的。
5. 从「进行中」里那一项的「下一步」接着做。

## 怎么更新

- 开始一个任务：在「进行中」里加一行，写清要改哪些地方。
- 做完一个任务：从「进行中」移到里程碑表里标 ✅，写上提交号。
- 每次停下之前（哪怕只做了一半）：更新「进行中」的「下一步」，在「日志」里加一行。
- 发现设计问题：写进「待决」，停下等维护者裁定。
- 改的是**状态**；计划变了才改 roadmap，架构变了才改设计文档。

---

## 代码基线

记于 2026-10-25，A0 完成时：

| 项 | 值 |
|---|---|
| 本仓库分支 | `rename-agix-harnix`，从 `upgrade-attacore-0.2.5`（HEAD `b1cfd7e`，0.3.1-2026102501）分出；两条都**没有推送** |
| `agix/` 钉的 | `802c9f6`（Agix 0.2.7，已推送，标签 `Agix-v0.2.7-2026102601`） |
| 远程地址 | GitHub 已改名（2026-10-25）：本仓库 origin 是 `openatta/harnix`，`.gitmodules` 与 `agix/` 的 remote 是 `openatta/agix`，`git ls-remote` 都通 |
| 不进 git 的输入文档 | `docs/helm-issue-01-nest-0.4-architecture.md`、`iprun-issue-01-sync.md`（Helm、IPDrun 的原文，不改名不改内容） |
| Agix issue | `.local/agix-issue-01…04-*.md`（0.4 用到的），都还没在 Agix 落地 |
| 本机目录 | 仍是 `Workspace/Nest`。改名的话，Claude 的 memory 目录要跟着挪 |



---|---|
| 分支 | `upgrade-agix-0.2.5`，比 `main`（`079086c`）多 6 个提交，**没有设上游，还没推** |
| HEAD | `b1cfd7e` Version 0.3.1-2026102501（标签 `nest-v0.3.1-2026102501`） |
| `agix/` 钉的 | `ee2d5d9`（Agix 0.2.5-2） |
| `agix/` 检出的 | `813dded`（Agix 0.2.6-2026102001）。工作区里是 `M core`，**和钉的不一致**，M0-1 会把钉改过来 |
| 未跟踪的文件 | `docs/harnix-0.4-design-and-plan.md`、`docs/helm-issue-01-nest-0.4-architecture.md`、`docs/glossary.md`、`docs/roadmap.md`、`docs/status.md`、`docs/harnix-sdk.md`、`iprun-issue-01-sync.md` |
| Agix issue | `.local/agix-issue-01…04-*.md`（0.4 用到的），都还没在 Agix 落地 |

注意：CLAUDE.md 的发布步骤要求 `agix/` 检出到钉的那个提交。在 M0-1 提交新钉之前，不要按现在的工作区发布。

---

## 里程碑

| 里程碑 | 版本 | 状态 | 说明 |
|---|---|---|---|
| 前置 A：Agix 改名与 0.2.7 | Agix 0.2.7 | ✅ | Agix 仓库里本地分包提交，A6 一次推送；推送前维护者要先在 GitHub 上把仓库改名为 `openatta/agix` |
| M0 引擎基线与构建方式 | 0.3.z | ✅（0.4.0 发布） | `923f8cd`、`d407316`、`156100e`；`check.sh` 全绿 |
| M1 LIVE 测试框架 | 0.3.z | ✅（0.4.0 发布） | 「Run the suites under Playwright Test…」那个提交；`check.sh` 全绿（remote 未跑） |
| M2 模块契约与应用装配 | **0.4.0** | ✅（0.4.0 发布） | 「Assemble the host from modules…」那个提交；`check.sh` 全绿（remote 未跑） |
| M3 实例层与服务端会话 | 0.4.z | ✅（0.4.0 发布） | 「Stand instances behind one link…」那个提交；`check.sh` 全绿（remote 未跑） |
| M4 前端底座 | — | ✅（0.4.0 发布） | 「Replace the interface with a React shell…」那个提交（带着 M5 的一部分）；M4 自己的各层 `check.sh` 全绿 |
| M5 Space 模型 | 0.4.z | ✅（0.4.0 发布） | 「Give sessions a space…」那个提交；`check.sh` 全绿（remote 未跑） |
| M6 系统 space | 0.4.z | ✅（0.4.0 发布） | 「Give the system space its tree and inspector」那个提交；`check.sh` 全绿（remote 未跑） |
| M7 模块的对话与链接能力 | 0.4.z | **进行中** | 分支 `m7` |
| M8 运行期界面模块 v2、品牌 | 0.4.z | 未开始 | |
| M9 桌面壳、文档、消费者落地 | 0.4.z | 未开始 | |

### 前置 A 任务

| 任务 | 状态 | 提交 | 备注 |
|---|---|---|---|
| A0-1 Agix 仓库改名 | ✅ | Agix `8468450`（本地，重放在上游 `beb69db` 之上） | 全量测试 2881 过，与改名前一致 |
| A0-2 本仓库改名 Harnix、跟上 Agix（`core/` → `agix/`） | ✅ | （本次提交，分支 `rename-agix-harnix`） | 连带提前做了 M0-4（维护者同意）；`check.sh` 结果见日志 |
| A1 实例隔离 | ✅ | Agix 分支 `agix-0.2.7`：`0be92d1`…`e6752bb`（6 个，本地） | 全量测试 2873 过；「录制 / 回放按池子配置」核实后不需要（Harnix 按会话传 `options.recorder`）；Assembly 多了 `mcp_oauth_resolver`，A6 时 Harnix 的 engine.rs 要跟着补 |
| A2 daemon 客户端的正确性 | ✅ | `5cb82a9` | 协议版本 3；`subscription_dropped`；监听器 bind 后才写 instances.d |
| A3 轮次收口与常驻调度器 | ✅ | `b0e3eb7` | 改成池子级调度器走 `run_turn`，串轮从源头消失，不必重构核心循环；`schedule.*`；生命周期进 `daemon.event`；顺带修整点任务只触发一次、非持久任务被写盘 |
| A4 会话任务 | ✅ | `78f3b2e` | `session.tasks` / `stopTask`；修好模型的 `TaskStop`；任务事件进 `session.event` |
| A5 上下文查看与修改 | ✅ | `38876ee` | `inspectContext`、`dropMessages`、`editMessage`；钉住与录像 RPC 推后 |
| A6 发布 Agix 0.2.7 | ✅ | Agix `802c9f6`，标签 `Agix-v0.2.7-2026102601`（已推送） | 全量测试 2874 过；Harnix 钉上后 `check.sh` 与升级前一致 |

### M0 任务

| 任务 | 状态 | 提交 | 备注 |
|---|---|---|---|
| M0-1 提交 `agix/` 指针 | 未开始 | | |
| M0-2 workspace 排除 agix，删依赖表 | ✅ | `923f8cd` | |
| M0-3 crate 显式元数据、`set-version`、改 CLAUDE.md | ✅ | `923f8cd` | |
| M0-4 `engine.rs` 改调 `assemble` | 随 A0-2 提前做了 | （A0-2 的提交） | 只做了能编译、能过测试的最小部分；M0-5 的 cwd 仍未做 |
| M0-5 `EngineConfig.cwd` | ✅ | `d407316` | |
| M0-6 中枢修正四项 | ✅ | `d407316` | N-106 对现在的 Harnix 不适用，M3 的无头会话再看 |
| M0-7 删中枢自记的 `project_root` | ✅ | `d407316` | |
| M0-8 测试隔离，`13-agent` 转绿 | ✅ | `156100e` | 根因：测试拿仓库当项目，旧 `.atta/memory` 让录像多了一次记忆召回 |
| M0-9 复核概念 §9 六条 | ✅ | `156100e` | 六条都已落地 |

### M1 任务

| 任务 | 状态 | 说明 |
|---|---|---|
| M1-1 pnpm workspace | ✅ | 根目录的 `pnpm-workspace.yaml`（不在 `ui/`：`tests/live` 也是成员；设计 §4.2 已改） |
| M1-2 ts-rs 生成协议类型 | ✅ | `crates/contract/tests/ts_export.rs` → `ui/packages/core/src/protocol/generated/contract.ts`；`check.sh` 的 frontend 层比对 |
| M1-3 `@harnix/core/client` | ✅ | 回流 IPDrun `nest-client`，Vitest 14 个 |
| M1-4 `@harnix/testkit` | ✅ | launcher、api、fixtures、`defineSuite` |
| M1-5 Playwright 两个 project | ✅ | `tests/live/playwright.config.ts`；`pnpm live --live` |
| M1-6 迁移 17 个套件与两份 spec | ✅ | 131 + 10 个用例，与迁移前逐条相同（套件体原样） |
| M1-7 标签与覆盖表 | ✅ | `tests/live/features.mjs`、`coverage.mjs`、`coverage.md` |
| M1-8 `docs/testing.md` | ✅ | |

M1 顺带修了一处：`include_dir!("ui")` 把 `ui/packages/**/node_modules` 也嵌进了二进制（63.8 MB），改成只嵌界面的几个目录。

### M2 任务

| 任务 | 状态 | 说明 |
|---|---|---|
| M2-1 `harnix-module` | ✅ | `crates/module`：`Module`、`Manifest`、方法 / 事件 / 路由注册表、`ModuleContext`、`assert_api`（版本不符编译不过，有 compile_fail doctest） |
| M2-2 `harnix-app` | ✅ | `crates/app`：`App::builder()`、启动顺序、`harnix.diagnostics`、`--version` 打全版本 |
| M2-3 二进制走 builder，内建改模块 | ✅ | `crates/harnix`（门面 + 二进制）；`BuiltinModule`；`tests/layering.rs` 守住内建只用公开 API |
| M2-4 三种主体、`app.<id>.*` | ✅ | `Subject::Module`；授权表按主体，中枢自有的方法不可授予 |
| M2-5 `/apps/<id>/` | ✅ | `crates/transport/src/apps.rs`；停用模块的路由 404，不落到 SPA |
| M2-6 失败隔离 | ✅ | 注册冲突、`migrate`、`start` 失败只停该模块；`crates/testapp` 四种故障模块，`modules`、`migrations` 套件 |
| M2-7 配置段与凭据 | ✅ | `[modules.<id>]`；`EnvCredentials` 读 `HARNIX_CREDENTIAL_*` |
| M2-8 协议形状带 `instance` | ✅ | 键 `(instance, session)`，缺省 `system`；新错误码 `INSTANCE_NOT_FOUND`、`SPACE_MISMATCH`；`instance-shape` 套件 |
| M2-9 传输小项 | ✅ | 从 IPDrun 的 vendor 回流 |
| M2-10 `templates/app` 与脚本 | ✅ | `tests/app-template.mjs` 离线构建过；`check.sh` 新增 template 层 |

### M3 任务

| 任务 | 状态 | 说明 |
|---|---|---|
| M3-1 `harnix-instance`、`EngineLink`、进程内载体 | ✅ | `crates/instance`（由 assembly 改名）：`link.rs`（trait、`LinkStatus`、`DownLink`）、`inproc.rs` |
| M3-2 登记表、账本、模板、`ctx.instances` | ✅ | 中枢的 `Slot` / `Registry`，`.harnix/instances.json`；模板来自 profile 与模块的 `templates()`；`harnix.instances.create / ensure / stop / templates` |
| M3-3 第二台进程内实例配置不同也隔离 | ✅ | `instances` 套件：场景不同、会话互不可见、数据根各自一份。MCP 与原生工具的不同靠 Agix 的 `two_pools_one_process` 守着，Harnix 侧没有单测 |
| M3-4 daemon 载体 | ✅ | `daemon_link.rs`：unix socket / TCP+token、协议版本比对、spawn 与看护（stdin 断了子进程退出）、`harnix daemon` 子命令（隐藏） |
| M3-5 resync、`subscription_dropped`、`daemon.event` | ✅ | `crates/hub/src/lifecycle.rs`；不是中枢发起的轮记为 external，`turn_completed` 或 resync 结算 |
| M3-6 `ctx.sessions` | ✅ | `crates/module/src/sessions.rs`：`SessionSpec`（provider、model、`output_schema`……）、`OnBusy` 四种 |
| M3-7 tap | ✅ | 有界（默认 512），满了丢帧记落后，`Lagged` 先于后来的帧报出，`catch_up` 按 seq 补 |
| M3-8 实例树主题与方法 | ✅ | `harnix.instances`、`.instances.sessions`、`.sessions.tasks`、`.sessions.status`；订阅时现算，变化时合并重算 |
| M3-9 `hello` 带提问超时与各实例协议版本 | ✅ | |
| M3-10 实测每台进程内实例的内存 | ✅ | 空载约 0.1–0.2 MB/台，写进设计 §10.6；`tests/budget.mjs` 加了一项（≤ 4 MB） |

M3 发现的 Agix 缺口：`.local/agix-issue-05-resume-recorder.md`（恢复的会话拿不回录制器），在 Agix 0.2.8 落地。

### M4 任务

| 任务 | 状态 | 说明 |
|---|---|---|
| M4-1 `@harnix/core/state` | ✅ | IPDrun nest-ui 的会话模型与控制器，键 `(instance, session)`，`harnix.session_resync` 时重新 attach；`custom` 块；Vitest 36 个 |
| M4-2 `@harnix/react` | ✅ | 贡献注册表（v1 的 6 个点）、`useNest()`（`NestProvider`）、i18n（两本词典由旧界面搬来）、令牌（`tokens.css`，组件变量也在这里）、图标 |
| M4-3 对话组件 | ✅ | nest-ui 的 `ChatPanel` 等，加了真的附件上传、`flow.block` 渲染钩子、输入框句柄；Vitest |
| M4-4 `@harnix/shell` | ✅ | 三栏、侧栏（分组、搜索、拖动、归档）、详情、设置八页、各对话框、横幅、快捷键、主题 |
| M4-5 静态检查、CSP、嵌入 | ✅ | `style-lint` 改成查新树；词典检查进 Vitest；CSP 不变；`ui/dist` 嵌进二进制（`build.rs` 没产物就停下并说怎么办）；`--ui-dir`、`ui export` 照旧 |
| M4-6 旧冒烟换掉 | ✅ | `reducer/contrib/i18n/ui/tool-smoke`、`dom.mjs` 删除，换成各包 Vitest 与 `tests/live/ui/` |
| M4-7 对等用例、删旧 `ui/` | ✅ | `tests/live/ui/` 23 个，两种拓扑都过；旧 `ui/` 原生模块删除 |

### M5 任务

| 任务 | 状态 | 说明 |
|---|---|---|
| M5-1 贡献点 v2 | ✅ | 8 个点（`space`、`home.card`），目录由 `harnix_module::catalog` 生成 |
| M5-2 宿主 API v2 | ✅ | `useNest()`：`call`（按 reachable 过滤）、`subscribe`、`session.*`、`instances.*`、`composer.insert`、`reveal` |
| M5-3 space 账本、`SPACE_MISMATCH`、系统 space 审计 | ✅ | `crates/hub/src/spaces.rs`；`harnix.audit` |
| M5-4 默认实例、切换时恢复 | ✅ | `template:<id>` 第一次用时建、归 space；每个 space 自己的当前会话，切回恢复，删了退到本 space 的列表 |
| M5-5 会话 space 的条目是工作区 | ✅ | 沿用工作区分组 |
| M5-6 HOME 与 `home.card` | ✅ | 仪表盘：问候、问一句（开 HOME 自己的会话）、卡片网格；内建「最近的会话」卡 |
| M5-7 `examples/hello-app` | ✅ | greeter（三部分齐全、自己的实例）、notes（界面 + Agent）；`check.sh` 与 CI 都构建它 |

界面风格按维护者 2026-10-26 的要求改成 Helm GUI demo 的样子（设计 §8.7）。

### M6 任务

| 任务 | 状态 | 说明 |
|---|---|---|
| M6-1 系统模块（后端） | ✅ | `crates/builtin/src/system.rs`：`harnix.system.*` 九个方法，以 `system` 模块的身份调引擎；profile 关掉时整个部分不注册、`system` space 不存在，借它的名义调实例方法回 `SPACE_MISMATCH` |
| M6-2 实例 → 会话 → 任务的树 | ✅ | 左栏是实例（状态点、载体），展开是它的会话（标出所属 space）；`harnix.instances`、`harnix.instances.sessions` 主题推送 |
| M6-3 会话检查器、调用页签 | ✅ | 每次信封变化的调用一行，展开看与前一次的差异（系统块、工具、模型、上限），可在详情里看全文 |
| M6-4 操作与审计 | ✅ | 压缩、清空、分叉、换模型、中断、改名、关闭、删除；实例按模板新建、停止、重启（新的 `harnix.instances.restart`）；全部写审计。「重新 attach」就是重启一台 attach 的实例 |
| M6-5 任务与上下文页签 | ✅ | `harnix.sessions.tasks` 主题与停止；`inspectContext` 的系统块、工具、合计，最近 50 条消息可改写、可去掉 |

后面里程碑的任务表在开始那个里程碑时从 roadmap 抄过来。

---

## 进行中

| 任务 | 开始 | 改到哪了 | 下一步 |
|---|---|---|---|
| M7-4 批准卡 | 2026-10-26 | M7-1–3 已提交 | hello-app 里只用公开契约做：限 `Device` 的回答、待批主题、扩展条目记决定、超时按拒绝；改二开指南 §10.1 |

格式：

| 任务 | 开始 | 改到哪了 | 下一步 |
|---|---|---|---|

---

## 待决

需要维护者裁定的问题。裁定后把结论写进设计文档或 roadmap，再从这里删掉。

（无）

---

## 日志

最新的在上面。一行一件事。

| 日期 | 事 |
|---|---|
| 2026-10-26 | Agix 0.2.8 发布（`Agix-v0.2.8-2026102602`，`2742afc`）：issue 06（`session.appendEntry` / `session.entries` / `extension_entry` 帧、整段分叉带尾部条目）与 issue 05（恢复时重新给 `recorder`）。全量测试 2880 过；中途一次 29 个失败是我把 `CARGO_TARGET_DIR` 指到别处、wasm 夹具找不到产物，改成软链后全过。Harnix 钉上它。M7-1（条目）、M7-2（长任务进度的范例）、M7-3（`space(id).fetch/socket`）完成 |
| 2026-10-26 | 发布 `Harnix-v0.4.0-2026102601`（维护者同意）：推送分支与标签，main 快进到 `ee7d959`。发布前的 `check.sh` 里 browser 层有一条在机器同时编 Agix 时超时，单独重跑这一层通过；remote 未跑。本地的旧标签 `nest-v0.3.1-2026102501` 没推。同一天维护者说开始 M7 |
| 2026-10-26 | 磁盘一度只剩 152 MiB（`target/` 41G、`agix/target/` 31G 是大头），Agix 检查因此中断；删了我自己 scratchpad 里的两个构建目录腾出空间，Agix 工作树改用 `agix/target` 做缓存 |
| 2026-10-26 | M6 完成：系统模块与开关、实例重启、系统 space 的实例树与四个页签（会话、调用、上下文、实例）、无头会话在树里可见。新增 API 用例 7 个（`system`）、界面用例 6 个（`system`、`headless`）；覆盖表里 system 与 headless 的 pending 去掉。M0–M6 这一轮做完，推送与 0.4.0 等维护者定 |
| 2026-10-26 | M5 全量检查：rust 层一个目录测试没放行「每进程」频率、template 层的 build.rs 用了编译期的 `CARGO_MANIFEST_DIR`（共享 target 目录时指向已删的旧目录），两处修好后这两层重跑通过，其余各层一次通过（remote 未跑）。开始 M6 |
| 2026-10-26 | M5 代码完成：space 账本与核对、默认实例、`@session` 引用、`harnix.audit`、8 个贡献点、每个 space 自己的当前会话、HOME 仪表盘、`examples/hello-app`。维护者要求界面参考 Helm 的 GUI demo，令牌、左栏、顶栏、对话、输入框、HOME 都照它改了 |
| 2026-10-26 | M4 的全量检查：M4 各层全绿；跑的过程中我改了 M5 的代码，导致 api/live/template 三层对着旧二进制失败（`owner` 从 `ui` 改成 `space:sessions`、模板编到半截的源码）。代码交错在同一批文件里，所以 M4 与 M5 的前半部分一起提交；当前树的 API 151、界面 23、模板、clippy 都过 |
| 2026-10-26 | M4 代码完成：React 外壳替换旧界面（对话主干来自 IPDrun nest-ui），`ui/dist` 嵌进二进制，旧 `ui/` 与 node 冒烟删除；界面用例 23 个、前端 Vitest 90 个。运行期包的 v1 界面模块在 M8 之前不加载（诊断页写明） |
| 2026-10-26 | M3 完成，`check.sh` 全绿（remote 未跑）。开始 M4 |
| 2026-10-26 | M3 代码完成：实例层（进程内 / spawn / attach 三种载体一条 `EngineLink`）、模板与账本、断线 resync、外部发起的轮、`ctx.sessions` 与 tap、实例树主题。新增 LIVE 套件 4 个（17 个用例）、录像 `spawns-agent`、测试启动器的 `restart` / `halt`。发现 Agix 缺口 05（恢复的会话拿不回录制器）。每台空载进程内实例约 0.1–0.2 MB |
| 2026-10-26 | M2 完成：`harnix-module`、`harnix-app`、内建改模块、三种主体、`/apps/<id>/`、失败隔离、配置段与凭据、协议带 `instance`、传输小项、`templates/app`。`check.sh` 全绿（remote 未跑）。0.4.0 未发布：推送与标签等维护者点头。开始 M3 |
| 2026-10-26 | M1 完成：pnpm workspace、`@harnix/core`（生成的协议类型 + 客户端）、`@harnix/testkit`、Playwright Test 两个 project、17 个 API 套件和两份浏览器 spec 迁进 `tests/live/`、覆盖表、`docs/testing.md`。`check.sh` 全绿（remote 未跑；browser 层的 package-e2e 与预算层在修完录像路径、内嵌目录后单独复验）。另：改名时把文档里的 `@harnix/core` 误改成了 `@harnix/agix`，改回 |
| 2026-10-26 | M0 完成：Agix 在自己的 workspace 里构建、各 crate 写明元数据、中枢四项修正、删中枢自记的项目根、`EngineConfig.cwd`、测试在独立项目里跑、三个录像去掉多余的记忆召回、概念 §9 复核。`check.sh` 第一次全绿（remote 未跑）。**Harnix 还没有发布**：0.3.1 及之后的提交都只在本地分支 `rename-agix-harnix` 上，发布要推送并合到 main，等维护者定 |
| 2026-10-26 | Agix 0.2.7 发布（A2–A6 完成，推送 main 与标签 `Agix-v0.2.7-2026102601`）。Harnix 钉上它、engine.rs 补 `mcp_oauth_resolver`；`check.sh` 与升级前完全一致（只剩 N-003 与 M0-7 绊线）。设计 §2、§5.7、§9.3、§17 与 roadmap M3、M6 改成 0.2.7 的实际情况；`.local` 的四份 issue 标为已落地。**构建写入失败越来越频繁**（A6 的全量测试重试了 19 次）。开始 M0 |
| 2026-10-26 | 维护者下令：Agix 0.2.7（A2–A6）之后，接着做 Harnix M0–M6——系统 space（看实例、会话、任务，改会话上下文）与界面迁到 React 外壳和 space 模型。M7–M9 不在这一轮 |
| 2026-10-26 | A1 完成，提交在 Agix 分支 `agix-0.2.7`（还没合进 main，A6 一起合）。A2–A5 继续在这个分支上做；工作树放在 scratchpad 的 `agix-a1`，丢了就 `git -C agix worktree add <路径> agix-0.2.7` 重建。A0 的 check 补跑完：rust、clippy 用重试脚本跑过；除 N-003（api/live 6 个、browser 5 个）与 M0-7 绊线外全过。**本机构建会间歇性报 `Illegal byte sequence (os error 92)` 写入失败**（不是沙箱、没有第三方系统扩展、日志无记录），cargo 用重试绕过 |
| 2026-10-25 | 维护者定：Harnix 的标签前缀统一成 `Harnix-v`（与 `Agix-v` 一致）；CLAUDE.md、术语、roadmap、二开指南、memory 已改 |
| 2026-10-25 | Agix 上游多了两个维护者的提交：`dbbd814` 仓库地址、`beb69db` 标签前缀改为 `Agix-v`（大写）。本地改名提交重放到其上，成为 `8468450`（与原 `6a23e31` 只差 CLAUDE.md 的标签前缀，测试结论不变）；A1 工作树跟着挪过去；文档与 memory 的前缀改成 `Agix-v` |
| 2026-10-25 | 换上有效 key 后重跑 `check.sh`：live 122 过、7 败（与 api 相同的 7 个）；interface 过；browser 5 败，页面收到 `{"selected_memories": []}`，是 N-003 的回放错位（M0-8）；rust 层报 doctest 找不到 crate，查实是 `target/debug` 里的构建缓存内容损坏（文件在、读不出），`cargo clean` 后重跑 |
| 2026-10-25 | 维护者在 GitHub 上改好仓库名（`openatta/agix`、`openatta/harnix`），本仓库 origin 已指过去。`.env` 换成 `../IPDrun/.env`（DeepSeek，curl 验证可用；旧的那份 key 已失效）。之前 `target/` 的文件损坏是维护者清理 `target/` 时撞上了正在跑的构建，不是磁盘问题 |
| 2026-10-25 | A0 完成。Agix `6a23e31`：全量测试 2881 过（与改名前一致）。Harnix：`check.sh` 里 build、rust、clippy、offline、budget 过；api 109 过 7 败（13-agent 6 个是 N-003 老问题，03-sessions 1 个是 0.2.6 引擎带上 `project_root` 触发的绊线，属于 M0-7）；live、browser、interface 验证不了：`.env` 里的 DeepSeek key 已失效（curl 直连同样报 invalid）；remote 没跑。另：`target/` 里两次出现单个文件损坏（写入报 EILSEQ、build script 变成非可执行文件），删掉重建后正常，磁盘值得查一下 |
| 2026-10-25 | 维护者决定：本仓库（Nest / AttaNest）改名 Harnix，与 Agix 改名同步做 |
| 2026-10-25 | 维护者决定：AttaCore 改名 Agix（atta 系列全改，旧数据不管，远程 `openatta/agix`）；MCP OAuth 保留；`ScheduleWakeup` 下架，做常驻调度器并开 `schedule.*` RPC；Agix 0.2.7 一次推送。排进 roadmap「前置 A」，开始 A0-1 |
| 2026-10-25 | 0.4 设计第二轮修订定稿（判据 §0.2、D18–D32）；新增术语表、roadmap、status、二开指南。开发未开始，等维护者下令 |
