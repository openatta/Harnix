# Harnix 0.4 —— 架构设计

> 状态：设计已定，待实施｜目标版本：Harnix `0.4.0`｜日期：2026-10-25（同日第二轮修订）
>
> 配套文档：
>
> | 文档 | 管什么 |
> |---|---|
> | 本文 | 0.4 的架构：是什么、为什么、边界在哪 |
> | [术语](glossary.md) | 每个词只有一个意思。本文、代码、讨论都照它用词 |
> | [路线](roadmap.md) | 里程碑、任务编号、验收 |
> | [状态](status.md) | 开发做到哪了，中断后从哪接上 |
> | [二开指南](harnix-sdk.md) | 给应用作者：怎么在 Harnix 上做一个应用、怎么迁移 |
>
> 来源：Helm Issue 01（已整体吸收进本文，**不必再对照它读**）、IPDrun 同步文档
> （`iprun-issue-01-sync.md`，需求参考）、2026-10-25 维护者的两轮决定（§1）。
>
> 0.4 落地以后，[概念与架构](concept_and_architecture.md) 照 §18 重写，本文退役。

---

## 0. 一句话、目标、判据

**Harnix 0.4 是 Agix 应用的公共底座。** 一个应用由若干 space 组成，一个 space 可以带
前端、后端、Agent 三个部分里的任意组合。中区的对话所有 space 共用。

### 0.1 目标

1. **能被二开**：应用以子仓引入 Harnix 和 Agix，用 `harnix::App::builder()` 装配自己的模块，
   不 fork、不留补丁。
2. **实例可组织**：一个 Harnix 可以有多台互相隔离的 Agix 实例，进程内的、独立 daemon 进程
   的都行。实例、实例里的会话、会话里的任务，统一由 Harnix 登记和观察（§2）。
3. **自带调试面**：内建「系统」space，按「实例 → 会话 → 任务」逐层看到 Harnix 管着的一切，
   看得到每次模型调用带的上下文，并且能操作。
4. **每一步都可测**：每个里程碑交付完整的功能模块，带 API 与 Playwright 两层 LIVE 测试。

概念文档 §1.2 的三个目标（可定制、可在边缘部署、安全边界清楚）不变，这四条是 0.4 落地它们的
方式。

### 0.2 什么进 Harnix：判据

> **只有在公开契约之上做不出来，或者做出来会破坏 Harnix 不变量的东西，才进 Harnix。**

「第二个应用也会要」不够：那样几乎什么都能塞进来。一个能力满足下面任一条才进 Harnix：

- 它要碰中枢、授权、传输、装配的内部（例如无头会话：要和客户端共用排队与 `seq`）；
- 应用各写一份会破坏一条不变量（例如会话与 space 的对应关系）；
- 它本身就是契约的一部分（模块 trait、贡献点、宿主 API）。

其余的，即使两个应用都要，也先由应用在模块契约上实现；等两份实现证明是同一个形状，再考虑收进来。
Helm、IPDrun 的需求是**输入**，不是必须满足的清单。

---

## 1. 已定的决策

| # | 决策 | 来源 |
|---|---|---|
| D1 | Agix 必选，不做引擎可替换 | Helm C1 |
| D2 | 后端模块本期只做编译期的（Rust crate，在 `main.rs` 里显式链接） | Helm C2 |
| D3 | **Agix 的位置可以指定**：单独的 Harnix 用根目录下的 `agix/`；应用把 `harnix/` 和 `agix/` 作为两个平级的子仓，应用和 Agix 直接打交道，不经 Harnix 中转（§11）。Harnix 自己能独立构建出一个空应用 | 维护者 |
| D4 | **前端用 React + TypeScript + Vite + Zustand，定死** | 维护者 |
| D5 | Tauri 只是外壳，webview 里一切走 HTTP/WS | Helm C6 |
| D6 | 骨架照旧写死：传输、授权、中枢、装配 | 概念 §2.1 |
| D7 | 版本 `0.4.0`，属于架构升级 | 维护者 |
| D8 | **Harnix 跟着 Agix 持续升级** | 维护者 |
| D9 | 服务端驱动的会话（无头会话）进中枢内核 | 维护者 |
| D10 | MCP 服务端不进 0.4；方法注册表带 schema，以后可以投影成 MCP 工具 | 维护者 |
| D11 | **运行时按四层组织：Harnix → 实例 → 会话 → 任务**；实例是隔离单位，进程内与 daemon 两种载体，一组接口；space 的前端、后端都能建自己的实例 | 维护者 |
| D12 | **内建系统 space**（§9），space 这个概念保留 | 维护者 |
| D13 | **LIVE 测试**：对着真的 `harnix` 二进制跑；前端用 Playwright，后端用 API；模型默认录像回放，另有连真实模型的 live 模式（§13） | 维护者 |
| D14 | Agix 缺的能力写成 issue 放进 `.local/`，由它驱动 Agix 开发，Harnix 侧不绕过 | 维护者 |
| D15 | daemon 实例 0.4 只支持本机（unix socket 或回环 ws） | 维护者 |
| D16 | Harnix 能 spawn 并做最小看护：启动、就绪判定、随 Harnix 退出、`restart = never \| on-failure`；默认起 `harnix daemon`，也可以 attach 外部 `agixd` | 维护者 |
| D17 | 系统 space 可以观看并操作任何 space 的会话，操作写审计 | 维护者 |
| D18 | 界面（包括运行期包）只能按模板建实例，并且只能用标了 `ui = true` 的模板 | 维护者 |
| D19 | 版本节奏：M0、M1 按 0.3.z 发；M2 合进 main 时发 0.4.0，之后每个里程碑只升 z。**协议形状在 M2 定下**（`instance` 字段、会话键），0.4.z 里协议与模块 API 只做加法 | 维护者；第二轮补 |
| D20 | **所有实例都由 Harnix 按账本重建**（第二轮改：原来模块建的由模块自己重建） | 第二轮 |
| D21 | IPDrun 的 `nest-client` 收进 `@harnix/core`；`nest-ui` 只收对话主干（对话、工具行、权限卡、markdown、diff、队列、草稿），语音、上下文环、快捷回复这类产品面留在 IPDrun | 第二轮改 |
| D22 | 一个 space 的会话可以分布在多台实例上；space 有默认实例 | 维护者 |
| D23 | 进 Harnix 的判据是 §0.2 | 第二轮 |
| D24 | **实例只从声明来**：profile 声明的固定实例，或者按模板建的。模块和界面都只能按模板建；模板是装配的一部分（§3.2） | 第二轮 |
| D25 | **一台 daemon 只服务一个 Harnix**。不支持几个宿主共用一台 daemon（§5.7） | 第二轮 |
| D26 | 批准原语**不进** Harnix 内核，由应用在模块契约上实现（做法见二开指南） | 第二轮 |
| D27 | 授权主体只有 `Kernel / Module(id) / Device`。角色令牌连同主体类型一起推到 0.4 之后，和 MCP 一起做 | 第二轮 |
| D28 | 运行期包新增 space、往别的 space 加页签（`extensible`、`allow_runtime_spaces`）推到 0.4 之后 | 第二轮 |
| D29 | 迁移只管顺序与失败隔离；不做备份钩子，不做 `harnix migrate --dry-run` | 第二轮 |
| D30 | 前端只分 4 个包（§4.2）；协议类型用 `ts-rs` 从 `harnix-contract` 生成 | 第二轮 |
| D31 | 术语照 [glossary.md](glossary.md)：「实例」只指 Agix 实例；space 左栏里用户新建的东西叫「条目」；space 的三个组成叫「部分」，不叫「模块」 | 第二轮 |
| D32 | 「防误」和「安全边界」分开写，每条约束标明是哪一种（§14） | 第二轮 |

**为什么选 React（D4）**：用的人最多，后续好维护；Agent 类界面的生态基本都是 React 的
（dsh 的 Web UI 是 React 18 + zustand，assistant-ui、Vercel AI SDK UI、LobeChat、Dify
也都是）；IPDrun 已经有能对着 Harnix 协议跑的 React 客户端与对话组件，可以直接回流。代价是
Helm 要把现有的 7 个 Vue 文件（约 2.5k 行）迁过来。

---

## 2. 运行时模型：四层

这是 0.4 最核心的一张表。Harnix 里一切和 Agent 有关的东西都落在这四层之一。

```text
Harnix 进程
 ├─ 实例 system           （进程内）  ← 隔离单位：数据根、设置、provider、MCP、插件、场景
 │   ├─ 会话 a            ← 一条轮次循环；同一时刻至多一轮
 │   │   ├─ 任务：子 Agent "explore"     ← 属于会话，跟着会话的权限、项目根、生命周期
 │   │   ├─ 任务：后台 Bash `npm run dev`
 │   │   └─ 任务：定时任务 0 9 * * *
 │   └─ 会话 b
 ├─ 实例 helm-l1          （进程内，Helm 模块按自己的模板建的）
 │   └─ …
 └─ 实例 ci-runner        （daemon，本机另一个进程）
     └─ …
```

| 层 | 是什么 | 拥有什么 | 隔离到什么程度 | 谁建 | 身份 |
|---|---|---|---|---|---|
| **Harnix 进程** | 一个二进制，一个用户 | 中枢、授权、传输、实例登记表、`.harnix/` 账本 | — | 运维 | — |
| **实例** | 一台 Agix 引擎，一个会话池 | 数据根（设置三层、transcript、记忆、技能、录像）、provider 路由与凭据、MCP 连接、插件、场景、原生扩展 | **彼此完全隔离**：一台实例里的任何配置、状态、会话都不会出现在另一台里。daemon 载体还隔离了崩溃和资源 | profile 声明；模块、界面按模板建（D24） | `instance` |
| **会话** | 实例里的一条轮次循环 | transcript、场景（创建时定，决定工具集）、项目根、模型、权限规则 | 共享所属实例的配置；彼此的上下文互不可见 | 客户端、模块（无头）、模型（子 Agent 的侧链） | `(instance, session)` |
| **任务** | 属于某个会话、在会话之外继续跑的工作 | 子 Agent（侧链 transcript）、团队成员、执行中的 Shell 与监视器、定时任务 | 跟着所属会话：同一套权限、项目根；会话删了，任务一起没 | 会话里的模型（经工具） | `(instance, session, task)` |

几条由这张表直接推出来的规则：

- **隔离只在实例这一层承诺。** 要两套互不相干的配置（不同的 MCP、不同的凭据、不同的场景），
  就建两台实例，不在一台实例里靠会话区分。
- **任务是 Agix 的东西，Harnix 只看、只转发停止。** 子 Agent 的侧链在 Agix 里也是一个「会话」，
  在 Harnix 里它是父会话的任务，不出现在会话列表的顶层。任务不进中枢的键，不进账本。
