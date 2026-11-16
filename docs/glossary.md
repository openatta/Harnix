# Harnix 术语

一个词只有一个意思。文档、代码、讨论都照这张表用词；要用表里没有的词，先加进来。
标「代码」的一栏是它在代码、协议、配置里的写法。

有争议、容易混的词放在每节末尾的「别混」里。

---

## 1. 整体

| 术语 | 代码 | 意思 |
|---|---|---|
| **Agix** | `agix/` | Agent 引擎。会话怎么跑、工具怎么派发、上下文怎么压缩，都是它的事。Harnix 带着它；应用只经 `harnix::engine`（Harnix 自己的引擎词汇）用到它，问题与需求提给 Harnix |
| **Harnix** | `harnix` | Agix 应用的公共底座：**框架 + 第一方扩展**。Harnix 自己发布的二进制 `harnix` 是框架加上全部第一方扩展、`packages` 与两个载体，缺省跑模式 2 |
| **框架** | `harnix` crate、`@harnix/core`、`@harnix/react`、`@harnix/shell` | 骨架、实例层、扩展接口与扩展宿主、载体、外壳核心。应用在它上面做 |
| **骨架** | — | 传输、路由与准入、中枢、装配四件事。写死，不可插拔 |
| **应用** | `App::builder()` | 在 Harnix 上做出来的产品，例如 Helm、IPDrun。一个应用 = 一个二进制 + 一份界面产物 |
| **Harnix app** | 扩展 `harnix-app`、UI 包 `@harnix/harnix-app` | Harnix 自带的一个第一方扩展：会话 space、项目 space，新建会话、按时间看、按项目管理。不叫「最小应用」。见 [Harnix app](harnix-app.md) |
| **二开** | — | 在 Harnix 上做应用，不 fork Harnix |
| **profile** | `profiles/*.toml` | 一份声明式配置：引擎、实例、拓扑、品牌、各扩展的配置节。每一节有一个所有者，没有所有者的节、不认识的键都拒绝启动 |
| **状态目录** / **账本** | `~/.<product_id>/` | 程序自己的持久状态：令牌、设备、会话与 space 的绑定、实例账本、各扩展的数据与扩展包。不是会话内容。缺省 `~/.harnix/`，`HARNIX_STATE_DIR` 能挪 |
| **产品 id** | `[brand] product_id` | 决定状态目录 `~/.<product_id>`；同一台机器上的两个产品不共用令牌和账本 |
| **品牌** | `[brand]` | 产品名、图标、产品 id；配色和文案由扩展的令牌覆盖与语言包给 |

**名字里的 `app` 与 `ext`**：代码与路径里的 `app` 只剩两个意思，`ext` 只指「扩展」这个概念。

| 名字 | 指什么 |
|---|---|
| 应用（产品）、`App`、`AppBuilder` | 在 Harnix 上做出来的产品与把它装起来的入口（`harnix-assembly`，经门面 `harnix` 用） |
| `harnix-app`（扩展，`crates/ext/harnix-app`）、`harnix-app.*`、`@harnix/harnix-app` | Harnix app：它的后端扩展、方法、UI 包 |
| 二进制 `harnix`（crate `harnix-cli`） | Harnix 自己发布的产品 |
| `harnix-ext`、`harnix-ext-<id>`、`harnix-host-api` | 扩展接口、第一方扩展、宿主组件接口这几个 crate |
| `harnix-ext.toml`、`.hxext` | 扩展包的清单与包 |
| `/ext/<id>/…` | 一个扩展的 HTTP 路由、静态文件 |
| `ext__<id>__<工具>` | 扩展包的工具在会话里的名字（id 里的 `.` 换成 `_`） |
| `ext-proc` | 产品二进制拉起扩展进程用的子命令 |
| `harnix-pack` | 扩展包作者与商店用的打包工具 |

## 2. 运行时四层

