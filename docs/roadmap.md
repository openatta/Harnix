# Harnix 路线

> 架构见 [0.4 设计](harnix-0.4-design-and-plan.md)（下文写「设计 §x」），用词见 [术语](glossary.md)，
> 做到哪了见 [状态](status.md)。
>
> 这份文档只在计划变化时改；日常进度只改 status.md。

---

## 规矩

- 每个里程碑交付**一个或几个完整的功能模块**（设计 §4.3 的行），带 API 与 Playwright 两层 LIVE 测试，
  可以单独验收。M1 之前没有 LIVE 框架，M0 用现有测试验收。
- **前一个里程碑的验收全部通过，才进下一个。** 例外：M4 前端底座与 M2、M3 并行，在 M5 汇合。
- 任务编号写成 `M<n>-<k>`，status.md 按编号记进度。任务做完、验收项通过，才算完成。
- 一个任务做到一半发现设计不对：停下，把问题写进 status.md 的「待决」，不要边做边改设计。
- Agix 缺的能力写进 `.local/` 的 issue，Harnix 侧不绕过（D14）。
- 版本（D19）：M0、M1 按 0.3.z 发；M2 合进 main 时发 `0.4.0`；之后每个里程碑合进 main 时升 z。
  0.4.z 里协议与模块 API **只做加法**。发布步骤照 CLAUDE.md。

```text
A ─→ M0 ─→ M1 ─→ M2 ─→ M3 ─┬─→ M5 ─→ M6 ─→ M7 ─→ M8 ─→ M9
           └───── M4 ──────┘
```

M4 依赖 M1（`@harnix/agix/client` 与 testkit），不依赖 M2、M3。

---

## 前置 A：Agix（原 Agix）改名与 0.2.7

**目标**：引擎改名为 Agix、本仓库改名为 Harnix；按 `.local/agix-issue-01…04` 补上 Harnix 0.4 要的能力，修掉核对时发现的现有缺陷；
发 Agix `0.2.7`，**一次推送**。A0 先做；A1–A5 在 Agix 仓库里本地分包提交，A6 统一发布。
M0 不等 A 全部完成；M3 开始前必须完成 A6。

已定（2026-10-25）：atta 系列标识全部改成 agix，旧数据目录不兼容也不迁移；远程仓库改成
`openatta/agix`（维护者在 GitHub 上改名）；MCP OAuth 保留，改成每台池子一份；`ScheduleWakeup`
下架，定时任务由 daemon 内常驻的调度器负责，并开 `schedule.*` RPC 给宿主；标签前缀 `Agix-v`（维护者在上游 `beb69db` 定的）。

| 任务 | 内容 |
|---|---|
| A0-1 | Agix 仓库（原 AttaCore）：`AttaCore`/`attacore`/`attacored`/`ATTACORE_*`/`AttaCode`/`ATTA_*`/`ATTAGO_*`/`.atta`/`atta-*`/`atta_*`、WIT 包 `atta:plugin`、OTel 指标 `atta.*` 全部改成 agix（`attacored` → `agixd`）；`bridges/atta-dsh-bridge`、`.claude/skills/atta-*`、测试夹具里的 `.atta/` 改名；仓库地址改成 `openatta/agix`；`openatta`、单独的「Atta」（指 Atta 这个更大的项目）、`AttaMeta`、`ClawPod` 不动；fmt、clippy、test 通过；本地提交 |
| A0-2 | 本仓库（原 Nest / AttaNest）改名 Harnix，并跟上 Agix：子仓 `core/` → `agix/`（含 `.gitmodules` 的名字与 URL）；`nest`/`Nest`/`NEST_`/`AttaNest` → `harnix`/`Harnix`/`HARNIX_`（crate、二进制、`@harnix/*` 包、`harnix.*` 协议方法、`.harnix/` 账本、标签前缀 `Harnix-v`），`.atta` → `.agix`、`ATTA_*` → `AGIX_*`；改名之前的标签与本机目录名不动；`.local` 的 issue 文件改名；**连带提前做 M0-4**（引擎改调 `daemon::assemble::pool`，0.2.6 起不这样就编译不过）和依赖表补一行 `process-wrap`；`scripts/check.sh` 通过；本地提交 |
| A1 | 实例隔离（issue 03）：`TODO_STORE`、`TASK_STORE` 收进会话；删掉死的全局状态；MCP OAuth 改成每台池子一份 `McpOAuth`；skill `BODIES` 按需现算；OTel 进程级加告警与文档；录制 / 回放改成按池子配置；守护测试 `two_pools_one_process` 与「新增进程级状态要登记」的源码检查 |
| A2 | daemon 客户端的正确性（issue 04 第 2–4 部分）：`daemon.auth` / `daemon.status` 带 `version`、`protocol_version`、能力位，协议版本 2 → 3；订阅被删时补发 `subscription_dropped`，改正协议文档 §5.4；监听器全部 bind 之后才写 `instances.d`；token 推荐用环境变量写进文档 |
| A3 | 轮次收口与常驻调度器（issue 04 第 1 部分 + cron 串轮）：会话事件按 `turn_id` 分发；消息带发起方；池子级的调度器取代会话内的 cron 定时器（会话被回收后照样触发，重启后不丢，经正常的轮次路径执行）；`schedule.create / list / delete`；`daemon.event` 的会话与轮次生命周期事件、`events_lagged`；`ScheduleWakeup` 下架 |
| A4 | 会话任务（issue 01 瘦身）：接上 `ctx.running_tasks`（修好模型的 `TaskStop`）；登记子 Agent、执行中的 Shell / 监视器、团队成员、定时任务；`session.tasks`、`session.stopTask`；轮次之外的任务事件 |
| A5 | 上下文查看（issue 02 瘦身）：`session.inspectContext`（空闲时）、`session.dropMessages`（墓碑）、录像的 `system_meta`。`editMessage`、`pinMessages`、录像 RPC 推后 |
| A6 | 发布 Agix `0.2.7`：协议文档、版本号、全量检查、一次推送并打 `Agix-v0.2.7-<日期><序号>`；Harnix 钉上它 |