- **会话不跨实例，也不跨 space**。要跨就用 `@session` 引用或分叉。前台 space 只看到、只操作
  自己的会话（§8.3）。
- **轮不一定由 Harnix 发起。** 定时任务（Agix 的调度器，`schedule.*`）会让一个会话自己开始一轮。
  中枢不能假定「我没发起的轮就不存在」：实例在 `daemon.event` 上报出每一轮的开始与结束
  （`turn_started` / `turn_completed`，带 `initiator`），中枢据此知道（Agix 0.2.7）。

**进程内实例的隔离由 Agix 承诺。** Agix 0.2.7 起，同一进程里的几台会话池互不可见，有守护测试
`two_pools_one_process`，新增进程级状态要登记理由。唯一共享的是 OTel 导出器（以第一台为准）。

---

## 3. 总体结构

```text
 浏览器（同一份产物；Tauri 只是装它的窗口）
 ┌───────────────────────────────────────────────────────────────────┐
 │ @harnix/shell  ── 外壳：布局、space 切换、设置；内建 space（会话 / HOME / 系统）│
 │ 应用的 space ── 应用自己的 React 组件，只依赖 @harnix/react               │
 │ @harnix/react  ── 贡献注册表、useNest()、i18n、令牌；对话组件 ChatPanel 等  │
 │ @harnix/core   ── 不依赖框架：协议类型（生成）、协议客户端、状态（zustand vanilla）│
 └─────────────────────────────┬─────────────────────────────────────┘
                               │ 五种通道语义（拓扑可选）· /apps/<id>/ 自有链接
 ┌─────────────────────────────▼─────────────────────────────────────┐
 │ harnix 进程（应用的二进制 = App::builder() + 模块）                       │
 │  传输 harnix-transport ── 通道、TLS、静态面、模块路由、启动链接             │
 │  授权 harnix-authz     ── 唯一准入点：主体 × 方法 / 路由，默认拒绝           │
 │  中枢 harnix-hub       ── 会话持有者：订阅、缓冲、turn、队列；两类调用方     │
 │  实例 harnix-instance  ── 实例登记表、模板、两种载体（EngineLink）           │
 │  模块 harnix-module    ── 编译期模块契约、注册表、生命周期                   │
 │  装配 harnix-app       ── App::builder()、profile、启动顺序、诊断            │
 └───────────┬───────────────────────────────┬───────────────────────┘
             │ 进程内调用                       │ JSON-RPC（unix socket / 回环 ws）
     ┌───────▼────────┐               ┌──────▼───────────────────┐
     │ Agix 实例   │  ……          │ Agix daemon 进程       │ ……
     │（作为库链接）    │               │（由 Harnix spawn 或 attach） │
     └────────────────┘               └──────────────────────────┘
```

### 3.1 相对 0.3 的结构变化

1. 「装配」拆成两层：**实例层**（`harnix-instance`，怎么建或连上一台引擎）和**应用装配**
   （`harnix-app`，这个进程由哪些模块组成）。
2. 中枢下面不再是「一个引擎」，而是**实例登记表**。中枢只认 `EngineLink`，不知道对面在
   进程内还是另一个进程里（§5.3）。
3. 内建功能全部改成**内建模块**，只走公开的模块 API。

依赖方向照旧单向，由 `tests/layering.rs` 断言：

```text
harnix-app ─→ harnix-transport ─→ harnix-authz ─┐
   ├──→ harnix-builtin(模块) ─→ harnix-module ─→ harnix-hub ─┴─→ harnix-contract
   │                                          └─→ harnix-instance ─→ Agix
```

| 层 | 不许知道 |
|---|---|
| 传输 | 会话、实例是什么 |
| 授权 | 帧长什么样、走的哪条通道、实例在哪 |
| 中枢 | HTTP、TLS、连接、拓扑、**实例的载体** |
| 实例层 | 客户端面协议、space、模块 |

### 3.2 这一轮对两条原则的修订

0.4 有两处和 0.3 的原则字面冲突。维护者决定保留这两项能力，所以改的是原则的写法，并写清楚
为什么改了之后原意还在。

**① 「运行期不改装配」（概念 §3.1、§10）。** 0.4 允许运行期建实例（D11、D18），字面上就是
运行期改装配。原则的本意是「运行期没有东西能决定这个进程是什么」。修订为：

> **装配在启动时定：有哪些模块、哪些模板、哪些载体。运行期能新建的只有会话，以及模板允许的
> 实例。模板本身不能在运行期新增或改写。**

这样，运行期建出来的每台实例在启动时就已经能被枚举（模板 × 允许的参数），「这个进程能变成
什么」仍然在启动时定死。D24（实例只从声明来）和 D20（Harnix 统一重建）就是这条修订的执行方式。

**② 「以库链接，不起子进程、不走 socket」（概念 §9）。** daemon 载体字面上违反它。原则的本意
是两件事：中枢是会话的唯一持有者；Harnix 不因为连接方式不同而改变会话语义。修订为：

> **进程内实例以库链接；daemon 实例经本机 JSON-RPC。两种载体对中枢是同一个 `EngineLink`，
> 会话语义不因载体改变；一台 daemon 只服务一个 Harnix（D25），中枢仍是它上面会话的唯一持有者。**

---

## 4. 模块划分与前后端对齐

### 4.1 后端 crate

| crate | 0.3 里的前身 | 职责 |
|---|---|---|
| `harnix-contract` | 同名 | 帧、握手、主体、错误码，纯类型。经 `ts-rs` **生成** `@harnix/core` 的协议类型 |
| `harnix-instance` | `harnix-assembly` | `EngineLink` 接口；进程内载体（调 `daemon::assemble::pool`）；daemon 载体（RPC 客户端、spawn、attach）；模板；实例登记表 |
| `harnix-hub` | 同名 | 会话持有，键是 `(instance, session)`；服务端调用方（无头会话、tap、`OnBusy`）；带参数的宿主主题；实例 → 会话 → 任务的整值快照 |
| `harnix-authz` | 同名 | 主体 `Kernel / Module(id) / Device`；方法表与路由表；审计 |
| `harnix-transport` | 同名 | 五种通道语义、三种拓扑、静态面、`/apps/<id>/` 路由、启动链接、Origin 白名单 |
| `harnix-module` | `harnix-contrib` | `Module` trait、`Manifest`、各注册表、`ModuleContext`、迁移生命周期、`MODULE_API_VERSION` |
| `harnix-app` | `crates/app` 的 `main.rs` | `App::builder()`、profile、启动顺序、诊断页、`--version`、`harnix daemon`；内建的 `devices` 模块（它要碰授权层的数据） |
| `harnix-builtin` | 同名 | 内建模块：会话 space、HOME、系统 space、设置、包管理、录像（每个都是一个 `Module`） |
| `harnix`（lib + bin） | `crates/app` | lib 是应用依赖的门面（`harnix::App`、`harnix::prelude`）；bin 是 `App::builder().with_builtins().run()`，就是那个「空应用」 |
| `harnix-testapp`（bin） | — | 测试用的应用：能用的模块与故意失败的模块，LIVE 的 `modules`、`routes`、`migrations` 套件跑它 |

### 4.2 前端包（pnpm workspace；包在 `ui/packages/`）

只分 4 个包。包的边界只画在「谁依赖谁」真正不同的地方，拆分以后随时可以做，合并却难。

| 包 | 前身 | 依赖框架吗 | 职责 | 谁用 |
|---|---|---|---|---|
| `@harnix/core` | IPDrun `packages/nest-client`、`ui/runtime/{client,topology,protocol,state}.js` | 否 | 子路径导出三层：`/protocol`（`ts-rs` 生成的类型与常量）、`/client`（握手、拓扑与回退、call / notify / answer、seq 追赶、连接状态）、`/state`（会话事件归约、实例树、队列、每个 space 的当前会话；zustand vanilla） | 一切 |
| `@harnix/react` | `ui/runtime/{host,contrib}.js`、IPDrun `packages/nest-ui` 的对话主干、`ui/shell/conversation.js`、`composer.js` | React | 贡献注册表（8 个点）、`useNest()`、i18n、令牌、图标；ChatPanel、工具行、权限卡、子 Agent、markdown、diff | **应用写 space 界面只依赖它** |
| `@harnix/shell` | `ui/shell/*`、`ui/main.js`、`ui/builtin/*` | React | 三栏布局、space 切换与恢复、设置页、模态框、断点；内建 space（会话、HOME、系统）放在 `src/spaces/` 下 | 应用的入口 |
| `@harnix/testkit` | `tests/api/harness.mjs`、`tests/browser/setup.mjs` | 否 | LIVE 测试的启动器、API 客户端、Playwright 夹具与组件对象（§13） | 测试 |

**分层的用意**：框架只出现在 `@harnix/react` 及以上。`@harnix/core` 是纯 TS，用 Vitest 测，
换框架也不动它，并且和后端一一对应：`/protocol` ↔ `harnix-contract`，`/client` ↔ `harnix-transport`，
`/state` ↔ `harnix-hub` 推出的事件与整值快照。

**内建 space 只用公开 API**：`@harnix/shell/src/spaces/` 下的代码只许 import `@harnix/react`、
`@harnix/core` 的公开导出，由检查脚本守住。这和后端「内建模块只用公开模块 API」是同一条。

**协议类型怎么生成**：`harnix-contract` 的类型在 `ts` feature 下 derive `ts_rs::TS`（release
构建不开，不进二进制预算）；`cargo test -p harnix-contract --features ts` 导出到
`ui/packages/core/src/protocol/generated/`，生成物提交进库，检查脚本重新生成并比对，漂移就失败。

### 4.3 功能模块：前后端对齐表

每个功能模块是一条完整的竖切：后端一块、前端一块、两层测试各一组。开发计划（[roadmap](roadmap.md)）
按这张表推进，**一个里程碑交付一个或几个完整的行**。

