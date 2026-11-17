# 嵌入指南：`harnix-flow`

[English](embed.en.md)

`harnix-flow` 是流程的库：读流程定义（流程包）和运行状态（运行记录），把一个运行一站一站往前推，并且——带上派工器
（runner）——驱动宿主的会话照流程和 skill 把活干完。它不链 Harnix 的任何 crate，也不链 Agix，整个目录可以拷进别的
workspace 用。Harnix 自己的宿主是扩展 `harnix-ext-flow`（`crates/ext/flow`），它把这个库整个转出（`pub use harnix_flow::*`），
把 Harnix 的会话交给派工器，再把运行接到 Harnix 的方法和主题上。

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
  src/wire.rs          库自己的行（DefRow、defs_list）
  src/runner/          派工器（feature `runner`）：把运行的活放到宿主的会话里做
    host.rs            SessionHost——派工器向宿主要什么——和经过它的东西
    mod.rs             Runner、RunnerConfig、Changed、CatalogPacks
    testing.rs         ScriptedHost：按脚本答话的宿主
    …                  派发、会话任务、轮、主会话、门禁、钩子、worktree、git、过程目录、信任、流程自己的工具
  src/author.rs        编写工具（feature `runner`）：在对话里编写流程、操作运行
  src/conformance.rs   每个 SessionHost 都要做到的事，对着真宿主查
  packs/examples/      一个示例流程包（example-review）：拷走库的宿主用 `MemPacks::examples()` 加载、校验、跑它（内容由库自己
                       `include_str!`）；Harnix 把它编进应用
  docs/guide.*.md      流程指南（GUIDE_EN、GUIDE_ZH 就是它）
  docs/embed.*.md      本文
  tests/               内核的测试；runner.rs（派工器对着 ScriptedHost 从头跑到尾）；conformance.rs；surface.rs、deps.rs
                       两个守卫；golden.rs 和它在 tests/golden/ 下的金样运行
  examples/flowctl.rs  命令行里的库：validate、schema、dry-run、run、show
  examples/demo_seed.rs  用真内核把几个演示流程跑到几种状态，写进一个 FileStore