**验收**：每个 issue 文件里的验收项（按核实后的前提改过）各有测试覆盖，不需要真模型；Agix 的
fmt、clippy、`cargo test --workspace` 全过；Harnix 在钉上 0.2.7 之后 `scripts/check.sh` 通过。

**A 之后 Harnix 侧要跟着改的**（进对应里程碑）：设计 §5.7 的 history 水位对账不再需要（M3-5 只剩
resync）；系统 space 的任务种类照实改成子 Agent、团队成员、Shell、监视器、定时任务；`[[ui]]` 改成
`x-` 前缀的段（Agix 0.2.6 起宿主段用 `x-` 前缀）。

---

## M0 引擎基线与构建方式（0.3.z）

**目标**：引擎正式钉在 Agix 0.2.7（前置 A 的 A6），Harnix 在它上面完全正确；构建改成设计 §11.2 的方式。
A6 之前可以在本地指针上开发，但不发布：指针必须指向上游存在的提交。

| 任务 | 内容 |
|---|---|
| M0-1 | 提交 `agix/` 指针到 Agix 0.2.7（A6 的发布提交） |
| M0-2 | 根 `Cargo.toml`：workspace `exclude = ["core"]`；删掉抄来的 Agix 依赖表，只留 Agix 入口 crate 的路径与 feature |
| M0-3 | 每个 Harnix crate 显式写 `version`、`license`、`edition`、`rust-version`；`scripts/set-version` 统一改版本号；检查脚本断言所有 crate 版本一致；CLAUDE.md 的发布步骤随之改 |
| M0-4 | `engine.rs` 改调 `daemon::assemble::pool`，补 `scenes`、`credentials`、`customize_session`（N-001、N-007） |
| M0-5 | `EngineConfig.cwd`（N-104） |
| M0-6 | 中枢修正：结算前过屏障（N-105）、`catch_up` 水位（N-120）、`tool_args_delta` 合并（N-121）、options 只在旧会话恢复时带（N-106） |
| M0-7 | 删掉中枢自记的 `project_root`，以引擎为准（N-002） |
| M0-8 | 测试的数据根与 HOME 按套件隔离；`13-agent` 转绿，否则查清并提 Agix（N-003） |
| M0-9 | 复核概念 §9「还需要 Agix 提供的」六条哪些已落地，更新概念文档 |

**验收**：Rust 测试全过；现有 API 套件每种拓扑全过，**包括 `13-agent`**；预算门禁通过；根
`Cargo.toml` 里不再有 Agix 的第三方依赖；`scripts/check.sh` 通过。

---

## M1 LIVE 测试框架（0.3.z）

**目标**：设计 §13 的框架立起来，之后每个里程碑的测试都写在它上面。

