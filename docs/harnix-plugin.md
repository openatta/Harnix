# Harnix 插件：插件 APP 怎么写（模式 2 / 3）

> 读者：给 Harnix 产品写插件 APP 的人（第三方也可以）；在自己的产品里开模式 2、要知道插件 APP 能做什么的产品方。
>
> 状态：**模式 2 在开发与测试中**（cargo feature `apps` 加 profile 的 `[apps] enabled`，两道开关都关着时构建与运行和没有这件事一样）；
> **模式 3 只有设计**。包对两种模式中立：本文写的清单、WIT、宿主函数、界面、桥、签名与商店，两种模式完全一样；两种模式
> 能观察到的不同只在 [应用开发](app-development.md) §10。三种模式怎么选见 [应用开发](app-development.md)。
>
> 用词照 [术语](glossary.md)：**插件**（插件 APP、运行期应用）是装进产品里跑的应用，代码里叫 `apps`；Agix 的包叫 **Agix 插件**
> （运行期包），它可以是插件 APP 的一部分（§5）。
>
> 分两部分。**第一部分（§1–§12）是指南**：怎么写、为什么。**第二部分（§13 起）是接口清单**。完整的例子是
> `examples/apps/showcase`（[它的 README](../examples/apps/showcase/README.md)）：每样东西一个。代码和这里对不上时以代码为准。

---

# 第一部分：指南

## 1. 一个插件 APP 是什么

**一个插件 APP 就是一个包：后端逻辑、给 Agent 的工具、Agent 能力、界面四样可任选，从商店下载、验签、装上就能用，
不改产品、不重新编译。** 作者只面对 Harnix，不需要知道 Agix。

| 部分 | 是什么 | 在哪跑 |
|---|---|---|
| 后端与工具 | 一个 WASM 组件（WIT `harnix:app@0.1.0`）：常驻的后端（方法、主题、定时器、订阅），给 Agent 的工具，工具调用的观察与否决 | Harnix 主进程里的 wasmtime，每个应用一台虚拟机（§4） |
| Agent 能力 | `agent/`：一个 Agix 插件（skill、agent 类型、场景、QuickJS 脚本） | 装进产品声明的实例里，在 Agix 里跑（§5） |
| 界面 | `ui/`：iframe 里的页面，经桥借外壳的手；外壳照清单画的行内点；受信发布者另可以带进页面的模块 | 浏览器（§6） |
| 清单 | `harnix-app.toml`：它是什么、要什么能力、有哪些方法与主题、界面放在哪 | 安装时读、给人看（§3） |

**一个应用一台虚拟机，只在主进程里。** 「虚拟机」是一个边界，不是一个进程：

| 这台虚拟机里有 | 说明 |
|---|---|
| 组件 | 应用的 `.cwasm`，加载、链接一次；整个进程共用一个 wasmtime `Engine` |
| 常驻实例（后端） | 一个长期存活的实例：应用的状态、订阅、定时器都在这里；方法调用排队串行进入 |
| 调用实例（工具与事件） | 每次工具调用、每次事件新建一个，用完即弃：并发安全，坏一次不连累后端；看不到后端的内存，要状态就 `host.call` 自己后端的方法 |
| 资源账 | 内存、单次调用的时间、存储按应用记账；一个工具连续出错只停那个工具，后端重建后再崩才停用整台 |

spawn 出来的 daemon 实例不起虚拟机：它的会话经这个应用的 MCP 端点（`/apps/<id>/mcp`）回到主进程那一台执行。所以在任何拓扑下
一个应用只有一台虚拟机、一份状态。

**数据只走 Harnix 的通道。** 应用的界面和它的虚拟机之间没有自己的连接：

```text
应用的界面（隔离 iframe，connect-src 'none'）
   │ 桥：MessageChannel，harnix-ui-bridge@1
   ▼
外壳 ── 页面的 Harnix 连接（两种拓扑同一套协议）──▶ 准入 ──▶ hub ──▶ apps 模块 ──▶ 后端 handle()
   ▲                                                                                  │
   └───────── harnix.topic `app.<id>.<主题>` ◀── hub 主题 ◀── host.publish() ◀────────┘
```

| 路径 | 走法 | 谁把关 |
|---|---|---|
| 界面调自己的方法 | iframe `call("app.<id>.notes.list")` → 桥 → 外壳 → 页面的连接 → 准入 → `backend.handle` | 外壳按清单过滤名字；hub 按设备准入、记审计 |
| 应用推界面 | `host.publish("notes", v)` → 主题 `app.<id>.notes` → 外壳 → 桥 → iframe | 只能推自己声明的主题 |
| 应用驱动 Agent | `host.call("session.create" / "harnix.send", …)` → 准入 → hub → Agix | 主体 `module:<id>`，只到清单 `hub` 列出的 |
| Agent 用应用的工具 | 模型调 `app__<id>__<tool>` → Agix 的权限（一律先问人）→ 新建调用实例 `call-tool`；spawn 的实例经 MCP | 权限照常；宿主函数按能力查 |
| 应用听引擎的动静 | `host.subscribe("harnix.sessions.status")` → `backend.on-topic`；要否决工具调用，用 `events` | 可订的主题按 `hub` 能力；`events` 缺省只看到自己工具的调用（`[events]`） |

**会话不直接调 `app.<id>.*`。** Agent 用的是应用的工具；工具要后端的状态，就 `host.call` 自己的方法。

## 2. 包

`.hxapp` 是一个 zip，解包前整个读一遍，有一样不对就拒，一个字节都不写：

```text
harnix-app.toml                     清单（§3）
ui/…                                iframe 的页面与资源；受信发布者另可以带进页面的 ES 模块
agent/plugin.toml, agent/…          一个 Agix 插件；agent/skills/<名字>/SKILL.md 在它旁边
wasm/<name>.wasm                    组件，可移植（可选；产品跑不了它，只反序列化编好的产物）
wasm/<name>.<指纹>.cwasm            为某一个引擎编好的组件，每个引擎一份
SIGNATURES                          每个文件的 sha256，发布者签名与商店副署（§8）
```

拒装的：绝对路径、往上爬（`..`）、链接、不属于上面几部分的条目；超过大小或条目数上限；清单读不了；组件没有这个引擎的产物
（报出包里有哪些引擎的）；清单点名的界面模块包里没有；`agent/` 没有 `plugin.toml`。

**为什么要商店预编译。** 产品构建不带编译器，只反序列化，二进制小、攻击面小（`budget` 层另给带 `apps` 的构建立了体积上限）。
代价是产物绑 wasmtime 的版本与 CPU 特性：商店为每个**引擎指纹**（wasmtime 版本 + 目标三元组 + 特性的哈希，含 `cpu=baseline`）各编一份。
Harnix 与 Agix 一一对应地发布，wasmtime 只在 Harnix 发布时变，所以商店为每个 wasmtime 版本编一遍。没有匹配指纹的产物就装不上。
反序列化只用于可信的产物，所以**先验签、再按签名里的 sha256 比对读进来的字节，才反序列化**。

## 3. 清单 `harnix-app.toml`

**严格读。** 应用能有的一切都是下面的一节或一个字段，别的按名字拒：清单也是安装时给人看的东西，一行没人读的字是白给人看的——
或者是有人以为已经给出去的一项能力。节与字段只增不改。完整的字段表见 §14。

### 3.1 `[app]`

```toml
[app]
id = "acme.notes"        # <发布者>.<名字>：点分的段，小写字母、数字、-，字母开头
version = "1.2.0"        # x.y.z，可带 -<预发布>
harnix_api = 1           # 应用接口的大版本，要等于这个 Harnix 的
title = "Notes"          # 可选
publisher = "acme"       # 就是 id 的第一段
description = "…"        # 可选
```

### 3.2 `[capabilities]`：写了才有

