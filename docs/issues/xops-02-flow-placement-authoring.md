# xops-02：流程会话的落点与模型、按会话参数交付与限写、空应用自带流程、在对话里编写流程、试运行

类型：新能力｜优先级：高｜代码位置对照 `48f51f4`｜依赖 xops-01

## 需求

xops-01 落地后有四处不够：

1. **会话只能跑在 `system` 实例、模块的那一个 space、实例的缺省模型上**。站不能指定模型、provider、实例（含模板实例）、space。
2. **交付与限写绑在 `system` 上**：交输出靠包装 scene 注入的原生工具 `flow_submit`，限写靠装在 `system` 上的工具中间件。
   spawn 模板的会话跑在 daemon 进程里，与模块不共享内存（ipdrun-10 的边界），这两样都用不了。
3. **空应用 `harnix` 没有流程**：要看、要调一条流程，得另编一个应用（现在是 testapp 加一份单独构建的界面）。
4. **没有编写流程的入口**：流程包只读；改流程要改文件、重启。没有不花钱、不起会话就能验证流程走法的办法。

原则不变：**流程的逻辑全部在 `harnix-flow`，不进 Agix，不改 Agix。** 本 issue 用到的 Agix 能力都是现成的通用会话参数
（`output_schema`、`permission_rules`、`provider` / `model`），Agix 不知道流程的存在。

## 建议

### 1. 站的会话落点与模型（定义只加不改）

```yaml
executor:
  kind: session
  scene: chat                 # 任何已注册的 scene；flow-* 包装 scene 仍可用（多出 flow_state / flow_propose）
  model: claude-sonnet-…      # 可选，透传 SessionSpec.model
  provider: anthropic         # 可选，透传 SessionSpec.provider
  instance: my-agents         # 可选：已有实例名；与 template 二选一
  template: myapp.sandbox     # 可选：模板全名；模块先 ensure_ready 再建会话
  params: {name: "${run}"}    # 可选：模板参数，值可引用 ${run}、${subject}、${inputs.k}…
  space: dev                  # 可选：缺省是模块的 space
  session: new | per_run | {key}
```

- `Catalog` 加 `templates`、`instances`、`spaces`、`models`（`providers` 并在 `models` 里写成 `provider/model` 也可，实现定）。
  应用填了哪一项，校验就查哪一项；空的不查。
- 运行时落点非法（实例被别的 space 占用 → `SPACE_MISMATCH`、模板不存在、ensure 超时、模型不可用）：这次尝试记 `fail`，
  理由原样写进任务的 `error`，按 `retry` 处理。
- `WorkItem::Task` 带上解析后的落点与模型。

### 2. 交付：结构化输出；限写：权限规则

- **交付**：session 站必须有 `output` 契约。模块建会话时把契约作为 `SessionSpec.output_schema` 传入；
  每轮结算（`turn_settled.outcome.structured_output`）有值就以调度者身份 `submit`。不合格由 Agix 的 `StructuredOutput` 当场退回让模型改；
  合格但内核判不过（例如 `tool` 出口没满足）就把未满足的条件作为下一条消息发给会话，让它改了再交。
  一轮结束没交：提醒一次；再一轮还没交：`fail`。
- 校验从「session 站必须用 `flow-*` 包装 scene」放宽为「session 站必须有 `output` 契约」。
- `flow-*` 包装 scene 与 `flow_state` / `flow_submit` / `flow_propose` 保留，作为 `system` 上的可选增强；`flow_submit` 与结构化输出殊途同归。
- **限写**：站的 `writes` 换成会话的 `permission_rules`：受限工具集（`guarded_tools`）里不被本站 `writes` 允许的，各写一条
  `{source: "session", behavior: "deny", tool_name}`。删掉 xops-01 的工具中间件 `module/guard.rs`。所有载体同一套。
- 协议层按交付方式写：结构化输出（缺省）或 `flow_submit`（包装 scene）。

### 3. 可写的流程包

