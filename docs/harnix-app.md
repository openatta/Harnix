# Harnix app

> 读者：用 Harnix 自己的二进制 `harnix` 的人；在 Harnix 上做应用、想看「用扩展做一个产品面最少要什么」的人；决定自己的产品留不留这两个 space 的人。
>
> Harnix 是三部分：**框架**、**Harnix app**、**系统 space**（[系统 space](system-space.md)）。Harnix app 就是会话 space 与项目 space：
> 新建会话、按时间看会话、按项目管理会话。它由两半组成：后端是第一方原生扩展 `harnix-app`（`crates/ext/harnix-app`），界面是前端包
> `@harnix/harnix-app`（`ui/packages/harnix-app`）。用词照 [术语](glossary.md)；扩展接口见 [Harnix 扩展](harnix-ext.md)。
>
> 本文照代码写：后端在 `crates/ext/harnix-app/src/`（`lib.rs`、`workspaces.rs`、`files.rs`、`project.rs`、`export.rs`、`store.rs`），界面在
> `ui/packages/harnix-app/src/`（`index.tsx`、`sessions.tsx`、`projects.tsx`、`sidebar.tsx`、`dialogs.tsx`、`store.ts`），一轮的改动由中枢记（`crates/hub/src/changes.rs`），外壳的菜单、层与新对话页在
> `ui/packages/shell/src/`（`Menu.tsx`、`Shell.tsx`、`dialogs.tsx`；层与菜单的底子在 `ui/packages/react/src/layers.ts`），profile 在
> `crates/assembly/src/profile.rs`。对不上时以代码为准。

---

## 1. 它是什么

| | 会话 space | 项目 space |
|---|---|---|
| id · 图标 · 次序 | `sessions` · `chat` · 10 | `projects` · `folder` · 20 |
| 标题 | 会话（Sessions）：全部会话，按时间 | 项目（Projects）：按项目看会话 |
| 左栏 | 搜索框 + 全部会话，按时间分组 | 一棵树：项目（工作区），一次只展开一个，它的会话挂在下面 |
| 中区 | 对话（`center: "chat"`） | 对话；新建的会话跑在展开的项目里 |
| 右栏 | 会话的停靠面：引导页，文件、预览、改动（§6），`@harnix/system` 的「输出」（Agent 后台 Shell 与监视器的只读输出流，一个任务一个页签，从对话里工具行的「查看输出」打开），别的包注册的页签，和 `reveal` 打开的详情 | 同左 |
| 后端 space | `sessions` | **同一个** `sessions`（界面注册里 `sessions: "sessions"`） |

**项目 space 是界面上的一个视图，不是第二个会话集合。** 「按时间看」和「按项目看」是同一批会话的两种排法；会话按 space 绑定是
不变量（[概念与架构](concept_and_architecture.md)），如果它们是两个后端 space，同一个会话就得属于两个 space。所以项目 space
写 `sessions: "sessions"`：两个界面视图，一个后端归属。外壳每次会话调用带的是 `sessions`，不是 `projects`。

**两半，一个扩展。** `harnix-app` 扩展声明后端的会话 space（`SpaceDecl::new("sessions").default_space()`：不声称 space 的设备调用按它检查，
没人安置的会话归它），提供 `harnix-app.*` 的方法，状态在自己的目录 `~/.<product_id>/ext/harnix-app/data/state.json`（工作区、归档、偏好；
删掉只丢分组）。会话的标题不在这里：它是引擎给会话的名字，`harnix.session.rename` 改的就是它。界面包 `@harnix/harnix-app` 注册两个 `space`、
`sidebar.group` 的 `harnix-app.time`、会话菜单的归档与导出两个 `session.action`、`@` 的文件补全，以及右栏的文件、预览、改动三种页签、
对话里的改动卡和工具行上「在预览中打开」的 `session.action`（§6）。

Harnix 自己发布的二进制 `harnix`（`crates/cli`）链进全部第一方扩展（`harnix::first_party::standard()`，含 `harnix-app`）、`packages` 与流程扩展。
它的缺省是原生扩展加模式 2（见 [应用开发](app-development.md)）。

