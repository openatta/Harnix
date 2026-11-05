# xops-01：流程能力——按站推动会话工作的可选模块（`harnix-flow`）

类型：新能力｜优先级：高｜代码位置对照 `3c264da`

## 需求

一类应用的核心是**流程**：一件事按一张**站**的图往前走，每站由会话按一份 skill 去做、或由应用的代码做、或等外面
（人、外部 Agent）来做；**过没过只看结构化信号**（按契约校验的输出、门的结果、应用工具的返回、确认），不看模型写的文字；
没过可以退回上游重做。

这些都能在公开契约上做出来（无头会话、原生工具、扩展条目、宿主主题、方法），但每个应用一份、形状各不相同。
IPDrun 已经有一份（`ipdrun-flow`），XOps 要第二份——同一形状出现在两个应用里，按 SDK §2 进 Harnix。

要的是：**一个可选的 crate，应用注册才有，不注册什么都不变。** 不进骨架，不进 `with_builtins()`，不加贡献点，
不改 `Module` trait。流程模块**不知道**：运行记录存在哪、谁是用户、谁能做什么、门里的命令怎么跑——这些由应用实现 trait 交进来。

## 建议

### 1. 形态

```text
crates/flow  (harnix-flow)
├── kernel   纯库：定义 · 校验 · 调度 · 出口判定 · 返工 · 租约 · 协议层。不起会话、不碰磁盘，经 trait 读写
├── store    两份 RunStore：MemStore（测试）、FileStore（目录）
├── packs    两份 Packs：MemPacks（内嵌）、DirPacks（目录，后者覆盖前者）
└── module   把内核接到 Harnix：派活给会话、program；原生工具；方法；主题；扩展条目
ui/packages/flow  (@harnix/flow)   可选的界面组件 + registerFlowSpace()
```

应用：

```rust
let flow = harnix_flow::Flow::builder()
    .store(FileStore::new(dir))          // 必填
    .packs(MemPacks::new().flow(YAML).skill("analyze", SKILL))   // 必填
    .scene(Arc::new(ChatScene))          // 站可用的 scene：模块把它包一层，加上流程的原生工具
    .programs(MyPrograms)                // 可选
    .gates(MyGates) .tools(MyTools) .hooks(MyHooks) .authorizer(MyAuth)   // 可选
    .space("flow")                       // 会话归哪个 space，缺省 "flow"
    .build();
let handle = flow.handle();              // 应用经 Rust 调内核（Actor 由应用填）
harnix::App::builder().with_builtins().module(flow).run()
```

模块 id 是 `flow`，所以方法、主题是 `app.flow.*`，扩展条目的 `ns` 是 `app.flow`（注册方式决定，SDK §5.1）。

### 2. 术语（加进 glossary）

| 术语 | 代码 | 意思 |
|---|---|---|
| **流程** | `FlowDef` | 站组成的有向无环图，YAML，随流程包分发 |
| **运行** | `RunRecord`，`run` | 一条流程的一次执行，处理一个对象（`subject`）。不叫"实例"：实例只指 Agix 实例 |
| **站** | `Station` | 判定单位：出口条件、返工、确认、强制通过都按站算 |
| **任务** | `TaskRec` | 执行单位：一站 0..N 个，每个交一份结构化输出 |
| **执行者** | `Executor` | `session` · `program` · `external` · `subflow` |
| **纯关卡** | — | 没有任务的站，进站就判出口 |
| **出口条件** | `ExitCond` | `output` · `tool` · `gate` · `confirm` · `children`，全部满足才出站 |
| **返工** | rework | 退回上游某站：它和它的全部下游过期、重来 |
| **强制通过** | force | 有人在条件不满足时放行一站，带理由、留痕 |
| **租约** | lease | 认领一个任务的期限与 token；过期可再认领，旧 token 交不进来 |
| **流程包** | pack | 流程定义 + skill，由 `Packs` 给 |
| **协议层** | — | 模块在 skill 正文前注入的一段：交什么、怎么交、能写什么 |

### 3. 定义（YAML）

