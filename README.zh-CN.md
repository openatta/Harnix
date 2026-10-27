# Harnix

[English](README.md) | 简体中文

**[Agix](https://github.com/openatta/agix) 应用的公共底座。** 做在 Harnix 上的应用是一个 Rust 二进制：Agent
引擎、Web 服务、应用自己的模块和界面编在一起。产品需要、又不该各自去做的东西由 Harnix 提供——会话由服务端
持有（不靠哪个浏览器标签页）、实例、授权、传输、外壳、space、系统视图——而 Harnix 本身做薄，产品是应用的。

![Harnix](docs/images/harnix.png)

## 状态

0.4。两个产品正在迁上来（Helm、IPDrun，迁移说明见下）。0.4.z 内模块 API 与协议只做加法；依赖它就钉一个
标签（`Harnix-v<x.y.z>-<日期><序号>`）。

## 跑起来

```sh
git clone --recurse-submodules https://github.com/openatta/Harnix.git
cd Harnix
pnpm install && pnpm --filter @harnix/shell build   # 界面；二进制把它编进去
cargo build --release
./target/release/harnix                              # → http://127.0.0.1:4080/
```

Rust 1.87 或更新，界面要 Node 和 pnpm。`--recurse-submodules` 不能少：Agix 在 `agix/`。跑起来的是空应用：
Harnix 自带的三个 space，别的什么也没有。

不需要任何环境变量。没配模型也能启动，页面会提示：点 **配置模型**（或头像 → **设置 → 模型与凭据**）添加
模型服务，可以配多个，每个是 Anthropic 或 OpenAI 兼容的 API，其中一个是默认。写进全局的
`~/.agix/settings.json`，不用重启就生效，不进浏览器，也不进日志。也可以用 `ANTHROPIC_AUTH_TOKEN`（加上可选的
`ANTHROPIC_BASE_URL`）。

## 应用能得到什么

- **一轮属于服务端，不属于某个标签页。** 中枢在任何客户端之前订阅每个会话，所以刷新、换一台设备、断线
  重连，都按 `seq` 从缓冲的帧里追上。客户端和应用自己的模块用同一套语义驱动会话：排队、中断、引导、
  无头会话和 tap。
- **编译期模块。** `harnix::Module` 声明它的方法（`app.<id>.*`）、主题、路由、原生工具与场景、实例模板和
  space；它拿到的是上下文，不是中枢的内部。迁移或启动失败的模块单独停用，原因写在诊断页。
- **实例。** 一条链接后面好几台 Agix 引擎——进程内的，或者起起来并看护的 daemon、连上去的 daemon——按模板建，
  互相隔离，账本记着，重启后重建。
- **space。** 一个 space 是三栏合起来：它自己的左栏、默认是对话的中区（它说不要就不要）、它自己的右栏页签。
  Harnix 自带的三个——会话（按时间）、项目（树）、系统——都是用例，产品自己决定留哪些。
- **调试用的系统 space。** 一个控制台，看此刻在跑的东西——实例、它们正在跑的会话、正在跑的任务；选中哪一项，
  右栏看它每一次模型调用到底送进去了什么、出来了什么，下一次调用要带的上下文，任务，以及实时的帧。
- **对话里的卡片。** 模块往会话的日志里写扩展条目：刷新、重放、分叉都在，模型看不到。批准卡就是这样做的。
- **不 fork 就有自己的样子。** `[brand]` 给产品名和图标；模块覆盖主题令牌、带自己的语言包。程序自己的状态
  在 `~/.<product_id>`。
- **装上的包也能画界面。** Agix 包里的界面模块是预编译的 ES 模块，从页面的 import map 拿 React——在其余都
  严格的 CSP 里按 hash 放行——每个产品可以整个关掉。
- **桌面壳。** `templates/desktop` 用 Tauri 2 包住一个应用，本机或连远端 Harnix，不加业务 IPC：业务流量全是
  页面自己的 HTTP/WS。macOS 上窗口没有标题栏，页面的顶栏就是拖窗口的地方——这是页面唯一能对壳请求的事。
- **对着真二进制的测试。** `@harnix/testkit` 起应用、用录像回放模型调用，驱动 API 和界面（Playwright）。

## 做一个应用

复制 `templates/app`，把 `agix/` 和 `harnix/` 加成平级的子仓，照 [二开指南](docs/harnix-sdk.md) 做；每个
接口都在 [SDK 参考](docs/harnix-sdk-reference.md) 里。`examples/hello-app` 是一个完整的应用：两个 space、
一台自己的实例、一个有结果卡片的长任务、自己的 HTTP 路由和 WebSocket、一张批准卡和一套品牌。

```rust
fn main() -> anyhow::Result<()> {
    harnix::App::builder()
        .name("hello-app", env!("CARGO_PKG_VERSION"))
        .with_builtins()
        .module(Greeter::default())
        .run()
}
```

## profile 与开关

profile 说这个进程是什么；命令行开关覆盖它。

```toml
[brand]
name = "Hello"
icon = "brand.svg"
product_id = "hello-app"         # 状态在 ~/.hello-app

[shell]
spaces = ["sessions", "projects"]   # 留哪些内建 space（不写就是三个都留）

[ui]
runtime_modules = true           # 装上的包的界面模块

[modules.greeter]                # 每个模块自己的段
```

| 开关 | 含义 |
|---|---|
| `--profile` | 上面那份 profile；每一段都拒绝不认识的键 |
| `--port` / `--host` | 默认 `4080` / `127.0.0.1`；绑非回环要 TLS 和配对过的设备 |
| `--agix-dir` | 引擎目录，默认 `$AGIX_CONFIG_HOME`，再默认 `~/.agix`（和 `agixd` 共用） |
| `--data-dir` | 项目在哪，默认 `$HARNIX_DATA_DIR`，再默认 `~/Documents` |
| `--ui-dir` / `--headless` | 从目录出界面 / 不出界面 |
| `--launch-link` | 令牌不进页面：浏览器凭一次性链接进来 |

## 目录

| 路径 | 是什么 |
|---|---|
| `~/.agix/` | 引擎：设置分层、transcript、记忆、技能、录像 |
| `~/.<product_id>/` | 程序自己的配置与状态：令牌、设备、工作区、账本、实例的数据、各模块的数据（默认 `~/.harnix`；`HARNIX_STATE_DIR` 能挪） |
| `~/Documents/` | 只放项目——里面没有隐藏目录 |
| `<项目>/.agix/` | 项目自己的数据与设置 |

## 结构

```
crates/harnix     应用依赖的那个 crate，以及 `harnix` 二进制（空应用）
crates/app        App::builder()：profile、启动顺序、模块生命周期、诊断
crates/module     编译期模块契约：Module、它的注册表、ModuleContext
crates/hub        会话中枢：实例、订阅、重放、seq、轮次归属、队列、主题
crates/instance   Agix 实例：进程内或 daemon（起的或连的），在一条链接后面
crates/transport  通道语义落在选定的拓扑上：帧、握手、静态页面、大负载、/apps/<id>/
crates/authz      唯一的准入点：主体 × 方法/路由/主题，默认拒绝，记审计
crates/builtin    Harnix 自己的功能，照公开契约写成的模块
crates/contract   这些层之间传递的类型
crates/testapp    给测试用的应用：好的模块，和故意坏的模块

ui/packages/      @harnix/core（协议、客户端、会话状态）、@harnix/react（对话、贡献注册表、i18n、令牌）、
                  @harnix/shell（界面）、@harnix/testkit
templates/app     一个新应用
templates/desktop 桌面壳（Tauri 2）
examples/hello-app  照应用的做法做出来的应用
tests/live/       LIVE 测试：Playwright Test 对着真二进制
agix/             Agix（子仓）
```

## 文档

| 文档 | 用途 |
|---|---|
| [docs/harnix-sdk.md](docs/harnix-sdk.md) | 做应用：指南 |
| [docs/harnix-sdk-reference.md](docs/harnix-sdk-reference.md) | 每个类型、方法、profile 键、开关、目录、贡献点 |
| [docs/migration-helm.md](docs/migration-helm.md)、[docs/migration-ipdrun.md](docs/migration-ipdrun.md) | Helm、IPDrun 迁到 0.4 |
| [docs/concept_and_architecture.md](docs/concept_and_architecture.md) | Harnix 是什么，为什么是这个形状 |
| [docs/contribution_points.md](docs/contribution_points.md) | 界面的贡献点，从代码生成 |
| [docs/testing.md](docs/testing.md) | 测试分层、LIVE 测试、录像、覆盖表 |
| [docs/glossary.md](docs/glossary.md) | 用词，一个词一个意思 |

*文档用中文写。*

## 测试

所有层，便宜的在前；跑不了的说出来，不当作通过：

```sh
scripts/check.sh                # 这台机器能跑的全部
scripts/check.sh --fast         # 不要模型、不开浏览器
scripts/check.sh --only api     # 一层
scripts/check.sh --with-remote  # 加上 TLS 配对——会在这个网络上开监听
scripts/prune.sh                # 构建缓存：删增量缓存，超过 25 GB 的 target 整个清掉
```

LIVE 测试（`tests/live`，见 [docs/testing.md](docs/testing.md)）跑真二进制：`api` 用它自己的协议驱动后端，
`ui` 在 chromium 里驱动界面，模型调用除非 `--live` 都从录像回放。

**不要跑 `cargo test --workspace`**：它会把收进来的 Agix crate 也跑一遍，其中两个假定自己是 workspace 根。
Agix 的测试在 Agix 里跑。

## 许可

[Apache-2.0](LICENSE)。