| 任务 | 内容 |
|---|---|
| M1-1 | pnpm workspace 起在 `ui/`，新包放 `ui/packages/`；旧的原生 `ui/` 模块原地不动，M4 再删 |
| M1-2 | `harnix-contract` 加 `ts` feature 与 `ts-rs` derive；生成到 `@harnix/agix/protocol`；检查脚本比对生成物 |
| M1-3 | `@harnix/agix/client`：回流 IPDrun 的 `harnix-client`（握手、拓扑与回退、call / notify / answer、seq 追赶），Vitest |
| M1-4 | `@harnix/testkit`：launcher（独立 HOME、数据根、引擎根、端口、拓扑、录像目录）、api、fixtures |
| M1-5 | Playwright Test 两个 project（api、chromium）；`--live` 模式 |
| M1-6 | 把 `tests/api` 的 17 个套件和 `tests/browser` 的两份 spec 迁进 `tests/live/`（行为不变，只换框架） |
| M1-7 | 用例标签 `@feature`、`@layer`；`coverage.md` 生成与检查；接进 `scripts/check.sh` |
| M1-8 | `docs/testing.md` |

**验收**：迁移后的套件全绿，并且与迁移前逐条对得上；覆盖表能列出 0.3 的现状；人为删掉一个用例，
检查会失败；协议类型生成物被人为改动，检查会失败。

---

## M2 模块契约与应用装配（发 0.4.0）

**目标**：Harnix 能当库用，内建功能都是模块；协议形状定下；应用能按设计 §11 的布局构建。

| 任务 | 内容 |
|---|---|
| M2-1 | `harnix-module`（由 `harnix-contrib` 改来）：`Module` trait、`Manifest`、各注册表、`ModuleContext`（配置段、数据目录、凭据、事件发布）、`MODULE_API_VERSION`（设计 §7） |
| M2-2 | `harnix-app`：`App::builder()`、启动顺序（设计 §7.3）、诊断页、`--version` 打全版本 |
| M2-3 | `harnix` 二进制走 builder；内建功能改写成内建模块；测试守住「内建只用公开模块 API」 |
| M2-4 | 主体 `Kernel / Module(id) / Device`；`app.<id>.*` 方法（带 schema 与主体声明，默认拒绝） |
| M2-5 | `/apps/<id>/` 路由与授权、审计 |
| M2-6 | 失败隔离：注册冲突、`migrate`、`start` 失败只停用该模块，进诊断页；迁移按顺序 |
| M2-7 | `[modules.<id>]` 配置段；`CredentialSource` 提供密钥 |
| M2-8 | **协议形状**：会话、任务相关的方法与帧带 `instance`；中枢的键改成 `(instance, session)`；此时只有 `system` |
| M2-9 | 传输小项：R-13 content-type、N-301 启动链接、N-302 令牌文件 0600、N-303 连接日志、N-601 Origin 白名单、N-602 Secure cookie |
| M2-10 | `templates/app`（`agix/`、`harnix/` 平级）；`sync-app-deps`、`check-pins`、`check-lock` |

**验收**：API 的 `modules`、`routes`、`migrations` 套件：测试模块的方法默认拒绝、声明后可达、主体不对
被拒；路由未授权被拒；`start` 失败只停用它自己；迁移失败只停用它自己。测试守住「内建只用公开 API」；
模块 API 版本不匹配**编译失败**；所有会话帧带 `instance`；模板在 CI 里从干净的 checkout **离线**构建
成功；`check-pins`、`check-lock`、`sync-app-deps` 的检查各能抓到一次人为制造的漂移。

---

## M3 实例层与服务端会话

**目标**：设计 §2、§5、§6.1、§6.3 落地，中枢不再只认一个引擎。

| 任务 | 内容 |
|---|---|
| M3-1 | `harnix-instance`（由 `harnix-assembly` 改来）：`EngineLink`；进程内载体 |
| M3-2 | 实例登记表、账本、模板（profile 的与模块的）；`ctx.instances`（`create`、`ensure`、`stop`）；Harnix 按账本统一重建 |
| M3-3 | 第二台进程内实例：MCP 配置与 `system` 不一致时启动失败并说清原因（issue 03 之前） |
| M3-4 | daemon 载体：RPC 客户端、协议版本比对、attach、spawn 与最小看护、`harnix daemon` 子命令；token 经环境变量交给子进程 |
| M3-5 | 断线 resync；结算时的 history 水位对账（临时，issue 04 之后删） |
| M3-6 | 服务端调用方 `ctx.sessions`：`create / open / send / interrupt / close`、`SessionSpec`（含 provider、`output_schema`，N-107）、`OnBusy` 含 steer |
| M3-7 | tap：有界、按 `seq`、落后收 `Lagged` 再追赶，不在锁里等待 |
| M3-8 | 带参数的宿主主题；`harnix.instances`、`harnix.instances.sessions`、`harnix.sessions.tasks`、`harnix.sessions.status` 与对应方法；`harnix.instances.create`（只认 `ui = true` 的模板） |
| M3-9 | `harnix.hello` 带权限提问超时（R-12）与各实例的协议版本 |
| M3-10 | 实测每台进程内实例的内存，写进设计 §10.6 |

