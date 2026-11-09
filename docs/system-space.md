# 系统 space

> 读者：在 Harnix 上做应用、要知道「系统 space 里看得到什么、它从哪读」的人；给系统 space 加东西的人。
>
> 系统 space 是 Harnix 的三部分之一（框架、[Harnix app](harnix-app.md)、系统 space），**始终在**，除非产品在
> `[shell] spaces` 里去掉它。为什么是这个形状见 [概念与架构](concept_and_architecture.md) §5.5。用词照 [术语](glossary.md)。
>
> 本文照代码写：界面在 `ui/packages/shell/src/system/`（入口 `index.tsx`；中栏 `console.tsx`、`agents.tsx`，应用页签
> `apps.tsx`，流程页签 `flows.ts`；右栏各页签在 `pane/`）和 `flow.ts`，流程的界面在 `ui/packages/flow/src/`，
> 后端在 `crates/builtin/src/system.rs` 与 `crates/hub`。对不上时以代码为准。

---

## 1. 它是什么

**开发调测用的基础面：按四层（进程 → 实例 → 会话 → 任务）看到这个 Harnix 管着的一切，并且能操作。** 它不持有对话：
要在一个会话里继续说话，就「到所属 space 打开」。它是内建模块加一个内建界面 space，只用公开的模块 API 与贡献点，
没有私有通道。

| 栏 | 是什么 |
|---|---|
| 左 | 实例列表（§3） |
| 中 | 控制台（`center: "hidden"`）：上面一条指标条（§2），下面五个页签（§4–§7） |
| 右 | 选中项的检查器：只出现适用于它的页签（§8）；什么都没选时是「说明」页签，说控制台当前页签是什么（§8） |

外壳里它不和别的 space 排在一起：入口在左栏底部，角标是正在跑的会话数。还有两条路进来：会话里一行工具调用上的
「在系统 space 里看」（跳到这个会话的「调用」页签并标出那次调用），以及流程的会话（§5）。

注册的条件：`[shell] spaces` 留着 `system`（不写就留），且 `harnix.reachable` 里有 `harnix.system.context`。

**键盘**：页签条（控制台的、右栏的）各是一个 Tab 停靠点，左右方向键在页签间走并切换，Home / End 到头尾。能点的行
（Agent、子 Agent、任务、每轮、应用）都能用 Tab 走到，回车或空格选中它；Agent 列表里上下键走到上一行、下一行，
右键展开、左键收起；每轮和应用的行回车展开。

**失败用人话说**：调用失败时，认得的错误码说成一句话（例如会话正在跑一轮时，上下文页签说「这一轮结束后才能看」），
后端的原话和错误码放在悬停提示里；不认得的码照原话。横幅里的失败也一样。

## 2. 指标条

| 项 | 值 | 来源 |
|---|---|---|
| Harnix、Agix | 版本（`-` 之前的部分） | 主题 `harnix.process` |
| 运行时长 | `uptime_secs`，页面每秒往上数（只重画这一格） | `harnix.process` |
| 内存（峰值） | `peak_rss_bytes` | `harnix.process` |
| 系统线程 | `threads` | `harnix.process` |
| 已连接用户 | `connections` 的条数；悬停列出设备、连接方式、拓扑、连上的时间 | `harnix.process` |
| 实例 | 实例数 | 主题 `harnix.instances` |
| Agent（活动/全部） | 主 Agent 的数 | 每台实例的 `harnix.instances.sessions {instance}` |
| 运行中会话 | 有一轮在跑的会话 | 主题 `harnix.sessions.status` |
| 运行中任务 | 状态是 running / waiting / pending 的任务 | `harnix.sessions.tasks {instance, session_id}` |
| 等你处理 | `waiting` 不为空的行 | `harnix.instances.sessions` |

`harnix.process` 有人订阅时 hub 每 5 秒重算一次推出；值里还有应用名与版本、`started_ms`，带插件 APP 运行时的构建另有
`apps_engine`（引擎指纹）。

## 3. 左栏：实例

每台实例一行（主题 `harnix.instances`）：状态点（`ready` / `degraded` / `down`，悬停是理由）、id、这台实例上正在跑的会话数。
点一行选中这台实例（§8.1）。

## 4. Agent：活动、全部

**每行一个主 Agent**，也就是每台实例的顶层会话（`harnix.instances.sessions {instance}`）；子 Agent 是会话的任务，不单列，
展开一行才看到。