| 术语 | 代码 | 意思 |
|---|---|---|
| **实例** | `instance` | 一台 Agix 引擎（一个会话池）。**隔离单位**：数据根、设置、provider、MCP、Agix 插件、场景都是它自己的 |
| **`main` 实例** | `main` | profile 的 `[engine]` 建的缺省实例，内核拥有，谁都不能停 |
| **实例载体** | `InstanceCarrier` | 一台实例怎么跑：**进程内**（经 `agix::embed` 起在本进程里，经内存双工连）、**spawn**（Harnix 起子进程并看护）、**attach**（连一台只给这个 Harnix 用的 daemon）、**收养**（扩展进程里的引擎） |
| **实例登记表** | `harnix-instance` | 建、确保、停、重启实例，模板、账本、拥有者都在这里；中枢只经 `InstanceBook` 问它 |
| **模板** | `InstanceTemplate` | 建实例的规格：实例载体、场景、数据根规则、允许填的参数、能不能由界面用。扩展与界面都只能按模板建实例 |
| **拥有者**（实例的） | `owner` | 实例归谁：`kernel`、某个扩展、某个 space（界面建的）。跨拥有者停、重启要能力 `instances.admin` |
| **实例的信任** | `InstanceTrust` | `Product`（产品自己拉起或连上的）或 `Extension`（扩展进程里的引擎，第三方代码控制）。中枢对后者设限、外壳把它的会话当不受信内容画 |
| **收养的实例** | `proc.<扩展 id>` | 模式 3 里扩展进程自己的引擎，由产品收养进实例登记表。不是模板建的，一个扩展至多一台 |
| **会话** | `session`，全名 `(instance, session)` | 实例里的一条轮次循环，同一时刻至多一轮。创建时定了场景，场景决定工具集 |
| **任务** | `task`，全名 `(instance, session, task)` | 属于某个会话、在会话之外继续跑的工作：子 Agent、团队成员、后台 Shell、定时任务与唤醒、监视器、dream。是 Agix 的东西，Harnix 只看、只转发停止 |
| **轮** | turn | 一次「用户消息 → 模型若干步 → 结束」 |
| **步** | step | 一轮里的一次模型调用及其工具执行 |
| **场景** | scene | Agix 的概念：一套系统提示词与工具集。会话创建时选定 |
| **场景层** | `SceneLayer` | 原生扩展包装一个场景的写法：只说要改的几样（追加的系统块、工具、交互方式、是否要项目、命名），其余照底版；由中枢在建会话时展开 |

**别混**：

- 「实例」**只**指 Agix 实例。space 左栏里用户新建的东西叫**条目**；虚拟机里的是 **Store**（§4）；流程的一次执行叫**运行**（§7）。都不叫实例。
- 「载体」要说清是哪一种：**实例载体**是一台引擎怎么跑，**扩展载体**是一个扩展怎么跑（§4）。
- 「子 Agent」是一种**任务**，不是实例，也不是顶层会话。
- 「Agent」单独出现时指「space 的 Agent 部分」（§4）或泛指模型侧行为，不指实例。

## 3. 中枢与通道

| 术语 | 代码 | 意思 |
|---|---|---|
| **中枢** | `harnix-hub` | 会话的持有者：订阅、缓冲、turn、队列、会话与 space 的绑定。不变量：**中枢永远比任何客户端先订阅** |
| **观看者** | watcher | 正在看某个会话的一方：客户端，或服务端的 tap |
| **客户端** | client | 连上 Harnix 的浏览器、Tauri 窗口、测试客户端 |
| **服务端调用方** | — | 内核或扩展，不经客户端驱动会话。与客户端同语义 |
| **无头会话** | headless session | 由服务端调用方创建、驱动的会话。界面随时可以 attach 旁观 |
| **tap** | `cx.sessions().tap(..)` | 服务端的观看者。有界、按 `seq`；落后了收到 `Lagged` 再追赶 |
| **OnBusy** | `Queue / Reject / Interrupt / Steer` | 会话正忙时一条新消息怎么处理 |
| **steer** | — | 在运行中的一轮的步骤边界把消息送进去，不中断 |
| **seq** | `seq` | 中枢按会话分配的帧序号，追赶靠它 |
| **追赶** | catch_up | 观看者从中枢的缓冲里补齐它没收到的帧 |
| **resync** | `harnix.session_resync` | 中枢重建一个会话的缓冲后，通知客户端照刷新那条路重新 attach |
| **结算** | settle，`harnix.turn_settled` | 中枢认定一轮结束，清掉这一轮的状态 |
| **引擎链接** | `EngineLink`、`EngineOps`、`EngineEvent` | 中枢与一台实例之间有类型的接口：一样操作一个方法，事件带中枢要用的类型。每种实例载体都实现它；只交给中枢，扩展拿不到 |
| **降级** | `Degraded` | 实例的链接断了或在刷屏：那边的轮照跑，Harnix 在重连或在丢帧。收养的实例在它的扩展进程没了时也是降级，等下一次运行 |
| **通道语义** | — | 五种：unary、宿主事件流、会话事件流、带外应答、大负载旁路。写死 |
| **拓扑** | topology | 用几条连接承载通道语义：单双向流、分离流、纯请求。可选 |
| **主题** | topic | 宿主事件流上的一个主题，推**整值快照**，可带订阅参数。内核的在 `harnix.*` 下，扩展的在 `<扩展 id>.*` 下 |
| **整值快照** | — | 推完整状态，客户端不做增量合并 |