```rust
pub trait Packs {
    // …已有的只读方法…
    fn writable(&self) -> bool { false }
    fn origin(&self, flow: &str) -> Origin { Origin::Builtin }       // Builtin | Override | User
    fn save_flow(&self, yaml: &str) -> Result<String /*digest*/, String> { Err("read-only".into()) }
    fn delete_flow(&self, name: &str) -> Result<(), String> { Err("read-only".into()) }
    fn save_skill(&self, name: &str, body: &str) -> Result<String, String> { Err("read-only".into()) }
    fn delete_skill(&self, name: &str) -> Result<(), String> { Err("read-only".into()) }
}
```

- `DirPacks::writable(dir)`：最上一层可写。改内置流程 = 在可写层另存一份覆盖（`origin = Override`），删掉覆盖就回到内置版本。
- 保存前整份校验，不过不存。已在跑的运行钉住了旧定义，不受影响。
- `Op` 加 `EditFlow { flow }`、`EditSkill { skill }`，经 `Authorizer`。
- 新主题 `defs`：流程列表快照（`[{name, title, digest, origin, valid}]`），保存、删除后重发。

### 4. 在流程 space 的对话里编写流程

流程 space 的**中区是对话**。space 的缺省 scene 是模块注册的 `flow-author`（包装一个应用给的 scene，缺省 Agix 的 chat scene），
它带编写流程的原生工具：

| 工具 | 只读 | 作用 |
|---|---|---|
| `flow_list` | 是 | 列出流程（名字、标题、来源、是否有效） |
| `flow_show` | 是 | **当前流程**的 YAML、用到的 skill 正文、校验结果、站图摘要 |
| `flow_schema` | 是 | 定义的 JSON Schema 与本应用的目录 |
| `flow_validate {yaml}` | 是 | 校验一份 YAML，不保存 |
| `flow_create {yaml, skills?}` | 否 | 新建流程（不需要当前流程）；成功后它成为这个会话的当前流程 |
| `flow_update {yaml?, skills?}` | 否 | 改当前流程 |
| `flow_delete` | 否 | 删当前流程（内置流程删的是覆盖） |
| `flow_skill_set {name, body}` / `flow_skill_delete {name}` | 否 | 改、删 skill |
| `flow_dry_run {inputs?, mocks?}` | 是 | 试运行当前流程到底（或到要人操作的地方），返回走过的路径；右栏能看到这条试运行 |
| `flow_start {subject, inputs?, title?}` | 否 | 正式启动当前流程 |

- 非只读工具照 Agix 的权限机制问用户（这是交互会话，用户就在对话里），流程模块不另造确认。
- **当前流程**由右栏的选择决定：界面在选择变化、切换会话时调设备方法 `focus {instance, session_id, flow | null}`，模块按会话记住。
  需要当前流程的工具在没有选择时返回：「还没有选择流程：请用户先在右边栏选择要操作的流程」，由 Agent 转告用户。
  `flow_create` 不需要选择；建成后模块把它设为这个会话的当前流程，并发主题 `focus {instance, session_id}`，右栏跟着选中。
- 编写工具**不含**确认、驳回、强制、返工、放行、取消：Agent 仍然不能代人批准。
- `flow-author` 带一段系统提示块：流程的写法要点、先 `flow_schema` 再写、改之前先 `flow_show`、先 `flow_validate` 再保存、`flow_dry_run` 验证走法。

### 5. 试运行（dry-run）

在沙箱里真跑内核，外部执行全部模拟：不建会话、不调应用的 `Programs` / `Gates` / `Tools`、不花模型费用。

- 模块另起一个 `Engine`：`MemStore` + 模拟的 `Tools`，运行号前缀 `dry-`，不进 `runs`，另有主题 `dry_runs`（`[{id, flow, status, created_at}]`），
  最多保留最近 20 条。`run {run}` 主题与 `get` 对试运行同样可用；站的人工操作（确认、驳回、强制、返工、放行、重试、认领、交付）
  对 `dry-` 运行同样可用，路由到沙箱。
- 模拟规则：
  - `session` / `program` 任务：自动以调度者身份交一份模拟输出。来源依次是 `mocks.stations.<id>`（单个值，或按任务 id 的表）、
    输出契约里的 `examples` / `default`、按契约生成的最小合法值（内核新增 `schema::minimal(&Value) -> Value`）。
  - 门：缺省通过；`mocks.gates.<gate>: false` 不通过。
  - `tool` 出口：返回 `mocks.tools.<tool>`，没给就是 `{}`。
  - 确认、`external` 任务：等人，在右栏的站详情里点，或经方法。
  - 子流程：在沙箱里开子运行。
  - 每个 session 任务记下**本来要发给会话的完整消息**（协议层 + skill + 对象），在站详情里可看。
