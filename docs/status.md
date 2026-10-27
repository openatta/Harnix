# Harnix 开发状态

> 这是接手开发的入口：上下文清掉、换了机器、隔了几周，都从这里接上。
> 架构见 [概念与架构](concept_and_architecture.md)，计划见 [roadmap](roadmap.md)，用词见 [术语](glossary.md)，
> 二开接口见 [SDK 指南](harnix-sdk.md)。

---

## 现在

| 项 | 值 |
|---|---|
| 阶段 | **0.4 的 M0–M9 全部完成并发布**；最新发布 `Harnix-v0.4.0-2026102604`（任务输出，Agix 0.2.9），`main` 与 `m7` 同步 |
| 当前里程碑 | 无。下一个里程碑要维护者说开始 |
| 当前任务 | 无 |
| 下一步 | 无。等维护者说开始下一个里程碑 |
| 最后更新 | 2026-10-26 |

## 怎么接上

1. 读本文的「现在」「进行中」「待决」三节。
2. `git status`、`git log --oneline -5`、`git -C agix log --oneline -1`，对照下面的「代码基线」。
   对不上就先查清楚，不要在不知道的状态上接着做。
3. `scripts/check.sh --fast` 看一眼当前是不是绿的。
4. 从「进行中」里那一项的「下一步」接着做。

## 怎么更新

- 开始一个任务：在「进行中」里加一行，写清要改哪些地方。
- 做完一个任务：从「进行中」移到里程碑表里标 ✅，写上提交号。
- 每次停下之前（哪怕只做了一半）：更新「进行中」的「下一步」，在「日志」里加一行。
- 发现设计问题：写进「待决」，停下等维护者裁定。
- 改的是**状态**；计划变了才改 roadmap，架构变了才改概念文档。
- 构建缓存：里程碑或发布之后跑 `scripts/prune.sh`。

---

## 代码基线

| 项 | 值 |
|---|---|
| 分支 | `m7`，`main` 快进到同一处（已推送到 0.4.0-2026102604） |
| `agix/` 钉的 | `e8a1dd8`（`Agix-v0.2.9-2026102603`） |
| 远程 | `openatta/Harnix`（GitHub 从 `openatta/harnix` 改名，旧地址会转）、`openatta/agix` |
| 不进 git 的输入文档 | `docs/helm-issue-01-nest-0.4-architecture.md`、`iprun-issue-01-sync.md`（Helm、IPDrun 的原文） |
| Agix issue | `.local/agix-issue-*.md`：01–07 都已在 Agix 落地 |

---

## 里程碑

| 里程碑 | 状态 | 说明 |
|---|---|---|
| 前置 A：Agix 改名与 0.2.7 | ✅ | `Agix-v0.2.7-2026102601` |
| M0 引擎基线与构建方式 | ✅（0.4.0） | |
| M1 LIVE 测试框架 | ✅（0.4.0） | |
| M2 模块契约与应用装配 | ✅（0.4.0） | |
| M3 实例层与服务端会话 | ✅（0.4.0） | |
| M4 前端底座 | ✅（0.4.0） | |
| M5 Space 模型 | ✅（0.4.0） | |
| M6 系统 space | ✅（0.4.0） | |
| M7 模块的对话与链接能力 | ✅（0.4.0-2026102602） | `002c407`、`06f28f1`；Agix 0.2.8 |
| M8 外壳框架、运行期界面模块 v2、品牌 | ✅（待发布） | `f960148` |
| M9 桌面壳、文档、迁移说明 | ✅（待发布） | `6fe3687`（桌面壳）与文档提交 |

各里程碑的任务与验收见 [roadmap](roadmap.md)；每一步的细节在提交信息里。

---

## 进行中

| 任务 | 开始 | 改到哪了 | 下一步 |
|---|---|---|---|
（无）

---

## 待决

需要维护者裁定的问题。裁定后把结论写进概念文档或 roadmap，再从这里删掉。

（无）

---

## 日志

最新的在上面。一行一件事。更早的记录在 git 历史里（`git log -p -- docs/status.md`）。