**验收**：

- API 的 `instances`：`system` 加一台进程内实例加一台 daemon 实例，各自建会话、跑一轮回放；在一台里
  建的会话、配置、记忆在另外两台里都看不到；界面只能按 `ui = true` 的模板建，给路径或参数被拒；
  重启后按账本重建。
- `instances-daemon`：杀掉 daemon，实例变降级，重启后 resync，客户端的内容完整；协议版本不匹配被拒；
  spawn 的 daemon 在 Harnix 退出后不残留。
- `headless`：测试模块起 3 个无头会话，各自模型，重启后恢复，界面 attach 旁观看得到全过程；一个
  故意不读的 tap 不卡住会话，读起来之后收到 `Lagged` 并补齐。
- 任务：一个派了子 Agent 的会话，`harnix.sessions.tasks` 列得出它，子 Agent 不出现在顶层会话里。

---

## M4 前端底座（与 M2、M3 并行）

**目标**：React 外壳与 0.3 界面功能对等，旧 `ui/` 删除。

| 任务 | 内容 |
|---|---|
| M4-1 | `@harnix/agix/state`：会话事件归约、队列、每个 space 的当前会话（键 `(instance, session)`），Vitest |
| M4-2 | `@harnix/react`：贡献注册表（先是 v1 的 6 个点）、`useNest()`、i18n、令牌、图标 |
| M4-3 | `@harnix/react` 的对话组件：回流 IPDrun 对话主干（D21），按 `data-testid` 约定 |
| M4-4 | `@harnix/shell`：三栏布局、设置页、模态框、断点 |
| M4-5 | 令牌与 i18n 两条静态检查；CSP；产物嵌进二进制；`--ui-dir`、`harnix ui export` 保留 |
| M4-6 | 旧的 node 冒烟（`reducer-smoke`、`contrib-smoke`、`i18n-smoke`）换成 Vitest 或 Playwright 的等价用例 |
| M4-7 | 设计 §10.2 的对等用例逐条通过；通过的那次提交里删掉旧 `ui/` 原生模块 |

**验收**：设计 §10.2 的 Playwright 对等用例全绿，两种拓扑都过；预算门禁；style、i18n 检查；
`readme-pairing` 重录。

---

## M5 Space 模型、会话 space、HOME

**目标**：设计 §8、§10.3、§10.4 落地。

| 任务 | 内容 |
|---|---|
| M5-1 | 贡献点 v2（8 个点，目录由代码生成，`contribution_points.md` 重新生成） |
| M5-2 | 宿主 API v2（设计 §10.4），含 `instances.*` 与 `reveal` |
| M5-3 | space 与会话绑定的账本；`SPACE_MISMATCH` 核对；系统 space 例外与审计 |
| M5-4 | space 默认实例（含 `template:<id>`）；切换时恢复 |
| M5-5 | 会话 space：工作区改成它的条目 |
| M5-6 | HOME 与 `home.card`；HOME 的通用会话 |
| M5-7 | `examples/hello-app`：HOME + 两个 space，一个三个部分齐全并有自己的实例，一个只有前端 + Agent；按设计 §11 的布局在 CI 里构建运行 |

**验收**：

- Playwright 的 `spaces`、`home`：多会话、切走再切回恢复、跨 space 用 `@session` 引用、会话不能挪 space。
- **对应关系（设计 §8.5）**：两个 space 各有自己的实例，故意让两台实例里出现同一个会话 id；在 A 里
  开会话、写一半草稿，切到 B 发送，再切回 A：B 的消息只出现在 B 的会话里，A 的草稿还在，A 恢复的是
  A 自己的会话；在 A 里删掉当前会话后切走再切回，退到 A 的列表而不是 B 的会话；两个浏览器标签页
  各开一个 space 并发操作，互不串。
- API 的 `spaces`：声称的 space 与账本不符的 `harnix.attach` / `harnix.send` 被拒（`SPACE_MISMATCH`），
  声称的实例不对被拒；系统 space 的跨 space 操作被放行并写进审计。
- 外壳真的去问了每个贡献点（`contrib-smoke` 的判据）。

---

## M6 系统 space

**目标**：设计 §9 落地。