| 页签 | 列什么 |
|---|---|
| **活动** | 在跑、在等人、有任务在跑、或已加载的主 Agent |
| **全部** | 每台实例的全部主 Agent，含 Harnix 自己起的；一页 100 个，「再显示 N 个」 |

**状态**（排序先按这一列，再按最近活动）：

| 条件 | 标签 |
|---|---|
| `waiting = approval` | 等批准（权限询问） |
| `waiting = answer` | 等回答（模型的提问） |
| `waiting = plan` | 等计划审阅（离开计划模式要人审阅计划） |
| `running` | 运行中（排着队时「运行中 · 排队 N」） |
| `tasks > 0` | N 个任务在跑（一轮已结束、任务还在跑） |
| 刚跑完、没人看过 | 已完成，未看 |
| 已加载 | 空闲 |
| 其余 | 未加载 |

「已完成，未看」：上一次推送时还在跑、这一次不跑了、又既没在这里选中也没在外壳里打开的会话。记在页面的
`localStorage`（`harnix.system.unread`，至多 200 条），选中它就清掉。

**每行带**（列表是一张表，顶上一行小字表头：Agent · 状态 · 子 Agent · 任务 · Token · 模型用时 · 最近）：名字列里是名字、
**Harnix 标记**（`started_by = "harnix"`：模块或内核起的，不是人起的）、所属 space、实例（不是 `system` 时）、目标与它的阶段
（有目标且没完成时；阶段的说法见 §8.2）——放不下时标签先省略；然后是状态、子 Agent 数、任务数、token（进、缓存读、缓存写、
出的合计）、模型用时、在跑的这一轮已经多久（`turn_started_ms`）或离最近活动多久。列表窄时（看的是列表自己的宽度）先收起
模型用时和最近两列，再收起状态的文字（状态点还在）和 token。

- 用量来自 `harnix.system.usage {instance, session_ids}`（一次至多 200 个），读的是录像：没有录像的会话这几格是「—」。
  行的推送变了就重读；有一行在跑或有任务时，跟着页面共用的 3 秒节拍重读（右栏的概况也在这个节拍上读，两边不各起一个钟）。
  时间格自己每秒走，不重画整张列表。
- 目标来自会话行的 `goal`；hub 还没听到过的，对已加载的会话问一次 `harnix.system.goal`。
- 搜索：名字、会话 id、space、实例、场景、项目根。筛选「谁起的」：所有人的 / 人起的 / Harnix 自己的。搜索、筛选和
  「再显示」各页签各记一份，页面开着时切走再回来还是原样。

**展开一行**：团队的看板（有团队时）、子 Agent 树（录像里的子会话按 `parent_session_id` 嵌套，并上 `kind: agent` 的任务）、
其余任务。活动页签上有任务的行一开始就是展开的。点子 Agent 或任务，右栏换成它（§8）。

## 5. 流程

流程是 Harnix 内置的应用开发工具链（模块 `harnix-flow`，界面 `@harnix/flow`），后端注册了流程模块才有这个页签：Harnix app
`harnix` 注册了，二开应用 `.module(Flow::builder()…build())` 了才有。流程是什么、怎么写见流程指南（`crates/flow/docs/guide.*.md`，
这里右栏的「参考」就是它）。

**页签本身**：流程列表（`FlowsTab`），下面是全部运行与试运行（`RunsTab`）。

- 流程：标题、来源（内置 / 已改 / 自建）、不合法的标记；按来源、合法与否筛选，按名字搜索，按名字或标题排序。「添加」
  打开通用会话并在输入框里放一句提示。数据是主题 `app.flow.defs`。
- 运行：状态、标题、试运行标记、流程、开始时间、耗时、token；按状态、流程、真跑 / 试运行筛选，按开始、耗时、token 排序。
  数据是主题 `app.flow.runs`、`app.flow.dry_runs`。
- 这一页只呈现，不编辑定义：改流程在它的对话里，由 Agent 做。

**点一条流程或一条运行，就进了这条流程**：

| 栏 | 是什么 |
|---|---|
| 左 | 「返回系统」；流程名（没选流程时是「通用会话」）；它自己的对话；它的运行与试运行，新的在前 |
| 中 | 左栏选中什么就是它的会话：流程的对话（`app.flow.session {flow}`，Agent 在里面改它、试运行、启动、操作它的运行，改动类操作要批准）；运行 → 它的主会话（`app.flow.get {run}` 的 `@main`；试运行没有会话，是流程的对话）；不属于哪条流程的是通用会话（`app.flow.session {}`） |
| 二级栏 | 选中的流程：流程图、定义、配套、只读 YAML，可以试运行、启动；选中的运行：状态、耗时、token，暂停 / 恢复 / 取消、试运行的单步与跑完、站的确认等操作、子 Agent 与任务 |
| 右 | 只有一个页签「参考」：流程指南（`app.flow.guide {lang}`）、本应用的目录与字段表 |

