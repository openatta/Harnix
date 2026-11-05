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
- `tool` 出口调的应用工具**必须只读、幂等**：站每次被判（每次 `evaluate`、会话任务每轮结束、每次再交）都会再调，
  一次操作遇到 rev 冲突重算时也会再调（前一次的结果被丢弃）。写东西的工具会被写任意多次。`Tools` trait 的文档写明了这一点。

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

## 实现记录

内核（`be9157f`）、界面包（`ebf93e0`）之后，模块与接入：

- `crates/flow/src/module/`（feature `module`，缺省开；`--no-default-features` 只剩纯内核，不链 Harnix / Agix）：
  `Flow::builder()`（`store` / `store_with(|data_dir| …)`，缺省是模块数据目录下的 `FileStore`；`packs`；`scene` 包成 `flow-<id>`，
  加 `flow_state`、`flow_submit`、`flow_propose`，包过的 id 并进目录；`programs` `gates` `tools` `hooks` `authorizer` `space`
  `guarded_tools`（缺省 `Bash PowerShell REPL Write Edit NotebookEdit`）`recording` `lease_ms` `tick_ms`），`FlowHandle`
  （`engine()`、`subscribe()`、`dispatch(outcome)` / `settle(result)`——应用从 Rust 调内核后把结果交回来，工作才有人做）。
- 模块：space 在 `system` 上；`engine()` 贡献包过的 scene 和只装在这些 scene 会话上的 `writes` 中间件；`start` 建内核、
  `scene.activate` 包过的 scene、发一次 `runs` 与各进行中运行的 `run`、按 `dispatched_work` 接着做重启前派出去没回来的活、起 1 s 的 `tick`。
- 派活：`(run, station, round, task)` 同一 token 只做一次（新 token——重试、过期重派——顶替旧的）；共用会话键的任务排队；
  会话任务每轮结束先 `evaluate`，交了就停，没交续租后提醒一次，再一轮还没交记 `fail`；轮次进行中按租约的三分之一续租。
  `TaskRec.session` 写成 `"<instance>/<session_id>"`（内核仍是不透明字符串），界面 `sessionOf` 两种都认。
- 内核补充：`Packs::flow_text`、`Packs::skills`（有缺省实现）；`dispatched_work(rec, now)`：调度器持有的未过期认领、本轮还在等结果的门。
- 界面包按真实形状改：`RunView` 的定义在 `record.def`（`defOf`）；`RunSummary.open`（`openOf`）；`CheckRec.status`（`checkOk`）；
  `skill_digest` 是 skill → 摘要的表；没有任务的站不带 `tasks`。
- 测试：内核加 `dispatched_work` 两条；模块单元测试（scene 包装、`writes` 匹配、派活去重、会话名）；`layering.rs` 加
  「flow 只用公开的模块 API」；testapp 注册 `demo-linear`、`demo-review` 与演示 programs / gates / tools / hooks；录像 `flow-analyze`、
  `flow-report`（由 `calls-a-native-tool` 手改）；`tests/live/api/flow.spec.ts` 6 条、`tests/live/ui/flow.spec.ts` 2 条（`target/testapp-ui`，
  `check.sh` 的 build 层构建它）。

没做：会话任务在重启后若会话空闲，提醒那一轮在回放下没有录像（回放名不随会话恢复），会记失败、按 `retry` 重派新会话——行为对，
但 LIVE 只测了「停在确认处重启后走完」；`flow_propose` 与 `writes` 只有单元测试，没有 LIVE 用例。

### 评审后的修改（第二轮）

