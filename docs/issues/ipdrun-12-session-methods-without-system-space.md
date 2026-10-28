# ipdrun-12：profile 去掉系统 space 后，输入框不能换模型，也没有会话级的压缩、清空

类型：缺陷｜优先级：中

## 现象

产品 profile 写 `[shell] spaces = []`（或不含 `system`）时，内建的 `SYSTEM` 部分整块不注册
（`crates/builtin/src/lib.rs:138-165` 的 `SYSTEM`，`:210-213` 的 `modules(system_space)`）：`harnix.system.*` 不进授权表，也就不在
`harnix.reachable` 里。这是对的——系统 space 是调试面，能看任何实例的会话、改写消息、读信封。

但这一块里还捎带着三个**只作用于调用方自己那个会话**的方法：`harnix.system.model`、`harnix.system.compact`、`harnix.system.clear`
（`crates/builtin/src/system.rs:31-38`，对应引擎的 `session.setModel` / `compact` / `clear`）。外壳的输入框只在
`reachable.includes("harnix.system.model")` 时才给 `onModel`（`ui/packages/shell/src/Shell.tsx:241`、`:298`、`:531`、`:616`）；
去掉系统 space 后，模型菜单只显示当前模型、不能换。

改名已经有一个不随系统 space 走的 `harnix.sessions.rename`（`lib.rs:82`），换模型、压缩、清空没有。模块可以 `grant` 引擎方法再开自己的
`app.<id>.*` 替人调，但外壳的输入框写死走 `harnix.system.model`，接不上。

## 建议

1. 在 `harnix.sessions.rename` 所在的那个内建部分加三个方法，作用域是调用方声称所在的 space 的会话（与 `harnix.sessions.rename` 相同的校验）：
   ```
   harnix.sessions.setModel { instance, session_id, model, provider? }
   harnix.sessions.compact  { instance, session_id }
   harnix.sessions.clear    { instance, session_id }
   ```
   它们不能跨 space、不能读信封；跨实例、跨 space 的那些仍只在 `harnix.system.*` 里。
2. 外壳的输入框优先用 `harnix.sessions.setModel`，`reachable` 里没有时再退回 `harnix.system.model`。
3. `harnix.system.model` / `compact` / `clear` 保留不动（系统 space 里继续用）。

## 兼容性

- 保留系统 space 的 profile：多了三个方法，外壳换成新方法后行为一样。
- 去掉系统 space 的 profile（Helm 的 `profiles/helm.toml` 就是 `spaces = []`）：输入框的模型菜单恢复可用。这对 Helm 是一处可见的变化。
  若要严格保持现状，可以加一个 profile 开关（例如 `[shell] session_methods = true`）控制这三个方法，缺省关；
  缺省开还是关交给维护者定。
- 授权表多三个设备方法；方法目录（`catalog_doc`）随之更新。协议版本不必升。

## 验收

- `tests/live/api/sessions.spec.ts`：`spaces = []` 的 profile 下，`harnix.reachable` 含 `harnix.sessions.setModel`、不含 `harnix.system.model`；
  `setModel` 之后 `session.get` 报新模型；`compact`、`clear` 各一条。
- 声称 space A、操作 space B 的会话：`SPACE_MISMATCH`（-31003）。
- `tests/live/ui/conversation.spec.ts`：同一个 profile 下，输入框的模型菜单可选，选了以后页脚显示新模型。

## 依赖

无。

## 来源

IPDrun 的产品 profile 不带系统 space，发现输入框不能换模型；会话里的压缩、清空它用自己的方法绕过去了，模型菜单绕不过去。