- 方法：`dry_run.start {flow | yaml, inputs?, mocks?, subject?}` → `{run}`；`dry_run.step {run}`（处理下一项工作）；
  `dry_run.run {run}`（跑到底或到要人操作）；`dry_run.list`。

### 6. 空应用 `harnix` 自带流程模块

- `crates/harnix` 的二进制注册 `Flow`：运行记录 `FileStore` 在模块数据目录 `runs/`；流程包 `DirPacks`：一层内置（随二进制嵌入的
  示例流程，可以为空）+ 可写层（模块数据目录 `packs/`）；scene：包装 Agix 的 chat 与 coding（`flow-chat`、`flow-coding`）与
  `flow-author`。没有 `Programs` / `Gates` / `Tools`（目录里也就没有，用到它们的流程校验不过——试运行不受影响，因为模拟）。
- **不让 `harnix` 库的使用者被迫编译 `harnix-flow`**：二进制的这个依赖做成可选 feature + `required-features`，或把二进制挪进
  单独的 crate（实现定，写进 `layering.rs` 的测试）。
- 二开应用不变：`App::builder().with_builtins()` 仍然要 `.module(Flow…)` 才有流程。

### 7. 外壳内建流程 space

- `@harnix/flow` 成为外壳的内建 space：**后端注册了流程模块才出现**（判断方式由实现查清外壳启动时能拿到什么——后端声明的
  space、或 `harnix.reachable` 的方法），并受 `[shell] spaces`（id `flow`）控制。
- 布局：
  - **中区：对话**（这个 space 的会话，scene `flow-author`）。输入框上方的提示（`composerNotice`）显示当前流程；没选时提示先在右栏选择，
    但不拦着发消息（创建不需要选择）。
  - **左栏**：这个 space 的会话（缺省）。
  - **右栏（只呈现，不编辑定义）**：
    - 「流程」页：流程列表（标题、来源、是否有效）；选中的流程画成定义模式的图；它的 skill 正文只读；它的试运行列表。
    - 「运行」页：选中流程的正式运行与试运行；选一条运行就画运行模式的图，点站看站详情。站详情里的确认、驳回、强制、返工、
      放行、重试、认领、交付按钮保留——它们是对运行的操作，不是编辑定义。
    - 「参考」页：字段说明与目录。
  - 右栏选中的流程 = 中区对话的当前流程（`focus`）；Agent 建了新流程，右栏跟着选中（主题 `focus`）。
- 去掉 xops-01 里中区的总览与 testapp 的单独界面构建（`target/testapp-ui`）。`budget` 层按需调额度。

## 兼容性

- 定义、`Catalog`、`Packs`、`Op` 都只加；旧定义照样能用——但**不带 `output` 契约、又不用 `flow-*` scene 的 session 站**以前本来就过不了
  校验，不受影响。用 `flow-*` scene 的旧流程继续走 `flow_submit`。
- 删掉工具中间件后，限写改由会话的权限规则实现，效果相同（受限工具集里不被允许的写被拒）。
- 空应用多了一个流程 space 与 `app.flow.*`；profile 用 `[shell] spaces` 去掉 `flow` 即可不显示。
- 不改骨架、`Module` trait、贡献点、协议形状；`MODULE_API_VERSION` 不升。

## 验收

- **单元**：新字段的解析与校验（目录里有才查）；放宽后的 session 站规则；权限规则的生成；`schema::minimal`；`DirPacks` 可写层
  （新建、覆盖内置、删覆盖回到内置、保存前校验）；沙箱的模拟规则与步进；`focus` 记忆与「未选择」报错；编写工具的只读标记。
- **LIVE API**（录像回放）：
  - 站在进程内模板实例、spawn 模板实例上，经结构化输出交付，运行走完；
  - 限写在模板实例上生效（被拒的写有用例）；
  - 会话落在站指定的 space，配置里是站指定的模型；
  - 落点非法时任务失败，理由可读；
  - 经 `flow-author` 会话的工具新建、修改、删除流程（录像里调这些工具）；没选择时 `flow_update` 返回「请先选择」；
  - 保存后能启动；试运行不建任何会话、走完，门设为不通过时停在该站。
