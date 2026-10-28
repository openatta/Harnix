# Harnix SDK 参考

> 读者：在 Harnix 上做应用的人，以及在 Helm、IPDrun 里做迁移分析的人（包括 Agent）。
>
> **这份参考写的是已经落地的接口**，对照的是 0.4 的全部里程碑（M0–M9，Agix 钉在
> `Agix-v0.2.9-2026102603`）。怎么用、为什么这样用见 [二开指南](harnix-sdk.md)；Helm、IPDrun 怎么迁过来见
> [迁移说明](migration-helm.md)、[IPDrun](migration-ipdrun.md)；用词照 [术语](glossary.md)。
>
> 写法：签名照抄代码；默认值照抄代码。代码和这里对不上时以代码为准，并且这里是错的，要改。

---

## 0. 版本号

| 名字 | 现在 | 在哪 | 什么时候升 |
|---|---|---|---|
| Harnix 版本 | `0.4.0-2026102602` | 各 crate 的 `Cargo.toml`；`harnix::HARNIX_VERSION` | 每次发布 |
| 模块 API | `1` | `harnix::MODULE_API_VERSION` | 模块契约有模块看得见的变化。**编译期**检查：`Module::API_VERSION` 不等就编译不过 |
| 贡献点 API | `2` | `harnix::CONTRIB_API_VERSION`；`harnix.hello` 里也有 | 贡献点的契约变了。2：运行期包的界面模块是预编译的 ES 模块，从页面的 import map 拿 React；**React 的大版本是它的一部分** |
| 协议 | `4` | `harnix_contract::PROTOCOL_VERSION` | 握手比对，不一致拒绝（`HANDSHAKE_REFUSED`） |

`<app> --version` 打印上面全部，加上 Agix 的版本。0.4.z 内协议与模块 API 只做加法。

---

## 1. Rust：`harnix` crate

应用只依赖 `harnix` 一个 crate（它是 `harnix-app` 与 `harnix-module` 的再导出）。模块源码一般只要
`use harnix::prelude::*;`。

```text
harnix::{App, AppBuilder, ModuleStatus, Namespace, HARNIX_VERSION, SHUTDOWN_GRACE,
         codes, RpcError, Subject, SYSTEM_INSTANCE,
         AttachTo, CallContext, CredentialRef, Credentials, EngineContrib, EngineProfile, Entry,
         EnvCredentials, EventRegistry, Events, Handler, InstanceHandle, InstanceTemplate,
         InstanceTemplates, Instances, Listen, Manifest, MethodRegistry, MigrateContext, MigrationReport,
         Module, ModuleContext, OnBusy, Principal, RouteRegistry, Secret, SessionRef, SessionSpec,
         Sessions, SpaceDecl, SpaceRegistry, Tap, TapEvent, TemplateCarrier,
         CONTRIB_API_VERSION, MODULE_API_VERSION, SETTLED_HELD, TAP_CAPACITY,
         async_trait, axum}
```

### 1.1 `App::builder()` → `AppBuilder`

| 方法 | 说明 |
|---|---|
| `.name(name, version)` | 应用名与版本：`--version`、`--help` 用它。界面上的产品名、图标、产品 id 由 profile 的 `[brand]` 给 |
| `.with_builtins()` | 加上 Harnix 自带的模块（工作区、文件、搜索、设置、插件包、系统 space、设备……）。一般都要 |
| `.module(m)` | 加一个模块；**顺序就是注册、`migrate`、`start` 的顺序**，也是 customizer 的组合顺序 |
| `.credentials(source)` | 换掉凭据源（默认 `EnvCredentials`：`helm/quote-api` → `$HARNIX_CREDENTIAL_HELM_QUOTE_API`） |
| `.profile_from_args()` / `.args(iter)` | 命令行从哪来；测试里用 `.args` |
| `.run()` | 跑起来，`anyhow::Result<()>` |

### 1.2 `trait Module`

```rust
#[harnix::async_trait]
pub trait Module: Send + Sync + 'static {
    const API_VERSION: u32;                                     // 写 harnix::MODULE_API_VERSION
    fn manifest(&self) -> Manifest;                             // 唯一必须实现的
    fn engine(&self, _: &mut EngineContrib) {}                  // 给 system 实例的原生贡献
    fn templates(&self, _: &mut InstanceTemplates) {}           // 实例模板
    fn spaces(&self, _: &mut SpaceRegistry) {}                  // 后端侧的 space 声明
    fn methods(&self, _: &mut MethodRegistry) {}                // app.<id>.*
    fn events(&self, _: &mut EventRegistry) {}                  // app.<id>.* 主题
    fn routes(&self, _: &mut RouteRegistry) {}                  // /apps/<id>/
    async fn migrate(&self, _: &MigrateContext) -> anyhow::Result<MigrationReport> { Ok(MigrationReport::none()) }
    async fn start(&self, _: ModuleContext) -> anyhow::Result<()> { Ok(()) }
    async fn stop(&self) {}
}
```