| 功能模块 | 后端 | 前端 | API 套件 | Playwright |
|---|---|---|---|---|
| 握手与连接 | transport、contract | core/client | `handshake`、`topology` | `connection` |
| 授权与设备 | authz、`devices` | shell（设置·设备） | `authz`、`devices` | `pairing` |
| 会话与轮次 | hub | core/state、react | `sessions`、`turns`、`hub`、`permissions` | `conversation`、`permission` |
| 无头会话 | hub（服务端调用方） | — | `headless` | `watch-headless`（界面旁观） |
| 实例层 | instance、hub | core/state（实例树） | `instances`、`instances-daemon` | — |
| 模块契约 | module、app | react（注册表） | `modules` | `module-panel` |
| space 与会话绑定 | builtin（账本）、hub | shell、react | `spaces` | `spaces` |
| 会话 space | builtin（工作区、标题、搜索、文件、上传） | shell/spaces/sessions | `workspaces`、`files` | `sidebar`、`composer` |
| HOME | builtin | shell/spaces/home | `home` | `home` |
| 系统 space | builtin（系统模块） | shell/spaces/system | `system` | `system-*` |
| 设置 | builtin（设置） | shell（设置各节） | `settings` | `settings` |
| 运行期包 | builtin（包管理） | react（import map） | `plugins` | `package-ui` |
| 迁移 | module、app | shell（诊断页） | `migrations` | `diagnostics` |
| 模块自有链接 | transport、authz | react（`space.fetch/socket`） | `routes` | — |
| 扩展条目 | module、hub | react（`flow.block`） | `entries` | `entry-card` |
| 品牌与主题 | app（profile `[brand]`） | react、shell | `brand` | `brand` |
| 应用构建 | `templates/app`、`check-pins` | `templates/app/ui` | `build`（脚本层） | — |
| 桌面壳 | `harnix-desktop` 模板 | — | — | `desktop`（本地与远程模式） |

---

## 5. 实例层

### 5.1 两种载体

| | **进程内** | **daemon** |
|---|---|---|
| 是什么 | `daemon::assemble::pool(...)` 建在本进程里，调 `dispatch_public` | 本机另一个 Agix daemon 进程，经 JSON-RPC 连接（unix socket 优先，其次回环 ws） |
| 怎么来 | Harnix 建 | **spawn**：Harnix 起子进程并做最小看护（D16），默认起 `harnix daemon`；**attach**：连一个配置好地址的、只给这个 Harnix 用的 daemon（D25） |
| 能带原生 Rust 贡献吗 | 能：场景、原生工具、`SessionCustomizer`、凭据源经 `Assembly` 交进去 | 不能，只有那个 daemon 自己的配置与插件 |
| 隔离 | 状态隔离（Agix 0.2.7 起）；崩溃与资源共用进程（轮次级隔离照旧） | 状态、崩溃、资源都隔离；断了是「实例降级」，不是 Harnix 挂掉 |
| 版本 | 必然是这次构建链接的那版 Agix | 握手时比对协议版本，不匹配就拒绝并说清楚，不静默降级 |
| 代价 | 每台的常驻内存记进预算（§10.6） | 另一个进程；不计入 Harnix 的内存预算 |

**`harnix daemon` 子命令**：Harnix 二进制里本来就链着 Agix，`DaemonServer` 也有 `serve_unix` /
`serve_tcp` 这些库入口，所以 `harnix daemon --scene … --socket …` 可以直接起一个 daemon。spawn
默认起的就是它自己，版本**按构造就对得上**，也不用另外分发一个 `agixd`。

**什么时候用哪种**：space 要一台带原生工具的实例（例如 Helm 带交易工具），用进程内；space 要
隔离崩溃与资源，或者想把它放到另一个进程里看护，用 daemon。

### 5.2 模板

**实例只有两个来源（D24）**：profile 里声明的固定实例，以及按模板建的实例。

```text
InstanceTemplate {
  id,                    // 全局唯一；模块的模板自动带上模块 id 前缀
  carrier,               // InProcess { scenes, contrib: EngineContrib } | Spawn { scene, args } | Attach { addr }
  data_root,             // 规则：在 Harnix 数据目录下，可以引用参数，例如 "instances/{name}"
  params,                // 允许调用方填的参数及其约束（名字、数据子目录之类）
  ui,                    // true：界面也能按它建（D18）
  max,                   // 可选：最多建几台
}
```

- **profile** 里 `[engine]` 建默认实例 `system`；`[[instances]]` 声明其余固定实例；
  `[[instance_templates]]` 声明模板（只能是 spawn 或 attach：profile 拿不出原生贡献）。
- **模块**在 `templates()` 里声明自己的模板（§7），进程内模板可以带原生贡献。模块在运行期用
  `ctx.instances.create(template, params)` 建，或者用 `ensure(template, name, params)` 确保一台
  存在（幂等，启动时常用）。
- **界面**用 `useNest().instances.create(template, params)` → `harnix.instances.create`，只能选
  `ui = true` 的模板，只能填模板放开的参数。**界面永远给不出二进制路径、命令行参数、环境变量、
  模型端点**：那些等于「装载子进程、改写模型端点」，概念 §3.4 明确不对设备开放。

模板是装配的一部分（§3.2 ①）：启动时注册，运行期不能新增或改写。

### 5.3 中枢与实例之间的接口：`EngineLink`

中枢只认这一个 trait。两种载体都实现它，中枢的代码里不出现「是不是远程」的分支。

```rust
/// 一台实例的连接。中枢看到的就是这些，看不到载体。
#[async_trait]
pub trait EngineLink: Send + Sync + 'static {
    /// 一次 JSON-RPC 调用。流式方法（run_turn、compact、clear）的事件走事件流，
    /// 这里只返回最终应答。链接不可用时立刻返回 INSTANCE_UNAVAILABLE。
    async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError>;

    /// 这条链接上全部的异步帧：session.event、daemon.event，以及链接自己的两种——
    /// 屏障标记和「重连上了」。只有一个接收端，第一个调用者拿走；重连由载体在内部完成，
    /// 接收端不换。
    fn take_events(&self) -> Option<EventReceiver>;

    /// 在调用时已经收到的所有帧后面放一个带 id 的屏障标记。中枢结算一轮前写一个，等它从
    /// 事件流里出来——等的是「已经路由」，不只是「已经收到」，否则 `turn_settled` 会超车
    /// 本轮最后的事件（IPDrun N-105 实测丢过 `turn_complete`）。
    async fn mark(&self, id: u64);

    /// 链接状态：Starting / Ready / Degraded { since_ms, reason } / Down { reason }。
    fn status(&self) -> watch::Receiver<LinkStatus>;

    fn identity(&self) -> Value;           // {version, protocol_version, capabilities}
    fn carrier(&self) -> Carrier;          // 只给实例列表看，中枢不按它分支
    fn pid(&self) -> Option<u32> { None }  // spawn 的子进程
    async fn shutdown(&self);
}
```

三种实现：`InProcessLink`（标记走引擎给这条连接的出站队列，先进先出）、`DaemonLink`（读线程先把
事件交出去再交后面的应答，所以应答到手时它前面的事件已经在流里，标记直接放进流）、`DownLink`
（建不起来的实例：账本里模板没了、拥有者停用了、载体起不来，列出来是 `down` 加原因）。

**`EngineLink` 是 Harnix 内核里的接口，不对模块开放。** 模块拿到的是 `ctx.instances` 返回的
`InstanceHandle`，所有调用仍然经过中枢和授权。模块不能绕过中枢直接对一台实例发
`session.run_turn`，否则「中枢先订阅」这条不变量就有了旁路。

### 5.4 实例登记表与账本

`harnix-instance` 持有一张表：`instance → { 载体, 模板, 参数, 拥有者, 所属 space, 数据根, 链接状态,
版本与协议版本, 能力位, 创建时间 }`。它是 Harnix 自己的状态，不在任何一台引擎里。

账本（`.harnix/instances.json`）记按模板建的实例：`instance → (模板, 参数, 拥有者, 所属 space)`。
登记表的活的那一半（每台的链接与路由任务）在中枢里（`harnix_hub::Slot`），模板、账本与怎么建
（`Factory`）在 `harnix_hub::Registry`；类型都来自 `harnix-instance`。实例的 id 就是建它时给的名字，
全进程唯一。M5 之前没有 space，界面建的实例拥有者记作 `ui`。**重启后由 Harnix
统一重建（D20）**：模板还在、拥有者模块启用着，就照账本重建；模板没了或模块停用了，这台实例在
系统 space 里显示为 `Down`，写明原因，数据根不动。

### 5.5 生命周期与归属

- 每台实例有**拥有者**：`kernel`（profile 里的）、`module:<id>`（模块按模板建的）、
  `space:<id>`（界面按模板建的，归属于发起时的前台 space）。
- 模块停用（迁移或启动失败）时，它拥有的实例和它的模板建出的实例一起停：进程内的关掉；spawn 的
  结束子进程；attach 的断开，不动对面。
- 实例停了，它的会话的 transcript 仍在它的数据根里；实例重建后照常 `resume`。
- 删除实例是显式操作（系统 space 或拥有者调用），删的是登记与账本条目；数据根默认保留。

### 5.6 会话与任务的地址

- 会话的全名是 `(instance, session)`；任务的全名是 `(instance, session, task)`。协议里所有和会话、
  任务相关的方法与帧都带上 `instance`；方法里省略时取前台 space 的默认实例（§8.4），**帧里始终
  带上**。这个形状在 M2 定下（D19），那时只有 `system` 一台。
- `seq` 照旧由中枢按会话分配，与载体无关。
- Harnix 的账本记 `会话 → (space, instance)`。

### 5.7 daemon 链接的语义

daemon 对一条通过认证的连接**完全信任**，并且没有逐方法授权（Agix 协议 §5.1）。所以：

- **独占（D25）。** 一台 daemon 只服务一个 Harnix：spawn 的天然如此（token 只有 Harnix 有）；attach
  的由运维保证，文档写明「几个宿主共用一台 daemon 不受支持」。理由：中枢的排队、结算、「先答者生效」
  都假定它是这台 daemon 上会话的唯一调用方；别的客户端直接跑轮，这些语义就没有了。
- **Harnix 持有 daemon 的 token**（经 `CredentialSource`），不写进 profile，不发给任何客户端；
  spawn 时经环境变量交给子进程，不放在命令行上。
- 透传白名单（`harnix_hub::PASSTHROUGH`）对每台实例都一样，`config.setProvider`、`mcp.addServer`
  这些对 daemon 实例同样拒绝。
- **断线**：实例变成 `Degraded`，daemon 那边的轮照跑（Agix §5.6）。中枢重连后对每个在观看的
  会话按「先 subscribe 拿 `last_seq`，再读 history，再接实时帧」重建缓冲，然后向客户端推
  `harnix.session_resync`，客户端照刷新那条路重新 attach。这期间的发送进中枢的队列，不拒绝。
  这是中枢本来就有的恢复路径，不是新语义。
- **丢订阅**：daemon 的出站队列满了，这条连接对该会话的订阅被取消，并收到一帧
  `subscription_dropped`（Agix §5.4）。中枢收到它就对这个会话走上面那条 resync。
- **不是 Harnix 发起的轮**：会话被定时任务叫起来，`daemon.event` 的 `turn_started` /
  `turn_completed`（`initiator:"schedule"`）告诉中枢；中枢订阅 `daemon.event`，两种载体一样。
  落后时收到 `events_lagged`，用 `session.list` 对账。

---

## 6. 中枢

