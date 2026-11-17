# 流程指南

流程是一张由**站**组成的有向无环图：一个**运行**带着一个**主题**（一份代码、一个需求、一篇稿子……）从第一站走到最后一站。每一站有它要做的**工作**和**出口条件**，条件都满足才放行到下一站。流程定义是一个 YAML 文件；它只说"做什么、怎么判"，具体怎么做好写在 **skill** 里。

整张图分五组，从"做什么"读到"怎么写"：

1. **流程骨架**——定义做什么：流程、站、工作与工作项、出口条件、输出契约。
2. **执行与运行**——谁来做、怎么跑：运行、执行者、主会话、子 Agent、独立会话、租约、返工、暂停。
3. **约束与把关**——谁能拦：规则、策略与权限、门禁、人的确认。
4. **对外接口**——和外面怎么连：钩子（往外报进度）、外部工作项（外面认领、交付）、应用提供的能力。
5. **编写与试用**——怎么写出来：流程包与件、流程会话、校验、试运行。

## 一、流程骨架

一条流程是一张站的有向无环图；每站有**工作**和**出口条件**，出口条件都满足才往下走。

| 对象 | 含义 |
|---|---|
| **流程**（flow） | 一份定义：`format`（可不写：定义格式，`1`；读旧格式的库整份拒读）、`name`、`title`、`inputs`（启动参数）、`main`（主会话放哪）、`rules` / `policies` / `permissions`（每一站都受的）、`guard`（执行语义，见第三节）、`output_dir`（交付物放哪，见第二节）、`workspace`（任务在项目本身还是它的 worktree 里干活，见第二节）、`stations`。改了定义不影响已经在跑的运行——运行启动时把定义连同它用到的策略、门禁一起钉住了。 |
| **站**（station） | 判断的单位。`needs` 排先后（不写 = 列表里的上一站，`[]` = 没有前置）；`when` 不成立就跳过；`hold: true` 进站后等人放行。 |
| **工作** | 站要做的事，四选一：`executor`（一个工作项）、`tasks`（几个各有 id 的工作项）、`foreach` + `executor`（列表里每项一个工作项，`max_parallel` 限并发）、都不写（纯检查点，进站就判）。 |
| **工作项**（task） | 一件具体的活。会话、子 Agent、程序工作项由流程派发；外部工作项等别人认领；子流程工作项开一个子运行。和项目里的「任务」（下一节的项目任务）不是一回事。 |
| **出口条件**（exit） | 全部满足才放行：`output`（每个工作项交的输出满足 `output` 契约）、`gate`（包里的一个门禁通过）、`confirm`（有人确认）、`children`（子运行都结束）。 |
| **输出契约**（output） | 一个 JSON Schema。会话 / 子 Agent 交的输出必须满足它，后面的站可以引用：`${stations.<id>.output.<字段>}`。 |

## 二、执行与运行

一次**运行**带着一个主题走这张图；每个工作项由一个**执行者**来做，主会话就是这条流程的主 Agent。