顺序：所有模块的注册函数各调一次 → 按顺序 `migrate` → 建实例 → 按顺序 `start` → 开始服务。注册冲突、
`migrate` 或 `start` 返回 `Err`：只停用这个模块和它的方法、主题、路由、space、实例，原因进诊断页。

退出：SIGINT、SIGTERM、`--exit-with-stdin` 下 stdin 关闭，都是倒序调各模块的 `stop`，再停实例、删本进程的上传目录。
整个收尾最多 `SHUTDOWN_GRACE`（10 s）；超了，或收尾中再收到一次 SIGINT / SIGTERM，记一条日志后以 1 退出。

### 1.3 声明

| 类型 | API |
|---|---|
| `Manifest` | `Manifest::new(id, version)`、`.disclosure(text)`（诊断页的披露文本）、`.grant(method)`（要设备够不着的方法，比如 `config.setProvider`；中枢独占的方法不能 grant）。id：小写字母、数字、`-`，以字母开头 |
| `MethodRegistry` | `m.method(name, Principal)` → `.allow(Principal)`、`.schema(json)`、`.describe(text)`，最后 `.handler(f)` / `.raw(f)` / `.with(Arc<dyn Handler>)`。全名 `app.<id>.<name>` |
| handler | `async fn(cx: CallContext, p: P) -> Result<R, RpcError>`，`P: DeserializeOwned`（不符返回 `INVALID_PARAMS`），`R: Serialize`。`.raw` 收发 `serde_json::Value` |
| `EventRegistry` | `e.topic(name)` → `.param(name)`（按参数分值）、`.only(&[Principal])`（默认设备与模块都能订阅）。全名 `app.<id>.<name>` |
| `Events` | `cx.events().publish(topic, params_json, value).await`：推**整值快照**；`topic` 写短名 |
| `RouteRegistry` | `r.serve(&[Principal], axum::Router)`：挂在 `/apps/<id>/` 下，同源，同一套准入 |
| `SpaceRegistry` | `s.add(SpaceDecl::new(id).instance("template:<模块>.<模板>" \| "<实例 id>").scene(scene))`。不声明：`system` 实例、实例的默认场景 |
| `InstanceTemplates` | `t.add(InstanceTemplate::new(id, carrier).data_root(pattern).param(name).ui().max(n).inherit_providers())`。`inherit_providers()`：这个模板的实例也看得到 `system` 的 provider（设置页配的）——建实例时、以及每次 `harnix.settings.setProvider` 之后，Harnix 把它们并进实例数据根的 `settings.json`（0600）再让它 `config.reload`；Harnix 写过哪些 id 记在 `.inherited-providers.json`，只动这些，实例自己配的同名 provider 不覆盖。全名 `<模块 id>.<模板 id>`；`data_root` 默认 `{name}`，可用 `{name}` 和声明过的参数 |
| `TemplateCarrier` | `InProcess { engine, contrib }`（只有模块的模板能带原生贡献）、`Spawn { engine, program: Option<PathBuf>, args }`（`None` = 本二进制的 `daemon` 子命令）、`Attach(AttachTo { socket, tcp, token })` |
| `EngineContrib` | `.scene(Arc<dyn AgentScene>)`、`.customize_session(\|builder, cx\| …)`、`.credentials(Arc<dyn CredentialSource>)`、`.extend(other)` |
| `EngineProfile` | `scene = "coding"`、`scenes = []`、`model = "claude-sonnet-4-6"`、`max_tokens = 2000`、`session_cap = 32`、`session_idle_timeout_secs = 3600`、`permission_prompt_timeout_secs = 300`（括号里是默认值） |

### 1.4 `Principal`

| 值 | 是谁 |
|---|---|
| `Device` | 一台配对过的设备，也就是一个人（界面上的点击） |
| `Module(String)` | 某一个模块 |
| `AnyModule` | 任何模块 |
| `Kernel` | 本二进制自己 |

方法、路由默认拒绝；声明时给一个主体，`.allow` 再加。

### 1.5 运行期：`ModuleContext`、`CallContext`

