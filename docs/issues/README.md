# 下游项目提的需求

在 Harnix 上做应用的项目（IPDrun、Helm、Grabix、XOps……）提的需求——缺陷、要加的能力、文档该补的地方——都在
[GitHub issue](https://github.com/openatta/Harnix/issues) 里记，每个项目一个标签（`ipdrun`、`helm`、`grabix`、`xops`），每条带原文与落地提交。
新需求直接在 GitHub 提，用 `<项目>-NN：标题` 作标题、打上项目标签，正文的结构照已有的：类型与优先级；现象或需求（带代码位置）；
建议（接口草图）；兼容性；验收；依赖；来源。用词照 [glossary](../glossary.md)。落地时，提交说明里写上编号（如 `ipdrun-05`）。

代码注释与提交说明里的编号按下表找。

## IPDrun

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

## Helm

| 编号 | GitHub | 标题 |
|---|---|---|
| helm-01 | [#59](https://github.com/openatta/Harnix/issues/59) | 模块不知道是哪台设备回答了某个工具的权限卡 |

## Grabix

| 编号 | GitHub | 标题 |
|---|---|---|
| grabix-01 | [#55](https://github.com/openatta/Harnix/issues/55) | space 不能关掉「新会话」（`newSession`） |
| grabix-02 | [#56](https://github.com/openatta/Harnix/issues/56) | space 不能定右栏的缺省宽度（`detailsWidth`） |
| grabix-03 | [#57](https://github.com/openatta/Harnix/issues/57) | space 不能在标题栏放自己的操作（`headerActions`） |
| grabix-04 | [#58](https://github.com/openatta/Harnix/issues/58) | 同一主题退订后立刻再订阅，服务端可能后执行退订，订阅从此丢失 |

## XOps

| 编号 | GitHub | 标题 |
|---|---|---|
| xops-01 | [#60](https://github.com/openatta/Harnix/issues/60) | 流程能力——按站推动会话工作的可选模块（`harnix-flow`） |
| xops-02 | [#61](https://github.com/openatta/Harnix/issues/61) | 流程会话的落点与模型、按会话参数交付与限写、空应用自带流程、在对话里编写流程、试运行 |
| xops-03 | [#62](https://github.com/openatta/Harnix/issues/62) | 流程的主 Agent 与子 Agent、运行状态与暂停、按站的耗时与用量、流程 space 改版 |
| xops-04 | [#63](https://github.com/openatta/Harnix/issues/63) | 流程的规则、策略与权限、门禁、钩子；详情进二级栏；demo 会话 |