```toml
[capabilities]
net = ["api.acme.com"]           # http-request 能到的主机，精确匹配（没有后缀规则），不跟随重定向
env = ["ACME_TOKEN"]             # secret 能读的名字：profile 的 [apps.secrets."<id>"] 给它的，不是进程的环境变量
storage_mb = 16                  # 存储上限：kv 加 /data 里的文件
hub = ["session.create", "harnix.send", "topic:harnix.sessions.status"]
                                 # 以 module:<id> 能调的方法、能订的主题（topic:<名>）；自己的 app.<id>.* 不用写
fs = ["${data}", "${project}"]   # ${data}：自己的数据目录 /data，读写；${project}：发起工具调用的会话的项目 /project，只读，只在工具调用里
```

一律默认不给。能力只有这五样，写了别的按名字拒。`fs` 给了 `${data}` 而 `storage_mb = 0` 时拒（写进去的东西算存储，0 就什么都写不了）。
每一项在宿主函数里逐次检查，拒绝时说是哪一项能力本可以放行它；`secret` 是例外：没列的名字、列了而 profile 没给的，读起来都和没设一样
（日志里记一笔），因为一个秘密有没有，本身也是不该给的信息。

- **`env` 读的是 profile 给它的秘密**：`[apps.secrets."<id>"]` 下这个名字的值（§19），**从不读 Harnix 进程自己的环境变量**——
  那里有产品自己的钥，一个名字不能等于「把进程碰巧有的这个变量给它」。
- **`net` 可以写本机或局域网的地址**（`127.0.0.1`、`localhost`、`*.local`、私网段），但那到的不是互联网，而是 Harnix 旁边的服务
  （包括 Harnix 自己的监听）：披露里这一行换一种说法（「它能访问这台机器或局域网上的服务」），商店也不副署这样的包。

### 3.3 `[[component]]` 与 `[backend]`

```toml
[[component]]                    # 至多一个
name = "notes"                   # wasm/<name>.<指纹>.cwasm
world = "harnix:app/backend-app@0.1.0"   # 或 harnix:app/tools-app@0.1.0
memory_mb = 64                   # 1–4096，缺省 64
timeout_ms = 30000               # 100–600000，缺省 30000

[backend]                        # 只有 backend-app 能写
resident = "on-demand"           # always（缺省）| on-demand
idle_secs = 600                  # 60–86400，缺省 600
```

```toml
[events]                         # 只在有组件时；它的 events 导出（§4）看到哪些工具调用
tools = "own"                    # own（缺省）：只有它自己的工具；all：每个会话的每一次工具调用，连同输入
```

- `tools-app`：给 Agent 工具、观察与否决事件，自己不留状态（kv 除外）。
- `backend-app`：另有常驻的后端，答方法、推主题、跑定时器、订 hub 的主题，也照样给工具与事件。
- `[backend] resident = "on-demand"`：后端闲置满 `idle_secs`、又没有东西拿着它时卸下，下一次调用冷启动（`init` 重跑，状态从 kv 回来），
  系统 space 显示「休眠中」。「拿着它」是：有订阅、有定时器、有排队的投递、有在途的调用（方法、工具调用、事件钩子都算）。
  工具调用本来就是临时实例，不受影响。
- `[events] tools = "all"` 是一项能力：它会看到每一次工具调用的名字与参数（`Bash` 的命令、`Write` 的文件内容……），装的时候进披露。
  缺省的 `own` 只看到自己工具的调用，不算能力，不进披露。

### 3.4 `[[method]]`、`[[topic]]`、`[[space]]`

```toml
[[method]]                       # app.<id>.<name>，后端的 handle 答
name = "notes.add"
subject = "device"               # 谁能调：device（缺省）、module、kernel，或几个；应用自己总能调
description = "Pin a note"       # 可选
[method.params]                  # 可选：一份 JSON Schema，参数先对它校验，不符答 INVALID_PARAMS
type = "object"
required = ["text"]

[[topic]]                        # app.<id>.<name>，后端 host.publish 推整值
name = "notes"

[[space]]                        # 一个后端 space：在它里面建的会话落在哪
id = "acme.notes"                # 应用的 id，或 <id>.<名字>
scene = "harnix"                 # 可选：新会话的场景
instance = "system"              # 可选：一台已有的实例
```

方法与主题只有 `backend-app` 能声明。方法、主题、space 在应用启动时登记进授权表、方法路由与 `harnix.spaces.list`，
停用或卸载时撤下：之后的调用答 `-32601`（和从没有过一样），在途的照常做完，主题的订阅者收到 `harnix.topic_ended`。
名字与任何编译期模块或已登记的应用重叠就拒，编译进来的优先。space 长什么样由界面给（§6）。

### 3.5 `[config]`

```toml
[config]                         # host.config() 与 backend.init 拿到的 JSON：这里的缺省，profile 可以按键盖过
greeting = "hello"
```

profile 里写 `[apps.config."acme.notes"] greeting = "…"`，按顶层的键覆盖。

### 3.6 界面：`[interface]`、`[[surface]]`、`[inline]`、`[[ui]]`

```toml
[interface]
isolate = false                  # true：受信发布者的界面也进 iframe
hub = ["harnix.sessions", "harnix.send"]
                                 # 页面除了自己的 app.<id>.* 还能调、能订的：白名单里的名字，限定在它自己的 space（§6.3）
links = true                     # 能请外壳打开网页（每次都问人）

[[surface]]                      # 一个面：外壳的一块区域，由应用的页面画在 iframe 里
point = "space"
space = "acme.notes"
page = "frame/board.html"        # ui/ 下的 .html
title = { en = "Notes", zh-CN = "笔记" }
icon = "file"                    # 外壳自己的字形名
order = 60
center = "hidden"                # hidden（缺省）| minimized | chat（中区是对话，不要 page）

[[inline.tool_row]]              # 外壳照它画的行内点，不跑应用的代码
tool = "search"
title = "Search · {input.query}"

[[ui]]                           # 进页面的 ES 模块，同 Agix 插件的 [[ui]]：只对受信发布者生效
point = "home.card"
module = "notes.js"
api_version = 2
```

各节的意思在 §6。`[interface] hub` 只能写白名单里的名字（§6.3 的表）：外壳自己的家族（`harnix.apps.*`、`harnix.plugins.*`、`harnix.settings.*`、
`harnix.devices.*`、`harnix.instances.*`、`harnix.system.*`、`harnix.upload.*`、`harnix.prompts.*`、`harnix.prefs.*`、`harnix.subscribe`、
`harnix.attach` 等）写了就拒，别的不在表里的名字也拒。

**一个 space 的左栏只有一种**：要么是外壳的会话列表（可以用 `[[inline.sidebar_group]]` 分组），要么是应用自己的 `space.sidebar` 面。
两样都声明，按名字拒。showcase 用两个 space 演示两种：`example.showcase` 画自己的左栏，`example.showcase.chats` 用外壳的会话列表并分组。

### 3.7 `[agent]`

```toml
[agent]
install_into = ["system"]        # agent/ 装进哪些实例：system，或安装时已经存在的实例的 id；缺省 ["system"]
```

## 4. 后端：WIT `harnix:app@0.1.0`

组件对着 `crates/wasm/wit/app.wit` 写（`harnix-app new` 拷一份到 `component/wit/app.wit`）。两个 world，各有一个带 `events` 的版本：

```wit
world tools-app          { import host; export tools; }
world backend-app        { import host; export backend; export tools; }
world tools-app-events   { include tools-app; export events; }
world backend-app-events { include backend-app; export events; }
```

`events` 是可选的导出：用不着观察工具调用的组件用前两个，清单的 `world` 也只写这两个之一（导出了 `events` 的组件仍是那个 world，
宿主从组件的导出里认出它）。