| 日期 | 事 |
|---|---|
| 2026-10-26 | 发布 `Agix-v0.2.9-2026102603` 与 `Harnix-v0.4.0-2026102604`（维护者同意）：Agix `main`、Harnix `m7` / `main` 与两个标签推送。发布前 Harnix `check.sh` 11 层全过（remote 未跑），改版本号后 `--fast` 过；`prune.sh` 后 79G |
| 2026-10-26 | Agix issue 07 落地（Agix 0.2.9，`aabd028`）：`session.taskOutput`、Shell / 监视器的输出边跑边记进任务、`TASK_NOT_FOUND`、能力位 `task_output`；Agix 全量测试过。Harnix 钉上它：系统模块加 `harnix.system.tasks.output`（授权里加 `session.taskOutput`），任务页显示任务的输出，跑着时每秒读一次。`task_progress` 推送帧没做 |
| 2026-10-26 | 发布 `Harnix-v0.4.0-2026102603`（M8、M9，维护者同意）：分支 `m7` 与标签推送，`main` 快进到 `088fd52`；本地 remote 改成 `openatta/Harnix` |
| 2026-10-26 | M9 完成：桌面壳模板 `templates/desktop`（Tauri 2，本机 / 远程，不加 IPC；`--smoke` 过，`check-no-ipc.mjs` 进 offline 层）、主进程 `--exit-with-stdin`；概念文档照设计 §18 重写，设计文档冻结；SDK 指南与参考对齐到 M8/M9；README 双语重写；Helm、IPDrun 迁移说明（维护者：不动那两个仓库）；`templates/app/scripts/build.sh` 接上 space 的界面。Agix 缺口 issue 07（任务输出）记进 `.local`，不卡 Harnix |
| 2026-10-26 | M8 完成（`f960148`）：M8-0 外壳框架（维护者看 demo 后定）、运行期界面模块 v2（import map + CSP hash）、`[ui] runtime_modules`、`[brand]` 与 `~/.<product_id>`、令牌覆盖与语言包、`harnix.process`、`harnix.system.trace`。API 177 个、界面 48 个用例在两种拓扑下都过；`tests/package-e2e.mjs` 由 API 与界面用例取代 |
| 2026-10-26 | 二开 SDK：新增 [SDK 参考](harnix-sdk-reference.md)（照代码列出全部接口、profile 键、目录、方法与主题、贡献点、迁移缺口）；指南对齐到 0.4.0-2026102602，加 §14.0「迁移分析怎么做」，给 Helm、IPDrun 做迁移分析用。查出 `templates/app/scripts/build.sh` 没接 `HARNIX_UI_MODULES`，记在参考 §9 |
| 2026-10-26 | 维护者定 M8 的四个决定（设计 §10.5 末尾）：程序状态放 `~/.<product_id>`、旧的 `~/Documents/.harnix` 不管；import map 只放 React 三件与 `@harnix/react`；`r.tokens` / `r.strings`；CSP hash 由服务端启动时算。M8 还没开始 |
| 2026-10-26 | 发布 `Harnix-v0.4.0-2026102602`（M7）：`check.sh` 跑了的 11 层全过，remote 没跑；分支 `m7` 与标签已推送。GitHub 提示仓库已改名为 `openatta/Harnix`（旧地址会转）。发布后 `prune.sh` |
| 2026-10-26 | M7 完成：全量 `check.sh` 11 层全过（remote 未跑）。跑之前磁盘只剩 1.3G，删了 `agix/target/debug/incremental`（19G，这一轮 Agix 工作产生的增量缓存） |
| 2026-10-26 | Agix 0.2.8 发布（`Agix-v0.2.8-2026102602`，`2742afc`）：issue 06（`session.appendEntry` / `session.entries` / `extension_entry` 帧、整段分叉带尾部条目）与 issue 05（恢复时重新给 `recorder`）。全量测试 2880 过；中途一次 29 个失败是我把 `CARGO_TARGET_DIR` 指到别处、wasm 夹具找不到产物，改成软链后全过。Harnix 钉上它。M7-1（条目）、M7-2（长任务进度的范例）、M7-3（`space(id).fetch/socket`）完成 |
| 2026-10-26 | 发布 `Harnix-v0.4.0-2026102601`（维护者同意）：推送分支与标签，main 快进到 `ee7d959`。发布前的 `check.sh` 里 browser 层有一条在机器同时编 Agix 时超时，单独重跑这一层通过；remote 未跑。本地的旧标签 `nest-v0.3.1-2026102501` 没推。同一天维护者说开始 M7 |
| 2026-10-26 | 磁盘一度只剩 152 MiB（`target/` 41G、`agix/target/` 31G 是大头），Agix 检查因此中断；删了我自己 scratchpad 里的两个构建目录腾出空间，Agix 工作树改用 `agix/target` 做缓存 |
| 2026-10-26 | M6 完成：系统模块与开关、实例重启、系统 space 的实例树与四个页签（会话、调用、上下文、实例）、无头会话在树里可见。新增 API 用例 7 个（`system`）、界面用例 6 个（`system`、`headless`）；覆盖表里 system 与 headless 的 pending 去掉。M0–M6 这一轮做完，推送与 0.4.0 等维护者定 |