- **LIVE 界面**（空应用 `harnix` 二进制）：有流程 space；右栏选一条流程，对话里让 Agent 改它（录像），右栏随之更新；试运行在右栏走完；
  启动正式运行、在站详情里确认、运行完成。没有流程模块的应用（testapp 去掉流程后的某个 profile，或 `[shell] spaces` 去掉 `flow`）里没有这个 space。
- 全量 `check.sh` 含 live 层。

## 依赖

xops-01。不依赖 Agix 的任何改动。

## 来源

XOps 的维护者在 xops-01 之后要求：流程会话要能落到不同的实例与模型；Harnix 默认界面直接有流程 space，在中区对话里让 Agent 编写流程，
右栏只呈现；能试运行；调试不再依赖 testapp。设计依据：XOps `docs/flow-design.md`。

## 实现记录

### 后端一半（分支 `flow-2`，`f24ec3d`…）

外壳（§7）另做；这一半是内核、模块、空应用、testapp 与 LIVE API。

**内核**（`crates/flow/src/kernel`，`--no-default-features` 仍不链 Harnix / Agix）

- `Executor::Session` 加 `model provider instance template params space`（都可选，只加）。校验：`instance` 与 `template` 二选一；
  `params` 只配 `template`、其中的引用照常查；`Catalog` 加 `templates instances spaces models`（`serde(default)`），**填了才查**；
  `models` 里写 `model` 或 `provider/model`，只写模型名时匹配任一 `*/model`。
- session 站必须有 `output` 契约（「必须用 `flow-*` 包装 scene」的规则没了：目录的 `scenes` 现在是应用给的加上包过的）。
- `WorkItem::Task` 加 `placement: Option<Placement>`（`params` 按任务的上下文解析成字符串）、`delivery: Option<Delivery>`。
  `Placement::template_instance(run)`：模板实例名取 `params.name`，没有就是 `flow-<run>`；其余参数原样交给模板。
- `Delivery`：`structured_output`（缺省）| `flow_submit`（`Engine::with_submit_scenes` 列出的 scene，即模块的 `flow-*`）。
  协议层按它写交法；限写一句改成列出**本站实际被拒的**受限工具（`protocol::refused`）。`Engine::delivery_of(scene)`、
  `Engine::dispatched_work(rec, now)`（重启接续时按引擎的交法重建；自由函数 `dispatched_work` 保留，一律当结构化输出）。
- `protocol::refused(guarded, writes) -> [(pattern, rule_tool_name)]`：受限的名字不被 `writes` 允许就拒；分组 `ns__*` 整组拒
  （规则写 `ns`，Agix 按 `__` 段匹配），但本站 `writes` 落在组内的不拒整组（`deny` 不能带例外）；不在 `__` 边界的 `prefix*` 写不成规则，跳过。
- `Packs` 加 `writable / origin / save_flow / delete_flow / save_skill / delete_skill`（缺省只读），`Origin { builtin, override, user }`。
  包只查「能解析、名字能存」；**整份校验在 `Engine::save_flow`**（用引擎的包——模块那层带全目录——判，问题即错误、什么都不写）。
  `Engine::{save_flow, delete_flow, save_skill, delete_skill}` 先问 `Authorizer`：新 `Op::EditFlow { flow }`、`Op::EditSkill { skill }`（`run` 为 `None`）。
- `MemPacks::writable()`（内存里的可写层）；`DirPacks::writable(dir)` / `.with_writable(dir)`（最上一层，原子写 `flows/<name>.yaml`、
  `skills/<name>/SKILL.md`）、`.with_builtin(MemPacks)`（嵌入二进制的最底层，其目录并入 `catalog()`）。删覆盖回到内置；删内置报错。
- `schema::minimal(&Value)`：`examples[0]` → `default` → `const` → `enum[0]` → `oneOf/anyOf` 第一支、`allOf` 合并 → 按类型
  （对象给全部 required、数组给 `minItems` 个、字符串补到 `minLength`、几种 `format`、数的下界）；什么都没说给 `{}`。