```

Feature：

| feature | 缺省 | 带来 |
|---|---|---|
| `runner` | 开 | 派工器、`ScriptedHost`、一致性套件（`tokio-util`、`futures`；tokio 的 `time`、`process`、`macros`、`io-util`） |
| `webhooks` | 开 | `Webhooks`，钩子的 HTTP 传输（`reqwest`，rustls）；关掉则宿主给自己的 `HookTransport`，不给就不投递钩子 |
| `ts` | 关 | 给定义、运行记录这些类型派生 TypeScript 声明（`ts-rs`），宿主自己生成 |

`default-features = false` 时这个 crate 只剩内核，和有派工器之前一样。

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
| `libc` | `0.2` | macOS 上 `FileStore` 的落盘（`fsync`）；输出目录名里的本地日期 | 有 |
| `tokio-util`（`rt`；feature `runner`） | `0.7` | 派工器的任务，跟踪、叫停 | 有 |
| `futures`（feature `runner`） | `0.3` | 把会话读成一个帧流 | 有 |
| `reqwest`（rustls，无缺省 feature；feature `webhooks`） | `0.12` | 钩子的 webhook | 有 |
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
| `Authorizer` | 谁能做什么（确认、认领、交付、编辑流程……，以及编写工具读什么），`Actor` 由宿主解释 | `AllowAll` |
| `Tools` | `tool` 门禁调的只读工具 | `NoTools` |
| `Gates` | `host` 门禁：应用自己的检查，按名字 | — |
| `Programs` | `kind: program` 的工作项：应用代码 | — |
| `Clock` | 现在几点 | `SystemClock`、`FixedClock`、`ManualClock`（测试） |

`Engine::new(store, packs)` 建引擎，`with_authorizer`、`with_tools`、`with_clock`、`with_lease_ms`、`with_retries`、
`with_guarded`、`with_submit_scenes`、`with_main_scene`、`with_workspaces`、`with_layout` 配它。每个操作（`create`、`submit`、`confirm`、
`tick`……）读、算、按 rev 提交，成功之后才交回一个 `Outcome { value, events, work, bookkeeping }`：

- `events`：发生了什么（`EVENT_KINDS` 是全部的种类），宿主转给它的订阅者；
- `work`：提交之后要宿主去做的 `WorkItem`——`Task`（会话、子 Agent、程序工作项，已由 `flow:dispatch` 认领，带 token）、
  `Gate`（跑一个 `command` 或 `host` 门禁，结果经 `gate_result` 交回）、`CloseSession`（一个被取消的工作项用过的会话）；
- `bookkeeping`：只是租约往前挪了，运行的摘要没变，宿主不必重发运行列表。

把活做掉——`WorkItem::Task`（开会话、发 skill、收交付、按权限答审批）、`WorkItem::Gate`、钩子、worktree、过程目录——是派工器
的事（`harnix_flow::runner::Runner`，见下）。只用内核的宿主自己做，执行工作项时自己调 `Gates`、`Programs`，不交给 `Engine`。

### 派工器与宿主的会话：`SessionHost`

`Runner::new(engine, host, RunnerConfig)` 建派工器；`start()` 接着做存储里说已派出、还没答复的活，把每个活跃运行的主会话
开起来或重新看住，再去找过期的租约；`stop(grace)` 让它的任务不再等（运行不取消——下次启动会接着做）。引擎每次调用的结果都经
`apply`（结果没点名运行时用 `apply_for(run, ..)`，整个 Result 用 `settle(result)`）：事件广播出去（`events()`），碰到的运行投影进
过程目录并告诉宿主（`changes()` → `Changed { runs, substantive }`，给要发主题的宿主），钩子得到通知，每个工作项起在派工器自己
的任务上。`start_run(input, by)` 照派工器的做法启动一个运行（先问信任、建 worktree、占输出目录）；`end_worktree`、
`ensure_main`、`hook_status`、`agents_of`、`run_of_session`、`set_trust`、`hear` 是宿主的方法会用到的其余几个。

`RunnerConfig` 是宿主提供给会话的和交进来的东西：`profiles`（定义里写的场景名，目录会列出来）、`submit_profiles`（其中用
`flow_submit` 交付的）、`main_profile`、`project_profiles`（其会话需要项目的：没有项目的运行在自己的目录里做）、`guarded`
（`DEFAULT_GUARDED`）、`command_gates`、`lease_ms`、`tick_ms`、`hook_headers`（投递头的前缀；Harnix 传 `X-Harnix`）、`recording`
与 `main_recording`（回放的宿主用）、`programs`、`gates`、`hook_transport`，以及 `trust`（与流程包共用的 `Arc<ProjectTrust>`）。
`CatalogPacks::new(&cfg, templates)` 把宿主的流程包包一层，加上派工器要往目录里添的东西（场景、守住的工具、命令门禁开没开、宿主
自己的模板），并给每个项目一个视图（先 `set_layout`）。

派工器只通过 `SessionHost` 这个 trait（`runner::host`）认识会话：宿主开会话、往里发话、把帧流出来、答它的问句。经过它的一切
都用流程自己的词——会话是宿主在 `open` 里给出的不透明 `SessionId`，派工器记进运行记录：

| 方法 | 宿主做什么 |
|---|---|
| `open(OpenSpec) -> Opened { id, existed }` | 按 spec 开会话，或者答回它的 `key` 已经指着的那个 |
| `lookup(id) -> Option<SessionInfo>` | 记录里记的会话还在不在（只在盘上的拉回来） |
| `watch(id) -> Watch { frames, info }` | 从现在起的帧，以及此刻是否有一轮在跑 |
| `send(id, text)` | 一条消息，排在正在跑的一轮后面 |
| `interrupt(id)`、`close(id)` | |
| `answer(id, ask, Decision)` | 答一个权限问句：`Allow`，或 `Deny { reason }` |
| `note(id, Note)` | 往会话记录里写一张给人看的卡片（`flow_propose` 的提案） |
| `project_trusted(root)` | 项目是否受信（Harnix 问它的引擎） |
| `run_over(run)` | 运行结束了：宿主专为它做的东西可以收回（缺省什么都不做） |

`OpenSpec` 带 `key`（任务自己的会话 `flow:<运行>/<站>/<任务>@<token>`，共享的 `flow:<运行>#<键>`，主会话 `flow:<运行>@main`）、
`run`、`main`、`profile`、主会话的 `title`、`project_root`、`Placement`（模型、provider、`effort`，以及 `instance`/`template`/
`params`/`space`——宿主自己解释的名字，不认识的要拒开，不能悄悄忽略）、`asks`（这些工具模式的每一次调用都必须作为 `Frame::Asked`
送来）、`output_schema`（`Delivery::StructuredOutput` 时）、`delivery`、`shown_in`、`recording`，以及 `tools`（要注册的流程工具，§4）。
它是 `#[non_exhaustive]`：只加字段，不删。

