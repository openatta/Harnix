# ipdrun-40：打开会话时定位不到其中的某一条

类型：新能力｜优先级：中

## 现象或需求

`host.session.open(ref, { switchSpace? })`（`ui/packages/react/src/host.tsx:41`）只打开会话。space 想「打开这个会话并停在某一条上」
（一张卡、一段回答），要自己等：新会话的对话挂上、`harnix.attach` 回来、转录追上、`flow.block` 画出来，然后在 DOM 里找、滚过去、
加高亮——各家各写一遍，等的方式多半是固定间隔轮询到某个超时。外壳知道会话何时追上（控制器的 `loading`），也知道对话在哪。

## 建议

```ts
session: {
  open(ref: SessionRef, options: { switchSpace?: boolean; reveal: string }): Promise<boolean>;
  open(ref: SessionRef, options?: { switchSpace?: boolean }): void | Promise<void>;   // 同以前
  /** 当前会话里的这一条：滚到中间并高亮；找不到（或没开会话）是 false。 */
  reveal(key: string): Promise<boolean>;
};
```

- 「这一条」：对话里 `[data-reveal="<key>"]` 的元素（`flow.block` 的渲染给自己的块加上这个属性），没有就找 `data-id` 是 `key` 的块（`.hx-blk`）。
- 等：前台当前会话就是 `ref`、它的控制器 `loading` 为假（失败或人已经换到别的会话 ⇒ false）；追上之后再给一小段时间等块画出来；总时长有上限。
- 找到：`scrollIntoView({ block: "center" })`，加 `data-revealed`，外壳样式闪 2.4 秒（用 `--accent-soft` / `--accent-line`），之后去掉。

## 兼容性

`open` 不带 `reveal` 时返回值与行为不变；`session.reveal` 是新成员；`data-revealed` 是新属性。带 `switchSpace` 又带 `reveal`、而会话不存在时，
与以前一样 reject。贡献点 API 版本不必升。

## 验收

`ui/packages/shell/test/reveal.test.ts`（jsdom，假控制器）：

- `open(ref, {reveal})`：转录还在追时不标记；追上后找到 `[data-reveal]`，滚到中间、带上 `data-revealed`，返回 true。
- `reveal(blockId)` 找到块；2.4 秒后标记去掉。
- 找不到的 key、没有会话、等的时候换到了别的会话 ⇒ false，且在上限内返回。
- 不带 `reveal` 的 `open` 照旧返回 `undefined`。

真实页面上的滚动与闪烁（live UI 测试）未写。

## 依赖

无。

## 来源

IPDrun 从修订 / 提议回到对话里提出它的那一张卡（`ui/kit/src/compat/focus.ts`，HARNIX-WORKAROUND C1-#14）：现在是打开会话后每 200ms 找一次卡、最多 15 秒，再自己滚动与闪烁。

## 落地

落地提交：`8d20005`（main，2026-11-01，未推送；版本待下次发布）。
外壳：`ui/packages/shell/src/reveal.ts`（`revealEntry`、`findEntry`、`markRevealed`），`hostFor` 的 `session.open` / `session.reveal`，
`styles/frame.css` 的 `#main [data-revealed]`。