| `ModuleContext` | 说明 |
|---|---|
| `id()` | 模块 id |
| `config::<T>()` / `config_value()` | profile 的 `[modules.<id>]` 段 |
| `data_dir()` | 这个模块自己的数据目录（现在是 `<状态目录>/modules/<id>`，见 §4） |
| `credentials()` | `&dyn Credentials`：`.get(reference) -> Result<Secret, String>` |
| `events()` | `&Events` |
| `call(method, params)` | 以 `Module(<id>)` 的身份调任何方法，和设备走同一个准入点、同样审计 |
| `sessions()` | `Sessions`，见 §1.6 |
| `instances()` | `Instances`：`create(template, name, params)`、`ensure(template, name, params)`（幂等，并发调同名也只建一台，`start` 里用）、`ensure_ready(template, name, params, timeout)`（同上，等到 `ready` 再返回；`down` 或超时报 `INSTANCE_UNAVAILABLE`）、`wait_ready(instance, timeout)`、`stop(instance)`（数据根留着） |
| `instance(id)` | 只读的 `InstanceHandle` |
| `listen()` / `listening().await` | 进程监听在哪：`Listen { addrs, url }`（`addrs` 是实际绑上的地址，`--port 0` 时是系统给的端口；`url` 是 `http(s)://<第一个地址>/`，不带令牌）。模块的 `start` 在监听**之前**，所以 `start` 里 `listen()` 是 `None`；要写运行记录就 `tokio::spawn` 一个任务 `listening().await` |
| `provide(v)` / `provide_arc(arc)` / `state::<T>()` | 模块状态，handler 里用 `cx.state::<T>()` 拿 |

`CallContext`：`subject()`、`device()`（是设备调的就给设备 id）、`module()`、`state::<T>()`。
`MigrateContext { id, data_dir, config }`；`MigrationReport::none()` / `::applied(steps)`。

**没有**中枢内部状态，**没有**直连引擎的链接：所有会话操作都经过中枢。

### 1.6 会话：`ctx.sessions()`

| 方法 | 说明 |
|---|---|
| `create(SessionSpec) -> SessionRef` | `SessionSpec { space, instance, scene, project_root, provider, model, permission_mode, permission_rules, output_schema, recording }`，全是 `Option`，`..Default::default()` |
| `open(instance, session_id)` | 已有的会话（内存里的或只在磁盘上的） |
| `send(&s, text, OnBusy)` | `OnBusy::{Queue（默认）, Reject, Interrupt, Steer}` |
| `interrupt(&s)` / `close(&s)` | — |
| `append_entry(&s, Entry::new(event, payload))` | 扩展条目（M7）：命名空间由中枢定为 `app.<模块 id>`；只给人看 |
| `entries(&s) -> Vec<Value>` | 本模块写过的条目，最早的在前 |
| `tap(&s)` / `tap_with_capacity(&s, n)` | 帧流，默认容量 `TAP_CAPACITY = 512`。`tap.next()` → `TapEvent::Frame(json)` 或 `TapEvent::Lagged { last_seq }`；落后了 `tap.catch_up()`。`tap.next_settled() -> Option<Value>`：跳过别的帧，只等下一个 `harnix.turn_settled` 的 params；满了时结算帧另存（最多 `SETTLED_HELD = 64`），落后也不丢 |

`SessionRef { instance, session_id }`：会话的全名永远带实例。

### 1.7 错误：`RpcError`、`harnix::codes`

`RpcError::new(code, msg)`、`::invalid_params`、`::not_found`、`::refused`。

| 码 | 名字 | 含义 |
|---|---|---|
| -32700 / -32600 / -32601 / -32602 / -32603 | JSON-RPC 标准 | 解析、请求、没有这个方法、参数、内部 |
| -31000 | `REFUSED` | 方法存在，这个主体不许调（和「没有这个方法」是两回事） |
| -31001 | `HANDSHAKE_REFUSED` | 握手没谈拢：版本、贡献点 API、拓扑 |
| -31002 | `INSTANCE_NOT_FOUND` | 没有这台实例 |
| -31003 | `SPACE_MISMATCH` | 会话不属于调用方声称的 space。**防 bug，不是权限** |
| -31004 | `INSTANCE_UNAVAILABLE` | 实例在，但现在够不着（daemon 重启中、协议版本不符） |
| -31005 | `BUSY` | 这条单双向连接上已有 `limits.max_in_flight`（16）个请求没答；这一个没执行，等一个答了再发。`data.max_in_flight` 是那个数 |

Agix 自己的错误码原样透传（例如会话不存在是 -32000）。

---

## 2. profile（TOML）

**每一段都是 `deny_unknown_fields`**：写了不认识的键，进程拒绝启动并说哪一行。