`Frame` 是宿主报告的会话动静：`TurnStarted { turn, text }`、`ToolCall`、`ToolResult`、`Asked { turn, ask, call, tool, paths, agent }`、
`AgentStarted`、`AgentEnded`、`TurnSettled { turn, end: TurnEnd, output, usage }` 和 `Lagged`。`TurnEnd` 是 `Done`、`Interrupted` 或
`Error { retryable, kind, message }`——不会好转的失败（没凭据）立刻让这次尝试失败；可能会好的（限流）当作一轮什么都没交。
`HostError` 是 `Gone`（尝试失败：「the session went away」）、`Refused`（尝试失败并带原因）、`Unavailable`（派工器放弃、不记失败；
租约过期后下次启动恢复）或 `Other`（记日志，当 `Unavailable`）。

**契约**（派工器依赖的；一致性套件查它，§5）：

- 帧按顺序到达：`TurnStarted(t)` 先于 `t` 的一切，`TurnSettled(t)` 最后。宿主落后了可以掉中间的帧，用 `Lagged` 说一声；
  **不能掉 `TurnSettled`**。
- `open` 幂等：同一 `key` 再开答同一个 id，`existed: true`；同一 key 配别的 spec 是 `Refused`。做得到的宿主（Harnix，id 从 key
  推出）跨自己的重启也保持；做不到的至少在进程内保持。
- `send` 排队：正在跑的一轮不被打断，一次 send 起且只起一轮，它的 `TurnStarted.text` 就是发出的文本（派工器靠它在主会话里认出自己的轮）。
- `interrupt` 对空闲会话是 `Ok`；对运行中的一轮，之后必有 `TurnSettled { end: Interrupted }`。`answer` 对轮已结束的问句是 `Ok` 或
  `Gone`。`close` 之后，看着它的流结束，`send` 得 `Gone`。
- 只有权限问句作为 `Asked` 送来；要话的问句（模型的、MCP server 的）是人的。宿主为 spec 的 `asks` 点名的工具发 `Asked`，也为它自己
  拦下的（项目外的写、保留目录里的写）发。
- 派工器只靠不再等来停，从不要求宿主取消。任务结束时轮还在跑的，打断并等它结束（30 秒），会话才给下一个任务。

`ScriptedHost`（`runner::testing`）是按脚本（`Turn::{Ends, Runs, Asks, Errors}`，按站、主会话、共享键）答话的 `SessionHost`：
有了它，派工器不用模型就能把整个流程包从头跑到尾（`tests/runner.rs`、`examples/flowctl.rs`）。

### 它在项目里写在哪：`ProjectLayout`

库没有自己的项目数据目录名，也不知道 worktree 放哪：由宿主给，`ProjectLayout { dir, worktrees_root, branch_prefix, agent_dirs }`
（`Engine::with_layout`；没有缺省值）。Harnix 给的是 `.harnix`、流程扩展数据目录下的 `worktrees`、`harnix/flow`，以及引擎沙箱
允许会话写的 `.harnix/` 子目录 `scratchpad`、`memory`（`with_agent_dirs`）；在 Harnix 上做的带品牌的应用给的是它自己的 `.<product_id>` 与 `<名字>/flow`。

```text
<项目>/<dir>/flows/          项目自己的流程包，随项目提交（ProjectPacks::new 收这个布局；项目受信才读）
<项目>/<dir>/runs/<运行>/    运行的过程目录；`<dir>/.gitignore` 只加一行 `runs/`
<worktrees_root>/<运行>/     运行的 worktree，在所有项目之外，分支 <branch_prefix>/<运行>
```

