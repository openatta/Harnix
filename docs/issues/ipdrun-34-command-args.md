# ipdrun-34：`/` 命令拿不到命令名后面输入的文字

类型：新能力｜优先级：低

## 需求

`/` 菜单总是用空查询问各个 `command` 贡献（`ui/packages/shell/src/Shell.tsx:817`，`c.complete("", ctx)`），选中带 `run` 的候选即执行，
输入框里已打的 `/xxx` 被清掉（`CommandCandidate.run(ctx)`，`ui/packages/react/src/contrib.ts:63-65`）。用户在 `/xxx ` 后面接着打的文字传不进去，
`/ask <问题>`、`/find <关键词>` 这类带参数的命令只能选中后另弹对话框再填一遍。

## 建议

```ts
interface CommandCandidate {
  // …原有成员不变
  /** args：命令名之后输入框里已有的文字（去掉首尾空白；没有则为 ""）。旧的 run(ctx) 照样能用。 */
  run?(ctx: CommandContext, args: string): void | Promise<void>;
}
```

- 输入框内容是 `/<name> <其余>` 且回车（或在菜单里选中该命令）时，外壳调 `run(ctx, "<其余>")`，然后清掉输入框。
- 可选：`/` 菜单也把 `/` 后已输入的查询传给 `complete(query, ctx)`（与 `@` 一致，`Shell.tsx:874`），候选可以按参数给提示。

## 兼容性

- 纯新增：多一个实参，只声明 `run(ctx)` 的贡献不受影响；没有 `run` 的候选仍是插入文字。
- 贡献点 API 版本可以不升。

## 验收

- `ui/packages/shell` 的单元测试：注册 `/echo`，输入 `/echo hello world` 回车 ⇒ `run` 收到 `args === "hello world"`，输入框清空；只打 `/echo` 选中 ⇒ `args === ""`。
- `tests/live/ui/interface.spec.ts`：testapp 的 `/echo` 把参数写进一个可见元素。

## 依赖

无；宜与 ipdrun-29（`run` 与 `hideCommands`，已落地）一起看。

## 来源

IPDrun 想在类目会话里给「`/ask <问题>`」「`/goto <编号>`」这类命令，现在只能选中后再弹框输入。
