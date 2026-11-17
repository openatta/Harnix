# 嵌入指南：`harnix-flow`

[English](embed.en.md)

`harnix-flow` 是流程的库：读流程定义（流程包）和运行状态（运行记录），把一个运行一站一站往前推。
它不链 Harnix 的任何 crate，也不链 Agix，整个目录可以拷进别的 workspace 用。Harnix 自己的宿主是扩展
`harnix-ext-flow`（`crates/ext/flow`），它把这个库整个转出（`pub use harnix_flow::*`），再把运行接到 Harnix 的会话、
方法和主题上。

流程是什么、怎么写（站、执行者、出口条件、规则、策略与权限、门禁、钩子……）见[流程指南](guide.zh-CN.md)，
本文只讲宿主怎么用这个库。

## 1. 拿到什么

```text
crates/flow/
  Cargo.toml           每个依赖都写明版本，不从 workspace 继承
  src/lib.rs           FLOW_API_VERSION、DEFINITION_FORMAT、RUN_FORMAT，根上的名字
  src/kernel/          内核：定义、校验、引用、运行记录、引擎、件与权限判定、协议层
  src/store.rs         MemStore、FileStore、Hooked
  src/packs.rs         MemPacks、DirPacks、ProjectPacks
  src/dryrun.rs        试运行：内核放在沙箱里，外面的一切都是假的
  src/wire.rs          库自己的行（DefRow）
  docs/guide.*.md      流程指南（GUIDE_EN、GUIDE_ZH 就是它）
  docs/embed.*.md      本文
  tests/               内核的测试，以及 surface.rs、deps.rs 两个守卫
  examples/demo_seed.rs  用真内核把几个演示流程跑到几种状态，写进一个 FileStore
```

Feature：只有 `ts`（缺省关）——给定义、运行记录这些类型派生 TypeScript 声明（`ts-rs`），宿主自己生成。

### 依赖

| crate | 版本 | 用途 | Harnix 的 workspace 也有 |
|---|---|---|---|
| `serde`（`derive`） | `1` | 定义、运行记录的序列化 | 有 |
| `serde_json` | `1` | 值、输出、历史 | 有 |
| `serde_yaml` | `0.9` | 定义与件的 YAML。上游已不再维护，先沿用 | 无 |
| `schemars` | `1` | 定义、运行记录的 JSON Schema | 无 |
| `jsonschema`（无缺省 feature） | `0.49` | 输出契约、输入的校验 | 无 |
| `async-trait` | `0.1` | 宿主 trait | 有 |
| `sha2` | `0.10` | 件的摘要（钉住） | 有 |
| `uuid`（`v4`） | `1` | 认领的 token、`DirPacks` 写盘时的临时文件名 | 有 |
| `tokio`（`rt`、`sync`） | `1` | `FileStore` 的磁盘操作放到阻塞池；试运行的锁 | 有 |
| `tracing` | `0.1` | `FileStore` 跳过的东西要说出来 | 有 |
| `libc` | `0.2` | macOS 上 `FileStore` 的落盘（`fsync`） | 有 |
| `ts-rs`（可选，feature `ts`） | `12` | TypeScript 声明 | 有 |
| 开发：`tempfile` | `3` | 测试的临时目录 | 有 |
| 开发：`tokio`（`macros`、`test-util`） | `1` | `#[tokio::test]`、暂停的时间 | 有 |
| 开发：`syn`（`full`）、`quote` | `2`、`1` | `tests/surface.rs` 读源码 | 无 |

两边都有的，版本写得一样（Harnix 的 `tests/crate-metadata.mjs` 查）。

## 2. 宿主要提供什么

内核不知道运行存在哪、用户是谁、谁能做什么、门禁怎么跑，这些都是 `kernel::host` 里的 trait，由宿主给：

| trait | 做什么 | 库里现成的 |
|---|---|---|
| `RunStore` | 运行记录与历史的读写；`commit` 必须原子、按 rev 比较 | `MemStore`、`FileStore::new(dir)`；`Hooked` 包一层，测试里改 id、拦提交 |
| `Packs` | 流程包：流程、skill、规则、策略、门禁、钩子，以及目录（`Catalog`：这里认识哪些 scene、程序、门禁、工具……） | `MemPacks`（代码里给）、`DirPacks`（目录分层，`with_builtin`、`with_writable`）、`ProjectPacks`（项目自带的流程包，项目受信才读） |
| `Authorizer` | 谁能做什么（确认、认领、交付、编辑流程……），`Actor` 由宿主解释 | `AllowAll` |
| `Tools` | `tool` 门禁调的只读工具 | `NoTools` |
| `Gates` | `host` 门禁：应用自己的检查，按名字 | — |
| `Programs` | `kind: program` 的工作项：应用代码 | — |
| `Clock` | 现在几点 | `SystemClock`、`FixedClock`、`ManualClock`（测试） |