项目数据目录可以和别人共用——Harnix 的 `.harnix` 也是它的引擎放设置、记忆、引擎自己的 worktree 的地方——所以库在里面只用
`flows/` 和 `runs/`，`.gitignore` 只加自己那一行，worktree 不放进去。

判定一次调用（`judge_under`）时还要给出「在项目里、但算项目外」的目录（`Call::reserved`；`ProjectLayout::reserved(root)`
给的就是项目数据目录）：任务的工具碰不得，除非有一条 `allow` 的 `paths` 指到它里面——写 `${workspace}/**` 的不算。其中宿主
点名为会话自己的子目录（`Call::agent_dirs`；`ProjectLayout::agent_dirs_in(root)`）按项目里的普通路径判。宿主自己往那里写的
东西（编写工具存的流程、过程目录）不经判定。

### 会话是怎么开的

会话工作项的 `WorkItem::Task` 带一个 `Placement`：模型、provider、`effort`（模型推理多用力，取值见 `EFFORTS`；不写随宿主的
设置），以及会话放哪（`instance`、`template`、`params`、`space`——宿主自己解释的名字；不认识的宿主要拒开，不能悄悄忽略）。
宿主照它开会话，再告诉内核开的时候用了什么：`attach_session(.., session_key, opened_with, ..)`、`attach_main(run, session,
opened_with)`。运行把它记在键上（`BoundSession { id, opened_with }`）：同一个键后来的工作项先和它比对，一样才在那个会话里做。
`opened_with` 是宿主自己的 JSON，内核只负责记着。

`CreateIn::shown_in` 是宿主还把运行的主会话展示在哪——宿主起名的地方（Harnix：流程的 space 之外的一个 space）；内核把它记在运行上。

## 3. 存储、流程包，以及在项目里写什么

- **运行**：一个 `RunStore`——`FileStore::new(dir)` 在 `dir` 里放 `<id>.json`、`<id>.history.jsonl`、`ids.json` 和 `FORMAT`；
  `MemStore` 给测试和试运行。
- **流程包**：一个 `Packs`——`DirPacks::new([dirs…])` 从分层目录读 `flows/*.yaml`、`skills/<name>/SKILL.md`、`rules/*.md`、
  `policies/*.yaml`、`gates/*.yaml`、`hooks/*.yaml`（`with_writable` 指定可写的一层，`with_builtin` 垫一个 `MemPacks`，
  `with_catalog` 给宿主认识的名字）；派工器用 `CatalogPacks` 再包一层。
- **没有项目的运行**：`Engine::with_workspaces(dir)`——需要目录的（`project_profiles` 场景上的会话、门禁、输出目录）得到 `<dir>/<运行>`。
- **在项目里**（`ProjectLayout`，见上）：项目自己的流程包在 `<项目>/<dir>/flows/`，受信时才读；每个运行的过程目录
  `<项目>/<dir>/runs/<运行>/`——`task.json`（运行现状，每次变化重写）、`flow.yaml`（启动时钉住的定义与件）、`stations/<站>/`
  里每次交付（`output.json`，或每个任务一个 `<任务>.json`）、命令门禁的 `context.json` 和 `gate-<门禁>.log`；`<dir>/.gitignore`
  只加一行 `runs/`。运行的输出目录（启动给的 `output_dir`，否则流程的，否则 `flows/${flow}-${date}${seq}`）启动时建在项目里
  （worktree 运行建在它的 worktree 里）。
- **worktree**（`workspace: worktree`）：从项目的 `HEAD` 建 `<worktrees_root>/<运行>`，分支 `<branch_prefix>/<运行>`；运行结束后
  `Runner::end_worktree(run, how, by)` 把分支合进项目当前分支（能快进就快进，否则合并提交；冲突是 `EndError::Conflict { files, .. }`，
  项目不动）、保留分支，或丢弃。worktree 里的项目数据目录不提交，只有它的 `flows/` 提交。
- **钩子**：运行的每个事件告诉流程包（和项目的）里想要它的钩子，经 `HookTransport` `POST` 出去，头是 `<前缀>-Event`、`-Delivery`、
  `-Timestamp`，有 `secret_env` 时还有 `-Signature`（`<timestamp>.<body>` 的 HMAC-SHA256）；`Runner::hook_status()` 列出每个钩子和
  它最近一次投递。

