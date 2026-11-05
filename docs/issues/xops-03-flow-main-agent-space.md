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

- **会话站缺省在主会话里做**：`session` 的缺省从 `new` 改成 `main`。`main` 的站不能再写 `scene` / `model` / `provider` /
  `instance` / `template` / `params` / `space`——那是主会话的，写在 `main:` 里；要别的模型，用子 Agent（§2）。
  `new` / `per_run` / `{key}` 仍可用，意思是**主会话之外的独立会话**（xops-02 的落点、结构化输出、权限规则都照旧）。
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
  拿到结果后整理成符合契约的输出，`flow_submit` 交上"。同一站一起派出的几个子 Agent 任务并成一轮，让主 Agent 在
  一次回复里并发调用 `Agent`。
- **以后是程序起**：Agix issue 草稿 `.local/agix-issue-09-host-started-subagents.md`（`session.startAgent`、`tag`、
  `structured_output`、`usage`）。daemon 报了 `host_agents` 就改走它，不经主 Agent；本 issue 不实现这条，只留接缝。
- 子 Agent 的权限询问冒到宿主时，模块按它那一站的 `writes` 答（与主会话同一规则）。
- 主会话里列出、打开子 Agent 与任务：`session.tasks`（模型起的子 Agent、Shell、监视器）、侧链会话
  （`session.list {parent_session_id}`）、这个运行的独立会话（§1 的 `new` / `{key}`）。都是 Agix 0.2.10 已有的。

### 3. 运行状态与暂停

内核存四种：`active`、`paused`、`finished`、`cancelled`。读者看到的 `RunView.phase` 由它与"此刻有没有活在跑"推出：

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

（实施中）
