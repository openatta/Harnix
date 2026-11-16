# 系统 space

> 读者：在 Harnix 上做应用、要知道「系统 space 里看得到什么、它从哪读」的人；给系统 space 加东西的人。
>
> 系统 space 是一个**第一方原生扩展**：后端是扩展 `system`，界面是 UI 包 `@harnix/system`。产品链了它、没有在 `[extensions] off` 里关掉它，它就在。
> 为什么是这个形状见 [概念与架构](concept_and_architecture.md) §5.5。用词照 [术语](glossary.md)。
>
> 本文照代码写：后端在 `crates/ext/system/`，界面在 `ui/packages/system/`；控制台里别的扩展注册进来的页签在它们自己的包里——流程在 `ui/packages/flow/`，
> 扩展在 `ui/packages/packages/`，检查器的轨迹在 `ui/packages/observability/`。对不上时以代码为准。

---

## 1. 它是什么

**开发调测用的基础面：按四层（进程 → 实例 → 会话 → 任务）看到这个 Harnix 管着的一切，并且能操作。** 它不持有对话：
要在一个会话里继续说话，就「到所属 space 打开」。它只用公开的扩展接口、宿主 API 与贡献点，没有私有通道；它的权力就是被授予的四项能力（§10）。

| 栏 | 是什么 |
|---|---|
| 左 | 实例列表（§3） |
| 中 | 控制台（`center: "hidden"`）：上面一条指标条（§2），下面一排页签：系统 space 自己的（活动、全部、诊断、扩展日志）与别的扩展经 `system.console.tab` 注册进来的（流程、扩展、用量） |
| 右 | 选中项的检查器：只出现适用于它的页签（§8），其中一部分由别的扩展经 `system.inspector.tab` 注册；什么都没选时是「说明」页签，说控制台当前页签是什么 |

外壳里它不和别的 space 排在一起：它声明 `place: "bottom"`，入口在左栏底部，角标是正在跑的会话数。还有两条路进来，都是它注册的 `session.action`：
会话标题栏上的「在系统 space 里打开」，以及会话里一行工具调用上的「在系统 space 里看」（跳到这个会话的「轨迹」页签并标出那次调用）。流程的会话也从这里进（§5）。

外壳在握手时从 `hello` 知道 `system` 启用了，才调 `@harnix/system` 的 `register`；`system` 被关掉或没链，这个包不激活，入口、动作与它定义的两个贡献点都不在。

**键盘**：页签条（控制台的、右栏的）各是一个 Tab 停靠点，左右方向键在页签间走并切换，Home / End 到头尾。能点的行
（Agent、子 Agent、任务、轨迹里的轮与调用、扩展）都能用 Tab 走到，回车或空格选中它；Agent 列表里上下键走到上一行、下一行，
右键展开、左键收起；轨迹里的轮和扩展的行回车展开。

**失败用人话说**：调用失败时，认得的错误码说成一句话（例如会话正在跑一轮时，上下文页签说「这一轮结束后才能看」），
后端的原话和错误码放在悬停提示里；不认得的码照原话。横幅里的失败也一样。

## 2. 指标条

| 项 | 值 | 来源 |
|---|---|---|
| Harnix、Agix | 版本（`-` 之前的部分） | 主题 `system.process` |
| 运行时长 | `uptime_secs`，页面每秒往上数（只重画这一格） | `system.process` |
| 内存（峰值） | `peak_rss_bytes` | `system.process` |
| 系统线程 | `threads` | `system.process` |
| 已连接用户 | `connections` 的条数；悬停列出设备、连接方式、拓扑、连上的时间 | `system.process` |
| 实例 | 实例数 | 主题 `harnix.instances` |
| Agent（活动/全部） | 主 Agent 的数 | 每台实例的 `harnix.instances.sessions {instance}` |
| 运行中会话 | 有一轮在跑的会话 | 主题 `harnix.sessions.status` |
| 运行中任务 | 状态是 running / waiting / pending 的任务 | `harnix.session.tasks {instance, session_id}` |
| 等你处理 | `waiting` 不为空的行 | `harnix.instances.sessions` |

**指标去哪**：一格数字可以链到它数的那张列表。系统 space 定义贡献点 `system.metrics.link`（`register`，`{id, metric, title(), open()}`，`metric`
是那一格的 id，即 `data-kpi`）；有人登记了某一格，那一格就画成按钮，悬停是 `title()`，点了调 `open()`。系统 space 不认识谁登记：
`@harnix/harnix-app` 给「等你处理」（`waiting`）登记，点了打开「待我处理」的整张列表（[Harnix app](harnix-app.md) §3.3）。那张列表
除了这里数的会话提问，还有流程等人的任务，所以两边的数可以不一样。没有 `harnix-app` 的产品里这一格照旧只是数字。