## 4. 流程自己的工具，和它答的问句

每个流程会话都有流程自己的三个工具——`flow_state`、`flow_submit {output}`、`flow_propose {kind, to?, reason}`——由宿主注册给
会话（`OpenSpec::tools` 带着它们的 spec；没有派工器时 `runner::tool_specs()` 也给），每次调用转给
`Runner::tools().call(session, tool, input)`，带上会话 id：派工器找到这个会话在做的任务，用文本答（`ToolReply { text, is_error }`）。
Harnix 把它的每个场景包成 `flow-<场景>`，加上这三个工具；走引擎 RPC 的宿主把它们注册为会话的宿主工具，同样答每次调用。

任务的权限问句由派工器来答：宿主开会话时要让 `OpenSpec::asks` 里的工具（每个守住的工具和流程的权限点名的每个工具；
`ask_rules(&asks)` 就是 Harnix 交给引擎的规则表）的每次调用都作为 `Frame::Asked` 送来，派工器按这一轮所做任务的权限经
`SessionHost::answer` 答——`allow` 放行，`deny` 带原因拒绝，`ask` 留给人（运行等 `approval`；宿主照常把问句给人看）。
没有任务拥有的轮（人在和主 Agent 说话）的问句派工器不答。

### 编写工具：`author`

在对话里编写流程、操作运行的是另一组工具，在 `harnix_flow::author`（feature `runner`）：`flow_list`、`flow_show`、
`flow_schema`、`flow_validate`、`flow_create`、`flow_update`、`flow_delete`、`flow_part_set`、`flow_part_delete`、
`flow_dry_run`、`flow_start`、`flow_guide`、`flow_runs`、`flow_run`，以及操作运行的 `flow_pause`、`flow_resume`、`flow_cancel`、
`flow_confirm`、`flow_force`、`flow_rework`、`flow_retry`、`flow_release`（`RUN_TOOLS`）。宿主把 `author::tool_specs()` 注册给
允许编写流程的会话——每个宿主里名字、参数、文字都一样——每次调用转给 `AuthorTools::call(session, tool, input, &by)`；
`AuthorTools::new(&runner, sandbox, host)` 在派工器建好之后建。工具经引擎、派工器（真的启动、项目的信任）和试运行的
`Sandbox` 做事，用文本答（`ToolReply`）。

- **目标**：作用于流程的工具在 `flow` 里给出流程，作用于运行的在 `run` 里给出运行。不给时用宿主的「当前」
  （`AuthorHost::current`、`current_run`）；都没有就拒绝，文字是 `AuthorHost::no_current`。把一个对话绑在一个流程上的宿主
  用 `AuthorHost::pinned` 说明：点名的不是当前流程、运行属于别的流程，都拒绝（Harnix：对话旁边选中的流程）。
- **作用域**：工具动手之前，以 `by`（宿主给的 `Actor`，带着调用的会话）问引擎的 `Authorizer`：`read_flows`（列表、
  schema、校验）、`read_flow`（查看）、`list_runs`、`read_run`（带运行）、`dry_run`、`edit_flow`/`edit_part`（写任何东西之前），
  以及照旧在引擎里问的——启动的 `create`、带运行的各个运行操作。要把流程运行里的会话限制在它自己的运行上，宿主在这里查 `run`。
- **宿主的事**（`AuthorHost`）：对话的当前流程和项目（`current`、`project`；`flow_create` 给了 `project_root` 时 `set_project`）、
  它新建的流程怎么处理（`created`，可以把它设为当前，返回给模型的一句说明），以及 `notice`（流程包变了、删了流程、试运行
  往前走了），宿主拿去告诉自己的人。项目自己的流程在 `ProjectLayout::pack_dir` 里读写，只在项目受信时。
- `flow_guide` 就是 `GUIDE_EN`/`GUIDE_ZH`；`catalog_json`、`part_schemas`、`parts_used` 是这些工具答的形状，宿主自己的方法也可以用。

## 5. 试运行、`flowctl` 与一致性套件

