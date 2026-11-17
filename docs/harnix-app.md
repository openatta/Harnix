# Harnix app

> 读者：用 Harnix 自己的二进制 `harnix` 的人；在 Harnix 上做应用、想看「用扩展做一个产品面最少要什么」的人；决定自己的产品留不留这几个 space 的人。
>
> Harnix 是两部分：**框架**与**第一方扩展**（[概念与架构](concept_and_architecture.md) §1）。Harnix app 是其中的首页、会话 space 与项目 space：
> 落地的总览、新建会话、按时间看会话、按项目管理会话。它由两半组成：后端是第一方原生扩展 `harnix-app`（`crates/ext/harnix-app`），界面是前端包
> `@harnix/harnix-app`（`ui/packages/harnix-app`）。同在 `harnix` 二进制里、挂在这几个 space 上的别的第一方部分也写在这里：右栏的上下文与输出（§6.1），
> 定时（§9），首页上别的包的区块（§10），流程 space（§11），设置里的诊断（§12）。用词照 [术语](glossary.md)；扩展接口见 [Harnix 扩展](harnix-ext.md)。
>
> 本文照代码写：后端在 `crates/ext/harnix-app/src/`（`lib.rs`、`workspaces.rs`、`files.rs`、`project.rs`、`export.rs`、`store.rs`），界面在
> `ui/packages/harnix-app/src/`（`index.tsx`、`home.tsx`、`sessions.tsx`、`projects.tsx`、`items.ts`、`overview.tsx`、`inbox.tsx`、`sidebar.tsx`、`dialogs.tsx`、`store.ts`），一轮的改动由中枢记（`crates/hub/src/changes.rs`），外壳的菜单、层与新对话页在
> `ui/packages/shell/src/`（`Menu.tsx`、`Shell.tsx`、`dialogs.tsx`、`settings.tsx`；层与菜单的底子在 `ui/packages/react/src/layers.ts`），profile 在
> `crates/assembly/src/profile.rs`，定时（§9）在 `crates/ext/schedule/src/`，观测在 `crates/ext/observability/` 与 `ui/packages/observability/`，
> 流程的界面在 `ui/packages/flow/`。对不上时以代码为准。

---

## 1. 它是什么

| | 首页 | 会话 space | 项目 space |
|---|---|---|---|
| id · 图标 · 次序 | `home` · `home` · 5 | `sessions` · `chat` · 10 | `projects` · `folder` · 20 |
| 标题 | 首页（Home）：新开的页面落在这里（`landing`） | 会话（Sessions）：全部会话，按时间 | 项目（Projects）：按项目看会话 |
| 左栏 | 只有「待我处理 N」（§3.3） | 搜索框 + 全部会话，按时间分组 | 一棵树：项目（工作区），一次只展开一个，它的会话和别的包登记的条目（流程的任务，§3.2）挂在下面 |
| 中区 | 首页的区块（`center: "hidden"`，§10），不持有对话 | 对话（`center: "chat"`） | 对话；新建的会话跑在展开的项目里 |
| 右栏 | — | 会话的停靠面：引导页，文件、预览、改动（§6），`@harnix/observability` 的「输出」（后台 Shell 与监视器的只读输出流，一个后台任务一个页签）与末尾常驻的「上下文」（§6.1），别的包注册的页签，和 `reveal` 打开的详情 | 同左；展开了项目、没有打开会话时是项目的「概况」（§3.2） |
| 后端 space | `home`（`no_sessions()`：不收会话） | `sessions` | **同一个** `sessions`（界面注册里 `sessions: "sessions"`） |

**项目 space 是界面上的一个视图，不是第二个会话集合。** 「按时间看」和「按项目看」是同一批会话的两种排法；会话按 space 绑定是
不变量（[概念与架构](concept_and_architecture.md)），如果它们是两个后端 space，同一个会话就得属于两个 space。所以项目 space
写 `sessions: "sessions"`：两个界面视图，一个后端归属。外壳每次会话调用带的是 `sessions`，不是 `projects`。

**两半，一个扩展。** `harnix-app` 扩展声明后端的会话 space（`SpaceDecl::new("sessions").default_space()`：不声称 space 的设备调用按它检查，
没人安置的会话归它）与首页的 space（`SpaceDecl::new("home").no_sessions()`，§10），提供 `harnix-app.*` 的方法，状态在自己的目录 `~/.<product_id>/ext/harnix-app/data/state.json`（工作区、归档、偏好；
删掉只丢分组）。会话的标题不在这里：它是引擎给会话的名字，`harnix.session.rename` 改的就是它。界面包 `@harnix/harnix-app` 注册三个 `space`、首页的两块（§10）、
`sidebar.group` 的 `harnix-app.time`、会话菜单的归档与导出两个 `session.action`、`@` 的文件补全，以及右栏的文件、预览、改动三种页签、
对话里的改动卡和工具行上「在预览中打开」的 `session.action`（§6）。

Harnix 自己发布的二进制 `harnix`（`crates/cli`）链进全部第一方扩展（`harnix::first_party::standard()`，含 `harnix-app`）、`packages`、流程扩展与定时扩展（`schedule`，§9）。
它的缺省是原生扩展加模式 2（见 [应用开发](app-development.md)）。

**关掉它**：profile 的 `[extensions] off = ["harnix-app"]`。关掉的扩展照样登记（别人占不了它的名字），但不安装、不迁移、不启动；
外壳按 `harnix.hello` 里启用的扩展激活 UI 包，所以 `@harnix/harnix-app` 也不注册，三个 space 的入口随之没有，新开的页面落在剩下的第一个 space 上。去掉之后，不声称 space 的调用
没有缺省 space 可落，可以用 `[extensions] default_space = "<space>"` 指一个已启用扩展声明的 space；指的不是这样一个 space，进程起不来。

## 2. 会话 space

**左栏**：

- **待我处理**：有等人处理的事时，最上面一行「待我处理 N」，点了打开整张列表（§3.3）。
- **搜索**：打字按名字筛；回车经 `harnix-app.search {query}` 搜对话内容（只搜最近的会话时会说），Esc 清掉。
- **分组**：问 `sidebar.group` 的注册，最后注册的那个生效。`@harnix/harnix-app` 注册的是 `harnix-app.time`（「按时间」）：今天、近 7 天、更早，按最近活动排，
  空的组不画。应用注册自己的 `sidebar.group` 就换掉它。
- 每组先列 16 条，「显示更多 N」。归档的默认不列，「已归档 N」切换，记在偏好里（`harnix-app.prefs.set {key: "showArchived"}`）。
- 每行：在跑的点、名字（没有就是「未命名会话」）、多久之前。点了打开；右键或 `⋯` 是会话菜单（§5）。`⋯` 是行旁边的另一个按钮：
  Tab 从行走到它，回车打开菜单。

**数据**：`harnix.sessions.list {space: "sessions"}` 列会话，`harnix-app.workspaces.list` 列项目，外壳把两者按 `project_root` 拼起来
（标题、归档、属于哪个项目）；谁在跑来自主题 `harnix.sessions.status`。

**中区的视图**：会话打开时中区顶上是「对话 | 轨迹」的切换（贡献点 `conversation.view`，[Harnix 扩展](harnix-ext.md) §22.10）。对话是缺省；
轨迹是 `@harnix/observability` 注册的，把这个会话记录下来的每一次模型调用按轮列成一张表，带时间概览与检查器。每个会话各记自己的选择，刷新后还在；
产品没链 `observability` 就没有切换。

**会话头**一行：状态点（等你、正在跑一轮、空闲、打不开）、会话名与它的菜单、space、场景、上下文环（和输入框旁边的是同一个，读会话自己的 `context`）、
这一轮的用量。

**对回答的反馈**（中区的对话，`@harnix/react` 的 `ChatPanel`，`features.feedback` 缺省开）：

- 一轮结束后，它的回答下面的动作行里有赞和踩（复制与「重新生成」之间）。还在跑的那一轮没有。给过反馈的那一行一直显示，按下的那个是实心的。
- 点赞或踩都先开一个小对话框（不是模态的，Esc、点外面就关）：踩的时候问哪里不好——不准确、没帮上忙、不安全、太慢、其他，可以不选；
  两种都可以补一句备注。已经给过的，对话框里带着它，可以改、可以撤回。
- 提交走 `harnix.session.feedback`（[Harnix 扩展](harnix-ext.md) §7.3），中枢记成会话日志里的条目 `harnix.feedback` / `rating`；
  改、撤都是再记一条，同一轮最后一条为准。按钮的状态从日志里的这些条目读，所以刷新、重开、分叉之后还在；
  别的设备开着同一个会话，经条目的推送马上看到变化。条目本身不画在对话流里。
- **只记在本机这个会话的日志里，模型看不到，Harnix 不往任何地方送。** 对话框里有一行小字说这件事。应用要收集反馈，
  自己写一个持 `feedback.read` 的扩展（[Harnix 扩展](harnix-ext.md) §7.3）；标准装配里没有。
- **导出带上**：Markdown 在每一轮最后一条消息下面写现在的反馈（`> **Feedback**: down (inaccurate) — 备注`；那一轮已不在转录里的放在末尾）；
  bundle 的 `entries` 和引擎打的 archive（`sessions/<id>/entries.json`）原样带着每一条，包括改过、撤回的历史。

### 2.1 对话里要人答的

会话停下来等人时，对话里出一张卡（`@harnix/react` 的 `AskCard`），左栏那一行与「待我处理」（§3.3）说它在等什么（`waiting`：
`approval` 待批准、`plan` 待审阅、`answer` 等回答）。卡分四种，`@harnix/core` 从引擎的 `prompt` 帧读出是哪一种（`AskForm`），
界面只画它：

| 卡 | 什么时候 | 怎么答（`harnix.session.respond`） |
|---|---|---|
| 权限 | 工具调用要批准 | `decision`：允许一次、本会话内允许、总是允许、拒绝 |
| 计划审阅 | 计划模式里模型调 `ExitPlanMode` | 计划是那次调用之前的回复，按 Markdown 画，连同它说执行时要放行的几类动作。「批准，开始执行」是 `decision: permit`（离开计划模式，回到进入前的模式；引擎没有别的模式可选）；「要求修改」写下意见，是 `decision: deny` 带意见作理由，模型留在计划模式、照意见改 |
| 模型的提问 | 模型调 `AskUserQuestion` | 选一个选项，或用自己的话答（`answer: {type: "answer", text}`，`text` 是选项的 `key` 或原话）；「不回答」是 `{type: "decline"}`。引擎一次问一个问题、只能选一项 |
| MCP server 的询问 | MCP server 在工具调用里要人填表或去一个页面 | 卡上标明是哪个 server 写的。表单按它的 `requested_schema` 画原始类型的字段（文字、数、整数、是否、选一个），先在本地查必填、数的范围与长度；交上去 server 还不收的，卡片回到没答、横幅说为什么，改了再交。页面的只显示完整网址与主机，不自动打开，人自己点开，回来按「已完成」。`answer` 是 `accept`（表单带 `content`）、`decline` 或 `cancel` |

提问与询问只给人开的会话：设备建的会话才向引擎说它能答这两种（扩展建的，如流程，只答权限），所以 `AskUserQuestion` 只在这样的
会话里、且场景允许时才有（`code` 有，`harnix` 没有）。

每张卡都有倒计时（`hello.permission_prompt_timeout_secs`），到零按超时收起：权限按拒绝，提问按「没回答」交还模型，询问回 server
`cancel`。任一设备都能答，先答者生效，后来的回答静默成功；中枢把第一个回答写进会话的流（`ask_answered`），别的设备的卡据此画出答了
什么并注明「在别处回答」。表单里填的内容只交给 server，不广播。卡片全靠原生控件，键盘能走完：Tab 换项，方向键换选项，文字框里回车提交，
修改意见里 ⌘/Ctrl+回车 发回。

## 3. 项目 space

**项目就是工作区**：一个目录，加一个名字。它只是分组：去掉一个项目只是忘了这个分组，会话与历史都还在。

- 左栏顶上「项目」与「+」（添加项目）。先列 5 个，「更多（N）」。有等人处理的事时，最上面是「待我处理 N」（§3.3）。
- **一次只展开一个**（记在页面的 `localStorage`，`harnix.project`）。点一个没展开的项目：展开它，并且停在新对话的空页上。
- 展开的项目列它的会话，和别的包登记的条目（§3.2：`@harnix/flow` 的任务）混在一起按最近活动排，先列 8 行，「全部 N 个」。
  项目行上的数字是两者之和；有在跑的会话或条目时，收起的项目行上有活动点。
- **新会话跑在展开的项目里**：在项目 space 里新建会话时 `project_root` 是这个项目的目录，归属仍是 `sessions`。
- 每个项目行旁边有「＋」：菜单里是「新会话」（§4 的对话框，停在这个项目上），再是各条目类型的新建项（「新任务」）。

**添加项目**（对话框）：

| 做法 | 方法 | 后端 |
|---|---|---|
| 打一个名字 | `harnix-app.projects.create {name}` | 在项目根（`projects_dir`；`--data-dir`，缺省 `$HARNIX_DATA_DIR`，再缺省 `~/Documents`）下建这个目录并登记成工作区；名字只能是一段路径 |
| 选一个已有目录 | `harnix-app.workspaces.create {path}` | 目录要已经存在；同一个目录再加一次是幂等的 |
| 浏览目录 | `harnix-app.listDirectory {path?}` | **选择器从 `projects_dir` 起**：不给 `path` 时打开它（还没有就建；建不了才退到 `$HOME`）；只在 `$HOME` 或项目根之内 |

