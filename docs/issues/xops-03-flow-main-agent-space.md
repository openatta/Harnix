# xops-03：流程的主 Agent 与子 Agent、运行状态与暂停、按站的耗时与用量、流程 space 改版

类型：新能力｜优先级：高｜代码位置对照 `3d13936`｜依赖 xops-02

## 需求

xops-02 落地后，用户手动试流程 space，提出四处要改：

1. **一个运行没有"主 Agent"**。每个会话站默认各开一个会话，运行是一堆零散的会话；选中运行时没有一个会话可以看执行过程、插话。
   用户的模型是：**运行有一个主会话，就是这个流程的主 Agent**；没有特别声明的站都在主会话里做；声明了子 Agent 的站，
   由主会话下的子 Agent 做；在主会话里能列出、打开子 Agent 与任务。
2. **运行状态不够**：内核只有 进行中 / 完成 / 取消，没有暂停，也看不出"在跑"还是"在等人"。
3. **看不到每站花了多久、多少 token**。
4. **流程 space 的三栏要改**：右栏选中什么，中间就是它的会话；流程页、运行页第一行是 添加 / 筛选 / 排序；
   流程详情能看 YAML；参考页是一份说明文档，会话里能问流程的帮助。DEMO 要有数据可看（只给 demo，空应用本身不带）。

原则不变：**流程的逻辑全部在 `harnix-flow`，不进 Agix，不改 Agix。**

## 建议

### 1. 主会话（主 Agent）

- **每个运行一个主会话**，运行建好就开（第一站不是会话站也开），记在 `RunRecord.main_session`。
  它的落点与模型在流程级定义，缺省是模块的主 scene（空应用：`flow-coding`）、模块的 space：

  ```yaml
  main:                      # 可选；字段同 xops-02 站的落点
    scene: flow-coding
    model: …
    provider: …
    instance: … | template: … + params: {…}
    space: …
  ```

- **会话站没写 `scene` 就在主会话里做**（`session` 不写或写 `main`）。这样的站不能写 `model` / `provider` / `instance` /
  `template` / `params` / `space`——那是主会话的，写在 `main:` 里；要别的模型，用子 Agent（§2）。写了 `scene` 的会话站是
  **主会话之外的独立会话**（`session` 缺省 `new`，也可 `per_run` / `{key}`；xops-02 的落点、结构化输出、权限规则都照旧），
  所以 xops-02 的流程语义不变。
- **主会话一次一轮**：主会话里的任务排队，一个任务一轮（xops-01 的共享会话那套：认领续租、等前一轮结束）。
- **交付走 `flow_submit`**：Agix 的 `output_schema` 是建会话时定的，一个会话一份；主会话里各站契约不同，所以主会话
  的任务一律用包装 scene 的 `flow_submit` 交，模块按这一站的 `output` 校验。
- **限写按站自动应答**：主会话建时对每个受限写工具（`guarded_tools`）加一条 `ask` 规则。模块盯着主会话：一轮属于某个
  任务时，这一轮的权限询问由模块按该站的 `writes` 自动答（允许 / 拒绝，`session.respondToPrompt`）；不属于任务的轮次
  （用户自己在主会话里说话）照常由人批准。拒绝的理由写明"站 X 不允许写 Y"。
- 用户可以在主会话里插话（排队 / 引导，Agix 现成的）。主会话里**没有**确认 / 强制 / 返工的工具——干活的 Agent 不能批准自己。

### 2. 子 Agent 站（`kind: agent`）

```yaml
executor:
  kind: agent                # 主会话下的子 Agent
  agent_type: reviewer       # 可选：Agix 的 agent 目录里的类型（模型、工具、skills 由它定）
  model: …                   # 可选：Agent 工具的 model
  description: …             # 可选：给人看的一句话
```

- **现在是模型起**：Agix 没有让宿主起子 Agent 的接口（`agent.*` 未实现）。子 Agent 站的任务是主会话里的一轮：
  写明"用 `Agent` 工具（`subagent_type` / `model` 为 …）把这件事交给子 Agent，`description` 用 `flow:<run>/<station>/<task>`，
  拿到结果后整理成符合契约的输出，`flow_submit` 交上"。主会话一次一轮，所以同一站的几个子 Agent 任务一个接一个；
  要并发，等程序起（下一条）。