| 对象 | 含义 |
|---|---|
| **运行**（run） | 一个流程在一个主题上的一次执行。状态见下文。 |
| **项目任务** | 在一个项目里跑的运行：启动时带上 `project_root`（已存在的绝对目录）。还可以带 `workspace`（在哪干活：`project` 项目本身，有项目时缺省；`worktree` 项目的一个 worktree；`scratch` 运行自己的目录，没有项目时缺省）、`output_dir`（这一次的输出目录，压过流程写的），运行都记下来，连同它是怎么起的（`trigger`：手动，或定时）。`flow.list {project}` 列出一个项目的任务。任务的主会话、独立会话、门禁都在项目里干活（在 worktree 里跑的就在 worktree 里）；Harnix 往项目里写的只有输出目录和 `.harnix/`，其余都是 Agent 做的。 |
| **输出目录**（`output_dir`） | 交付物放的地方，属于用户、不加 ignore。流程定义里写一个相对项目的路径模板，可以用 `${flow}`、`${date}`（启动那天的本地日期 `YYYYMMDD`）、`${seq}`（两位序号：同一项目、同一流程、同一天已有目录的最大号加 1）、`${title}`、`${inputs.<k>}`；缺省 `flows/${flow}-${date}${seq}`。优先级：这次启动给的 > 流程写的 > 缺省。启动时解析、建好，必须落在项目里（`..`、绝对路径、指到外面的链接都不行，否则启动被拒）；变量的值里的 `/` 等换成 `-`，不会多出一层或爬出去。运行记下解析后的绝对路径，站消息里告诉 Agent 产出写到这里；skill、权限、门禁里用 `${output_dir}` 引用它。没有项目的运行放在它自己的工作目录下，布局一样。 |
| **过程目录** | `<项目>/.harnix/runs/<运行>/`，软件用的中间过程与状态：`task.json`（流程、inputs、主题、触发方式、状态、各站概要、输出目录，每次变化后重写）、`flow.yaml`（启动时钉住的定义和件）、`stations/<站>/`（一个工作项的 `output.json`，几个工作项时每个 `<工作项>.json`；命令门禁的 `context.json`、`gate-<门禁>.log`）。第一次在项目里建任务时往 `<项目>/.harnix/.gitignore` 写一行 `runs/`（已有的文件只补这一行；引擎往同一个文件追加它自己的行，项目自己的流程 `.harnix/flows/` 留着提交）。`.harnix/` 是项目数据目录，Harnix 和它的引擎共用；嵌入流程库的别的应用用自己的目录名。这是投影：权威的账本在应用的数据目录里，过程目录写坏了不影响运行。 |
| **worktree** | 流程写 `workspace: worktree`（缺省 `project`），或者这一次启动带 `workspace: worktree`（启动给的压过流程写的）：任务不直接改项目，在项目之外自己的 worktree 里干活（流程扩展的数据目录下的 `worktrees/<运行>`，在应用的状态目录里），分支 `harnix/flow/<运行>`，从启动时项目的 `HEAD` 拉出来。项目必须是 git 仓库（不是裸仓库、至少有一个提交），否则启动被拒，理由说清楚；`flow.workspace_info {project_root}` 事先问得到（`{git, reason?, branch?}`）。项目是仓库的子目录时，任务在 worktree 里对应的子目录干活。主会话、独立会话、门禁（`FLOW_WORKSPACE`）、输出目录都在 worktree 里，`${workspace}` 是它；判项目内外时，「项目」就是这个 worktree。过程目录不跟过去，仍在项目自己的 `.harnix/runs/<运行>/`，所有运行在一处。子运行在父运行的 worktree 里。运行结束（完成或取消）后由人决定：**合回**（`flow.worktree.merge {run}`：先把任务没提交的改动提交到分支上，再并进项目当前的分支——能快进就快进，否则在旁边另开一个临时 worktree 做合并提交，再把项目的分支快进过去；有冲突、或者项目里没提交的改动挡路时停下，答 `CONFLICT` 和冲突的文件，项目的工作区一点不动；合回后删掉 worktree 和分支）、**保留分支**（`flow.worktree.keep {run}`，缺省：提交没提交的改动，删掉 worktree 目录，分支留着，之后还能合回或丢弃）、**丢弃**（`flow.worktree.discard {run}`：worktree 和分支都删掉，里面的东西全没了）。决定记在运行上（`worktree.outcome`）。提交时 worktree 里的 `.harnix/` 只收项目自己的流程（`.harnix/flows/`），其余是引擎的（它给子 Agent 开的 worktree 在仓库根的 `.wkts/`，引擎把这一行加进根目录的 `.gitignore`）；Harnix 不改项目自己的 `.gitignore`，要不要忽略 `.harnix/` 由项目定。 |
| **执行者**（executor） | 谁来做：`session`（会话，照 skill 做）、`agent`（主 Agent 交给的子 Agent）、`program`（应用自己的代码）、`external`（人或外部 Agent 认领）、`subflow`（另一个流程）。 |
| **主会话 / 主 Agent** | 每个运行都有一个主会话，就是这个流程的主 Agent。没写 `scene` 的会话站、所有子 Agent 站，都在主会话里一轮一轮地做。流程级的 `main` 写它放哪、用哪个模型、模型推理多用力（`effort`：`low`、`medium`、`high`、`xhigh`、`max`；不写随设置）。在流程 space 的运行页选中运行，中间就是它；可以在里面插话。 |
| **子 Agent**（`kind: agent`） | 主会话下的子 Agent：主 Agent 用 `Agent` 工具把工作项交给它（可指定 `agent_type`、`model`），拿到结果后交给流程。 |
| **独立会话** | 写了 `scene` 的会话站有自己的会话（`session: new`，或 `{key: …}` 按键共享），可以放到别的实例、模板实例、space，用别的模型，有自己的 `effort`。按键共享一个会话的站要开得一样（场景、契约、模型、effort、放哪）：运行记着这个会话是怎么开的。 |
| **租约**（lease） | 认领一个工作项就拿到一份限时租约，做的过程中续租；过期了工作项回到待做（或按 `retry` 重试）。 |
| **返工**（rework） | 退回到某一站重做（`rework_to` 列出能退到哪；`on_unmet` 自动返工）；可以只重做其中几个工作项。 |
| **暂停**（pause） | 跑完手上的再停：正在跑的一轮不打断、结果照常记下，不进下一站、不派新工作项；恢复后接着走。状态见下文"运行的状态"。 |