试运行（`Sandbox::new(packs, config)`）只要流程包：真内核跑在内存里（`MemStore`），会话、程序、门禁、工具一个都不调，结果由调用方模拟（`Mocks`）或取契约的示例。

`examples/flowctl.rs` 是命令行里的库，不带任何宿主的东西：

```text
cargo run -p harnix-flow --example flowctl -- validate <pack-dir> [--catalog catalog.json]
cargo run -p harnix-flow --example flowctl -- schema definition|run|tools|policy|gate|hook
cargo run -p harnix-flow --example flowctl -- dry-run <pack-dir> <flow> [--inputs '{}'] [--mocks '{}']
cargo run -p harnix-flow --example flowctl -- run <pack-dir> <flow> --script script.yaml [--state <dir>] [--project <dir>]
cargo run -p harnix-flow --example flowctl -- show <state-dir> <run>
```

`run` 用派工器在一个 `FileStore` 上跑流程，会话按 YAML 脚本（主会话的轮、每站的、每个共享键的）答话，每个事件打一行；运行结束
或等人时停下。拷走库的宿主原样跑它，就是「拷入后能加载示例包、校验、试运行」的验证。

`harnix_flow::conformance::run_all(&Subject)` 是每个 `SessionHost` 都要做到的事，对着真宿主查：一个 key 只开一次；一次 send
恰起一轮并且结束；空闲时 interrupt 是 `Ok`；问句送来并能被答；关掉的会话不在了；note 和信任问句有答复。宿主给出它的
`Subject`——自己、一个场景、一句让会话答完就停的话，以及（能的话）一句让它调一个守住的工具的话——断言没有失败。`ScriptedHost`
用脚本跑它（`tests/conformance.rs`），套件本身因此也被查过；Harnix 对着它的中枢用录像跑。

## 6. 版本与格式

| 号 | 现在 | 在哪 | 什么时候升 |
|---|---|---|---|
| `FLOW_API_VERSION` | `1` | `src/lib.rs` | 库的 Rust 接口有破坏性变化：宿主实现的 trait（含 `SessionHost`）、它们交出与收进的类型、根上的名字、存储与流程包的构造、派工器提供的东西。`tests/surface.rs` 守着这几张表 |
| `DEFINITION_FORMAT` | `1` | `src/lib.rs` | 流程定义与件的格式语义变了。只加带缺省的字段不升（定义拒收不认识的字段，旧库读到新字段会清楚地报错） |
| `RUN_FORMAT` | `1` | `src/lib.rs` | 运行记录、历史、运行 id 的格式变了。`tests/golden.rs` 留着几个运行的记录与历史，跟着一起变 |

写在哪：

- 定义和每个件可以写 `format: N`（不写就是 1，1 也不写回去）。号大于 `DEFINITION_FORMAT` 的整份拒读——校验先说它要哪个格式，
  不去报本库不认识的字段——`0` 或不是数字的不算格式；
- 每条运行记录带 `format`。`FileStore` 在目录里写进第一个运行时一并写 `FORMAT` 文件（`1`）；`FORMAT` 大于 `RUN_FORMAT` 的目录、
  有运行却没有 `FORMAT` 的目录（格式还没记下来时写的）每次调用都拒，并说明原因；格式更大的单条记录单独拒。不会只读一半。

承诺：

1. 公开面只做加法；加的东西在下面的版本一节记一条，同时加进 `tests/surface.rs` 的表。
2. 删、改名、改签名才升 `FLOW_API_VERSION`，并在版本一节写清宿主要怎么改。
3. 同一个格式号里只加带缺省的字段；改名、删字段就升号。升号时写清哪些字段变了、宿主该怎么办，金样运行一起变；不提供自动迁移的代码（宿主清掉旧数据，或自己迁）。

## 7. 怎么拷走

1. 把整个 `crates/flow` 目录拷进你的 workspace，列进 `members`。它不读 `[workspace.dependencies]`，也不靠 workspace 的
   `[workspace.package]`，所以不用改 `Cargo.toml`；想用 workspace 里已有的依赖版本，照上表核对一下就行。