## 4. 扩展

| 术语 | 代码 | 意思 |
|---|---|---|
| **扩展** | `Extension`、`ExtensionDescriptor` | 往一个 Harnix 产品里登记方法、主题、space、界面、工具的东西。**一个描述，三种载体**；只有一个概念，区别只在载体 |
| **扩展载体** | — | 一个扩展怎么跑：**原生** · **WASM 同进程**（模式 2）· **WASM 进程**（模式 3） |
| **原生扩展** | `impl Extension` | Rust 写、编进二进制的扩展，产品在 `main.rs` 里列出并授予能力。信任来自出身 |
| **WASM 扩展** | — | 一个装上的扩展包所成的扩展，跑在同进程（模式 2）或它自己的进程（模式 3）。能力来自同意记录 |
| **第一方扩展** | `harnix-ext-<id>`、`harnix::first_party` | Harnix 自己带的原生扩展：`system`、`harnix-app`、`settings`、`plugins`、`observability`、`devices`、`flow`。**是出处，不是特权**：它们的权力就是被授予的那几项能力 |
| **宿主组件** | `packages`、`harnix-carrier-wasm`、`harnix-carrier-proc` | 写成扩展的形状、但要碰宿主管道的那几个，凭能力 `extensions.manage` 拿 `harnix-host-api` 的服务 |
| **`packages`** | `harnix-ext-packages` | 管扩展包的宿主组件：安装记录、商店、披露与同意、升级与回滚、启停、卸载，按名字选载体，把每个启用的包登记成扩展 |
| **扩展宿主** | `harnix-assembly` 的 `host` | 骨架里登记、迁移、启动、停用扩展，查授予，把描述装进路由表、中枢、传输层与界面板的那部分 |
| **模式 1 / 2 / 3** | 模式 2：feature `wasm`；模式 3：feature `proc` + `[ext.packages] carrier = "process"` | 在 Harnix 上做应用的三种模式，产品都是 Rust 写的：**模式 1** 纯 Rust 应用，不装扩展包；**模式 2** Rust 加同进程的扩展，扩展包在产品进程里、每个包一台 WASM 虚拟机，**是缺省**；**模式 3** Rust 加每个扩展一个进程，每个扩展包一个关在操作系统沙箱里的扩展进程，给安全要求高的场景，**不是缺省**。见 [应用开发](app-development.md) |
| **扩展包** | `.hxext` | WASM 扩展的分发单位：清单 `harnix-ext.toml`、预编译的 `.cwasm`、`agent/`（一个 Agix 插件）、`ui/`、`SIGNATURES`。对载体中立：同一个包在模式 2、3 里都能装 |
| **清单** | `harnix-ext.toml` | 扩展包是什么、要什么能力、能调什么、有哪些方法、主题、space、工具（`[[tool]]`）、面与行内点。严格读，未知的节与字段按名拒；也是装前给人看的披露 |
| **扩展进程** | `<产品> ext-proc` | 模式 3 里一个扩展包的进程：产品二进制以子命令重新执行自己。它自己的 Agix 与 WASM 在里面是完整的，宿主只路由；关在沙箱里 |
| **沙箱** | `[ext.packages] sandbox` | 扩展进程所在的操作系统沙箱（macOS `sandbox-exec`、Linux Landlock），由产品在 exec 之前套上：只看得见自己的运行目录与包。**不挡网络**。是唯一的边界：沙箱开着时 Agix 自己的命令沙箱在里面关掉 |
| **扩展线** | `harnix-ext-wire@1` | 扩展进程与产品之间、继承的 socketpair 上的 JSON-RPC。进程发的方法是 `proc.*`，产品发的是 `ctl.*` |
| **监护器** | `harnix-supervise` | 产品侧拉起、看护、退避、回收子进程（daemon 与扩展进程）的那部分 |
| **虚拟机** | `harnix-wasm` | 一个扩展包的那一台 WASM「虚拟机」：一个边界，不是一个进程。里面是常驻 Store 与每次工具调用、每次事件的调用 Store |
| **常驻 Store** / **调用 Store** | wasmtime 的 `Store` | 组件跑在里面的那份内存与状态。常驻 Store 是后端：长期存活，状态、订阅、定时器在这里，方法调用排队串行进入；超时、越过配额时换一个新的。调用 Store 每次工具调用、每次事件新建一个，用完即弃 |
| **宿主核心** | `harnix-wasm-host` | 两种 WASM 载体共用的那一份：生命周期、注册表、健康、分派、休眠。模式 2 在产品进程里，模式 3 在扩展进程里 |
| **信任等级** | `store` / `trusted` / `unsigned` | 扩展包由签名定的出身：商店副署、受信发布者签全、未签名（只在开发模式） |
| **休眠** | `hibernating`，`[backend] resident = "on-demand"` | `on-demand` 的扩展包闲置、没有东西拿着它时卸下后端（模式 3 连进程一起停），下一次要它做事时再起。不是故障 |
| **置于一旁** | `faulted` | 扩展包的后端或进程反复崩溃，不再拉起；行留在表里，答 `EXTENSION_UNAVAILABLE`。停用再启用会重来 |
| **Agix 插件** | `plugin.toml`，`plugins` 扩展 | Agix 的包：QuickJS 脚本、skill、agent 类型、manifest，装进实例、由引擎判能力与披露。可以带 `[[ui]]` 段（Harnix 读）。扩展包的 `agent/` 就是一个 |
| **界面模块** | Agix 插件的 `[[ui]]`；扩展的 `ui.modules` | 给浏览器加载的预编译 ES 模块，从页面的 import map 拿 React。与页面同级；扩展包只有 `trusted` 发布者才能带 |
| **UI 包** | `@harnix/<扩展 id>` | 编译进外壳产物的一个原生扩展的界面，导出 `extension` 与 `register(r, host)`；外壳按 `hello` 只激活启用了的扩展的包 |
| **外壳核心** | `@harnix/shell` | 每个产品都要的那部分界面：外框、space 切换、对话区、会话菜单、设置面板的框、扩展包界面的运行时。不认识任何第一方 UI 包 |
| **面** | surface，`[[surface]]` | 外壳的一块区域，扩展包的页面画在里面的一个隔离 iframe 里 |
| **行内点** | `[inline]` | 扩展包在外壳自己的视图里声明的样子：工具行、流块卡片、命令、侧栏分组。外壳照声明画，不跑包里的代码 |
| **桥** | `harnix-ui-bridge@2`，`@harnix/ext-sdk` | 扩展包的 iframe 与外壳之间的协议：一个 `MessageChannel` 端口上的请求、应答与事件。iframe 只能经它请外壳去做；它的调用在服务端可辨 |
| **space** | `space` | 总称：一块产品面。分两半，各有各的声明，用同一个 id 对上；单说 space 时看上下文是哪一半，要分清就说下面两个词 |
| **界面 space** | 贡献点 `space` | 外壳里的那块：左栏 + 中区（缺省是对话）+ 右栏，原子注册 |
| **后端 space** | `SpaceDecl`、清单的 `[[space]]`、`harnix.spaces.list`、`SPACE_MISMATCH` | 会话的归属：在它里面建的会话落在哪台实例、用哪个场景，会话永远属于建它的 space。由扩展声明，内核不认识任何 space id |
| **缺省 space** | `SpaceDecl::default_space()` | 不声称 space 的设备调用按它检查的 space；一个进程至多一个（Harnix app 的 `sessions`） |
| **跨 space 的 space** | `SpaceDecl::cross_space()` | 声称它的设备调用可以观看、操作任何 space 的会话。**防误的标志，不是能力**；只有原生扩展能声明（系统 space） |
| **部分** | — | 一个 space 的三个组成：**前端部分**（UI 包，或扩展包的面）、**后端部分**（一个扩展）、**Agent 部分**（引擎贡献或 Agix 插件）。任意组合 |
| **条目** | entry | space 左栏里用户新建的东西，例如项目 space 的项目（工作区）、Helm 的关注组。存在拥有它的扩展的数据里 |
| **前台 space** | — | 当前显示的 space。中区、输入框、右栏只操作它自己的会话 |
| **space 的缺省实例** | `SpaceDecl::instance(..)` | space 里新建会话缺省落在哪台实例：`main`、一台固定实例、或 `template:<id>` |
| **系统 space** | 扩展 `system`、`@harnix/system` | 第一方扩展：开发调测用的面，按实例 → 会话 → 任务看一切、能操作，可以观看与操作任何 space 的会话。见 [系统 space](system-space.md) |
| **固定部分** | — | 外壳里不随 space 变的东西：顶部的标识、新建对话、搜索，底部的头像（设置入口，外圈表示连接）。其余都是 space 的 |
| **贡献点** | contribution point | 前端的接缝。**内核点** 11 个：`space`、`home.card`、`tool.row`、`flow.block`、`details.panel`、`sidebar.group`、`command`、`settings.section`、`session.action`、`sidebar.tab`、`shortcut`。外加扩展定义的点 |
| **扩展定义的贡献点** | `r.definePoint("<id>.<名>", ..)` | 一个原生扩展的界面定义、别的扩展往里注册的点，例如 `system.console.tab`。不算进 11 个 |
| **宿主 API** | `HostApi`、`useHost()` | 前端代码拿到的全部能力。拿不到 Harnix 自己的连接 |
| **停靠面** | `SurfaceState`，`harnix.sidebar.v1.<会话>` | 一个会话在右栏里的那一份：开着的页签、哪一页在前、布局树（本版一格）。存在浏览器本地，刷新后还在；会话之间互不影响 |
| **页签类型** | 贡献点 `sidebar.tab` | 右栏能打开的一种页：资源类按 `patterns` 认领资源地址，页类按 `kind` 打开。一个 `kind` 至多一份 extension 加一份 builtin，extension 生效 |
| **资源地址** | `harnix-resource://<类型>/<路径>` | 右栏打开的一个资源的名字，由页签类型认领：按优先级档、模式长度、注册顺序 |
| **导航控制器** | `host.sidebar` | 进右栏的每条路：`openResource`、`openTab`、`close`、`active`、`isExpanded`、`toggleExpanded`。同一内容只开一份 |
| **页签域** | `useTabInfo()` | 一个打开的页签的正文拿到的东西：它是哪一页、怎么被导航到的（`params`、`revision`）、`signal`、它能做的动作、关闭处理器 |
| **引导页** | `kind: "guide"` | 停靠面的缺省页：列出各页签类型 `guide` 入口的卡片；恰好一个入口时直接开那一页。外壳的 builtin，扩展可以顶替 |
| **令牌覆盖** | `r.tokens` | 扩展改主题已有令牌的值；不能新增令牌名 |
| **语言包** | `r.strings` | 扩展带的文案；key 以它的 owner 开头 |
| **引擎贡献** | `EngineContribution`，`Registrar::engine()` / `engine_everywhere()` | 原生扩展交给进程内实例的东西：场景层、原生工具（`Tool`、`ToolProvider`）、`PromptGate`。扩展包没有 |
| **扩展的 HTTP 路由** | `Registrar::route`、`static_dir`，`/ext/<id>/` | 原生扩展自己的 HTTP 路由与静态文件。同源、同一个准入，但不享受 Harnix 通道的保证 |
| **扩展条目** | extension entry | 扩展往会话时间线写的、只给人看的条目，由 `flow.block` 画出来；命名空间是写它的扩展的 id |
| **反馈** | `harnix.session.feedback`，条目 `harnix.feedback` / `rating` | 人对一轮回答的赞或踩，可带类别和备注。中枢记成会话日志里的条目，只给人看，同一轮后来的盖过先前的；不往任何地方送，持 `feedback.read` 的扩展才收得到 |

