# Harnix 术语

一个词只有一个意思。文档、代码、讨论都照这张表用词；要用表里没有的词，先加进来。
标「代码」的一栏是它在代码、协议、配置里的写法。

有争议、容易混的词放在每节末尾的「别混」里。

---

## 1. 整体

| 术语 | 代码 | 意思 |
|---|---|---|
| **Agix**（原 AttaCore） | `agix/` | Agent 引擎。会话怎么跑、工具怎么派发、上下文怎么压缩，都是它的事 |
| **Harnix**（原 Nest） | `harnix` | Agix 应用的公共底座：中枢、授权、传输、装配、外壳 |
| **应用** | app | 在 Harnix 上做出来的产品，例如 Helm、IPDrun。一个应用 = 一个二进制 + 一份界面产物 |
| **空应用** | `harnix` 二进制 | 只带内建模块的应用，Harnix 自己发布的那个 |
| **二开** | — | 在 Harnix 上做应用，不 fork Harnix |
| **骨架** | — | 传输、授权、中枢、装配四件事。写死，不可插拔 |
| **profile** | `profiles/*.toml` | 一份声明式配置：场景、provider、界面、拓扑、实例、模块配置段 |
| **账本** | `~/.<product_id>/` | 程序自己的持久状态：设备、工作区、会话与 space 的绑定、按模板建的实例。不是会话内容。默认 `~/.harnix/` |
| **产品 id** | `[brand] product_id` | 决定程序自己的目录 `~/.<product_id>`；同一台机器上的两个产品不共用令牌和账本 |
| **品牌** | `[brand]` | 产品名、图标、产品 id；配色和文案由模块的令牌覆盖与语言包给 |

## 2. 运行时四层

| 术语 | 代码 | 意思 |
|---|---|---|
| **实例** | `instance` | 一台 Agix 引擎（一个会话池）。**隔离单位**：数据根、设置、provider、MCP、插件、场景都是它自己的 |
| **`system` 实例** | `system` | profile 的 `[engine]` 建的默认实例 |
| **载体** | `carrier` | 实例怎么跑：**进程内**（in-process，作为库链接）或 **daemon**（本机另一个进程，JSON-RPC） |
| **spawn / attach** | — | daemon 实例的两种来法：Harnix 起子进程并看护 / 连一个已经在跑、只给这个 Harnix 用的 daemon |
| **模板** | `InstanceTemplate` | 建实例的规格：载体、场景、数据根规则、允许填的参数。模块和界面都只能按模板建实例 |
| **拥有者** | `owner` | 实例归谁：`kernel`、`module:<id>`、`space:<id>` |
| **会话** | `session`，全名 `(instance, session)` | 实例里的一条轮次循环，同一时刻至多一轮。创建时定了场景，场景决定工具集 |
| **任务** | `task`，全名 `(instance, session, task)` | 属于某个会话、在会话之外继续跑的工作：子 Agent、团队成员、后台 Shell、定时任务与唤醒、监视器、dream。是 Agix 的东西，Harnix 只看、只转发停止 |
| **轮** | turn | 一次「用户消息 → 模型若干步 → 结束」 |
| **步** | step | 一轮里的一次模型调用及其工具执行 |
| **场景** | scene | Agix 的概念：一套系统提示词与工具集。会话创建时选定 |

**别混**：

- 「实例」**只**指 Agix 实例。space 左栏里用户新建的东西叫**条目**，不叫实例。
- 「子 Agent」是一种**任务**，不是实例，也不是顶层会话。
- 「Agent」单独出现时指「space 的 Agent 部分」（§4）或泛指模型侧行为，不指实例。

## 3. 中枢与通道

| 术语 | 代码 | 意思 |
|---|---|---|
| **中枢** | `harnix-hub` | 会话的持有者：订阅、缓冲、turn、队列。不变量：**中枢永远比任何客户端先订阅** |
| **观看者** | watcher | 正在看某个会话的一方：客户端，或服务端的 tap |
| **客户端** | client | 连上 Harnix 的浏览器、Tauri 窗口、测试客户端 |
| **服务端调用方** | — | 内核或模块，不经客户端驱动会话。与客户端同语义 |
| **无头会话** | headless session | 由服务端调用方创建、驱动的会话。界面随时可以 attach 旁观 |
| **tap** | `tap(session)` | 服务端的观看者。有界、按 `seq`；落后了收到 `Lagged` 再追赶 |
| **OnBusy** | `Queue / Reject / Interrupt / Steer` | 会话正忙时一条新消息怎么处理 |
| **steer** | — | 在运行中的一轮的步骤边界把消息送进去，不中断 |
| **seq** | `seq` | 中枢按会话分配的帧序号，追赶靠它 |
| **追赶** | catch_up | 观看者从中枢的缓冲里补齐它没收到的帧 |
| **resync** | `harnix.session_resync` | 中枢重建一个会话的缓冲后，通知客户端照刷新那条路重新 attach |
| **结算** | settle，`harnix.turn_settled` | 中枢认定一轮结束，清掉这一轮的状态 |
| **EngineLink** | `EngineLink` | 中枢与一台实例之间的接口。两种载体都实现它；不对模块开放 |
| **降级** | `Degraded` | daemon 实例的链接断了，那边的轮照跑，Harnix 在重连 |
| **通道语义** | — | 五种：unary、宿主事件流、会话事件流、带外应答、大负载旁路。写死 |
| **拓扑** | topology | 用几条连接承载通道语义：单双向流、分离流、纯请求。可选 |
| **宿主主题** | host topic | 宿主事件流上的一个主题，推**整值快照**，可带订阅参数 |
| **整值快照** | — | 推完整状态，客户端不做增量合并 |