`system.process` 是 `system` 的主题（也是同名的方法），有人订阅时每 5 秒重算一次推出；数据是 `system` 凭 `process.observe` 从宿主拿的进程信息。
值里还有产品名与版本（`app`）、`started_ms`。

## 3. 左栏：实例

每台实例一行（主题 `harnix.instances`）：状态点（`starting` / `ready` / `degraded` / `down` / `hibernating`，悬停是理由，没有理由时是状态）、id、这台实例上正在跑的会话数。
收养的实例（`proc.<扩展 id>`，模式 3 扩展进程里它自己的引擎）和别的实例一样列在这里，载体在它的概况里（§8.1）；它休眠时状态是 `hibernating`，
会话列表由产品在回收前存下的那份答，打开这里不会唤醒它。
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
| `waiting = approval` | 待批准（权限询问） |
| `waiting = answer` | 等回答（模型的提问） |
| `waiting = plan` | 等计划审阅（离开计划模式要人审阅计划） |
| `running` | 运行中（排着队时「运行中 · 排队 N」） |
| `tasks > 0` | N 个任务在跑（一轮已结束、任务还在跑） |
| 刚跑完、没人看过 | 已完成，未看 |
| 已加载 | 空闲 |
| 其余 | 未加载 |

「已完成，未看」：上一次推送时还在跑、这一次不跑了、又既没在这里选中也没在外壳里打开的会话。记在页面的
`localStorage` 里（至多 200 条），选中它就清掉。

**每行带**（列表是一张表，顶上一行小字表头：Agent · 状态 · 子 Agent · 任务 · Token · 模型用时 · 最近）：名字列里是名字、
**Harnix 标记**（`started_by = "harnix"`：扩展或内核起的，不是人起的）、所属 space、实例（不是 `main` 时）、目标与它的阶段
（有目标且没完成时；阶段的说法见 §8.2）——放不下时标签先省略；然后是状态、子 Agent 数、任务数、token（进、缓存读、缓存写、
出的合计）、模型用时、在跑的这一轮已经多久（`turn_started_ms`）或离最近活动多久。列表窄时（看的是列表自己的宽度）先收起
模型用时和最近两列，再收起状态的文字（状态点还在）和 token。会话的信任是 `extension` 的（收养的实例上的），名字照不受信内容画。

- 用量来自 `observability.usage {instance, session_ids}`（一次至多 200 个），读的是录像，只覆盖会话最近一次运行（daemon 重启后录像重写），不是总数；没有录像的会话、或者产品没链 `observability`，这几格是「—」。
  每个会话的结果带 `run`（这次运行从何时起，`earlier_run` 为真表示之前的运行不在里面）与 `stale`：休眠的模式 3 进程不为读用量叫醒，给读过的旧数并标 `stale`，界面据此标注。
  行的推送变了就重读；有一行在跑或有任务时，跟着页面共用的 3 秒节拍重读（右栏的概况也在这个节拍上读，两边不各起一个钟）。
  时间格自己每秒走，不重画整张列表。
- 目标来自会话行的 `goal`；中枢还没听到过的，对已加载的会话问一次 `system.goal`。
- 搜索：名字、会话 id、space、实例、场景、项目根。筛选「谁起的」：所有人的 / 人起的 / Harnix 自己的。搜索、筛选和
  「再显示」各页签各记一份，页面开着时切走再回来还是原样。

**展开一行**：团队的看板（有团队时）、子 Agent 树（录像里的子会话按 `parent_session_id` 嵌套，并上 `kind: agent` 的任务）、
其余任务。活动页签上有任务的行一开始就是展开的。点子 Agent 或任务，右栏换成它（§8）。

## 5. 流程

流程是 Harnix 内置的应用开发工具链（第一方扩展 `flow`，界面 `@harnix/flow`）。这个页签不是系统 space 的：`@harnix/flow` 把它注册进 `system.console.tab`，
产品链了 `flow` 才有——Harnix 自己的二进制 `harnix` 链了，二开的产品 `.extension(flow)` 了才有。流程是什么、怎么写见流程指南（`crates/ext/flow/docs/guide.*.md`，
这里右栏的「参考」就是它）。

**页签本身**：流程列表，下面是全部运行与试运行。