**别混**：

- 「扩展」是 Harnix 的：原生扩展、WASM 扩展都是它。Agix 的包说「**Agix 插件**」，不单说「插件」；第一方扩展 `plugins` 管的就是 Agix 插件。
- 「扩展包」是分发单位（一个 `.hxext` 文件），「WASM 扩展」是它装上之后在内核里的那个扩展。说运行时的事说扩展，说装、签名、商店说扩展包。
- 「第一方」只说出处。系统 space 能做的，任何被授予同样能力、声明同样 space 的扩展都能做。
- 「宿主组件」也是扩展，经同一个登记口、同一套授予进来；它们多拿的只有 `harnix-host-api` 那几样，凭的是 `extensions.manage`。
- 「Harnix app」是一个第一方扩展；它和 Harnix 自己发布的二进制 `harnix` 不是一回事，和装配（`harnix-assembly`）也不是一回事。
- space 的三个组成叫「部分」，Agix 插件里的界面段叫「界面模块」，编进外壳的扩展界面叫「UI 包」。
- 「功能模块」是 LIVE 覆盖表的一行（`tests/live/features.mjs`），不是代码里的东西。
- 产品的形状只按**模式 1 / 2 / 3** 说，不另起「某某版」这类产品版本的名字。桌面壳（`templates/desktop`）只是装界面的窗口，和浏览器里是同一个产品，不是一种模式。

