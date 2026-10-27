# harnix-desktop：Harnix 应用的桌面壳

> 读者：要把自己的 Harnix 应用做成桌面程序的人。为什么这样设计见
> [0.4 设计 §12](../../docs/harnix-0.4-design-and-plan.md)（D5：Tauri 只是外壳）。

一个 Tauri 2 窗口，里面就是应用自己发出的网页界面。**壳只管三件事：窗口、子进程、配置。** 应用做的
一切（调用、事件、上传）都是页面自己对服务端发的 HTTP/WS，和在浏览器标签页里一模一样。所以桌面版
和浏览器版是同一个产品：同一份界面、同一条准入路径、同一套测试。

两种模式：

| 模式 | 壳做什么 | 页面从哪来 |
|---|---|---|
| **本地** `local` | 以子进程启动应用二进制（配置的路径，或打包进来的 sidecar），等它就绪，拿一次性启动链接打开 | `http://127.0.0.1:<端口>/` |
| **远程** `remote` | 打开配置好的地址 | 另一台机器上的 Harnix，`https://主机:端口/` |

连不上服务端时窗口里只显示壳自带的「无法连接」页（`dist/`），壳里不放业务界面。

## 目录

```text
templates/desktop/
├── check-no-ipc.mjs          不许有业务 IPC 的静态检查（node，无依赖）
├── dist/                     壳自带的唯一页面：「正在启动」「无法连接」
│   ├── index.html            无内联脚本、无内联样式，CSP 只放 'self'
│   ├── status.js             从地址栏参数读出错误原因，填进页面
│   └── style.css
└── src-tauri/                自成一个 Cargo workspace，不进应用的 workspace
    ├── Cargo.toml
    ├── tauri.conf.json       产品名、标识、CSP、图标、打包
    ├── build.rs
    ├── capabilities/         只有一份，什么权限都不给
    ├── desktop.example.toml  配置样例
    ├── icons/                占位图标，换成你的
    └── src/
        ├── main.rs           窗口与生命周期、--smoke
        ├── config.rs         desktop.toml 与环境变量
        └── local.rs          本地模式：子进程、就绪、启动链接
```

## 先跑起来

要 Rust ≥ 1.87。Linux 另需 webkit2gtk-4.1 等系统包（见 Tauri 文档的 Prerequisites）。

```sh
cd templates/desktop/src-tauri
cargo build

# 本地模式，指向一个已经编好的应用二进制
cat > target/debug/desktop.toml <<'EOF'
mode = "local"
[local]
program = "../../../../../target/release/harnix"
EOF
./target/debug/harnix-desktop
```

`--smoke` 不开窗口，把窗口会做的事走一遍再退出：启动子进程、等就绪、拿启动链接、照窗口的方式跟进去
（链接种下 cookie → 带 cookie 拿到页面 → 不带 cookie 被拒 → 同一条链接第二次被拒），然后让子进程
干净退出。全过时退出码为 0。看不到窗口的地方（CI、远程机器）就靠它：

```sh
HARNIX_DESKTOP_PROGRAM=/path/to/myapp ./target/debug/harnix-desktop --smoke
```

## 本地模式

壳以 `--host 127.0.0.1 --port <端口> --launch-link` 加上配置里的 `--profile` 与 `args` 启动应用。
端口不配时每次挑一个空闲的，两个副本不会撞。子进程的工作目录是用户主目录（从桌面启动时壳自己在 `/`，
而引擎会在工作目录下记 `.agix/`），所以 `args` 里的相对路径相对于主目录；`program`、`profile` 在交给
子进程前已经换成绝对路径。

**窗口怎么被准入。** 子进程开着 `--launch-link`，页面里不带令牌。壳从应用的状态目录读 `token` 文件
（只有本人可读），用它 `POST /auth/link` 换一条一次性链接，再让窗口打开这条链接；链接种下
`HttpOnly`、`SameSite=Strict` 的会话 cookie 并跳到界面，之后 WebSocket 凭这个 cookie 准入，Origin
校验照旧（同源）。选这条路而不是读应用打印的 `open → …` 那一行，是因为：

- `POST /auth/link` 是应用给程序用的接口（JSON，随服务端一起定版），控制台那一行是写给人看的；
- 它顺带就是就绪检查：服务起来了、磁盘上的令牌是这一次运行的，它才答 200；
- 令牌只经过一个文件和一次回环请求，**不经 Tauri IPC**，窗口只见到一条用过即废的链接。

状态目录按应用自己的规则找：`HARNIX_STATE_DIR`，否则 `~/.<product_id>`，`product_id` 取 profile 的
`[brand] product_id`，没有就是 `harnix`。子进程继承壳的环境变量，两边读的是同一个。