| # | 问题 | 改法 | 回归测试 |
|---|---|---|---|
| 1 | 租约 token 经 `get`、`run` 主题外泄 | `view()` 清空 `lease.token`（空 token 不序列化），`Engine::record()` 给宿主自己用；push（session/program）与 subflow 任务的 `submit`/`fail`/`heartbeat` 只认 `flow`（session 另认 `agent`）；`Op::Submit`/`Op::Fail` 带 `executor`，新增 `Op::Heartbeat`；`heartbeat` 带行为者；再交也查租约是否过期，`heartbeat` 可续交过的任务 | `hardening.rs`：`readers_never_see_a_lease_token`、`push_tasks_are_handed_in_by_the_flow_alone_even_with_the_token`、`a_resubmission_needs_a_live_lease_and_a_heartbeat_extends_it`；`flow.spec.ts`「lease tokens stay with their claimants…」 |
| 2 | `foreach` 列表没解析出来时 `evaluate` 让空站通过 | `judge` 对没有任务的 `foreach` 站重新解析，不是列表（或超过 `MAX_FOREACH_ITEMS` 1000）就留一条 `foreach` 失败；解析成 `[]` 照常通过 | `an_unresolved_fan_out_stays_unmet_and_an_empty_one_passes` |
| 3 | 重启后共用会话里没 attach 过的任务只收到提醒 | 只有任务**自己**记过的会话才按「接着等」处理；否则按新任务发协议层与 skill | 逻辑在 `dispatch.rs::with_session`（需要真会话，无单元测试） |
| 4 | 共用会话：上一任务的轮还在跑就换绑 | 任务结束时若轮还在跑先 `interrupt` 并等它结算再放锁；拿到锁后若会话里有别人的轮在跑，先等它结算再绑定（那次结算不算本任务的） | `flow.spec.ts`「tasks sharing the run's session take turns in it…」（`demo-shared`，`per_run`，两站各交各的，输出契约互斥） |
| 5 | 部分返工后其余在途任务被调度器当作「不是我的了」 | 调度器只按 token 判断认领是否仍有效，不看轮次 | `dispatch.rs`：`a_partial_rework_leaves_the_other_claims_standing` |
| 6 | program 不续租；push 任务过期无限重派 | program 运行期间按租约三分之一续租；push 任务过期走失败路径，受 `retry` 次数限制 | `expired_push_task_is_dispatched_again_within_its_retries`、`an_expired_push_task_out_of_retries_fails_instead_of_looping` |
| 7 | 等共用会话锁的任务不续租 | 等锁时同样定时续租，续不上就放弃 | 逻辑在 `wait_turn_on_session` |
| 8 | `writes` 的缺口 | 4 修好了「绑定提前没了」；目录的 `scenes` 只给包过的 scene，`chat` 这类未包装的 scene 过不了校验；协议层如实列出宿主守卫的工具（`Engine::with_guarded`），`guarded_tools` 文档写明只守这些 | `module::tests::a_session_task_must_name_a_wrapped_scene`、`protocol` 单元测试、`flow.spec.ts`「a session's write its station does not allow is refused…」（`demo-propose`：testapp 把 `TodoWrite` 列为写，回放里调它被拒） |
| 9 | `lease_ms` 溢出 | 内核 `MAX_LEASE_MS` 一天封顶，`saturating_add` | `leases_are_capped_and_never_overflow`；LIVE 里 `heartbeat` 传 `MAX_SAFE_INTEGER` |
| 10 | `retry` 不看站状态 | 要求站在 working / unmet | `retry_needs_an_open_station` |
| 11 | `tick` 遇错就停 | 单条运行出错跳过，继续其余 | `a_run_that_fails_to_tick_does_not_stop_the_others` |
| 12 | 递归子流程 | 校验沿包里的子流程定义找回到自身的环；派生时子运行嵌套超过 `MAX_SUBFLOW_DEPTH`（8）层就让该任务失败 | `recursive_subflows_are_refused_and_nesting_is_capped` |
| 13 | program 的输出因站不开放被拒也算一次失败 | 新增 `FlowError::Contract`（码 `contract`）；只有契约不符才 `fail` | 由 `FlowError::Contract` 的单元测试覆盖（`sequence_runs_to_the_end`） |
| 14 | 交了之后不能再交；共享会话不关；过期的 `new` 会话不关；锁表只增不减 | 交过的任务保留绑定直到站通过或认领失效（发布时按记录清理）；运行结束 / 取消时关闭 `session_keys` 里的会话；`new` 会话任务过期重派前关闭旧会话；会话锁最后一个用户离开时删除 | `a_finished_or_cancelled_run_closes_its_shared_sessions`、`an_expired_session_task_closes_its_own_session_before_it_is_dispatched_again`、`shared::tests::a_session_lock_is_removed_when_nobody_uses_it`、`dispatch::tests::a_binding_counts_until_its_station_passes_or_its_claim_is_superseded` |
| 15 | `FileStore` 路径穿越、`list` 每次扫目录、阻塞 IO | `history`、`commit` 也校验运行 id；`list` 用首次扫描建立、由本进程提交维护的摘要索引；磁盘操作在有 Tokio 运行时时进 `spawn_blocking` | `store.rs`：`file_store_names_no_file_outside_its_directory`、`file_store_lists_from_an_index_its_commits_keep_current` |
| UI | 重启后已结束的运行一直「加载中」 | `useRunView` 在主题没有值时用 `get`；模块启动时也发布列表里全部运行 | `components.test.tsx`「reads a run the topic has no value for…」；`ui/flow.spec.ts`「a run that finished before a restart opens from the list…」 |
| UI | 返工、强制按钮与内核不一致 | 返工只在站 held / working / unmet 时可用；强制在 held 时也可用 | `model.test.ts` 的 actions 两条 |
| UI | 提议可被采纳两次 | `flow_propose` 的条目带 `session`；`rework`/`cancel` 接受 `proposal {instance, session_id, entry}`，模块串行判断、只做一次并在会话里写 `proposal_handled`；新方法 `proposal` 查是否已采纳；卡片按它显示；`proposal_handled` 条目有自己的 `flow.block` | `components.test.tsx`「names itself when accepted…」；`flow.spec.ts`「…its proposal is accepted once」 |
| UI | subflow 任务租约 `until = u64::MAX` 显示乱码 | `leaseUntil()` 认出这种租约，显示「到子运行结束」 | `model.test.ts`「draws a subflow task's lease…」 |