「返回系统」回到控制台，停在流程页签上。在流程里时左栏底部系统 space 的入口仍然亮着。

**它的 space**：流程是一个界面 space，`within` 系统 space：不列在左栏，只能由代码进入。id 是 `harnix.spaces.list` 里
scene 为 `flow-author` 的那个后端 space（缺省 `flow`），所以旧的会话和深链接照开。

**`[shell] spaces` 怎么管它**：

| 写了 | 结果 |
|---|---|
| `system` 和 `flow`（缺省） | 流程在系统 space 里，如上 |
| `flow`，没有 `system` | 退回一个单独的、列在左栏的流程 space：左栏是会话列表，右栏是「流程」「运行」「参考」三个页签，没有「返回」 |
| 没有 `flow` | 哪里都没有流程的界面（后端模块注册了照样在） |

自己编外壳的应用调 `registerSystemFlows(registry, { host, within })` 或 `registerFlowSpace(registry, { host })`
（例子 `ui/packages/flow/demo/register.tsx`）。

## 6. 应用

后端有插件 APP 的方法（`harnix.reachable` 里有 `harnix.apps.list`，也就是 `[apps] enabled = true`）时才有这个页签。
数据只有一个：主题 `harnix.apps`（值同 `harnix.apps.list`，后端每秒看一次、变了才推，见 [插件](harnix-plugin.md) §17）。
**这一页只看，不操作**：启停、权限、数据、卸载、更新在设置 → 应用。

表有六列；放不下时表在自己的框里横向滚动，不撑宽页面，表头不折字。

| 列 | 是什么 |
|---|---|
| 应用 | 状态点、标题或 id，下一行 id 与版本 |
| 状态 | 见下表 |
| 后端 | 只对带后端的应用：休眠 / 运行中 / 未运行 |
| 调用 · 出错 | `calls.calls` · `calls.errors` |
| 被拒 | 最近被拒的调用的条数（它以 `module:<id>` 经 `host.call` / `host.subscribe` 要、清单的 `hub` 能力里没有的；留最近 20 条） |
| 界面 · 出错 | 本页上它的界面面板：在用的个数 · 崩过的次数（只是这一页的，刷新就清零） |

| 状态 | 标签 | 什么时候 |
|---|---|---|
| `running` | 运行中 | 起来了 |
| `hibernating` | 休眠中 | `[backend] resident = "on-demand"` 的后端闲置后卸下了，下一次调用再起 |
| `faulted` | 已搁置 | 后端重建后再崩，虚拟机被停用；记录仍是启用的，方法答「不可用」；在设置里停用再启用会重起。一个工具连续出错只把那个工具放在一旁（`health.tools_aside`），应用照常 `running` |
| `stopped` | 已停用 | 安装记录是停用的 |
| `failed` | 没有启动 | 启用着但没起来，理由在展开的那一行 |

点一行（或回车）展开：虚拟机的四个数——换新 / 重建（`recycles`，超时后换掉实例 / `rebuilds`，崩溃后重建）、
故障 · 超时（`faults` · `timeouts`）、内存（`memory_bytes / memory_limit_bytes`）、存储（`storage_bytes / storage_limit_bytes`，
kv 加 `/data`）；没起来的理由、最近一次错误、它的方法与主题、**最近被拒的调用**（方法 — 理由）、**本页上的界面面板**
（每个面的点、id、状态：加载中 / 运行中 / 出错 / 已搁置 / 已停止）与**最近被拒的界面请求**（桥上被拒的名字与理由，留最近 20 条）。
一个面 5 分钟内失败 3 次就被置于一旁。

## 7. 诊断

和设置 → 诊断是同一个面板：

- **引擎**：`daemon.status` 的版本、运行时长、会话数，以及启用的场景。
- **运行期界面模块**：贡献点 API 版本、`[ui] runtime_modules`、页面的 import map 与 CSP，以及各 Agix 插件的 `[[ui]]` 条目（`harnix.plugins.list`）。
- **界面部件**：注册表拒掉的贡献与理由。
- **自检**：`daemon.doctor`。

`harnix.diagnostics`（模块、授权、方法、主题、路由、迁移）与 `harnix.audit`（审计环）由后端提供，这一页目前不画它们。