## 三、约束与把关

每个时间点只有一处能拦：

| | 是什么 | 什么时候起作用 | 拦不拦 |
|---|---|---|---|
| **规则** | 有名字的散文：整个流程里应当怎样 | 喂给干活的会话 | 不拦 |
| **策略 / 权限** | 一组 `allow` / `ask` / `deny`：哪些工具、在什么参数上 | 干活过程中，每一次工具调用 | **拦** |
| **门禁** | 有名字的确定性检查 | 出站时，作为出口条件 | **拦** |
| **人的确认** | 有人拍板 | 出站时（`confirm`），或一次调用等批（`ask`） | **拦** |

skill 讲"这一站怎么做"，规则讲"整个流程里不能违背什么"，都只是给 Agent 看的：一段散文扩大不了、也缩小不了能做的事——那只由权限定。规则要检查，就写成门禁。

| 对象 | 含义 |
|---|---|
| **规则**（rule） | `rules/<名>.md`，第一个 `# ` 标题是它的标题。流程级和站级的 `rules` 引用它；一站受的规则 = 流程级 ∪ 站级，放进会话工作项的消息里（在 skill 之前），进站时固定版本。从不拦。 |
| **策略**（policy） | `policies/<名>.yaml`：有名字的一组权限，流程级和站级的 `policies` 引用它。 |
| **权限**（permission） | 一条 `effect`（`allow` / `ask` / `deny`）+ `tools`（工具名或 `ns__*` 一组），可选 `paths`（路径参数按 glob；相对的从运行工作目录算）、`commands`（命令按 glob）、`args`（输入的字段 → glob 列表）、`message`。模式里可以有引用：`${run}`、`${inputs.k}`、`${item…}`、`${workspace}`（任务是项目，在 worktree 里跑的是 worktree；没有项目时是运行自己的目录）、`${output_dir}`。 |
| **判定** | 一个工作项的权限 = 流程级策略 → 流程级权限 → 站级策略 → 站级权限，派工时解析引用、定下来。每次工具调用：命中的里 `deny` 大于 `ask` 大于 `allow`；一条都没命中时怎么办、碰到项目之外的路径怎么办，由流程的 `guard` 定（见下一行）。路径先规范化（`..` 等折叠），带路径的 `deny` 任一路径命中即拒；命令按 `;` `&&` `|` 拆开比，`allow` 不放行带命令替换、重定向的整行。`ask` 留给人在会话里批，运行显示"等待：审批"；没人在实例的批准时限（缺省 300 秒）内答，引擎当作拒绝。主会话、独立会话都由流程这一处答。 |
| **执行语义**（`guard`） | 流程顶层的 `auto`（缺省）或 `strict`。`auto`：不审批——没有权限命中的受限工具（Bash、Write、Edit……）在项目（或 worktree）里直接放行；碰到项目之外的路径一律拒绝，理由 Agent 看得到，不卡住运行；要放开某个外面的地方，写一条 `paths` 指到那里的 `allow`。`strict`：没有命中的受限工具拒绝，碰到项目之外的路径转给人。两种下作者写的 `ask`、`confirm`、`hold` 都照样等人；引擎自己要问的（项目外写入、`.git`、`.harnix` 等），`auto` 下也由流程按同样的规则代答，`strict` 下照旧交给人。没有项目的运行，"项目"就是它自己的工作目录。两种下项目的 `.harnix/` 都算在项目之外：它是应用的（设置、规则、项目自己的流程），只有 `paths` 指到它里面的 `allow` 才放开——写 `${workspace}/**` 的不算；引擎让会话自己写的 `scratchpad/`、`memory/` 除外，按项目里的路径判。在编写会话里存进项目的流程由 `flow_create` 写，不是任务用 Write 写。 |
| **门禁**（gate） | `gates/<名>.yaml`，出口条件 `{kind: gate, gate: <名>}` 引用它。三种：`tool`（调应用的只读工具，对结果断言，判站时当场判）、`command`（在运行工作目录跑一条命令，退出码 0 为过；应用打开了才能用；环境变量只留基础几个，运行的上下文从 `FLOW_RUN`、`FLOW_STATION`、`FLOW_WORKSPACE`（项目或 worktree）、`FLOW_CONTEXT_FILE`（过程目录里这一站的 `context.json`；小的还有 `FLOW_CONTEXT`）读，不拼进命令行；输出另存为这一站的 `gate-<门禁>.log`；超时整组杀掉）、`host`（应用自己的检查，按名字）。 |
| **人的确认** | 出口条件 `confirm`（`who` 交给应用的授权方解释）；权限 `ask` 等人批；`hold: true` 进站后等人放行；人还可以强制放行一站（带理由、留痕）或退回返工。谁能做这些由应用决定，流程只把 `who` 交给它。任务里的会话没有提问工具，模型不会停下来问人：需要人看一眼、拍板的地方，作者写一个 `confirm` 站。 |

