# Harnix app

> 读者：用 Harnix 自己的二进制 `harnix` 的人；在 Harnix 上做应用、想看「用框架做一个应用最少要什么」的人；决定自己的产品留不留这两个 space 的人。
>
> Harnix 是三部分：**框架**、**Harnix app**、**系统 space**（[系统 space](system-space.md)）。Harnix app 就是会话 space 与项目 space：
> 新建会话、按时间看会话、按项目管理会话。它是用框架自己的公开接口做的一个小应用，也是模式 1 最小的例子。
> 以前叫「空应用」「最小应用」。别和 crate `harnix-app`（`crates/app`）、插件打包工具 `harnix-app` 混：那两个是代码里的名字。用词照 [术语](glossary.md)。
>
> 本文照代码写：界面在 `ui/packages/shell/src/builtins.tsx`、`sessions.tsx`、`projects.tsx`、`dialogs.tsx`、`Menu.tsx`、`Shell.tsx`（层与菜单的底子在 `ui/packages/react/src/layers.ts`），后端在
> `crates/builtin/src/`（`sessions` 那一组），profile 在 `crates/instance/src/profile.rs`。对不上时以代码为准。

---

## 1. 它是什么

| | 会话 space | 项目 space |
|---|---|---|
| id · 图标 · 次序 | `sessions` · `chat` · 10 | `projects` · `folder` · 20 |
| 标题 | 会话（Sessions）：全部会话，按时间 | 项目（Projects）：按项目看会话 |
| 左栏 | 搜索框 + 全部会话，按时间分组 | 一棵树：项目（工作区），一次只展开一个，它的会话挂在下面 |
| 中区 | 对话（`center: "chat"`） | 对话；新建的会话跑在展开的项目里 |
| 右栏 | 只有详情 | 只有详情 |
| 后端 space | `sessions` | **同一个** `sessions`（`SpaceContribution.sessions = "sessions"`） |

**项目 space 是界面上的一个视图，不是第二个会话集合。** 「按时间看」和「按项目看」是同一批会话的两种排法；会话按 space 绑定是
不变量（[概念与架构](concept_and_architecture.md) §5.4），如果它们是两个后端 space，同一个会话就得属于两个 space。所以项目 space
写 `sessions: "sessions"`：两个界面视图，一个后端归属。外壳每次会话调用带的是 `sessions`，不是 `projects`。

Harnix 自己发布的二进制 `harnix`（`crates/cli`）就是框架加 Harnix app 加系统 space，再注册了流程模块：

```rust
fn main() -> anyhow::Result<()> {
    harnix::App::builder().with_builtins().module(flow()).run()
}
```

`flow()` 是带一条示例流程的流程模块（可写的流程包在模块数据目录下的 `packs/`，`command` 门禁打开）。流程的界面在系统 space 里
（[系统 space](system-space.md) §5）。嵌入版是这个二进制再开模式 2（见 [应用开发](app-development.md) §2）。

## 2. 会话 space

**左栏**：

- **搜索**：打字按名字筛；回车经 `harnix.search {query}` 搜对话内容（只搜最近的会话时会说），Esc 清掉。
- **分组**：问 `sidebar.group` 的注册，最后注册的那个生效。内建的是 `builtin.time`（「按时间」）：今天、近 7 天、更早，按最近活动排，
  空的组不画。应用注册自己的 `sidebar.group` 就换掉它。
- 每组先列 16 条，「显示更多 N」。归档的默认不列，「已归档 N」切换，记在偏好里（`harnix.prefs.set {key: "showArchived"}`）。
- 每行：在跑的点、名字（没有就是「未命名会话」）、多久之前。点了打开；右键或 `⋯` 是会话菜单（§5）。`⋯` 是行旁边的另一个按钮：
  Tab 从行走到它，回车打开菜单。

**数据**：`harnix.sessions {space: "sessions"}` 列会话，`harnix.workspaces.list` 列项目，外壳把两者按 `project_root` 拼起来
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
| 打一个名字 | `harnix.projects.create {name}` | 在项目根（`--data-dir`，缺省 `~/Documents`）下建这个目录并登记成工作区；名字只能是一段路径 |
| 选一个已有目录 | `harnix.workspaces.create {path}` | 目录要已经存在；同一个目录再加一次是幂等的 |
| 浏览目录 | `harnix.listDirectory {path?}` | 只在 `$HOME` 或项目根之内 |

**项目菜单**：在这个项目里新建会话（打开新会话对话框，§4）、改名（`harnix.workspaces.update {id, title}`）、移除分组
（`harnix.workspaces.remove`）。

## 4. 新建会话