中枢的状态与语义仍然不可替换（概念 §2.1）。0.4 的改动：

### 6.1 第二类调用方：服务端（D9）

概念 §3.2 的「turn 由客户端发起」改成：**turn 由中枢发起，调用方有两类，客户端和服务端
（内核、模块），语义相同**，排队、中断、结算、广播都一样。「中枢先订阅」不变。

服务端调用方的 API 就是 IPDrun 在 vendor 里验证过的那套（N-101、N-102、N-103、N-107、N-108），
收进 `ModuleContext.sessions`：

| 能力 | 说明 |
|---|---|
| `create / open / send / interrupt / close` | `SessionSpec`：实例、space、场景、项目根、provider、模型、权限模式与规则、`output_schema` |
| `tap(session)` | 服务端的观看者，收到客户端观看者会收到的全部会话通知 |
| `OnBusy { Queue, Reject, Interrupt, Steer }` | 客户端的 `harnix.send` 和服务端的发送共用一条路径 |
| `append_entry(session, entry)` | 往会话时间线写扩展条目（§7.6） |
| 界面旁观 | 无头会话照样出现在它所属 space 的会话列表里，界面随时可以 attach |

**tap 的背压**：tap 和客户端观看者是同一套语义，不是一条特殊的无损通道。它是有界通道，按 `seq`
递增投递；投递**不在中枢的锁里等待**，通道满了就把这个 tap 标成落后，它下一次读到
`Lagged { last_seq }`，按客户端追赶那条路（`catch_up`）补齐。**中枢永远不会因为一个慢的 tap 卡住
会话。** IPDrun 记的「tap 对 `open_turn` 的 `user_message` 收两次」随投递路径统一而消失。

### 6.2 跟着 Agix 0.2.6 必须做的修正

这几条不是功能，是升级 Agix 后的正确性问题，IPDrun 已经定位过。它们是 M0 的验收内容：

| IPDrun 编号 | 问题 | 修法 |
|---|---|---|
| N-001、N-007 | `engine.rs` 抄了一份装配 | 改成调 `daemon::assemble::pool`，补上 `scenes`、`credentials`、`customize_session` |
| N-105 | `turn_settled` 超车本轮最后的事件 | 结算前过屏障（M3 起是 `EngineLink::barrier`） |
| N-120 | 运行中的一轮已经部分落盘，attach 时会画两遍 | `catch_up` 按「本轮开始前的消息数」给水位 |
| N-121 | `tool_args_delta` 不合并，重放缓冲顶到上限 | 按 `id` 合并；完整 `tool_use` 到达时丢掉同 id 的片段 |
| N-106 | options 重交 | 只在旧会话恢复时带上 |
| N-002 | 中枢自记一份 `project_root` | 以引擎的会话列表为准，删掉自记的 |
| N-003 | `13-agent` 有 6 个用例失败（录像回放错位） | 每个套件独立的数据根和 HOME；仍然错位就提 Agix（§13.3） |

### 6.3 实例树的整值快照

中枢按四层推整值快照（概念 §3.2「推送一律整值」），都是**带参数的宿主主题**（§7.5）：

| 主题 | 参数 | 内容 |
|---|---|---|
| `harnix.instances` | — | 实例登记表：每台的载体、模板、拥有者、所属 space、链接状态、版本、会话数 |
| `harnix.instances.sessions` | `instance` | 这台实例的顶层会话：状态（空闲 / 运行 / 排队）、场景、项目根、所属 space、任务数 |
| `harnix.sessions.tasks` | `instance`、`session_id` | 这个会话的任务（Agix `session.tasks`）：种类、状态、开始与结束时间、细节；随 `task_started` / `task_finished` 更新 |
| `harnix.sessions.status` | — | 所有会话的运行状态（IPDrun R-11），给会话列表用 |

对应的一问一答方法 `harnix.instances.list`、`harnix.instances.sessions`、`harnix.sessions.tasks` 返回同样
的形状。建、停实例是 `harnix.instances.templates`（调用方能用的模板）、`.create`、`.ensure`、`.stop`，
谁能用哪个模板、停哪台由调用方的主体决定（§5.2、§5.5）。

值在有人订阅时才算：订阅时现算一次，之后在它读到的东西变了时重算，短时间内的多次变化合成一次。
订阅一个会话的任务会让中枢订阅这个会话（只限在内存里的会话，不为了看任务把冷会话恢复）——任务事件
走的是会话自己的事件流。

### 6.4 其余

- **`harnix.hello`** 带上权限提问超时（R-12）、各实例的协议版本。
- 权限提问照旧「只有 `Device` 能回答，先答者生效，超时按拒绝」。它防的是什么、不防什么见 §14。

---

## 7. 模块契约（编译期）

### 7.1 模块是什么，不是什么

**名字叫 `Module`，不叫 `Plugin`。** 概念 §2.2 的标题是「只有一种插件体系，而它是 Agix 的」，
插件这个词留给 Agix 的包。编译期模块不是包，它是应用作者链接进来的源码，和内建模块同级。

扩展的来源与信任（概念 §2.3「信任按出身定」照旧成立，多了一种出身）：

| 来源 | 什么时候进来 | 谁写的 | 信任级 | 能加 space 吗 | 能加后端方法吗 |
|---|---|---|---|---|---|
| **编译期模块**（新） | 应用构建时链接 | 应用作者本人 | **内核级** | 能 | 能，在 `app.<id>.*` 命名空间 |
| 运行期包 · Agent 段 | 安装时 | 第三方 | 插件级 | — | 不能；能力门控 + 披露，**不变** |
| 运行期包 · `[[ui]]` 段 | 安装时 | 第三方 | 页面级 | **0.4 不能**（D28） | 不能 |

两条原规则因此**不用推翻**，只需写清适用范围：「Harnix 不执行包里的任何东西」（概念 §2.2.1）与
「不给插件贡献 `harnix.*` 方法」（概念 §10）都**对运行期包仍然成立**。

### 7.2 trait

```rust
/// 一个编译期模块。应用在 main.rs 里显式列出，顺序就是注册顺序。
#[async_trait]
pub trait Module: Send + Sync + 'static {
    fn manifest(&self) -> Manifest;                       // id、版本、MODULE_API_VERSION、披露文本
    fn engine(&self, _e: &mut EngineContrib) {}           // 给 system 实例的原生贡献：场景、原生工具、customizer、凭据源
    fn templates(&self, _t: &mut InstanceTemplates) {}    // 本模块的实例模板（§5.2），进程内的可带原生贡献
    fn spaces(&self, _s: &mut SpaceRegistry) {}           // 后端侧的 space 声明（会话策略、默认实例）
    fn methods(&self, _m: &mut MethodRegistry) {}         // app.<id>.*，带 schema 与主体声明，默认拒绝
    fn events(&self, _e: &mut EventRegistry) {}           // 宿主主题（整值快照），可带订阅参数
    fn routes(&self, _r: &mut RouteRegistry) {}           // /apps/<id>/ 下的 HTTP 与 WS
    async fn migrate(&self, _c: &MigrateContext) -> anyhow::Result<MigrationReport> {
        Ok(MigrationReport::none())
    }
    async fn start(&self, _c: ModuleContext) -> anyhow::Result<()> { Ok(()) }
    async fn stop(&self) {}
}

fn main() -> anyhow::Result<()> {
    harnix::App::builder()
        .profile_from_args()
        .with_builtins()
        .module(helm_home::Module::default())
        .module(helm_l1::Module::default())
        .run()
}
```

原生贡献只有两条路：给 `system` 的走 `engine()`，给自己实例的走 `templates()` 里进程内模板带的
`EngineContrib`。多个模块的 `SessionCustomizer` 由 Harnix 按注册顺序组合，**Agix 不用改**。

`ModuleContext` 有：自己的配置段、自己的数据目录、`credentials`（`CredentialSource`）、`events`
（发布自己的主题）、`sessions`（§6.1）、`instances`（按自己的模板建、`ensure`、停止、拿
`InstanceHandle`）。**拿不到中枢的内部状态，也拿不到 `EngineLink`。**

### 7.3 启动顺序

1. 读 profile。
2. **注册**：按顺序调每个模块的 `manifest` 与各注册函数。`MODULE_API_VERSION` 不匹配就**编译失败**
   （常量断言）；id、方法、路由冲突时后注册的那个停用，进诊断页。
3. **迁移**：按顺序调 `migrate`；失败只停用这个模块。
4. **建实例**：`system`（带上仍启用的模块的 `engine()` 贡献）、profile 的 `[[instances]]`、账本里
   按模板建过的（§5.4）。
5. **启动**：按顺序调 `start`；失败只停用这个模块和它的方法、路由、space、实例，进诊断页。
   它给 `system` 的场景留到下次启动（已建好的池子里摘不掉），它的 customizer 由 Harnix 组合时跳过，
   新会话不再带它的原生工具；诊断页写明这一点。
6. 开始服务。

### 7.4 授权主体与配置

- 主体：`Kernel / Module(id) / Device`（D27）。每个方法、路由声明给哪类主体用，进授权表，默认拒绝。
- 配置段：profile 里写 `[modules.<id>]`；其中的密钥只写引用，值由 `CredentialSource` 提供
  （可以接系统钥匙串），**不写进 profile 文件**。
- 迁移（D29）：Harnix 只管**什么时候迁、失败了怎么办**，不管**用什么迁**。数据库选型、迁移工具、
  连接池都由模块自己决定；Harnix 自己的 `.harnix/` 账本与模块数据分开。

### 7.5 前后端之间的通道：不加新语义

五种通道语义（概念 §3.3.1）不变：

| 模块需求 | 用什么 |
|---|---|
| 一问一答 | unary，调 `app.<id>.*` |
| 状态更新 | 宿主事件主题，推整值快照；**支持带参数订阅** |
| 长任务进度 | 启动时返回任务 id，进度作为带参数的主题推送（`app.<id>.job` + `job_id`） |
| 大文件 | 大负载旁路 |

主题的「追加模式」（复用 `seq` 追赶、有界缓冲）本期不做，出现真实的高频增量需求再开 issue。

### 7.6 往对话里写东西：扩展条目

例如「采集任务跑完，往对话里推一张任务卡」：

- 走 Agix 的 `history.extension_entry`（引擎已开放的注册点），经 `ctx.sessions.append_entry`，
  由 `flow.block` 画出来。卡片进入会话日志，刷新、重放、分叉都看得到。
- 这类条目**只给人看，不给模型看**。
- Harnix 不另存一份会话状态（概念 §9「不在引擎之外维护第二份会话状态」）。

