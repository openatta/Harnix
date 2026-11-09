# 测试：LIVE 框架

> 读者：改 Harnix 的人；在 Harnix 上做应用、要给自己的应用写 LIVE 测试的人；写插件 APP 的人。
>
> 为什么这样测见 [概念与架构](concept_and_architecture.md) §8。这里写怎么用：检查分几层、LIVE 怎么跑、怎么写、怎么录像（含手做）、
> 覆盖表怎么读、插件 APP 怎么测、应用怎么用 `@harnix/testkit` 写自己的 LIVE。

---

## 1. 一句话

**功能测试都对着真的 `harnix` 二进制跑。** 不用假 DOM，不 mock 中枢，不用假后端。后端经它自己的
协议测（`api` project，不开浏览器），界面在 chromium 里测（`ui` project）。只有一个运行器：
Playwright Test。两种拓扑（`single_duplex`、`split_streams`）各跑一遍。

`@harnix/core` 等前端包的 Vitest 是单元测试，**不算功能覆盖**。

## 2. 检查的分层：`scripts/check.sh`

```sh
scripts/check.sh                 # 这台机器能跑的每一层，便宜的先跑
scripts/check.sh --fast          # 跳过模型与浏览器
scripts/check.sh --only api      # 只跑一层（层名见下表）
scripts/check.sh --with-remote   # 另跑配对与 TLS 那条路（会在本机网络上开一个监听）
scripts/prune.sh                 # 清构建缓存：增量缓存全删，超过 25G 的 target 整个清掉
```

| 层 | 跑什么 | 要什么 |
|---|---|---|
| `build` | `pnpm install --frozen-lockfile`、外壳构建；`harnix-cli` 与 `harnix-testapp` 的发布构建；带 `apps` 的 `harnix`、打包工具 `harnix-app`、带 `apps` 的 `harnix-testapp`，都编在 `target/apps`；`examples/hello-app` 照应用的方式构建 | — |
| `rust` | Harnix 各 crate 的 `cargo test`；`harnix-wasm`、`harnix-app-host`、`harnix-apps` 各带 `compile` 再跑一遍；`harnix-app` 带 `apps` 再跑一遍 | — |
| `clippy` | 同样的 crate 与 feature，`-D warnings`、`--no-deps` | — |
| `offline` | `tests/crate-metadata.mjs`（版本、许可、Agix 钉，以及 SDK 文档写的版本）、`app-scripts`、`style-lint`、`readme-pairing`；桌面壳的 `check-no-ipc`、`check-rename` | — |
| `frontend` | 各包的 typecheck 与 Vitest；协议类型对着 Rust 重新生成再比；覆盖表 `--check`；插件 APP 带的 SDK 与 ui-kit 是否最新（`tests/app-ui-vendored.mjs`） | — |
| `api` | LIVE 的 `api` project，回放，两种拓扑；`HARNIX_COVERAGE_GATE=1`（§7） | — |
| `template` | 把 `templates/app` 拷出去，当一个新应用离线构建（`target/template`） | — |
| `live` | `api` project 连真模型，两种拓扑 | `.env` 里有模型凭据 |
| `browser` | LIVE 的 `ui` project，两种拓扑（后端只给这一种，页面得自己走过去） | chromium（`pnpm install:browser`） |
| `interface` | 起一个同时给两种拓扑的后端，`tests/topology-parity.mjs` 逐条比两边的答 | 模型 |
| `remote` | 绑非回环、带 TLS，`tests/remote-smoke.mjs` 走配对 | `--with-remote` 与模型 |
| `budget` | `tests/budget.mjs`：二进制体积（默认构建 ≤ 48 MB，`target/apps` 的带 `apps` 构建另有 ≤ 52 MB）、界面产物、冷启动、内存、延迟；最后一个、单独跑 | 发布构建 |