**项目菜单**：在这个项目里新建会话（打开新会话对话框，§4）、各条目类型的新建项和它们加的项（§3.2，如「为这个项目建流程」）、信任这个项目 / 不再信任这个项目（§3.1）、
改名（`harnix-app.workspaces.update {id, title}`）、移除分组（`harnix-app.workspaces.remove`）。

### 3.1 项目信任

一个项目自己的设置（`<项目>/.harnix/` 下的设置、agent 类型、hook 脚本，项目里的 skill）是仓库内容。引擎缺省**不信任**项目：
没受信的项目只能收紧设置，它自己的 hook、MCP 服务器、比下面几层更宽的权限与沙箱、自己的 provider 与模型端点、遥测去向、
更宽的插件、替换同名类型的 agent、带命令的 skill 都被扣下，不生效。信任是**项目根**的属性，引擎记在全局层的
`settings.local.json`（`project_trust.trusted`）里，和场景无关。

- **项目菜单「信任这个项目…」**：先重读这个项目的状态，在对话框里按类列出信任之后会开始生效的东西（每类后面是它点名的
  MCP 服务器、agent 类型、规则……），确认了才写。已受信的项目菜单里是「不再信任这个项目」，不用确认：它只删指向这个目录的那一条；
  全局设置里另有一条 glob 或缺省就是受信时，项目仍然受信，提示里说是哪一条。没有自己项目层的目录（如 `$HOME`）不出这一项。
- **受信项目**在项目树里名字后面有一个盾牌。
- **横幅**：在项目 space 里打开一个会话，它的项目没受信、又有被扣下的东西时，对话上方有一条横幅（`SpaceContribution.pinned`），
  说哪几类没生效，带「信任…」（同菜单的对话框）和「不再提示」。关掉的按项目记在页面的 `localStorage`（`harnix.trust.dismissed`），
  这个项目以后不再提示；信任之后横幅自己消失。
- 后端读不了信任（产品没授予 `harnix-app` 能力 `engine.control`，见下）时，菜单不出这一项，也没有盾牌和横幅。

设备不持有能力，信任经 `harnix-app` 的方法做；`harnix-app` 持可选能力 `engine.control`，替设备调引擎的 `engine.project.status`、
`engine.project.trust`。**只对设备看得到的项目**：登记过的项目，或列出来的某个会话跑在里面的目录；别的目录答 `REFUSED`
（防误：免得一个页面点名磁盘上任意目录就把它信任了）。`projects.trust` 进审计。

| 方法 | 参数 | 答 |
|---|---|---|
| `harnix-app.projects.status` | `{project_root, instance?, scene?}` | 项目的信任状态（下表） |
| `harnix-app.projects.trust` | `{project_root, trusted, instance?}` | 写完之后的状态，另带 `written_to` |
| `harnix-app.sessions.projectStatus` | `{instance?, session_id, space?}`，按 `RowScope::Session` 判 | 会话所在项目的状态（按会话的场景判），另带 `session_id`；无项目会话是 `trust: "none"`、什么都不扣 |

状态的形状，引擎原样的字段加上 `harnix-app` 算好的三项——项目 space、右栏的「上下文」页读的都是这一份：

```jsonc
{ "project_root": "/Users/x/repo",          // 解析之后的真实目录
  "trust": "untrusted",                     // trusted | untrusted | none（没有自己的项目层）
  "trust_source": "default",                // default | user | host（从这里信任的）| null
  "matched": null,                          // 信任它的那条 project_trust.trusted
  "default": "untrusted",
  "requires_trust": ["hooks_config", "mcp_servers.github"],   // 引擎说要信任才生效的键，信不信都列
  "items": [ { "key": "hooks_config", "category": "hooks", "name": null },
             { "key": "mcp_servers.github", "category": "mcp", "name": "github" } ],
  "categories": ["hooks", "mcp"],           // items 里有的类，按下面的顺序
  "withheld": [ … ] }                       // 现在被扣下的：untrusted 时就是 items，否则为空
```

`category` 是 `hooks`（hook 与脚本）、`mcp`、`permissions`、`sandbox`、`models`（provider、模型端点与令牌、按任务的模型）、
`telemetry`（遥测、录像、指令文件）、`plugins`、`agents`、`skills`、`other` 之一；`name` 是键点名的那一个（MCP 服务器、
agent 类型、skill、规则），没有时是 `null`。主题 `harnix-app.trust`（无参数）的值是 `{projects: {<项目根>: <状态>}}`：
这个进程里最近一次读到或写下的每个项目的状态，变了才推，别的页面据此刷新。在 Harnix 之外改了信任（手改全局设置）不推，
下一次读时才看得到。

设置页的「项目层写了但不生效」照旧由 `settings.describe` 的 `withheld` 逐个字段说，和这里互不重复：那里说某个值没用上，这里说为什么、
怎么放开。

**项目自带的流程**也等信任：`<项目>/.harnix/flows/` 里的流程和件随仓库提交（[流程指南](../crates/flow/docs/guide.zh-CN.md)
「项目自带的流程」），只对这个项目的任务生效，同名时项目的优先。流程扩展自己持 `engine.control`，直接读引擎的
`engine.project.status`，不经 `harnix-app`。没受信时 `flow.defs {project}` 照样列出它们，但不读内容：`origin: "project"`、
`valid: false`、`reason: "untrusted"`，界面显示「信任项目后可用」；`flow.start {project_root}` 启动它答 `REFUSED`。
`flow.defs {project}`、`flow.def {name, project}`、`flow.start` 每次都重读信任和文件，信任之后、文件改了之后不用重启。

### 3.2 项目里的条目与概况：`harnix-app.project.items`、`harnix-app.project.panels`

项目 space 不认识流程，也不认识别的会放进项目的东西：它定义两个贡献点（`ui/packages/harnix-app/src/items.ts`，`definePoint`），
别的 UI 包往里注册；后端 `harnix-app` 在 `UiDecl` 里声明同样两个点，
[贡献点](contribution_points.md)的第二张表由此生成。没有 `harnix-app` 的产品里这两个点没人定义，注册时给 `orElse` 的包（`@harnix/flow`）就什么都不注册。

**`harnix-app.project.items`**（`register`）：项目树里会话之外的一种条目。

```ts
r.register("harnix-app.project.items", {
  id: "flow.task",                                  // <owner>.<kind>
  title: () => "任务",                              // 这种条目叫什么（复数）
  rows: (root) => cache.rows(root),                 // 一个项目目录的行；每次画时读，所以从自己的缓存答
  subscribe: (fn) => cache.subscribe(fn),           // 行变了就叫 fn；树只在项目展开时订阅
  render: (row, context) => <TaskRow … />,          // 行的正文（树给按钮、位置和菜单键）
  open: (row, context) => openTask(…),              // 点了行
  menu: (row, context) => [{ id, label, run }],     // 可选：行的右键和「⋯」
  create: { label: () => "新任务", icon: "fork", run: (context) => newTaskDialog(…) },   // 可选：项目「＋」菜单和项目菜单里的一项
  projectMenu: (context) => [{ id, label, run }],   // 可选：项目菜单（「⋯」、右键）里 create 之后的项：「为这个项目建流程」
}, owner, { orElse: () => undefined });
```

一行是 `{ id, title, at?, running?, session?, data? }`：`at` 是最近活动（毫秒或 ISO 时间，树按它和会话的 `last_active` 一起排，没有的排最后）；
`running` 为真时收起的项目行上有活动点；`session` 是它在中区打开的会话，那个会话是当前会话时行标成当前；`data` 随行带给 `render`、`open`、`menu`。
`context` 是 `{ root, workspace, space, trust? }`，`space` 是项目 space 列会话的后端 space（`sessions`）：一种条目要让自己拥有的会话在这里打开，就把会话展示到它
（`harnix.session.expose`，[概念与架构](concept_and_architecture.md) §5.4 第 6 条）。`trust` 是项目的信任（§3.1），给要用到项目自己文件的条目：
`status()` 每次重读，答 `trusted`、`untrusted`、`none` 或读不到时 `null`；`ask()` 打开和项目菜单同一个「信任这个项目…」对话框，关掉后答是否已受信。
读不了信任的产品里没有这一项。`rows`、`subscribe`、`render`、`open`、`menu` 抛错只丢这一种条目、记进诊断。
行在树里是 `[data-testid="project-item"][data-kind="<id>"][data-item="<row.id>"]`，和会话行一样是 `.srow`、树的一项；它的「⋯」是 `project-item-more`，
项目的「＋」是 `project-new`。

**`harnix-app.project.panels`**（`render`）：项目概况页的一个区块，`{ id, title(), order?, render(context) }`，`context` 同上。

**`harnix-app.session.badge`**（`render`）：左栏会话行上的一个标记，`{ id, render(session) }`，`session` 是 `{ instance, sessionId }`；两个 space 的会话行都画，
在名字和时间之间（`[data-testid="session-badges"]`），没话说时 `render` 答 `null`。每次画行都问，所以从自己的缓存答。抛错只丢这一个标记、记进诊断。
`@harnix/schedule` 在这里画定时的时钟（§9.1）。

**项目概况**（`ui/packages/harnix-app/src/overview.tsx`）：项目 space 里展开了一个项目、又没有打开会话（新对话的空页）时，右栏是它的「概况」。
它是项目 space 自己的页签（`details.tabs`，`id` 是 `harnix-app.overview`），只在这时出现：一打开会话，右栏就是那个会话的停靠面，和别处一样。
展开一个项目时如果没有打开的会话，右栏打开并停在概况上。页面是 `harnix-app.project.panels` 的全部区块，按 `order` 从小到大（没写算 100）；
`@harnix/harnix-app` 自己的几块也是这样注册的，别的包的插在中间。每块抛错只丢那一块。都是只读的，要改的地方给链接。

| `order` | 区块 | 谁 | 内容 |
|---|---|---|---|
| 10 | 待我处理 | `harnix-app.inbox` | 这个项目里等人处理的（§3.3），至多 6 行 |
| 20 | 会话 | `harnix-app.sessions` | 项目最近的 5 个会话（按最近活动，在跑的带点），点了打开；更多时写「全部 N 个」 |
| 30 | 任务 | `flow.tasks`（`@harnix/flow`） | 运行中、等待、已结束各几个；最近的 5 个任务，点了照 §3.2 打开；「新任务」 |
| 40 | 定时 | `schedule.project`（`@harnix/schedule`） | 目标在这个项目里的定时（新会话、任务），每条带状态、下一次时间与操作；「新建定时」（§9.1） |
| 50 | 最近改动 | `harnix-app.changes` | 项目最近 5 个会话的 `harnix.changes` 条目里改过的文件，每个文件只列最近一次，至多 8 个，相对项目根；点了打开那个会话、右栏的「改动」停在那一轮。项目里的会话一轮结束时重读 |
| 60 | 生效的设置 | `harnix-app.settings` | 权限模式（生效的值与写它的层，`settings.describe {project_root}`）、权限规则（各层几条）、MCP 服务器（生效的那些与它们的层，`settings.mcp.list {project_root}`）、Skill（项目最近一个会话看得到的，按来源层计数，`harnix.session.skills`；没有会话时说「项目里有会话后列出」）；项目层有被扣下的值时标「部分被扣下」；每行「去设置」打开对应的一节。产品没有 `settings` 扩展时，读不到的行不画 |
| 70 | 信任 | `harnix-app.trust` | 已信任、未信任（连同扣下了哪几类）或无需信任（§3.1）；「信任…」或「不再信任这个项目」 |

**任务**（`@harnix/flow`，`ui/packages/flow/src/task.tsx`）：`flow.runs` 主题里 `project_root` 是这个项目的运行（试运行不算），
一行写标题（没有就是对象）、流程名、状态（运行中、等待及原因、已暂停、已完成、已取消）、「第 k/n 站」、在 worktree 里跑的写分支名（合回或丢弃后划掉），由定时启动的带时钟；
时钟后面是别的包经 `flow.task.badge` 放的标记（定时还开着时，`@harnix/schedule` 写它的下一次时间，§9.1）。
点了行：主会话归 `flow` 的后端 space，项目 space 声明的是 `sessions`，直接在这里打开本会被 `SPACE_MISMATCH` 拒绝（[概念与架构](concept_and_architecture.md) §5.4），
所以从项目 space 建的任务，`flow.start` 带上 `shown_in: context.space`，流程扩展把主会话**展示**到 `sessions`（`harnix.session.expose`，可对话；
运行记录与 `flow.runs` 的行带 `shown_in`）：任务就像项目里别的会话一样在项目 space 的中区打开、能插话，会话列表不把它当普通会话列（行带 `exposed_from`，
由任务行来画），右栏开「流程」页签（`sidebar.tab` 的 `kind: "flow"`，`openTab("flow", { params: { run } })`，§11），
页签靠 `flow.runOf {instance, session_id}` 找到自己的运行。关闭、删除、改名这些仍是流程的事，从项目 space 调答 `SPACE_MISMATCH`。
没有展示到这里的运行（从流程 space 或定时启动的）点了行则到流程 space、停在这条运行上（§11），左栏的「返回」回到项目 space；
任务行的「⋯」菜单里「在流程里打开」对任何任务都这样进。
「新任务」对话框：流程（全部，标来源，不合法的不能选）、按 `inputs` 的 schema 生成的表单（字符串、数字、布尔、枚举、字符串数组，其余是 JSON 框）、
对象（必填）、标题、输出目录（留空用缺省 `flows/<流程>-<YYYYMMDD><NN>`，对话框预估当天序号，由扩展最终定）、在 worktree 里跑（见下）；
项目里已有任务在跑时提示；流程不是 `guard: strict` 时写明「这个流程会直接修改项目文件」（在 worktree 里跑时改说「合回之后项目才会变」）。
确认即 `flow.start {flow, subject, inputs?, title?, project_root, output_dir?, workspace?, shown_in}`，然后打开这个任务。
对话框底部「开始」旁边是别的包经 `flow.start.action` 登记的另一种起法（「定时运行这个流程…」，§9.1）：拿到的是已经填的这些（`draft`，对象可以还空着），对话框先关上；
流程详情（流程 space 的二级栏，§11）的标题行上也有同一批按钮，只带流程名。