```yaml
name: demo-review            # [a-z][a-z0-9-]*
title: 演示：代码评审
inputs:
  mode: {schema: {enum: [manual, auto]}, default: manual}
rework_limit: 5              # 同一站最多进几轮，缺省 10，上限 20
hooks: [{on: run.finished, action: notify}]
stations:
  - id: collect
    executor: {kind: program, step: collect}
    output: {type: object, required: [modules], properties: {modules: {type: array}}}
    exit: [{kind: output}]
  - id: analyze                                  # needs 缺省 = 上一站
    foreach: ${stations.collect.output.modules}
    max_parallel: 2
    executor: {kind: session, scene: flow-chat, session: new}
    skill: analyze
    output: {type: object, required: [findings]}
    exit: [{kind: output}]
    retry: {max: 1}
    writes: []
  - id: lint
    needs: [collect]
    exit: [{kind: gate, gate: lint}]            # 纯关卡
  - id: self-check
    needs: [analyze]
    when: [{path: inputs.mode, eq: auto}]
    executor: {kind: program, step: self-check}
    exit: [{kind: tool, tool: open_high, args: {run: "${run}"}, assert: [{path: total, eq: 0}]}]
    on_unmet: {rework_to: analyze, max: 2}
  - id: review
    needs: [self-check, lint]
    exit: [{kind: confirm, who: reviewer}]
    rework_to: [analyze, lint]
  - id: report
    tasks:
      - {id: summary, executor: {kind: session, scene: flow-chat}, skill: report}
      - {id: sign, executor: {kind: external, who: reviewer}}
    output: {type: object}
    exit: [{kind: output}]
```

- 一站的活写成三种之一：`executor`（一个任务）、`tasks`（静态多个，各带 `id`）、`foreach` + `executor`（按列表展开，一项一个任务）；
  都不写就是纯关卡。
- `session` 分配：`new`（缺省）· `per_run` · `{key: "${...}"}`。同一会话的任务排队。
- 引用：`${run}`、`${subject}`、`${inputs.k}`、`${stations.<id>.output...}`、`${item}`、`${task}`。整串是一个引用时保留类型。
- 断言（`when` 与 `tool` 出口共用）：`{path, eq | ne | len | min_len | exists}` 恰好其一。
- **结构校验**（不过不能启动，`validate` 返回全部问题）：名字合法；站 id 唯一；`needs` 存在且无环；`rework_to`、`on_unmet.rework_to`
  是本站的祖先或本站；每站至少一个出口条件；`output` 出口要有 `output` 契约；`children` 只配 `subflow`；`confirm` 外的条件不能配纯关卡以外…
  （见 `kernel/validate.rs` 的测试）；session 站必须有 `skill` 与 `scene`；`foreach` 只配 `executor`；引用的 step、gate、tool、scene、
  skill、action 必须在目录里（问 `Packs::catalog`）。

### 4. 运行记录

```rust
RunRecord  { id, flow, def, def_digest, seq, title, subject, inputs, status: active|finished|cancelled, rev,
             created_at, created_by: Actor, finished_at, parent: Option<ParentRef>,
             stations: BTreeMap<String, StationRec>, session_keys: BTreeMap<String, String> }
StationRec { status: pending|held|working|unmet|passed|skipped|stale, round, rounds_total, auto_reworks,
             skill_digest, tasks: Vec<TaskRec>, checks: Vec<CheckRec>, confirms: Vec<ConfirmRec>,
             forced: Option<ForcedRec>, entered_at, left_at }
TaskRec    { id, item, status: ready|claimed|submitted|failed|cancelled, attempt,
             lease: Option<Lease{by, token, until}>, output, error, session, child }
History    { seq, at, by: Actor, op, station, task, detail }
Actor      { kind: String, id: String }
```

运行记录 = 快照（上面）+ 只追加的历史。`def` 是启动时钉住的整份定义（包改了，旧运行仍按它走），`seq` 是最后一条历史的序号；
`StationRec.skill_digest` 是 skill 名 → 摘要（静态 `tasks` 可以各用各的 skill）。JSON 形状就是 serde 的输出，`app.flow.schema {of:"run"}` 给它的 JSON Schema。

