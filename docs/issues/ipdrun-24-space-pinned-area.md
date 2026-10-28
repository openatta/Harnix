# ipdrun-24：space 不能在转录顶部放固定的内容

类型：新能力｜优先级：中

## 需求

一段对话常有一件「始终要看见」的东西：这个会话正在等人回答的问题、当前的目标、一张需要人批准的卡。`ChatPanel` 有 `pinned?: ReactNode`
（`ui/packages/react/src/chat/ChatPanel.tsx:73`，画在消息流顶上，`:313-316`，`data-testid="conv-pinned"`），外壳没有把它开给 space
（`ui/packages/shell/src/Shell.tsx:598-620`）。space 只能把这件东西写进流里（滚走了就看不见），或在右栏另放一份。

## 建议

```ts
export interface SpaceContribution {
  // …原有成员不变
  /** 固定在当前会话的消息流顶上的内容；没有就返回 null / undefined。每次画对话时读。 */
  pinned?(session: SessionRef): ReactNode;
}
```

- 外壳把结果传给 `ChatPanel` 的 `pinned`，包在 `Contributed` 里（`contrib.ts` 已有的失败隔离），抛错只丢这一块。
- 内容变化由 space 自己的 React 状态驱动（它返回的是组件）；`ipdrun-19` 的 `redraw()` 用于「有没有」的变化。

## 兼容性

- 不给 `pinned` 的 space：没有变化。
- 贡献点 API 版本：可选字段，可以不升。
- Helm、hello-app 不受影响。

## 验收

- `tests/live/ui/conversation.spec.ts`：testapp 的 space 对某个会话返回一个带按钮的块；打开它，`conv-pinned` 在消息流上方、滚动消息流时它不动；
  点按钮能调到 space 自己的方法。别的会话没有 `conv-pinned`。
- `pinned` 抛错：对话照常画，没有固定区，诊断页记一条。

## 依赖

无。

## 来源

IPDrun 的一类会话里有一张等人拍板的卡，要始终在顶上；现在卡写在流里，右栏再固定一份。

## 实现记录

外壳（F3b）：

- `SpaceContribution.pinned?(session)`，结果包在 `Contributed` 里给 `ChatPanel.pinned`；函数本身抛错就没有固定区，返回的组件画的时候抛错只丢这一块
  （外壳样式 `#main .hx-pinned:empty` 不留空带）。
- 测试：`tests/live/ui/host.spec.ts` 的「pinned: a session's block stays above the flow and calls its space; other sessions have none」（固定块在第一条消息之上，
  按钮调到 space 自己的函数；别的会话没有 `conv-pinned`）。没有单测「滚动时不动」：`ChatPanel` 里固定区本来就在消息流的滚动区之外。