**对话里的 `/flow <流程> [主题]`**（`@harnix/flow` 的 `ui/packages/flow/src/slash.tsx`，一条 `command`，外壳不认识它）：在项目里的对话中，
「/」菜单列出 `/flow` 和这个项目能启动的每个流程 `/flow <名>`（`flow.defs {project}`，对话的项目取会话列表那一行的 `project_root`）。
发出的 `/flow …` 交给它（`submit`），不发给会话：
- 带了主题、流程又没有必填而没有缺省的输入：直接 `flow.start {flow, subject, project_root, shown_in, from_session}`；
- 没带主题、或流程要填输入：打开上面的「新任务」对话框，停在这个流程上、主题已填好，确认时同样带 `from_session`；
- `shown_in` 和 `from_session.space` 都是对话所在的后端 space（会话列表那一行的 `space`），任务因此在项目 space 里打开；
- 流程扩展往原对话里写一条条目 `task_started`，`@harnix/flow` 用 `flow.block` 画成一张卡（`flow-task-card`）：流程、主题、任务现在的状态和站，
  「打开任务」照上面打开它（主会话在中区，右栏「流程」页签）。原对话不变成任务，也不跳走；
- 对话没有项目：不启动，横幅说任务属于项目、要在项目里的对话中用；没有这个流程、或它不能启动（不合法、项目没受信）：横幅说明，输入框里的字留着。

**为这个项目建流程**（项目菜单，`projectMenu`）：项目的流程存在项目里（`<项目>/.harnix/flows/`，§3.1 末），受信才读，所以先问信任：
已受信就往下走；未受信先确认「先信任这个项目？」，再打开同一个信任对话框；`none`（没有自己的项目层）说明存不进去、不往下走；读不到信任的照样往下，
由流程扩展在保存时拒绝并说明。然后把流程的界面带到前台（「返回」回项目 space），打开流程的通用对话，`flow.focus {instance, session_id, flow: null, project}`
让它从此在这个项目里读写流程，输入框里放好一句写明项目路径的开头，人接着说要什么流程。编写对话的提示词不变。

**在 worktree 里跑**：对话框先问 `flow.workspace_info {project_root}`：答 `{git: true, branch, prefix}` 时开关可用，起始状态照流程的 `workspace`；
答 `{git: false, reason, detail}`（`reason` 是 `not_a_repository`、`bare`、`no_commit`、`no_git`）时开关不可用，下面写原因；流程本身要求 worktree 的，
再提示「在这里启动会被拒绝」。开关动过才带 `workspace`（`worktree` 或 `project`），没动过由流程定。任务在项目之外的 worktree（流程扩展数据目录下的 `worktrees/<运行>`）、
分支 `harnix/flow/<运行>` 上跑（运行记录和 `flow.runs` 的行里有 `worktree: {path, prefix?, branch, base, outcome?}`），过程目录仍在项目自己的
`.harnix/runs/<运行>/`。运行结束后右栏「流程」页签的 worktree 一行给出三个操作：**合回**（`flow.worktree.merge {run}`，按钮写「合回 <当前分支>」，先确认）、
**保留分支**（`flow.worktree.keep {run}`，缺省，不用确认）、**丢弃**（`flow.worktree.discard {run}`，确认、标成危险操作）；合回有冲突时答 `CONFLICT`，
`data.files` 是冲突的文件，项目里什么都不变，页签照实写出来。做过的决定记在运行上（`worktree.outcome: {kind: merged|kept|discarded, at, by, into?, commit?, how?}`），
保留之后还可以合回或丢弃。细节见[流程指南](../crates/flow/docs/guide.zh-CN.md)第二节。

### 3.3 待我处理：`harnix-app.inbox`

一张列表，列出这台设备看得到的、在等人的事：会话里的权限询问、计划审阅、模型的提问、MCP server 的询问（§2.1），流程里等确认、
等放行（hold）、等外部工作项被认领的任务。点一行直达能答它的地方。

**来源**是贡献点 `harnix-app.inbox`（`register`，`ui/packages/harnix-app/src/inbox.tsx`）：`{ id, items(), subscribe(fn), open(item) }`。
一项是 `{ id, kind, label, title, detail?, project_root?, data? }`：`kind` 是等什么（`approval`、`plan`、`answer`、`elicitation`；
`confirm`、`held`、`external`），`label` 是它的说法，`title` 是在等的会话或任务，`detail` 多一行（工具、问句、server、站），
`project_root` 让概况只列自己项目的。列表不认识任何一种来源；`items` 抛错只丢那个来源、记进诊断。

| 来源 | 数据 | 点了 |
|---|---|---|
| `harnix-app.asks`（本包） | 中枢的主题 `harnix.inbox`（[Harnix 扩展](harnix-ext.md) §21.3）：每个会话正在等的每一个提问，带会话的 space、名字、项目和引擎原样的 `prompt` | `host.session.open(会话, {switchSpace: true})`：到会话所属的 space 打开它，卡片就在对话里，照 §2.1 作答；答了这一行就消失 |
| `flow.waiting`（`@harnix/flow`） | 流程自己的 `flow.runs` 主题里阶段是 `waiting`、原因是 `confirm`、`held` 或 `external` 的运行（试运行不算） | 照 §3.2 打开这个任务，右栏「流程」页签上确认、放行 |

中枢不认识流程：它只给会话的提问，流程的部分由流程的界面包从它自己的主题来，页面把两者合在一起。

**在哪里**：项目概况的第一块（只列这个项目的）；首页的一块（全部的，至多 8 行，§10）；首页、会话 space 与项目 space 左栏最上面，有等人的事时出「待我处理 N」（`inbox-open`），
点了在对话框里打开整张列表（`inbox-list`，每行 `inbox-item`，`data-kind`、`data-source`）；首页全局统计的「待我处理」那一格（§10.1）也打开它。

## 4. 新建会话

- **左栏顶上的「新会话」**（⌘K）：不建会话，只打开新对话的空页。前台是 `center: "hidden"` 的 space（首页）或嵌在别的
  space 里的 space 时，先换到第一个能对话的 space。⌘\ 收起左栏，⌘/ 聚焦输入框。
- **快捷键**：上面三个是外壳自己的命令（`shell.session.new`、`shell.sidebar.toggle`、`shell.composer.focus`），缺省键在 Mac 上是
  ⌘K、⌘\、⌘/，别的平台是 Ctrl+K、Ctrl+\、Ctrl+/；「打开或关闭右栏」（`shell.details.toggle`）与「打开设置」（`shell.settings.open`）
  缺省没有键；对话里 ⌥↑ / ⌥↓（别的平台 Alt+↑ / Alt+↓）跳到上一轮、下一轮（`chat.turn.previous`、`chat.turn.next`，§4.2）。UI 包可以加自己的命令（贡献点 `shortcut`，[Harnix 扩展](harnix-ext.md) §22.8）。设置的「快捷键」一节列出全部命令（名字、谁的、
  现在的键）：点键位、按下新组合就录下（Esc 放弃）；要带 Ctrl、⌘ 或 ⌥（功能键除外），复制、粘贴、撤销与浏览器自己的窗口、标签键不收；
  和别的命令同键时提示「已被『…』占用」、不改；可以逐条或全部恢复缺省。改过的键只存在这个浏览器里（`localStorage` 的
  `harnix.shortcuts.<命令>`），别的标签页跟着变。一个键按下时，焦点在输入框或右栏里的命令先于全局的；对话框开着时都不生效；
  焦点在文字框里时，不带 Ctrl、⌘、⌥ 的键是打字。扩展包的页面（iframe）里按下的全局键也交给外壳。
- **窄窗口**（≤ 960 宽）：左栏是 56px 的轨，轨上的「会话」入口、收起键或 ⌘\ 把它作为抽屉盖在页面上打开；在抽屉里选了会话或 space、
  按 Esc、点幕布就收回。打开的右栏是从右边盖住中区的抽屉（`narrow_details = "beside"` 时 720–960 宽仍三栏并排），里面是同一条页签栏、同一个会话的停靠面（[Harnix 扩展](harnix-ext.md) §22.7）；
  会话头角落的按钮在右栏收起时把它展开。宽窗口里人自己
  收起或展开的左栏，窗口变窄再变宽后照旧。
- **新对话的空页**：问候（按钟点是早上、下午、晚上）、这个 space 的 `subtitle()`、输入框、`suggestions()` 的建议（点了就建会话
  并发出）、`notice()`，下面是 `home.card` 的卡片。**第一次发出时才建会话**：`harnix.session.create`（带前台 space 的归属，项目 space 里带项目的
  目录），然后 `harnix.session.send`。Harnix app 的两个 space 不给建议与提示；卡片是首次运行的三步（§4.1），应用的扩展与扩展包也可以给 `home.card`。
- **新会话对话框**（从项目菜单或首次运行的「场景与权限」进来）：场景（启用的场景，引擎缺省的在前）、权限模式（`default`、`acceptEdits`、`plan`、
  `bypassPermissions`）、项目目录（`harnix-app.recentProjects` 给的最近目录，或不带项目），可以浏览目录。
- **换模型**：会话的模型菜单在有 `harnix.session.setModel` 时用它，只对前台 space 的会话；没有它时菜单只显示模型、不能换。有没有它是 profile 的
  `[shell] model_switch`，profile 不写时是产品自己的缺省（`AppBuilder::model_switch`，[Harnix 扩展](harnix-ext.md) §17.1）：`harnix` 二进制缺省开着，
  写 `model_switch = false` 关掉；库的缺省是关，应用不说就把换模型留给设置页。
  同一个菜单下面是这个会话的推理强度（`low` … `max`）与思考开关（`auto` / `off` / `on`），「随设置」是没有会话自己的值；
  新会话的输入框里选的，建会话时经 `options.effort`、`options.thinking_mode` 给。

### 4.1 首次运行

新对话的空页上，输入框下面是三步，每一步是拥有它的那个 UI 包注册的一张 `home.card`，外壳不认识它们：

| 卡（`home.card` 的 id） | 谁的 | 什么时候算做完 | 没做完时 |
|---|---|---|---|
| 模型（`settings.setup.model`，`order` 10） | `@harnix/settings` | `harnix.hello` 说有模型可用：配了 provider，或引擎启动时带着 `ANTHROPIC_*` 的密钥（同输入框上的「配置模型」提示；读不到 provider 时算有）。卡画出来时读一次，此后每关一次对话框（设置页就是在对话框里配的）再读 | 说还没有模型，「打开模型设置」到设置 · 模型与凭据 |
| 项目（`harnix-app.setup.project`，20） | `@harnix/harnix-app` | `harnix-app.workspaces.list` 里有项目 | 说会话可以跑在一个项目（目录）里，「新建或选择项目…」打开添加项目的对话框（§3）；做完了是「打开项目」：到项目 space 并展开它 |
| 场景与权限（`harnix-app.setup.scene`，30） | `@harnix/harnix-app` | 没有可读的完成状态：读过、点「知道了」或关掉就算 | 各一行：新会话缺省的场景（`hello.engine.scene` 和它的说明），缺省的权限模式（引擎设置的 `permission_mode`，经 `settings.describe` 按缺省场景读，读不到是 `default`）和它意味着什么。`code` 在、又不是缺省场景时，加一行推荐它，「用 code 新建会话…」打开停在 `code` 上的新会话对话框；「选场景与权限新建…」打开新会话对话框；「更改缺省…」到设置的权限一节 |

- **做完的一步**：这一页看着它从没做完变成做完的，画成做完（一个勾，模型那步写出新会话用的模型）；页面起来时就已经做完的，不画。
- **关掉**：每张卡右上角的「不再显示」。关掉的记在这个浏览器的 `localStorage`（`harnix.setup.dismissed`，卡的 id 的数组），以后不再画。
- **不占位**：没有 `title`、`render()` 什么都没画的 `home.card`，外壳不给它留位置（`:empty`），所以做完、关掉的步骤从空页上消失。
  三步的外观与「做完、关掉」的记法在 `@harnix/react` 的 `SetupStep`、`useSetupStep` 里，应用的包要加自己的一步也用它。
- **不说信任**：新建的项目是不受信的，但空页上不提这件事。那是对话上方横幅（§3.1）的事：在一个有被扣下设置的项目里打开会话时才说，
  两处不重复。
- 产品关掉 `settings` 扩展时没有模型那一步，关掉 `harnix-app` 时没有后两步。
- **首页也有**：新开的页面落在首页（§10），它的「开始使用」块画的是同一组 `home.card`，上面另有「没有模型」的提示；都做完、关掉之后那一块不显示。

**`harnix` 的缺省场景**：不给 `--profile` 时，`harnix` 二进制的 `[engine]` 是 `scene = "harnix"`、`scenes = ["code"]`
（`AppBuilder::default_engine`，见 [Harnix 扩展](harnix-ext.md) §17.1）：新会话仍在 `harnix` 场景里，`code`（要一个项目的编程 Agent）在新会话
对话框里可选。给了 profile 就照 profile 写的，`--scene`、`--scenes` 照旧盖在上面。

