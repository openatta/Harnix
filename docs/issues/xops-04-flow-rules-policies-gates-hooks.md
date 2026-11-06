# xops-04：流程的规则、策略与权限、门禁、钩子；详情进二级栏；demo 会话

类型：重构 + 新能力｜优先级：高｜代码位置对照 `cfae024`｜依赖 xops-03

## 需求

xops-03 落地后，用户手动试流程 space，又提出：

1. **详情挤在列表下面**，分不清。流程、运行选中后，详情用**二级右边栏**。
2. **流程详情**应该是逻辑框图、流程定义、配套的 Skill 与规则。对照 `../XForge` 的流程定义，用户要的是它里面
   **规则（rules）、策略（policy）、门禁（gate）、权限（permission）四者的关系**能否沿用。讨论后定（2026-11-06）：
   - 四者的分工整套沿用；门叫**门禁**；
   - 审批策略（几个人批、批与做不能是同一人）**不做**，归应用（Harnix 不管用户）；
   - XForge 的台账、产出声明、站的 `human`、资格、receipt 链等**不考虑**。
3. **钩子**：第三方软件要知道进度，而且**不改流程**。形状 `on` / `kind` / `action`；现在只支持一种 `kind`，以后加种类
   不动架构。
4. **中间会话没内容**：demo 里选中流程、运行，会话是空的。要有内容可看（只给 demo）。

**不考虑兼容**：还没发布，流程的写法按最合理的方式重构，现有的示例、测试、文档一起改。

原则不变：流程的逻辑全部在 `harnix-flow`，不进 Agix，不改 Agix。

## 建议

### 0. 四者的分工

每个时间点只有一处能拦（XForge 的「写入范围只有策略执法」「执法的只有门」）：

| | 是什么 | 什么时候起作用 | 拦不拦 |
|---|---|---|---|
| **规则** | 有名字的散文：这个流程里**应当怎样** | 喂给干活的会话 | **不拦** |
| **策略** | 有名字的一组**权限**，流程与站引用 | 干活过程中 | — |
| **权限** | 一条 `allow` / `ask` / `deny`：哪些工具、在什么参数上 | 每一次工具调用 | **拦**（过程中唯一能拦的） |
| **门禁** | 有名字的确定性检查 | 出站时，作为出站条件 | **拦**（出站时与 `output` / `confirm` / `children` 并列） |

Skill 讲「这一站怎么做」，规则讲「整个流程里不能违背什么」，都只是给 Agent 看的；**一段散文扩大不了也缩小不了
能做的事**——那只由权限定。规则要检查，就写成门禁。

### 1. 流程包

包目录（内置的、可写层的都一样）：

```
flows/<name>.yaml        流程
skills/<name>/SKILL.md   Skill
rules/<name>.md          规则：第一个 `# ` 标题是它的标题，其余是正文
policies/<name>.yaml     策略
gates/<name>.yaml        门禁
hooks/<name>.yaml        钩子
```

名字都是 `[a-z][a-z0-9-]*`。`Packs` 按「种类 + 名字」统一读写（`PartKind`：`skill` `rule` `policy` `gate` `hook`），
覆盖与删除的规矩同流程（xops-02）：可写层存的同名件盖住内置的，删掉它内置的就回来。

### 2. 规则

```yaml
# 流程级：每一站都受它约束
rules: [no-secrets]
stations:
  - id: draft
    rules: [house-style]     # 站级：再加上这些
```

- 一站受的规则 = 流程级 ∪ 站级（去重，按出现顺序）。
- 会话任务的协议层里放进这些规则的正文（「## 规则」一节，在 Skill 之前），子 Agent 任务的交办说明里也带上。
  程序、外部、子流程任务不喂。
- **进站时固定版本**（同 Skill）：记下每条规则的摘要，派工时包里的已经变了就不派、任务失败并说明。

### 3. 策略与权限

```yaml
# policies/table-writes.yaml
name: table-writes
title: 只写声明的表
description: …
permissions:
  - effect: deny
    tools: [Bash, PowerShell]
    message: 这个流程不跑命令。
  - effect: allow
    tools: [mcp__desk__emit_row]
    args: {table: [orders, "order_*"]}
  - effect: ask
    tools: [Write]
    paths: ["${workspace}/release/**"]
    message: 写发布目录要人点头。