**M7 的实际做法**（2026-10-26）：Agix 0.2.7 只在库里开放了这个注册点，daemon 协议里既写不进、
也读不回，整段分叉还会丢掉最后一条消息之后的条目，所以先补了 Agix（issue 06，0.2.8）：
`session.appendEntry`、`session.entries`（`at` 是条目前面的消息数）、`extension_entry` 帧。Harnix 这边：
中枢的 `harnix.entries.append` 只给模块与内核，命名空间由中枢定成 `app.<模块>`；`session.entries`
直通给设备读，核对 space；界面打开会话时按 `at` 把条目插回消息之间，实时的按 id 去重，都成为
`custom` / `entry` 块交给 `flow.block`。

### 7.7 模块自有链接

**有后端模块的 space，它的后端和前端之间可以有自己的链接**，承载 Harnix 通道不适合的东西（例如
模块自己的流式协议）：

- 挂在 `/apps/<id>/` 下：HTTP 路由与 WebSocket，由 Harnix 的传输层发出，与界面**同源**。
- 准入走同一套：同一份 token / 设备凭据，在授权层按「主体 × 模块路由」判定，默认拒绝，有审计。
- 前端经宿主 API 的 `space.fetch()` / `space.socket()` 使用，凭据由宿主带上；模块代码不自己拼
  地址，也连不到别的来源（CSP 不放宽）。
- 自有链接**不享受**拓扑无关、`seq` 追赶这些 Harnix 通道的保证。要这些保证，就用 §7.5 的通道。
- Tauri 下同样走 HTTP/WS。

### 7.8 不进内核、由应用自己做的

按 §0.2，下面这些应用在模块契约上就做得出来，0.4 不进 Harnix。做法写在[二开指南](harnix-sdk.md)：

- **批准**（D26）：「这个业务动作要不要人确认」。用限 `Device` 的 `app.<id>.*` 方法收回答、带参数
  的主题推待批事项、扩展条目记下决定、`flow.block` 画卡片；先答者生效、超时按拒绝由模块自己实现。
  引擎权限管「这个工具能不能跑」，批准管「这个业务动作要不要人确认」，后者的策略本来就归应用。
- **角色令牌与 MCP 服务端**（D10、D27）：0.4 之后和 MCP 一起做。在那之前应用要给外部 Agent 开 MCP，
  在自己的模块里起自己的监听，它不在 Harnix 的授权与保证之内，由应用负责。

---

## 8. Space 模型

### 8.1 定义

**space = 左栏应用 + 右栏应用，一起注册，所以「成对」是原子的。** 一个 space 包括：

- 左栏入口（图标、排序、可用 / 置灰 + 角标），以及可选的**条目**：用户在这个 space 里新建的东西，
  例如会话 space 的工作区、Helm 的一个关注组。条目存在 `.harnix/` 账本里。
- 右栏页签（每个页签一个前端组件）和详情抽屉。
- 中区模式：`chat`（默认）/ `minimized`（对话收成底部一条，可以展开）/ `hidden`。
- 会话策略（§8.3）与新建会话的默认值：场景、标题、初始上下文、**默认实例**（§8.4）。

一个 space 的三个**部分**：

| 部分 | 在哪执行 | 来源 |
|---|---|---|
| 前端 | 浏览器 | 编译期（打进应用的界面产物）；运行期 `[[ui]]` 包只能给已有的贡献点做贡献（D28） |
| 后端 | Harnix 进程内 | **只有编译期**：一个 `Module` |
| Agent | Agix 实例里 | 原生 Rust（经模块的 `engine()` 或模板）或运行期 Agix 插件包（WASM / QuickJS） |

**最大集是三个部分都有；也可以只有其中一部分**，例如只有前端加 Agent，或只有前端。

### 8.2 布局

```text
┌──────┬──────────────────────────────┬──────────────┐
│ 左栏 │ 顶栏：会话标题 · 右栏开关       │ 右栏页签      │
│ space│                              │（属于当前     │
│ 入口 │ 对话区（所有 space 共用一个）   │   space）     │
│ 与   │                              │              │
│ 条目 │ 输入框                        │ 详情抽屉      │
│ 设置 │                              │              │
└──────┴──────────────────────────────┴──────────────┘
```

断点沿用现有规则：先收右栏，再把左栏收成 56px 图标栏，<960px 两侧都变抽屉。右栏宽度可以拖拽，
收缩状态记在本地。

### 8.3 会话规则

| 规则 | 内容 |
|---|---|
| 绑定 | 一个会话在创建时绑定一个 space 和一台实例，**不能挪**；要跨 space 延续，就用 `@session` 引用或分叉成新会话 |
| 多会话 | 一个 space 可以有多个会话，可以分布在本 space 能用的几台实例上（D22） |
| 切换 | 离开一个 space 时记住它当前的会话；切回来时恢复这个会话 |
| 创建 | space 的前端或后端可以新建会话，新会话自动绑定到这个 space，带上它的默认场景、初始上下文、默认实例 |
| 上下文 | space 的代码可以读当前会话的上下文，也可以把某个会话设为当前会话（限本 space 内） |
| HOME | HOME 有自己的一个通用会话，底部最小化的那条对话连到它 |

**硬理由**：Agix 的会话在创建时就定了场景，场景决定工具集。不同 space 的工具不同，它们的会话
本来就不能共用。所以按 space 绑定不是界面偏好，是引擎语义的直接结果。会话与 space 的绑定是 Harnix
自己的账本数据（引擎不知道 space），与工作区同类。

### 8.4 默认实例

space 的声明里写 `instance = "system"`（默认）、一台固定实例的 id，或者 `"template:<id>"`（第一次
在这个 space 里建会话时按模板建一台，之后复用；账本记下这台实例归这个 space）。这就是「一个 space
要一台独立隔离的 Agent」的写法。space 的前端、后端可以在本 space 能用的实例里显式选。

### 8.5 前台 space 与它的会话：对应关系不能错

一个 space 的会话落在一台或几台实例上，三者的对应是 Harnix 账本里的事实：
`会话 (instance, session) → space`，创建时写下，之后不变。**哪个 space 在前台，中区、输入框、
右栏看到和操作的就只能是这个 space 自己的会话。** 这件事错一次就是把消息发进了别的产品面、
别的实例、别的工具集，所以它是不变量，不是界面上的约定：

1. **前端没有全局的「当前会话 id」。** `@harnix/core/state` 里记的是「每个 space 的当前会话」：
   `space → (instance, session)`。中区、输入框、右栏都从「前台 space」取它自己的那一项；切换 space
   就是切换取哪一项，不会把上一个 space 的会话带过来。
2. **会话总是带着实例一起出现。** 草稿、滚动位置、权限卡、队列这些按会话保存的状态，键都是
   `(instance, session)`，不只用 `session`。两台实例里的 id 撞上也不会串。
3. **服务端也核对，不只靠界面。** `harnix.attach`、`harnix.send`、`session.*` 带上调用方声称的 `space`。
   中枢对着账本核对：会话不属于这个 space 就拒绝（`SPACE_MISMATCH`），错误里写明它属于哪个 space。
   实例也一样：会话不在声称的实例上，就拒绝。**系统 space 是唯一的例外**（D17），它可以观看、操作
   任何 space 的会话；每一次跨 space 的操作都写审计。**这一条是防误，不是安全边界**（§14）。
4. **新建会话的去向只看前台 space**：落在这个 space 的默认实例上（或它显式选的、本 space 能用的
   实例），自动绑定到这个 space。不存在「先建再挪」。
5. **切回一个 space 时恢复的是它自己记住的那一项**。那个会话已经被删了、实例停了，就退到这个 space
   的会话列表，**不退到别的 space 的会话**。
6. **系统 space 里打开别的 space 的会话，是「观看」，不改变那个会话的归属**，也不改写那个 space
   记住的当前会话。从系统 space 点「到所属 space 打开」，是切换到所属 space，再在那里打开。

### 8.6 内建 space

| space | 左栏条目 | 右栏 | 中区 | 空应用里有没有 |
|---|---|---|---|---|
| 会话 | 工作区 | 详情抽屉（今天的详情页） | 对话 | 有 |
| HOME | — | 无 | 总览（`home.card`），对话最小化 | 由应用决定 |
| **系统** | **实例**（这是系统 space 的条目） | 实例树与检查器（§9） | 选中会话的对话 | 有，profile 可关 |

---

### 8.7 M5 的实际做法（2026-10-26）

- **账本**：`.harnix/spaces.json` 记 `instance/session → space`（会话 space 不记，没有记录就是它）和
  `space → 按模板建的实例`。实例拥有者多了 `space:<id>`：界面建的实例归发起时的前台 space，模板默认
  实例归它的 space；旧账本里的 `ui` 读作 `space:sessions`。
- **核对**：带 `space` 的会话调用（attach、send、history、delete、fork……）对着账本核，`SPACE_MISMATCH`
  的 `data` 里写着它属于哪个 space；错误帧现在原样带 `data`。不带 `space` 的调用不核（API 客户端、
  不知道 space 的模块）。只有界面与 Agent 的 space 不用在后端声明，取默认。`[system_space]
  enabled = false` 时没人能声称系统 space。
- **审计**：系统 space 跨 space 的每次操作记一条（`harnix.audit` 能读到；会话在 `system` 实例上的也
  写进它的 transcript）。
- **`@session`**：Agix 没有会话引用；`harnix.send` 的 `references` 由中枢把被引用会话说过的话作为
  `text` 附件交给这一轮。复制的是话，不是会话。
- **界面风格**：照 Helm 的 GUI demo（`docs/design/11-gui-demo`）：纸色中性色、赭红强调色、读的东西
  用衬线；左栏是新对话、space 列表、前台 space 的会话（搜索、排序），底部是设置与连接；空会话是
  衬线问候加居中的输入框与建议；总览 space（HOME）是仪表盘：问候、问一句、卡片网格。
- **应用的界面模块**：`virtual:harnix-modules` 由 `HARNIX_UI_MODULES` 生成，`HARNIX_UI_OUT` 放产物，
  `HARNIX_UI_DIST` 让二进制嵌进它。`examples/hello-app` 按这个流程构建，界面套件对着它跑。

## 9. 系统 space

### 9.1 用途与边界

开发调测用的基础面：**按四层（§2）看到这个 Harnix 管着的一切，并且能操作。** 它是内建模块，
只用公开 API（§6.3 的主题与方法、`ModuleContext`），没有私有通道。这也让它成了「space 模型真能
承载复杂面板」的第一个内建证明。

录像里有完整的系统提示词，敏感级别和 transcript 相同（概念 §6.4），所以：

- 只能经授权过的方法读；录像目录永不经 HTTP 暴露（不变）。
- profile 里 `[system_space] enabled = true | false`。空应用默认开，产品可以关；关掉时对应的方法
  也一起从授权表撤掉，不只是不画。
