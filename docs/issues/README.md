# 下游项目提的需求

这个目录放在 Harnix 上做应用的项目提过来的需求：缺陷、要加的能力、文档该补的地方。一个需求一个文件，文件名是
`<项目>-NN-<英文短名>.md`，前缀是提出的项目，`NN` 由那个项目在本仓库里自己编号。

每份的结构相同：类型与优先级；现象或需求（带代码位置）；建议（接口草图）；兼容性（对现有应用、缺省行为、协议与贡献点 API 版本的影响）；
验收（本仓库的测试怎么证明）；依赖；来源（提出方在哪里撞到的，一两句）。

规矩：需求、接口、验收只用 Harnix 自己的概念（用词照 [glossary](../glossary.md)），提出方的业务只出现在「来源」里；接口只加不改，
旧调用方不改代码就能继续工作，缺省行为不变（缺陷修复改掉的那一处行为在「兼容性」里写明）。

落地时，提交说明里写上对应的编号（如 `ipdrun-05`）。

## IPDrun

提出于 2026-10-27，代码位置对照 `790b3a2`（`Harnix-v0.4.0-2026102701`）。同日按框架边界复审过一遍：撤回的留一行「已撤回」，编号不重排。依赖一栏里 `Agix ipdrun-NN` 指 Agix 仓库 `docs/issues/` 里的那一条。

### 后端

| 编号 | 标题 | 类型 | 优先级 | 依赖 |
|---|---|---|---|---|
| [ipdrun-01](ipdrun-01-in-flight-frame-dropped.md) | 单双向流上超过在途上限的请求被静默丢掉 | 缺陷 | 高 | — |
| [ipdrun-02](ipdrun-02-sigterm-skips-module-stop.md) | SIGTERM 不走模块的 `stop` | 缺陷 | 高 | — |
| [ipdrun-03](ipdrun-03-testkit-stop-waits-for-exit.md) | testkit 的 `stop()` 不等进程退出就删目录 | 缺陷 | 中 | 宜与 ipdrun-02 一起 |
| [ipdrun-04](ipdrun-04-concurrent-ensure.md) | 两个并发的 `instances.ensure` 同名时，第二个失败 | 缺陷 | 高 | — |
| [ipdrun-05](ipdrun-05-ensure-wait-ready.md) | spawn 实例 `ensure` 返回时还在 starting，紧接着的会话调用失败 | 新能力 | 高 | — |
| ipdrun-06 | （已撤回：在公开契约之上能做，模板写 `program` 即可） | — | — | — |
| [ipdrun-07](ipdrun-07-spawn-instance-leftover-processes.md) | spawn 实例停下以后，它的会话起的后台进程还活着 | 缺陷 | 高 | 正常退出那一半依赖 Agix ipdrun-01 |
| [ipdrun-08](ipdrun-08-daemon-data-root-lock.md) | 同一台实例的新旧两个 daemon 会同时写它的数据根 | 缺陷 | 中 | — |
| [ipdrun-09](ipdrun-09-state-dir-single-process.md) | 同一个状态目录可以同时起两个进程 | 缺陷 | 中 | — |
| [ipdrun-10](ipdrun-10-spawn-template-native-contributions.md) | spawn 模板带不了原生贡献 | 新能力 | 中 | — |
| [ipdrun-11](ipdrun-11-template-instances-inherit-providers.md) | 模板建的实例读不到「设置 → 模型」里配的 provider | 新能力 | 中 | — |
| [ipdrun-12](ipdrun-12-session-methods-without-system-space.md) | profile 去掉系统 space 后，输入框不能换模型（`[shell] model_switch`，缺省关） | 新能力 | 中 | — |
| [ipdrun-13](ipdrun-13-prompt-middleware-for-modules.md) | 模块拿到 Agix 的 prompt 中间件 | 新能力 | 中 | Agix ipdrun-03、Agix ipdrun-04；spawn 载体要 ipdrun-10 |
| [ipdrun-14](ipdrun-14-module-knows-listen-address.md) | 模块不知道进程监听在哪个地址 | 新能力 | 低 | — |
| [ipdrun-15](ipdrun-15-page-token-after-restart.md) | 服务重启后，已经打开的页面永远连不回去 | 缺陷 | 中 | — |
| [ipdrun-16](ipdrun-16-host-subscribe-refcount.md) | `useNest().subscribe` 的退订不计数，同一页里另一个订阅者从此收不到推送 | 缺陷 | 高 | — |
| [ipdrun-17](ipdrun-17-turn-settled-carries-outcome.md) | `harnix.turn_settled` 不带这一轮的结果 | 新能力 | 低 | — |

