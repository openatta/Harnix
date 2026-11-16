# Harnix app

> 读者：用 Harnix 自己的二进制 `harnix` 的人；在 Harnix 上做应用、想看「用扩展做一个产品面最少要什么」的人；决定自己的产品留不留这两个 space 的人。
>
> Harnix 是三部分：**框架**、**Harnix app**、**系统 space**（[系统 space](system-space.md)）。Harnix app 就是会话 space 与项目 space：
> 新建会话、按时间看会话、按项目管理会话。它由两半组成：后端是第一方原生扩展 `harnix-app`（`crates/ext/harnix-app`），界面是前端包
> `@harnix/harnix-app`（`ui/packages/harnix-app`）。用词照 [术语](glossary.md)；扩展接口见 [Harnix 扩展](harnix-ext.md)。
>
> 本文照代码写：后端在 `crates/ext/harnix-app/src/`（`lib.rs`、`workspaces.rs`、`files.rs`、`export.rs`、`store.rs`），界面在
> `ui/packages/harnix-app/src/`（`index.tsx`、`sessions.tsx`、`projects.tsx`、`dialogs.tsx`、`store.ts`），外壳的菜单、层与新对话页在
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
| 右栏 | 会话的停靠面（引导页；本版由别的包注册的页签，包括 `@harnix/system` 的「输出」：Agent 后台 Shell 与监视器的只读输出流，一个任务一个页签，从对话里工具行的「查看输出」打开），和 `reveal` 打开的详情 | 同左 |
| 后端 space | `sessions` | **同一个** `sessions`（界面注册里 `sessions: "sessions"`） |

**项目 space 是界面上的一个视图，不是第二个会话集合。** 「按时间看」和「按项目看」是同一批会话的两种排法；会话按 space 绑定是
不变量（[概念与架构](concept_and_architecture.md)），如果它们是两个后端 space，同一个会话就得属于两个 space。所以项目 space
写 `sessions: "sessions"`：两个界面视图，一个后端归属。外壳每次会话调用带的是 `sessions`，不是 `projects`。

**两半，一个扩展。** `harnix-app` 扩展声明后端的会话 space（`SpaceDecl::new("sessions").default_space()`：不声称 space 的设备调用按它检查，
没人安置的会话归它），提供 `harnix-app.*` 的方法，状态在自己的目录 `~/.<product_id>/ext/harnix-app/data/state.json`（工作区、归档、偏好；
删掉只丢分组）。会话的标题不在这里：它是引擎给会话的名字，`harnix.session.rename` 改的就是它。界面包 `@harnix/harnix-app` 注册两个 `space`、
`sidebar.group` 的 `harnix-app.time`、会话菜单的归档与导出两个 `session.action`、`@` 的文件补全。

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
  space 里的 space 时，先换到第一个能对话的 space。⌘\ 收起左栏，⌘/ 聚焦输入框。对话框开着时这三个键都不生效。
- **窄窗口**（≤ 960 宽）：左栏是 56px 的轨，轨上的「会话」入口、收起键或 ⌘\ 把它作为抽屉盖在页面上打开；在抽屉里选了会话或 space、
  按 Esc、点幕布就收回。打开的右栏是从右边盖住中区的抽屉（`narrow_details = "beside"` 时 720–960 宽仍三栏并排），里面是同一条页签栏、同一个会话的停靠面（[Harnix 扩展](harnix-ext.md) §22.7）；
  会话头角落的按钮在右栏收起时把它展开。宽窗口里人自己
  收起或展开的左栏，窗口变窄再变宽后照旧。
- **新对话的空页**：问候（按钟点是早上、下午、晚上）、这个 space 的 `subtitle()`、输入框、`suggestions()` 的建议（点了就建会话
  并发出）、`notice()`，下面是 `home.card` 的卡片。**第一次发出时才建会话**：`harnix.session.create`（带前台 space 的归属，项目 space 里带项目的
  目录），然后 `harnix.session.send`。Harnix app 的两个 space 不给建议、提示与卡片；应用的扩展与扩展包可以给 `home.card`。
- **新会话对话框**（从项目菜单进来）：场景（启用的场景，引擎缺省的在前）、权限模式（`default`、`acceptEdits`、`plan`、
  `bypassPermissions`）、项目目录（`harnix-app.recentProjects` 给的最近目录，或不带项目），可以浏览目录。
- **换模型**：会话的模型菜单在有 `harnix.session.setModel`（`[shell] model_switch = true`）时用它，否则用系统 space 的
  `system.model`。