- 流程：标题、来源（内置 / 已改 / 自建）、不合法的标记；按来源、合法与否筛选，按名字搜索，按名字或标题排序。「添加」
  打开通用会话并在输入框里放一句提示。数据是主题 `flow.defs`。
- 运行：状态、标题、试运行标记、流程、开始时间、耗时、token；按状态、流程、真跑 / 试运行筛选，按开始、耗时、token 排序。
  数据是主题 `flow.runs`、`flow.dry_runs`。
- 这一页只呈现，不编辑定义：改流程在它的对话里，由 Agent 做。

**点一条流程或一条运行，就进了这条流程**：

| 栏 | 是什么 |
|---|---|
| 左 | 「返回系统」；流程名（没选流程时是「通用会话」）；它自己的对话；它的运行与试运行，新的在前 |
| 中 | 左栏选中什么就是它的会话：流程的对话（`flow.session {flow}`，Agent 在里面改它、试运行、启动、操作它的运行，都直接执行、不审批，校验照旧）；运行 → 它的主会话（`flow.get {run}` 的 `@main`；试运行没有会话，是流程的对话）；不属于哪条流程的是通用会话（`flow.session {}`） |
| 二级栏 | 选中的流程：流程图、定义、件、只读 YAML，可以试运行、启动；选中的运行：状态、耗时、token，暂停 / 恢复 / 取消、试运行的单步与跑完、站的确认等操作、子 Agent 与任务 |
| 右 | 只有一个页签「参考」：流程指南（`flow.guide {lang}`）、本产品的目录与字段表 |

「返回系统」回到控制台，停在流程页签上。在流程里时左栏底部系统 space 的入口仍然亮着。

**同一条运行的另一个入口：项目里的任务**（[Harnix app](harnix-app.md) §3.2）。项目树里点一个任务，走的也是这条路——流程的界面 space
带到前台、停在这条运行上，主会话在中区——只是左栏的「返回」回到项目 space（`FlowSpaceState.enter(flow, run, { from })`），
会话旁的详情面板不开，改在右栏开「流程」页签（`sidebar.tab`，`kind: "flow"`，`keepMounted`；`openTab("flow", { params: { run } })`；
没带 `run` 时按这个会话在浏览器里记下的，再没有就在最近的运行里找主会话是它的那条）。
两处画的是同一组件（`ui/packages/flow/src/RunPanel.tsx`）：`RunHead`（状态，暂停 / 恢复 / 取消）、`FlowGraph`（当前站高亮）、
选中站的 `StationPanel`（确认、放行、退回、强制通过……按站给）、`RunStations`（每站的状态、耗时、token、输出一行、门禁结果）、
`OutputDirLine`（输出目录：复制路径、打开文件页签）、`RunSessionsList`（子 Agent 与任务）。控制台这一侧仍是 `RunDetail`，只是从这里取件。

**它的 space**：流程是一个界面 space，`within` 系统 space：不列在左栏，只能由代码进入。id 是 `harnix.spaces.list` 里
scene 为 `flow-author` 的那个后端 space（缺省 `flow`，由 `flow` 扩展声明），所以旧的会话和深链接照开。

**挂在哪由点有没有定义决定，不按 id**：

| 产品里有 | 结果 |
|---|---|
| `system` 和 `flow` | 流程在系统 space 的控制台里，如上 |
| `flow`，没有 `system` | `system.console.tab` 没人定义，`@harnix/flow` 的 `orElse` 注册一个单独的、列在左栏的流程 space：左栏是会话列表，右栏是「流程」「运行」「参考」三个页签，没有「返回」 |
| 没有 `flow` | 哪里都没有流程的界面 |

自己编外壳产物的产品，把 `@harnix/flow` 编进去就行；它在哪由上表决定，产品不用另写注册。

## 6. 扩展

`@harnix/packages` 把这个页签注册进 `system.console.tab`：产品链了 `packages`（管扩展包的宿主组件）才有。
数据只有一个：主题 `packages.list`（值同方法 `packages.list`；载体报告状态变了才推，另有至多 10 秒一次的兜底，见 [扩展包](harnix-ext-package.md)）。
**这一页只看，不操作**：启停、同意、数据、卸载、更新在设置 → 扩展。

表上方：模式 3 的扩展进程不在沙箱里时，一条红字写明原因——`[ext.packages] sandbox = "off"`（只给开发用），或者这个平台套不上沙箱（扩展进程因此不拉起）。

表有五列；放不下时表在自己的框里横向滚动，不撑宽页面，表头不折字。