## 四、对外接口

流程和外面的系统在三处相接：往外报进度，从外面收活，用应用自己的能力。

| 对象 | 含义 |
|---|---|
| **钩子**（hook） | `hooks/<名>.yaml`：`on`（事件，可写 `station.*`；不写 = 全部）、`flows`（只管这些流程；不写 = 全部）、`kind`（现在只有 `webhook`）、`action`（URL）、`secret_env`（可选：带签名）。**不写在流程里**：放进包里就生效，不改任何流程。投递失败重试三次，从不卡住流程。 |
| **外部工作项**（`kind: external`） | 等外面来认领的工作项：人或外部系统用 `flow.claim` 认领（拿到带期限的租约，`heartbeat` 续），`flow.submit` 交付，交付按输出契约校验。`who` 交给应用的授权方解释。 |
| **应用的能力** | 应用注册给流程用的：程序（`kind: program`，应用自己的代码）、工具（`tool` 门禁调的只读工具）、检查（`host` 门禁）、模板与实例（独立会话放在哪）。流程只用目录里有的名字。 |
| **子流程**（`kind: subflow`） | 一个工作项开另一条流程的一次运行，子运行结束就交付；出口条件 `children` 等子运行都结束。 |

## 五、编写与试用

| 对象 | 含义 |
|---|---|
| **流程包与件** | 流程包里有流程和五种件：`skills/<名>/SKILL.md`、`rules/<名>.md`、`policies/<名>.yaml`、`gates/<名>.yaml`、`hooks/<名>.yaml`。每样有来源：`builtin`（随应用）、`override`（可写层里覆盖内置的那份，删掉内置的就回来）、`user`（只在可写层）、`project`（项目自带，见下一行）。 |
| **项目自带的流程** | 项目可以带自己的流程包，随仓库提交：`<项目>/.harnix/flows/<名>.yaml`，件放在同目录的 `skills/`、`rules/`、`policies/`、`gates/`、`hooks/` 下。只对这个项目里的任务生效，是最上面的一层：同名的流程和件都以项目的为准，内置和自建的件项目流程照样能用，内置和自建的流程在项目里跑时也用项目的同名件。**项目受信才读**：没受信时这些流程照样列出来，但不读内容、不能启动，原因是 `untrusted`（「信任项目后可用」），同名的内置流程在这个项目里也被它挡住；信任之后下一次列出就能用。文件改了不用重启，下一次列出、下一次运行就是新的。`flow.defs {project}` 列出一个项目能用的全部流程。在编写会话里让 Agent「存到这个项目」（`flow_create` 带 `project_root`），之后这个会话的保存都落在项目里，启动的运行也是这个项目的任务。 |
| **流程会话 / 通用会话** | 每条流程一个自己的会话：在里面让 Agent 改它、试运行、启动、操作它的运行，都直接执行，不用人批准；校验照旧，不合法的存不进去。没选流程时是通用会话，用来新建流程、问问题。 |
| **校验** | 存之前把定义与它用到的件一起查一遍，有问题就不存；件自己存的时候也查，用到它的流程因此坏了会列出来。 |
| **试运行**（dry run） | 用真的流程内核走一遍，外面的一切都模拟：不建会话、不调程序、不花钱。看路径对不对、每个会话站本来要收到什么消息。 |