**跳过的层要说，不能当作通过。** 缺模型、缺 chromium、没给 `--with-remote` 的层报 `skipped` 和原因，汇总里看得到；一层没跑不是一层过了。
层的输出写进临时文件，只有失败时才显示最后 30 行。

一次从零开始的全量检查要编几十 G。`check.sh` 开头看一眼剩余空间，不到 15G（`HARNIX_CHECK_NEED_GB`）就
拒跑并说明，免得跑到一半在某一层报出一个看不出原因的错。cargo 只增不删，每个里程碑、每次发布之后跑一次
`scripts/prune.sh`。**不要跑 `cargo test --workspace`**：会连带跑吸收进来的 Agix crate，Agix 的测试在 Agix 里跑。

## 3. 跑 LIVE

```sh
pnpm install && pnpm install:browser   # 第一次
pnpm live                              # api 和 ui 两个 project，模型调用回放
pnpm live --project api                # 只跑 API
pnpm live --project api --topology split_streams   # API 套件走分离流
pnpm live --project ui --topology split_streams    # 界面：后端只给分离流，页面得自己回退过去
pnpm -r run test                       # 前端各包的 Vitest（会话状态、组件、注册表、i18n、外壳）
pnpm live --live                       # 连 .env 里的真模型
pnpm live --no-model                   # 不碰模型：要模型的套件跳过（只给没有录像的机器用）
pnpm live --strict                     # 严格回放：请求与录像不一致的那次调用失败
pnpm live api/hub.spec.ts              # 一个文件；其余参数原样交给 playwright test
scripts/check.sh                       # 所有层，从便宜到贵
scripts/prune.sh                       # 清构建缓存：增量缓存全删，超过 25G 的 target 整个清掉
```

`ui` project 从磁盘读构建好的界面（`--ui-dir ui/dist`），所以改了前端只要
`pnpm --filter @harnix/shell build`，不用重编后端。

`pnpm live` 每次都问一遍 cargo 要不要重建（cargo 是唯一知道要不要重建的东西；对着昨天的二进制跑出的
绿比不跑更糟）。刚构建过可以设 `HARNIX_BIN=target/release/harnix` 跳过。

**拓扑**：`--topology` 决定 API 套件说哪一种（缺省 `single_duplex`）；`ui` project 的后端只给这一种，页面得自己握手回退过去。
`scripts/check.sh` 两种各跑一遍，所以新写的功能两种拓扑都要过。

### 三种模型模式

| 模式 | 用途 | 怎么选 |
|---|---|---|
| **回放**（默认） | 确定、离线、不花钱。引擎按位置重放 `tests/live/fixtures/recordings/` 里的调用 | 不加参数 |
| live | 刷新录像、冒烟 | `--live`（`HARNIX_MODEL=live`），读 `.env` |
| 无模型 | 没有录像的机器 | `--no-model`。它看着像回放模式，覆盖却少得多，别误用 |

回放缺省是非严格的：请求与录下的不一样只报告、不失败。`pnpm live --strict`（`HARNIX_REPLAY_STRICT=1`，即
`harnix serve --replay-dir <目录> --replay-strict`、testkit 的 `replayStrict: true`）让不一样的那次调用失败（ipdrun-45）：
改了提示词、工具表或场景之后，用它找出该重录的录像。引擎给回放的会话按录像开录那天定日期（Agix ipdrun-06），
所以严格回放不会隔天就红；路径要应用自己保持稳定（固定的根目录，或文件工具的相对路径）。

`ui` project 永远回放：一个布局测试因为模型慢了而失败，大家就学会了无视它。

标着 `replayed: true` 的 API 套件在 live 模式下也自己起一个回放的后端：它的价值就是每次在同一处
调同一个工具，对着真模型就成了另一个同名的测试。标着 `needsLiveModel` 的只在 `--live` 下跑
（`turns` 要真实耗时的轮，`recordings` 要的是录像**被写下**，回放的会话不写）。