- 可以观看并操作任何 space 的会话（D17）：画的时候标出它属于哪个 space，给「到所属 space 打开」
  的入口；每次操作写审计。它自己新建的会话绑在系统 space 上。

### 9.2 布局：一棵树，逐层往下看

```text
┌────────────┬─────────────────────────────┬────────────────────────────────┐
│ 系统        │ 顶栏：ci-runner › 会话 a      │ 会话 a          [概况][调用][上下文]│
│ ──────     │                             │                                │
│ ● system   │ 会话 a 的对话                 │ 任务                             │
│   进程内    │（与会话 space 同一个          │  ▸ 子 Agent explore   运行 1m12s │
│   12 会话   │   ChatPanel，可以发送、中断）   │  ▸ 后台 Bash npm run dev  运行   │
│ ● helm-l1  │                             │  ▸ 定时任务 0 9 * * *  等待        │
│   进程内    │                             │ 调用                             │
│ ◐ ci-runner│                             │  #12 step 3  12.4k tok  1.8s    │
│   daemon   │                             │   ├ 系统块 7（来源、tokens）       │
│   降级      │                             │   ├ 工具目录 23                   │
│            │ 输入框                        │   └ 消息 0…41                    │
└────────────┴─────────────────────────────┴────────────────────────────────┘
```

| 层 | 在哪 | 显示什么 |
|---|---|---|
| 实例 | 左栏，系统 space 的条目 | 状态点（Ready / Degraded / Down）、载体、会话数 |
| 会话 | 右栏的会话列表（选中实例时） | 状态、场景、项目根、所属 space、任务数 |
| 任务 | 右栏，展开会话之后 | 种类、状态、时长、用量；子 Agent 能点进去看它的侧链 |
| 调用与上下文 | 右栏，选中会话之后的检查器页签 | §9.3 |

中区始终是选中会话的对话，和别的 space 用同一个组件。

### 9.3 每一层看什么、能做什么

| 层 | 看什么 | 数据从哪来 | 能做什么 | Agix 现在有没有 |
|---|---|---|---|---|
| 实例 | 载体、模板、拥有者、所属 space、版本与协议版本、运行时长、会话数、自检、provider 路由解析、MCP 连接 | `harnix.instances`；`daemon.status`、`daemon.doctor`、`mcp.status` | 按模板新建、停止、重启、重新 attach | ✅（操作是 Harnix 自己的） |
| 会话 | 顶层会话，内存里的和冷的；状态、场景、项目根、压缩次数 | `harnix.instances.sessions`（底下是 `session.list`）、Harnix 账本 | 压缩、清空、从第 N 条分叉（相当于回退）、换模型、中断、改名、关闭、删除 | ✅ |
| 任务 | 子 Agent（前台、后台）、团队成员、执行中的 Shell 与监视器、定时任务；状态、时长、细节 | `session.tasks`、`task_started` / `task_finished` 帧 | 停止（`session.stopTask`）；子 Agent 能点进去看侧链（`session.list {parent_session_id}`、`session.history`） | ✅（0.2.7） |
| 调用 | 选中会话每次模型调用发出去的系统块、工具目录、消息区间、配置、用量与耗时；相邻两次的差异 | Harnix 读录像（请求信封）、`session.history`、`step_start` / `step_end` 帧 | — | ✅（本机，D15） |
| 上下文 | 下一次调用**将要**带的上下文：每块的块名、来源、tokens 与正文，工具表，消息数，合计与压缩阈值 | `session.inspectContext` | 删除用户消息或工具结果、改写用户消息（`session.dropMessages` / `editMessage`） | ✅（0.2.7）；钉住消息没有 |

**Agix 没给的，不画假的。** 页签或按钮在 Agix 补上之前不出现，而不是做成灰的或者在 Harnix 侧
模拟。这是概念 §9「不在 Harnix 侧绕过去」的同一条规矩。

### 9.4 M6 的实际做法（2026-10-26）

- **后端分两半。** 设备本来调不到的引擎方法（`inspectContext`、`dropMessages`、`editMessage`、
  `stopTask`、`compact`、`clear`、`setModel`、`rename`）和读录像，包成内建模块 `system` 的
  `harnix.system.*`：设备调它，它以 `system` 模块的身份经授权调引擎，两次判定都进审计。其余
  （`session.history`、`fork`、`interrupt`、`close`、`delete`，`harnix.instances.*`）设备直接调，
  带上 `space: "system"`：中枢的 space 核对放行并记一笔跨 space 审计；实例的新建、停止、重启
  列为必记的审计项。
- **开关。** `[system_space] enabled = false` 时 `system` 模块不注册（方法连同它的授权一起没有），
  `system` space 不存在：`harnix.sessions`、`harnix.attach`、`harnix.instances.*` 声称来自它的
  一律 `SPACE_MISMATCH`。
- **重启**（`harnix.instances.restart`）：模板建的按账本那一条重建，profile 声明的按声明重建，
  attach 的就是重新连上（「重新 attach」不另设方法）；`system` 实例是这个进程本身，不能重启。
  重建之后照断线重连那样 resync 订阅了它的会话。
- **系统 space 里可以按任何模板建实例**（设备在别的 space 只能用 `ui = true` 的），建出来的归
  `space:system`。它自己新建的会话绑在 `system` 上，不进会话 space 的列表。
- **会话列表的刷新**：引擎改名、压缩、清空、改写或去掉消息时不发事件，中枢在这几个方法成功之后
  重发那台实例的 `harnix.instances.sessions`；分叉同样。
- **界面**：左栏是实例树（选中一台展开它的会话，标出所属 space），中区是选中会话的对话，右栏四个
  页签——会话（事实、到所属 space 打开、操作、任务）、调用（信封变化的那几次与相邻差异）、上下文
  （下一次调用的系统块、工具、合计；最近 50 条消息的改写与去掉）、实例（事实、重启、停止、按模板
  新建）。只有 `harnix.reachable` 里有 `harnix.system.context` 时才注册这个 space。

---

## 10. 前端

### 10.1 技术栈与硬规矩

React 19 + TypeScript + Vite + Zustand。markdown 与代码高亮沿用 IPDrun 对话组件的做法；长列表
（调用、时间线、任务）用虚拟列表。

照旧保留：「只用令牌不写死颜色」「只用 key 不写死文案」两条静态检查；CSP 不放宽
（`script-src 'self'`，不要 `unsafe-eval`）；产物自包含、无 CDN；README 里「no npm、no build step」
的说法随之改掉。

### 10.2 迁移方式

不是「一次性重写后再上线」。M4 先在 `ui/packages/` 里把新外壳立起来，用 Playwright 的**对等用例**
逐条对齐：现有 `tests/browser` 的全部场景；功能对等清单（流式 markdown、工具卡、权限卡、子 Agent、
发送排队、`/` 命令补全、`@` 文件引用、图片粘贴、请求信封查看、设置各节、中英双语）；IPDrun 对话组件
的几项（按会话保留草稿、断线禁发并保留草稿、steer 送达的显示、被中断工具的标记、`data-testid`
约定）。全部通过的那一次提交里删掉旧的 `ui/` 原生模块。

**M4 的实际做法**（2026-10-26）：

- 对话主干是 IPDrun nest-ui 的 `ChatPanel`，外壳不再画流；外壳加的东西走同一套贡献点：工具行上的
  「在详情里打开」（`tool.row`）、请求信封的变化（引擎不产生、界面自己放进流的 `custom` 块，由
  `flow.block` 认领）、两种详情页（`details.panel`）、工作区分组（`sidebar.group`）、引擎的命令
  （`command`）、八个设置页（`settings.section`）。
- 附件真的能传：输入框的 ＋、粘贴、拖放经 `harnix.upload.begin` 上传；图片预览用 `data:`（CSP 不放
  `blob:`）。
- 贡献点 API 2 的模块加载在 M8；在那之前运行期包里照 v1 写的界面模块不导入，诊断页逐条写明理由，
  包里的其余东西照常（`tests/package-e2e.mjs` 守着）。
- `window.harnix = { app, client, host }`：给控制台和从页面里驱动的测试。同源脚本本来就拿得到令牌，
  这里不多给任何能力。
- 对等用例在 `tests/live/ui/`，两种拓扑各跑一遍（后端只给被测的那一种，页面自己回退过去）；旧的
  node 冒烟换成各包的 Vitest：会话状态（core）、组件、注册表与词典（react）、信封与「每个点都被读」
  （shell）。

### 10.3 贡献点 v2

| 贡献点 | 状态 | 贡献什么 |
|---|---|---|
| `space` | **新** | 左栏入口与条目、右栏页签、中区模式、会话策略与新建默认值、默认实例 |
| `home.card` | **新** | 总览页上的一张卡（通常每个 space 贡献一张摘要卡） |
| `tool.row` / `flow.block` | 保留 | 改为注册 React 组件 |
| `details.panel` | 保留 | 变成右栏里的详情抽屉 |
| `sidebar.group` | 保留 | 作用域收窄到「会话」space 内部 |
| `command` / `settings.section` | 保留 | 改为注册 React 组件；可以声明只在某个 space 里出现 |

6 → 8。两个新点都由内建在用（会话 space、HOME），频率都是每次交互一次，没有点跟着流式分片求值。
目录照旧由代码生成。

### 10.4 宿主 API v2（`useNest()`）

- `call(method, params)`：按 `harnix.reachable` 过滤，包含 `app.<id>.*`。
- `subscribe(topic, params?)`：宿主事件流，包含模块主题与带参数订阅。
- **会话**：`session.current()`、`session.context()`、`session.list()`（本 space）、`session.open(ref)`、
  `session.create({ scene?, title?, seed?, instance? })`（自动绑定前台 space）。参数里的会话都是
  `(instance, session)`；宿主 API 自动带上前台 space，服务端据此核对（§8.5）。
- **实例**：`instances.list()`、`instances.sessions(instance)`、`instances.tasks(instance, session)`、
  `instances.create(template, params)`、`instances.stop(id)`：按 `harnix.reachable` 过滤。
- `composer.insert(text)`。
- **联动**：`reveal(panelId, subject)`：在消息里点一张卡片，右栏切到对应页签并打开详情。
- `space.fetch()` / `space.socket()`：模块自有链接（§7.7）。
- `t()`、令牌、图标集、信封读取：不变。
- 照旧**拿不到 Harnix 自己的连接**，所以同一份代码在任何拓扑下都能跑。

### 10.5 运行期包的界面模块、品牌

- `[[ui]] api_version = 2`：模块是**预编译好的 ES 模块**（JSX 已经编译掉），不带自己的 React
  （两份运行时会让 hooks 失效）。外壳经 import map 暴露 `react`、`react-dom`、`zustand`、`@harnix/react`。
  **React 的大版本因此是贡献点 API 的一部分**，升 React 大版本就是升 `api_version`。