### 外壳与贡献点

| 编号 | 标题 | 类型 | 优先级 | 依赖 |
|---|---|---|---|---|
| [ipdrun-18](ipdrun-18-host-spaces-switch.md) | space 不能把别的 space 带到前台（`host.spaces.switch`） | 新能力 | 高 | — |
| [ipdrun-19](ipdrun-19-host-details-control.md) | space 管不了右栏：选哪个页签、开合、重画（`host.details.*`、`host.redraw`） | 新能力 | 高 | — |
| [ipdrun-20](ipdrun-20-host-settings-open.md) | space 不能打开设置面板的某一节（`host.settings.open`） | 新能力 | 中 | — |
| [ipdrun-21](ipdrun-21-host-session-onchange.md) | 前台会话变了，space 收不到通知（`host.session.onChange`） | 新能力 | 中 | — |
| [ipdrun-22](ipdrun-22-space-badge.md) | space 在左栏不能显示角标（`badge`） | 新能力 | 中 | 即时刷新要 ipdrun-19 |
| [ipdrun-23](ipdrun-23-space-read-only-conversation.md) | space 不能让中区的对话只读（`readOnly`） | 新能力 | 中 | — |
| [ipdrun-24](ipdrun-24-space-pinned-area.md) | space 不能在转录顶部放固定的内容（`pinned`） | 新能力 | 中 | — |
| [ipdrun-25](ipdrun-25-notice-in-open-session.md) | space 的提示只在新对话首屏出现（`composerNotice`） | 新能力 | 中 | — |
| [ipdrun-26](ipdrun-26-space-chat-ids.md) | 对话正文里的某种模式变成可点的 chip（`ids`） | 新能力 | 中 | 跳到别的 space 要 ipdrun-18 |
| [ipdrun-27](ipdrun-27-mention-contribution-point.md) | `@` 的候选不能由应用贡献（`command` 加 `trigger`，不加贡献点） | 新能力 | 低 | 宜与 ipdrun-29 一起 |
| [ipdrun-28](ipdrun-28-composer-steer.md) | 输入框没有「立即送达」（steer）的选项 | 新能力 | 中 | 宜与 Agix ipdrun-04 一起 |
| [ipdrun-29](ipdrun-29-command-run-and-hide.md) | `/` 命令只能插入文字，space 也藏不掉引擎的命令 | 新能力 | 低 | — |
| [ipdrun-30](ipdrun-30-register-icons.md) | space 的图标只能从外壳自带的里挑（`r.icons`） | 新能力 | 低 | — |

`NestHost` 上的新成员（18–21）与 `SpaceContribution` 上的新字段（22–26、28、29）都是可选的；维护者可以按自己的节奏分几版发，
建议先做 18、19、21、22。

### 模板与文档

| 编号 | 标题 | 类型 | 优先级 | 依赖 |
|---|---|---|---|---|
| [ipdrun-31](ipdrun-31-desktop-product-id.md) | 桌面模板在没有 profile 时把状态目录写死成 `~/.harnix` | 缺陷 | 中 | — |
| [ipdrun-32](ipdrun-32-app-pnpm-workspace-notes.md) | 外部应用的两个 pnpm workspace：模板少一行，文档少两句 | 文档 | 低 | — |

## Helm

提出于 2026-10-27，代码位置对照 `c9dc17e`（`Harnix-v0.4.0-2026102703`）。

| 编号 | 标题 | 类型 | 优先级 | 依赖 |
|---|---|---|---|---|
| [helm-01](helm-01-prompt-answerer.md) | 模块不知道是哪台设备回答了某个工具的权限卡（`harnix.prompts.answerer`） | 新能力 | 中 | — |