```toml
[engine]                       # system 实例；字段同 EngineProfile（§1.3）
scene = "coding"
scenes = []
model = "claude-sonnet-4-6"
max_tokens = 2000
session_cap = 32
session_idle_timeout_secs = 3600
permission_prompt_timeout_secs = 300

[transport]
topologies = ["single_duplex", "split_streams"]   # 还有 "request_only"
host = "127.0.0.1"             # 非回环要 TLS 和配对过的设备
port = 4090
launch_link = false            # true：令牌不进页面，浏览器凭一次性启动链接换 cookie
extra_origins = []             # 回环之外允许的浏览器 Origin，例如 "https://192.168.1.20:4317"

[brand]
name = "Helm"                  # 页面标题、左栏产品名；不写是 Harnix
icon = "brand/helm.svg"        # 品牌图标（SVG），相对 profile 所在目录；服务在 /brand/icon.svg
product_id = "helm"            # 程序自己的目录 ~/.helm（§4）；小写字母、数字、`-`，字母开头；不写是 harnix

[shell]
spaces = ["sessions", "projects", "system"]   # 留哪些内建 space；不写是三个都留，[] 是一个不留
                               # 去掉 system 时它的方法一起撤：谁也声称不了系统 space，也不能跨 space 操作
model_switch = false           # true：加 harnix.sessions.setModel，没有系统 space 也能在输入框换模型（只限调用方声称的 space 的会话）

[ui]
dir = "…"                      # 从这个目录出界面（同 --ui-dir）；不写用编进二进制的
runtime_modules = true         # false：运行期包的 [[ui]] 一律不提供、不服务；交易类产品要关

[[instances]]                  # 固定实例
id = "ci"
carrier = "spawn"              # in_process | spawn | attach
data_root = "ci"               # 可选
program = "/path/to/daemon"    # 可选，spawn 用；默认本二进制的 daemon 子命令
args = []
[instances.engine]             # 同 [engine]
scene = "ci"
[instances.attach]             # attach 用
socket = "/run/agix/shared.sock"   # 或 tcp = "127.0.0.1:7878" + token = "<凭据名>"

[[instance_templates]]         # profile 里的模板只能 spawn 或 attach
id = "sandbox"
carrier = "spawn"
data_root = "sandbox/{team}/{name}"   # 默认 "{name}"
params = ["team"]
ui = true                      # 界面可以按它建
max = 4
inherit_providers = true       # 可选：它的实例也看得到「设置 → 模型」里配的 provider（并进实例的全局层，设置页一改就推过去）
[instance_templates.engine]
scene = "general"

[modules.watchlist]            # 各模块自己的段，模块用 ctx.config() 读
db = "watchlist.sqlite"
quote_api_key = { credential = "helm/quote-api" }
```

---

## 3. 命令行与环境变量

| 开关 | 说明 |
|---|---|
| `--profile <file>` | 上面的 profile；下面的开关只在给了的时候覆盖它 |
| `--port`、`--host` | 监听 |
| `--scene`、`--scenes a,b`、`--model` | `system` 实例的默认值 |
| `--ui-dir <dir>` / `--headless` | 换界面目录 / 不出界面（纯 RPC 节点） |
| `--replay-dir <dir>` | 模型调用改成录像回放（测试用；只有运维能开，客户端要不到） |
| `--tls-cert`、`--tls-key` | 绑非回环必须 |
| `--agix-dir` | 引擎目录，默认 `$AGIX_CONFIG_HOME`，再默认 `~/.agix` |
| `--data-dir` | 项目目录，默认 `$HARNIX_DATA_DIR`，再默认 `~/Documents` |
| `--launch-link` | 同 `[transport] launch_link` |
| `--exit-with-stdin`（隐藏） | stdin 关了就退出：桌面壳带走它的子进程 |
| `ui export <dir>` | 把编进去的界面写出来，给 CDN / 反代 |
| `daemon …`（隐藏） | spawn 的实例跑的就是它。Harnix 起它时带 `--instance <id>`：它在 `<数据根>/.daemon-<id>.lock` 上持独占锁（内容是 pid），先等同一台实例的旧 daemon 放手（最多 `--lock-wait` 秒，默认 10）再建引擎；等不到就非零退出，实例记为 `down`，理由「the data root is held by pid N」。模板写了 `program` 的不带这两个参数、不拿锁 |

| 环境变量 | 说明 |
|---|---|
| `AGIX_CONFIG_HOME` | 引擎目录 |
| `HARNIX_DATA_DIR` | 项目目录 |
| `HARNIX_STATE_DIR` | 程序自己的目录，单独挪（§4） |
| `HARNIX_CREDENTIAL_<名字>` | 默认凭据源读这里；名字是引用转大写、非字母数字转 `_` |
| `HARNIX_UI_MODULES`、`HARNIX_UI_OUT` | 界面构建：应用的界面入口（逗号分隔绝对路径）、输出目录（§6.5） |
| `HARNIX_UI_DIST` | `cargo build` 时嵌进二进制的界面目录 |

---