- **import map 只能内联**，而 CSP 是 `script-src 'self'`。办法是 import map 在构建时生成，把它的
  `'sha256-…'` 写进 CSP，不算放宽，也不需要 `unsafe-inline`、`unsafe-eval`。
- 照 v1 写的包按概念 §5.1.1 报「这一条不生效」，不拒绝整个包。M4 换掉旧外壳之后、M8 之前，v1 包
  都处在这个状态。
- profile 的 `[ui] runtime_modules = true | false`：关掉时运行期包的 `[[ui]]` 段一律不加载（§14）。
- profile 加 `[brand]`：产品名、品牌图标、产品 id（决定 `.harnix/` 账本命名空间与 token 文件位置）。
  模块可以贡献令牌覆盖（只许覆盖已有令牌名）和语言包（key 带模块命名空间）。应用不 fork `ui/` 就能
  拥有自己的外观。

**M8 开工前定下的（维护者 2026-10-26）**：

1. **目录**：程序自己的配置与状态（现在 `<data-dir>/.harnix` 里的全部）挪到 `~/.<product_id>`，不配
   `[brand]` 就是 `~/.harnix`；`HARNIX_STATE_DIR` 照旧能挪。Documents 里只放项目，不放隐藏目录。旧的
   `~/Documents/.harnix` 不迁移、不读。项目里，项目自己的数据与配置一律放 `<项目>/.agix/`。
2. **import map 只暴露** `react`、`react-dom`、`react/jsx-runtime`、`@harnix/react`；`zustand` 不放（外壳
   不用它，要的包自己打进去）。外壳自己也用这几份，页面上只有一个 React。
3. **令牌覆盖与语言包**：注册表上的 `r.tokens({...})`、`r.strings(locale, {...})`，编进来的模块和运行期
   v2 包都能用；不算贡献点，贡献点还是 8 个。
4. **CSP 的 hash**：服务端启动时从要出的 `index.html` 算 import map 的 `sha256`，而不是构建时写死，
   这样 `--ui-dir` 出的界面也对。

### 10.6 预算

| 指标 | 0.3 | 0.4 |
|---|---|---|
| 界面产物（gzip） | ≤ 300 KB | ≤ 300 KB 不变（React + ReactDOM + zustand 约 50 KB） |
| 后端二进制 | ≤ 48 MB | ≤ 48 MB 不变（`harnix daemon` 不增加依赖；`ts-rs` 只在 `ts` feature 下） |
| 空载常驻内存 | ≤ 80 MB | ≤ 80 MB，只算 `system` 一台实例 |
| 每多一台进程内实例 | — | ≤ 4 MB（空载）。M3 实测（2026-10-26，release，macOS）：只有 `system` 时 14.5 MB，多 8 台空载的进程内实例 15.5 MB，每台约 0.1 MB，8 台一共多 6 个线程；实例里的会话照「每个空闲会话」那一项另算 |

---

## 11. 构建与仓库布局

### 11.1 两种布局

```text
单独的 Harnix（开发 Harnix、发布空应用）       应用（Helm、IPDrun）
Harnix/                                      <app>/
├── Cargo.toml   workspace 根               ├── Cargo.toml        workspace 根，唯一的锁文件
├── Cargo.lock                              ├── Cargo.lock
├── agix/        子仓 → Agix             ├── agix/             子仓 → Agix（应用自己钉）
├── crates/                                 ├── harnix/             子仓 → Harnix（不初始化它的 agix/）
└── ui/packages/ pnpm 包（workspace 在根）    ├── app/              bin crate：App::builder()…
                                            ├── spaces/<x>/{backend,agent,ui}
                                            ├── pnpm-workspace.yaml   harnix/ui/packages/*、spaces/*/ui
                                            └── profiles/<app>.toml
```

应用里 `agix/` 和 `harnix/` 平级。应用发现 Agix 的问题，直接在自己的 `agix/` 子仓里开分支、回推
Agix，不用先改 Harnix、再让 Harnix 升 Agix、再升 Harnix。这是 D3 的本意。

### 11.2 Cargo 怎么做到「Agix 的位置可指定」

几种做法在 2026-10-25 用一个最小样例实测过：

| 做法 | 结果 |
|---|---|
| Harnix 把 Agix 声明成 git 依赖，应用用 `[patch."<Agix 的 URL>"]` 指到 `agix/` | ✗ 打了补丁的 git 源**仍然要去拉原仓库**：离线构建失败；还没推上去的 Agix 提交也构建不了 |
| `[patch]` 指路径依赖、`.cargo/config.toml` 的 `paths` 覆盖 | ✗ 都不作用于路径依赖，Cargo 仍然去读 `harnix/agix/…` |
| Harnix 的 workspace `exclude = ["core"]`，Agix 的 crate 按 Agix **自己的** `Cargo.toml` 解析继承 | ✓ |
| 应用的 workspace 把 `harnix/crates/*` 收成**成员**，由应用的 `[workspace.dependencies]` 指向平级的 `agix/` | ✓ |

于是定成：

1. **Harnix 不再抄 Agix 的依赖表。** Harnix 的 workspace 加 `exclude = ["core"]`，Agix 的 crate
   按 Agix 自己的 `Cargo.toml` 解析 `workspace = true`。今天根 `Cargo.toml` 里「mirrors
   Agix's table」那一大段删掉，只留 Agix 的入口 crate（`daemon`、`base`……）的路径和 feature 选择。
2. **应用的 workspace 收 Harnix 的 crate 为成员，不排除 `harnix/`。** 排除 `harnix/` 时 Harnix 的 crate 只会
   按 `harnix/Cargo.toml` 解析，Agix 就只能在 `harnix/agix`。
3. **应用的 `[workspace.dependencies]` 里 Harnix 那一段是生成的。** Harnix 的表是唯一的真相（第 1 条
   之后它只剩 Harnix 自己的 crate、Agix 的入口 crate、Harnix 直接用的第三方依赖，二十来行）。
   `harnix/scripts/sync-app-deps` 把它重写成以应用根为基准的路径，写进应用 `Cargo.toml` 里一段带
   标记的区块；检查脚本对比两边，漂移就失败。这是「生成并检查」，不是手抄。
4. **Harnix 的 crate 不再从 workspace 继承 `version`、`license`、`edition`、`rust-version`。** 成为
   应用 workspace 的成员以后，继承拿到的是应用的值（IPDrun 的 N-004：拿到了 AGPL 许可）。改成每个
   crate 显式写；`scripts/set-version` 统一改版本号，检查脚本断言所有 Harnix crate 的版本一致。**这会
   改动 CLAUDE.md 的发布步骤第 2 条**，M0 一起改。
5. `[profile.release]`、`[patch]` 只在 workspace 根生效。Harnix 根目录不放应用必须继承的 `[patch]`；
   release profile 写进应用模板。

### 11.3 Agix 的版本：两个钉，一条规则

单独的 Harnix 里，`agix/` 子仓的指针就是 Harnix 验证过的 Agix 版本（D8）。应用里 `agix/` 由应用自己钉，
可能和 `harnix/` 里记的那个不同。

规则（`harnix/scripts/check-pins`，进应用模板的 CI）：

| 应用的 `agix/` 对比 Harnix 钉的版本 | 结果 |
|---|---|
| 相同 | 通过 |
| 更新，并且在 Agix 上存在 | **开发分支上告警**：Harnix 只在它钉的版本上验证过。应用发布前要么回到相同，要么先让 Harnix 升到这一版 |
| 不在 Agix 上（只在本地） | 失败。和「`harnix/` 指针只能指向 Harnix 上存在的提交」同一条纪律 |

`check-lock`：对比应用锁文件与 Harnix 锁文件里共有依赖的版本，有漂移就报出来。

### 11.4 前端与一条命令

- 外壳和 space 的界面是同一个 pnpm workspace 里的包。构建脚本按应用的模块清单生成虚拟模块
  `virtual:harnix-modules`，Vite 一次打包成一份产物，顺序与 `main.rs` 里的模块顺序一致。
- 产物嵌进二进制；`--ui-dir` 与 `harnix ui export` 保留。
- `scripts/build.sh`（模板里也有一份）：`pnpm build` → `cargo build`。没有界面产物就直接 `cargo build`
  会**明确失败并告诉你该跑什么**，不悄悄嵌一份空界面；`--headless` 构建可以不要界面产物。
- `--version` 打印应用、Harnix、Agix、模块 API、贡献点 API 的版本。
- `scripts/check.sh` 各层继续可用，模板里带一份裁剪版。前端不依赖 Agix 的位置。

### 11.5 公共开发流程

- 公共改动**直接在应用的 `harnix/` 子仓里开分支** → 回推 Harnix → 合并 → 打 tag → 应用 bump 子仓。
  代码始终只有一份。
- 应用里不许有长期补丁：子仓指针只能指向上游存在的提交（`check-pins`）。
- 应用独有的东西（领域服务、产品面板、品牌）永远不进 Harnix；介于两者之间的按 §0.2 判。

---

## 12. 桌面：Tauri 只是外壳

- Harnix 提供通用的 `harnix-desktop`（Tauri 2）模板，应用只换品牌。
- **webview 直接加载服务端的 HTTP 地址**，界面由服务端发出（界面和后端一起编译，不会对不上）；
  所有业务走 HTTP/WS。
- 两种模式：**本地**：Tauri 以子进程启动应用二进制，监听回环端口，就绪后加载
  `http://127.0.0.1:<port>/`；**远程**：加载配置好的 URL，走 TLS + 设备配对。
- Tauri 能力只开托盘、窗口、通知、更新、单实例、日志。**没有业务 IPC 命令，不用自定义协议
  发界面，不用本地 socket**。
- 本地模式的 token 交接走一次性启动链接（N-301），不经 IPC；同源，所以 Origin 校验不变。
- 连不上服务端时只显示壳里打包的「连接失败」页，壳里不放业务界面。

---

## 13. 测试：LIVE 框架

### 13.1 原则

- **功能测试都对着真的 `harnix` 二进制跑。** 不用假 DOM，不 mock 中枢，不用假后端。概念 §8 记过一次
  教训：假后端还返回旧形状，界面测试全绿而功能是坏的。
- 后端走 API（协议层直接调方法、订阅事件），前端走 Playwright（真浏览器）。
- **只用一个运行器：Playwright Test**。API 套件是不开浏览器的 project，界面套件是 chromium project，
  同一份报告、同一套夹具。
- `@harnix/core` 用 Vitest 做单元测试；它们是单元测试，**不算功能覆盖**。

### 13.2 结构