| 列 | 是什么 |
|---|---|
| 扩展 | 状态点、标题或 id（扩展进程没有沙箱时跟一个「无沙箱」标记），下一行 id 与版本 |
| 状态 | 见下表 |
| 后端 | 只对带后端的扩展包：休眠 / 运行中 / 未运行 |
| 调用 · 出错 | `calls.calls` · `calls.errors` |
| 被拒 | 最近被拒的调用的条数（它以 `ext:<id>` 经 `host.call` / `host.subscribe` 要、同意的 `calls` 里没有的，或者在它的 space 之外的；留最近 20 条） |

| 状态 | 标签 | 什么时候 |
|---|---|---|
| `running` | 运行中 | 起来了 |
| `hibernating` | 休眠中 | `[backend] resident = "on-demand"` 的后端闲置后卸下了（模式 3 连进程一起停了），下一次要它做事时再起 |
| `faulted` | 已搁置 | 反复崩溃，不再拉起（模式 3：起来之后 60 秒内崩溃 3 次）；记录仍是启用的，方法答「不可用」；在设置里停用再启用会重起。一个工具连续出错只把那个工具放在一旁（`health.tools_aside`），扩展照常 `running` |
| `stopped` | 已停用 | 安装记录是停用的 |
| `failed` | 没有启动 | 启用着但没起来（验签不过、复核不过、名字被原生扩展占了 `namespace_taken`、模式 3 的沙箱套不上……），理由在展开的那一行 |

点一行（或回车）展开：虚拟机的四个数——换新 / 重建（`recycles`，超时后换掉 Store / `rebuilds`，崩溃后重建）、
故障 · 超时（`faults` · `timeouts`）、内存（`memory_bytes / memory_limit_bytes`）、存储（`storage_bytes / storage_limit_bytes`，
kv 加 `/data`）；模式 3 另有扩展进程 stderr 的最后几行（至多 40 行）；没起来或置于一旁的理由、最近一次错误、
它的方法与主题、**最近被拒的调用**（方法 — 理由）。本页上它的界面面板是外壳的，在诊断里（§7）。

### 6.1 扩展日志

`@harnix/system` 自己往 `system.console.tab` 注册的页签（`system.extlog`，排在别的扩展的页签后面）：本进程记下的、属于各个扩展的日志——原生扩展在应答调用、
计算主题、启动与停止时写的，WASM 扩展包自己的日志（`host::log`）与载体关于它的行，模式 3 扩展进程的 stderr，宿主关于某个扩展的行。怎么认一行是谁的见
[原生扩展](harnix-ext.md) §17.9。进程只留最新的：每个扩展至多 500 行，全进程至多 4000 行，超出时先丢最旧的，页签上方说丢了多少。

数据是 `system.extensions.log {extension?, level?, after?, limit?}`：`system` 凭 `process.observe` 读内核的 `harnix.extensions.log`，设备直接调后者被拒。

| 控件 | 做什么 |
|---|---|
| 扩展 | 全部，或只看一个（每个扩展后面是它留着的行数）；换了就从头读 |
| 级别 | 全部，或某一级及以上；换了就从头读 |
| 实时 | 缺省开：每 2 秒读一次上次最后一行之后的，新行加在底下；读者往上翻时不把视图拉回底部。关掉就不再读 |
| 复制 | 把显示着的行按 `时间 级别 扩展 [来源] 消息` 一行一条复制成文本 |

每行：时间、级别（错误、警告着色）、扩展 id、来源（调用中 / 扩展包 / stderr / 宿主，悬停是 tracing 的 target）、消息（之后跟着事件的其余字段 `name=value`）。
页面至多留 1000 行。

### 6.2 用量

`@harnix/observability` 往 `system.console.tab` 注册的页签（`observability.summary`，排在「扩展」之后、「扩展日志」之前）：
**最近 30 天里每个会话用了多少**，从录像里读。产品没链 `observability` 就没有这个页签。

**读谁**：每台实例（主题 `harnix.instances`）列出的主 Agent（`harnix.instances.sessions {instance}`）里，`last_active` 落在这 30 天里的
（没写 `last_active` 的也问）。按实例、一次至多 200 个调 `observability.usage.daily {instance, session_ids, days: 30, tz_offset_min}`，
`tz_offset_min` 是这台设备此刻在 UTC 以东多少分钟：一次调用算在它发出的那天，日界按这台设备的时区。每个会话的数含它下面的子 Agent，
一个会话只属于它所在的项目（会话行的 `project_root`）。打开页签读一次，「重新读」再读；页签不跟着轮次自己刷新。

