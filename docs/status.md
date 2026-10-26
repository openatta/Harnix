# Harnix 开发状态

> 这是接手开发的入口：上下文清掉、换了机器、隔了几周，都从这里接上。
> 计划见 [roadmap](roadmap.md)，架构见 [0.4 设计](harnix-0.4-design-and-plan.md)，用词见 [术语](glossary.md)。

---

## 现在

| 项 | 值 |
|---|---|
| 阶段 | **M2 模块契约与应用装配**（M0、M1 已完成，本地提交，未发布） |
| 当前里程碑 | M2 |
| 当前任务 | M2-1…M2-9 一起做（见「进行中」） |
| 下一步 | 见「进行中」。维护者 2026-10-26 下令：A 做完后接着做 M0–M6（系统 space 与界面迁移），M7–M9 不在这一轮 |
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
| M0 引擎基线与构建方式 | 0.3.z | ✅（未发布） | `923f8cd`、`d407316`、`156100e`；`check.sh` 全绿 |
| M1 LIVE 测试框架 | 0.3.z | ✅（未发布） | 「Run the suites under Playwright Test…」那个提交；`check.sh` 全绿（remote 未跑） |
| M2 模块契约与应用装配 | **0.4.0** | **进行中** | |
| M3 实例层与服务端会话 | 0.4.z | 未开始 | M3-3 的完整版、M3-5 的删除依赖 Agix issue 03、04 |
| M4 前端底座 | — | 未开始 | 依赖 M1，可与 M2、M3 并行 |
| M5 Space 模型 | 0.4.z | 未开始 | 依赖 M3、M4 |
| M6 系统 space | 0.4.z | 未开始 | M6-5 依赖 Agix issue 01、02 |
| M7 模块的对话与链接能力 | 0.4.z | 未开始 | |
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

后面里程碑的任务表在开始那个里程碑时从 roadmap 抄过来。

---

## 进行中

| 任务 | 开始 | 改到哪了 | 下一步 |
|---|---|---|---|

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