```text
ui/packages/testkit/          @harnix/testkit
  launcher.ts                 起一个 harnix：独立的 HOME、数据根、引擎根、端口、拓扑、录像目录；
                              可选再起几台 `harnix daemon`
  api.ts                      基于 @harnix/core/client 的测试客户端（被测的客户端本身就在用）
  fixtures.ts                 Playwright 夹具：harnix 进程、已连接的 api、page
  components/                 组件对象（照 data-testid 约定）
tests/live/
  api/<功能模块>.spec.ts       每个功能模块一份
  ui/<功能模块>.spec.ts
  fixtures/                   模型录像（按套件分目录）
  coverage.md                 功能覆盖表（生成）
```

### 13.3 模型

| 模式 | 用途 | 怎么跑 |
|---|---|---|
| **回放（默认）** | 确定、离线、不花钱 | 引擎的录像按位置重放；每个套件独立的数据根，全局记忆和 telemetry 不会串进来（修 N-003） |
| **live** | 刷新录像、冒烟 | `--live`：连 `.env` 里配置的真实模型服务；跑完可以写回录像 |

隔离之后回放仍然错位的话（同一轮里可选的旁路调用，比如记忆召回，有时有有时没有），就向 Agix
提「回放按调用种类匹配而不是按位置」，不在 Harnix 侧绕过去。

### 13.4 覆盖是可检查的

每个用例带标签 `@feature:<功能模块>`、`@layer:api|ui`。`coverage.md` 由标签生成，列出 §4.3 每一行
在两层各有多少用例。**一行在某一层是零，要么补用例，要么在表里写明为什么这一层不适用**，否则
`scripts/check.sh` 失败。这是概念 §2.5「每一条承诺都可检查」用在测试上。

`scripts/check.sh` 照旧：按从便宜到贵的顺序分层，跑不了的层要报出来，不算通过。

---

## 14. 哪些约束是强制的，哪些不是

概念 §1.2 第 3 条：每一处约束要么是强制的，要么被明说成不是强制的。0.4 新增的约束逐条标明：

| 约束 | 种类 | 防什么 | 不防什么 |
|---|---|---|---|
| 授权表：主体 × 方法 / 路由，默认拒绝 | **安全边界** | 未授权的主体调方法、访问模块路由 | — |
| 界面只能按 `ui = true` 的模板建实例 | **安全边界** | 设备装载子进程、改写模型端点、指定任意数据目录 | — |
| daemon 的 token 不出 Harnix；透传白名单 | **安全边界** | 设备经 Harnix 拿到 daemon 的完全信任 | 本机上能读 Harnix 进程内存或环境的人 |
| 系统 space 关掉时方法从授权表撤掉 | **安全边界** | 产品里有人读录像、跨 space 操作 | — |
| 模块拿不到 `EngineLink` 和中枢内部状态 | **结构约束**（编译期可见性） | 模块不小心绕过中枢 | 模块是内核级的源码，真要绕可以 unsafe 或改 Harnix；这一条防的是误用 |
| `SPACE_MISMATCH`：会话与声称的 space、实例对不上就拒绝 | **防误** | 界面或模块的 bug 把消息发进别的 space、别的实例 | 同页的代码：space 是调用方自己声称的，任何 `Device` 代码都能声称任何 space，包括 `system` |
| 「只有 `Device` 能回答」：权限提问，以及应用自己做的批准 | **安全边界**，只对后端成立 | 引擎、模块、内核替人回答 | **区分不了人和同页的运行期界面模块**：`[[ui]]` 与页面同级（概念 §4.6），装进来的包能替人点。需要「必须是人点的」的产品（例如交易），在 profile 里设 `[ui] runtime_modules = false` |

---

## 15. 不在 0.4 做的

| 不做 | 说明 |
|---|---|
| 运行期后端模块 | 后续方向：参照 Agent 侧，WASM / QuickJS / 原生 Rust 三种载体，届时运行期 space 也能有后端 |
| dylib / ABI 稳定的动态 Rust 模块 | D2 |
| 运行期包新增 space、往别的 space 加页签 | D28。`extensible`、`allow_runtime_spaces` 随它一起做 |
| 批准原语进内核 | D26。两个应用各自实现之后，形状一致再收 |
| 角色令牌、MCP 服务端 | D10、D27，一起做 |
| 迁移的备份钩子、`harnix migrate --dry-run` | D29 |
| 跨机器的 daemon 实例 | D15 |
| 几个宿主共用一台 daemon | D25 |
| sidecar 进程监管（非 Rust 服务） | 走 Agix 的 MCP（运维在 profile 里配置），或经模块路由转发 |
| 引擎可替换 | D1 |
| 通道「追加模式」 | §7.5，有真实需求再开 issue |
| Tauri 私有通道 | D5 |
| 多租户 | 概念 §10，不变 |

---

## 16. IPDrun 的需求落在哪里

IPDrun 在旧架构上的改动**不合并**，作为需求和参考实现，在新架构里按下表重新实现。里程碑编号见
[roadmap](roadmap.md)。

| IPDrun | 内容 | 落在 |
|---|---|---|
| N-001、N-007 | 装配调 `assemble` | M0 |
| N-002 | 会话列表的 `project_root` 以引擎为准，删掉中枢自记的那份 | M0 |
| N-003、R-08 | 回放隔离 | M0、M1 |
| N-004 | 包元数据不从 workspace 继承 | M0（§11.2 第 4 条），从 vendor 专用变成正式做法 |
| N-005 | Agix 作为平级的路径依赖 | §11 的应用布局（D3） |
| N-101、N-102、N-103、N-108 | 无头会话、tap、`OnBusy`、steer | M3（§6.1）；tap 改成有背压的语义 |
| N-104 | `EngineConfig.cwd` | M0 |
| N-105、N-106、N-120、N-121 | 中枢修正 | M0（§6.2） |
| N-107 | 按会话指定 provider、`output_schema` | M3 的 `SessionSpec` |
| N-301、N-302、N-303、N-601、N-602 | 启动链接、令牌文件权限、连接日志、Origin 白名单、Secure cookie | M2 |
| R-01 | 应用注册宿主方法 | M2（`app.<id>.*`） |
| R-02、R-03 | 无头会话、会话规格 | M3 |
| R-04 | 角色令牌 | **0.4 之后**，和 MCP 一起（D27） |
| R-05 | MCP 服务端 | 0.4 之后（D10）；在那之前 IPDrun 在自己的模块里起自己的监听（§7.8） |
| R-06 | React 界面 | M1（`@harnix/core/client`）、M4（对话主干，D21）、M5 |
| R-07 | Tauri 外壳 | M9 |
| R-09 | 可测性约定 | M1（testkit 的组件对象）、M4 |
| R-10 | 装配库化 | M2（`App::builder()`） |
| R-11、R-12 | 会话运行状态推给所有客户端、hello 里的超时 | M3（`harnix.sessions.status`） |
| R-13 | content-type | M2 |
| 待定栏 | sqlite 运行态、git 原语、代码扫描、执行流程 | 不进 0.4，留在 IPDrun。迁移框架的需求由 M2 的迁移生命周期覆盖 |

---

## 17. 给 Agix 的 issue

放在 `.local/`，由它们驱动 Agix 开发。Harnix 侧不绕过（D14）。按 §2 的四层组织：

| 文件 | 层 | 内容 | 阻塞 |
|---|---|---|---|
| `agix-issue-01-session-tasks.md` | 任务 | 按会话列出、停止它的任务，任务的生命周期进会话事件流 | **已落地，Agix 0.2.7** |
| `agix-issue-02-context-inspection.md` | 会话 | 「下一次调用将要带的上下文」及每块的来源与 tokens；删除、修改单条消息 | **已落地，Agix 0.2.7**（钉住消息、录像 RPC 推后） |
| `agix-issue-03-instance-isolation.md` | 实例 | 同一进程里多个会话池互不可见，有测试守住 | **已落地，Agix 0.2.7** |
| `agix-issue-04-instance-observability.md` | 实例 | 会话与轮的生命周期事件；协议版本与能力位；丢订阅通知；spawn 的就绪 | **已落地，Agix 0.2.7** |
| `agix-issue-05-resume-recorder.md` | 会话 | 恢复的会话拿不回录制器：`recorder` 不落盘，恢复时又忽略调用带的那份 | **已落地（Agix 0.2.8）**：恢复会话的那次调用带的 `recorder` 生效；`instances-daemon` 的重启用例改为验证这一轮照常回放 |
| `agix-issue-06-extension-entries-rpc.md` | 会话 | 扩展条目只在库里能写，协议里写不进、读不回；整段分叉丢尾部条目 | **已落地（Agix 0.2.8）**：`session.appendEntry`、`session.entries`、`extension_entry` 帧（§7.6） |

每份文件开头写了实际做法与原诉求的出入。

---

## 18. 文档要改的地方

0.4 落地时（M9），概念文档照下表重写，并照概念 §2.2.2 的惯例写清楚**为什么改**：

| 位置 | 改什么 |
|---|---|
| 概念 §1 | 定位：从「浏览器里的 Agix」改为「Agix 应用的公共底座」；三个目标不变；加 §0.2 的判据 |
| 概念 §2.2 | 扩展的来源与信任（§7.1）。记一笔这是第三次改方向；编译期**模块**不是插件，插件这个词只属于 Agix |
| 概念 §3 | 内核图加上实例层；四层模型（§2）；§3.1 装配拆成「实例」和「应用装配」，并换上 §3.2 ① 的修订；§3.2 改成 turn 有两类调用方 |
| 概念 §3.4 | 主体是三类；实例模板是设备建实例的边界 |
| 概念 §5 | React 外壳、space、贡献点 v2、宿主 API v2、系统 space |
| 概念 §6 | 加 §14 那张表 |
| 概念 §8 | 改造的层次：profile → 运行期包 → 换界面 → **编译期模块与应用**；LIVE 测试取代「界面的测试不需要浏览器」那一条 |
| 概念 §9 | 「依赖方式」换上 §3.2 ② 的修订 |
| 概念 §10 | 「运行期改变装配」一行写明修订后的意思；「给插件的 `harnix.*` 方法」限定为运行期包；补上本文 §15 |
| 概念 §11 路线 | 插入 0.4，指向 [roadmap](roadmap.md) |
| `contribution_points.md` | 由 catalog 重新生成（8 个点） |
| `CLAUDE.md` | 发布步骤：版本号改在每个 crate 上（§11.2 第 4 条）；应用布局下 `agix/` 的检出规则（M0） |
| [二开指南](harnix-sdk.md) | 随每个里程碑更新「可用于哪个版本」一栏；M9 的验收是「照它从模板做出一个应用，不问作者」 |
| 新文档 `docs/testing.md` | LIVE 框架怎么用、怎么录像、覆盖表怎么读（M1） |
| README（双语） | 安装、构建、结构、测试各节；改完两份一起重录 `readme-pairing` |