**生命周期。** 关窗口、Cmd-Q、壳收到 SIGINT/SIGTERM/SIGHUP，都走到同一个退出点：先给子进程发
SIGINT（Harnix 收到它会逆序停模块、关中枢、清上传目录），5 秒内没退出就强制结束。子进程自己退出
时，窗口换成「无法连接」页并写明退出状态。

## 远程模式

```toml
mode = "remote"
[remote]
url = "https://build-box.local:4317/"
```

- 只认 `https://`（回环地址除外）：Harnix 不加 TLS 就不肯监听回环以外的地址。
- 服务端的 profile 要把自己对外的 Origin 写进 `[transport] extra_origins`（例如
  `"https://build-box.local:4317"`），否则页面的连接会因 Origin 不在白名单被拒。
- 第一次打开时在网页界面里完成设备配对，壳不参与。
- 证书要被系统信任（自签证书就加进系统钥匙串 / 证书存储）；webview 不给「仍然继续」的按钮。
- 壳打开前只试一次 TCP 连接，连不上就显示「无法连接」页；TLS 与配对的错误由页面自己说。

## 配置

`desktop.toml`，按顺序找，找到第一个为止：

1. `HARNIX_DESKTOP_CONFIG` 指定的文件（必须存在）；
2. 用户配置目录下、以应用标识命名的目录里的 `desktop.toml`（macOS 是
   `~/Library/Application Support/<identifier>/`，Linux 是 `~/.config/<identifier>/`，Windows 是
   `%APPDATA%\<identifier>\`）——用户自己写的；
3. 可执行文件旁边的 `desktop.toml`——应用随包带的；
4. 都没有：本地模式，用打包进来的 sidecar。

| 键 | 环境变量 | 说明 |
|---|---|---|
| `mode` | `HARNIX_DESKTOP_MODE` | `local`（默认）或 `remote` |
| `[local] program` | `HARNIX_DESKTOP_PROGRAM` | 应用二进制；不写用可执行文件旁边的 sidecar |
| `[local] profile` | `HARNIX_DESKTOP_PROFILE` | 作为 `--profile` 传给应用 |
| `[local] args` | — | 原样追加的参数；不要再写 `--host`、`--port`、`--launch-link` |
| `[local] port` | `HARNIX_DESKTOP_PORT` | 固定端口；不写每次挑空闲的 |
| `[remote] url` | `HARNIX_DESKTOP_URL` | 远程 Harnix 的地址 |

文件里的相对路径相对于文件所在目录；环境变量里的相对路径相对于当前目录。拼错的键直接报错，不会悄悄
不生效。完整样例见 `src-tauri/desktop.example.toml`。

## 改成你的应用

**改名。** 这几处一起改：

- `src-tauri/Cargo.toml` 的 `name`（可执行文件名）；
- `src-tauri/tauri.conf.json` 的 `productName`（窗口标题、`.app` 名）、`identifier`（反向域名，
  也决定用户配置目录）、`version`；
- `src-tauri/src/config.rs` 的 `ENV_PREFIX`（环境变量前缀）和 `SIDECAR`（sidecar 的名字）；
- `dist/index.html` 的 `<title>`，以及 `dist/` 里的文案与样式。

**图标。** 准备一张 1024×1024 的 PNG，在 `src-tauri/` 下运行 `cargo tauri icon your.png`
（`cargo install tauri-cli --version '^2'`）。它会生成一整套，`tauri.conf.json` 的 `bundle.icon` 只用
其中五个，其余可以删。

**打包本地二进制（sidecar）。** 应用二进制按目标三元组命名放进 `src-tauri/binaries/`，并在
`tauri.conf.json` 的 `bundle` 里声明：

```sh
cargo build --release -p myapp                       # 在应用的 workspace 里
triple=$(rustc -vV | sed -n 's/host: //p')
mkdir -p src-tauri/binaries
cp target/release/myapp "src-tauri/binaries/myapp-$triple"
```

```json
"bundle": { "externalBin": ["binaries/myapp"] }
```

`SIDECAR` 改成 `"myapp"`。打包时 Tauri 去掉三元组后缀，把它放在壳的可执行文件旁边，壳不配
`program` 时就找它。`cargo build` 也会把它复制进 `target/<profile>/`，开发时一样能用。声明了
`externalBin` 而文件不在，构建会失败——所以模板默认不声明，`binaries/` 也不进版本库。

```sh
cd src-tauri && cargo tauri build      # .app / .dmg / .msi / .deb / AppImage，按平台
```

**签名。** macOS：`bundle.macOS.signingIdentity` 填开发者证书，公证用 `APPLE_ID`、`APPLE_PASSWORD`、
`APPLE_TEAM_ID` 环境变量；sidecar 和壳一起签。Windows：`bundle.windows.certificateThumbprint` 或
`signCommand`。细节见 Tauri 文档的 Distribute 一节。

**可以加的。** 设计允许的 Tauri 能力只有托盘、窗口、通知、更新、单实例、日志，都只在 Rust 这边用。加了
插件也不要往 `capabilities/` 里给页面开权限。

## 无标题栏（macOS）

窗口用 `TitleBarStyle::Overlay` + `hidden_title`：没有独立的标题栏，红绿灯浮在页面左上角，中心在 52px 顶栏带
的正中（26px），和页面里收起左栏的按钮、顶栏文字在一条水平线上。红绿灯的大小随 macOS 版本变（macOS 26 是
14×14），而 Tauri 只在建窗口时放一次，所以壳先用一个不显示的探测窗口量出按钮的位置再算（`main.rs` 的
`traffic_lights`）；改页面顶栏高度时一起改 `TOP_BAND`。

- **页面怎么知道。** Tauri 往它打开的每个页面里放 `__TAURI_INTERNALS__`，Harnix 的界面看到它、又是 Mac，就在
  `<html>` 上标 `data-chrome="overlay"`：左栏顶行给红绿灯让出位置，收起左栏的按钮放在左栏外、固定在红绿灯右边
  （展开、收成窄栏都在同一处，收起的淡出动画也不带它），左栏收起时主区顶栏让开它；横幅浮到顶栏带下面。壳不往
  页面里送任何东西。
- **怎么拖。** 页面把各栏顶栏标成 `data-tauri-drag-region="deep"`：按在空白处拖窗口、双击最大化；按钮、页签
  这些可点的照常点。Tauri 注入的脚本据此请求窗口开始拖动——这是页面对壳唯一能发的请求，`window-drag.json`
  只给它这两条。网页没有办法自己挪动原生窗口，所以这条口子省不掉；它只动窗口，不碰应用的数据。
- **远程模式**下页面在 `https://` 的另一台机器上，不在 `remote.urls` 里，拖不动：要拖就把那个地址加进
  `window-drag.json`（`check-no-ipc.mjs` 只认回环，你的应用里那份要相应放宽并说明）。