- `dryrun`（crate 根，纯内核）：`Sandbox`，每条试运行一个 `Engine` + `MemStore`（`dry-<flow>-<n>`，子运行同库），`Mocks {stations, gates, tools}`，
  `SandboxPacks`（可给未保存的 YAML；目录收进各流程用到的 step/gate/tool/action/scene 等名字——它们都被模拟）。`start / step / run / list /
  get / engine_for / absorb / root_of / runs_of`；`KEEP = 20`。站的模拟输出：`mocks.stations.<id>`（多任务的站可给按任务 id 的表）→
  契约的 `minimal`；不合契约就 `fail` 并写明。门缺省过、`false` 不过；`tool` 出口给 `mocks.tools.<tool>` 或 `{}`；确认与 external 等人。

**模块**（`crates/flow/src/module`）

- 落点（`dispatch::place`）：space = 站的或模块的；`template` → `cx.instances().ensure_ready(template, name, params, ensure_ms)`（缺省 60 s），
  实例归 `module:flow`；`instance` 照给；都没有时在模块 space 里用 `system`，在别的 space 里由中枢取那个 space 的缺省实例。
  `SessionSpec` 带 `space instance scene model provider recording`、`permission_rules`（`permission_rules(guarded, writes)`：
  `{source:"session", behavior:"deny", tool_name}`），结构化输出交付时带 `output_schema`（`flow-*` 不带，旧录像照旧走 `flow_submit`）。
  落点非法（模板不存在、超时、`SPACE_MISMATCH`、模型不可用）→ 本次 `fail`，理由原样（例：`template flow.nowhere gave no instance
  `flow-…`: there is no template `flow.nowhere``），按 `retry` 处理。
- 交付：每轮结算读 `outcome.structured_output`，有就以调度者身份 `submit`（无论哪种交法）；契约不过（理论上 Agix 已退回）或交了但站
  因**失败的**检查（如 `tool` 出口）未满足，把问题发给会话再交，最多 2 次；一轮什么都没交提醒一次（文案按交法），再一轮还没交 `fail`。
- 删了 `module/guard.rs` 与 `customize_session`；`DEFAULT_GUARDED`、`matches_any` 挪到模块 / 内核协议层，公开名不变。
- `Flow::builder()` 加 `.packs_with(|data_dir| …)`、`.author_scene(scene)`（缺省 Agix chat）、`.template(InstanceTemplate)`
  （模块自己的模板，站里写 `flow.<id>`——中枢只许模块用自己的和 profile 的模板）、`.ensure_ms(ms)`。
- `tools::StructuredScene`（id `structured-<base>`）：把 `StructuredOutput` 加进白名单的包装，不持模块状态，可放进模板的 `native`
  （spawn 的 daemon 在自己进程里再建一遍）。**原因**：Agix 只在 scene 允许 `StructuredOutput` 时把它给带 `output_schema` 的会话，
  而 chat scene 的白名单里没有它（coding 有，但要项目）。
- `flow-author`（`module/author.rs`）：包应用给的 scene，加系统提示块 `AUTHOR_PROMPT` 与 11 个工具（只读：`flow_list flow_show
  flow_schema flow_validate flow_dry_run`；非只读：`flow_create flow_update flow_delete flow_skill_set flow_skill_delete flow_start`）。
  非只读的工具实现 `check_permissions` 返回 `ask`——Agix 原生工具缺省 `allow`，与 `is_read_only` 无关，不改就不会问。
  `flow_create/flow_update` 先把 YAML 连同要一起存的 skills 整份校验，有问题只返回问题、什么都不写；`flow_update` 的 YAML 必须同名。
  没有当前流程时返回 `NO_FOCUS`（含「请用户先在右边栏选择」）。不自动起名（回放下那是一次没录的模型调用）。
- 流程 space 的 `SpaceDecl` 缺省 scene 改为 `flow-author`；`start` 激活包过的 scene 与 `flow-author`、建沙箱、发 `defs` 与 `dry_runs`。

**方法与主题**（`app.flow.*`，都只给 `Device`）