- **以后是程序起**：Agix issue 草稿 `.local/agix-issue-09-host-started-subagents.md`（`session.startAgent`、`tag`、
  `structured_output`、`usage`）。daemon 报了 `host_agents` 就改走它，不经主 Agent；本 issue 不实现这条，只留接缝。
- 子 Agent 的权限询问冒到宿主时，模块按它那一站的 `writes` 答（与主会话同一规则）。
- 主会话里列出、打开子 Agent 与任务：`session.tasks`（模型起的子 Agent、Shell、监视器）、侧链会话
  （`session.list {parent_session_id}`）、这个运行的独立会话（§1 的 `new` / `{key}`）。都是 Agix 0.2.10 已有的。

### 3. 运行状态与暂停

内核的 `RunStatus` 不变（`active` / `finished` / `cancelled`），暂停记在 `RunRecord.paused`（暂停中的运行仍是 `active`，
提交、确认这些照常可做）。读者看到的 `phase`（`RunRecord::phase()`，`RunSummary.phase`、`RunView.phase`）由它与
"此刻有没有活在手上"推出：

| `phase.state` | 含义 |
|---|---|
| `running` | 有任务在做：被认领的会话 / 程序任务、在跑的闸门 |
| `waiting` | 没结束、没有活在跑，卡在人或外部：`phase.reason` 写明（待确认、搁置、未达标、任务失败待重试、等外部认领、等子运行） |
| `pausing` | 已暂停，手上的活还在跑 |
| `paused` | 已暂停，手上的活都完了 |
| `finished` / `cancelled` | 结束 |

**暂停**（`pause` / `resume`，新操作，`Op::Pause` / `Op::Resume` 过授权）：

- 已经在跑的任务跑完，结果照常记下，这一站照常判定；
- 不进下一站，同一站里还没认领的任务也不派发、不许外部认领；
- 恢复后照常推进。暂停中可以取消；确认、强制、返工照常可做（只改记录，不派发，恢复时再派）。

### 4. 耗时与用量

- `TaskRec` 加 `started_at`（认领）、`ended_at`（交付 / 失败）、`usage`（这一任务各轮的 token 合计）。
- `RunRecord.usage_other`：主会话里不属于任何任务的轮次（用户插话）。
- 模块在每一轮结束时（`harnix.turn_settled` 的 `outcome.usage`）记到这一轮的任务上（`Engine::account`，不写历史）。
  主会话的轮次按 turn id 归属：这一轮第一帧到来时它在为哪个任务做（交付常在一轮中途，到结束时任务已经放手）。
- 读者：`RunView` 给每站耗时（`entered_at` → `left_at` 或现在）与用量合计、运行合计；`runs` 列表每项带 `phase`、
  `usage`、`elapsed`。没有会话的站显示"—"。子 Agent 的用量：Agix 的一轮用量含不含子 Agent 的部分文档没说
  （issue 09 请 Agix 写明），先如实记一轮的数。

### 5. 每个流程一个会话，运行操作工具

- **流程会话**：`app.flow.session {flow?}` 给出（没有就建）这个流程的会话；不给 `flow` 是**通用会话**（新建流程、问帮助）。
  记在模块数据目录 `sessions.json`。流程会话的焦点就是这个流程、改不了；删掉流程时它的会话从记录里去掉。
- 通用会话里改 / 删 / 试运行 / 启动某个流程：工具回"到这个流程的会话里做"（右栏选中它）；新建不受限。
- **运行操作工具**（`flow-author`，只对本流程的运行）：`flow_runs`、`flow_run`（只读）；`flow_pause`、`flow_resume`、
  `flow_cancel`、`flow_confirm`、`flow_force`、`flow_rework`、`flow_retry`、`flow_release`（要用户批准，
  批准即是人的确认：记的是批准它的设备）。
- **`flow_guide`**（只读）：流程说明文档（§6），会话里问流程怎么写、对象是什么意思，Agent 照它答。

### 6. 流程 space 改版（外壳）

- **右栏选中什么，中间就是它的会话**：流程页选中流程 → 流程会话；运行页选中运行 → 主会话；没选 → 通用会话。
- **流程页**：第一行左"添加"（切到通用会话、输入框预填"新建一个流程："），右筛选（来源、有效、名字）与排序（名字、修改时间）。
  详情：图、站详情、skills、**YAML（只读）**。试运行挪到运行页。