**关掉它**：profile 的 `[extensions] off = ["harnix-app"]`。关掉的扩展照样登记（别人占不了它的名字），但不安装、不迁移、不启动；
外壳按 `harnix.hello` 里启用的扩展激活 UI 包，所以 `@harnix/harnix-app` 也不注册，两个 space 的入口随之没有。去掉之后，不声称 space 的调用
没有缺省 space 可落，可以用 `[extensions] default_space = "<space>"` 指一个已启用扩展声明的 space；指的不是这样一个 space，进程起不来。

## 2. 会话 space

**左栏**：

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

会话停下来等人时，对话里出一张卡（`@harnix/react` 的 `AskCard`），左栏那一行与系统 space 说它在等什么（`waiting`：
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

- 左栏顶上「项目」与「+」（添加项目）。先列 5 个，「更多（N）」。
- **一次只展开一个**（记在页面的 `localStorage`，`harnix.project`）。点一个没展开的项目：展开它，并且停在新对话的空页上。
- 展开的项目列它的会话（按最近活动，先列 8 个，「全部 N 个」）。
- **新会话跑在展开的项目里**：在项目 space 里新建会话时 `project_root` 是这个项目的目录，归属仍是 `sessions`。

**添加项目**（对话框）：

| 做法 | 方法 | 后端 |
|---|---|---|
| 打一个名字 | `harnix-app.projects.create {name}` | 在项目根（`projects_dir`；`--data-dir`，缺省 `$HARNIX_DATA_DIR`，再缺省 `~/Documents`）下建这个目录并登记成工作区；名字只能是一段路径 |
| 选一个已有目录 | `harnix-app.workspaces.create {path}` | 目录要已经存在；同一个目录再加一次是幂等的 |
| 浏览目录 | `harnix-app.listDirectory {path?}` | **选择器从 `projects_dir` 起**：不给 `path` 时打开它（还没有就建；建不了才退到 `$HOME`）；只在 `$HOME` 或项目根之内 |

**项目菜单**：在这个项目里新建会话（打开新会话对话框，§4）、信任这个项目 / 不再信任这个项目（§3.1）、
改名（`harnix-app.workspaces.update {id, title}`）、移除分组（`harnix-app.workspaces.remove`）。

### 3.1 项目信任

一个项目自己的设置（`<项目>/.agix/` 下的设置、agent 类型、hook 脚本，项目里的 skill）是仓库内容。引擎缺省**不信任**项目：
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

状态的形状，引擎原样的字段加上 `harnix-app` 算好的三项——项目 space、右栏的「上下文」页、项目自带的流程读的都是这一份：

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

## 4. 新建会话

- **左栏顶上的「新会话」**（⌘K）：不建会话，只打开新对话的空页。前台是系统 space、`center: "hidden"` 的 space 或嵌在别的
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
- **换模型**：会话的模型菜单在有 `harnix.session.setModel`（`[shell] model_switch = true`）时用它，否则用系统 space 的
  `system.model`。

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
（`Write`、`Edit`、`MultiEdit`、`NotebookEdit`）的工具行下面也有一项「**把文件退回到这次调用之前**」。space 把对话设成只读时没有消息菜单。

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

**谁能调**：设备，只对它声称的 space 的会话（不声称时是缺省 space），不能跨 space——系统 space 的跨 space 也不行，答 `SPACE_MISMATCH`，同
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

右栏是每个会话一个停靠面（[Harnix 扩展](harnix-ext.md) §22.7）。引导页上还有 `@harnix/observability` 的两页——**上下文**（下一次调用带什么：
系统 space 在时上下文的组成与系统块，生效的技能与钩子，MCP 服务器的工具与指令，被项目信任扣下的项与「信任这个项目」）和**用量**
（本会话这次运行的轮数、调用、token、缓存命中、用时，带「打开轨迹」）；它们读的是设备自己的方法，在 [Harnix 扩展](harnix-ext.md) §22.7、§22.10。
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

## 7. profile

### 7.1 `[shell]` 与 `[extensions]`