## 4. 结构

```text
ui/packages/core/            @harnix/core：协议类型（生成）、协议客户端、会话状态；Vitest
ui/packages/react/           @harnix/react：对话组件、贡献注册表、i18n、令牌；Vitest（jsdom）
ui/packages/shell/           @harnix/shell：界面本身；Vitest，构建到 ui/dist
ui/packages/app-sdk/         @harnix/app-sdk：应用界面在隔离 iframe 里的一侧（桥 harnix-ui-bridge@1、mock 宿主）；Vitest
ui/packages/ui-kit/          @harnix/ui-kit：iframe 里的外壳令牌与基础组件；Vitest
ui/packages/testkit/         @harnix/testkit
  src/launcher.ts            startHarnix：独立的 HOME、数据根、引擎根、项目、端口、拓扑、回放目录
  src/api.ts                 connect：基于 @harnix/core/client 的测试客户端；finish、inconclusive；覆盖计数
  src/fixtures.ts            Playwright 夹具：harnix（每个 worker 一个）、api（每个用例一个）、mode
  src/suite.ts               defineSuite：0.3 套件形状的适配
tests/live/
  playwright.config.ts       两个 project：api、ui
  api/<名字>.spec.ts          API 套件
  ui/<名字>.spec.ts           界面套件（ui/page.ts：开页面、按录像建会话、打字）
  fixtures/recordings/       模型录像
  features.mjs               功能模块的行，以及每一层为什么可以是零
  coverage.mjs → coverage.md 覆盖表（生成）
  record-fixture.mjs         录一份新的录像
```

## 5. 写一个测试

### 新写的：直接用夹具

```ts
import { expect, finish, test } from "@harnix/testkit";

const TAGS = { tag: ["@feature:sessions", "@layer:api"] };

test("a new session is idle and in the list", TAGS, async ({ api }) => {
  const { session_id } = await api.call("session.create", { scene: "harnix", project_root: api.project });
  try {
    expect((await api.call("session.get", { session_id })).turn_state).toBe("idle");
    const { sessions } = await api.call("harnix.sessions");
    expect(sessions.some((s) => s.session_id === session_id)).toBe(true);
  } finally {
    await finish(api, session_id);
  }
});
```

界面测试拿 `page` 和 `harnix`：`await page.goto(harnix.url)`。

要一个配置不同的后端（种子数据、只开一种拓扑、带某个 profile 段），在用例里
`startHarnix({ seed, topologies, profile })`，用完 `stop()`。

### 0.3 迁过来的：`defineSuite`

0.3 的 17 个 API 套件原样搬了过来（M1-6），形状保留，好让「迁移后的套件与迁移前逐条对得上」
可以一眼核对：

```ts
defineSuite("hub", { feature: "sessions" }, {
  needsModel: true,           // 没有模型也没有录像时跳过
  setup: async ({ backend }) => ({ /* 并进每个用例的上下文 */ }),
  tests: {
    "what this shows": async ({ client, backend, replay }) => { /* 抛错即失败 */ },
  },
});
```

Playwright 会把这些用例的位置记成 `suite.ts`（`test()` 在那里调用）。看用例名前面的套件名即可。

### 客户端

- `call(method, params)`：被拒就抛出 JSON-RPC 错误。
- `refused(method, params)`：返回错误，走通了返回 `null`。
- `waitFor(predicate, { timeout, describe })`：等一帧；`describe` 是超时消息里写的「在等什么」。
  **等条件，不等时钟**：这是「慢」和「不稳」的区别。
- `eventsOfKind(kind, sessionId)`、`text(sessionId)`：收到了什么。
- `handshake`、`project`、`backend`。

每个用例**自己一个客户端**。事件日志是按客户端记的，共用一个时，一个用例能匹配到前一个用例产生
的帧，有两个用例就这样隔一次绿一次红。

### 收尾