## 4. 目录

| 路径 | 是什么 |
|---|---|
| `~/.agix/` | 引擎：设置分层、transcript、记忆、技能、录像。和 `agixd` 共用 |
| `~/.<product_id>/`（不配 `[brand]` 就是 `~/.harnix/`；`HARNIX_STATE_DIR` 可挪） | 程序自己的配置与状态：`token`、`state.json`（工作区、会话标题、偏好、设备）、`spaces.json`、`instances.json`、`instances/<data_root>/`（按模板建的实例的数据根）、`modules/<id>/`（`ctx.data_dir()`）、`uploads/<pid>/`、`harnix.lock`（进程活着就持有的独占锁，内容 `{"pid", "url"}`，`url` 在监听后写上：一个状态目录只能有一个进程，第二个启动时报「`<目录>` is in use by pid N (url)」并以非零退出） |
| `~/Documents/` | 只放项目：选项目从这里开始，新建项目建在这里。Documents 里不放隐藏目录 |
| `<项目>/.agix/` | 项目自己的数据与配置**一律**放这里（Agix 的项目层设置也在这），不开 `<项目>/.<产品>/` |
| `~/Documents/.harnix/`（0.4.0-2026102602 及以前） | **不迁移、不读**。升级后旧的令牌、工作区分组、实例账本都从空开始 |

模块的数据（`ctx.data_dir()`）随产品 id 走；同一台机器上装 Helm 和 IPDrun 不会抢令牌和账本。

---

## 5. 线上协议

### 5.1 端点

| 路径 | 用途 |
|---|---|
| `/handshake` | 握手：协议版本、贡献点 API、拓扑 |
| `/ws` | `single_duplex`：一条双向连接 |
| `/rpc`、`/events/session`、`/events/host`、`/respond` | `split_streams` / `request_only` |
| `/pair`、`/challenge` | 设备配对 |
| `/auth/link`、`/auth/{code}` | 启动链接 |
| `/plugins/<包>/ui/<模块>` | 运行期包的界面模块文件（`[ui] runtime_modules = false` 时不服务） |
| `/brand/icon.svg` | `[brand] icon` |
| `/apps/<模块 id>/…` | 模块的自有路由 |

握手拿到的 `credential`：`Authorization: Bearer …` 或 `?credential=`；回环上也认进程令牌和启动链接的
cookie。一帧最大 16 MiB，一次上传最大 32 MiB。

### 5.2 方法

**权威清单在运行时**：`harnix.reachable` 列出当前主体能调的；`harnix.diagnostics` 列出全部（含主体、
schema、来自哪个模块）。下表是分组索引。

| 组 | 方法 | 谁 |
|---|---|---|
| 握手与会话驱动（中枢） | `harnix.hello`（含 `shell`：品牌、留下的内建 space、`runtime_modules`、目录）、`harnix.sessions`、`harnix.attach`、`harnix.detach`、`harnix.send`、`harnix.queue.remove`、`harnix.subscribe`、`harnix.unsubscribe` | 设备 |
| 进程（中枢） | `harnix.process`：版本、运行时长、峰值内存、已连接用户（设备、连接方式、User-Agent、连上的时间） | 设备 |
| 实例（中枢） | `harnix.instances.list / templates / sessions / create / ensure / stop / restart / wait`、`harnix.sessions.tasks`。`create` / `ensure` 可带 `wait: "ready"`、`timeout_ms`（缺省 60000）：等这台变成 `ready` 再答，变成 `down` 立即以 `-31004` 报错（带理由），超时同样；`wait {instance, timeout_ms?}` 是对已有实例的同一种等。并发 `ensure` 同名同模板都拿到同一台 | 设备（`create` 只认 `ui` 模板） |
| space、条目（中枢） | `harnix.spaces.list`、`harnix.entries.append`（只许模块与内核） | — |
| 引擎直通 | `session.create / list / get / history / entries / fork / resume / respondToPrompt`、`session.interrupt / close / delete`、`scene.list / describe / activate`、`commands.list`、`mcp.status`、`plugin.list`、`daemon.status / ping / doctor` | 设备 |
| 应用层 | `harnix.contributions`、`harnix.diagnostics`、`harnix.audit`、`harnix.reachable` | 设备 |
| 自带模块（`with_builtins`） | `harnix.workspaces.*`、`harnix.prefs.set`、`harnix.projects.create`、`harnix.recentProjects`、`harnix.listDirectory`、`harnix.files`、`harnix.search`、`harnix.upload.begin`、`harnix.sessions.rename / archive`、`harnix.settings.*`（`setProvider` 的返回多一个 `reloaded_instances`：这次推到了哪些继承 provider 的实例）、`harnix.plugins.*`、`harnix.devices.*` | 设备 |
| `[shell] model_switch = true` 时 | `harnix.sessions.setModel {instance?, session_id, model, provider?, space?}`：引擎的 `session.setModel`，只作用于调用方声称的 space（不写是 `sessions`）的会话，会话属别的 space 答 `SPACE_MISMATCH`；声称 `system` 也答 `SPACE_MISMATCH`（跨 space 换模型是 `harnix.system.model` 的事） | 设备 |
| 中枢，只给内核（自带的设置模块经 grant 调） | `harnix.instances.inheritProviders`：把 `system` 的 provider 并进所有 `inherit_providers` 模板的实例并让变了的重读，答 `{reloaded: [id…]}` | 内核 |
| 系统 space（`[shell] spaces` 留着 `system` 时） | `harnix.system.context / messages.drop / messages.edit / tasks.stop / tasks.output / compact / clear / rename / model / calls`（`tasks.output {task_id, tail_bytes?}` 是任务到目前的输出，默认最后 8 KiB）；`harnix.system.trace`（这一次运行里每次模型调用的输入与输出、它要的工具调用和结果）、`harnix.system.trace.call {index}`（一次调用的全文） | 设备 |
| 检视（内核，模块可 grant） | `session.tasks`、`session.taskOutput`、`session.stopTask`、`session.inspectContext`、`session.dropMessages`、`session.editMessage` | 内核 |
| 你的模块 | `app.<id>.*` | 你声明的 |