**页面**，从上到下：

| 部分 | 是什么 |
|---|---|
| 合计 | 会话（有调用落在这些天里的）、调用（轮次的调用，`+N` 是轮次之外的：压缩、记忆、标题）、token 合计、输入、输出、缓存读、缓存写、模型用时 |
| 每天的 token | 30 根竖条，一天一根，高度按最多的那天折算，没调用的那天只留底线；悬停是那天的 token 与调用数 |
| 按项目、按模型、按天 | 三张表，列是名字、会话、调用、输入、输出、缓存读、缓存写、合计、模型用时；点表头按那一列排，再点反过来（数字列先从大到小）。没有项目的会话归在「（无项目）」，录像没写模型的归在「（未写明）」；按天只列有调用的那几天 |
| 别的包加的区块 | 贡献点 `observability.usage.section`（见下），按 `order` |

读不到的实例（连不上、重启中）在合计下面点名，其余照常；休眠的模式 3 进程不为这个叫醒，给的是之前读过的数，页面说有几个会话是这样。

**数的口径**：录像只覆盖会话最近一次运行（§4），更早的运行用掉的不在里面，所以这不是「这 30 天一共用了多少」，而是「这些会话现在的录像里、
落在这 30 天的那部分」。读者为这个页签另记每个会话按天按模型的合计（每个时区偏移各一份，至多 1024 份，只在内存里）：链路没见到这个会话
再有动静，就不再读它的录像。

**区块**：`@harnix/observability` 定义贡献点 `observability.usage.section`（呈现，`{id, title(), order?, render(context)}`，
`context` 是 `{since, days}`：`since` 是第一天零点的毫秒数），别的包往里加自己那一份，页签不认识它们。`@harnix/flow` 加「按流程」：
`flow.list {limit: 2000}` 读最新的运行，`created_at` 不早于 `since` 的、不是试运行的，按流程加起来——运行数（没全结束时括号里是结束了的）、
轮数、四项 token 与合计（运行记录里各站的轮次报上来的，含重试）、运行时长（结束了的运行从开始到结束，含等人的时间）。这一份来自流程自己的
运行记录，和上面的录像是两本账：流程的会话也在上面的表里。

## 7. 诊断

和设置 → 诊断是同一个面板（外壳核心的）：

- **引擎**：`engine.daemon.status` 的版本、运行时长、会话数，以及启用的场景。
- **运行期界面模块**：贡献点 API 版本、`[ext.plugins] ui_modules`、页面的 import map 与 CSP，以及各 Agix 插件的 `[[ui]]` 条目（`plugins.list`）。
- **扩展**：每个扩展（hello 与 `harnix.ui` 说的）是否启用、载体、信任、版本；在本页上有界面面板的，下面跟着**本页上的面板**
  （在用的个数 · 崩过的次数，每个面的点、id、状态：加载中 / 运行中 / 出错 / 已搁置 / 已停止）与**最近被拒的界面请求**
  （iframe 经桥要、被拒的名字与理由，留最近 20 条）。只是这一页的，刷新就清零；一个面 5 分钟内失败 3 次就被置于一旁。
- **界面部件**：注册表拒掉的贡献与理由，含注册进了没人定义的点、最后被丢掉的那些。
- **自检**：`engine.daemon.doctor`。

`harnix.extensions`（每个扩展：id、信任、载体、版本、启用、授予、行数、停用理由）、`harnix.diagnostics`（扩展、授予、路由表、主题、路由、迁移）与
`harnix.audit`（审计环）由后端提供，这一页不画它们。

## 8. 右栏：检查器

**什么都没选时**右栏是一个「说明」页签，说控制台当前的页签是什么、在那里能做什么。流程、扩展、诊断三个页签没有可以在右栏
看的行：切过去就放掉原来选中的东西，右栏换成这个页签的说明，而不是留着上一个会话的概况。

检查器的页签有两种来路：系统 space 自己的（概况、上下文、任务、原始帧），以及别的扩展经 `system.inspector.tab` 注册的（`@harnix/observability` 的轨迹）。
后者每次画时问 `match(target)`，只在适用于选中项时出现；产品没链 `observability`，这个页签就不在。

### 8.1 选中一台实例

**概况**：实例、状态与理由、实例载体（进程内 / spawn / attach / 收养）、模板、拥有者、版本与协议、场景、进程号、数据根；这台实例的会话
（点了选中）；按模板新建实例（`system.instances.templates` 列全部模板、`system.instances.create`）。操作：重启（`system.instances.restart`，不是 `main` 时；
重启 attach 的实例就是重连）、停止（`system.instances.stop`，拥有者不是内核时，先确认）。