- **运行页**：所有流程的运行与试运行（打标签）。第一行只有筛选（状态、流程、类型）与排序（开始时间、耗时、token）。
  每行：状态（暂停中转圈）、流程、开始时间、耗时、token。详情：图上每站耗时与 token、站详情与操作、暂停 / 恢复 / 取消；
  主会话里的子 Agent 与任务列表，点开在中间打开。
- **参考页**：流程说明文档（中英两份，随模块走：抽象对象、含义、怎么定义、例子），后面附本应用目录与字段表。
  例子旁"照这个新建"：切到通用会话、预填。

### 7. DEMO 数据

`scripts/harnix-demo.sh start` 第一次起时导入一组示例流程（串行、并行汇合、逐项、子 Agent 站、人工确认），并造几条
不同状态的运行（完成、等确认、暂停、取消；外部任务由脚本交付，不花钱）。只在 demo 目录；空应用本身不带。

## 兼容性

- 流程模块还没发版（xops-01 / 02 在 main 未推送），定义的变更不另设兼容：`session` 缺省改为 `main`，xops-02 的流程
  里要独立会话的站写 `session: new`。
- `RunRecord` 新字段都有缺省，旧记录照读。

## 验收

1. 一个三站流程（会话站、子 Agent 站、确认站）：一个主会话，两站的轮次都在里面；子 Agent 站的子 Agent 是主会话的侧链；
   主会话里能列出它并打开。
2. 主会话里某站写了它 `writes` 外的受限工具：被自动拒绝；用户自己在主会话里要写：照常出批准卡。
3. 暂停：在跑的任务跑完并记下，下一站不进；`phase` 依次 `pausing` → `paused`；恢复后继续。
4. 每站有耗时；会话站有 token；运行合计与各站之和相等。
5. 流程会话 / 通用会话：选中流程切到它的会话；通用会话里不能改别的流程；运行操作工具要批准、只管本流程。
6. 外壳：三页的工具条、筛选、排序；YAML；运行页状态与耗时、token；参考页文档与"照这个新建"。
7. demo：`harnix-demo.sh start` 后有示例流程与各状态的运行。

## 依赖

- Agix：不改。程序起子 Agent 要 Agix 实现草稿 09，之前走模型起。

## 来源

用户 2026-11-05 手动试 xops-02 的流程 space 后的四点意见与拍板（主 Agent / 子 Agent 用方案 B：子 Agent 是 Agix 的子 Agent，
可以模型起，也可以程序起；暂停是跑完手上的再停，暂停中转圈；运行操作在流程会话里做，不另设总控会话；demo 要数据）。

## 实现记录

分支 `flow-3`（工作树 `../Harnix-flow`），基于 `3d13936`。

### 内核

- `def.rs`：`FlowDef.main: Option<MainSession>`（scene、model、provider、instance / template + params、space）；
  `Executor::Agent {agent_type, model, description}`；`Executor::Session.scene` 可空、`session: Option<SessionBinding>`，
  `SessionBinding::Main`；`Executor::binding()`（不写时：没 scene 是 `main`、有 scene 是 `new`）、`in_main()`；
  `MAIN_SESSION_KEY = "@main"`（会话键，定义里写不出：`@` 开头的 key 校验不过）。
- `run.rs`：`Usage`（input / output / 两项 cache / turns）、`TaskRec.started_at / ended_at / usage`、
  `RunRecord.paused: Option<PauseRec>`、`usage_other`、`main_session()`、`usage()`、`phase()`；
  `RunSummary.phase / usage / dry`；`RunView.phase`。
- `engine.rs`：`pause` / `resume`（`Op::Pause` / `Op::Resume` 过授权）、`account`（不写历史）、`attach_main`、
  `with_main_scene`；外部认领在暂停中被拒（"nothing new is taken on"），认领记 `started_at`。
- `txn.rs`：暂停中 `advance` 不进站、`dispatch` 不派；主会话里的任务（主会话站、子 Agent 站）的工作项：会话键 `@main`、
  落点取 `main`、scene 取 `main.scene` 或模块的主 scene（`WorkItem::Task.scene`）、交付 `flow_submit`。
- `validate.rs`：主会话站写了落点字段、子 Agent 站没有输出契约、`main.scene` 不是 `flow_submit` 的包装 scene、
  有主会话任务却无处可放（流程和模块都没主 scene）、`main` 的落点问题、`@` 开头的会话键。