**`[shell] new_session = false`**：整个外壳不画「新会话」、「新会话」的键不归外壳（会话由应用自己建）。**只是界面**：空页上的输入框只看 space
自己的 `newSession`，所以空的 space 仍然给一个输入框；要一个 space 没有它，在那个 space 上写 `newSession: false`（这时画
`placeholder()`）。服务端照旧收 `harnix.session.create`。

### 4.2 对话：轮次导航

对话区右缘有一条竖轨，每轮一个刻度（按开这一轮的那条人的话），两轮起才画。

- **数据**：`harnix.session.outline`（[Harnix 扩展](harnix-ext.md) §21.2）给出整个会话的各轮，打开时读一次，这个会话每结束一轮再读一次；
  正在跑的那一轮从对话里补上。读不到时只列已经画出来的那几轮。
- **刻度**：正在读的那一轮（视图顶上的那一轮，滚到底时是最后一轮）高亮，`aria-current`。轮多时竖轨自己滚动，只挂看得见的刻度，
  长会话也只有几十个。任务的站开始的那一轮，刻度换个颜色（`marks`）。
- **预览**：指针指向或键盘聚焦一个刻度，旁边出一张卡：第几轮、时间、提问一行、回复三行。
- **跳转**：点刻度滚到那一轮的第一条话。还没读进来的老历史（打开时只读最近的 1000 条消息）先按 `seq` 读进来、放在前面，再落到那一轮。
- **键盘**：竖轨是一组按钮，Tab 进来停在当前那一轮，↑ / ↓、Home / End 在刻度之间走，回车跳过去；⌥↑ / ⌥↓ 在任何地方
  跳到正在读的那一轮的上一轮、下一轮（已经读过那一轮开头时 ⌥↑ 先回到它的开头）。焦点在有字的文字框里时 ⌥↑ / ⌥↓ 留给文字框。
- **窄**：对话区不到 560px 宽时不画竖轨，⌥↑ / ⌥↓ 照样能用。应用可以用 `ChatPanel` 的 `features.turnRail = false` 关掉它。

### 4.3 错误呈现

常见的失败用人话说：发生了什么、下一步做什么；引擎或宿主的原话与错误码留在悬停提示里（对话里的一行在 `title`，通知里跟在括号里），
给支持看。认不得的失败照原话。分类在 `@harnix/core/state` 的 `failureClass`：引擎在一轮的错误里给 `data.kind`
（`auth`、`rate_limited`……），宿主自己的码（实例不可达、扩展不可用、超时）另认；话在 `chat.fail.*`（中英两份）。

| 情形 | 归类 | 在哪里显示 | 话里给的下一步 |
|---|---|---|---|
| 没有模型服务或密钥，发消息（引擎不发请求，`data.kind` 是 `no_credentials`） | `no_model` | 对话里的一行，带「打开模型设置」 | 到 设置 · 模型与凭据 添加 |
| 服务拒绝密钥（401、403）、额度不足、模型名不对 | `auth`、`billing`、`model_rejected` | 同上 | 检查密钥、额度或模型名；带「打开模型设置」 |
| 限流、服务过载、连不上或超时 | `rate_limited`、`overloaded`、`model_unavailable` | 对话里的一行 | 等一会儿再发（服务给了等多久就说几秒）；限流时服务自己的说明（哪个额度、几点恢复）摘要跟在后面（`serviceSaid`：压成一行，至多 160 字）；连不上时检查网络与代理 |
| 对话超出上下文 | `context_too_long` | 对话里的一行 | 压缩对话或新开会话 |
| 引擎的连接中途断了、实例不可达 | `engine_down`、`instance_unavailable` | 对话里的一行；发送被拒时在通知里 | 稍等再发；一直不恢复看 设置 · 诊断 |
| 扩展包的后端陷阱或没有启动 | 扩展不可用（`extension_unavailable`） | 设置 · 扩展包 里该包的状态行（带原因）；工具的失败在工具行；界面面板在它自己的失败页 | 看原因与扩展日志，修好后停用再启用；面板上有「重新加载」「再试一次」 |
| 流程的站的门禁没过 | 站的状态「未满足」 | 流程的站面板的「尚未满足」，每项的原因逐条列出 | 退回重做，或由人写明理由放行；命令门禁的完整输出在运行目录的 `gate-*.log` |

对话里认得的失败带一个入口时，入口走宿主的 `settings.open("models")`；没有宿主的对话（`@harnix/react` 单独用）只说话、不画入口。
通知与横幅里的失败经 `explainError` 说（同一套分类，码跟在括号里）。LIVE 用例：`tests/live/api/failures.spec.ts`
（每种失败到达时的形状，模型服务是 `stub-provider.ts` 里故意出错的桩）、`tests/live/ui/errors.spec.ts`（界面上的话与入口）。

## 5. 会话菜单

右键一行、`⋯`，或会话标题。它是在那里打开的菜单，不是对话框：右键时在指针处，`⋯` 与标题在按钮下面。焦点落在第一项，
↑ / ↓ 换项，回车选，Esc 或 Tab 关掉，焦点回到打开它的地方；点菜单外面也关。只有要再问一次的项（删除）或要填东西的项
（重命名、导出）才开对话框。项目菜单（§3）一样。

| 项 | 方法 | 说明 |
|---|---|---|
| 打开 | — | |
| 重命名 | `harnix.session.rename` | Harnix 账本里的标题，盖在引擎给的名字上 |
| 归档 / 取消归档 | `harnix-app.sessions.archive` | 只影响左栏列不列它 |
| 从这里分叉 | `harnix.session.fork` | |
| 导出 | `harnix-app.sessions.export` | Markdown，或引擎打的数据包（zip，可带录像）；都带上对回答的反馈（§2） |
| 定时… | `schedule.*`（`@harnix/schedule`） | 这个会话的定时（发给它的、复用它的新会话目标、模型在它里面建的），每条可立即执行、暂停、编辑、删除、看投递记录；「新建定时」是往它发一轮（§9.1）。别的扩展建、只是展示到这里的会话（任务的主会话）不出这一项 |
| 导入… | `harnix-app.sessions.importUpload` → 上传 → `harnix-app.sessions.import` | 选一个导出的数据包（`.zip`），经大负载旁路上传，装成当前空间里的新会话（连同它的子代理会话）；不是选中某个会话的操作，在会话列表的「+」菜单里。包里的项目在这台机器上不存在时，装成无项目会话并提示 |
| 关闭 | `harnix.session.close` | 放掉运行时，历史留着，还能重开 |
| 删除 | `harnix.session.delete` | 先试删一次看连带几条侧链会话，确认后删；连同录像 |

两个 space 里的菜单一样：项目 space 列的是会话 space 的会话，所以菜单的调用都声明会话 space（`app.claim()`），不是前台的项目 space。

**层与提示**。外壳画在页面上面的东西是一个栈：对话框（对话框里再开的对话框叠在上面）、菜单、外壳替扩展包问的确认（总在最上面）。

- **焦点**：对话框打开时焦点移进去，Tab 只在里面转，关上后回到打开它的地方；它下面的页面与对话框是 inert 的。
- **Esc 只关最上面一层**：先关菜单，再关最上面的对话框；确认开着时 Esc 答「否」，页面别处一个键都收不到。
- **输入框里的 Esc**：先关弹层（`/`、`@`、＋、模型）；一轮在跑时，第一次 Esc 只提示「再按一次中断」，几秒内再按一次才中断。
- **横幅的提示排队**：一次显示一条，后面还有几条会写出来；普通的 6 秒后自己走，错误 12 秒，要重新加载页面这类状态一直在。
  横幅是播报区（错误 `role=alert`，其余 `role=status`）；对话里来了权限询问，也单独播报一句「需要批准：工具名」。

### 5.1 消息菜单与回退

人的一条话（开启了一轮的那条，不是运行中送进去的）下面有 `⋯`，悬停或聚焦时出现，是**消息菜单**：外壳问 `session.action` 里 `at` 有
`message` 的（`target` 是 `{kind: "message", session, space, turnId, text}`）。外壳自己往里放一项「**回到这里**」；文件工具
（`Write`、`Edit`、`MultiEdit`、`NotebookEdit`）的工具行下面也有一项「**把文件退回到这次调用之前**」。`@harnix/observability` 再放两项改这条话本身的
（下面的「改写与去掉」）。space 把对话设成只读时没有消息菜单。

**改写与去掉**（`observability.message.edit`、`observability.message.drop`）。不回退、不撤掉哪一轮，只改这一条，模型从下一次调用起看到改过的，
它已经答过的不变：

| 项 | 做什么 | 方法 |
|---|---|---|
| 改写这条消息 | 对话框里改这条话的字（字没变时「改写」按不下去），确认后原地换掉 | `observability.messages.edit {instance?, session_id, space?, at, text, total}` → `{edited}`（引擎的 `editMessage`：只改人自己写的、只有文字的消息） |
| 从上下文中去掉 | 先问一次，确认后这条话在此后的上下文里换成 `[removed by the host]`，位置留着、对话里照样有它 | `observability.messages.drop {instance?, session_id, space?, at: [..], total}` → `{dropped}`（引擎的 `dropMessages`：人的话或工具结果，模型的消息不能去掉） |

- **哪一条**：`harnix.session.outline` 里这一轮（`turnId`）的 `seq` 是开启它的那条人的话在历史里的位置，`total` 是读的时候历史有几条。两样都带上：
  `total` 必须给，引擎看历史的条数对不上（期间有新的、压缩过）就拒绝（`INVALID_PARAMS`），改的只会是人看到的那一条。
- **什么时候不行**：这个会话有一轮在跑或在排队时两项是灰的，悬停说为什么；引擎此时也答 `SESSION_BUSY`。
- **之后**：引擎就地改了消息、不推帧，所以中枢在 `editMessage`、`dropMessages` 之后照回退那样清掉这个会话的追赶缓冲、向每个观看者推
  `harnix.session_resync`：各台设备重读历史，画出来的就是引擎现在的。
- **谁能调**：设备，`RowScope::Session`，同 §6.1 的两个读：`observability` 先以调用方声称的 space 问 `engine.session.get`（看不到的会话由中枢拒绝，
  `SPACE_MISMATCH`），再凭被授予的 `engine.control` 调引擎，只转它点名的键，两次判定都进审计。产品不授予 `engine.control` 时这两个方法与 §6.1 的停止被拒，别的照旧。

两项都走 `harnix.session.rewind`，也都先问一次 `dry_run`，弹一个对话框写清楚：

| 列出的 | 来自 |
|---|---|
| 要撤掉的轮（每轮开头那句话） | `conversation.removed_turns` |
| 要恢复的文件；那时还没有的，标「删除」；项目外但当时批准过的，标出来 | `files.restored`（`to: "file" \| "absent"`） |
| 之后在会话之外改过的文件（手改、`Bash` 改、别的程序改） | `files.skipped` 里 `overridable_by: "force"` 的（`issue: "modified"`） |
| 回不去的文件（写前内容没留下、写路径策略拒绝、是符号链接、worktree 已删） | `files.skipped` 里其余的 |

有冲突时要勾「覆盖这些改动」（`force`），有回不去的要勾「跳过回不去的文件」（`allow_partial`），否则「回退」按不下去；「先分叉一份」（`fork_first`）
把当前全貌先分叉出去，撤掉的轮留在分叉里，分叉落在同一个 space。对话框还写一句：**只退回文件工具写的，命令做的（`Bash` 里的重定向、`git`、
包安装）不退回**——这是引擎的规则（daemon 协议 §6.3）。确认后执行；撤掉的第一轮（就是点的那句话）放回输入框，横幅说撤了几轮、恢复了几个文件。
一轮在跑或在排队时这两项是灰的，悬停说为什么。

| 方法 | 参数 | 答 |
|---|---|---|
| `harnix.session.rewind` | `{instance?, session_id, space?, to, scope?, dry_run?, fork_first?, force?, allow_partial?}`。`to` 是 `"start"`（第一轮之前）、`{turn}`（这一轮刚结束时）、`{before_turn}`（这一轮开始前：前一轮结束时，第一轮就是 `start`；「回到这里」用它）或 `{tool_use}`（这次调用写任何东西之前，只回文件）。`scope` 是 `files`、`conversation`、`both`，缺省 `both`，`tool_use` 缺省 `files` | `RewindResult`（`harnix_contract::wire`）：`{instance, session_id, rewind_id, dry_run, scope, to, conversation?: {messages_before, messages_after, removed_turns: [{turn_id, prompt}]}, files?: {restored: [{path, to, outside_project?, unverified?}], skipped: [{path, issue, detail, overridable_by}], overwrote_external}, fork_session_id?}`；`to` 是解析过的（`before_turn` 换成它落到的那一轮或 `start`） |

**谁能调**：设备，只对它声称的 space 的会话（不声称时是缺省 space），不能跨 space——声称 `cross_space()` 的 space 也不行，答 `SPACE_MISMATCH`，同
`harnix.session.setModel`；扩展要 `engine.control`，扩展包只到自己的 space。**什么时候不行**：这个会话有一轮在跑、在排队，或另一次回退没完，中枢答
`SESSION_BUSY`（-32015），不排队；后台任务在跑时引擎也这样答。有文件不能照要求恢复时引擎答 `REWIND_PRECONDITION`（-32023，`data.issues`
同 `skipped` 的形状，什么都没改）；做到一半失败答 `REWIND_FAILED`（-32024，已恢复的退回原样，除非 `data.partial`）。会话不记 checkpoint
（引擎设置 `checkpoints.enabled = false`）却要回文件，答 `INVALID_PARAMS`。