### 8.2 选中一个会话（主 Agent）

五个页签：

| 页签 | 看什么 | 从哪来 | 能做什么 |
|---|---|---|---|
| **概况** | 实例、会话、名字、space、场景、模型、项目根、谁起的；上下文环（实测或估算的 token 对窗口）；这次运行的轮数、调用数、token、缓存命中、输出速度、首字、模型与工具用时，以及子 Agent 的合计；目标（见下） | `observability.usage`、`observability.turns`、`system.context`、`system.goal` | 设定目标（`system.goal.set {objective, max_rounds}`；最多轮数是引擎最多为它开几轮，用完就受阻，缺省 256，表单上写着）、暂停 / 继续 / 编辑 / 清除（`system.goal.update`）；到所属 space 打开；这个 space 的全部会话；压缩、清空、分叉、换模型、中断、导出、改名、关闭、删除 |
| **轨迹** | 这个会话**每一次记录下来的模型调用**，按轮列成一张表：每轮人说的话、每次调用的模型、token、用时、首字、速度、停止原因与回答的开头，嵌着它要的工具调用（名字、输入的开头、结果出没出错）、子 Agent 的调用、旁边的调用（压缩、记忆、标题）；每轮的行上是这一轮的汇总（首字、模型与工具时间、进出、缓存命中、速度、上下文增量或累计）；点开一条在旁边是输入（系统块、工具、消息、配置）与输出，可以和上一次比。和对话区「轨迹」视图是同一个组件（[Harnix 扩展](harnix-ext.md) §22.10），在这个窄栏里检查器占列表的位置 | 录像，`observability.trace` / `observability.trace.call {index}` / `observability.turns`；轮次 `harnix.session.outline`；反馈 `engine.session.entries` | 折叠轮、调用、子 Agent；搜索（含折起来的）；只看失败、只看工具，跳到失败处；时间概览上拖选、滚轮缩放；在对话里定位这一次；**在会话里打开**（到所属 space、直接用轨迹画） |
| **上下文** | 下一次调用**将要**带的上下文：系统、工具、消息的比例，实测（上一次调用）与估算（下一次），压缩阈值；每个系统块的来源与正文、工具表（这部分是 `@harnix/react` 的 `ContextReport`，会话右栏的「上下文」页画的是同一份）、最近 50 条消息 | `system.context`（底下是 `engine.session.inspectContext`，另带 `measured`）、`engine.session.history` | 改写人说的话（`system.messages.edit`）、去掉消息（`system.messages.drop`）、从某条消息分叉（`harnix.session.fork {at_message}`） |
| **任务** | 团队看板（阶段、成员）；子 Agent、团队成员、Shell、监视器、定时任务……的状态 | `harnix.session.tasks` | 停止（3 秒内按两次，`system.tasks.stop`）；展开一个任务见 §8.3 |
| **原始帧** | 这个会话实时的帧：seq、种类、时间、参数（留最近 500） | `harnix.session.attach` 之后的会话事件流 | 按种类筛选、暂停、复制 JSON |

轨迹页签由 `@harnix/observability` 注册，概况里的用量也来自它的方法；产品没链 `observability` 时，这个页签不在，概况里的这几格是「—」。

会话的操作：压缩、清空、换模型、改名是 `system.compact`、`system.clear`、`system.model`、`system.rename`；分叉、中断、关闭、删除是内核的 `harnix.session.*`，
都带 `space: "system"`；导出是 Harnix app 的 `harnix-app.sessions.export`，产品链了 `harnix-app` 才有这一项。

目标的阶段用一个词说，行上的标签和右栏一致：`active` 且 `armed` 是「进行中」；`active` 但没武装（某一轮因取消、出错或用完预算
而结束，或者会话从日志恢复，引擎不再自动开轮，要人「继续」）是「已停摆」，并说明原因；其余照 `phase`：已暂停、受阻、已完成。

### 8.3 选中一个任务或子 Agent

- **任务**：同一组页签，「任务」排第一，展开的就是它：种类、状态、做什么、agent 类型、开始与结束、用时、token、在等什么；
  引擎给的原样 `detail`；**输出**（`system.tasks.output {task_id, from_byte?}`，跟着 `task_progress` 推，至多留 256 KiB，可以不跟）。
  定时任务没有输出。同一份输出也进当前会话的右栏：Shell、监视器的工具行有「查看输出」，开一个 `output` 页签（只读，自动跟到末尾，往回翻就停；`system.tasks.output` 是设备的方法，不限系统 space，任何设备都能读自己会话的任务）。