| 导出 | 函数 | 在哪个实例里 |
|---|---|---|
| `tools` | `list-tools() -> list<tool-def>`、`call-tool(name, input-json, call-id) -> tool-output` | 每次调用一个临时实例 |
| `events`（可选） | `on-event(event, payload-json) -> hook-decision`：`proceed`、`block(理由)`、`add-context(一段话)` | 每次一个临时实例 |
| `backend` | `init(config-json)`、`handle(method, params-json, caller-json) -> result<string, app-error>`、`on-topic(topic, value-json)`、`on-timer(id)`、`stop()` | 常驻实例 |

**宿主函数**（`interface host`，每一个都是查能力的地方）：`log`、`now-ms`、`config`、`progress`、`http-request`（`net`）、`secret`（`env`）、
`kv-get` / `kv-set` / `kv-delete`（`storage_mb`）、`call`（`hub`）、`publish`、`subscribe` / `unsubscribe`（只后端，`hub`）、`set-timer` /
`cancel-timer`（只后端）。签名见 §15。**没有文件 API**：文件走 WASI，只在 `fs` 预开的 `/data`、`/project` 里——把路径检查重写成宿主函数，
正是这类检查悄悄出错的方式。

**规矩**：

- **`handle` 要快。** 一个后端的调用串行进入：等着宿主（`call`、`http-request`）的 `handle` 仍然占着实例。慢活拆成「发起」加
  「定时器或主题回报」。排队等后端的调用也只等 `timeout_ms`：到时还轮不到，答「busy」（`failure: "unavailable"`，`health.queue_timeouts` 计一次）。
- **超时换实例。** 超过 `timeout_ms` 的调用被放弃，后端实例随之换成新的（`init` 重跑，内存里的状态丢失，`health.recycles` 计一次）：
  停在半路的代码不能信它留下的状态是一致的。
- **崩溃重建一次，再崩搁置。** 后端陷阱后重建一次；再崩就把应用置于一旁（`faulted`）：记录仍是启用的，方法答「不可用」，
  停用再启用会重起。健康运行 10 分钟后出错计数清零。
- **一个工具连续出错，只停那个工具。** 同一个工具连续陷阱 3 次，它被放在一旁（`health.tools_aside`），之后的调用答「不可用」；
  别的工具、后端、界面照常。工具的输入来自模型（和注入进会话的东西），它出的错不能连累更多。事件钩子的陷阱不计（钩子失败本来就放行）。
  停用再启用会把它放回来。
- **`/data` 也受 `storage_mb` 管。** 文件经 WASI 写，宿主看不见写入，所以每次调用后量一次目录（不在异步线程上量；条目超过 10 万算超额）。
  超了之后，新的实例看到的 `/data` 只读（还能删）；常驻的后端也一样：它的文件越过配额，常驻实例就像超时那样换掉，换来的那个 `/data`
  只读（`health.storage_recycles` 计一次）；删回配额以内，再换一次，又能写。
- **要留下的状态放 kv。** 内存里的状态随时可能因为换实例、重建或休眠而丢掉。kv 按应用分、后端与工具共用、有上限、卸载即删。
  `kv-get` 答 `result<option<…>>`：`none` 是这个键没有值，错误是存储读不了——两者不一样，别把读不了当空的写回去。
- **工具与事件的实例不 `init`。** 它们要的应用状态，向后端 `host.call` 要；配置用 `host.config()` 读。
- **工具的权限照常。** `tool-def.read-only` 只用于调度，宿主不因为组件说自己无害就放宽权限。

**稳定性**：在一个 `harnix_api` 之内，函数与类型只增不改（新函数是这个包的补丁版本，老组件照样链接）；**已有记录的字段与变体的分支冻结**
（组件模型的类型在链接时按结构检查，记录加一个字段就是另一个类型，老组件全链接不上）。要长的形状一律是 JSON 字符串
（`params-json`、`caller-json`、`config-json`、`payload-json`），那里只加字段、不认识的字段忽略。别的变化是新的 `harnix_api`。

## 5. Agent 部分

- **`agent/` 是一个 Agix 插件**：`plugin.toml` 加 skill、agent 类型、场景、QuickJS 脚本。安装时打包成一个 zip，经引擎的
  `plugin.install` 装进 `[agent] install_into` 列出的每台实例，随应用启停（`plugin.enable` / `disable`），卸载时撤掉。
- **skill**：`agent/skills/<名字>/` 拷进实例数据根的 `skills/<名字>/`，带一个标记文件说它是哪个应用的；同名的、不是这个应用的 skill
  已经在那里，就拒。拷完让实例重读设置。
- **QuickJS 脚本**挂在 Agix 的扩展点上，没有 I/O，和应用的虚拟机之间**没有通道**：要有逻辑的钩子，写成 WASM 的 `events`。QuickJS 只限资源，
  不是隔离边界：它的信任来自签名（§7），不来自沙箱。
- **工具名**：会话里叫 `app__<id>__<tool>`，id 里的 `.` 换成 `_`（模型 API 的工具名只收字母、数字、`_`、`-`，id 里没有 `_`，所以一一对应），
  例如 `app__acme_notes__search`；全名至多 64 个字符。spawn 的 daemon 经 MCP 看到的是 `mcp__app-<id>__<tool>`（同样换 `_`）。
- **每会话登记**：Harnix 的 Rust 插件 `AppsPlugin` 在每个会话建起时登记当下在跑的应用的工具。新装的应用，新会话马上有；已有的会话在
  下次重建时才有。子 Agent 带着父会话的插件构建，所以也有。
- **权限**：应用的工具每次调用都先问人（「应用 X 想跑它的工具 Y」），宿主的权限处理照常。
- **`events`**：应用的事件钩子包在会话的工具调用外面（`tool.before`），可以放行、拒绝、加一段可归属的话；钩子自己失败时放行。
  它看到哪些调用由清单的 `[events]` 定：缺省只有它自己的工具；`tools = "all"` 是每一次（连同输入），装的时候披露。
  加的话只挂在它自己工具的结果上，至多 2000 字：下载来的包不能往模型调的每个工具的结果里写东西。
  `tool.before` 的 payload 是 `{tool, input, session_id, depth}`（`tool` 是会话里的名字，`app__…`）。
- **进度**：工具调用里的 `host.progress(call-id, text)` 推到主题 `harnix.apps.progress`，界面在那一行工具上显示。
- **spawn 的 daemon**：经 `/apps/<id>/mcp` 用应用的工具。端点只答本机的对端（监听开在网络上时，别处来的请求有令牌也答 403），
  再看这个应用的令牌（每次启动重发，进程退出即作废，只写进 spawn 的 daemon 的 MCP 配置）。那边没有 `/project`、没有进度、没有事件钩子；
  daemon 的引擎照常在每次调用前问人。

## 6. 界面

### 6.1 怎么跑：进页面，还是进 iframe

| 信任 | 界面怎么跑 |
|---|---|
| `store`（商店副署） | **隔离 iframe**：它的面（`[[surface]]`）与行内点（`[inline]`）；`[[ui]]` 模块一个都不跑 |
| `unsigned`（只在开发模式） | 同上 |
| `trusted`（受信发布者签全） | **进页面**：它的 `[[ui]]` 模块，和编译期的界面同权限；`[interface] isolate = true`，或者它没有 `[[ui]]` 模块时，也进 iframe |

这条规则只有一处（`harnix_app_package::runs_in_frames`）。进页面的模块与页面同级（[概念与架构](concept_and_architecture.md) §5.8），
所以只给受信发布者。

### 6.2 面

**面**是外壳的一块区域，应用的页面画在里面的一个 iframe 里。iframe 适合整块的区域，不适合成百个的小部件（一个流块一个 iframe 养不起），
所以小部件是**行内点**（§6.5），由外壳照声明画。