2. `cargo test -p harnix-flow`：内核和派工器的测试，加上两个守卫——
   - `tests/deps.rs`：在你的 workspace 里读 `cargo metadata`，查它编进来的东西里没有 Harnix、没有 Agix、没有本地路径的 crate，
     `Cargo.toml` 里没有从 workspace 继承的东西；
   - `tests/surface.rs`：解析源码，查公开面和表里写的一样。你改了库，它会告诉你哪一条是破坏、哪一条是要记下来的新增。
3. Harnix 这边的 `scripts/check-flow-copy.sh` 就是这么做的：把目录拷进一个只有它的临时 workspace，不带 `Cargo.lock`，
   编译并跑它的测试。

## 8. 版本与迁移

一节一个 `FLOW_API_VERSION`，新的在上。

### `FLOW_API_VERSION = 1`

库的第一版：内核、存储、流程包、试运行从 `harnix-ext-flow` 搬出来；`DEFINITION_FORMAT = 1`、`RUN_FORMAT = 1`。首次发布之前，
这一版里还有：

- **派工器**（`runner`，feature `runner`，缺省开）：`Runner`、`RunnerConfig`、`Changed`、`CatalogPacks`、`FlowTools`/`tool_specs`、
  `DEFAULT_GUARDED`、`withheld`、`worktree_info`、`EndError`、`redact_url`；trait **`SessionHost`** 和 `OpenSpec`、`Opened`、
  `SessionInfo`、`Watch`、`Frame`、`TurnEnd`、`Decision`、`Note`、`HostError`、`ToolSpec`、`SessionId`；`HookTransport` 与
  `Webhooks`（feature `webhooks`）；`ScriptedHost` 与 `Turn`；`conformance::{Subject, Failure, run_all}`；`wire::defs_list`。
- **编写工具**（`author`，feature `runner`）：`AuthorTools`、`tool_specs`，trait **`AuthorHost`** 和 `Notice`，22 个工具的名字、
  `RUN_TOOLS`、`CONVERSATION_KIND`、`NO_CURRENT`、`project_root`、`catalog_json`、`part_schemas`、`parts_used`——从
  `harnix-ext-flow` 搬来，每个工具都可以显式给出流程或运行。`Op` 标上 `#[non_exhaustive]`，加了 `ReadFlows`、`ReadFlow`、
  `ListRuns`、`ReadRun`、`DryRun`；`Engine::authorize` 公开。
- **`ProjectLayout`**（新，在根上）：项目数据目录、worktree 的根和分支前缀，由宿主给——`Engine::with_layout`、`Engine::layout`
  （代替写死的 `kernel::refs::worktree_dir`、`worktree_branch`，两个都删了）；`project_pack_dir(root, &layout)`、
  `ProjectPacks::new(base, root, &layout, trusted)` 收它。`.gitignore` 只写 `runs/`；worktree 不再放在项目里。
- **`ProjectLayout::agent_dirs`**（`with_agent_dirs`、`agent_dirs_in`）与 **`Call::agent_dirs`**：数据目录里会话当作自己的来写的子目录。
- **`Call::reserved`**：判定调用时，根里面算作根外面的目录。
- **格式写下来了**：`FlowDef`、`PolicyDef`、`GateDef`、`HookDef`、`RunRecord` 的 `format`；`FileStore` 的 `FORMAT` 文件，没有它的
  运行目录不再读。
- **`shown_in`** 代替 `show_in_space`（`CreateIn`、`RunRecord`、`RunSummary`）。
- **`BoundSession { id, opened_with }`**（新，在根上）：`RunRecord::session_keys` 里键对应的不再是光秃秃的会话 id；
  `attach_session`、`attach_main` 多收 `opened_with`。
- **`effort`**：会话执行者、`MainSession`、`Placement` 上；`EFFORTS`（新，在根上）。
- 示例流程包（`packs/examples`）和金样运行（`tests/golden.rs`）；**`MemPacks::examples()`**（新）：库自己带上的示例包，目录里的
  每个文件都在里面。

迁移：从 `harnix-ext-flow` 用过来的宿主，`harnix_ext_flow::X` 仍然可用（它转出了整个库）；直接依赖本库的写 `harnix_flow::X`。
这些改动之前写下的运行状态不再读：`show_in_space` 和光秃秃的会话 id 没有了，没有 `FORMAT` 的存储目录会被拒——清掉，或自己迁。
