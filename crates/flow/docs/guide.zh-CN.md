# 流程指南

流程是一张由**站**组成的有向无环图：一个**运行**带着一个**主题**（一份代码、一个需求、一篇稿子……）从第一站走到最后一站。每一站有它要做的**工作**和**出站条件**，条件都满足才放行到下一站。流程定义是一个 YAML 文件；它只说"做什么、怎么判"，具体怎么做好写在 **skill** 里。

流程包里除了流程和 skill，还有四样：**规则**、**策略**（一组**权限**）、**门禁**、**钩子**。它们的分工是：每个时间点只有一处能拦——

| | 是什么 | 什么时候起作用 | 拦不拦 |
|---|---|---|---|
| **规则** | 有名字的散文：整个流程里应当怎样 | 喂给干活的会话 | 不拦 |
| **策略 / 权限** | 一组 `allow` / `ask` / `deny`：哪些工具、在什么参数上 | 干活过程中，每一次工具调用 | **拦** |
| **门禁** | 有名字的确定性检查 | 出站时，作为出站条件 | **拦** |
| **钩子** | 把运行的事件告诉外面的系统 | 事件发生后 | 不拦 |

skill 讲"这一站怎么做"，规则讲"整个流程里不能违背什么"，都只是给 Agent 看的：一段散文扩大不了、也缩小不了能做的事——那只由权限定。规则要检查，就写成门禁。

## 对象与含义

| 对象 | 含义 |
|---|---|
| **流程**（flow） | 一份定义：`name`、`title`、`inputs`（启动参数）、`main`（主会话放哪）、`rules` / `policies` / `permissions`（每一站都受的）、`stations`。改了定义不影响已经在跑的运行——运行启动时把定义连同它用到的策略、门禁一起钉住了。 |
| **站**（station） | 判断的单位。`needs` 排先后（不写 = 列表里的上一站，`[]` = 没有前置）；`when` 不成立就跳过；`hold: true` 进站后等人放行。 |
| **工作** | 站要做的事，四选一：`executor`（一个任务）、`tasks`（几个各有 id 的任务）、`foreach` + `executor`（列表里每项一个任务，`max_parallel` 限并发）、都不写（纯检查点，进站就判）。 |
| **任务**（task） | 一件具体的活。会话、子 Agent、程序任务由流程派发；外部任务等别人认领；子流程任务开一个子运行。 |
| **执行器**（executor） | 谁来做：`session`（会话，照 skill 做）、`agent`（主 Agent 交给的子 Agent）、`program`（应用自己的代码）、`external`（人或外部 Agent 认领）、`subflow`（另一个流程）。 |
| **出站条件**（exit） | 全部满足才放行：`output`（每个任务交的输出满足 `output` 契约）、`gate`（包里的一个门禁通过）、`confirm`（有人确认）、`children`（子运行都结束）。 |
| **输出契约**（output） | 一个 JSON Schema。会话 / 子 Agent 交的输出必须满足它，后面的站可以引用：`${stations.<id>.output.<字段>}`。 |
| **运行**（run） | 一个流程在一个主题上的一次执行。状态见下文。 |
| **主会话 / 主 Agent** | 每个运行都有一个主会话，就是这个流程的主 Agent。没写 `scene` 的会话站、所有子 Agent 站，都在主会话里一轮一轮地做。在流程 space 的运行页选中运行，中间就是它；可以在里面插话。 |
| **子 Agent**（`kind: agent`） | 主会话下的子 Agent：主 Agent 用 `Agent` 工具把任务交给它（可指定 `agent_type`、`model`），拿到结果后交给流程。 |
| **独立会话** | 写了 `scene` 的会话站有自己的会话（`session: new`，或 `{key: …}` 按键共享），可以放到别的实例、模板实例、space，用别的模型。 |
| **规则**（rule） | `rules/<名>.md`，第一个 `# ` 标题是它的标题。流程级和站级的 `rules` 引用它；一站受的规则 = 流程级 ∪ 站级，放进会话任务的消息里（在 skill 之前），进站时固定版本。从不拦。 |
| **权限**（permission） | 一条 `effect`（`allow` / `ask` / `deny`）+ `tools`（工具名或 `ns__*` 一组），可选 `paths`（路径参数按 glob；相对的从运行工作目录算）、`commands`（命令按 glob）、`args`（输入的字段 → glob 列表）、`message`。模式里可以有引用：`${run}`、`${inputs.k}`、`${item…}`、`${workspace}`。 |
| **策略**（policy） | `policies/<名>.yaml`：有名字的一组权限，流程级和站级的 `policies` 引用它。 |
| **判定** | 一个任务的权限 = 流程级策略 → 流程级权限 → 站级策略 → 站级权限，派工时解析引用、定下来。每次工具调用：命中的里 `deny` 大于 `ask` 大于 `allow`；一条都没命中时，受限工具（Bash、Write、Edit……）拒绝，其余放行。`ask` 留给人在会话里批，运行显示"等待：审批"。主会话、独立会话都由流程这一处答。 |
| **门禁**（gate） | `gates/<名>.yaml`，出站条件 `{kind: gate, gate: <名>}` 引用它。三种：`tool`（调应用的只读工具，对结果断言，判站时当场判）、`command`（在运行工作目录跑一条命令，退出码 0 为过；应用打开了才能用；运行的上下文从环境变量 `FLOW_RUN`、`FLOW_STATION`、`FLOW_WORKSPACE`、`FLOW_CONTEXT` 读，不拼进命令行）、`host`（应用自己的检查，按名字）。 |
| **钩子**（hook） | `hooks/<名>.yaml`：`on`（事件，可写 `station.*`；不写 = 全部）、`flows`（只管这些流程；不写 = 全部）、`kind`（现在只有 `webhook`）、`action`（URL）、`secret_env`（可选：带签名）。**不写在流程里**：放进包里就生效，不改任何流程。投递失败重试三次，从不卡住流程。 |
| **返工**（rework） | 退回到某一站重做（`rework_to` 列出能退到哪；`on_unmet` 自动返工）；可以只重做其中几个任务。 |
| **租约**（lease） | 认领一个任务就拿到一份限时租约，做的过程中续租；过期了任务回到待做（或按 `retry` 重试）。 |
| **试运行**（dry run） | 用真的流程内核走一遍，外面的一切都模拟：不建会话、不调程序、不花钱。看路径对不对、每个会话站本来要收到什么消息。 |