**之后**。引擎的日志只追加：撤掉的轮在磁盘上还在、标了作废，`engine.session.history` 不再有它们（`total` 变小，`seq` 从切点接着编），
导出的数据包（`engine.session.export`）里的 `transcript.jsonl` 有原样的行。中枢清掉这个会话的追赶缓冲（切点之后画过的帧），向每个观看者推 `harnix.session_resync`——
各台设备照刷新那条路重新 attach、重读历史，撤掉的轮在哪儿都消失；引擎推的 `rewound` 帧照常作为 `harnix.event` 送到，不进缓冲。
回退不是经中枢做的（直接连引擎的客户端），中枢收到 `rewound` 帧也一样处理。**改动卡**：撤掉的轮的 `harnix.changes` / `turn` 条目在日志里
也还在，所以中枢接着记一条 `harnix.changes` / `rewind`（`{rewind_id, turn_ids}`）；`engine.session.entries` 经过中枢时，**某条 `rewind` 之前、
它列出的那几轮的 `turn` 条目不列**，改动卡、改动页签跟着消失，`rewind` 条目本身照列、界面不画。别的命名空间的条目（扩展自己的）不受影响：
它们的 `at` 落到切点，扩展要自己看 `rewound` 帧或这条 `rewind` 条目决定怎么办。

## 6. 右栏：文件、预览、改动

右栏是每个会话一个停靠面（[Harnix 扩展](harnix-ext.md) §22.7）。`@harnix/observability` 在里面放三种页签：页签栏末尾常驻的**上下文**、
引导页上的**用量**（本会话这次运行的轮数、调用、token、缓存命中、用时，带「打开轨迹」）和后台任务的**输出**（§6.1）。
`@harnix/harnix-app` 往里注册三种页签类型，都只读、都限在**这个会话的项目根**
（引擎记的 `project_root`）里：

| 页签 | `sidebar.tab` | 打开它 | 内容 |
|---|---|---|---|
| 文件 | `kind: files`，`builtin`，引导页入口，`keepMounted`（树的展开在切页签、切会话后还在） | 引导页、`openTab("files")` | 项目的目录树，一层一层按需读。`.gitignore` 忽略的和 `.git`、`node_modules`、`target` 不列，只数个数；「显示被忽略的（N）」切换。点文件用预览打开它 |
| 预览 | 资源类，`patterns: ["harnix-resource://file/**"]`，`fallback` | `openResource(harnix-resource://file/<相对项目根的路径>, {params})` | 文本与代码（带行号，不高亮），Markdown 用对话里的同一个渲染器（「源码 / 渲染」切换），图片。二进制、超过 1 MiB 的文本或 5 MiB 的图片、Office 与 PDF：说一句为什么不预览。`params.line` 标出那一行并滚过去；`params.text` 标出这段文字所在的第一行；每次导航（`revision` 变）都重新落到那一行，落到行上时 Markdown 显示源码 |
| 改动 | `kind: changes`，`builtin`，引导页入口 | 改动卡、引导页、`openTab("changes", {params: {turn}})` | 一轮改了哪些文件：每个文件的 `+增 −删`、新建的标「新建」、逐个展开 unified diff（和工具行的 diff 一个样子，行号是文件里的行号），「打开文件」用预览打开。给了 `turn` 只看那一轮，「看全部轮次」看本会话的每一轮（新的在前）；一轮结束后自己刷新 |

扩展可以注册同一个 `kind` 的 `extension` 优先级的类型顶替文件或改动页，注册更具体的 `patterns` 认领某一类文件的预览（比如 `*.csv`）。

**文件的读法**（`harnix-app.files.*`，设备调用，`RowScope::Session`：像别的会话调用一样声明会话所在的 space，由 `engine.session.get` 查这台设备看不看得到这个会话）：

| 方法 | 参数 | 答 |
|---|---|---|
| `harnix-app.files.list` | `{instance?, session_id, space?, path?, all?}`：`path` 相对项目根，缺省是根；`all: true` 连忽略的一起列 | `{root, path, entries: [{name, path, dir, link?, size?}], hidden, truncated}`：目录在前；`hidden` 是没列的个数；一层至多 2000 项 |
| `harnix-app.files.read` | `{instance?, session_id, space?, path}` | `{path, size, kind}`：`kind` 是 `text`（带 `text`）、`image`（带 `mime`、base64 的 `data`）、`binary` 或 `too_large` |

路径只能是相对的、不往上走：`..`、绝对路径一律 `REFUSED`，不管它存不存在；然后顺着符号链接解析，落到项目根外面的同样 `REFUSED`——项目里一个指向 `/etc` 的链接不是出口。
会话没有项目时答 `INVALID_PARAMS`（文件页说「这个会话没有项目」），不存在的文件答 `NOT_FOUND`。

**输入框 `@` 的文件补全**（`harnix-app.files`，设备调用，`RowScope::Session`）走同一道围栏：

| 参数 | 走哪个目录 |
|---|---|
| `{instance?, session_id, space?, query?, limit?}`（输入框用这个） | 这个会话的项目根，像上表一样声明 space、由 `engine.session.get` 查；同时给了 `project_root` 时不看它 |
| `{project_root, query?, limit?}` | 只在它**是这里的项目**时：登记过的项目（工作区），或某个会话跑在里面的目录；按解析过符号链接的真实目录比，和项目信任（§3.1）认的是同一批。别的目录一律 `REFUSED`，`..`、指向别处的链接也是。隔离 iframe 里的页面不能这样给，只能给会话 |
| 两个都不给 | `INVALID_PARAMS`：不再退到项目目录 |

答 `{project_root, files: [{path, size}]}`：`project_root` 是解析过的真实目录，`path` 相对它；不跟符号链接走，`.gitignore` 忽略的和隐藏文件不列，至多 30 条。

**目录选择器**（`harnix-app.listDirectory`，§3）先把 `path` 顺着 `..` 和符号链接解析成真实目录，再看在不在 `$HOME` 或项目根之内；解析不了的（不存在、读不了）答 `INVALID_PARAMS`，不按写法比。

**改动卡与改动的来历**。引擎只在 `Write`、`Edit`、`NotebookEdit` 的工具结果旁边报一次写了哪些文件（daemon 协议 §7.5 的 `file_changes`），
transcript 里没有 diff。所以**一轮结束时，中枢把这一轮写过的文件（子 Agent 写的也算）记成会话日志里的一条条目**，命名空间 `harnix.changes`，
事件 `turn`——和审计用的是同一条路，引擎不解析它，不认识它的界面跳过它；刷新、重开会话、分叉之后都还在：

```jsonc
{ "turn_id": "t_…", "added": 12, "removed": 3, "more": 0,      // more：超出 200 个、没列的文件数
  "files": [ { "path": "/abs/src/lib.rs", "op": "modify",       // create | modify；同一个文件写几次合成一行
               "added": 10, "removed": 3, "bytes": 1851,
               "diff": "--- a/src/lib.rs
+++ b/src/lib.rs
@@ …",  // 几次写就几段，按次序
               "truncated": false },
             { "path": "/abs/logo.png", "op": "create", "added": 0, "removed": 0, "omitted": "binary" } ] }  // too_large | binary | redacted
```

一条条目里的 diff 合计至多 512 KiB，超出的文件只留行数、写 `omitted: "too_large"`：打开会话时要整个读回日志，一轮改了一大片不能让以后每次打开都付这个代价。
`Bash` 改了哪些文件引擎不知道，不报，这里也就没有。界面里只有 `@harnix/core` 的 `parseTurnChanges` 认识这个形状。
回退（§5.1）撤掉的轮，它们的这条条目由中枢在列条目时去掉。
对话里，这条条目画成一张卡「改了 N 个文件 +增 −删」，后面是前几个文件名；点它打开改动页签到这一轮。

**工具行里的文件**：`Read`、`Write`、`Edit`、`NotebookEdit` 这类参数里有 `file_path`（或 `notebook_path`）、而且在项目里的调用，工具行上的路径是链接，
点了用预览打开那个文件——`Read` 跳到它的 `offset`，`Edit` 跳到它写进去的文字所在的那一行；展开工具行，下面也有「在预览中打开」。
这是一个 `session.action`（`harnix-app.open-file`，`at: ["arg", "tool"]`），外壳不认识任何工具。

### 6.1 上下文与输出：`@harnix/observability`

产品链了 `observability` 才有（`harnix` 二进制链了）。两页读的都是会话的**内部**，设备自己够不着引擎的这些方法（`engine.session.inspectContext`、
`taskOutput` 要 `engine.inspect`，`stopTask` 要 `engine.control`），所以经 `observability` 的方法：设备调它，它以自己的身份、凭被授予的能力调引擎，两次判定都进审计，后者挂在前者下面。
方法都是设备的、按 `RowScope::Session` 判：只对设备声称的 space 的会话（像别的会话调用一样，`engine.session.get` 查这台设备看不看得到它），`instance` 不写是 `main`。
`engine.control` 是 `observability` 的可选能力（`harnix::first_party::standard` 授予）：不授予时停止任务与 §5.1 的改写、去掉被拒，读照旧。

| 方法 | 参数 | 答 |
|---|---|---|
| `observability.context` | `{instance?, session_id, space?}` | 引擎的 `inspectContext`：下一次调用要带的系统块（来源、正文、估算的 token）、工具表、消息，压缩阈值；另带 `measured`，上一次调用 provider 实测的数 |
| `observability.tasks.output` | `{instance?, session_id, space?, task_id, tail_bytes?, from_byte?}` | 引擎的 `taskOutput`：一个后台任务的输出，整份、末尾或从某个字节起；`{output, from_byte, next_byte, dropped, final}` |
| `observability.tasks.stop` | `{instance?, session_id, space?, task_id}` | 引擎的 `stopTask`：Shell 命令连同它的进程组结束，子 Agent 在下一个检查点停下；`{stopped}`，没有这个任务或它已经结束是 `false` |

**上下文**（`sidebar.tab`，`kind: "context"`，`pinned`）：每个会话的页签栏最后一个，常驻、不能关，不进引导页（`pinned` 见 [Harnix 扩展](harnix-ext.md) §22.7）。
下一次调用**将要**带的东西，每一部分各读各的、各自失败——产品没有某个方法时只少那一部分，页签照在：

| 部分 | 从哪来 |
|---|---|
| 系统、工具、消息的比例，实测（上一次调用）与估算（下一次），压缩阈值；每个系统块的来源与正文，工具表（`@harnix/react` 的 `ContextReport`） | `observability.context` |
| 生效的技能与钩子；技能标来源层，引擎用户层的分开标「全局」「场景」（`user_tier`），悬停是说明和读它的目录 | `harnix.session.skills`、`harnix.session.hooks` |
| 每个 MCP 服务器的工具与指令 | `settings.mcp.context` |
| 被项目信任扣下的项，带「信任这个项目」（§3.1） | `harnix-app.sessions.projectStatus` |

这个会话一轮结束后、项目的信任变了（主题 `harnix-app.trust`）时，各部分重读。

**输出**（`sidebar.tab`，`kind: "output"`，`multiple` 且 `keepMounted`，地址 `harnix-resource://output/<实例>/<会话>/<任务>`）：一个后台 Shell 或监视器一个页签，
从对话里 `Bash`、`Monitor` 工具行的「查看输出」打开（`session.action` 的 `observability.output`，`at: ["tool"]`）。只读：先读末尾（`observability.tasks.output`），
再跟着会话推来的 `task_progress` 往下接，中间有缺口就补读；自动跟到末尾，往回翻就停，回到底又跟；页面至多留 256 KiB（引擎留的也是最后 256 KiB）。
任务本身来自会话的后台任务列表（`harnix.session.tasks`）。模型建的定时没有输出。
任务还在跑（`running`、`waiting`、`pending`）时，页签头上有「停止」：按一次只是变成「再按一次停止」，3 秒内再按才停（`observability.tasks.stop`），
同中断一轮的两次 Esc，免得误点；停下的状态从任务列表回来，「停止」随之消失。已经结束的说一句「这个任务已经结束了」，被拒的照外壳的话说。

## 7. profile

### 7.1 `[shell]` 与 `[extensions]`

```toml
[shell]
model_switch = true       # 加 harnix.session.setModel，能在输入框换模型（只限前台 space 的会话）；false 不加。不写是产品的缺省（AppBuilder::model_switch）：harnix 二进制开，库的缺省关
new_session = true        # false：整个外壳不画「新会话」、⌘K 不归外壳；只是界面
narrow_details = "over"   # ≤ 960 宽时打开的右栏：over 是盖住中区的抽屉；beside 三栏并排（< 720 仍是抽屉）

[extensions]
off = ["flow"]            # 链进来了但关掉的扩展；不写是都开
default_space = "sessions"  # 不声称 space 的调用按它检查，换掉扩展声明的缺省；必须是已启用扩展声明的 space
```

两节都拒绝不认识的键；0.4.2 的 `[shell] spaces` 已删，写了启动就拒绝。哪些内建 space 在，现在是哪些扩展开着：

| `[extensions] off` 里的名字 | 去掉之后 |
|---|---|
| `harnix-app` | 首页、会话 space 与项目 space 的后端与界面都没有；后端的 `sessions`、`home` 两个 space 也不再声明（见 §1：要另指缺省 space） |
| `flow` | 流程 space 的后端与界面都没有（§11），项目里没有任务，首页没有最近任务 |
| `observability` | 右栏没有上下文、用量、输出，没有轨迹视图，消息菜单里没有改写与去掉，首页没有全局统计与用量，诊断里没有扩展日志 |