| 方法 | 参数 → 结果 |
|---|---|
| `save` | `{yaml}` → `{name, digest, origin}`；校验不过是 `INVALID_PARAMS`，`message` 里列出全部问题 |
| `delete` | `{name}` → `{name, origin}`（`origin` 是删后这个名字下的流程的来源，没有了是 `null`）；删内置报错 |
| `skill.save` / `skill.delete` | `{name, body}` → `{name, digest}` / `{name}` → `{name}` |
| `focus` | `{instance, session_id, flow \| null}` → `{flow}`；不存在的流程 `not_found` |
| `dry_run.start` | `{flow \| yaml, inputs?, mocks?: {stations?, gates?, tools?}, subject?, title?}` → `{run}`（只排队，不走） |
| `dry_run.step` / `dry_run.run` | `{run}` → `{run, steps: [{kind, run, station?, task?, gate?, result}], pending, status}` |
| `dry_run.list` | → `[{id, flow, status, created_at}]`，新的在前 |
| `defs`（改） | → `[{name, title, digest, origin, valid}]` |
| `def`（改） | 多了 `origin`、`valid`、`problems`、`writable`；`protocol` 按各站的交法写 |
| `catalog`（改） | 多了 `templates instances spaces models writable` |
| `get` / `history` / `claim` / `heartbeat` / `submit` / `fail` / `retry` / `confirm` / `force` / `rework` / `release` / `cancel` | `run` 以 `dry-` 开头时进沙箱；动作产生的工作在沙箱里排队，试运行被 `dry_run.run` 驱动过就接着走到下一个停处 |

| 主题 | 参数 | 值 |
|---|---|---|
| `defs` | — | 同 `defs` 方法；启动、保存、删除（含 skill）后重发 |
| `focus` | `{instance, session_id}` | `{flow: string \| null}`；`focus` 方法、`flow_create`、删掉当前流程时发 |
| `dry_runs` | — | 同 `dry_run.list` |
| `run`（改） | `{run}` | 试运行的 `RunView` 也在这里；其 session 任务带 `preview: {round, attempt, scene, message, placement, delivery, output_schema, session_key}`（`message` 是本来要发给会话的完整首条消息：协议层 + skill） |

外壳怎么知道后端有流程模块：外壳启动时已经调 `harnix.reachable`（`app.snapshot.reachable`），**有 `app.flow.defs` 即有**；流程 space 的 id
（缺省 `flow`，应用可改）从 `harnix.spaces.list` 里 `scene === "flow-author"` 的那项取。hello 快照里 `engine.active_scenes` 也会有
`flow-author`，但那是间接的。

**空应用**：二进制挪进新 crate `crates/cli`（`harnix-cli`，`[[bin]] name = "harnix"`），注册 `Flow`：`FileStore`（缺省，模块数据目录 `runs/`）、
`DirPacks::writable(<数据目录>/packs).with_builtin(…)`（内置示例 `example-review`：session 站 + 确认，skill `review`，在 `crates/cli/packs/`），
`flow-chat`、`flow-coding`、`flow-author`。`crates/harnix` 只剩库、不依赖 `harnix-flow` / `scene`（`layering.rs` 新测试
`the_library_does_not_carry_the_empty_applications_flow_module`）。`check.sh` 改建 `-p harnix-cli`，testkit 建 `harnix` 时用 `-p harnix-cli`；
`examples/hello-app/Cargo.lock` 多了这个成员。

**testapp**：流程模块带两个模板 `flow.local`（进程内）、`flow.remote`（spawn），引擎都激活 `structured-chat`（`native` 贡献）；包换成
`DirPacks::writable(<数据目录>/packs).with_builtin(…)`；新演示 `demo-placed`（`fleet` space + 自己的模型 + 进程内模板；spawn 模板）、
`demo-guarded`（模板实例上写 `TodoWrite` 被拒）、`demo-misplaced`（模板不存在）。去掉 `target/testapp-ui` 的构建；`ui/flow.spec.ts` 整体 `fixme`，等 §7。

**录像**（手工，形状照 `flow-analyze`，多轮之间放填充调用）：`flow-local`、`flow-remote`（`StructuredOutput` 一次）、`flow-guarded`（`TodoWrite` →
`StructuredOutput {done: true}`）、`flow-author-unfocused`（`flow_update`）、`flow-author-edit`（三轮：`flow_schema`、`flow_validate`、`flow_create` ·
`flow_show`、`flow_update`、`flow_dry_run` · `flow_delete`）。

**测试**

