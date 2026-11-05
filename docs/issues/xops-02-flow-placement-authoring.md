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