明确拒绝并说明原因的（不是「没有这个方法」）：引擎里交凭据、开端点的方法；`session.run_turn`、
`session.subscribe` 这类中枢独占的方法；`plugin.install` 等包生命周期（走 `harnix.plugins.*`）；
`mcp.addServer`。

### 5.3 主题

| 主题 | 参数 | 值 |
|---|---|---|
| `harnix.instances` | — | 实例树 |
| `harnix.instances.sessions` | `instance` | 一台实例的会话 |
| `harnix.sessions.tasks` | `instance`、`session_id` | 一个会话的任务 |
| `harnix.sessions.status` | — | 各会话的运行状态 |
| `app.<id>.<主题>` | 你声明的 | 你推的整值快照 |

会话事件不是主题：`harnix.attach` 之后经 `harnix.event` 帧来，带 `seq`，断线按 `seq` 追赶。M7 起
条目以 `harnix.event { event: { kind: "extension_entry", entry } }` 推给正在看的人。

一轮结束时中枢发 `harnix.turn_settled { instance, session_id, turn_id, result | error, outcome? }`。`outcome` 是这一轮
`turn_complete` 的要点：`stop_reason`、`usage`，有才有的 `structured_output`、`message`（例如 hook 截停的理由）；中枢没看到
`turn_complete` 时没有这个字段。回答的全文不在里面，读 `session.history`。

---

## 6. 前端

### 6.1 包

| 包 | 用不用 |
|---|---|
| `@harnix/react` | 用：`Registry`、`useNest()`、`ChatPanel` 与对话部件、i18n（`t`、`useT`、`setLocale`）、`Icon`、`markdown`、样式 `@harnix/react/styles.css`（只用令牌） |
| `@harnix/core` | 少用：`/client`（连接）、`/state`（会话状态机）、`/protocol`（帧与生成的契约类型） |
| `@harnix/shell` | 不 import；它是应用入口 |
| `@harnix/testkit` | 测试用（§7） |

React 19（现在 `19.3.1`）。界面包写成 `peerDependencies: { react: "^19" }`，依赖 `@harnix/react: workspace:*`。

### 6.2 入口

```ts
export function register(r: Registry, host: NestHost): void
```

`r.register(point, contribution, owner)`：`owner` 写模块 id。形状不对（缺必需的函数）或 `id` 重复：
这一条被拒，记进诊断页，别的照常。贡献渲染出错只影响这一条。

### 6.3 八个贡献点

