# ipdrun-29：`/` 命令只能插入文字，space 也藏不掉引擎的命令

类型：新能力｜优先级：低

## 需求

`command` 贡献点的候选是 `{replacement, label, description?, source?}`（`ui/packages/react/src/contrib.ts:54-59`、`:117`）：选中后把
`replacement` 插进输入框（`ui/packages/react/src/chat/ChatComposer.tsx:271-280`），随消息作为文本发给引擎。引擎的命令（`/compact`、`/clear`…）
也是这样来的（`ui/packages/shell/src/builtins.tsx:102-114`，读 `hello.commands`）。外壳汇总时只留下名字和说明（`ui/packages/shell/src/Shell.tsx:652-683`）。

两件事做不到：

1. **选中就执行。** space 想提供一个命令，选中后直接调自己的方法（打开一个对话框、调一次 `app.<id>.*`），而不是发一段文字给模型。
2. **在自己的 space 里藏掉某个引擎命令。** 有的 space 对「清空」「压缩」有自己的语义（例如清空前要先保存什么），引擎原样的 `/clear` 在这里会做错事；
   space 只能让它照常出现。

另外 `command` 是全局的：贡献不知道前台是哪个 space，没法只在自己的 space 里给候选。

## 建议

都是可选的新增：

```ts
export interface CommandCandidate {
  replacement: string;
  label: string;
  description?: string;
  source?: string;
  /** 给了就选中即执行：不插入文字、不发送；输入框里已打的 `/xxx` 清掉。 */
  run?(ctx: { space: string; session: SessionRef | null }): void | Promise<void>;
}

export interface Contributions {
  command: {
    id: string;
    // 多一个可选的 ctx；只收 query 的旧实现照常工作
    complete(query: string, ctx?: { space: string; session: SessionRef | null }): CommandCandidate[] | Promise<CommandCandidate[]>;
  };
}

export interface SpaceContribution {
  // …原有成员不变
  /** 在这个 space 的输入框里不出现的命令名（不带 `/`），例如 ["clear"]。 */
  hideCommands?: string[];
}
```

- 外壳汇总时把 `run` 一起带上（`ChatCommand` 加可选的 `run`），`ChatComposer` 选中带 `run` 的候选时调它；`run` 抛错时用外壳的错误提示显示。
- 外壳现在只在注册表或 `hello` 变化时调一次 `complete("")`、再在本地过滤（`Shell.tsx:652-683` 的 `useCommands`）；给了 `ctx` 以后，
  前台 space 或打开的会话变了也要重问一次，不然 `ctx` 是旧的。
- 同名去重（`useCommands` 的 `seen`）保持先到先得；space 想用自己的同名命令替掉引擎的，配合 `hideCommands` 把引擎那条藏掉。
- `hideCommands` 只影响前台 space 的菜单；人手打 `/clear` 发出去，照旧作为文本到引擎（这是**防误**，不是禁止）。要真正禁止，用 Agix 的命令配置或 `ipdrun-13` 的中间件。

## 兼容性

- 候选不带 `run`：与现在一样插入文字。
- `complete` 的第二个参数是可选的，现有贡献不改代码。
- 不给 `hideCommands` 的 space：菜单与现在一样。
- 贡献点 API 版本：只加可选成员，可以不升。Helm、hello-app 不受影响。

## 验收

- `ui/packages/react/test/chat-panel.test.tsx`：带 `run` 的命令被选中时 `run` 被调一次、输入框清空、没有发送；不带 `run` 的照旧插入 `replacement`。
- `tests/live/ui/conversation.spec.ts`：testapp 注册一个 `run` 命令（调一个 testapp 方法并记下调用）；在 space A（`hideCommands: ["clear"]`）里打 `/`，
  菜单里有这个命令、没有 `/clear`；在别的 space 里 `/clear` 照常出现。
- `complete(query, ctx)` 收到的 `ctx.space` 是前台 space。

## 依赖

无。

## 来源

IPDrun 的一类会话的「清空」是「以摘要重建」，和引擎的 `/clear` 不同；现在 `/clear` 仍是引擎的，IPDrun 在右栏另放了「压缩 / 清空」按钮。