`Engine::new(store, packs)` 建引擎，`with_authorizer`、`with_tools`、`with_clock`、`with_lease_ms`、`with_retries`、
`with_guarded`、`with_submit_scenes`、`with_main_scene`、`with_workspaces` 配它。每个操作（`create`、`submit`、`confirm`、
`tick`……）读、算、按 rev 提交，成功之后才交回一个 `Outcome { value, events, work, bookkeeping }`：

- `events`：发生了什么（`EVENT_KINDS` 是全部的种类），宿主转给它的订阅者；
- `work`：提交之后要宿主去做的 `WorkItem`——`Task`（会话、子 Agent、程序工作项，已由 `flow:dispatch` 认领，带 token）、
  `Gate`（跑一个 `command` 或 `host` 门禁，结果经 `gate_result` 交回）、`CloseSession`（一个被取消的工作项用过的会话）；
- `bookkeeping`：只是租约往前挪了，运行的摘要没变，宿主不必重发运行列表。

这一版里，派发 `WorkItem::Task`（开会话、发 skill、收交付、按权限答审批）、跑 `command` 门禁、投递钩子都还是宿主的事；
Harnix 的做法在 `crates/ext/flow/src/module/`。`Gates`、`Programs` 由宿主在执行工作项时自己调，不交给 `Engine`。

试运行（`Sandbox::new(packs, config)`）只要流程包：真内核跑在内存里（`MemStore`），会话、程序、门禁、工具一个都不调，结果由调用方模拟（`Mocks`）或取契约的示例。

## 3. 版本与格式

| 号 | 现在 | 在哪 | 什么时候升 |
|---|---|---|---|
| `FLOW_API_VERSION` | `1` | `src/lib.rs` | 库的 Rust 接口有破坏性变化：宿主实现的 trait、它们交出的类型、根上的名字、存储与流程包的构造。`tests/surface.rs` 守着这几张表 |
| `DEFINITION_FORMAT` | `1` | `src/lib.rs` | 流程定义与件的格式语义变了。只加带缺省的字段不升（定义拒收不认识的字段，旧库读到新字段会清楚地报错） |
| `RUN_FORMAT` | `1` | `src/lib.rs` | 运行记录、历史、运行 id 的格式变了 |

两个格式号这一版只声明，还没写进定义或记录；没写的就是格式 1。

承诺：

1. 公开面只做加法；加的东西在下面的版本一节记一条，同时加进 `tests/surface.rs` 的表。
2. 删、改名、改签名才升 `FLOW_API_VERSION`，并在版本一节写清宿主要怎么改。
3. 格式升号时写清哪些字段变了、宿主该怎么办；不提供自动迁移的代码（宿主清掉旧数据，或自己迁）。

## 4. 怎么拷走

1. 把整个 `crates/flow` 目录拷进你的 workspace，列进 `members`。它不读 `[workspace.dependencies]`，也不靠 workspace 的
   `[workspace.package]`，所以不用改 `Cargo.toml`；想用 workspace 里已有的依赖版本，照上表核对一下就行。
2. `cargo test -p harnix-flow`：内核的测试，加上两个守卫——
   - `tests/deps.rs`：在你的 workspace 里读 `cargo metadata`，查它编进来的东西里没有 Harnix、没有 Agix、没有本地路径的 crate，
     `Cargo.toml` 里没有从 workspace 继承的东西；
   - `tests/surface.rs`：解析源码，查公开面和表里写的一样。你改了库，它会告诉你哪一条是破坏、哪一条是要记下来的新增。
3. Harnix 这边的 `scripts/check-flow-copy.sh` 就是这么做的：把目录拷进一个只有它的临时 workspace，不带 `Cargo.lock`，
   编译并跑它的测试。

## 5. 版本与迁移

一节一个 `FLOW_API_VERSION`，新的在上。

### `FLOW_API_VERSION = 1`

库的第一版：内核、存储、流程包、试运行从 `harnix-ext-flow` 搬出来，接口与行为照旧；`DEFINITION_FORMAT = 1`、`RUN_FORMAT = 1`。

迁移：无。从 `harnix-ext-flow` 用过来的宿主，`harnix_ext_flow::X` 仍然可用（它转出了整个库）；直接依赖本库的写
`harnix_flow::X`。