起过一轮的用例都以 `finish(client, sessionId)` 结束：中断，**等结算**，再删除。删一个还在跑的
会话，引擎会继续为一个不存在的东西干活，下一个用例来买单。

### 三种结果，不是两种

通过、失败，以及**不确定**：权限测试的模型压根没去调工具，就没有证明权限是好的；报绿是撒谎，报红
会让人学会无视红。抛 `inconclusive(reason)`，报告里是跳过，原因写在注释里。

## 6. 录像

```sh
node tests/live/record-fixture.mjs <名字> "<提示词>"    # 需要 .env
```

起一个 headless 的 harnix，在独立项目里建会话，发提示词，所有权限提问一律允许（否则工具调用的
录像停在提问那里），轮结束后把录像拷到 `tests/live/fixtures/recordings/<名字>/`。用例建会话时带
`options: { recorder: { name: "<名字>" } }`，回放模式下的后端就取这一份；不在回放模式时这个名字被
忽略，调的是真模型。

录像按位置回放。每个后端都有自己的数据根、HOME 和项目，全局记忆与 telemetry 不会串进来（N-003）。
隔离以后回放仍然错位（同一轮里可选的旁路调用时有时无），就向 Agix 提「按调用种类匹配」，不在
Harnix 侧绕过。

### 手做一份录像

插件 APP、原生工具这类录像大多是手做的：真模型不一定肯调那个工具，而测试要的只是「模型调了这个工具、然后说了一句话」。照已有的
`app-*` 做（`tests/live/fixtures/recordings/README.md` 里每一份都写了是从哪一份改出来的）：

1. 拷一份结构相同的：一次工具调用加一句话的，从 `calls-a-native-tool` 或 `app-echo` 拷；两个会话的（子 Agent），从 `app-subagent` 拷。
2. 改 `calls.jsonl` 里属于它自己的那几行：`tool_args_chunks` 的 `args`（工具输入的 JSON 文本）、`kind: "tool_use"` 那一块的 `name` 与 `input`、
   `text_chunks` 的 `texts`（最后那句话）。插件 APP 的工具名是 `app__<id 的 . 换成 _>__<工具>`，spawn 的实例里是
   `mcp__app-<id 的 . 换成 _>__<工具>`。
3. `blobs/` 里是请求的内容（按哈希存）。缺省回放按位置、不严格，请求和录下的不一样只报告，所以 blob 可以原样留着；要 `--strict` 也过，
   就得真录一份。
4. 在 `recordings/README.md` 的表里加一行：它演示什么、从哪一份改出来、改了什么。
5. 用例建会话时带 `options: { recorder: { name: "<名字>" } }`。

## 7. 覆盖表

每个用例带两个标签：`@feature:<id>`（`tests/live/features.mjs` 里的一项）、
`@layer:api|ui`。`tests/live/coverage.md` 由标签生成：

```sh
pnpm --filter @harnix/live-tests coverage          # 重新生成
node tests/live/coverage.mjs --check               # scripts/check.sh 跑的就是它
```

`--check` 在这几种情况下失败：

- `coverage.md` 和标签算出来的不一样（删了一个用例、加了一个没重新生成）；
- 用例没带标签，或者标了不存在的 `feature`；
- 某一行在某一层是零，`features.mjs` 里又没写 `na`（不适用，写理由）或 `pending`（哪个里程碑补）；
- 一行已经有用例了，还标着 `pending`。

### API 方法覆盖

跑完 `api` project 会打印两组数，比的是后端 `harnix.reachable` 说客户端能调的方法，不是手写的清单：

```text
── API coverage ──
  56/56 reachable methods called (100%)
  25/56 also driven into an error (45%)
```

第二个数存在，是因为第一个数太容易满足。一个只回答过、从没被拒绝过的方法，参数没被检查过，前置
条件没被测过，拒绝路径没走过。**它不会也不该到 100%**：不带参数的方法（`harnix.hello`、各种
`*.list`）没有失败可走。它的用处是：加了一个有参数的方法，这个数掉了，那就是信号。