**`[shell] new_session = false`**：整个外壳不画「新会话」、⌘K 不归外壳（会话由应用自己建）。**只是界面**：空页上的输入框只看 space
自己的 `newSession`，所以空的 space 仍然给一个输入框；要一个 space 没有它，在那个 space 上写 `newSession: false`（这时画
`placeholder()`）。服务端照旧收 `harnix.session.create`。

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
| 导出 | `harnix-app.sessions.export` | Markdown，或引擎打的数据包（zip，可带录像） |
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

## 6. profile

### 6.1 `[shell]` 与 `[extensions]`

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

### 6.2 `[brand]` 与目录

```toml
[brand]
name = "Harnix"           # 页面标题、左栏产品名；不写是 Harnix
icon = "brand/icon.svg"   # SVG，相对 profile；服务在 /brand/icon.svg
product_id = "harnix"     # 程序自己的目录 ~/.<product_id>；小写字母、数字、-，字母开头；不写是 harnix
```

Harnix app 的项目在项目根 `projects_dir`（`--data-dir`，缺省 `$HARNIX_DATA_DIR`，再缺省 `~/Documents`）里，工作区登记、归档、偏好在 `~/.<product_id>/ext/harnix-app/data/state.json`，会话标题在引擎里，
对话在引擎的目录（`~/.agix/`）里。

### 6.3 `harnix.hello` 里的 `shell`

外壳不读 profile，读握手之后 `harnix.hello` 的 `shell`：`brand {name, icon, product_id}`、`runtime_modules`、`new_session`、
`model_switch`、`narrow_details`、`state_dir`、`projects_dir`、`user`。哪些 space 在，外壳看 `hello.extensions` 里启用的扩展
（会话与项目是 `harnix-app` 的，系统 space 是 `system` 的）。`harnix-app.describe` 也答 `projects_dir`。

### 6.4 设置页：通用、显示、隐私与高级

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

## 7. 它是怎么用框架做的

Harnix app 是原生扩展最小的例子：一个后端扩展、一个界面包，用的都是二开产品也用的接口。

| 部分 | 用了什么 | 二开产品对应的做法 |
|---|---|---|
| 后端 | 扩展 `harnix-app`：`workspaces.list/create/update/reorder/remove`、`sessions.archive`、`sessions.export`、`sessions.importUpload`、`sessions.import`、`prefs.set`、`search`、`files`、`listDirectory`、`recentProjects`、`projects.create`、`projects.status`、`projects.trust`、`sessions.projectStatus`、`describe`，主题 `trust`；`Manifest::requires([transcripts.read, engine.export])`、`.optional([engine.control])` | 一个 `Extension` 的 `register`、`Manifest::requires`（[Harnix 扩展](harnix-ext.md)） |
| 行的开放范围 | 全部 `Access::device()`：人经界面做的决定只对设备开；`sessions.*` 另按 `RowScope::Session` | 同一套 `Access`，扩展也要调的行写 `.with_extensions()` |
| 后端 space | `sessions`，`default_space()` | `register` 里的 `SpaceDecl` |
| 界面 space | 两个 `space` 注册，owner 是扩展 id `harnix-app`；项目 space 写 `sessions: "sessions"` | `r.register('space', …)`（[Harnix 扩展](harnix-ext.md)的前端一节） |
| 分组 | `sidebar.group` 的 `harnix-app.time` | 注册自己的 `sidebar.group` 换掉它 |
| 数据 | 方法 `harnix.sessions.list`，主题 `harnix.sessions.status` | `useHost()` 的 `call` 与 `subscribe` |

`harnix-app` 只用公开的扩展接口，由 `crates/harnix/tests/layering.rs` 盯着（第一方扩展只依赖 `harnix-ext`）。界面一侧，`@harnix/harnix-app`
只从 `@harnix/react` 与 `@harnix/core` 的公开导出取东西，经 `HostApi` 调后端；两个左栏的组件在这个包里，不在外壳里。

**要一个不同的 Harnix app**：不 fork 外壳，在自己的产品里把 `harnix-app` 写进 `[extensions] off`，用自己的扩展声明 space、用自己的 UI 包
注册 space 顶替（[Harnix 扩展](harnix-ext.md)）。0.4.2 里后端随框架链进来、只能藏不能去；现在它是一个独立扩展，关掉就是整个不在，
`harnix-app.*` 的方法也不再登记。
