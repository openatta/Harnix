# ipdrun-26：space 不能让对话正文里的某种模式变成可点的 chip

类型：新能力｜优先级：中

## 需求

应用的对话里常出现它自己的标识：工单号、订单号、文件里的锚点、某种 `ABC-123` 形式的编号。人希望在回复正文里点它就打开对应的东西。
`@harnix/react` 的 `ChatPanel` 早就支持（`ids?: ChatIds`，`ui/packages/react/src/chat/ChatPanel.tsx:135`、`:190`；
`ChatIds = { pattern: RegExp; onOpen?(id); canOpen?(id) }`，`ui/packages/react/src/chat/types.ts:123-129`），
但外壳画对话时不从 space 取它（`ui/packages/shell/src/Shell.tsx:598-620`）。space 只能在自己画的卡片里做可点的编号，模型回复的正文里做不到。

## 建议

```ts
export interface SpaceContribution {
  // …原有成员不变
  /** 正文里哪些文字是可点的标识，以及点了以后做什么。每次画对话时读。 */
  ids?(): ChatIds | undefined;
}
```

- 外壳把结果传给 `ChatPanel` 的 `ids`。`pattern` 必须带 `g` 标志（`ChatIds` 的现有约定），不带时外壳补上并在诊断里记一条。
- `onOpen` 里 space 通常会切 space、开右栏、打开会话：配合 `ipdrun-18`（`spaces.switch`）与 `ipdrun-19`（`details.open`）。
- 只对前台 space 的对话生效；哪些位置的文字参与匹配（例如代码块里的要不要）沿用 `ChatPanel` 现在的规则，不在本条改。

## 兼容性

- 不给 `ids` 的 space：没有变化。
- 贡献点 API 版本：可选字段，可以不升。
- Helm、hello-app 不受影响。

## 验收

- `tests/live/ui/conversation.spec.ts`：testapp 的 space 给 `ids: () => ({ pattern: /\bTK-\d+\b/g, onOpen: (id) => …记下 id })`；
  录像里模型回答「见 TK-12」：正文里 `TK-12` 是可点的 chip，点了以后 space 记下 `TK-12`。
- `canOpen` 返回 false 的编号：画成普通文字（或不可点的样式，按 `ChatPanel` 现有行为）。
- 不给 `ids` 的 space：同一段回答里 `TK-12` 是普通文字（回归）。

## 依赖

无；`onOpen` 里想跳到别的 space 要 `ipdrun-18`。

## 来源

IPDrun 要求对话里出现的任何编号都能点开，现在只有它自己画的卡片里的编号可点，模型回复正文里的不行。