| 点 | 外壳的哪里 | 什么时候建 | 什么时候毁 |
|---|---|---|---|
| `space` | 一个 space：它的入口与中区 | 第一次进入 | 应用停下（离开时留着、藏起，保状态） |
| `space.sidebar` | 那个 space 的左栏中段 | 第一次进入 | 应用停下 |
| `space.tab` | 它右栏的一个页签 | 第一次选中 | 应用停下 |
| `space.secondary` | 它的二级栏 | 打开时（`secondary.open`） | 关掉 |
| `details.panel` | 右栏详情里画它自己 `kinds` 的项 | 显示一项时 | 换一项 |
| `settings.section` | 设置里的一节 | 设置打开到它 | 设置关上 |
| `home.card` | 新对话输入框下的一张卡（自动高度） | 空页显示它 | 离开 |
| `modal` | 外壳画的对话框（`openModal`） | 打开 | 关上 |

- **iframe**：`sandbox="allow-scripts allow-forms"`，不给 `allow-same-origin`（不透明源，读不到页面的令牌、存储与 cookie），不给
  `allow-popups`、`allow-top-navigation`（外链只经 `openLink`）。`allow-forms` 不是多余的：没有它，表单的 `submit` 事件根本不触发，
  页面自己的表单就收不了输入；表单能提交到哪里由 CSP 的 `form-action 'none'` 管（哪里都不能）。
- **iframe 能自己导航走，但出不了这个源。** 沙箱与 iframe 自己的 CSP 都挡不住页面 `location = "…"` 把自己换掉；挡住它的是外壳页面的
  CSP `frame-src 'self'`：iframe 的导航也按它查，别的主机一个请求都不发（LIVE 钉着这一点），所以页面没法把它拿到的东西放进地址带去别处。
  它还能导航到 Harnix 自己的路径（地址只到这个 Harnix）。外壳在 iframe 第二次加载时立刻把它清空（`about:blank`），记一次界面失败
  （应用页签、诊断里都看得到）。这条边界靠的是外壳页面的 CSP：把外壳嵌进别的页面、换掉这条策略的产品，要自己守住它。
  装的时候，有页面的应用披露里有这一行（`frames`）。
- **CSP**：`/apps/<id>/ui/` 下的每个文件带 `default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:;
  font-src 'self'; connect-src 'none'; frame-src 'none'; form-action 'none'; base-uri 'none'; frame-ancestors 'self'`。**`connect-src 'none'`**：
  页面不能联网，数据只走桥。页面是不透明源，所以用经典脚本（`<script src>`）：模块脚本要 CORS。
- **外框**：外壳给每个面画一圈外框（标题、加载中、出错、重新加载），应用只画框里的东西。一个崩掉的应用看起来是「这一块出错了，重试」。
  15 秒内不说 `hello`、连续 5 次心跳没答（每秒一次）、或页面第二次加载（它自己导航走了），都算失败；5 分钟内失败 3 次，这个面置于一旁。
- **常驻的面**（space 的中区与左栏、右栏页签）画在 React 树之外、叠在槽位上，所以换 space 再回来状态还在。
- **键盘**：外壳的快捷键（⌘K、⌘\、⌘/）在 iframe 里按下也交给外壳；在外壳的对话框（模态框、设置节）里，Esc 也交给外壳；外壳的
  对话框开着时，iframe 一个键都收不到（整个 inert）。所以一个键只落在一处。

### 6.3 桥与 `@harnix/app-sdk`

页面加载完，外壳经 `postMessage` 交给它一个 `MessageChannel` 端口（只此一次、只给这个 frame），之后一切走端口：请求、应答、事件。
页面先说 `hello`，外壳答它是哪个面、外观、它的 space 当前的会话。协议的定义就是 `ui/packages/app-sdk/src/protocol.ts`（两边都 import 它）。

`@harnix/app-sdk` 是页面一侧的小 SDK，不依赖框架：

```js
// <script src="vendor/harnix-app-sdk.js"></script> 把 SDK 放在 window.HarnixApp；打包的页面 import { connect } from "@harnix/app-sdk"
const app = await HarnixApp.connect();                         // 等外壳的端口，hello
const notes = await app.call("app.acme.notes.notes.list");
const off = app.subscribe("app.acme.notes.notes", {}, (value) => draw(value));
if (await app.confirm("Delete every note?", { danger: true })) await app.call("app.acme.notes.notes.clear");
app.onEnv((env) => (document.documentElement.dataset.theme = env.theme));
```

连上之后它自己做的：答外壳的心跳；外壳按内容定高度的面（`home.card`、设置节、模态框）报高度；把外壳的键交给外壳；把页面没接住的错误
报给外壳的诊断。全部方法见 §20。

**页面拿不到的**：页面的令牌、外壳的连接、DOM、别的应用的名字、剪贴板（`copy` 由外壳写）。它只能请外壳去做，外壳只做清单让它请的：

- **数据**：自己的 `app.<id>.*`，加上 `[interface] hub` 列出的；主题同样（`topic:<名>`）。`[interface] hub` 是一张**白名单**
  （`harnix_app_package::surface::FRAME_HUB`，外壳的 `FRAME_HUB` 同一张），每个名字带一条限定，外壳按它收窄这次调用，再由 hub
  当作页面（一台设备）准入：

  | 名字 | 限定 |
  |---|---|
  | `harnix.sessions` | 只列它自己的 space：`space` 不写就填它的，写了别的 space 就拒 |
  | `harnix.send`、`harnix.sessions.tasks`、`session.get`、`session.history`、`session.entries`、`session.interrupt`、`topic:harnix.sessions.tasks` | `session_id`（加 `instance`）要是它自己 space 里的会话，外壳先查一遍 |
  | `harnix.spaces.list`、`topic:harnix.spaces` | 不涉及哪个会话，原样 |

- **会话**：只在应用自己的 space 里：`session.create` 建的会话归它的第一个 `[[space]]`，`session.open` 只开它自己 space 的会话；
  经 `call` 打会话的名字同样只到它自己 space 的会话（上表）。
- **外壳的手**：`spaces.switch`、`details.open` / `details.show`（只画它自己 `kinds` 的项）、`secondary.open` / `close`、`settings.open`（它自己的节）、
  `composer.insert`、`badge`（落在它自己 space 的入口上）、`notify`、`copy`、`openModal`（它自己的模态框）、`confirm`、`openLink`、`upload`。
  `confirm` 是外壳画的对话框，应用造不出假的确认；`openLink` 要 `[interface] links`，每次先问人；`upload` 由外壳代传，答落盘路径，给下一条消息当附件。
- **被拒**的请求答 `refused`，记进系统 space 应用页签的「最近被拒的界面请求」，也进设置 → 诊断。

### 6.4 `@harnix/ui-kit`

外壳的令牌（颜色、间距、圆角、字号）加几样基础部件（按钮、输入、字段、表格、卡片、空态、加载、提示）的 CSS 与轻量的 DOM 函数，
打进应用自己的包里；`follow(app)` 让主题与语言跟着外壳变。用不用 React 随便，它不依赖 React。showcase 带着两者构建出的文件
（`ui/frame/vendor/`）。

### 6.5 行内点

| 点 | 清单里写什么 | 外壳做什么 |
|---|---|---|
| `[[inline.tool_row]]` | 某个工具一行的样子：字形、标题模板（`{input.<路径>}`、`{result}`）、标题后面跟什么（`result` 或 `input.<路径>`）、结果折不折起（缺省折起） | 用自己的工具行组件照画 |
| `[[inline.flow_block]]` | 应用写的某种扩展条目（`kind` 是条目的事件名）画成卡片：标题、Markdown 正文、键值行（至多 12）、按钮（至多 4，调它自己给设备的方法，可先 `confirm`，完了显示 `done`） | 画卡片；点按钮经页面的连接调方法。要写条目，`hub` 里得有 `harnix.entries.append` |
| `[[inline.command]]` | 一条 `/` 命令或一个 `@` 来源 | 命令菜单照常，候选来自后端方法 `app.<id>.complete`（要声明它） |
| `[[inline.sidebar_group]]` | 它的某个 space 怎么给会话分组：按 `scene`、按会话行的某个字段（`field`），或问后端方法（`method`：`{space, sessions}` → `{groups}`）；`other` 是哪组都不进的 | 外壳排、画 |