## 运行的状态

| 状态 | 含义 |
|---|---|
| 运行中 | 有任务在做 |
| 等待 | 没结束，但此刻没有活在跑，卡在人或外部上：待审批（一次调用等人批）、待确认、搁置、未达标、任务失败、等外部认领、等子运行 |
| 暂停中 | 已暂停，手上的活还在跑完 |
| 已暂停 | 手上的活都完了；不进下一站，也不派新任务 |
| 已完成 / 已取消 | 结束 |

暂停是"跑完手上的再停"：正在跑的一轮不会被打断，结果照常记下；恢复后接着走。每一站记耗时和 token（会话站、子 Agent 站有 token）。

## 怎么定义一个流程

1. 先想清楚**站**和它们的先后：哪些可以并行（`needs` 指向同一个前置），哪里汇合（`needs: [a, b]`）。
2. 每站写**工作**和**出站条件**。要会话做的，写 `skill` 和 `output` 契约；要人拍板的，加 `confirm`。
3. 主 Agent 的落点和模型写在流程级 `main:`；某站要别的模型，用 `kind: agent` 或独立会话。
4. 写**权限**：这一站能动什么。几个流程都用的，写成策略放进包里，用 `policies` 引用。
5. 不能违背的约束写成**规则**；能机械判的写成**门禁**，放进出站条件。
6. 要让别的系统知道进度，往包里放一个**钩子**。
7. 校验（`flow_validate`），试运行（`flow_dry_run`），再启动。

## 例子

### 串行：起草 → 人确认

```yaml
name: draft-and-sign
title: 起草并确认
stations:
  - id: draft
    title: 起草
    executor: {kind: session}        # 没写 scene：在主会话里做
    skill: draft
    output:
      type: object
      required: [summary]
      properties: {summary: {type: string}}
    exit: [{kind: output}]
  - id: sign
    title: 确认
    exit: [{kind: confirm}]
    rework_to: [draft]
```

### 并行再汇合

