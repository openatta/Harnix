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

## 实现记录

后端（F3a）：

- `ShellProfile.model_switch`（缺省 false）。打开时内建多注册一个模块 `model`（`crates/builtin/src/lib.rs` 的 `MODEL`，授权 `session.setModel`），方法
  `harnix.sessions.setModel {instance?, session_id, model, provider?, space?}`（`crates/builtin/src/system.rs`）：先以声称的 space（缺省 `sessions`）
  调一次 `session.get`，由中枢的 space 校验拒绝别的 space 的会话（`SPACE_MISMATCH`，`data` 带归属）；声称 `system` 直接 `SPACE_MISMATCH`，跨 space 仍只在
  `harnix.system.*`。再调引擎的 `session.setModel`。`Builtin::modules(system_space)` 不变，新增 `modules_with(system_space, model_switch)`。
- 测试：Rust `profile::tests::the_shell_keeps_every_built_in_space_unless_told`（加了 `model_switch` 的缺省与读取）；`tests/live/api/model-switch.spec.ts` 的
  「with model_switch, a space changes its own session's model」（`reachable` 有 `setModel`、无 `harnix.system.model`；换完 `inspectContext` 报新模型——
  `session.get` 不带模型字段，验收里那一句改用它）、「claiming one space for another space's session is refused」、「without the line, nothing is added」。

外壳部分待做：`ui/packages/shell/src/Shell.tsx` 里 `SET_MODEL`（`:298`）的两处用法（`:241`、`:531`）改为 `reachable` 有 `harnix.sessions.setModel` 时用它
（参数 `{instance, session_id, model, provider?, space: <当前 space>}`），没有时照旧看 `harnix.system.model`；界面用例 `tests/live/ui/conversation.spec.ts`
（开关打开时模型菜单可选、页脚显示新模型；不打开时只显示当前模型）。

外壳（F3b）：

- `ui/packages/shell/src/Shell.tsx` 的 `setModelMethod` / `setModel`：`reachable` 有 `harnix.sessions.setModel` 时用它（参数带 `space: app.claim()`），
  否则照旧看 `harnix.system.model`，两个都没有时模型菜单只显示当前模型。首屏（新会话先建再换）与打开的会话两处都改。
- 测试：`tests/live/ui/host.spec.ts` 的「model_switch: without the system space the composer changes the model through harnix.sessions.setModel」
  （profile `spaces = ["sessions"]`、`model_switch = true`：`reachable` 有 `setModel`、无 `harnix.system.model`，菜单可换、底栏显示新模型；不写开关时菜单里没有自填项）。
  偏离：放在新文件 `host.spec.ts` 而不是 `conversation.spec.ts`，用自己起的两个后端；`spaces = []` 时外壳没有可对话的 space，所以留 `sessions`。