文本可以是一个字符串，或按语言给（`{ en = "…", zh-CN = "…" }`）。每一项都对着清单的其余部分查：按钮与分组的方法要是它给设备的，命令要有
`complete`，卡片要有 `harnix.entries.append`，工具行的工具要是组件列出的（启动时查）。命令与按方法分组会在安装披露里写明「它会知道你为它的
命令打的字」「它会知道这个 space 的会话」。

### 6.6 `[[ui]]`：进页面的模块

受信发布者的界面可以是进页面的 ES 模块，写法同 Agix 插件的 `[[ui]]`（贡献点 API 2，从页面的 import map 拿 React，[SDK](harnix-sdk.md) §22.6）。
它们和编译期的界面同权限，所以只对 `trusted` 生效；商店与未签名的包里的 `[[ui]]` 不跑，它们的面跑。showcase 两样都带：受信时跑模块，
否则跑面。

### 6.7 开发时

`[apps] dev = true` 下，一个未签名应用的 `ui/` 变了（`runtime.ui_rev`），外壳重新加载它的 iframe。应用停下，它的 iframe 跟着撤掉。

## 7. 信任等级

| 等级 | 什么时候 | 界面 | 谁能装 |
|---|---|---|---|
| `store` | 商店的副署对这个 Harnix 带的商店公钥验得过 | iframe | 任何开了 `[apps] enabled` 的产品 |
| `trusted` | 它的公钥是 profile 的 `[apps.trusted_publishers]` 为这个发布者写的，且它的签名覆盖每一个文件——经商店来的，覆盖商店加的编译产物以外的每一个文件（产物由商店副署） | 进页面（可选 iframe） | 配了这个发布者的产品 |
| `unsigned` | 没有 `SIGNATURES` | iframe | 只有 `[apps] dev = true` 的进程，只能从文件装 |

- 等级在安装时由签名决定，写进安装记录（`parts.trust`）；每次启动对盘上的文件再验一次。
- 发布者受信时，经不经商店都是 `trusted`：商店副署验得过、发布者也受信，取 `trusted`，不是 `store`。
- 商店副署是未知的商店公钥、而发布者受信且签全时，也算 `trusted`。
- 开发模式下，验不过的签名一律拒，不当作 `unsigned`。

## 8. 签名

`SIGNATURES` 列出包里每个文件的 sha256，两方签这张表：

- **发布者**用自己的 ed25519 钥签它做的文件（清单、`ui/`、`agent/`、可移植的组件、它自己编的产物）；
- **商店**查过发布者、为它服务的每个引擎编好组件，再副署整张表——连同发布者的签名，以及它加的编译产物（它只能加这些）。

多一个文件、少一个、改了一个，都拒，并说是哪一个。`.cwasm` 读进来准备加载时，按签名里的 sha256 再比一次：反序列化的正是签过的。
`harnix-app keygen / sign / verify / countersign`（§10）做这些。

## 9. 商店与同意

- **目录**：一份静态的 `index.json`（`https://`，或本机的 `file://`；`http://` 拒），profile 的 `[apps] store`，不写用内置的。列每个应用的版本、
  `harnix_api`、要的能力、签名的钥、每个引擎指纹一份的包（地址、sha256、大小）。目录只是报价：下载下来的先对它列的 sha256，再对包自己的
  `SIGNATURES`；目录里的能力与钥只是在下载之前给人看的。每次问都重读目录。
- **装**：`harnix.apps.install {id, version?}` 分两步。第一次答**披露**（发布者、签名、版本、要的每一项能力、界面能调的名字、是否进页面、
  行内点会知道什么；`net_local`——它能访问这台机器或局域网上的服务、`events`——它会看到每一次工具调用的名字与参数、
  `frames`——它有自己的页面，隔离着跑，各是一行）与一个 digest，不装；带上 `consent: <digest>` 再调一次才装。要装的东西在这之间变了（包、签名、要替换的版本），就重新披露。
  同意是无状态的：第二次调用重新下载、重新验签。
- **装的顺序**：验签 → 解包到 `~/.<product_id>/apps/<id>/<version>/` → 把 `agent/` 装进声明的实例、拷 skill → 登记方法、主题、space →
  加载组件、后端 `init` → 发界面。任何一步失败整体回滚，只停这一个应用。
- **升级**：`harnix.apps.update {id, version?}` 到商店最新（或给的）版本。新版只多要了能力时才要同意，披露里只列多出来的那几项；失败留着原版本，
  数据保留。
- **卸载**：`harnix.apps.uninstall {id}`：停下，撤掉安装放进去的一切（Agix 插件、skill、界面），删掉它的目录，连同数据。
- **清数据**：`harnix.apps.clearData {id}`：只删它存的东西（数据目录与 kv）；在跑的先停、清完再起（`init` 从空开始）。
- **从文件装**：`harnix.apps.installFile {path}`：签了的验签后装；未签名的只在 `[apps] dev = true` 时装。
- **界面**：设置 → 应用（启停、权限、数据大小与清空、卸载、更新；披露是一个模态框），系统 space 的应用页签（只看，[系统 space](system-space.md) §6）。
  左栏里应用的 space 带一个小标记，设置里可以关掉。
- 产品可以换自己的商店（`[apps] store`）与受信发布者，也可以不开商店，只用开发模式的 `installFile`。

## 10. `harnix-app` 工具

作者与商店用的命令行（`cargo build --release -p harnix-app-package --features compile --bin harnix-app`，带编译器）：

| 命令 | 做什么 |
|---|---|
| `new <目录> --id <发布者.名字> [--title <t>]` | 起一个应用：清单、一个带一个工具 `hello` 的 `tools-app` 组件（`component/`，WIT 在 `component/wit/app.wit`）、README；`build` 层每次都把它编一遍 |
| `build <应用目录> [-o <目录>]` | 在 `component/` 里 `cargo build --release --target wasm32-wasip2`，再为这个引擎编译（基线 CPU）到 `build/`；打印 `.wasm`、`.cwasm` 与指纹 |
| `pack <应用目录> [--wasm <组件>] [--build] [-o <out.hxapp>]` | 打包：清单、`ui/`、`agent/`、组件的两种形式；缺省 `<id>-<version>.hxapp` |
| `keygen <钥文件>` | 生成 ed25519 钥：文件里是私钥（0600，不覆盖），打印公钥 hex |
| `sign <包> --key <钥文件> [-o <out>]` | 以发布者签每一个文件 |
| `verify <包> [--store-key <hex>]… [--trust <发布者>=<hex>]…` | 验，打印 id、版本与信任等级 |
| `countersign <包> --key <商店钥> [--artifact <x.cwasm>]… [-o <out>]` | 商店用：查发布者的签名，加编译产物，副署 |
| `index <商店目录> [--name <名字>]` | 商店用：从 `packages/*.hxapp` 写出 `index.json` |
| `fingerprint` | 打印这个引擎的指纹 |

工具链：`rustup target add wasm32-wasip2`；rustc 的 wasip2 链接器直接出组件，不需要别的工具。

## 11. 测试

| 测什么 | 怎么测 |
|---|---|
| 界面页面 | `@harnix/app-sdk/mock` 的 `createMockHost({ app, point, hub, methods, requests, … })`：页面 `connect({ port: host.port })`，脱离 Harnix 单测；`host.requests`、`host.refused`、`host.publish(…)`、`host.setEnv(…)` |
| 组件 | 照 Rust 的单测写；`harnix-app build` 出产物 |
| 整个应用对着真的产品 | LIVE：带 `apps` 编的二进制，profile 加 `[apps] enabled = true`、`dev = true`，`harnix.apps.installFile {path}` 装你的 `.hxapp`，再像任何客户端一样调它的方法、订它的主题、建会话让 Agent 用它的工具 |
| Agent 用工具的那一轮 | 手做的录像：模型调 `app__<id>__<tool>`，见 [测试](testing.md) §6 |