一个 space 都没有时，外壳说「没有注册任何 space」。

### 7.2 `[brand]` 与目录

```toml
[brand]
name = "Harnix"           # 页面标题、左栏产品名；不写是 Harnix
icon = "brand/icon.svg"   # SVG，相对 profile；服务在 /brand/icon.svg
product_id = "harnix"     # 程序自己的目录 ~/.<product_id>；小写字母、数字、-，字母开头；不写是 harnix
```

Harnix app 的项目在项目根 `projects_dir`（`--data-dir`，缺省 `$HARNIX_DATA_DIR`，再缺省 `~/Documents`）里，工作区登记、归档、偏好在 `~/.<product_id>/ext/harnix-app/data/state.json`，会话标题在引擎里，
对话在 `main` 的引擎目录（`~/.<product_id>/engine/`）里。

### 7.3 `harnix.hello` 里的 `shell`

外壳不读 profile，读握手之后 `harnix.hello` 的 `shell`：`brand {name, icon, product_id}`、`runtime_modules`、`new_session`、
`model_switch`、`narrow_details`、`state_dir`、`projects_dir`、`user`。哪些 space 在，外壳看 `hello.extensions` 里启用的扩展
（首页、会话与项目是 `harnix-app` 的，流程 space 是 `flow` 的）。`harnix-app.describe` 也答 `projects_dir`。

### 7.4 设置页：通用、显示、隐私与高级

设置页的字段分两处存放，同一种写法（`harnix-ext.md` §22 的字段声明）：引擎的设置在后端 `settings` 扩展的字段表里，
这个浏览器自己的偏好在外壳的 `settings.tsx`，存在 `localStorage`，不进引擎。

**通用与显示**（外壳的「通用」节，浏览器本地，立即生效）：

| 偏好 | 说明 |
|---|---|
| 主题、语言 | 语言选定时同时写进引擎全局层的 `language`（`auto` 则交还），引擎按它作答；写的是 `@harnix/settings` 订阅语言选择做的，外壳不认识它 |
| 字号、正文字体、代码字体 | 经 token `--font-size-base`、`--font-sans`、`--font-code` 作用于整个文档；缺省值时不在文档上留内联样式 |
| 工具详情缺省展开 | 编辑类工具的 diff 本来缺省展开，这一项管其余工具 |
| 折叠思考 | 想完的内容首次画出时是否折叠 |
| 用量显示 | 不显示 / 只显示总数 / 总数加缓存命中，管会话抬头的用量标签 |
| 链接打开方式 | 新标签页（缺省，链接自己的行为）/ 当前标签页 / 每次询问；只管消息正文里的链接 |
| 运行中按回车 | 排队（缺省）/ 插话（steer）。只在支持插话的 space 里有意义；会话里输入框上的开关仍可单独改，改了就以它为准 |