### 5. 内核

```rust
pub struct Engine { /* Arc<dyn RunStore>, Arc<dyn Packs>, Arc<dyn Authorizer>, Arc<dyn Tools>, Arc<dyn Clock> */ }
impl Engine {
  async fn validate(&self, yaml: &str) -> Vec<Problem>;
  async fn create(&self, CreateIn{flow, subject, inputs, title, parent}, by) -> Result<Outcome>;
  async fn get(&self, run) -> Result<RunView>;           // 快照 + 定义 + 每站未满足的条件
  async fn list(&self, RunQuery) -> Result<Vec<RunSummary>>;
  async fn history(&self, run) -> Result<Vec<History>>;
  async fn claim(&self, run, station, task, by, lease_ms) -> Result<Outcome>;      // value = {token, until}
  async fn heartbeat(&self, run, station, task, token, lease_ms) -> Result<Outcome>;
  async fn submit(&self, run, station, task, token, output, by) -> Result<Outcome>;
  async fn fail(&self, run, station, task, token, error, by) -> Result<Outcome>;
  async fn attach_session(&self, run, station, task, token, session, session_key, by) -> Result<Outcome>;  // 记下任务用的会话
  async fn confirm(&self, run, station, pass, reason, by) -> Result<Outcome>;
  async fn force(&self, run, station, reason, by) -> Result<Outcome>;
  async fn rework(&self, run, to, tasks: Option<Vec<String>>, reason, by) -> Result<Outcome>;
  async fn release(&self, run, station, by) -> Result<Outcome>;
  async fn retry(&self, run, station, task, by) -> Result<Outcome>;
  async fn cancel(&self, run, reason, by) -> Result<Outcome>;
  async fn evaluate(&self, run) -> Result<Outcome>;
  async fn gate_result(&self, run, station, round, gate, GateResult) -> Result<Outcome>;
  async fn tick(&self) -> Result<Vec<Outcome>>;        // 过期的租约
}
pub struct Outcome { pub value: Value, pub events: Vec<Event>, pub work: Vec<WorkItem> }
pub enum WorkItem { Task{run, station, round, task, token, executor, skill, item, protocol},
                    Gate{run, station, round, gate}, Hook{action, event}, CloseSession{session} }
```

- **一次操作 = 读 → 算 → `commit`（比对 rev）→ 返回事件与工作**。冲突就重读重算（有上限）。内核自己调 `RunStore::commit`，
  提交成功之前不返回任何事件与工作。子运行与父运行在同一次 `commit` 里改。
- `session` / `program` 任务由内核以 `Actor{kind:"flow", id:"dispatch"}` 自动认领并发 `WorkItem::Task`；`external` 任务停在
  `ready` 等外面 `claim`。
- 所有改状态的操作先问 `Authorizer::check(&Op, &Actor, &RunRecord)`，缺省全放行。

宿主 trait：

```rust
#[async_trait] pub trait RunStore { alloc_id(flow) · load(run) -> Option<(RunRecord)> · list(&RunQuery) ·
                                     commit(&ChangeSet) -> Result<(), StoreError::Conflict|Other> · history(run) }
pub trait Packs { flows() · flow(name) -> Option<(FlowDef, digest)> · skill(name) -> Option<(body, digest)> · catalog() -> Catalog }
pub trait Authorizer { fn check(&self, op: &Op, by: &Actor, run: Option<&RunRecord>) -> Result<(), String>; }
#[async_trait] pub trait Tools { async fn call(&self, tool, args) -> Result<Value, String>; }
#[async_trait] pub trait Gates { async fn run(&self, run: &RunRecord, station, gate) -> GateResult; }
#[async_trait] pub trait Programs { async fn run(&self, step, run: &RunRecord, item: Option<&Value>) -> Result<Value, String>; }
#[async_trait] pub trait Hooks { async fn fire(&self, action, event: &Event); }
pub trait Clock { fn now_ms(&self) -> u64; }
```

### 6. 模块