```ts
import { startHarnix, connect } from "@harnix/testkit";

const backend = await startHarnix({
  bin: process.env.HARNIX_APPS_BIN,             // 带 apps 编的二进制
  profile: "[apps]\nenabled = true\ndev = true\n",
  replay: true,
});
const api = await connect(backend);
await api.call("harnix.apps.installFile", { path: "/abs/path/acme.notes-0.1.0.hxapp" });
await api.call("app.acme.notes.notes.add", { text: "hi" });
```

Harnix 自己怎么测插件 APP（宿主核心的回环、商店的 mock 钥、休眠的隐藏钩子）见 [测试](testing.md) §8。

## 12. 例子：showcase

`examples/apps/showcase`（id `example.showcase`）每样东西一个：带方法、主题与两个 space 的后端，工具（回声、计数、读项目、慢的、被否决的……），
事件钩子，Agent 包与 skill，进页面的模块与每一个面，四种行内点。[它的 README](../examples/apps/showcase/README.md) 说每一部分是什么、
怎么构建与打包。Harnix 的 LIVE 套件全拿它测。

---

# 第二部分：接口清单

## 13. 版本

| 名字 | 现在 | 在哪 | 什么时候升 |
|---|---|---|---|
| 应用接口 | `1` | 清单的 `[app] harnix_api`；`harnix_app_package::HARNIX_API` | 破坏性的变化（改签名、删函数、新的必需导出）。不等就拒装 |
| WIT 包 | `harnix:app@0.1.0` | `crates/wasm/wit/app.wit` | 只增函数时升补丁版本；老组件照样链接 |
| 桥 | `harnix-ui-bridge@1` | `@harnix/app-sdk/protocol` 的 `PROTOCOL` | 协议有不兼容的变化 |
| SDK | `0.1.0` | `@harnix/app-sdk` 的 `SDK_VERSION`，`hello` 时带给外壳 | |
| 签名 / 目录格式 | `harnix-app-signatures@1` / `harnix-store-index@1` | `SIGNATURES` / `index.json` 的 `format` | |

## 14. 清单字段

| 节 · 字段 | 类型 | 缺省 | 规则 |
|---|---|---|---|
| `[app] id` | string | 必填 | `<发布者>.<名字>`，点分，每段小写字母、数字、`-`，字母开头 |
| `[app] version` | string | 必填 | `x.y.z`，可带 `-<字母数字与 .>`，至多 64 |
| `[app] harnix_api` | int | 必填 | 等于 `HARNIX_API`（1） |
| `[app] publisher` | string | 必填 | id 的第一段 |
| `[app] title`、`description` | string | 无 | |
| `[capabilities] net` | [string] | `[]` | 精确的主机名：不含 `/ : @ *` 与空格；本机与局域网的地址披露为 `net_local`，商店不副署 |
| `[capabilities] env` | [string] | `[]` | 字母、数字、`_`；值来自 profile 的 `[apps.secrets."<id>"]` |
| `[capabilities] storage_mb` | int | 0 | `fs` 有 `${data}` 时要大于 0 |
| `[capabilities] hub` | [string] | `[]` | 点分的方法名，或 `topic:<主题>` |
| `[capabilities] fs` | [string] | `[]` | 以 `${data}` 或 `${project}` 开头 |
| `[[component]]` | 表数组 | 无 | 至多一个 |
| `… name` | string | 必填 | 小写字母、数字、`-`，至多 64 |
| `… world` | string | 必填 | `harnix:app/tools-app@0.1.0` 或 `harnix:app/backend-app@0.1.0` |
| `… memory_mb` | int | 64 | 1–4096 |
| `… timeout_ms` | int | 30000 | 100–600000 |
| `[events] tools` | string | `own` | `own` 或 `all`；只在有组件时；`all` 进披露 |
| `[backend] resident` | string | `always` | `always` 或 `on-demand`；只有 backend-app |
| `[backend] idle_secs` | int | 600 | 60–86400 |
| `[[method]] name` | string | 必填 | 点分，字母、数字、`_`、`-`，至多 128，唯一；只有 backend-app |
| `… subject` | string 或 [string] | `"device"` | `device`、`module`、`kernel` |
| `… description` | string | 无 | |
| `… params` | 表 | 无 | 一份 JSON Schema（写成 TOML） |
| `[[topic]] name`、`description` | string | | 同方法名的规则；只有 backend-app |
| `[[space]] id` | string | 必填 | 应用的 id 或 `<id>.<名字>`，唯一 |
| `… scene`、`instance` | string | 无 | |
| `[config]` | 表 | `{}` | 任意；profile 的 `[apps.config."<id>"]` 按顶层键覆盖 |
| `[interface] isolate` | bool | false | |
| `[interface] hub` | [string] | `[]` | `FRAME_HUB` 白名单里的名字（§6.3）；外壳自己的家族、别的名字都拒 |
| `[interface] links` | bool | false | |
| `[[surface]] point` | string | 必填 | `space`、`space.sidebar`、`space.tab`、`space.secondary`、`details.panel`、`settings.section`、`home.card`、`modal` |
| `… id` | string | 点名 | 小写字母、数字、`-`；`space.tab`、`settings.section`、`modal` 必填；同一点（与 space）里唯一 |
| `… page` | string | — | `ui/` 下的 `.html`；只有 `center = "chat"` 的 space 没有 |
| `… title` | string 或 `{en, zh-CN}` | 无 | |
| `… space` | string | — | `space*` 的面必填，是自己的 space；别的点不许写 |
| `… icon`、`order`、`center` | | `center = "hidden"` | 只有 `point = "space"`；`center` 是 `hidden`、`minimized`、`chat` |
| `… kinds` | [string] | `[]` | 只有 `details.panel`，必填，每个是 `<id>.<名字>` |
| `… wide` | bool | false | 只有 `modal` |
| | | | 一个 space 至多一个 `space.sidebar`、一个 `space.secondary`；`space.*` 的面所在的 space 要有 `point = "space"` 的面 |
| `[[inline.tool_row]]` | `tool`、`icon?`、`title?`、`summary?`、`collapsed`（缺省 true） | | `tool` 是组件的工具 |
| `[[inline.flow_block]]` | `kind`、`icon?`、`title`、`body?`、`field[] {label, value}`、`button[] {label, method, params?, confirm?, done?}` | | `kind` 点分、唯一；至多 12 个字段、4 个按钮；文本至多 200 字，正文 4000 |
| `[[inline.command]]` | `trigger`（`/` 或 `@`）、`name`、`description?` | | 要有 `[[method]] name = "complete"` |
| `[[inline.sidebar_group]]` | `space?`、`by`（`scene` / `field` / `method`）、`field?`、`method?`、`other?` | | `space` 是自己的 `[[space]]`（缺省应用的 id），每个 space 一次；不能和那个 space 的 `space.sidebar` 并存 |
| `[[ui]]` | `point`、`module`、`api_version` | | `module` 是 `ui/` 下的 `.js`；`api_version` 等于贡献点 API（2） |
| `[agent] install_into` | [string] | `["system"]` | `system`，或安装时已有的实例的 id |

## 15. WIT

```wit
interface host {
  log: func(level: string, msg: string);                     // error | warn | info | debug，别的按 info
  now-ms: func() -> u64;
  config: func() -> string;                                   // [config] 加 profile 的覆盖，JSON
  progress: func(call-id: string, text: string);              // 只在工具调用里有去处
  http-request: func(method: string, url: string, headers: list<tuple<string, string>>,
                     body: option<list<u8>>) -> result<http-response, string>;   // net；不跟随重定向
  secret: func(key: string) -> option<string>;                // env；profile 的 [apps.secrets."<id>"]，不读进程环境
  kv-get: func(key: string) -> result<option<list<u8>>, string>;    // none 是没有值；错误是存储读不了
  kv-set: func(key: string, value: list<u8>) -> result<_, string>;   // 超上限、键为空或超过 120 字节时失败
  kv-delete: func(key: string) -> result<_, string>;                 // 删一个没有的键也成功
  call: func(method: string, params-json: string) -> result<string, string>;     // 以 module:<id>，hub
  publish: func(topic: string, value-json: string) -> result<_, string>;         // app.<id>.<topic>
  subscribe: func(topic: string, params-json: string) -> result<_, string>;      // 只后端；hub
  unsubscribe: func(topic: string, params-json: string);
  set-timer: func(id: string, after-ms: u64) -> result<_, string>;               // 只后端；有数量上限，不过重启
  cancel-timer: func(id: string);
}
```