## 8. 右栏：检查器

**什么都没选时**右栏是一个「说明」页签，说控制台当前的页签是什么、在那里能做什么。流程、应用、诊断三个页签没有可以在右栏
看的行：切过去就放掉原来选中的东西，右栏换成这个页签的说明，而不是留着上一个会话的概况。

### 8.1 选中一台实例

**概况**：实例、状态与理由、载体（进程内 / spawn / attach）、模板、拥有者、版本与协议、场景、进程号、数据根；这台实例的会话
（点了选中）；按模板新建实例（`harnix.instances.templates`、`.create`）。操作：重启（不是 `system` 时；重启 attach 的实例就是重连）、
停止（不是 `system`、拥有者不是 `kernel` 时，先确认）。

### 8.2 选中一个会话（主 Agent）

六个页签：

| 页签 | 看什么 | 从哪来 | 能做什么 |
|---|---|---|---|
| **概况** | 实例、会话、名字、space、场景、模型、项目根、谁起的；上下文环（实测或估算的 token 对窗口）；这次运行的轮数、调用数、token、缓存命中、输出速度、首字、模型与工具用时，以及子 Agent 的合计；目标（见下） | `harnix.system.usage`、`.turns`、`.context`、`.goal` | 设定目标（`goal.set {objective, max_rounds}`；最多轮数是引擎最多为它开几轮，用完就受阻，缺省 256，表单上写着）、暂停 / 继续 / 编辑 / 清除（`goal.update`）；到所属 space 打开；这个 space 的全部会话；压缩、清空、分叉、换模型、中断、导出、改名、关闭、删除 |
| **调用** | 这个会话**每一次记录下来的模型调用**：按轮分组，每次的用途、模型、token、用时、首字、速度、停止原因，嵌着它要的工具调用；点开是输入（系统块、工具、消息、配置）与输出，可以和上一次比 | 录像，`harnix.system.trace` / `.trace.call {index}` | 只看失败、只看工具，跳到失败处，时间线上拖选；在对话里找到这一次 |
| **每轮** | 按轮汇总：调用数（加辅助调用）、首字、模型与工具时间、进出 token、缓存命中、速度、上下文较上一轮的增量（或累计） | `harnix.system.turns` | 展开一轮看它的调用 |
| **上下文** | 下一次调用**将要**带的上下文：系统、工具、消息的比例，实测（上一次调用）与估算（下一次），压缩阈值；每个系统块的来源与正文、工具表、最近 50 条消息 | `harnix.system.context`（底下是 `session.inspectContext`，另带 `measured`）、`session.history` | 改写人说的话（`messages.edit`）、去掉消息（`messages.drop`）、从某条消息分叉（`session.fork {at_message}`） |
| **任务** | 团队看板（阶段、成员）；子 Agent、团队成员、Shell、监视器、定时任务……的状态 | `harnix.sessions.tasks` | 停止（3 秒内按两次，`tasks.stop`）；展开一个任务见 §8.3 |
| **原始帧** | 这个会话实时的帧：seq、种类、时间、参数（留最近 500） | `harnix.attach` 之后的会话事件流 | 按种类筛选、暂停、复制 JSON |

目标的阶段用一个词说，行上的标签和右栏一致：`active` 且 `armed` 是「进行中」；`active` 但没武装（某一轮因取消、出错或用完预算
而结束，或者会话从日志恢复，引擎不再自动开轮，要人「继续」）是「已停摆」，并说明原因；其余照 `phase`：已暂停、受阻、已完成。

### 8.3 选中一个任务或子 Agent

- **任务**：同一组六个页签，「任务」排第一，展开的就是它：种类、状态、做什么、agent 类型、开始与结束、用时、token、在等什么；
  引擎给的原样 `detail`；**输出**（`harnix.system.tasks.output {task_id, from_byte?}`，跟着 `task_progress` 推，至多留 256 KiB，可以不跟）。
  定时任务没有输出。
- **子 Agent**：只有概况：会话、父会话、agent 类型、做什么、模型、用时；它在父会话录像里那份用量；它的消息（`session.history`，
  一次 30 条，往前翻）；「回到它的主 Agent」。
- **给它发话**：在跑的子 Agent 与团队成员下面有一个输入框，回车经 `harnix.system.tasks.message {task_id, message}` 送进去，它在下一个
  步骤读到；答 `{delivered, reason?}`，界面照写「已送达」或「没送达：理由」。

## 9. 数据来源

