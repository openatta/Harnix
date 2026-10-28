# ipdrun-23：space 不能让中区的对话只读

类型：新能力｜优先级：中

## 需求

有些会话只该看、不该再说话：它所对应的东西已经结束、归档、被合进别处，或当前的人没有发言权。`@harnix/react` 的 `ChatPanel` 早有
`readOnly?: ReactNode | boolean`（`ui/packages/react/src/chat/ChatPanel.tsx:95`，`:298`、`:343-348` 把输入框换成一句说明），
但外壳画对话时不从 space 取它（`ui/packages/shell/src/Shell.tsx:598-620` 的 `<ChatPanel …>` 没有这个属性）。

## 建议

`SpaceContribution` 加一个可选函数，外壳每次画对话时读，和 `placeholder()` 一样：

```ts
export interface SpaceContribution {
  // …原有成员不变
  /** 这个会话只读时返回说明文字（换掉输入框）；可以写时返回 false / undefined。 */
  readOnly?(session: SessionRef): string | false | undefined;
}
```

- 外壳把结果传给 `ChatPanel` 的 `readOnly`；`session` 是当前打开的那个，space 可以按会话判断。
- 只读只是界面：服务端照旧接受 `harnix.send`。需要强制的应用在后端自己拒（例如 `ipdrun-13` 的中间件），文档写明这一点（这是**防误**，不是安全边界）。
- 只读的会话里，快捷回复、建议、`/` 命令都不出现（`ChatPanel` 现在的只读行为）。

## 兼容性

- 不给 `readOnly` 的 space：没有变化。
- 贡献点 API 版本：可选字段，可以不升。
- Helm、hello-app 不受影响。

## 验收

- `tests/live/ui/conversation.spec.ts`：testapp 的 space 对名字以 `ro-` 开头的会话返回「已归档」；打开这样的会话，输入框不在、显示「已归档」；
  打开普通会话，输入框照常。
- `readOnly` 抛错：按可写处理，诊断页记一条。

## 依赖

无。

## 来源

IPDrun 里已经结束（合并、拆分、删除）的那一项，它的会话应当可看不可说；现在只能任其可写。

## 实现记录

外壳（F3b）：

- `SpaceContribution.readOnly?(session)`，外壳每次画对话时读、结果给 `ChatPanel.readOnly`；抛错按可写处理、记一条失败（`ask`）。只是界面，服务端照旧收 `harnix.send`。
- 测试：`tests/live/ui/host.spec.ts` 的「readOnly: a session the space calls read-only has no composer, only the words」；抛错的规则由
  `host.test.ts` 的「give the fallback when they throw…」覆盖。偏离：按会话 id 判断，不按名字前缀 `ro-`（新会话没有名字）。