```yaml
name: review-both
stations:
  - {id: collect, executor: {kind: session}, skill: collect, output: {type: object}, exit: [{kind: output}]}
  - {id: code, needs: [collect], executor: {kind: agent, agent_type: reviewer}, skill: review-code, output: {type: object}, exit: [{kind: output}]}
  - {id: docs, needs: [collect], executor: {kind: agent}, skill: review-docs, output: {type: object}, exit: [{kind: output}]}
  - {id: join, needs: [code, docs], exit: [{kind: confirm}]}
```

### 逐项：每个模块一个任务

```yaml
name: per-module
stations:
  - id: list
    executor: {kind: session}
    skill: list-modules
    output: {type: object, required: [modules], properties: {modules: {type: array}}}
    exit: [{kind: output}]
  - id: analyze
    foreach: ${stations.list.output.modules}
    max_parallel: 2
    executor: {kind: agent, model: claude-haiku-5-5}
    skill: analyze
    output: {type: object}
    exit: [{kind: output}]
```

### 规则、策略、门禁

```yaml
name: guarded-draft
rules: [no-secrets]                 # 每一站都受
stations:
  - id: draft
    executor: {kind: session}
    skill: draft
    permissions:                    # 这一站能写草稿目录；写发布目录要人点头
      - {effect: allow, tools: [Write, Edit], paths: ["${workspace}/drafts/**"]}
      - {effect: ask, tools: [Write], paths: ["${workspace}/release/**"], message: 写发布目录要人点头}
    output: {type: object}
    exit: [{kind: output}, {kind: gate, gate: ci}]
  - id: review
    executor: {kind: agent}
    skill: review-docs
    policies: [read-only]           # 评审只看不改
    output: {type: object}
    exit: [{kind: output}]
```

`deny` 总是赢：流程级拒绝了的，站级放不开。所以通用的拒绝放在用得上的站上，放开的权限写在需要它的站上。

包里配套的几样：

```yaml
# rules/no-secrets.md 是一段 markdown：
#   # 不外泄机密
#   密钥、口令、令牌一律不写进产出。

# policies/read-only.yaml
name: read-only
permissions:
  - {effect: deny, tools: [Write, Edit, NotebookEdit], message: 这一站只看不改}
```

```yaml
# gates/ci.yaml —— 在运行工作目录跑测试
name: ci
title: 测试通过
kind: command
command: cargo test --quiet
timeout_seconds: 600
```

```yaml
# hooks/erp.yaml —— 进度推给 ERP，不改任何流程
name: erp
on: [run.created, "station.*", task.asking, run.finished, run.cancelled]
flows: [guarded-draft]
kind: webhook
action: https://erp.example.com/harnix/flow
secret_env: ERP_HOOK_SECRET
```

钩子收到的是 `POST` 的 JSON：`{hook, delivery, event: {kind, run, flow, at, station?, task?, data}, run: {id, flow, title, subject, status, phase, usage}}`，头里有 `X-Harnix-Event`、`X-Harnix-Delivery`；配了 `secret_env` 时有 `X-Harnix-Signature: sha256=<正文的 HMAC>`。同一次投递重试时 `delivery` 不变。

### 独立会话：放到别的实例、用别的模型

```yaml
  - id: build
    executor: {kind: session, scene: flow-coding, model: claude-sonnet-5-5, session: new}
    skill: build
    permissions: [{effect: allow, tools: [Bash, Write, Edit]}]
    output: {type: object}
    exit: [{kind: output}, {kind: gate, gate: ci}]
```

## 在流程 space 里

- 右栏**流程**页：选中一个流程，详情在二级栏打开（逻辑框图、定义、配套的 skill / 规则 / 策略 / 门禁 / 钩子、YAML），中间就是这个流程自己的会话——在里面让 Agent 改它、试运行它、启动它、操作它的运行（暂停、恢复、确认、强制、返工……每一样都要你批准）。
- 右栏**运行**页：选中一个运行，详情在二级栏打开，中间是它的主会话；子 Agent 与任务在运行详情里列出，点开就看。一次调用等人批时，批准卡片就在主会话里。
- 什么都没选：中间是通用会话，用来新建流程、问问题。在通用会话里新建的流程，这个会话就成了它的会话。