## 5. 调用、能力与安全

| 术语 | 代码 | 意思 |
|---|---|---|
| **主体** | `Subject` | 一次调用的调用方，四种：`kernel`（本二进制）、`device:<id>`（一个人）、`ext:<id>`（一个扩展以自己的身份）、`engine:<实例>`（一个实例的引擎，审计记它做的事，如扩展的引擎发得太多；没有哪一行对它开）。审计照这个写法记 |
| **设备** | `Device` | 一个配对过的客户端，或本机带令牌的页面。代表一个人。**设备永远不持有能力** |
| **准入** | `harnix-authz`（`Devices`、`ChildBindings`） | 唯一的准入点：设备、本机令牌、启动链接、扩展进程的绑定，状态都在这里，一处发、撤、审计 |
| **路由表** | `harnix-router` | 方法、主题（`topic:<名>`）、路由（`route:<扩展 id>`）都是表里的一行：所有者、处理者、谁能用、要什么能力、范围。一次调用判定一次 |
| **所有者**（名字的） | `Owner` | 已登记的、与一个名字整段前缀匹配的最长 id：内核（`harnix`）、引擎（`engine`）或某个扩展。只有所有者能登记它名下的名字 |
| **访问** | `Access` | 一行对谁开放：只对设备（`Access::device()`，人经界面做的决定）；设备与扩展（`.with_extensions()`）；只对扩展；某一个扩展；持有某项能力的扩展；引擎主体；或谁都不行（中枢独占） |
| **范围** | `RowScope` | 一行作用在什么上：`Open`（与 space、会话无关）、`Space`、`Session`。扩展包只在自己的 space 与会话上（它引擎自己建的子会话随父会话的 space） |
| **调用上下文** | `CallContext`；线上是 `CallMeta` | 一次调用随身带的：主体、客户端、调用 id、父调用、调用链、取消、截止时间 |
| **调用链** | `chain` | 正等着这次调用的扩展与它们被占着的那次调用。宿主核心据此当场拒绝重入，而不是等到超时 |
| **能力** | `Capability` | 一项有名字的特权。**内核目录**一份（`instances.admin`、`engine.inspect`……），原生扩展可以声明自己的（`<id>.<名>`） |
| **授予** | `.grant([..])` | 产品在装配时给一个原生扩展的能力；违反启动检查就拒绝启动 |
| **同意记录** | `ext/<id>/install.json` | 装扩展包时人同意了的：清单的摘要、能力、能调的名字（`calls`）、工具。扩展包的权力只从这里来 |
| **信任** | `Trust` | `Origin`（原生：出身即信任，按每行的访问判）或 `Consent`（扩展包：只到同意的 `calls`，再收窄到自己的范围） |
| **受信项目** | `engine.project.trust`；`harnix-app.projects.trust` | 引擎的概念：项目根受信了，项目自己的设置（hook、MCP 服务器、更宽的权限与沙箱、provider、agent 类型、带命令的 skill）才生效；缺省不受信，只能收紧，放不开的叫「被扣下」。记在实例全局层的 `settings.local.json`。界面上的动作是「信任这个项目」「不再信任这个项目」，见 [Harnix app](harnix-app.md) §3.1 |
| **引擎直通** | `engine.*` | 经中枢转给引擎的方法：每一条是表里显式的一行，有自己的访问与范围 |
| **中枢独占** | `hub_owned` | 绕过中枢的引擎方法（`run_turn`、`subscribe`……）：谁都不能调，不能授予 |
| **披露** | disclosure | 装包或链进扩展时给人看的「它能做什么、替谁做」。扩展包的在装之前、人同意了才装；Agix 插件的在装之后（引擎的顺序） |
| **安全边界** | — | 对恶意代码也成立的约束 |
| **防误** | — | 只防 bug、不防恶意的约束。文档里必须明说是这一种 |

