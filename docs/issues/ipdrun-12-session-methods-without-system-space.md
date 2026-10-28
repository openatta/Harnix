# ipdrun-12：profile 去掉系统 space 后，输入框不能换模型

类型：新能力｜优先级：中

## 现象

产品 profile 写 `[shell] spaces = []`（或不含 `system`）时，内建的 `SYSTEM` 部分整块不注册
（`crates/builtin/src/lib.rs:138-165` 的 `SYSTEM`，`:210-213` 的 `modules(system_space)`）：`harnix.system.*` 不进授权表，也就不在
`harnix.reachable` 里。这是对的——系统 space 是调试面，能看任何实例的会话、改写消息、读信封。

但换模型只有这一块里的 `harnix.system.model`（`crates/builtin/src/system.rs:31-38`，对应引擎的 `session.setModel`）。外壳的输入框只在
`reachable.includes("harnix.system.model")` 时才给 `onModel`（`ui/packages/shell/src/Shell.tsx:241`、`:298`、`:531`、`:616`）；
去掉系统 space 后，模型菜单只显示当前模型、不能换。模块可以 `grant` 引擎方法再开自己的 `app.<id>.*` 替人调，但外壳的输入框写死走
`harnix.system.model`，接不上。

改名已经有一个不随系统 space 走的 `harnix.sessions.rename`（`lib.rs:82`）。压缩、清空不需要另开方法：引擎的 `/compact`、`/clear`
经 `hello.commands` 进输入框的 `/` 菜单（`ui/packages/shell/src/builtins.tsx:102-114`），不依赖系统 space。

## 建议

profile 加一个开关，**缺省关**：

```toml
[shell]
model_switch = true   # 缺省 false
```

打开时：

1. 内建部分注册 `harnix.sessions.setModel { instance, session_id, model, provider? }`（授权 `session.setModel`），作用域是调用方声称所在的
   space 的会话，校验与 `harnix.sessions.rename` 相同；不能跨 space，跨 space 的仍只在 `harnix.system.*` 里。
2. 外壳的输入框 `reachable` 里有 `harnix.sessions.setModel` 时用它，没有时照旧看 `harnix.system.model`。

`harnix.system.model` 不动；保留系统 space 的 profile 不写这个开关，行为与现在一样。

## 兼容性

- 不写 `model_switch` 的 profile（Helm 的 `profiles/helm.toml` 是 `spaces = []`、不写它）：授权表、`harnix.reachable`、模型菜单都与现在一样。
- `ShellProfile` 多一个带缺省值的字段；`deny_unknown_fields` 下旧 profile 照读。
- 授权表在开关打开时多一个设备方法；方法目录（`catalog_doc`）随之更新。协议版本不必升。

## 验收

- `tests/live/api/sessions.spec.ts`：`spaces = []`、`model_switch = true` 的 profile 下，`harnix.reachable` 含 `harnix.sessions.setModel`、不含
  `harnix.system.model`；`setModel` 之后 `session.get` 报新模型。
- 声称 space A、操作 space B 的会话：`SPACE_MISMATCH`（-31003）。
- `spaces = []`、不写 `model_switch`：`harnix.reachable` 不含 `harnix.sessions.setModel`（缺省不变的回归）。
- `tests/live/ui/conversation.spec.ts`：开关打开的 profile 下，输入框的模型菜单可选，选了以后页脚显示新模型；不打开时只显示当前模型。

## 依赖

无。

## 来源

IPDrun 的产品 profile 不带系统 space，发现输入框不能换模型；会话里的压缩、清空它用自己的方法绕过去了，模型菜单绕不过去。