```toml
[shell]
model_switch = false      # true：加 harnix.session.setModel，没有系统 space 也能在输入框换模型
new_session = true        # false：整个外壳不画「新会话」、⌘K 不归外壳；只是界面
narrow_details = "over"   # ≤ 960 宽时打开的右栏：over 是盖住中区的抽屉；beside 三栏并排（< 720 仍是抽屉）

[extensions]
off = ["flow"]            # 链进来了但关掉的扩展；不写是都开
default_space = "sessions"  # 不声称 space 的调用按它检查，换掉扩展声明的缺省；必须是已启用扩展声明的 space
```

两节都拒绝不认识的键；0.4.2 的 `[shell] spaces` 已删，写了启动就拒绝。哪些内建 space 在，现在是哪些扩展开着：

| `[extensions] off` 里的名字 | 去掉之后 |
|---|---|
| `harnix-app` | 会话 space 与项目 space 的后端与界面都没有；后端的 `sessions` space 也不再声明（见 §1：要另指缺省 space） |
| `system` | 系统 space 的后端与界面都没有，它的方法连同授权一起撤掉（[系统 space](system-space.md)） |
| `flow` | 流程的界面不注册；留 `flow` 去掉 `system` 时它是一个单独的流程 space（[系统 space](system-space.md)） |

一个 space 都没有时，外壳说「没有注册任何 space」。

### 7.2 `[brand]` 与目录

```toml
[brand]
name = "Harnix"           # 页面标题、左栏产品名；不写是 Harnix
icon = "brand/icon.svg"   # SVG，相对 profile；服务在 /brand/icon.svg
product_id = "harnix"     # 程序自己的目录 ~/.<product_id>；小写字母、数字、-，字母开头；不写是 harnix
```

Harnix app 的项目在项目根 `projects_dir`（`--data-dir`，缺省 `$HARNIX_DATA_DIR`，再缺省 `~/Documents`）里，工作区登记、归档、偏好在 `~/.<product_id>/ext/harnix-app/data/state.json`，会话标题在引擎里，
对话在引擎的目录（`~/.agix/`）里。

### 7.3 `harnix.hello` 里的 `shell`

外壳不读 profile，读握手之后 `harnix.hello` 的 `shell`：`brand {name, icon, product_id}`、`runtime_modules`、`new_session`、
`model_switch`、`narrow_details`、`state_dir`、`projects_dir`、`user`。哪些 space 在，外壳看 `hello.extensions` 里启用的扩展
（会话与项目是 `harnix-app` 的，系统 space 是 `system` 的）。`harnix-app.describe` 也答 `projects_dir`。

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
引擎自己的缺省是开；**Harnix 的缺省是关**：实例建起来（非 attach 的承载方式）时，`harnix-instance` 在该实例全局层的 `settings.json`
里缺 `telemetry_enabled` 就写 `false`（`crates/instance/src/defaults.rs`），已有值（开或关）不动，其余内容不动。
人在设置页里「清除」这一项后，下次启动会再写一次 `false`。

**高级**：压缩（`compaction`）、执行参数（`execution`）、hooks（`hooks_config`）、团队（`team`）、检查点（`checkpoints`）的当前值，
按引擎实际使用的值显示，并说哪一层写了它；没有任何一层写的，说是引擎缺省。这些字段声明为 `json` 类型、没有可写的层，
`settings.write` 在任何层都拒绝；要改就改文件。hooks 只显示每个事件有几条，不显示每条跑什么（hook 里可能带凭据）。
Agent 循环内部的参数不做表单。

### 7.5 设置页：MCP 服务器

「MCP」一节（`@harnix/settings`，后端 `settings` 扩展的 `settings.mcp.*`，[Harnix 扩展](harnix-ext.md) §22.9）列出这一对 (项目, 场景)
每一层的服务器，连停用的。每张卡片：名字、连接方式、哪一层；状态（已连接、已登录、需要登录、登录失败、连接中、已断开、失败、被策略拒绝、
项目未受信、已停用、未报告）和引擎给的原因；命令与参数或 URL；工具数。另一层有同名的，低的那条说「未生效」；要 `private_key_jwt` 的说这个构建没有它。