- `protocol.rs`：主会话里的任务被告知"你是这个运行的主 Agent"；子 Agent 任务被告知用 `Agent` 工具（`description` 为
  标签 `flow:<run>/<station>/<task>`、`subagent_type` 为站的 `agent_type`，缺省 `flow-agent`，可选 `model`）、自己不做、
  拿到结果按契约用 `flow_submit` 交。
- 试运行沙箱：吸收 `main.scene`、预览子 Agent 任务的消息、带主 scene；摘要有 `phase` 与 `finished_at`。

### 模块

- `module/main.rs`（新）：主会话。运行建好（`run.created`）就开（`run#@main` 锁下：记录里有且打得开就用，否则按 `main` 落点
  新建、`attach_main`），任务先到也走同一处；回放用 `main_recording(run)`。权限规则对每个受限工具 `ask`。主会话被盯着：
  一轮在为某个任务做时，这一轮（含子 Agent 冒上来的）权限询问由模块按该站 `writes` 答（`session.respondToPrompt`：
  `permit` / `deny` 带理由）；不属于任务的轮次照常由人批准。每轮结束记用量。从事件里记子 Agent（`Agent` 调用的
  `description` → `subagent_progress` 的 `agent_session_id`）。重启后（`recover`）对未结束的运行重新盯上。
- 主 scene 需要项目（Agix 的 coding）时，模块给运行建工作目录 `<数据目录>/workspaces/<run>` 作主会话的项目，并写入
  agent 类型 `.agix/agents/flow-agent.md`（`scene:` 为内层场景）：子 Agent 继承包装场景时 Agix 会把场景工具再注册一遍而
  建不起来（Agix issue 草稿 10，`.local/agix-issue-10-subagent-scene-extra-tools.md`），内层场景没有这个问题。
- 方法：`session {flow?}`（流程会话 / 通用会话，`sessions.json`）、`pause` / `resume`、`sessions {run}`（主会话、独立会话、
  `agents`、引擎的 `tasks` 与侧链）、`guide {lang}`。`focus` 拒绝改流程会话的焦点。manifest 授 `session.rename`、
  `session.tasks`（模块给会话起名、列任务）。
- `flow-author`：`flow_guide`、`flow_runs`、`flow_run`（只读）；`flow_pause` / `resume` / `cancel` / `confirm` / `force` /
  `rework` / `retry` / `release`（要批准，只管本流程的正式运行，记的人是 `conversation:<instance>/<session>`）。
  通用会话里 `flow_create` 后这个会话成了新流程的会话；删流程时去掉它的会话记录。
- 流程指南 `crates/flow/docs/guide.{en,zh-CN}.md`：`flow_guide` 与参考页用；其中三份完整例子由测试校验。

### 空应用、testapp、demo

- 空应用：主会话在 `flow-coding`；`example-review` 的 review 站改为主会话里做。
- testapp：加 `flow-coding`、主 scene `chat`；新流程 `demo-main`（外部 → 两个主会话站，一个不许写一个许写 → 确认）、
  `demo-agent`（子 Agent 站，`main: {scene: flow-coding}`）；录像 `flow-main`、`flow-agent`（手工做，后者含子 Agent 的会话）。
- `scripts/harnix-demo.sh`（`start | stop | restart | status`）：第一次起时导入 `scripts/demo` 的四条流程与 skills，
  用 `crates/flow/examples/demo_seed.rs`（真内核、FileStore）造六条运行：完成、等确认、暂停、取消、退回一次后通过、
  等外部交付；都没有在手上的活，起来不调模型。`crates/cli` 的测试校验 demo 流程在空应用里有效。

### 外壳

- `@harnix/flow`：中区跟着右栏选中项走（流程会话 / 主会话 / 通用会话，`host.session.open`），选中不再调 `focus`；
  三页按 §6 改版（工具条、只读 YAML、跨流程的运行列表、phase 徽标、耗时与 token、暂停 / 恢复 / 取消、子 Agent 列表、
  指南与"照这个新建"，`host.composer.insert` 预填）。

### 测试

- 内核 `tests/main_agent.rs` 8 条（主会话与子 Agent 的工作项、主会话站的校验、暂停让手上的跑完、暂停中站通过不往下走、
  暂停中不许认领、用量归属、`attach_main`、指南例子有效）；模块单测（按站答权限、主会话的 `ask` 规则、只读工具表）。