| 事 | 做法 |
|---|---|
| `session` 任务 | 在应用给的 space 建无头会话（scene 是包过的那个），送 **协议层 + skill 正文 + 本任务的 item**；`tap.next_settled()` 每轮结束续租并 `evaluate`；没交就提醒一次，会话再结束仍没交记 `fail` |
| 原生工具 | 包过的 scene 带 `flow_state`、`flow_submit {output}`、`flow_propose {kind: rework|cancel, to?, reason}`；按调用方的 `session_id` 找到它服务的任务，别的会话调就报错。`flow_propose` 写扩展条目 `proposal`，人点了才生效 |
| `writes` | 工具中间件：流程会话里，不在本站 `writes` 的非只读工具被拒 |
| `program` / `gate` / `hook` | 调应用的 trait，结果回 `submit` / `gate_result`；hook 失败只记日志 |
| 方法（`Device`） | `list` `get` `history` `start` `defs` `def` `schema` `catalog` `validate` `claim` `heartbeat` `submit` `fail` `confirm` `force` `rework` `release` `retry` `cancel`，行为者 `{kind:"device", id}` |
| 主题 | `runs`（运行列表快照）、`run {run}`（一条运行的 `RunView`） |
| 计时 | 每秒 `tick`（租约过期） |
| 重启 | `start()` 时列出进行中的运行：属于 `flow` 认领的 session 任务重新打开原会话接着等，program 任务重派；同一 `(run, station, round, task)` 只派一次 |
| Rust 入口 | `FlowHandle`：`engine()`、`subscribe()`（事件广播），应用自己填 `Actor` |

### 7. 界面（`@harnix/flow`）

只依赖 `@harnix/react`。组件：`FlowGraph`（定义模式 / 运行模式同一张图）、`StationPanel`（定义页 / 运行页，带确认、强制、返工、放行、重试按钮）、
`RunList`、`FlowReference`（schema 字段说明 + 目录 + 校验）、`ProposalCard`（`flow.block`）。`registerFlowSpace(registry, {space, module})`
把它们组成一个 space。i18n 的 owner 是 `flowui`（`flow.` 已被外壳的消息流占用）。

## 兼容性

纯增加：一个新 crate、一个新前端包；不注册的应用没有任何变化。不改骨架、协议形状、`Module` trait、贡献点：`MODULE_API_VERSION` 不升。
`harnix` 不重导出它，应用直接依赖 `harnix-flow`（`sync-app-deps` 的生成区块会带上它）。

## 验收

- **内核单元测试**（`MemStore`、`MemPacks`、假的 trait）：结构校验逐条；顺序、并行汇合、`when` 跳过后汇合；`tasks` 与 `foreach` 全交才判；
  返工的过期范围（下游过期、无关分支保留）、部分返工、返工上限；`on_unmet` 自动返工与上限；强制通过留痕；确认通过 / 驳回；
  hold 放行；租约过期后旧 token 被拒、任务可再认领；重试；子运行结束才出站；`rev` 冲突时不返回事件；授权钩子拒绝；钉住的定义摘要；
  `FileStore` 过同一组存储测试。
- **LIVE API**（`tests/live/api/flow.spec.ts`，回放）：testapp 跑演示流程，session 站经录像调 `flow_submit`，并行汇合、确认、
  强制、返工走通；设备方法以外的调用方被拒；重启后运行接着走完。
- **LIVE UI**（`tests/live/ui/flow.spec.ts`）：流程 space 里看到图与参考；启动一条演示运行，在站详情里点确认，运行走完。

## 依赖

无。用的都是已有的：无头会话、`output`/tap、扩展条目、宿主主题、`AgentScene::extra_tools`、工具中间件、录像回放。
第三方库都已在锁文件里（`serde_yaml`、`jsonschema`、`schemars`）。

## 来源

XOps（2026-11-04 起基于 Harnix 重建）：业务流程按 skill 推动 Agent，运行记录存成 XOps 自己的表，审批人经 XOps 自己的 MCP 回答——
所以存储、授权都交给应用。IPDrun 的 `crates/flow`（`1572a27`）是这个形状的第一份实现，定义格式、断言、协议层、钉住摘要取自它。
设计依据：XOps `docs/flow-design.md`。