这些偏好由 `@harnix/react` 的 `display` 模块保存与分发（`localStorage` 键 `harnix.display`），聊天组件读它，下次画时生效。
字体列表进样式声明，含 `;{}<>\` 的值不接受。

**设置文件**：「引擎」「隐私」「高级」节列出各层 `settings.json` 的路径并带「复制路径」。没有新增在系统里定位文件的特权接口，
由人用自己的编辑器或文件管理器打开。

**隐私**：遥测是引擎对自己做了什么的记录，每个会话一个 JSONL，写在引擎数据目录的 `telemetry/` 下，不含提示词、消息文本和工具的输入输出
（`agix/docs/telemetry.md`）。字段 `telemetry_enabled` 与 `telemetry_url`（OTLP 端点，只有带导出支持的引擎才用）只写全局层，项目没有发言权。
引擎自己的缺省是开；**Harnix 的缺省是关**：实例建起来（非 attach 的承载方式，主实例 `main` 也在内）时，`harnix-instance` 在该实例全局层的 `settings.json`
里缺 `telemetry_enabled` 就写 `false`（`crates/instance/src/defaults.rs`），已有值（开或关）不动，其余内容不动。
人在设置页里「清除」这一项后，下次启动会再写一次 `false`。

**高级**：压缩（`compaction`）、执行参数（`execution`）、hooks（`hooks_config`）、团队（`team`）、检查点（`checkpoints`）的当前值，
按引擎实际使用的值显示，并说哪一层写了它；没有任何一层写的，说是引擎缺省。这些字段声明为 `json` 类型、没有可写的层，
`settings.write` 在任何层都拒绝；要改就改文件。hooks 只显示每个事件有几条，不显示每条跑什么（hook 里可能带凭据）。
Agent 循环内部的参数不做表单。

### 7.5 设置页：MCP 服务器

「MCP」一节（`@harnix/settings`，后端 `settings` 扩展的 `settings.mcp.*`，[Harnix 扩展](harnix-ext.md) §22.9）列出这一对 (项目, 场景)
每一层的服务器，连停用的。每张卡片：名字、连接方式、哪一层；状态（已连接、已登录、需要登录、登录失败、连接中、已断开、失败、被策略拒绝、
项目未受信、已停用、第一个会话时连接、未报告）和引擎给的原因；命令与参数或 URL；工具数。另一层有同名的，低的那条说「未生效」；要 `private_key_jwt` 的说这个构建没有它。

| 操作 | 说明 |
|---|---|
| 添加服务器 | 选层（全局、场景、当前项目）、名字、连接方式（stdio：命令、每行一个参数、环境变量；http / sse：URL、请求头）。环境变量和请求头的每一项是「值」（密码框）或「取自环境变量」；项目层只有「值」，它读不到引擎的环境 |
| 编辑 | 名字和层不改；已写的值只显示末 4 位，缺省「保留」，改成「值」就换掉 |
| 停用 / 启用 | 在这一层写它的开关 `mcp_enabled.<名字>`（`engine.mcp.setEnabled`）：停用的留在文件里，引擎不启动它，状态是「已停用」；未受信的项目只能停用 |
| 重连 | 引擎按这一对现在的设置重连这一个服务器，别的不动（`engine.mcp.reconnect`）；这一对还没连过时不能重连 |
| 移除 | 先问一次，确认了才删，连同它的值 |
| 登录 / 退出登录 | 远端（http、sse）的服务器有。登录开一个新窗口去授权页，回来落在 Harnix 自己的路由上，完成后窗口关掉、卡片刷新；页面不在 `https` 或本机地址上时，登录要在引擎那台机器的浏览器里完成 |

每一层都经引擎写（`engine.mcp.addServer / updateServer / removeServer / setEnabled {tier}`），扩展自己不写文件：引擎在那一层目录的锁下写，值进那一层的
`settings.local.json`（0600，不提交），其余进 `settings.json`，写完只对这一个服务器对账。项目层的服务器在项目没受信时引擎拒绝，什么都不写（先在项目菜单里信任它）。
状态是引擎对这一对 (项目, 场景) 说的（`engine.mcp.status {project_root, scene}`）；这一对还没有会话连过时，引擎不为列表启动它们，状态是「第一个会话时连接」。

## 8. 它是怎么用框架做的

Harnix app 是原生扩展最小的例子：一个后端扩展、一个界面包，用的都是二开产品也用的接口。

| 部分 | 用了什么 | 二开产品对应的做法 |
|---|---|---|
| 后端 | 扩展 `harnix-app`：`workspaces.list/create/update/reorder/remove`、`sessions.archive`、`sessions.export`、`sessions.importUpload`、`sessions.import`、`prefs.set`、`search`、`files`、`files.list`、`files.read`、`listDirectory`、`recentProjects`、`projects.create`、`projects.status`、`projects.trust`、`sessions.projectStatus`、`describe`，主题 `trust`；`Manifest::requires([transcripts.read, engine.export])`、`.optional([engine.control])` | 一个 `Extension` 的 `register`、`Manifest::requires`（[Harnix 扩展](harnix-ext.md)） |
| 行的开放范围 | 全部 `Access::device()`：人经界面做的决定只对设备开；`sessions.*`、`files.list`、`files.read` 另按 `RowScope::Session` | 同一套 `Access`，扩展也要调的行写 `.with_extensions()` |
| 后端 space | `sessions`，`default_space()`；`home`，`no_sessions()` | `register` 里的 `SpaceDecl` |
| 界面 space | 三个 `space` 注册，owner 是扩展 id `harnix-app`；项目 space 写 `sessions: "sessions"`；首页写 `landing: true`、`center: "hidden"` | `r.register('space', …)`（[Harnix 扩展](harnix-ext.md)的前端一节） |
| 分组 | `sidebar.group` 的 `harnix-app.time` | 注册自己的 `sidebar.group` 换掉它 |
| 自己定义的点 | `harnix-app.project.items`、`harnix-app.project.panels`、`harnix-app.session.badge`（§3.2），`harnix-app.inbox`（§3.3），`harnix-app.home.panels`（§10），`definePoint`；后端 `r.ui().point(…)` 声明 | 自己的 space 要让别的包往里放东西，就这样定义一个点 |
| 右栏 | `sidebar.tab` 的 `files`、`preview`、`changes`，`flow.block` 的改动卡，`session.action` 的 `harnix-app.open-file`（`at: ["arg", "tool"]`） | 注册同一个 `kind` 的 extension 类型顶替，或更具体的 `patterns` |
| 数据 | 方法 `harnix.sessions.list`，主题 `harnix.sessions.status` | `useHost()` 的 `call` 与 `subscribe` |

`harnix-app` 只用公开的扩展接口，由 `crates/harnix/tests/layering.rs` 盯着（第一方扩展只依赖 `harnix-ext`）。界面一侧，`@harnix/harnix-app`
只从 `@harnix/react` 与 `@harnix/core` 的公开导出取东西，经 `HostApi` 调后端；两个左栏的组件在这个包里，不在外壳里。

**要一个不同的 Harnix app**：不 fork 外壳，在自己的产品里把 `harnix-app` 写进 `[extensions] off`，用自己的扩展声明 space、用自己的 UI 包
注册 space 顶替（[Harnix 扩展](harnix-ext.md)）。0.4.2 里后端随框架链进来、只能藏不能去；现在它是一个独立扩展，关掉就是整个不在，
`harnix-app.*` 的方法也不再登记。

## 9. 定时

到点替人做一件事：往一个会话里发一轮、在项目里新建一个会话发一轮、或在项目里起一个任务（一次流程运行）。
**由 Harnix 调度**，是第一方扩展 `schedule`（`crates/ext/schedule`，`harnix` 二进制带它，`harnix` 门面的 feature `schedule`）：
引擎自己的定时只能发给已有会话、按 UTC 算、开不了新会话（`agix/docs/daemon_rpc_protocol.md` §6.7）。
**Harnix 没在跑时不投递**：一个定时器在进程里，进程起来时按每条的「错过」策略处理停着的那段（见下）。

| 项 | 怎么做 |
|---|---|
| 时间（`timing.kind`） | `once {at}`（RFC 3339 带偏移，或时区里的墙钟 `YYYY-MM-DDTHH:MM[:SS]`）· `after {seconds}`（建好之后多久，一次）· `every {minutes}`（每 N 分钟，≥ 1，按经过的时间算）· `daily {time: "HH:MM"}` · `weekly {weekdays: [1..7], time}`（周一 1 … 周日 7）· `cron {expr}`（5 段，照 Vixie：`*`、数字、范围、列表、步长；不认名字和宏；日、周两段都写了时任一满足即可）。每月、工作日由界面拼成 `cron`、`weekly` |
| 时区 | `time_zone` 是 IANA 名字，缺省是本机的；墙钟时间都按它算。夏令时跳过的那段钟点当天不到期，重复的那段只算第一次；`once` 落在跳过的那段里，顺延到跳过之后 |
| 目标（`target.kind`） | `session {instance, session_id, prompt}`：已有会话发一轮，会话忙就排队（`OnBusy::Queue`）；上一次投的还在队列里时这一次并进去（回执 `skipped`、`merged`）。`new_session {mode, project_root?, scene?, instance?, provider?, model?, prompt}`：新会话，`new_once_then_reuse`（缺省，第一次新建、之后都发给它）或 `new_each`（每次新建），标题是「定时标题 · 计划时间」（时区里的 `YYYY-MM-DD HH:MM`）。`task {flow, subject, inputs?, project_root, title?, output_dir?, workspace?, show_in_space?}`：每次 `flow.start` 一个运行，`trigger` 是 `{kind: schedule, schedule, fire_at}`；`show_in_space` 原样交给 `flow.start`（它照认这个旧名，即运行的 `shown_in`；建定时的界面传它列项目会话的 space，任务就在项目 space 里打开） |
| 错过（`missed`） | 晚了一分钟以上的算错过（Harnix 没在跑、机器睡着）：`catch_up_latest`（缺省）只补最近一次，`skip` 一次也不补、记一条 `skipped`、`missed` |
| 并发（`concurrency`） | 只对任务：上一次的运行还在跑时 `skip`（缺省，记 `previous_run`）或 `parallel`（再起一个，配合 `workspace: worktree`）。会话目标按会话排队 |
| 最小间隔 | 同一条定时两次投递至少隔 60 秒，「立即执行」也算；不到就记 `skipped`、`min_interval` |
| 回执（`Receipt`） | `scheduled_at`（计划时间；立即执行是按下的时间）、`delivered_at`、`manual`、`outcome`（`pending` · `delivered` · `skipped` · `failed`）、`reason`（跳过的四种理由，或失败的错误）、`target`（`{instance, session_id, created, item?}` 或 `{run}`）。只说明交出去了，不说明模型做得怎样。**失败不重试**（只有上面「幂等」说的那一种重投）。每条留最近 200 条、30 天以内的，最近一条总留着 |
| 幂等 | 投递一条一条来。先写下 `pending` 回执。新会话的 id 在建之前就定下：由定时的 id 和计划时间推出（v5 UUID，写成引擎的 BASE58），先写进回执，再带着它 `harnix.session.create`（`session_id`，引擎对已有的 id 答那个会话）；建好后 `new_once_then_reuse` 还写进 `target.session`，再起名、发提示词。启动时还是 `pending` 的回执：正在开新会话的那条**重投一次**——同一个 id 建出来的就是上次那个会话，不会多出一个；它已经收到过东西（有消息或有一轮在跑）就不再发提示词，记 `delivered`。别的目标（往已有会话发、起任务）改成 `failed`（`interrupted…`），不重投，因为不知道那一轮、那个运行发出去没有 |
| 存储 | 扩展数据目录下的 `schedules.json`：整份写到临时文件再改名，带版本号；读不了的放到一边（`.json.unread`），从空开始，不迁移旧格式 |
| 归属与授权 | 一条定时属于一个 space（`space`，建时声称的，缺省是缺省 space）；带 `space` 的调用只够得着那个 space 的定时（不是的答 `SPACE_MISMATCH`），不带的够得着全部（首页的「定时」一块这样调，§10.2）。会话目标建时核对会话属于这个 space。设备不要能力：人管自己的定时，不审批；别的扩展要被授予 `schedule.manage`（定时扩展声明的） |
| 改 | `update` 带读到的 `revision`；别人改过了答 `CONFLICT`，`data.revision` 是现在的。改了时间就从现在重新算下一次；`every`、`after` 原样送回不会重新起算。开关（`enable`、`disable`）也算一次改；重新打开从现在算，关着期间到期的不补 |
| 模型建的 | 模型用引擎的 `CronCreate` 建的（存在引擎里，按会话、按 UTC）也列在 `schedule.list` 的 `model` 里，标「模型建的」，只能删（`schedule.model.delete {instance, id}`）。读、删走 `engine.schedule.list`、`engine.schedule.delete`，定时扩展为此被授予 `engine.inspect`、`engine.control`；够不着的实例列在 `unreachable` |

**方法**（参数、答的类型从 `crates/ext/schedule/src/wire.rs` 生成到 `@harnix/core`：`Schedule`、`ScheduleTiming`、`Target`、`Receipt`、`ScheduleList`……）：

| 方法 | 参数 → 答 |
|---|---|
| `schedule.list` | `{space?, project?}` → `{schedules, model, unreachable}`：按下一次时间排，不再到期的排在后面 |
| `schedule.get` | `{id, space?}` → `Schedule`（`next_fire`、`last_delivery`、`revision`、`created_by`……） |
| `schedule.create` | `{space?, title, timing, time_zone?, target, missed?, concurrency?, enabled?}` → `Schedule`；一次性的时间已经过了、cron 永远不到期、项目目录不在，答 `INVALID_PARAMS` |
| `schedule.update` | `{id, revision, space?, title?, timing?, time_zone?, target?, missed?, concurrency?}` → `Schedule` |
| `schedule.delete` | `{id, space?}` → `{deleted}`：连同回执，不删它开过的会话 |
| `schedule.enable` / `disable` | `{id, space?}` → `Schedule` |
| `schedule.runNow` | `{id, space?}` → `Receipt`：开着关着都能立即执行，下一次的时间不变 |
| `schedule.history` | `{id, space?, limit?}` → `{receipts, total}`，新的在前 |
| `schedule.model.delete` | `{instance, id, space?}` → `{deleted}` |

主题 `schedule.list`：全部定时，和上次读到的模型建的，每次变了推整值。

### 9.1 界面：`@harnix/schedule`

第一方 UI 包 `@harnix/schedule`（`ui/packages/schedule/`），`schedule` 扩展启用时激活。它不 import 别的第一方包，只往别人定义的点里注册；点没人定义（产品没有项目 space、
没有首页、没有流程）时那一处悄悄不出现。数据是主题 `schedule.list`（有东西在画时才订阅）和上面的方法。

| 在哪 | 贡献点 | 是什么 |
|---|---|---|
| 会话菜单「定时…」 | `session.action`（`schedule.session`，`at: ["menu"]`） | 对话框（`schedule-session`）：这个会话的定时和模型在它里面建的；「新建定时」的目标是往它发一轮，定时属于会话所在的 space。别的扩展展示过来的会话（`exposed_from`）不出这一项 |
| 新任务对话框、流程详情「定时运行这个流程…」 | `flow.start.action`（`@harnix/flow` 定义，[Harnix 扩展](harnix-ext.md) 的扩展定义的点） | 编辑器，目标是任务，对话框里已填的流程、inputs、对象、标题、输出目录、worktree 带过来；从项目来的项目固定，`show_in_space` 与定时的 space 是项目 space 列会话的那个；流程详情没有项目，编辑器里问 |
| 项目概况第 40 块「定时」 | `harnix-app.project.panels`（`schedule.project`） | 目标在这个项目里的定时（`overview-schedules`）；「新建定时」可选新会话或任务，项目固定，`show_in_space` 同上 |
| 首页第 50 块「定时」 | `harnix-app.home.panels`（`schedule.all`，`wide`） | 全部定时与模型建的，见 §10.2 |
| 会话行的时钟 | `harnix-app.session.badge`（`schedule.clock`） | 有开着的定时发给它（或复用它），或模型在它里面建了定时：一个时钟（`session-schedule-badge`，`data-count`），悬停列出前两条和下一次时间 |
| 任务行的下一次时间 | `flow.task.badge`（`schedule.next`） | 起它的那条定时还开着时，「下一次 13 小时后」（`task-schedule-next`）；流程自己的时钟在它前面 |

**一行定时**（`schedule-row`，`data-state` 是 `on` · `paused` · `ended`，`data-source="user"`，`data-kind` 是目标）：标题、状态、来源；时间的说法（规则时区不是本机的时写出来）；
做什么；下一次（本机时间，加「13 小时后」；`data-next`）；上一次投递的结果。操作：**立即执行**（`schedule.runNow`，结果——已投递、跳过及理由、失败——进横幅）、
**暂停 / 恢复**（`disable` / `enable`）、**编辑**、**投递记录**（在行下展开 `schedule.history`，新的在前、一次 20 条、「再读一些」；时间按规则时区；每条可以打开它去的会话，
或它起的任务的主会话——任务展示到前台 space 列的那个 space 时就在前台打开，否则到它所属的 space）、**删除**（先确认，连同记录，不删会话和任务）。
模型建的一行（`schedule-model-row`，`data-source="model"`）：五段 cron（UTC）、提示词、在哪个会话（可打开）、下一次；只能删（`schedule.model.delete`，先确认）。

**编辑器**（`schedule-editor`）：

- **标题**（必填，至多 120 字）；**做什么**：往会话发一轮（提示词）· 新会话（项目，可空；「第一次新建，之后复用」或「每次新建」；场景、模型；提示词）·
  任务（项目；流程，`flow.defs {project}` 列出、不能启动的不能选；按 `inputs` 的 schema 画的表单——文字、数字、是否、选项，其余是 JSON；对象（必填）、任务标题、输出目录、
  在 worktree 里跑（不动它就照流程的）；上一次的运行还没结束时「这次跳过」或「再起一个」）。新建时可选哪几种看从哪打开的；编辑时目标的种类不变。
- **什么时候**是引导式的：仅一次（日期与时间）· 多久以后（分钟、小时、天）· 每 N（分钟、小时）· 每天 · 每周（星期几的圆片，至少一个）· 工作日 · 每月（几号，29–31 提示没有这天的月份跳过）·
  自定义（五段 cron）。下面一行把它说成话（「工作日 09:00」）。送出去的形状：工作日是 `weekly {weekdays: [1,2,3,4,5]}`，每月是 `cron "M H D * *"`；读回来时反过来认。
- **时区**：搜索框（时区名、城市、偏移如 `+8`）加列表，最近用过的（这个页面的 `localStorage`，`harnix.schedule.zones`，保存时记下）和本机的在最上面；缺省是本机的。
- **错过了怎么办**：只补最近一次（缺省）或跳过；写明 Harnix 没在跑时不投递。
- 有问题的项点「创建 / 保存」后在各自下面说；页面这一侧查得出的（标题、时间格式、一次性的时间已经过了、星期没选、cron 的形状、绝对目录、必填）不送出去，
  调度器才知道的（cron 永远不到期、时区不认得、项目目录不在）照它的原话写在底部。
- **改**只送改了的字段，带读到的 `revision`；`every`、`after` 没动就不送，不会从现在重新起算。别人改过了（`CONFLICT`）时底部是一条提示：
  **保留我的修改并复核**——重读这一条（`schedule.get`），写出别人改了哪几项，自己改过的字段留着、没动过的换成新的，再保存就是在新版本上改；或**用他们的**，丢掉自己的修改。

## 10. 首页

新开的页面落在这里：首页是左栏第一项（`home`，`order: 5`），界面注册写 `landing: true`——新开一个标签页、重开应用时打开它，而不是上次留在前台的 space
或产品的缺省 space；同一个标签页里刷新留在原处（标签页自己的 `sessionStorage` 记着已经落过地，`harnix.landed`）。外壳不认识它：哪个 space 是落地处，
是 space 自己说的（[Harnix 扩展](harnix-ext.md) §22.3 的 `landing`）。

| 栏 | 是什么 |
|---|---|
| 左 | 只有「待我处理 N」（§3.3），有等人的事时才有；首页没有自己的列表 |
| 中 | 问候（按钟点）与日期，下面是 `harnix-app.home.panels` 的全部区块，按 `order` 从小到大（没写算 100）；`wide` 的占整行，其余各占一列。一块都没有时说一句 |
| 右 | 没有 |

**它不持有对话。** 中区是 `center: "hidden"`、`newSession: false`；左栏的「新会话」先换到第一个能对话的 space（§4）。后端的 `home` 是 `SpaceDecl::new("home").no_sessions()`：
声称它的读照常答（它是一个存在的 space，列出来是空的），而 `harnix.session.create`、`harnix.session.import`、`harnix.session.move` 落进它一律
`SPACE_MISMATCH`（「the space `home` holds no sessions」）。从首页打开一个会话，就到它所属的 space 打开。

**`harnix-app.home.panels`**（`render`，`ui/packages/harnix-app/src/home.tsx`）：首页的一个区块，`{ id, title(), order?, wide?, render(context) }`。`context` 是只有首页才有的东西：
`{ waiting, openWaiting() }`——现在等人处理的条数（`harnix-app.inbox` 全部来源合起来，§3.3）与打开整张列表。首页每次画都问 `render`，
所以要跟着什么变的块自己订阅。每块抛错只丢那一块。首页不认识往里放东西的包，每块由拥有那份数据的包注册：

| `order` | 区块 | id（谁的） | 内容 |
|---|---|---|---|
| 5 | 开始使用 | `harnix-app.setup`（`@harnix/harnix-app`），`wide` | 首次运行，只在还有没做完的时候有（见下） |
| 10 | 全局统计 | `observability.stats`（`@harnix/observability`），`wide` | 一行数字（§10.1） |
| 20 | 待我处理 | `harnix-app.inbox` | §3.3 的列表，全部项目的，至多 8 行 |
| 30 | 最近会话 | `harnix-app.sessions` | 会话 space 最近活动的 8 个（归档的不列），在跑的带点，有项目的写项目名；点了到所属 space 打开（`host.session.open(…, {switchSpace: true})`）；「全部会话」到会话 space |
| 40 | 最近任务 | `flow.tasks`（`@harnix/flow`） | 全部项目的任务（试运行不算）：运行中、等待、已结束各几个；最近活动的 8 个，同项目概况的任务行（§3.2），后面写项目名；点了照 §3.2 打开 |
| 50 | 定时 | `schedule.all`（`@harnix/schedule`），`wide` | 这个进程里全部的定时（§10.2） |
| 60 | 用量 | `observability.summary`（`@harnix/observability`），`wide` | 最近 30 天的用量汇总（§10.3） |

产品没链某个扩展，它那一块就不在；没有 `harnix-app` 就没有首页，别的包往这个点里注册时给的 `orElse` 什么都不做。

**开始使用**：新开的页面落在首页，所以首次运行在这里也说一遍，和新对话的空页（§4.1）是同一套：

- 上面是「还没有配置后端模型」的提示与「配置模型」（到 设置 · 模型与凭据），`harnix.hello` 说没有模型可用时才有（同 §4.1 模型那一步的判断）；
  画出来时读一次，此后每关一次对话框再读。它不能关，配好模型就没了。这条提示是 `@harnix/react` 的 `ModelSetupNote`，输入框上方的是同一个。
- 下面是 `home.card` 的全部卡片，按 `order`，每张由它的包画、各自失败——就是空页上的那几张（模型、项目、场景与权限，以及应用与扩展包加的），
  做完、关掉的一样不画；关掉的记在同一个 `harnix.setup.dismissed` 里，两处一起不再画。
- **做完了就没有**：卡片什么都没画时不占位置（`:empty`）；提示不在、卡片全空时，整块不显示。

### 10.1 全局统计

| 项 | 值 | 来源 |
|---|---|---|
| Harnix、Agix | 版本（`-` 之前的部分） | 主题 `observability.process` |
| 运行时长 | `uptime_secs`，页面每秒往上数（只重画这一格） | `observability.process` |
| 内存（峰值） | `peak_rss_bytes` | `observability.process` |
| 系统线程 | `threads` | `observability.process` |
| 已连接用户 | `connections` 的条数；悬停列出设备、客户端种类（桌面、手机、浏览器、脚本）、拓扑、连上多久 | `observability.process` |
| 实例 | 实例数 | 主题 `harnix.instances` |
| Agent（活动/全部） | 主 Agent 的数：活动的是在跑、在排队、在等人、有后台任务在跑或已加载的 | 每台实例的主题 `harnix.instances.sessions {instance}` |
| 运行中会话 | 有一轮在跑的会话 | 主题 `harnix.sessions.status` |
| 运行中的后台任务 | 各行 `tasks` 的和 | `harnix.instances.sessions` |
| 待我处理 | `context.waiting`；点了打开待我处理（`context.openWaiting()`），有等的事时标亮 | 首页 |

每个读都是设备自己的、不声称 space：这些是整个进程的数。`observability.process` 是 `observability` 的方法，也是同名的主题，有人订阅时每 5 秒重算一次推出；
数据是 `observability` 凭 `process.observe` 从宿主拿的进程信息，值里还有产品名与版本（`app`）、`started_ms`。`process.observe` 是 `observability` 的可选能力：
产品不授予时这个方法与主题被拒，录像、上下文这些照旧。

### 10.2 定时

`@harnix/schedule` 的 `schedule.all`：不带 `space` 调，所以是这个进程里**全部**的定时，不论属于哪个 space（§9）。数据是主题 `schedule.list`，变了就推，不用刷新。

- 上方：「新建定时」（目标可选新会话或任务，项目在编辑器里填）、按状态过滤（全部、开着、已暂停、已结束）、按标题和提示词搜索。
- 每条定时一行（`schedule-row`），行的样子与操作同 §9.1：标题、状态、来源、时间的说法、做什么、下一次、上一次投递；立即执行、暂停 / 恢复、编辑、投递记录、删除。
- 「模型建的」一节：模型用引擎的 `CronCreate` 在会话里建的（`schedule-model-row`），五段 cron（UTC）、提示词、在哪个会话（可打开）、下一次；**只能删**。
- 读不到定时的实例（`unreachable`）在下面写出来；最下面一行写明 Harnix 没在跑时不投递。

### 10.3 用量

`@harnix/observability` 的 `observability.summary`（组件 `UsageSummaryTab`）：**最近 30 天里每个会话用了多少**，从录像里读。

**读谁**：每台实例（主题 `harnix.instances`）列出的主 Agent（`harnix.instances.sessions {instance}`）里，`last_active` 落在这 30 天里的
（没写 `last_active` 的也问）。按实例、一次至多 200 个调 `observability.usage.daily {instance, session_ids, days: 30, tz_offset_min}`，
`tz_offset_min` 是这台设备此刻在 UTC 以东多少分钟：一次调用算在它发出的那天，日界按这台设备的时区。每个会话的数含它下面的子 Agent，
一个会话只属于它所在的项目（会话行的 `project_root`）。画出来时读一次，「重新读」再读；不跟着轮次自己刷新。

| 部分 | 是什么 |
|---|---|
| 合计 | 会话（有调用落在这些天里的）、调用（轮次的调用，`+N` 是轮次之外的：压缩、记忆、标题）、token 合计、输入、输出、缓存读、缓存写、模型用时 |
| 每天的 token | 30 根竖条，一天一根，高度按最多的那天折算，没调用的那天只留底线；悬停是那天的 token 与调用数 |
| 按项目、按模型、按天 | 三张表，列是名字、会话、调用、输入、输出、缓存读、缓存写、合计、模型用时；点表头按那一列排，再点反过来（数字列先从大到小）。没有项目的会话归在「（无项目）」，录像没写模型的归在「（未写明）」；按天只列有调用的那几天 |
| 别的包加的区块 | 贡献点 `observability.usage.section`（见下），按 `order` |

读不到的实例（连不上、重启中）在合计下面点名，其余照常；休眠的模式 3 进程不为这个叫醒，给的是之前读过的数，页面说有几个会话是这样。

**数的口径**：录像只覆盖会话最近一次运行（daemon 重启后录像重写），更早的运行用掉的不在里面，所以这不是「这 30 天一共用了多少」，而是「这些会话现在的录像里、
落在这 30 天的那部分」。读者另记每个会话按天按模型的合计（每个时区偏移各一份，至多 1024 份，只在内存里）：链路没见到这个会话
再有动静，就不再读它的录像。

**区块**：`@harnix/observability` 定义贡献点 `observability.usage.section`（呈现，`{id, title(), order?, render(context)}`，
`context` 是 `{since, days}`：`since` 是第一天零点的毫秒数），别的包往里加自己那一份，汇总不认识它们。`@harnix/flow` 加「按流程」：
`flow.list {limit: 2000}` 读最新的运行，`created_at` 不早于 `since` 的、不是试运行的，按流程加起来——运行数（没全结束时括号里是结束了的）、
轮数、四项 token 与合计（运行记录里各站的轮次报上来的，含重试）、运行时长（结束了的运行从开始到结束，含等人的时间）。这一份来自流程自己的
运行记录，和上面的录像是两本账：流程的会话也在上面的表里。

## 11. 流程 space

流程是 Harnix 内置的应用开发工具链（第一方扩展 `flow`，界面 `@harnix/flow`），产品链了 `flow` 才有——Harnix 自己的二进制 `harnix` 链了，
二开的产品 `.extension(flow)` 了才有。流程是什么、怎么写见流程指南（`crates/flow/docs/guide.*.md`，这里右栏的「参考」就是它）。

**它是一个 space**（`ui/packages/flow/src/space.tsx` 的 `registerFlowSpace`）：id 是 `harnix.spaces.list` 里 scene 为 `flow-author` 的那个后端 space（缺省 `flow`，
由 `flow` 扩展声明），图标 `fork`，`place: "bottom"`——它管的是应用的工具链、不是产品的一个面，所以入口在左栏底部、头像上面。自己编外壳产物的产品，
把 `@harnix/flow` 编进去就行；名字改过的产品自己调 `registerFlowSpace(r, { space, module })`。

| 栏 | 是什么 |
|---|---|
| 左（`FlowSidebar`） | 「返回」（只在从项目 space 进来看一个任务时，回到那里）；通用会话；全部流程，选中的那条展开：它自己的对话，它的运行与试运行，新的在前。在这里点了什么，右栏就换到它所在的那张列表，两栏说的是同一件事 |
| 中 | 左栏选中什么就是它的会话：流程的对话（`flow.session {flow}`，Agent 在里面改它、试运行、启动、操作它的运行，都直接执行、不审批，校验照旧）；运行 → 它的主会话（`flow.get {run}` 的 `@main`；试运行没有会话，是流程的对话）；不属于哪条流程的是通用会话（`flow.session {}`）。输入框上一行写着它在说哪条流程 |
| 二级栏 | 选中的流程：流程图、定义、件、只读 YAML，可以试运行、启动，标题行上有别的包经 `flow.start.action` 加的起法（「定时运行这个流程…」）；选中的运行：状态、耗时、token，暂停 / 恢复 / 取消、试运行的单步与跑完、站的确认等操作、子 Agent 与后台任务。关掉它只是收起，选中的不变，下一次选或换页签时再出来 |
| 右 | 三个页签：**流程**、**运行**、**参考** |

- **流程**：标题、来源（内置 / 已改 / 自建 / 项目）、不合法的标记；按来源、合法与否筛选，按名字搜索，按名字或标题排序。「添加」
  打开通用会话并在输入框里放一句提示。数据是主题 `flow.defs`。
- **运行**：全部运行与试运行，状态、标题、试运行标记、流程、开始时间、耗时、token；按状态、流程、真跑 / 试运行筛选，按开始、耗时、token 排序。
  数据是主题 `flow.runs`、`flow.dry_runs`。在这里点一条运行，它的流程也一起选中：左栏换到那条流程、展开它的运行，这一条标为当前；再点一次是放开这条运行，流程留着。
- **参考**：流程指南（`flow.guide {lang}`）、本产品的目录与字段表。
- 这一处只呈现，不编辑定义：改流程在它的对话里，由 Agent 做。

**同一条运行的另一个入口：项目里的任务**（§3.2）。从项目起的任务，`flow` 把主会话展示到项目 space（`shown_in`），
项目树里点它，主会话就在项目 space 的中区打开；没展示到那里的运行（从流程 space 或定时起的）——流程 space 带到前台、停在这条运行上，
左栏的「返回」回到项目 space（`FlowSpaceState.enter(flow, run, { from })`），任务行菜单的「在流程里打开」对任何任务都这样进。
两种进法，会话旁的二级栏都不开，改在右栏开「流程」页签（`sidebar.tab`，`kind: "flow"`，`keepMounted`；`openTab("flow", { params: { run } })`；
没带 `run` 时按这个会话在浏览器里记下的，再没有就在最近的运行里找主会话是它的那条）。
两处画的是同一组件（`ui/packages/flow/src/RunPanel.tsx`）：`RunHead`（状态，暂停 / 恢复 / 取消）、`FlowGraph`（当前站高亮）、
选中站的 `StationPanel`（确认、放行、退回、强制通过……按站给）、`RunStations`（每站的状态、耗时、token、输出一行、门禁结果）、
`OutputDirLine`（输出目录：复制路径、打开文件页签）、`RunSessionsList`（子 Agent 与后台任务）。流程 space 的二级栏是 `RunDetail`，从这里取件。

## 12. 诊断（在设置里）

设置 → 诊断是外壳核心的一节（`ui/packages/shell/src/settings.tsx` 的 `Diagnostics`），排在各扩展的节之后、「关于」之前：

- **引擎**：`engine.daemon.status` 的版本、运行时长、会话数，以及启用的场景。
- **运行期界面模块**：贡献点 API 版本、`[ext.plugins] ui_modules`、页面的 import map 与 CSP，以及各 Agix 插件的 `[[ui]]` 条目（`plugins.list`）。
- **扩展**：每个扩展（hello 与 `harnix.ui` 说的）是否启用、载体、信任、版本；在本页上有界面面板的，下面跟着**本页上的面板**
  （在用的个数 · 崩过的次数，每个面的点、id、状态：加载中 / 运行中 / 出错 / 已搁置 / 已停止）与**最近被拒的界面请求**
  （iframe 经桥要、被拒的名字与理由，留最近 20 条）。只是这一页的，刷新就清零；一个面 5 分钟内失败 3 次就被置于一旁。
- **界面部件**：注册表拒掉的贡献与理由，含注册进了没人定义的点、最后被丢掉的那些。
- **自检**：`engine.daemon.doctor`。
- 然后是别的包放进这一节的区块（见下），各在自己的标题下。

`harnix.extensions`（每个扩展：id、信任、载体、版本、启用、授予、行数、停用理由）、`harnix.diagnostics`（扩展、授予、路由表、主题、路由、迁移）与
`harnix.audit`（审计环）由后端提供，这一页不画它们。

**放进诊断的区块**：`settings.section` 写 `within: "diagnostics"` 的，不是导航里单独的一节，而是画在外壳自己的诊断内容之后、它的 `label()` 下面
（[Harnix 扩展](harnix-ext.md) §22.3）。第一方的两块：

| 区块 | 谁的 | 是什么 |
|---|---|---|
| 扩展包运行状况（`packages.runtime`） | `@harnix/packages`（组件 `PackagesTab`） | 下面 §12.1 |
| 扩展日志（`observability.extlog`） | `@harnix/observability` | 下面 §12.2 |

`host.settings.open("packages.runtime")` 打开设置并停在诊断上。

### 12.1 扩展包运行状况

产品链了 `packages`（管扩展包的宿主组件）才有。数据只有一个：主题 `packages.list`（值同方法 `packages.list`；载体报告状态变了才推，另有至多 10 秒一次的兜底，见 [扩展包](harnix-ext-package.md)）。
**这一块只看，不操作**：启停、同意、数据、卸载、更新在设置 → 扩展包。

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
它的方法与主题、**最近被拒的调用**（方法 — 理由）。它在本页上的界面面板是外壳的，在上面的「扩展」里。

### 12.2 扩展日志

本进程记下的、属于各个扩展的日志——原生扩展在应答调用、计算主题、启动与停止时写的，WASM 扩展包自己的日志（`host::log`）与载体关于它的行，
模式 3 扩展进程的 stderr，宿主关于某个扩展的行。怎么认一行是谁的见 [原生扩展](harnix-ext.md) §17.9。进程只留最新的：每个扩展至多 500 行，
全进程至多 4000 行，超出时先丢最旧的，区块上方说丢了多少。

数据是 `observability.extensions.log {extension?, level?, after?, limit?}`：设备的方法，`observability` 只把这四个键转给内核的 `harnix.extensions.log`
（要 `process.observe`，`observability` 的可选能力）。日志里的行可能说出包作者没打算给别人看的东西，所以内核那一行不对设备与扩展包开放——设备直接调它被拒，
经这里读，每次读都进审计。

| 控件 | 做什么 |
|---|---|
| 扩展 | 全部，或只看一个（每个扩展后面是它留着的行数）；换了就从头读 |
| 级别 | 全部，或某一级及以上；换了就从头读 |
| 实时 | 缺省开：每 2 秒读一次上次最后一行之后的，新行加在底下；读者往上翻时不把视图拉回底部。关掉就不再读 |
| 复制 | 把显示着的行按 `时间 级别 扩展 [来源] 消息` 一行一条复制成文本 |

每行：时间、级别（错误、警告着色）、扩展 id、来源（调用中 / 扩展包 / stderr / 宿主，悬停是 tracing 的 target）、消息（之后跟着事件的其余字段 `name=value`）。
页面至多留 1000 行。