- **子 Agent**：只有概况：会话、父会话、agent 类型、做什么、模型、用时；它在父会话录像里那份用量；它的消息（`engine.session.history`，
  一次 30 条，往前翻）；「回到它的主 Agent」。
- **给它发话**：在跑的子 Agent 与团队成员下面有一个输入框，回车经 `system.tasks.message {task_id, message}` 送进去，它在下一个
  步骤读到；答 `{delivered, reason?}`，界面照写「已送达」或「没送达：理由」。

## 9. 数据来源

系统 space 只用公开的方法与主题，下面是它和注册进来的页签用到的全部。参数与返回写在各方法登记时的说明里：运行中的进程用 `harnix.reachable` /
`harnix.diagnostics` 列出，代码在各扩展自己的 crate 里；[原生扩展](harnix-ext.md) 有它们的分组索引。

| 主题 | 参数 | 所有者 | 用在 |
|---|---|---|---|
| `system.process` | — | `system` | 指标条 |
| `harnix.instances` | — | 实例登记表 | 实例列表、指标条 |
| `harnix.instances.sessions` | `instance` | 中枢 | Agent 页签、实例概况、指标条 |
| `harnix.session.tasks` | `instance`、`session_id` | 中枢 | 展开的行、任务页签、子 Agent |
| `harnix.sessions.status` | — | 中枢 | 运行中会话、左栏角标 |
| `packages.list` | — | `packages` | 扩展页签 |
| `flow.defs`、`flow.runs`、`flow.run {run}`、`flow.dry_runs`、`flow.focus {instance, session_id}` | | `flow` | 流程 |

| 方法 | 所有者 | 用在 |
|---|---|---|
| `system.context`、`system.goal`、`system.goal.set`、`system.goal.update` | `system` | 概况、上下文、Agent 页签的目标 |
| `system.compact`、`system.clear`、`system.model`、`system.rename`、`system.messages.edit`、`system.messages.drop` | `system` | 会话的操作 |
| `system.tasks.stop`、`system.tasks.message`、`system.tasks.output` | `system` | 任务 |
| `system.instances.templates`、`system.instances.create`、`system.instances.restart`、`system.instances.stop` | `system` | 实例 |
| `system.extensions.log` | `system` | 扩展日志 |
| `observability.usage`、`observability.turns`、`observability.trace`、`observability.trace.call` | `observability` | Agent 页签的用量、概况、轨迹（按会话判的三个声称 `space: "system"`） |
| `observability.usage.daily` | `observability` | 用量页签 |
| `flow.list`、`flow.defs` | `flow` | 用量页签的「按流程」 |
| `harnix.session.outline`、`engine.session.entries {namespaces: ["harnix.feedback"]}` | 中枢、引擎直通 | 轨迹的轮次与反馈 |
| `harnix.sessions.list`、`harnix.session.fork`、`harnix.session.interrupt`、`harnix.session.close`、`harnix.session.delete`、`harnix.session.attach`、`harnix.session.detach` | 中枢 | 会话，都带 `space: "system"` |
| `engine.session.history` | 引擎直通 | 上下文、子 Agent 的消息 |
| `harnix-app.sessions.export` | `harnix-app` | 导出（链了 `harnix-app` 才有） |
| `engine.daemon.status`、`engine.daemon.doctor`、`plugins.list` | 引擎直通、`plugins` | 诊断 |
| `flow.*` | `flow` | 流程 |

监听的通知：`harnix.turn_settled`（一轮结束后重读）、`harnix.event`（任务输出、原始帧）。

## 10. 后端：扩展 `system`

**它的权力就是四项能力。** `harnix::first_party::standard()` 授予 `system`：`instances.admin`、`engine.inspect`、`engine.control`、`process.observe`；它声明一个 `cross_space()` 的
space `system`（界面一侧 `@harnix/system` 把它画成 `center: "hidden"`、`place: "bottom"`）。骨架里没有一行认 `system` 这个 id：一个被授予同样四项能力、声明同样 space 的扩展能做系统 space 能做的每一件事，
不授予就每一样都被拒——这有测试盯着。