- Windows、Linux 照旧用系统标题栏。

## 业务流量的规则

**页面里的一切都走它自己的 HTTP/WS。** 具体是：

- 没有 `#[tauri::command]`，不调 `invoke_handler`，`withGlobalTauri` 保持 `false`；
- 不注册自定义协议来发界面，不用本地 socket；
- `capabilities/default.json` 什么权限都不给。
- **唯一的例外是窗口本身的**：`capabilities/window-drag.json` 让主窗口里的页面（壳自带的页，和回环上的
  `http://127.0.0.1:*`、`http://localhost:*`）能请求「开始拖动窗口」和「双击最大化」，别的都不行。见下面
  「无标题栏」。

`check-no-ipc.mjs` 查这些，出现就失败（字面匹配，注释也算；`*.md` 不查）；capability 按 JSON 读，除了那两条
窗口权限，任何权限都算失败，`remote.urls` 只能是回环地址，窗口只能是 `main`：

```sh
node templates/desktop/check-no-ipc.mjs          # 本模板
node harnix/templates/desktop/check-no-ipc.mjs .  # 你的应用里那一份
```

**自己验证。** 打开 Web Inspector 看 Network：只有对服务端的 `http(s)://…` 请求和 `ws(s)://…` 连接。

- debug 构建：在窗口里右键 →「检查元素」（macOS 是 Safari 的 Web Inspector，Windows 是 Edge
  DevTools，Linux 是 WebKitGTK Inspector）；
- release 构建：临时给 `tauri` 加上 `devtools` feature 再编。

也可以挂代理抓包：远程模式下把系统代理指向 mitmproxy / Charles 并信任它的根证书；本地模式的流量在
回环上（系统代理默认绕过回环），用 Web Inspector，或者 `tcpdump -i lo0 port <端口>`（Linux 是 `lo`）。
抓到的应当全是 HTTP 与 WebSocket 帧，没有别的通道。

## 已知的限制

- 壳被强杀（SIGKILL、崩溃）时来不及发 SIGINT；应用靠 `--exit-with-stdin` 跟着退出：它的 stdin 是壳
  持有的管道，壳一没，管道就断。这条路走的是应用自己的退出流程。
- Windows 上没有给无控制台子进程发 Ctrl-C 的干净办法，关窗口时等 5 秒后直接结束子进程，模块的
  `stop` 不会执行。
- 状态目录的查找规则抄的是应用这边的（`crates/app/src/paths.rs`）；那边改了，`local.rs` 的
  `state_dir` 要跟着改。