| 点 | 形状（必需的加粗） |
|---|---|
| `space` | **`id`**、**`icon`**、**`title()`**、`order`、`place: "top" \| "bottom"`（默认 top；bottom 列在左栏底部、系统之上，给全局而非业务面的入口）、`details: "open" \| "closed"`（首次进入时右栏开合，默认 closed；之后每个 space 各记各的）、`center: "chat" \| "minimized" \| "hidden"`（默认 chat）、`sidebar()`（左栏中段）、`overview()`（中区不是对话时）、`tabs: [{ id, title(), render(), align? }]` 或返回它的函数（随选中项变；`align: "end"` 的页签排在条的右端，给不常用的）、`sessions`（列哪个后端 space 的会话，默认自己的 id）、`subtitle()`、`suggestions()`、`notice()`、`placeholder()` |
| `home.card` | **`id`**、**`render()`**、`title()`、`order`：画在新对话的输入框下面 |
| `tool.row` | **`id`**、**`match(toolBlock)`**、**`render(toolBlock)`** |
| `details.panel` | **`id`**、**`match(item)`**、**`render(item)`**、`title(item)`；`item = { kind, name?, … }`，由 `host.reveal(item)` 打开 |
| `flow.block` | **`id`**、**`match(block)`**、**`render(block)`**：对话流里的块，先于内建的匹配 |
| `sidebar.group` | **`id`**、**`group(sessions)`** → `[{ key, title, hint?, workspace?, rows }]`、`label()` |
| `command` | **`id`**、**`complete(query)`** → `[{ replacement, label, description?, source? }]` |
| `settings.section` | **`id`**、**`label()`**、**`render()`** |

条目（M7）在流里是 `{ kind: "custom", type: "entry", data: { id, ns, event, payload, at } }`，用
`flow.block` 认领；没人认领不画。

### 6.4 `NestHost`（`useNest()`）

| 成员 | 说明 |
|---|---|
| `call<T>(method, params?)` | 只放行 `harnix.reachable` 里的 |
| `subscribe(topic, params \| undefined, fn)` | 返回取消函数 |
| `session.current() / context() / list() / open(ref) / create(spec?)` | 都是前台 space 的；`create` 自动绑定前台 space |
| `instances.list() / sessions(instance) / tasks(instance, sessionId) / create(template, name, params?) / stop(instance)` | — |
| `composer.insert(text)` | — |
| `space(moduleId).fetch(path, init?) / socket(path)` | 模块自有链接（M7） |
| `reveal(item)` | 右栏打开详情 |
| `t(key, …)` | 文案 |

拿不到 Harnix 自己的连接。

### 6.4.1 外观：令牌覆盖与语言包

| 调用 | 说明 |
|---|---|
| `r.tokens({ "--accent": "#2f6fb3", … }, owner)` | 只许覆盖主题已有的令牌名（`:root` 上声明过的）；新名字、`url(` 之类的值被拒并记进诊断。后注册的赢 |
| `r.strings("zh-CN" \| "en", { "<owner>.key": "…" }, owner)` | 语言包；key 必须以 `<owner>.` 开头，别的被拒。之后 `host.t("<owner>.key")` |

两者不是贡献点（贡献点还是 8 个）。编进来的模块和运行期 v2 包都能用；运行期包拿到的注册表把 `owner`
钉成包名。页面标题、左栏产品名与图标来自 profile 的 `[brand]`。

### 6.4.2 外壳的框架

一个 space 就是三栏：左栏中段是它的 `sidebar()`，中区默认是对话（`center`），右栏是它的 `tabs`。不随
space 变的只有：顶部的标识、新建对话、搜索；底部的头像（就是设置入口，外圈颜色表示连接状态；点开是
设置、主题、语言、关于、连接详情）。两边的栏都能拖宽度。内建三个 space——会话（按时间）、项目（树，
一次展开一个）、系统（入口在底部）——都是用例，`[shell] spaces` 决定留哪些。

### 6.5 构建

```sh
HARNIX_UI_MODULES=<各 space 的 ui/src/index.tsx，逗号分隔绝对路径> HARNIX_UI_OUT=<目录> \
  pnpm --dir harnix --filter @harnix/shell build
HARNIX_UI_DIST=<同一目录> cargo build --release -p <app>
```

外壳的 Vite 配置把 `react`、`react-dom`、`@harnix/react`、`@harnix/core` 去重到外壳那一份，所以应用的
界面入口在 Harnix 的 pnpm workspace 之外也能解析到它们；**别的第三方依赖**要在应用自己的
`node_modules` 里。完整的例子是 `examples/hello-app/scripts/build.sh`。

规矩：颜色只用令牌、文案只用 key（静态检查会拦）；CSP `script-src 'self'; style-src 'self'`：没有
`eval`、远程 import、CDN、内联脚本与样式；交互元素带 `data-testid`、状态用 `data-state`。

### 6.6 运行期包的界面模块

```toml
[[ui]]
point = "tool.row"
module = "rows.js"     # 包的 ui/ 下
api_version = 2
```

- 模块是**预编译好的 ES 模块**，不带 React：`import { jsx } from "react/jsx-runtime"`、
  `import { useNest } from "@harnix/react"`。页面的内联 import map 把 `react`、`react-dom`、
  `react-dom/client`、`react/jsx-runtime`、`@harnix/react` 指到外壳自己用的那几份，页面上只有一个 React。
- 导出 `register(registry, host)`；`registry.register(point, c)`、`registry.tokens(t)`、
  `registry.strings(locale, d)`，`owner` 一律是包名。