## 运行的状态

| 状态 | 含义 |
|---|---|
| 运行中 | 有工作项在做 |
| 等待 | 没结束，但此刻没有活在跑，卡在人或外部上：待审批（一次调用等人批）、待确认、搁置、未达标、工作项失败、等外部认领、等子运行 |
| 暂停中 | 已暂停，手上的活还在跑完 |
| 已暂停 | 手上的活都完了；不进下一站，也不派新工作项 |
| 已完成 / 已取消 | 结束 |

暂停是"跑完手上的再停"：正在跑的一轮不会被打断，结果照常记下；恢复后接着走。每一站记耗时和 token（会话站、子 Agent 站有 token）。

## 怎么定义一个流程

1. 先想清楚**站**和它们的先后：哪些可以并行（`needs` 指向同一个前置），哪里汇合（`needs: [a, b]`）。
2. 每站写**工作**和**出口条件**。要会话做的，写 `skill` 和 `output` 契约；要人拍板的，加 `confirm`。
3. 主 Agent 的落点和模型写在流程级 `main:`；某站要别的模型，用 `kind: agent` 或独立会话。
4. 写**权限**：这一站能动什么。几个流程都用的，写成策略放进包里，用 `policies` 引用。
5. 不能违背的约束写成**规则**；能机械判的写成**门禁**，放进出口条件。
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

### 在项目里：改代码、出报告，不审批

```yaml
name: fix-and-report
title: 修复并出报告
# guard 不写就是 auto：项目里放手干，项目外一律拒绝
output_dir: "reports/${flow}-${date}${seq}"   # 不写就是 flows/${flow}-${date}${seq}
stations:
  - id: fix
    executor: {kind: session}
    skill: fix                     # skill 里可以写 "报告写到 ${output_dir}/report.md"
    output: {type: object, required: [changed], properties: {changed: {type: array}}}
    exit: [{kind: output}, {kind: gate, gate: ci}]
  - id: review
    exit: [{kind: confirm}]        # 要人看的地方写 confirm 站：任务里模型不会停下来提问
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

### 逐项：每个模块一个工作项

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

钩子收到的是 `POST` 的 JSON：`{hook, delivery, event: {kind, run, flow, at, station?, task?, data}, run: {id, flow, title, subject, status, phase, usage}}`，头里有 `X-Harnix-Event`、`X-Harnix-Delivery`、`X-Harnix-Timestamp`；配了 `secret_env` 时有 `X-Harnix-Signature: sha256=<"时间戳.正文" 的 HMAC>`，收方据时间戳拒收旧的。同一次投递重试时 `delivery` 不变。

### 独立会话：放到别的实例、用别的模型

```yaml
  - id: build
    executor: {kind: session, scene: flow-harnix, model: claude-sonnet-5-5, session: new}
    skill: build
    permissions: [{effect: allow, tools: [Bash, Write, Edit]}]
    output: {type: object}
    exit: [{kind: output}, {kind: gate, gate: ci}]
```

## 在哪里用

- **在项目里**：项目的「＋」菜单里选「新任务」，选一条流程、按它的 `inputs` 填表、写主题，确认就开跑。在项目里的对话中打 `/flow <流程> [主题]` 也一样，对话里留一张链到任务的卡片；主题齐了、没有别的要填时直接开跑，否则打开同一个对话框。任务的主会话在项目 space 的中间打开，可以插话；右栏的「流程」页签显示它走到哪一站、每站的输出和门禁。项目菜单里的「为这个项目建流程」打开通用会话，在里面建、存的流程落在这个项目的 `.harnix/flows/`（项目要先受信）。
- **在流程 space 里**（入口在左栏底部）：左栏列出通用会话和全部流程，右栏的「流程」「运行」页签列出全部流程和运行。选中一个流程，中间就是这个流程自己的会话——在里面让 Agent 改它、试运行它、启动它、操作它的运行（暂停、恢复、确认、强制、返工……你让它做才做，直接执行）；详情（逻辑框图、定义、配套的 skill / 规则 / 策略 / 门禁 / 钩子、YAML）在中间旁边的二级栏打开。选中一个运行，中间是它的主会话；子 Agent 与后台任务在运行详情里列出，点开就看。作者写的 `ask` 等人批时，批准卡片就在主会话里。
- 什么都没选：中间是通用会话，用来新建流程、问问题。在通用会话里新建的流程，这个会话就成了它的会话。