| 操作 | 说明 |
|---|---|
| 添加服务器 | 选层（全局、场景、当前项目）、名字、连接方式（stdio：命令、每行一个参数、环境变量；http / sse：URL、请求头）。环境变量和请求头的每一项是「值」（密码框）或「取自环境变量」；项目层只有「值」，它读不到引擎的环境 |
| 编辑 | 名字和层不改；已写的值只显示末 4 位，缺省「保留」，改成「值」就换掉 |
| 停用 / 启用 | 停用的服务器挪出这一层的文件，由扩展收着（连同它的值），引擎看不到；启用原样放回 |
| 重连 | 引擎重读这一对的设置，它的服务器都重连 |
| 移除 | 先问一次，确认了才删，连同它的值 |
| 登录 / 退出登录 | 远端（http、sse）的服务器有。登录开一个新窗口去授权页，回来落在 Harnix 自己的路由上，完成后窗口关掉、卡片刷新；页面不在 `https` 或本机地址上时，登录要在引擎那台机器的浏览器里完成 |

项目层的服务器经引擎写：项目没受信时引擎拒绝，什么都不写（先在项目菜单里信任它）。全局层、场景层由扩展照引擎的写法写文件：值进那一层的
`settings.local.json`（0600，不提交），其余进 `settings.json`。

## 8. 它是怎么用框架做的

Harnix app 是原生扩展最小的例子：一个后端扩展、一个界面包，用的都是二开产品也用的接口。

| 部分 | 用了什么 | 二开产品对应的做法 |
|---|---|---|
| 后端 | 扩展 `harnix-app`：`workspaces.list/create/update/reorder/remove`、`sessions.archive`、`sessions.export`、`sessions.importUpload`、`sessions.import`、`prefs.set`、`search`、`files`、`files.list`、`files.read`、`listDirectory`、`recentProjects`、`projects.create`、`projects.status`、`projects.trust`、`sessions.projectStatus`、`describe`，主题 `trust`；`Manifest::requires([transcripts.read, engine.export])`、`.optional([engine.control])` | 一个 `Extension` 的 `register`、`Manifest::requires`（[Harnix 扩展](harnix-ext.md)） |
| 行的开放范围 | 全部 `Access::device()`：人经界面做的决定只对设备开；`sessions.*`、`files.list`、`files.read` 另按 `RowScope::Session` | 同一套 `Access`，扩展也要调的行写 `.with_extensions()` |
| 后端 space | `sessions`，`default_space()` | `register` 里的 `SpaceDecl` |
| 界面 space | 两个 `space` 注册，owner 是扩展 id `harnix-app`；项目 space 写 `sessions: "sessions"` | `r.register('space', …)`（[Harnix 扩展](harnix-ext.md)的前端一节） |
| 分组 | `sidebar.group` 的 `harnix-app.time` | 注册自己的 `sidebar.group` 换掉它 |
| 右栏 | `sidebar.tab` 的 `files`、`preview`、`changes`，`flow.block` 的改动卡，`session.action` 的 `harnix-app.open-file`（`at: ["arg", "tool"]`） | 注册同一个 `kind` 的 extension 类型顶替，或更具体的 `patterns` |
| 数据 | 方法 `harnix.sessions.list`，主题 `harnix.sessions.status` | `useHost()` 的 `call` 与 `subscribe` |

`harnix-app` 只用公开的扩展接口，由 `crates/harnix/tests/layering.rs` 盯着（第一方扩展只依赖 `harnix-ext`）。界面一侧，`@harnix/harnix-app`
只从 `@harnix/react` 与 `@harnix/core` 的公开导出取东西，经 `HostApi` 调后端；两个左栏的组件在这个包里，不在外壳里。

**要一个不同的 Harnix app**：不 fork 外壳，在自己的产品里把 `harnix-app` 写进 `[extensions] off`，用自己的扩展声明 space、用自己的 UI 包
注册 space 顶替（[Harnix 扩展](harnix-ext.md)）。0.4.2 里后端随框架链进来、只能藏不能去；现在它是一个独立扩展，关掉就是整个不在，
`harnix-app.*` 的方法也不再登记。