- **左栏顶上的「新会话」**（⌘K）：不建会话，只打开新对话的空页。前台是系统 space、`center: "hidden"` 的 space 或嵌在别的
  space 里的 space 时，先换到第一个能对话的 space。⌘\ 收起左栏，⌘/ 聚焦输入框。对话框开着时这三个键都不生效。
- **窄窗口**（≤ 960 宽）：左栏是 56px 的轨，轨上的「会话」入口、收起键或 ⌘\ 把它作为抽屉盖在页面上打开；在抽屉里选了会话或 space、
  按 Esc、点幕布就收回。打开的右栏是从右边盖住中区的抽屉（`narrow_details = "beside"` 时 720–960 宽仍三栏并排）。宽窗口里人自己
  收起或展开的左栏，窗口变窄再变宽后照旧。
- **新对话的空页**：问候（按钟点是早上、下午、晚上）、这个 space 的 `subtitle()`、输入框、`suggestions()` 的建议（点了就建会话
  并发出）、`notice()`，下面是 `home.card` 的卡片。**第一次发出时才建会话**：`session.create`（带前台 space 的归属，项目 space 里带项目的
  目录），然后 `harnix.send`。Harnix app 的两个 space 不给建议、提示与卡片；应用的模块与插件 APP 可以给 `home.card`。
- **新会话对话框**（从项目菜单进来）：场景（启用的场景，引擎缺省的在前）、权限模式（`default`、`acceptEdits`、`plan`、
  `bypassPermissions`）、项目目录（`harnix.recentProjects` 给的最近目录，或不带项目），可以浏览目录。
- **换模型**：会话的模型菜单在有 `harnix.sessions.setModel`（`[shell] model_switch = true`）时用它，否则用系统 space 的
  `harnix.system.model`。

**`[shell] new_session = false`**：整个外壳不画「新会话」、⌘K 不归外壳（会话由应用自己建）。**只是界面**：空页上的输入框只看 space
自己的 `newSession`，所以空的 space 仍然给一个输入框；要一个 space 没有它，在那个 space 上写 `newSession: false`（这时画
`placeholder()`）。服务端照旧收 `session.create`。

## 5. 会话菜单

右键一行、`⋯`，或会话标题。它是在那里打开的菜单，不是对话框：右键时在指针处，`⋯` 与标题在按钮下面。焦点落在第一项，
↑ / ↓ 换项，回车选，Esc 或 Tab 关掉，焦点回到打开它的地方；点菜单外面也关。只有要再问一次的项（删除）或要填东西的项
（重命名、导出）才开对话框。项目菜单（§3）一样。

| 项 | 方法 | 说明 |
|---|---|---|
| 打开 | — | |
| 重命名 | `harnix.sessions.rename` | Harnix 账本里的标题，盖在引擎给的名字上 |
| 归档 / 取消归档 | `harnix.sessions.archive` | 只影响左栏列不列它 |
| 从这里分叉 | `session.fork` | |
| 导出 | `harnix.sessions.export` | Markdown，或引擎打的数据包（zip，可带录像） |
| 关闭 | `session.close` | 放掉运行时，历史留着，还能重开 |
| 删除 | `session.delete` | 先试删一次看连带几条侧链会话，确认后删；连同录像 |

两个 space 里的菜单一样：项目 space 列的是会话 space 的会话，所以菜单的调用都声明会话 space（`app.claim()`），不是前台的项目 space。

**层与提示**。外壳画在页面上面的东西是一个栈：对话框（对话框里再开的对话框叠在上面）、菜单、外壳替插件 APP 问的确认（总在最上面）。

- **焦点**：对话框打开时焦点移进去，Tab 只在里面转，关上后回到打开它的地方；它下面的页面与对话框是 inert 的。
- **Esc 只关最上面一层**：先关菜单，再关最上面的对话框；确认开着时 Esc 答「否」，页面别处一个键都收不到。
- **输入框里的 Esc**：先关弹层（`/`、`@`、＋、模型）；一轮在跑时，第一次 Esc 只提示「再按一次中断」，几秒内再按一次才中断。
- **横幅的提示排队**：一次显示一条，后面还有几条会写出来；普通的 6 秒后自己走，错误 12 秒，要重新加载页面这类状态一直在。
  横幅是播报区（错误 `role=alert`，其余 `role=status`）；对话里来了权限询问，也单独播报一句「需要批准：工具名」。

## 6. profile

### 6.1 `[shell]`

```toml
[shell]
spaces = ["sessions", "projects", "system", "flow"]   # 留哪些内建 space；不写是都留，[] 是一个不留
model_switch = false      # true：加 harnix.sessions.setModel，没有系统 space 也能在输入框换模型
new_session = true        # false：整个外壳不画「新会话」、⌘K 不归外壳；只是界面
narrow_details = "over"   # ≤ 960 宽时打开的右栏：over 是盖住中区的抽屉；beside 三栏并排（< 720 仍是抽屉）
```