**别混**：「受信项目」（引擎的项目信任）、扩展的「信任」（`Trust`）、「实例的信任」（`InstanceTrust`）、扩展包的「信任等级」是四回事。

## 6. 构建、测试、版本

| 术语 | 代码 | 意思 |
|---|---|---|
| **子仓** | submodule | 应用只有一个：`harnix/`。它自己带 `agix/`（Agix），应用经 `--recursive` 取到、不单独引入 |
| **钉** | pin | 子仓指针指向的提交。应用钉 Harnix；Harnix 钉 Agix，应用随之拿到同一份 |
| **版本标识** | `Harnix-v<x.y.z>-<YYYYMMDD><NN>` | 发布的标签名，见 CLAUDE.md |
| **稳定层级** | S1 · S2 · S3 · U | S1 扩展接口（Rust、包、界面三份）与 S2 线上协议只做加法；S3 是骨架内部的接口，不对外承诺；U 不在任何承诺里 |
| **扩展接口版本** | `EXT_API_VERSION` | `harnix-ext` 的版本，写进文档与 `--version`；兼容性靠 crate 的 semver |
| **贡献点 API 版本** | `CONTRIB_API_VERSION`，`[[ui]] api_version` | 界面的接口照哪一版写的 |
| **协议版本** | `PROTOCOL_VERSION`、`hello.engine_protocol` | 线上协议的版本；随 Agix 变的那两块由 `engine_protocol` 另行标明 |
| **扩展包接口版本** | `harnix_api` | 清单、WIT、桥、`@harnix/ext-sdk` 合起来的版本 |
| **扩展线版本** | `EXT_WIRE_VERSION` | 扩展线的版本；扩展进程总是产品二进制自己，逐字相等才放行 |
| **层次规则** | `crates/harnix/tests/layering.rs` | 依赖方向与源码约束的断言：谁能依赖谁、哪里不许出现哪些名字 |
| **方法表快照** | `method-table.snap.json` | 产品缺省装配的路由表，每一行的所有者与访问；改了要在提交说明里写明 |
| **LIVE 测试** | `tests/live/` | 对着真的二进制跑的功能测试：API 层和 Playwright 层，两种拓扑，扩展包的套件再按两种载体（`--carrier`）各跑一遍 |
| **录像 / 回放** | recording / replay | 引擎记下的模型调用；测试缺省**严格**回放，比对系统块、工具表与每条消息的摘要 |
| **live 模式** | `--live` | 连真实模型服务跑测试，可以写回录像 |