| 类型 | 字段 |
|---|---|
| `tool-def` | `name`（应用内唯一）、`description`（一行，每次都进模型的工具表）、`doc`（长的用法，按需取）、`input-schema`（JSON Schema）、`read-only`、`concurrency-safe` |
| `tool-output` | `content`（给模型的）、`structured`（JSON，可选）、`is-error`。错误结果是工具在回答，只有陷阱才是工具失败 |
| `hook-decision` | `proceed`、`block(string)`、`add-context(string)`。没有「改写」：悄悄改别人的工具调用和拒绝它是两种权力，下载来的包不给前一种 |
| `http-response` | `status`、`headers`、`body` |
| `app-error` | `code`（应用自定，原样转给调用方）、`message`、`data-json` |

`handle` 的 `caller-json` 是 `{kind, id, subject}`（例如 `subject: "device:d1"`）；hub 已经准入过。

## 16. `harnix.apps.*`

内建模块 `apps`，只在 `[apps] enabled = true`（且 `with_builtins`）时注册；主体是设备（内核也可以）。改东西的调用一个一个来。
不带插件 APP 运行时的构建（没开 cargo feature `apps`）照样有记录与开关，`installFile`、`catalog`、`install`、`update` 答 `REFUSED`
（「this build carries no application runtime …」）。

| 方法 | 参数 | 答 | 要知道的 |
|---|---|---|---|
| `list` | — | 快照（下） | |
| `installFile` | `path` | `{id, version, title, enabled, trust, source: "file", installed}` | 未签名的要 `[apps] dev = true`；同 id 已装就拒（先卸载）；全有或全无 |
| `catalog` | — | `{name, store, fingerprint, apps: [{id, title, publisher, description, latest, installable, capabilities, versions: [{version, harnix_api, installable, engines}], installed?}]}` | `installed` 是这里装着的版本 |
| `install` | `id`，`version?`，`consent?` | 第一次（或同意过期）：`{id, version, consent: "required", digest, disclosure, changed}`；同意对得上：`{id, version, title, enabled: true, trust, source: "store", installed}` | 已装就拒（用 `update`）；全有或全无 |
| `update` | `id`，`version?`，`consent?` | 已是最新：`{id, version, latest, up_to_date: true}`；多要能力：同 `install` 的第一步；否则 `{id, version, from, title, enabled, trust, updated}` | 数据保留，失败放回原版本 |
| `enable` / `disable` | `id` | `{id, enabled, registered, withdrawn, runtime, started}` | 启用时起不来：记录退回停用，答错误 |
| `uninstall` | `id` | `{id, uninstalled: true, removed: {agent: {package, instances} \| null, skills}}` | |
| `clearData` | `id` | `{id, cleared_bytes, data_bytes, restarted, started}` | 记进审计 |

`installed` / `updated` 是 `{fingerprint, trust, agent: {package, instances, disclosure} | null, skills, started}`。没装的 id 答 `INVALID_PARAMS`。

**快照**（`list` 的答，也是主题 `harnix.apps` 的值）：

```text
{ dir,
  apps: [{ id, version, title, enabled, registered, trust, source, data_bytes, runtime?, app? }],
  unreadable: [{ dir, reason }],
  registered: [{ id, by, methods, topics, spaces, hub?, since_ms }],
  runtime: { fingerprint, dev } | null }
```

- `trust`：`store` / `trusted` / `unsigned`；`source`：`store` / `file`。
- `app`：清单给人看的那份：`{publisher, description, capabilities {net, env, storage_mb, hub, fs}, events, lines, component {world, memory_mb}, ui, inline}`（`events` 是 `[events] tools`）。
- `runtime`：在跑的应用的状态：

```text
{ state, reason, resident, idle_ms, world, methods, topics, spaces, hub,
  calls, tools: [{ name, tool, description }], health,
  ui, frames, surfaces, ui_rev, mcp, inline }
```

| 字段 | 是什么 |
|---|---|
| `state` | `running` · `hibernating`（休眠中）· `faulted`（被置于一旁）· `failed`（没起来，`reason`）· `stopped` |
| `world` | `tools` 或 `backend` |
| `calls` | `{calls, errors, methods: [{name, calls, errors}], hub_calls, published, dropped: {publishes, topic_values, progress}, recent_errors: [{method, code, message, at_ms}], refused: [{method, reason, at_ms}]}`，两个列表新的在前，各留 20；`dropped` 是队列满了丢掉的（每条队列至多 256） |
| `health` | `app`、`world`、`state`（`running` / `stopped` / `disabled` 加 `reason`）、`calls`、`faults`、`timeouts`、`consecutive_faults`（哪个工具眼下连续出错最多的次数）、`tools_aside`（被放在一旁的工具 → 为什么）、`rebuilds`、`recycles`、`storage_recycles`、`queue_timeouts`、`dropped_deliveries`、`last_error`、`memory_bytes`、`memory_peak_bytes`、`memory_limit_bytes`、`storage_bytes`、`storage_files_bytes`、`storage_limit_bytes`、`subscriptions`、`timers`、`hibernating`、`wakes` |
| `tools` | 会话里的名字（`app__…`）、组件里的名字、说明 |
| `ui`、`frames`、`surfaces` | 进页面的模块（`/apps/<id>/ui/<模块>`）、是否进 iframe、面（`<点>:<id>`） |
| `ui_rev` | 开发模式下 `ui/` 的版本，变了外壳重新加载 iframe |
| `mcp` | `/apps/<id>/mcp` |
| `inline` | 清单的 `[inline]`，外壳照它画 |

## 17. 主题

| 主题 | 参数 | 值 | 什么时候推 |
|---|---|---|---|
| `harnix.apps` | — | 同 `harnix.apps.list` | 后端每秒看一次，变了才推；设备与模块可订 |
| `harnix.apps.progress` | — | `{app, tool, session_id, tool_use_id, text}`（`tool` 是 `app__…` 全名） | 进程内会话的工具调用每次 `host.progress`；只在带运行时的构建里 |
| `harnix.spaces` | — | 同 `harnix.spaces.list` | 插件 APP 加减 space 时；只在 `[apps] enabled` 时有 |
| `app.<id>.<主题>` | 无 | 后端 `host.publish` 的值 | 设备与模块可订 |

应用撤下时，它的主题的每个订阅者按订阅的参数组各收到一次 `harnix.topic_ended { topic, params, reason }`，之后再订答 `-32601`。

## 18. 谁能调什么

| 谁 | 能调 | 怎么判 |
|---|---|---|
| 设备（页面、外壳） | `app.<id>.<方法>`，`subject` 含 `device` 的 | 授权表，和编译期模块的方法同一个准入点、同一套审计 |
| 任何模块 | `subject` 含 `module` 的 | 同上 |
| 内核 | 都能 | |
| 应用自己（`module:<id>`） | 自己的全部方法；`hub` 能力列出的方法与主题（`topic:`） | 登记时 `hub` 的条目像模块的 grant 一样写进授权表（指向别的应用 `app.*` 的跳过，由那个应用的 `subject` 决定）；没列的答 `REFUSED`「it is not in the application's `hub` capability」，记审计，进 `calls.refused` |
| 应用的界面（iframe） | 自己的 `app.<id>.*`，`[interface] hub` 列出的（白名单里的） | 外壳先对清单查名字、按限定收窄到它自己 space 的会话（§6.3），再由 hub 当作设备准入 |
| 会话（Agent） | 不直接调方法；用应用的工具 | Agix 的权限，一律先问人 |