| 任务 | 内容 |
|---|---|
| M6-1 | 系统模块（后端）：只用公开 API；profile 开关，关掉时方法撤出授权表 |
| M6-2 | 实例 → 会话 → 任务的树（实例是左栏条目） |
| M6-3 | 会话检查器：概况、调用页签（录像信封、相邻两次的差异） |
| M6-4 | 设计 §9.3 里 Agix 已有的操作：压缩、清空、分叉、换模型、中断、改名、关闭、删除；实例的新建、停止、重启、重新 attach；全部写审计 |
| M6-5 | Agix 补上 issue 01、02 之后：其余任务种类、上下文页签、消息级操作（没补上就留到 0.4 之后，不画假的） |

**验收**：API 的 `system`：关掉开关后方法从授权表撤掉；操作写进审计。Playwright 的 `system-tree`、
`system-calls`、`system-ops`：看得到 M3 的三台实例和各自的会话、任务；选一个会话能看到每次调用的信封
与差异；压缩、清空、分叉、换模型、中断都在界面上生效，并且其他观看者同步看到；在系统 space 里操作
别的 space 的会话，回到所属 space 能看到结果。

---

## M7 模块的对话与链接能力

**目标**：设计 §7.5–§7.8 落地，并证明 §7.8 那些「由应用自己做」的东西在公开契约上真做得出来。

| 任务 | 内容 |
|---|---|
| M7-1 | `ctx.sessions.append_entry`：扩展条目，只给人看；`flow.block` 画出来 |
| M7-2 | 长任务进度：任务 id + 带参数的主题，写成二开指南里的范例 |
| M7-3 | `space.fetch()`、`space.socket()` |
| M7-4 | `hello-app` 里用公开契约做一张批准卡：限 `Device` 的方法、待批主题、扩展条目记决定、超时按拒绝；写进二开指南 |

**验收**：API 的 `entries`、`routes`：扩展条目刷新、重放、分叉后都在，模型看不到；自有链接未授权
被拒。Playwright 的 `entry-card`：卡片画出来。`hello-app` 的批准：模块主体回答被拒，超时就是拒绝，
两台设备先答者生效。**这个范例没有改动 Harnix 的内核 crate**（`git diff` 只在 `examples/`）。

---

## M8 运行期界面模块 v2、品牌

**目标**：设计 §10.5 落地。

| 任务 | 内容 |
|---|---|
| M8-1 | `[[ui]] api_version = 2`；import map 构建时生成，`sha256` 写进 CSP |
| M8-2 | v1 的包报「这一条不生效」，不拒绝整个包 |
| M8-3 | `[ui] runtime_modules = false` 开关 |
| M8-4 | `[brand]`；模块的令牌覆盖与语言包 |

**验收**：装一个带 v2 界面模块的包，它的贡献生效；v1 的包报「这一条不生效」；关掉开关后界面模块
不加载；换品牌之后外观变了，没有 fork `ui/`。

---

## M9 桌面壳、文档、两个消费者落地

**目标**：Helm、IPDrun 都按设计 §11 的布局落上来；文档改成 0.4 的样子。

| 任务 | 内容 |
|---|---|
| M9-1 | `harnix-desktop`（Tauri 2）模板，本地与远程两种模式 |
| M9-2 | 二开指南定稿；概念文档照设计 §18 重写；README 双语；设计文档退役 |
| M9-3 | IPDrun：删掉 `vendor/harnix` 和照抄的装配，改成平级的 `harnix/`、`agix/` 子仓 + builder |
| M9-4 | Helm：Vue 面板迁到 React，插件改成模块，批准按二开指南自己实现 |

**验收**：照二开指南从模板做出一个换了品牌的应用，不问作者；Tauri 抓包确认业务流量全是 HTTP/WS；
IPDrun 的两个子仓指针都指向上游存在的提交，`check-pins` 通过，没有补丁，并且它自己的测试通过。

---

## 0.4 之后

按 §0.2 的判据，有真实需求时再排：

| 方向 | 前提 |
|---|---|
| 角色令牌 + MCP 服务端（方法注册表投影成 MCP 工具） | 有外部 Agent 接入的需求；D10、D27 |
| 跨机器的 daemon 实例 | D15 |
| 运行期包新增 space、`extensible`、`allow_runtime_spaces` | D28 |
| 批准原语进内核 | Helm、IPDrun 各自实现之后形状一致（D26） |
| 运行期后端模块（WASM / QuickJS / 原生） | 编译期模块与 space 模型做实之后 |
| 通道追加模式 | 出现真实的高频增量需求 |
| 迁移的 `--dry-run` 与备份钩子 | D29 |
| 系统 space 的上下文页签与其余任务种类 | Agix issue 01、02（若 0.4 内没落地） |