```

```yaml
# 流程里
policies: [table-writes]            # 流程级
permissions: [ … ]                  # 流程级，直接写的权限
stations:
  - id: edit
    policies: [repo-edit]           # 站级
    permissions:                    # 站级
      - {effect: allow, tools: [Write, Edit]}
```

- **一条权限**：`effect`（`allow` | `ask` | `deny`），`tools`（工具名或 `ns__*` 一组，至少一个），可选的
  `paths`（工具的路径参数 `file_path` / `path` / `notebook_path` 按 glob 比对；相对的按运行工作目录）、
  `commands`（`command` 参数按 glob 比对）、`args`（输入的顶层字段 → glob 列表，每个列出的字段都要命中）、
  `message`（拒绝或询问时给的说明）。模式里可以有引用：`${run}` `${inputs.k}` `${item…}` `${workspace}`。
- **一个任务的权限** = 流程级策略 → 流程级权限 → 站级策略 → 站级权限，**派工那一刻解析引用、定下来**，记进任务，
  这一轮做完之前不变。
- **判定**（一次工具调用）：收集命中的权限，`deny` > `ask` > `allow`；一条都没命中时，受限写工具
  （`guarded_tools`，缺省 `Bash` `PowerShell` `REPL` `Write` `Edit` `NotebookEdit`）拒绝，其余放行。
  - `allow`：模块替它批准；
  - `deny`：模块替它拒绝，理由是那条的 `message`（没有就写明哪一站、哪条策略不允许）；
  - `ask`：**留给人**。这一轮停在那张审批卡片上，运行显示「等待：审批」，人在会话里批或拒。
- **怎么到达 Agix**：会话建时对「受限写工具 ∪ 权限里提到的工具」各加一条 `ask` 规则，Agix 每次都来问，
  模块按上面判定答（`session.respondToPrompt`）。主会话、独立会话一样：**判定只有模块这一处**，不再给独立会话
  下 `deny` 规则。不属于任务的轮次（人自己在主会话里说话）照常由人批。模块答不了（看不懂输入、找不到任务）就拒绝
  （失败朝安全）。
- 站上的 `writes` **取消**：`writes: [Write]` 写成 `permissions: [{effect: allow, tools: [Write]}]`。
  受限组只放开一部分（xops-02 的 `partial_writes`）不再是问题：按每次调用判定，组里哪个都能单独放。
- 策略与门禁在**建运行时**连同定义一起固定进运行记录（`RunRecord.parts`）：判定、重启恢复都只看运行自己记下的。

### 4. 门禁

```yaml
# gates/rows-valid.yaml
name: rows-valid
title: 行合法
description: 每一行都过表的校验。
kind: tool                    # tool | command | host
tool: table.check             # 只读、幂等的应用工具（同 xops-01 的 tool 出站）
args: {rows: "${stations.emit.output.rows}"}
assert: [{path: ok, eq: true}]
```

```yaml
kind: command                 # 应用打开了才能用（FlowBuilder::command_gates）
command: cargo test --quiet
cwd: "${workspace}"           # 缺省运行工作目录
timeout_seconds: 600          # 缺省 600，最多 3600
```

```yaml
kind: host                    # 应用的 Gates 按名字跑（catalog.gates）
```

- 出站条件：`output`、`gate`、`confirm`、`children` 四种。`{kind: gate, gate: rows-valid}` 引用包里的门禁；
  **`tool` 出站取消**，写成 `kind: tool` 的门禁。
- `tool` 门禁判站时同步调用；`command` / `host` 门禁异步跑，结果回 `gate_result`（同 xops-01）。
  `command` 退出码 0 为过，说明是输出的最后 2000 字。
- 命令门禁在宿主上执行命令，所以**应用显式打开**才行（空应用 CLI 打开；testapp 打开；别的应用缺省关），关着时校验报错。

### 5. 钩子

```yaml
# hooks/erp-progress.yaml
name: erp-progress
description: 把进度推给 ERP。
on: [run.created, "station.*", run.finished, run.cancelled]   # 不写 = 全部事件
flows: [demo-write]           # 可选：只管这些流程；不写 = 全部
kind: webhook                 # 现在只有这一种
action: https://erp.example.com/harnix/flow
secret_env: ERP_HOOK_SECRET   # 可选：带签名
```

- **钩子不写在流程里**，流程的 `hooks:` 取消；加一个钩子就是往包里放一个文件，**不改任何流程**。
  `Hooks` trait 与目录里的 `actions` 一起取消：现在只有 `webhook` 一种，`kind` 是带标签的枚举，以后加种类只加一个分支。
- 事件：内核的全部事件，加上 `run.paused` / `run.resumed` / `task.asking`（权限 `ask` 等人）。
- `webhook`：`POST` JSON
  `{hook, delivery, event: {kind, run, flow, at, station?, task?, data}, run: {id, flow, title, subject, status, phase, usage}}`，
  头 `X-Harnix-Event`、`X-Harnix-Delivery`；有 `secret_env` 时加 `X-Harnix-Signature: sha256=<HMAC>`。
  10 秒超时，失败按 1s / 5s / 30s 重试 3 次，同一次投递 `delivery` 不变（对方去重）。**从不阻塞流程**；
  最近一次的结果（时间、状态、错误）在 `app.flow.hooks` 里能看到。
- 钩子是旁观者：跟当前包里的走，不固定进运行。

### 6. 编写

- 作者工具：`flow_skill_set` / `flow_skill_delete` 改为 `flow_part_set {kind, name, text}` / `flow_part_delete {kind, name}`，
  五种件都走它；照旧逐次要人批准。`flow_show` 带上这个流程用到的全部件。
- 方法：`app.flow.def` 回 `{…, parts: {skills, rules, policies, gates}, hooks}`；`app.flow.parts {kind}`、
  `app.flow.part {kind, name}`、`app.flow.save_part`、`app.flow.delete_part` 取代 skill 的那几个；`app.flow.hooks`。
- 存件时校验它自己；用到它的流程因此变得不合法的，在回复里列出来。
- 指南（中英文）重写对应章节，三个完整示例改成新写法。

### 7. 详情进二级栏（外壳）

- 外壳加**二级栏**：space 声明 `secondary`，它在中间会话与右栏之间展开，能拖宽、能关；窄屏时浮在会话上。
- 流程 space：右栏是列表（流程 / 运行 / 参考），选中一项，详情在二级栏，中间会话照旧跟着选中走。
- **流程详情**：逻辑框图（大）· 定义（每站：谁做、Skill、规则、权限、出站条件、退回到哪）· 配套（这个流程用到的
  Skill、规则、策略、门禁，以及管着它的钩子，每件能打开看原文）· YAML。
- **运行详情**：状态、每站耗时与 token、主会话 / 子 Agent / 任务、操作，内容同 xops-03。

### 8. demo 会话

- 不写 Agix 的私有存储（没有导入接口，换版本就可能读不出）。demo 第一次启动时，用 Harnix 现成的**进程级回放**
  把 demo 程序跑一遍：每个流程会话有问答与一张提议卡，通用会话里照指南新建一个流程，每个运行的主会话里有干活的过程
  （`flow_state`、交付、一次被拒绝的写、一次 `ask` 等人批、一次子 Agent）。运行记录照旧由内核按时间线造，
  时间线改成结束在启动那一刻。
- 不是 demo：空会话显示一段说明（这个会话做什么、能怎么问）。

## 兼容性

不兼容，不迁移（未发布）：`writes`、`tool` 出站、流程级与站级 `hooks`、`Hooks` trait、`catalog.actions`、
`flow_skill_*` 与 skill 的方法都取消；示例、demo、测试、文档全部改成新写法。

## 验收

- 规则进协议层、进站固定版本；策略与权限按 `deny > ask > allow` 判定，主会话与独立会话都由模块答，`ask` 留给人且运行
  显示等待审批；派工时定下的权限不随包变；门禁三种都能判，命令门禁未打开时校验报错；钩子按事件与流程过滤投递、重试、
  签名，失败不影响运行。
- 二级栏：选中流程、运行，详情在二级栏，能拖宽、能关；流程详情四块齐全。
- demo：选中任一流程、运行，中间会话有内容。
- 全量 `scripts/check.sh` 11 层通过。

## 依赖

xops-03。不需要 Agix 新接口（`ask` 规则 + `respondToPrompt` 是现成的）。

## 来源

用户 2026-11-06 的反馈与讨论（详情布局、XForge 四者的沿用、钩子、demo 会话）。