第一个数是门槛：`scripts/check.sh` 跑整个 `api` project 时设 `HARNIX_COVERAGE_GATE=1`，后端说能调、却没有用例调到的
方法让这一层失败，除非 `features.mjs` 的 `UNCALLED` 写了它和理由；`UNCALLED` 里的方法有用例调到了也失败，免得清单
替已经做完的事继续开脱。只跑一个文件时不设，只打印。

## 8. 插件 APP 的测试

插件 APP 的套件要带 `apps` feature 的二进制：它连着应用运行时和 WebAssembly 引擎，缺省的构建里没有。

| 东西 | 在哪 |
|---|---|
| 带 `apps` 的 `harnix` | `cargo build --release -p harnix-cli --features apps --target-dir target/apps`；`scripts/check.sh` 编好后经 `HARNIX_APPS_BIN` 告诉套件，单跑时套件自己编（`tests/live/api/apps-host.ts`） |
| 打包工具 `harnix-app` | 同一个目录，`-p harnix-app-package --features compile --bin harnix-app` |
| 带 `apps` 的 `harnix-testapp` | 同一个目录；信 mock 商店、能缩短休眠的只有它（见下面的隐藏钩子） |
| 示例应用 | `examples/apps/showcase`：测试开始时编组件（`wasm32-wasip2`）、为这个引擎编译、打成 `.hxapp`，按源码哈希缓存在 `target/apps/showcase/` |
| mock 商店 | `tests/live/api/apps-store.ts` 在临时目录里用 `harnix-app sign` / `countersign` / `index` 把 showcase 做成两个版本（0.2.0 多要几项能力）、写出 `index.json`，经 `[apps] store = "file://…"` 给后端；mock 的发布者与商店密钥在 `tests/live/fixtures/store/`，只是测试钥 |

**Rust 这边**：

- `harnix-wasm` 拿真组件跑：`crates/wasm/tests/fixtures/` 下的组件用 `cargo build --target wasm32-wasip2` 现编（rustc 的 wasip2 链接器直接出组件，
  不需要别的工具），按源码哈希缓存在 `target/tmp/` 下。第一次之前 `rustup target add wasm32-wasip2`。它跑两遍：`cargo test -p harnix-wasm` 是产品的
  构建（不带编译器，那里的测试加载编译工具编出的产物），`cargo test -p harnix-wasm --features compile` 跑组件测试。
- `cargo test -p harnix-app-host --features compile`：应用宿主对着一个假的 `Platform`，`Platform` 与 `Control` 两个方向的每次调用都经
  `harnix-app-port` 的回环适配器（feature `loopback`）用 `serde_json` 编码再解码，证明这条边界能搬上 RPC。
- `cargo test -p harnix-apps --features compile`：嵌入的粘合层，照 `apps` 模块那样装、用、停、卸，和装到一半失败的回滚；装签名包、篡改、
  升级与升级失败的回滚在 `crates/apps/tests/signed.rs`。
- 签名、目录、披露的逻辑另有单测（`crates/app-package/src/{signature,store,disclosure,publish}.rs`）；休眠的规则在
  `crates/app-host/tests/idle.rs`（`HostOptions::idle`）与 `crates/wasm/tests/hibernate.rs`。

**LIVE 这边**（都在 `tests/live/{api,ui}/apps*.spec.ts`）：装与运行时、后端、商店、iframe、行内点、休眠，API 与界面各一份，两种拓扑都跑。

- iframe 的套件装 showcase：未签名、商店签的界面画在沙箱 iframe 里，受信发布者的进页面，`[interface] isolate = true` 时同一个应用也进 iframe
  （套件用 mock 发布者的钥匙现签一份）。iframe 里的东西用 `page.frameLocator` 找，看它自己的世界（存储、令牌、网络）用 `frame.evaluate`。