- import map 是页面里唯一的内联脚本，服务端出页面时按它的 `sha256` 写进 CSP；没有 `unsafe-inline`、
  `unsafe-eval`。
- `api_version` 不是 2（或没写，按 v1 算）：这一条报「这一条不生效」并写明原因，包的其余部分照常。
- `[ui] runtime_modules = false`：一条都不提供、不服务。
- 运行期包仍然**不能**新增 space。范例：`tests/fixtures/packages/demo-rows-v2/`。

---

## 7. 测试：`@harnix/testkit`

| 导出 | 说明 |
|---|---|
| `startHarnix(opts) → Backend` | 起一个进程，自己的 HOME、数据根、端口。`opts.bin` 写应用二进制的绝对路径；`profile`（追加的 TOML）、`env`、`args`、`replay`、`seed`、`uiDir`、`scratch` |
| `Backend` | `url`、`port`、`token`、`scratch`、`state`（`$HOME/.<product_id>`）、`project`、`pid`、`log()`、`stop()`（发 SIGTERM、立刻删目录，不等退出）、`dispose()`（等进程退出——15 s 后 SIGKILL——再删目录；收尾用它，夹具与 `defineSuite` 已用它）、`halt(signal?)`、`restart()` |
| `connect(backend) → TestClient` | `call`、`refused`（期望被拒，返回错误）、`waitFor(pred, { timeout, describe })`、`events`、`eventsOfKind`、`text`、`close` |
| `test`、`expect` | Playwright Test，带 `api` 夹具 |
| `inconclusive(reason)` | 环境不满足时标成「没结论」而不是通过 |

用例打标签 `@feature:<功能>`、`@layer:api|ui`，覆盖表从标签生成。`replay: true` 读的是 Harnix 自己的
`tests/live/fixtures/recordings`；应用要用自己的录像，`args: ["--replay-dir", <目录>]`。

---

## 8. 应用仓库用的脚本

| 脚本 | 什么时候跑 |
|---|---|
| `harnix/scripts/sync-app-deps.mjs <app> [--check]` | 升 Harnix 之后：重写应用根 `Cargo.toml` 里带标记的依赖块；`--check` 只比对 |
| `harnix/scripts/check-pins.mjs <app>` | 构建前：`agix/` 与 Harnix 钉的是否一致、是否在上游 |
| `harnix/scripts/check-lock.mjs <app>` | 构建后：应用的锁文件里 Harnix 的依赖是否和 Harnix 的一致 |
| `harnix/scripts/set-version.sh` | 不是给应用的：Harnix 自己发布用 |

仓库地址：`https://github.com/openatta/Harnix.git`（GitHub 上已从 `openatta/harnix` 改名，旧地址会转），
`https://github.com/openatta/agix.git`。

---

## 9. 迁移时要知道的缺口

| 缺口 | 影响 | 什么时候补 |
|---|---|---|
| 没有 MCP 服务端、角色令牌 | 外部 Agent 要连：模块 `start()` 里自己起监听 | 0.4 之后 |
| 批准原语 | 不进 Harnix：照指南 §10.1 用条目 + 主题 + 只许设备的方法自己做 | 不做 |
| 运行期包新增 space | 第三方包只能贡献 8 个点里除 `space` 之外的 | 0.4 之后（D28） |
| 子 Agent 的进度 | 任务页能读 Shell / 监视器边跑边出的输出、子 Agent 的回答，但看不到子 Agent 正在第几步；要看过程走它的侧链会话 | Agix，有需要再提 |
| 应用自己的 pnpm workspace 怎么摆 | 外部应用靠 Vite 去重解析 `react`、`@harnix/react`；类型检查与别的依赖要应用自己装（见迁移说明） | 消费者落地时定 |
| 桌面壳在 Windows 上没有干净的停止 | 关窗口 5 秒后直接结束子进程，模块的 `stop` 不跑 | 有需要再做 |

## 10. 桌面壳：`templates/desktop`

Tauri 2，两种模式：

- **本机**：壳起应用（`--host 127.0.0.1 --port <空闲> --launch-link --exit-with-stdin`），读
  `~/.<product_id>/token`，`POST /auth/link` 换一条一次性链接给窗口；关窗口时 SIGINT 停应用，壳被强杀时
  应用读到 stdin 断开自己退出。
- **远程**：窗口开一个配置好的 `https://` Harnix，配对走网页自己的流程。

业务流量全是页面自己的 HTTP/WS：没有 command、没有 invoke handler、没有全局 Tauri 对象，
`check-no-ipc.mjs` 在 offline 层查。`--smoke` 不开窗口走一遍起、准入、停。详见它的 README。