- 单元：`tests/validate.rs`（放宽后的规则、落点字段与目录、组合与引用）、`tests/authoring.rs`（落点与交法、`MemPacks`/`DirPacks` 可写层、
  `Engine::save_flow` 的校验与授权、试运行的模拟规则与步进、保留 20 条）、`kernel::schema`、`kernel::protocol`（`refused`、结构化输出文案）、
  模块（权限规则生成、目录、懒加载的包、`focus` 记忆、编写工具的只读标记、`StructuredScene`）、`crates/cli`（示例流程有效）。
- LIVE API：`flow-placement.spec.ts`（进程内与 spawn 模板实例经结构化输出走完、在 `fleet` space、模型 `flow-demo-model` 经 `app.fleet.model`
  读回；模板实例上 `TodoWrite` 得到 `Denied by permission: denied by rule: TodoWrite`；落点非法任务失败、理由可读）、`flow-authoring.spec.ts`
  （未选择时 `flow_update` 返回「请先选择」；对话里建、看、改、试运行、删，`focus` 与 `defs` 主题跟着变，非只读工具都被问过；`save` 后
  `start`、覆盖与恢复；试运行不建会话、走到人、经同样的方法确认 / 认领 / 交付后结束、`preview` 可见；门 `false` 停在该站；未保存的
  YAML 也能试运行）、`flow-empty-app.spec.ts`（`harnix` 二进制：方法、space、示例、目录、试运行、覆盖）。`flow.spec.ts` 按新目录与新演示改两处。
- 跑过：`cargo test -p harnix -p harnix-cli -p harnix-flow -p harnix-testapp`、`-p harnix-flow --no-default-features`、这些 crate 的 clippy
  `-D warnings`；`crate-metadata` 等离线层；`pnpm -r typecheck` 与 `test`；LIVE `api` 全部（两种拓扑各 236 过、15 跳过）；`ui` 全部一次（75 过、3 跳过）。
  全量 `check.sh` 未跑（等外壳一半）。

偏离与留下的：

- 试运行只在内存里，重启就没了；`preview` 写在发布出去的视图里，不进 `RunRecord`。
- 交了结构化输出、但那一轮在重启时还没结算的任务，重启后按「没交」提醒一次（回放下没有录像，会失败后按 `retry` 重派）。
- 组内部分开放的受限分组不拒整组（见上），协议层如实只列出被拒的。
- `schema::minimal` 给数组的是 `minItems` 个（缺省 0）：`foreach` 站在试运行里不给 `mocks` 就没有任务、直接通过。

### 外壳一半（分支 `flow-2`）

**外壳怎么知道有流程模块**（`ui/packages/shell/src/flow.ts`）：`registerBuiltins` 改成异步、`App.boot` 等它注册完再找落地的 space。
`harnix.reachable` 里有 `app.flow.defs` 才有；space 的 id 取 `harnix.spaces.list` 里 `scene === "flow-author"` 的那项（缺省 `flow`，
列表取不到时按 `flow`，列表里没有这样的 space 就不注册）；`[shell] spaces` 里写的 `flow` 指这个内建 space（不论后端 space 叫什么）。

**唯一的后端改动**：`[shell] spaces` 原来只认 `sessions projects system`，写 `flow` 进程起不来、缺省也不含它——外壳因此永远
`keeps("flow") == false`。`BUILTIN_SPACES` 加 `flow`（`crates/instance/src/profile.rs`，缺省全留；单测补一句）。只影响界面：
中枢不按它撤任何方法。显式列了内建 space 的 profile（没写 `flow`）从此不显示流程 space，这正是 §7 要的去掉方式。

**布局**（`registerFlowSpace`，`@harnix/flow`；`SpaceContribution` 用到的字段）：

- `center: "chat"`，不给 `sidebar`（左栏是这个 space 的会话）、不给 `newSession`（可新建）、不给 `overview`；会话的 scene 由后端
  space 声明（`flow-author`），外壳建会话时不指定。
- `composerNotice(session)`：`当前流程：<标题> <name>`，取该会话的 `focus`（主题，未到时取右栏选择）；没有就提示先在右栏选、
  新建不需要选。不拦发送。`notice()`（新对话首页）同义，`placeholder()` 换成编写流程的说法。