另：`tokio` 改为内核的常规依赖（`FileStore` 的阻塞池），`module` 特性不再单独带它；testapp 多了 `demo-propose`、`demo-shared` 两条演示流程与 `flow-propose`、`flow-shared` 两份录像。
回放的发现：引擎在每轮正常结束后会多调一次模型做记忆提取，按位置回放时它会吃掉下一轮的录像调用——多轮录像要在轮与轮之间放一个占位调用（`flow-shared` 这样做了）。

全量 `check.sh` 第一遍暴露两处：① `examples/hello-app` 的 harnix 依赖区块过期（缺 `harnix-flow`，也缺此前就漏了的 `harnix-client`），
`sync-app-deps.mjs` 重写后 `--check` 通过（`0e6c229`）；② `demo-propose` 的返工第二轮偶发等 60 s：`new` 会话任务的锁按
`run/station/task` 取，第二轮要等第一轮会话的轮结算，而那一轮结算前 chat scene 会用模型给会话起名——回放下这次调用不在录像里、
直接上网。改为 `new` 会话任务的锁键带 token（各轮各次互不等待），`FlowScene` 不自动起名（`3802b2b`；
`dispatch::only_tasks_sharing_a_session_wait_for_each_other`）。之后全量 `check.sh`：build、rust、clippy、offline、frontend、api（两种拓扑）、
template、browser（两种拓扑）、budget 全过；live、interface 本机 `.env` 无模型未跑，remote 未要求。

## 落地

落地提交：`be9157f`…`2df2b74`（main，2026-11-05，快进合并自分支 `flow`，未推送；版本待下次发布）。
内核、存储与包、模块在 `crates/flow`；界面在 `ui/packages/flow`；testapp 的演示流程在 `crates/testapp/src/flow_demo.rs`。
测试：`crates/flow/tests/{engine,hardening,store,validate}.rs` 与模块单元测试；live API `tests/live/api/flow.spec.ts`、
界面 `tests/live/ui/flow.spec.ts`；全量 `check.sh` 11 层过（remote 未要求；live、interface 用模型另跑一遍，都过）。