## 4. 应用的组成

| 术语 | 代码 | 意思 |
|---|---|---|
| **模块** | `Module` | 编译期链接进来的 Rust 代码，实现 `Module` trait。内核级信任。内建功能也是模块（**内建模块**） |
| **插件** | plugin | **只**指 Agix 的包（WASM / QuickJS / manifest）。Harnix 没有自己的插件体系 |
| **运行期包** | package | 安装时进来的 Agix 插件包。可以带 Agent 段（引擎读）和 `[[ui]]` 段（Harnix 读） |
| **界面模块** | `[[ui]]` | 运行期包里给浏览器加载的预编译 ES 模块（贡献点 API 2：从页面的 import map 拿 React）。页面级信任；`[ui] runtime_modules = false` 时一律不加载 |
| **space** | `space` | 左栏（`sidebar()`）+ 中区（默认是对话，`center`）+ 右栏（`tabs`）合起来，原子注册。会话按 space 绑定 |
| **部分** | — | 一个 space 的三个组成：**前端部分**（React）、**后端部分**（一个模块）、**Agent 部分**（原生 Rust 贡献或运行期包）。任意组合 |
| **条目** | entry | space 左栏里用户新建的东西，例如项目 space 的项目（工作区）、Helm 的关注组。存在账本里 |
| **前台 space** | — | 当前显示的 space。中区、输入框、右栏只操作它自己的会话 |
| **默认实例** | `instance = …` | space 里新建会话默认落在哪台实例 |
| **内建 space** | `[shell] spaces` | 会话 space（按时间）、项目 space（按项目，列的是会话 space 的会话）、系统 space。都是用例，产品决定留哪些 |
| **固定部分** | — | 外壳里不随 space 变的东西：顶部的标识、新建对话、搜索，底部的头像（设置入口，外圈表示连接）。其余都是 space 的 |
| **令牌覆盖** | `r.tokens` | 模块改主题已有令牌的值；不能新增令牌名 |
| **语言包** | `r.strings` | 模块带的文案；key 以模块 id 开头 |
| **系统 space** | — | 调试面：按实例 → 会话 → 任务看一切、能操作。可以观看与操作任何 space 的会话 |
| **贡献点** | contribution point | 前端的接缝。v2 共 8 个：`space`、`home.card`、`tool.row`、`flow.block`、`details.panel`、`sidebar.group`、`command`、`settings.section` |
| **宿主 API** | `useNest()` | 前端代码拿到的全部能力。拿不到 Harnix 自己的连接 |
| **原生贡献** | `EngineContrib` | 模块交给进程内实例的东西：场景、原生工具、`SessionCustomizer`、凭据源 |
| **自有链接** | `/apps/<id>/` | 模块自己的 HTTP / WS 路由。同源、同一套授权，但不享受 Harnix 通道的保证 |
| **扩展条目** | extension entry | 模块往会话时间线写的、只给人看的条目，由 `flow.block` 画出来 |

**别混**：

- 「模块」只指 Rust 的 `Module`。space 的三个组成叫「部分」，运行期包里的界面段叫「界面模块」。
- 「插件」只指 Agix 的包。说 Harnix 的东西不用这个词。
- 「功能模块」是开发计划的单位（设计文档 §4.3 的一行），不是代码里的东西。

## 5. 授权与安全

| 术语 | 代码 | 意思 |
|---|---|---|
| **主体** | principal | 授权判定的「谁」：`Kernel`、`Module(id)`、`Device` |
| **设备** | `Device` | 一个配对过的客户端，或本机带 token 的页面。代表一个人 |
| **授权表** | — | 主体 × 方法 / 路由，默认拒绝。Harnix 唯一的准入点 |
| **透传白名单** | `PASSTHROUGH` | 设备可以经 Harnix 调到的 Agix 方法 |
| **披露** | disclosure | 装包或启用模块时给人看的「它能做什么」 |
| **安全边界** | — | 对恶意代码也成立的约束 |
| **防误** | — | 只防 bug、不防恶意的约束。文档里必须明说是这一种 |

## 6. 构建、测试、版本

| 术语 | 代码 | 意思 |
|---|---|---|
| **子仓** | submodule | `agix/`（Agix）、`harnix/`（Harnix）。应用里两者平级 |
| **钉** | pin | 子仓指针指向的提交。Harnix 钉一个 Agix；应用钉自己的 Agix 和 Harnix |
| **版本标识** | `Harnix-v<x.y.z>-<YYYYMMDD><NN>` | 发布的标签名，见 CLAUDE.md |
| **模块 API 版本** | `MODULE_API_VERSION` | `Module` trait 的版本。不匹配编译失败 |
| **贡献点 API 版本** | `[[ui]] api_version` | 界面模块照哪一版贡献点写的 |
| **LIVE 测试** | `tests/live/` | 对着真的 `harnix` 二进制跑的功能测试：API 层和 Playwright 层 |
| **录像 / 回放** | recording / replay | 引擎记下的模型调用；测试默认按位置重放 |
| **live 模式** | `--live` | 连真实模型服务跑测试，可以写回录像 |
| **里程碑** | M0…M9 | 见 roadmap.md。任务编号写成 `M3-4` |