- `tabs`：`<space>.flows`「流程」、`<space>.runs`「运行」、`<space>.reference`「参考」（`align: "end"`）；`details: "open"`、
  `detailsWidth: 560`。
  - 流程：列表（标题、来源徽标 内置 / 已改 / 自建、无效徽标；`defs` 主题，主题未到时用方法），选中流程的定义图（点站看站的定义，只读）、
    `problems`、skill 正文（只读）、它的试运行（`dry_runs` 主题）与「试运行」按钮（`dry_run.start {flow}`，开出来的显示在运行页）。
    列表的 `digest` 变了就重读 `def`（对话里改过）。
  - 运行：选中流程的正式运行（`runs`）与试运行合在一个列表（试运行带徽标）；「启动运行」（`StartForm`，预选当前流程）、「试运行」；
    选一条画运行图（`run {run}`，没值时 `get`）、头部的状态与取消；试运行多一排「单步 / 跑到底」（`dry_run.step` / `dry_run.run`，
    显示这次走了几步、还排几项）；点站看 `StationPanel`（确认、驳回、强制、返工、放行、重试保留；**新加**：`external` 任务 `ready`
    时「交付…」——`claim` 再带 token `submit`；试运行的 session 任务显示 `preview`：scene、交法与 `message` 全文）。
  - 参考：`FlowReference`（字段、目录——目录多了 templates / instances / spaces / models），校验框保留但只校验、不保存，并写明。
- 去掉了 xops-01 的中区总览、运行头部（挪进运行页）与左栏的 `RunList`（组件仍导出给自己编外壳的应用）。

**选择 ↔ focus**（`FlowSpaceState`，`ui/packages/flow/src/state.ts`）：选流程即对本 space 打开的会话调 `focus`；`host.session.onChange`
换到本 space 的另一个会话时，有选择就把选择发过去，没选择就接过那个会话记着的；订阅当前会话的 `focus {instance, session_id}`，
Agent 建的流程被选中、删掉的清掉选择；自己那次 `focus` 还没回时到的旧值不跟。别的 space 的会话不碰。`registerFlowSpace({host})`
时立刻接上，否则 space 的任一部分第一次画出来时接上。

**测试**

- 单元：`flow/test/space.test.tsx`（选择即 focus：选时发、换会话再发、别的 space 不发、跟主题、接过会话自己的、未回不跟旧值；
  输入框上方的提示两种文案；流程页列表 / 徽标 / 图 / skill / 试运行列表 / 不调任何写方法、digest 变就重读；运行页列表合并与过滤、
  试运行有控制而正式运行没有、点站看站；单步与跑到底的调用与结果、结束后禁用；`preview` 与 external 交付），`components.test.tsx`
  改了注册断言；`shell/test/flow-space.test.ts`（探测、改名、取不到列表、`[shell] spaces` 去掉、没有模块）。`tests/style-lint.mjs`
  的扫描加上 `ui/packages/flow`。
- LIVE 界面 `tests/live/ui/flow.spec.ts`（空应用 `harnix`、缺省 `ui/dist`、回放）6 条：有流程 space、列出 `example-review`、选中画图、
  新对话的提示写它、`focus` 主题是它、参考页；对话里让 Agent 改（新录像 `flow-ui-edit`：`flow_show` → `flow_update` → 一句话，
  在界面上允许权限卡），右栏列表、标题、digest、提示都跟着变、来源变「已改」；没选择时（录像 `flow-author-unfocused`）Agent 转告「请先选择」；
  运行页试运行：单步、跑到底、停在确认、看 `preview`、确认后结束、流程页的试运行列表里也有；正式运行：启动、在列表与图里、session 站
  （空应用没给录像名，回放下会失败成 `unmet`）在站详情里强制通过、确认、运行完成；`[shell] spaces` 去掉 `flow` 的 profile 没有这个 space
  （模块仍在）。没有流程模块的后端：`examples/hello-app`，`ui/spaces.spec.ts` 本来就断言它的 space 只有 `sessions projects greeter notes`。
- `coverage.md` 重生成（flow：API 19、界面 6）。`budget`：界面 gzip 166 KB → 186 KB（额度 300 KB），不调。`check.sh` 不用改
  （`target/testapp-ui` 后端一半已去掉）。