**设备够不着的引擎方法包在 `system` 后面。** 设备自己永远没有能力：`engine.session.inspectContext`、`taskOutput`、`goal` 要 `engine.inspect`，
`stopTask`、`messageTask`、`dropMessages`、`editMessage`、`compact`、`clear`、`setModel`、`rename`、`setGoal`、`updateGoal` 要 `engine.control`。`system` 把它们包成 13 个设备能调的方法：
`system.context`、`messages.drop`、`messages.edit`、`tasks.stop`、`tasks.message`、`tasks.output`、`compact`、`clear`、`model`、`rename`、`goal`、`goal.set`、`goal.update`。
设备调它，它以自己的身份、凭自己的能力调那个 `engine.session.*`，两次判定都进审计，后者挂在前者下面。`instance` 不写是 `main`，`session_id` 必填；
它转过去的调用总声称它自己的 space `system`，不管设备声称的是什么，所以替设备读别的 space 的会话，在审计环与日志里记一条「跨 space」，与设备直接调一致。
其余会话方法（`engine.session.history`、`harnix.session.fork`、`interrupt`……）设备直接调，带上 `space: "system"`。

另外几个：

- `system.instances.stop`、`restart`、`create`、`templates`：以自己的身份、凭 `instances.admin` 做设备做不到的实例操作——停、重启别的拥有者的实例，按任何模板建，列出全部模板
  （设备经 `harnix.instances.templates` 只看得到 `ui` 模板）。仍然只能是启动时定下的模板；内核拥有的实例谁都不能停。
- `system.process`：方法与主题，有人订时每 5 秒算一次，数据凭 `process.observe` 从宿主拿。
- `system.extensions.log`：转给内核的 `harnix.extensions.log`（要 `process.observe`），只传 `extension`、`level`、`after`、`limit`。日志里的行可能说出包作者没打算给别人看的东西，
  所以内核那一行不对设备与扩展包开放，设备经这里读，每次读都进审计。
- 录像与用量不在这里：轨迹、用量是 `observability` 的（凭 `recordings.read`），经引擎的协议读。按会话判的那几个方法（`trace`、`trace.call`、`turns`、`calls`）先以调用方的身份 `engine.session.get` 一次，中枢按声称的 space 判：设备只看得到自己 space 的会话，声称 `system` 的跨 space 读进审计。轨迹的列表只有概况与工具调用的名字、输入的开头，输入与结果点开检查器才取；整棵会话树（含子 Agent）在根会话的一份录像里。

**跨 space**：声称 `system` 的调用可以观看、操作任何 space 的会话，每一次跨 space 的操作都写审计；在系统 space 里看别的 space
的会话是「观看」，不改归属。`cross_space()` 是**防误**的标志：同页的代码能声称任何 space。真正的跨 space 读写权力是上面那几项能力，它们不对设备与扩展包开放。

**去掉它**：不链 `system`，或者 profile 写 `[extensions] off = ["system"]`。它的方法、主题、space 连同授予一起从路由表撤下；`harnix.spaces.list` 里没有它；
任何声称 `space: "system"` 的调用一律 `SPACE_MISMATCH`——**撤的是方法，不只是不画**。界面一侧，`@harnix/system` 不激活，它定义的两个贡献点没人定义，
注册进去的页签照各自的 `orElse` 挂到别处或不出现。要在没有系统 space 的产品里换模型，开 `[shell] model_switch`（`harnix.session.setModel`，只作用于调用方声称的 space）。

**Agix 没给的，不画假的。** 页签或按钮在 Agix 补上之前不出现，而不是做成灰的或者在 Harnix 侧模拟。

## 11. 给它加东西

系统 space 是一个普通的第一方扩展，加东西走的是任何扩展都走的路：

- **后端**：你自己扩展的方法与主题（[原生扩展](harnix-ext.md)）。要读写任何会话的内部，在 `manifest` 里要 `engine.inspect` / `engine.control`，产品在装配时授予；
  不要包一层 `system.*`。
- **界面**：往系统 space 定义的三个点里注册——控制台的页签进 `system.console.tab`（`{id, title(), render(), order?}`），检查器的页签进 `system.inspector.tab`
  （`{id, title(target), match(target), render(target)}`），指标条上一格数字去哪进 `system.metrics.link`（§2）。要在没有系统 space 的产品里也有个去处，注册时给 `orElse`。
  用量页签里的一个区块进 `observability.usage.section`（§6.2），那个点是 `@harnix/observability` 定义的。
- 流程、扩展、轨迹、用量这几个页签就是例子：它们分别是 `flow`、`packages`、`observability` 的 UI 包注册进来的，系统 space 自己不认识它们。
- 应用自己的、和系统 space 无关的调试面，做成自己的 space，或者一个扩展包的 space。