- LIVE api `flow-main.spec.ts` 3 条（主会话一开始就有、暂停 → 暂停中 → 已暂停 → 恢复、两站同一主会话、按站自动拒绝 /
  允许 TodoWrite、用量；子 Agent 真跑起来并对上流程任务；流程会话 / 通用会话、焦点不能改、指南两种语言）；
  已有 api 用例按主会话调整（`session_keys` 多了 `@main`、提示语、空应用的预览 scene）。
- LIVE ui `flow.spec.ts` 改写为 9 条。

### 评审后的修改

评审（独立 agent，读 `3d13936..fddc0f6`）提出 3 条会出错的、4 条较小的，都改了：

| # | 问题 | 改法 | 测试 |
|---|---|---|---|
| 1 | 主会话按站答写工具时，多级命名空间组（`mcp__desk__*`）按第一段比，站不许的写被放行（安全） | 按站仍拒的受限**模式**判（`matches_any`），不再拆命名空间 | `main::tests::a_station_decides_…`（加多级组） |
| 2 | 权限询问与用量按"此刻绑定的任务"算：人在主会话里排队的一轮会被按任务的 `writes` 自动答、记到任务上；任务的 `carry` 也会把人的那轮结束当成自己的 | 一轮属于哪个任务看它的 `user_message`：派发方发给任务的文本先记下（`note_sent`），对上的那轮才是任务的，别的都是人的；`carry` 在主会话里只认自己发出的轮次（`wait_own_turn`） | LIVE `flow-main`（两站各自的 TodoWrite 按站答） |
| 3 | 运行操作工具只靠工具自己的 `ask`，一次"总是允许"之后 Agent 就能自己确认 / 强制；记的人不是批准的设备 | 流程会话建时对每个改动类工具加 `ask` 规则（压过 allow）；记的人取 `harnix.prompts.answerer`（`device:<id>`），取不到才记会话 | — |
| 4 | 暂停时已认领、还在排队等主会话的任务照样一个个做 | 拿到会话锁时运行已暂停且什么都没发：`Engine::requeue` 交回认领（回到待做、不算一次尝试），恢复时重派 | `a_claim_nothing_was_done_under_goes_back_while_paused` |
| 5 | 返工把任务的用量清零，合计少算 | 同 id 的任务跨轮带着用量；跳过、换了项而消失的任务的用量记进站的 `spent` | `tokens_spent_stay_counted_through_a_rework` |
| 6 | 被人认领的外部任务算"运行中"，暂停后一直"暂停中" | 外部认领算"等待（external）" | `a_claimed_external_task_is_waited_on_not_running` |
| 7 | 暂停中返工重进一个没任务的站会立刻判、跑闸门；子 Agent 记录不清理 | 暂停中进站不判不派，恢复时判；子 Agent 记录最多留 256 个运行 | `a_station_entered_again_while_paused_is_judged_on_resume` |

### 全量检查

全量 `check.sh`，带模型（`.env` 从主工作树临时拷来、跑完删掉）：build、rust、clippy、offline、frontend、api、template、
live、browser、interface、budget 都过（remote 未要求）。第一次跑 frontend 因 `tests/live/coverage.md` 没重生成、live 因
`flow-main` 的一个等待条件碰到进站前的旧帧而失败；修了以后这两层与 api 层重跑都过（live 695 s）。

### 已知

- 子 Agent 站在主会话里一轮一个，同站的几个子 Agent 不并发；要并发得等 Agix 的宿主起子 Agent（草稿 09）。
- 站写了别的 `agent_type`、而那个类型没写 `scene:` 时，子 Agent 仍会撞上 Agix 的重复注册（草稿 10）。
- Agix 0.2.10 的 `session.tasks` 对跑完的前台子 Agent 回空、侧链列表也空；子 Agent 列表靠事件，不跨重启。
- 回放里流程会话不带录像，LIVE 界面的两条录像用例在另建的、设了焦点的对话里跑。
- 流程列表没有修改时间，只按名字排序。

## 落地

分支 `flow-3` 快进合并进 main：`458732f`…`ac23191`（未推送，未发版）。全量 `check.sh` 带模型 11 层过（见上）。
Agix issue 草稿 09（宿主起子 Agent）、10（场景带工具的会话起不了子 Agent）在 `.local/`，待转交。