- `@harnix/app-sdk` 与 `@harnix/ui-kit` 有自己的 Vitest（SDK 带 mock 宿主）；showcase 带着两者构建出的文件（`ui/frame/vendor/`），
  `node tests/app-ui-vendored.mjs` 在 `frontend` 层查它们是不是最新的（`--write` 重新生成）。
- 休眠的套件在测试里把 showcase 改成 `[backend] resident = "on-demand"`（`watch_sessions = false`，`init` 不订阅），跑在带 `apps` 的
  `harnix-testapp` 上。

**隐藏的测试钩子**（`AppBuilder` 上 `#[doc(hidden)]` 的三个方法，产品用不着，profile 也改不了；`harnix-testapp` 用它们）：

| 钩子 | 做什么 | `harnix-testapp` 怎么用 |
|---|---|---|
| `app_host(module)` | 除了 `apps` 内建模块，也把应用宿主给这个模块（没有 `[apps] enabled` 就没有宿主可给） | 它的 `fakehost` 模块装一个假应用，测运行期登记 |
| `store_keys(keys)` | 用这些 hex 的 ed25519 公钥验商店副署，替掉内置的商店公钥（只在带 `apps` 的构建里读） | mock 商店的公钥（`tests/live/fixtures/store/mock-store.pub`） |
| `app_idle(duration)` | 所有 `on-demand` 后端闲置这么久就休眠，替掉清单的 `idle_secs`（不短于一分钟的那条规则只对清单） | 环境变量 `HARNIX_TESTAPP_APP_IDLE_MS`（毫秒），休眠的套件设它 |

`harnix` 内置的是商店自己的公钥，profile 改不了，所以 mock 商店在它那里什么也装不上（套件也测这一条）。

## 9. 应用写自己的 LIVE：`@harnix/testkit`

在 Harnix 上做的应用用同一套东西测自己：`@harnix/testkit` 起进程、连客户端、给 Playwright 夹具。导出与选项的全表见
[SDK](harnix-sdk.md) §24。

- **起谁**：`startHarnix({ bin })` 的 `bin` 写应用二进制的绝对路径（不写是 `harnix`）。每个后端有自己的 HOME、数据根、引擎根、项目、端口。
- **配什么**：`profile`（追加在 `[transport]` 之后的 TOML）、`env`、`args`、`topologies`、`seed`（引擎读之前就要在的状态）、`uiDir`。
- **模型**：`replay: true` 读的是 Harnix 自己的 `tests/live/fixtures/recordings`；应用要用自己的录像，`args: ["--replay-dir", <目录>]`；
  `replayStrict: true` 是严格回放。
- **标签**：用例打 `@feature:<功能>`、`@layer:api|ui`，应用可以照 `tests/live/coverage.mjs` 生成自己的覆盖表。
- **收尾**：`await backend.dispose()`（等进程退出再删目录），不要用 `stop()`；起过一轮的用例以 `finish(client, sessionId)` 结束。
- **组件测试**：`link:` 进来的 `@harnix/react` 会从 `harnix/` 那边解析 `react`，vitest 里写 `resolve: { dedupe: ["react", "react-dom"] }`。

写插件 APP 的人测法一样：`bin` 指带 `apps` 的二进制，profile 加 `[apps] enabled = true`、`dev = true`，`harnix.apps.installFile` 装自己的 `.hxapp`
（[插件](harnix-plugin.md) §11）。

## 10. 协议类型

`@harnix/core/protocol` 的 `generated/contract.ts` 由 `harnix-contract` 生成：

```sh
cargo test -p harnix-contract --features ts       # 改了 harnix-contract 的类型后
```

`scripts/check.sh` 的 `frontend` 层把它生成到临时目录再比对，手改或忘了重新生成都会失败。
`ts-rs` 只在 `ts` feature 下链接，不进发布的二进制。