## 19. profile `[apps]`

```toml
[apps]
enabled = false          # true：harnix.apps.*、模块的应用宿主、harnix.spaces 主题；不写或 false 时一概没有，授权表就是装配出来的那张
dev = false              # true：installFile 装未签名的包，未签名的应用能启动。开发用：没有人为这样的包担保
store = "https://…/index.json"   # 商店目录（https:// 或本机 file://）；不写用内置的。列什么都要验签才装

[apps.trusted_publishers]        # 发布者 id → 以它签名的 ed25519 公钥（hex）；它签全的包不经商店副署也能装（trusted）
acme = ["<64 位 hex>"]

[apps.config."acme.notes"]       # 按键盖过应用自己的 [config]
greeting = "hi"

[apps.secrets."acme.notes"]      # 它的 host.secret 能读的值，按它 env 里声明的名字；只有这里，不读进程的环境变量
ACME_TOKEN = "…"
```

每一段都拒绝不认识的键；钥写错了进程起不来，并说是哪个发布者。缺省商店是 `https://store.harnix.dev/index.json`（占位），内置的商店公钥是
`harnix_app_package::store::STORE_KEYS`。cargo feature `apps` 不开时，`[apps] enabled = true` 也只有记录与开关（§16）。

## 20. 桥：`harnix-ui-bridge@1` 与 `HarnixApp`

消息三种：`{kind: "request", id, method, params?}`、`{kind: "response", id, result? | error?}`、`{kind: "event", name, data?}`。

**页面请外壳的**（`HarnixApp` 上的方法 → 桥的请求）：

| 方法 | 请求 | 答 |
|---|---|---|
| `connect(options?)` | `hello {sdk}` | `HarnixApp`；`options`：`port`（测试给 mock 的）、`timeoutMs`（缺省 10000）、`autoHeight`、`keys`（缺省 true） |
| `surface`、`env` | — | 它是哪个面：`{app, point, id, space, params, autoHeight, links}`；外观：`{theme, tokens, locale, narrow, visible, modal}` |
| `onEnv(fn)`、`onParams(fn)` | — | 外观变了；模态框、二级栏、详情面板拿到新的参数 |
| `call(method, params?)` | `call` | 方法的结果 |
| `subscribe(topic, params, fn)` | `subscribe` / `unsubscribe` | 返回取消函数，带 `.ready`（被拒时 reject） |
| `session.current() / open(ref) / create({scene?, options?}) / onChange(fn)` | `session.*` | 它的 space 打开的会话；`SessionRef = {instance, sessionId}` |
| `spaces.switch(space)` | `spaces.switch` | |
| `details.open(tab?)`、`details.show(item)` | `details.*` | `item.kind` 是它自己的 `<id>.<名字>` |
| `secondary.open(params?, title?)`、`secondary.close()` | `secondary.*` | |
| `settings.open(section?)` | `settings.open` | |
| `composer.insert(text)` | `composer.insert` | |
| `badge(value)` | `badge` | 它的 space 入口上的角标；null、0、"" 清掉 |
| `notify(text, level?)` | `notify` | `info`（缺省）或 `error` |
| `copy(text)` | `copy` | |
| `openModal(id, params?)` | `openModal` | 模态框关上时给的值；被关掉是 null |
| `close(result?)` | 事件 `close` | 在模态框里：关上并答给打开它的人 |
| `confirm(text, {title?, confirm?, danger?})` | `confirm` | 人确认了是 true |
| `openLink(url)` | `openLink` | 打开了是 true；要 `[interface] links` |
| `upload(file: Blob)` | `upload {name, type, data}` | `{path}` |
| `disconnect()` | — | |

**页面发给外壳的事件**：`resize {height}`、`key {key, code, meta, ctrl, shift, alt}`、`close {result?}`、`error {message}`。
**外壳发给页面的**：请求 `ping`（心跳，答 `pong`）；事件 `env`、`topic {sub, value}`、`topic.end {sub, reason}`、`session {current}`、`params {params}`。

**mock 宿主**（`@harnix/app-sdk/mock`）：`createMockHost({app, point, id, space, params, autoHeight, links, hub, env, current, methods, requests})` →
`{port, requests, events, refused, env, publish(topic, value), setEnv(change), setCurrent(ref), setParams(params), ping(), next(name), close()}`。

## 21. 错误

**方法**：后端没给出答案时答

```text
{ code: -32603 /* INTERNAL_ERROR */, message: "`app.<id>.<方法>`: <为什么>",
  data: { app: "<id>", failure: "<原因>" } }
```

| `failure` | 什么时候 |
|---|---|
| `stopped` | 应用没在跑 |
| `timed_out` | 超过 `timeout_ms`；后端实例换掉 |
| `cancelled` | 调用被取消 |
| `faulted` | 应用的代码陷阱 |
| `unavailable` | 应用被置于一旁（「`<id>` is disabled: …」）；排队等后端超过 `timeout_ms`（「busy: …」）；或这次调用会重入一个正在等它自己的后端 |

后端自己答的错误（`app-error`）原样转给调用方：`code`、`message`、`data`。参数不符 schema 答 `INVALID_PARAMS`。应用的 `host.call` 越权答
`REFUSED`（-31000）。

**工具**：进程内被取消的调用是 Agix 的取消；别的失败是一条错误结果「`<id>: <消息>`」（被放在一旁的工具：「the tool `<名>` of `<id>` is set aside: …」）。
经 MCP：取消是 -32800，别的失败是 `isError: true`；不是本机的对端答 HTTP 403。

**桥**：`error.code` 是 `refused`（外壳不替这个应用做：清单没列的名字、别的 space 的会话、不许开的链接）、`failed`（做了，失败了；后端的错误是消息）、
`unknown`（没有这个请求）、`invalid`（参数不对）、`closed`（答案回来之前桥没了）、`timeout`（SDK 自己的连接超时）。SDK 里是 `BridgeError`。

## 22. 名字

| 东西 | 名字 |
|---|---|
| 应用 id | `<发布者>.<名字>`，例如 `acme.notes` |
| 方法、主题 | `app.<id>.<名字>`，例如 `app.acme.notes.notes.list` |
| 后端 space、界面 space | `<id>` 或 `<id>.<名字>` |
| 会话里的工具 | `app__<id 的 . 换成 _>__<工具>`，例如 `app__acme_notes__search` |
| spawn 的 daemon 里的工具 | `mcp__app-<id 的 . 换成 _>__<工具>` |
| MCP 端点 | `POST /apps/<id>/mcp`（只答本机的对端；`Authorization: Bearer <令牌>`；`initialize`、`ping`、`tools/list`、`tools/call`） |
| 界面文件 | `/apps/<id>/ui/<路径>` |
| 详情项的种类 | `<id>.<名字>` |
| 扩展条目的命名空间 | `app.<id>` |
| 主体 | `module:<id>` |
| 编好的组件 | `wasm/<组件>.<指纹>.cwasm`；指纹 `wasmtime-<版本>+<目标>+<12 位哈希>` |

## 23. 目录

```text
~/.<product_id>/apps/<id>/install.json   {id, version, title?, enabled, parts?}；parts.trust、parts.source
~/.<product_id>/apps/<id>/<version>/     解开的包
~/.<product_id>/apps/<id>/data/          /data（fs ${data}）
~/.<product_id>/apps/<id>/kv/            键值存储
```

`data/` 与 `kv/` 都算 `storage_mb`；卸载时整个目录删掉。引擎指纹另在 `harnix.process` 的 `apps_engine`、`harnix.apps.list` 的
`runtime.fingerprint` 里，`harnix-app fingerprint` 打印它。
