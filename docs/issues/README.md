# 下游项目提的需求

这个目录放在 Harnix 上做应用的项目提过来的需求：缺陷、要加的能力、文档该补的地方。一个需求一个文件，文件名是
`<项目>-NN-<英文短名>.md`，前缀是提出的项目，`NN` 由那个项目在本仓库里自己编号。

每份的结构相同：类型与优先级；现象或需求（带代码位置）；建议（接口草图）；兼容性（对现有应用、缺省行为、协议与贡献点 API 版本的影响）；
验收（本仓库的测试怎么证明）；依赖；来源（提出方在哪里撞到的，一两句）。

规矩：需求、接口、验收只用 Harnix 自己的概念（用词照 [glossary](../glossary.md)），提出方的业务只出现在「来源」里；接口只加不改，
旧调用方不改代码就能继续工作，缺省行为不变（缺陷修复改掉的那一处行为在「兼容性」里写明）。

落地时，提交说明里写上对应的编号（如 `ipdrun-05`）。

## IPDrun

IPDrun 提的需求（ipdrun-01…55，2026-10-27 起）已迁到 GitHub issue，标签 [`ipdrun`](https://github.com/openatta/Harnix/issues?q=label%3Aipdrun)，每条带原文与落地提交；以后 IPDrun 的新需求直接在 GitHub 提（同一标签），不再放这个目录。代码注释与提交说明里的 `ipdrun-NN` 按下表找：

| 编号 | GitHub | 标题 |
|---|---|---|
| ipdrun-01 | [#1](https://github.com/openatta/Harnix/issues/1) | 单双向流上超过在途上限的请求被静默丢掉 |
| ipdrun-02 | [#2](https://github.com/openatta/Harnix/issues/2) | SIGTERM 不走模块的 `stop` |
| ipdrun-03 | [#3](https://github.com/openatta/Harnix/issues/3) | testkit 的 `stop()` 不等进程退出就删目录 |
| ipdrun-04 | [#4](https://github.com/openatta/Harnix/issues/4) | 两个并发的 `instances.ensure` 同名时，第二个失败 |
| ipdrun-05 | [#5](https://github.com/openatta/Harnix/issues/5) | spawn 实例 `ensure` 返回时还在 starting，紧接着的会话调用失败 |
| ipdrun-07 | [#6](https://github.com/openatta/Harnix/issues/6) | spawn 实例停下以后，它的会话起的后台进程还活着 |
| ipdrun-08 | [#7](https://github.com/openatta/Harnix/issues/7) | 新旧两个 daemon 会同时写同一个实例数据根 |
| ipdrun-09 | [#8](https://github.com/openatta/Harnix/issues/8) | 同一个状态目录可以同时起两个进程 |
| ipdrun-10 | [#9](https://github.com/openatta/Harnix/issues/9) | spawn 模板带不了原生贡献 |
| ipdrun-11 | [#10](https://github.com/openatta/Harnix/issues/10) | 模板建的实例读不到「设置 → 模型」里配的 provider |
| ipdrun-12 | [#11](https://github.com/openatta/Harnix/issues/11) | profile 去掉系统 space 后，输入框不能换模型 |
| ipdrun-13 | [#12](https://github.com/openatta/Harnix/issues/12) | 模块拿到 Agix 的 prompt 中间件 |
| ipdrun-14 | [#13](https://github.com/openatta/Harnix/issues/13) | 模块不知道进程监听在哪个地址 |
| ipdrun-15 | [#14](https://github.com/openatta/Harnix/issues/14) | 服务重启后，已经打开的页面永远连不回去 |
| ipdrun-16 | [#15](https://github.com/openatta/Harnix/issues/15) | `useNest().subscribe` 的退订不计数，同一页里另一个订阅者从此收不到推送 |
| ipdrun-17 | [#16](https://github.com/openatta/Harnix/issues/16) | `harnix.turn_settled` 不带这一轮的结果 |
| ipdrun-18 | [#17](https://github.com/openatta/Harnix/issues/17) | space 不能把别的 space 带到前台 |
| ipdrun-19 | [#18](https://github.com/openatta/Harnix/issues/18) | space 管不了右栏：选哪个页签、开合、页签变了让外壳重画 |
| ipdrun-20 | [#19](https://github.com/openatta/Harnix/issues/19) | space 不能打开设置面板的某一节 |
| ipdrun-21 | [#20](https://github.com/openatta/Harnix/issues/20) | 前台会话变了，space 收不到通知 |
| ipdrun-22 | [#21](https://github.com/openatta/Harnix/issues/21) | space 在左栏不能显示角标 |
| ipdrun-23 | [#22](https://github.com/openatta/Harnix/issues/22) | space 不能让中区的对话只读 |
| ipdrun-24 | [#23](https://github.com/openatta/Harnix/issues/23) | space 不能在转录顶部放固定的内容 |
| ipdrun-25 | [#24](https://github.com/openatta/Harnix/issues/24) | space 的提示只在新对话首屏出现，打开的会话里没有 |
| ipdrun-26 | [#25](https://github.com/openatta/Harnix/issues/25) | space 不能让对话正文里的某种模式变成可点的 chip |
| ipdrun-27 | [#26](https://github.com/openatta/Harnix/issues/26) | `@` 的候选不能由应用贡献 |
| ipdrun-28 | [#27](https://github.com/openatta/Harnix/issues/27) | 输入框没有「立即送达」（steer）的选项 |
| ipdrun-29 | [#28](https://github.com/openatta/Harnix/issues/28) | `/` 命令只能插入文字，space 也藏不掉引擎的命令 |
| ipdrun-30 | [#29](https://github.com/openatta/Harnix/issues/29) | space 的图标只能从外壳自带的那几十个里挑 |
| ipdrun-31 | [#30](https://github.com/openatta/Harnix/issues/30) | 桌面模板在没有 profile 时把状态目录写死成 `~/.harnix` |
| ipdrun-32 | [#31](https://github.com/openatta/Harnix/issues/31) | 外部应用的两个 pnpm workspace：模板少一行，文档少两句 |
| ipdrun-33 | [#32](https://github.com/openatta/Harnix/issues/32) | 应用不能给页面标题加待处理数前缀 |
| ipdrun-34 | [#33](https://github.com/openatta/Harnix/issues/33) | `/` 命令拿不到命令名后面输入的文字 |
| ipdrun-35 | [#34](https://github.com/openatta/Harnix/issues/34) | 窄于 960 时右栏够不着 |
| ipdrun-36 | [#35](https://github.com/openatta/Harnix/issues/35) | `composer.insert` 不能指定会话，刚打开的会话插不进去 |
| ipdrun-37 | [#36](https://github.com/openatta/Harnix/issues/36) | `@` 菜单里应用贡献的候选被文件挤掉 |
| ipdrun-38 | [#37](https://github.com/openatta/Harnix/issues/37) | space 不知道外壳的模态层开着，Esc 同时落到 space 的键盘处理上 |
| ipdrun-39 | [#38](https://github.com/openatta/Harnix/issues/38) | space 说不了自己当前是哪个页签，换回来总在第一个 |
| ipdrun-40 | [#39](https://github.com/openatta/Harnix/issues/39) | 打开会话时定位不到其中的某一条 |
| ipdrun-41 | [#40](https://github.com/openatta/Harnix/issues/40) | tap 落后时，逐帧读的调用方拿不到缺口里的结算；结算也不带回答文本与工具调用 |
| ipdrun-42 | [#41](https://github.com/openatta/Harnix/issues/41) | 没有轻量的 Rust 客户端 |
| ipdrun-43 | [#42](https://github.com/openatta/Harnix/issues/42) | 桌面模板的壳不能当自己子进程的客户端 |
| ipdrun-44 | [#43](https://github.com/openatta/Harnix/issues/43) | 模块侧发消息不能带附件与 `@session` 引用 |
| ipdrun-45 | [#44](https://github.com/openatta/Harnix/issues/44) | 回放只能非严格 |
| ipdrun-46 | [#45](https://github.com/openatta/Harnix/issues/45) | 产品关不掉整个外壳的「新会话」（`[shell] new_session`） |
| ipdrun-47 | [#46](https://github.com/openatta/Harnix/issues/46) | 左栏的 space 列表不能分组（`SpaceContribution.group`） |
| ipdrun-48 | [#47](https://github.com/openatta/Harnix/issues/47) | space 不能在左栏暂时灰掉（`SpaceContribution.disabled`） |
| ipdrun-49 | [#48](https://github.com/openatta/Harnix/issues/48) | 左栏品牌下面没有地方放应用的上下文（`host.shell.context`） |
| ipdrun-50 | [#49](https://github.com/openatta/Harnix/issues/49) | 窄屏时右栏只能盖住中区（`[shell] narrow_details`） |
| ipdrun-51 | [#50](https://github.com/openatta/Harnix/issues/50) | 会话建了就不能换 space（`harnix.sessions.move`） |
| ipdrun-52 | [#51](https://github.com/openatta/Harnix/issues/51) | 页面不知道自己在桌面壳里、是不是本地模式（`window.__HARNIX_SHELL__`） |
| ipdrun-53 | [#52](https://github.com/openatta/Harnix/issues/52) | 左栏的 space 入口与下面的会话之间太挤，没有分隔 |
| ipdrun-54 | [#53](https://github.com/openatta/Harnix/issues/53) | space 的主会话在左栏会话列表里与 space 入口重复（`SpaceContribution.listSession`） |
| ipdrun-55 | [#54](https://github.com/openatta/Harnix/issues/54) | 再点前台 space 的入口没有反应（`SpaceContribution.reenter`） |

ipdrun-06 已撤回（在公开契约之上能做，模板写 `program` 即可），没有建。

## Helm

提出于 2026-10-27，代码位置对照 `c9dc17e`（`Harnix-v0.4.0-2026102703`）。

| 编号 | 标题 | 类型 | 优先级 | 依赖 |
|---|---|---|---|---|
| [helm-01](helm-01-prompt-answerer.md) | 模块不知道是哪台设备回答了某个工具的权限卡（`harnix.prompts.answerer`） | 新能力 | 中 | — |

## Grabix

提出于 2026-10-30，代码位置对照 `5af5728`（`Harnix-v0.4.0-2026102801`）。已落地：grabix-01、grabix-02 → `0debd3e`；grabix-03 → `c05275a`；grabix-04 → `e1433ff`（main，未发版）。

| 编号 | 标题 | 类型 | 优先级 | 依赖 |
|---|---|---|---|---|
| [grabix-01](grabix-01-space-new-session.md) | space 不能关掉「新会话」（`newSession`） | 新能力 | 中 | — |
| [grabix-02](grabix-02-space-details-width.md) | space 不能定右栏的缺省宽度（`detailsWidth`） | 新能力 | 低 | — |
| [grabix-03](grabix-03-space-header-actions.md) | space 不能在标题栏放自己的操作（`headerActions`） | 新能力 | 中 | — |
| [grabix-04](grabix-04-topic-resubscribe-race.md) | 同一主题退订后立刻再订阅，服务端可能后执行退订，订阅从此丢失 | 缺陷 | 中 | —（与 ipdrun-16 同类） |

## XOps

提出于 2026-11-04，代码位置对照 `3c264da`。已落地：xops-01 → `be9157f`…`2df2b74`；xops-02 → `c0c3a7d`…`eea2ab2`；xops-03 → `458732f`…`ac23191`（main，未推送，未发版）。

| 编号 | 标题 | 类型 | 优先级 | 依赖 |
|---|---|---|---|---|
| [xops-01](xops-01-flow.md) | 流程能力：按站推动会话工作的可选模块（`harnix-flow`、`@harnix/flow`） | 新能力 | 高 | — |
| [xops-02](xops-02-flow-placement-authoring.md) | 流程会话的落点与模型；结构化输出交付、权限规则限写；空应用自带流程；对话里编写流程；试运行 | 新能力 | 高 | xops-01 |
| [xops-03](xops-03-flow-main-agent-space.md) | 流程的主 Agent 与子 Agent；运行状态与暂停；按站的耗时与用量；流程 space 改版；demo 数据 | 新能力 | 高 | xops-02 |