`[shell]` 拒绝不认识的键；`spaces` 里写了不是内建 space 的名字，进程起不来并说是哪一个。

| `spaces` 里的名字 | 去掉之后 |
|---|---|
| `sessions` | 会话 space 的入口不注册；剩下的第一个（按次序）成为默认。后端的 `sessions` space 照样在：项目 space 和应用的代码仍然把会话建在那里 |
| `projects` | 项目 space 的入口不注册 |
| `system` | 系统 space 不注册，它的方法连同授权一起撤掉（[系统 space](system-space.md) §10） |
| `flow` | 流程的界面不注册；留 `flow` 去掉 `system` 时它是一个单独的流程 space（[系统 space](system-space.md) §5） |

一个都不留（`[]`）而应用也没有注册 space 时，外壳说「没有注册任何 space」。

### 6.2 `[brand]` 与目录

```toml
[brand]
name = "Harnix"           # 页面标题、左栏产品名；不写是 Harnix
icon = "brand/icon.svg"   # SVG，相对 profile；服务在 /brand/icon.svg
product_id = "harnix"     # 程序自己的目录 ~/.<product_id>；小写字母、数字、-，字母开头；不写是 harnix
```

Harnix app 的项目在 `--data-dir`（`$HARNIX_DATA_DIR`，缺省 `~/Documents`）里，工作区登记、标题、归档、偏好在 `~/.<product_id>/` 的账本里，
对话在引擎的目录（`~/.agix/`）里。

### 6.3 `harnix.hello` 里的 `shell`

外壳不读 profile，读握手之后 `harnix.hello` 的 `shell`：`spaces`、`brand {name, icon, product_id}`、`runtime_modules`、`new_session`、
`narrow_details`、`state_dir`、`projects_dir`、`user`。`model_switch` 不在里面：外壳看 `harnix.reachable` 里有没有
`harnix.sessions.setModel`。

## 7. 它是怎么用框架做的

Harnix app 是模式 1 最小的例子：一个后端模块、两个界面 space、一个贡献，用的都是二开应用也用的接口。

| 部分 | 用了什么 | 二开应用对应的做法 |
|---|---|---|
| 后端 | 内建模块 `sessions`：`harnix.workspaces.*`、`harnix.sessions.rename / archive / export`、`harnix.projects.create`、`harnix.listDirectory`、`harnix.recentProjects`、`harnix.files`、`harnix.search`、`harnix.prefs.set`、`harnix.upload.begin`；grant `session.export` | 一个 `Module` 的 `methods()`、`manifest().grant(…)`（[SDK](harnix-sdk.md) §5） |
| 后端 space | `sessions`，由 hub 自己声明，总在 | `spaces()` 里的 `SpaceDecl`（[SDK](harnix-sdk.md) §8.2） |
| 界面 space | 两个 `space` 注册，owner `builtin`；项目 space 写 `sessions: "sessions"` | `r.register('space', …)`（[SDK](harnix-sdk.md) §8.2） |
| 分组 | `sidebar.group` 的 `builtin.time` | 注册自己的 `sidebar.group` 换掉它 |
| 数据 | 方法 `harnix.sessions`，主题 `harnix.sessions.status` | `useNest()` 的 `call` 与 `subscribe`（[SDK](harnix-sdk.md) §22.4） |

内建模块只用公开的模块 API，由 `crates/harnix/tests/layering.rs` 的 `the_builtins_use_only_the_public_module_api` 盯着：`builtin` 不许
依赖 hub、authz、instance、app，也不许用框架内部的构造。界面一侧，外壳只从 `@harnix/react` 与 `@harnix/core` 的公开导出取东西
（两个包的 `exports` 不放别的路径），space 经公开的注册表注册；但两个左栏是外壳自己的组件，用的是外壳内部的状态对象，
不是模块拿到的 `NestHost`，这一侧也没有测试盯着「只用公开 API」。

**要一个不同的 Harnix app**：不 fork 外壳，在自己的产品里用 `[shell] spaces` 去掉这两个 space，注册自己的 space 顶替
（[SDK](harnix-sdk.md) §4、§8）。

**它的后端随框架一起链进来，只能藏、不能去。** 会话 space 的后端是自带模块（`with_builtins()`）里 `sessions` 那一组，和设置、
Agix 插件、录像同在一处，全要或全不要；`[shell] spaces` 去掉的只是界面。所以一个想要自己会话模型的产品（比如不要「工作区 = 目录」）
仍然扛着 `harnix.workspaces.*`、`harnix.projects.create`、`harnix.listDirectory`，它们对设备可见（`harnix.reachable` 里有）。
把这一组拆成可以不注册的模块是以后的里程碑。