## 7. 流程（可选扩展 `flow`）

产品链了 `flow` 扩展才有这些东西；不链的产品里这些词不出现。

| 术语 | 代码 | 意思 |
|---|---|---|
| **流程** | `FlowDef` | 站组成的有向无环图，一份 YAML，随流程包分发 |
| **运行** | `RunRecord`，`run` | 一条流程的一次执行，处理一个对象（`subject`）。不叫「实例」：实例只指 Agix 实例 |
| **项目任务** | `RunRecord.project_root`，`flow.start {project_root}` | 在一个项目里按指定流程执行的运行，界面上简称「**任务**」：带 `project_root`（已存在的绝对目录），`workspace` 是 `project`（缺省）或 `worktree`；`trigger` 记下是手动还是定时起的。不带项目的运行在自己的 `scratch` 目录里干活。和流程里的执行单位 `TaskRec`、引擎的后台任务都不是一回事 |
| **站** | `Station` | 流程里的判定单位：出口条件、返工、确认、强制通过都按站算 |
| **任务** | `TaskRec` | 流程里的执行单位：一站 0..N 个，每个交一份结构化输出 |
| **执行者** | `Executor` | 做一个任务的那一方：`session`（会话）· `agent`（主会话下的子 Agent，xops-03）· `program`（应用代码）· `external`（人或外部 Agent 来认领）· `subflow`（子运行） |
| **主会话** / **主 Agent** | `RunRecord::main_session`，会话键 `@main` | 每个运行一个，运行建好就开（xops-03）：没写 `scene` 的会话站、子 Agent 站都在它里面一轮一轮地做；落点与模型写在流程级 `main:`。写了 `scene` 的会话站是**独立会话** |
| **子 Agent 站** | `kind: agent` | 主 Agent 用 Agix 的 `Agent` 工具把任务交给子 Agent（`agent_type`、`model` 可选），拿到结果用 `flow_submit` 交；`description` 是任务的标签 `flow:<run>/<station>/<task>` |
| **运行状态** | `Phase` | 读者看到的：`running` · `waiting`（`reason`：approval / confirm / held / unmet / failed / external / children / work）· `pausing` · `paused` · `finished` · `cancelled`。由 `RunStatus` 与「此刻有没有活在手上」推出 |
| **暂停** | `pause` / `resume`，`RunRecord.paused` | 手上的活跑完、照常记下，不进下一站、不派新任务；恢复后接着走。状态仍是 `active` |
| **用量** | `Usage` | 一个任务各轮的 token 合计（`TaskRec.usage`）；主会话里不属于任务的轮次记在运行的 `usage_other` |
| **流程会话** / **通用会话** | `flow.session {flow?}` | 每条流程一个 `flow-author` 对话，它的当前流程就是这条、改不了；不属于哪条流程的是通用会话（新建流程、问帮助），在里面新建的流程把它认作自己的会话 |
| **纯关卡** | — | 没有任务的站，进站就判出口 |
| **出口条件** | `ExitCond` | 出站要满足的结构化条件：`output` · `gate` · `confirm` · `children` |
| **门禁** | `GateDef`，`gates/<名>.yaml` | 流程包里有名字的确定性检查，`{kind: gate, gate: <名>}` 引用它（xops-04）：`tool`（调应用的只读工具、对结果断言，判站时当场判）· `command`（在运行工作目录跑命令，退出码 0 为过；应用 `command_gates(true)` 才有）· `host`（应用自己的检查 `Gates`，按名字）。运行建立时连同定义一起钉住 |
| **规则** | `rules/<名>.md` | 有名字的散文：整个流程里应当怎样（xops-04）。流程级、站级 `rules` 引用，放进会话任务的消息里（skill 之前），进站时固定摘要；**从不拦** |
| **策略** | `PolicyDef`，`policies/<名>.yaml` | 有名字的一组权限，流程级、站级 `policies` 引用（xops-04）；运行建立时钉住 |
| **权限** | `Permission`、`Granted` | 一条 `allow` / `ask` / `deny`：`tools`（名字或 `ns__*`）＋可选 `paths` / `commands` / `args` 范围、`message`。一个任务的权限 = 流程级策略 → 流程级权限 → 站级策略 → 站级权限，派工时解析引用、定下来 |
| **判定** | `judge` | 一次工具调用：命中的权限里 `deny` > `ask` > `allow`；都没命中时受限工具拒绝、其余放行。流程会话（主会话、独立会话）对受限工具与权限提到的工具都 `ask`，由 `flow` 扩展按这一轮所属任务的权限答；`ask` 留给人（任务 `asking`，运行 `waiting: approval`），人不答引擎到时（实例的 `permission_prompt_timeout_secs`，缺省 300 秒）当拒绝 |
| **钩子** | `HookDef`，`hooks/<名>.yaml` | 把运行的事件告诉外面（xops-04）：`on`、`flows` 过滤，`kind: webhook`（现在只此一种）、`action` 是 URL、`secret_env` 可签名。**不写在流程里**，放进包里就生效；失败重试三次，从不卡住流程 |
| **返工** | rework | 退回上游某站：它和它的全部下游过期、重来 |
| **强制通过** | force | 有人在条件不满足时放行一站，带理由、留痕 |
| **租约** | lease | 认领一个任务的期限与 token；过期可再认领，旧 token 交不进来 |
| **流程包** | pack，`PartKind` | 流程定义 + 件（skill、规则、策略、门禁、钩子），由应用实现的 `Packs` 给 |
| **协议层** | — | `flow` 扩展在站的 skill 正文前注入的一段：交什么、怎么交、受哪些权限与门禁，再接这一站的规则 |
| **落点** | placement，`Placement` | 站的会话建在哪：space、实例或模板实例（`instance` / `template` + `params`），用哪个模型（`model` / `provider`）（xops-02） |
| **交付方式** | `Delivery` | 会话怎么交输出：结构化输出（`structured_output`，缺省：契约作为会话的 `output_schema`，模型调 `StructuredOutput`）或 `flow_submit`（`flow-*` 包装 scene） |
| **受限工具** | `guarded_tools` | 没有权限放行就拒绝的工具（缺省引擎的 shell、文件编辑、notebook、解释器）；流程会话对它们都 `ask`，由 `flow` 扩展判 |
| **来源** | `Origin` | 流程包里一条流程从哪来：`builtin`（随应用）· `override`（可写层里覆盖内置的那份）· `user`（只在可写层） |
| **当前流程** | focus | 一个 `flow-author` 对话操作的那条流程：流程会话的就是它自己那条；别的对话是 `flow.focus` 设的，或 `flow_create` 刚建的 |
| **试运行** | dry run，`dry-` 运行 | 在沙箱里真跑流程的内核、外部全部模拟：不建会话、不调应用代码、不花模型费用；`mocks` 给站的输出、命令 / 宿主门禁、工具门禁的工具结果 |

**别混**：

- 「流程」指这个扩展的流程定义；贡献点 `flow.block` 与外壳文案里的 `flow.*` 指**对话流**（转录里的一块），两回事。
  流程扩展的界面文案用 owner `flowui`。
- 「运行」不是「实例」，也不是「会话」：一条运行有一个主会话，还可能有独立会话与子 Agent 的侧链会话。
- 流程的「子 Agent」就是 Agix 的子 Agent（父会话的一件**任务**、一条侧链会话）；流程的「任务」（`TaskRec`）是另一回事。
- 流程的「来源」（`Origin`：`builtin`、`override`、`user`）和扩展的「信任」（`Trust::Origin`）是两回事。
- 「试运行」是一条真的运行，只是活在沙箱里：运行号以 `dry-` 开头，不进 `runs`，最多留最近 20 条；不叫「模拟运行」。
  这个沙箱是流程自己的模拟环境，和扩展进程的操作系统沙箱无关。
- 界面上，人还没答的权限询问（`ask`，运行的 `waiting: approval`）说「待批准」；站的 `hold`（进站后等人放行）说「等人放行」
  「待放行」，不说「暂停」——「暂停」只指 `pause`。