系统 space 只用公开的 hub 方法与主题，下面是它用到的全部。参数与返回的形状见 [SDK](harnix-sdk.md) §21。

| 主题 | 参数 | 用在 |
|---|---|---|
| `harnix.process` | — | 指标条 |
| `harnix.instances` | — | 实例列表、指标条 |
| `harnix.instances.sessions` | `instance` | Agent 页签、实例概况、指标条 |
| `harnix.sessions.tasks` | `instance`、`session_id` | 展开的行、任务页签、子 Agent |
| `harnix.sessions.status` | — | 运行中会话、左栏角标 |
| `harnix.apps` | — | 应用页签 |
| `app.flow.defs`、`app.flow.runs`、`app.flow.run {run}`、`app.flow.dry_runs`、`app.flow.focus {instance, session_id}` | | 流程 |

| 方法 | 用在 |
|---|---|
| `harnix.system.usage`、`.goal`、`.goal.set`、`.goal.update`、`.turns`、`.context`、`.trace`、`.trace.call` | Agent 页签、概况、调用、每轮、上下文 |
| `harnix.system.compact`、`.clear`、`.model`、`.rename`、`.messages.edit`、`.messages.drop` | 会话的操作 |
| `harnix.system.tasks.stop`、`.tasks.message`、`.tasks.output` | 任务 |
| `harnix.sessions`、`session.history`、`session.fork`、`session.interrupt`、`session.close`、`session.delete`、`harnix.attach`、`harnix.detach`、`harnix.sessions.export` | 会话，都带 `space: "system"` |
| `harnix.instances.restart`、`.stop`、`.templates`、`.create` | 实例 |
| `daemon.status`、`daemon.doctor`、`harnix.plugins.list` | 诊断 |
| `app.flow.*` | 流程 |

监听的通知：`harnix.turn_settled`（一轮结束后重读）、`harnix.event`（任务输出、原始帧）。

## 10. 后端：`harnix.system.*`

**设备够不着的引擎方法包在系统模块后面。** 系统模块注册 18 个方法：`harnix.system.context`、`calls`、`trace`、`trace.call`、`turns`、
`usage`、`tasks.stop`、`tasks.message`、`tasks.output`、`messages.drop`、`messages.edit`、`compact`、`clear`、`model`、`rename`、`goal`、
`goal.set`、`goal.update`。其中 13 个以 `system` 模块的身份调引擎（`session.inspectContext`、`stopTask`、`messageTask`、`taskOutput`、
`dropMessages`、`editMessage`、`compact`、`clear`、`setModel`、`rename`、`goal`、`setGoal`、`updateGoal`，模块的 grant），两次判定都进审计；
`trace`、`trace.call`、`turns`、`usage` 读录像；`calls` 给的是请求头。`instance` 不写是 `system`，`session_id` 必填。其余会话方法
（`session.history`、`fork`、`interrupt`……）设备直接调，带上 `space: "system"`。

**跨 space**：声称 `system` 的调用可以观看、操作任何 space 的会话，每一次跨 space 的操作都写审计；在系统 space 里看别的 space
的会话是「观看」，不改归属。声称 `system` 的设备可以按任何模板建实例（仍然只能是启动时定下的模板），可以停止、重启
`system` 与内核拥有的之外的实例。

**去掉它**：`[shell] spaces` 里没有 `system` 时，系统模块不注册，`harnix.system.*` 连同授权一起没有；`harnix.spaces.list` 里没有它；
任何声称 `space: "system"` 的调用一律 `SPACE_MISMATCH`——**撤的是方法，不只是不画**。要在没有系统 space 的产品里换模型，
开 `[shell] model_switch`（`harnix.sessions.setModel`，只作用于调用方声称的 space）。插件 APP 的界面面板永远拿不到 `harnix.system.*`。

**Agix 没给的，不画假的。** 页签或按钮在 Agix 补上之前不出现，而不是做成灰的或者在 Harnix 侧模拟。

## 11. 给它加东西

系统 space 是一个普通的内建 space，加东西走的是任何 space 都走的路：

- 后端：一个模块的方法与主题（[SDK](harnix-sdk.md) §5）；要设备够不着的引擎方法，在 manifest 里 `.grant(…)`。
- 界面：`registerSystem` 之外的东西不往控制台里塞；应用自己的调试面做成自己的 space，或者一个插件 APP 的 space。
- 流程页签是一个例子：流程模块的界面经 `offerSystemFlows` 给控制台一个页签，经 `within: "system"` 把自己的 space 嵌进来。
